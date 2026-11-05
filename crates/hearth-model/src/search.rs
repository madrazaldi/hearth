//! Message search across the local FTS log and soju.im/search bouncers.

use crate::buffer::BufId;
use crate::effect::{Effect, HistoryKind, Reply};
use crate::irc::{casefold, escape_attr, iso_time, parse_time};
use crate::state::{App, Pending};
use crate::time::Zone;
use crate::types::{SearchHit, SearchQuery};
use chrono::{Duration, NaiveDate};
use regex::Regex;
use std::collections::HashSet;
use std::sync::OnceLock;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFilters {
    pub text: String,
    pub from: Option<String>,
    pub in_: Option<String>,
    /// Exclusive bounds, in ms.
    pub before: Option<i64>,
    pub after: Option<i64>,
}

impl SearchFilters {
    fn is_filtered(&self) -> bool {
        !self.text.is_empty() || self.from.is_some() || self.in_.is_some() || self.before.is_some() || self.after.is_some()
    }
}

#[derive(Debug, Clone, Default)]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    /// Pass as `before` for the next page; `None` when there's nothing older.
    pub next: Option<i64>,
}

/// A local day (`YYYY-MM-DD`, `today` or `yesterday`) as [start, end) in ms.
fn parse_day(v: &str, now: i64, zone: Zone) -> Option<(i64, i64)> {
    static YMD: OnceLock<Regex> = OnceLock::new();
    let ymd = YMD.get_or_init(|| Regex::new(r"^(\d{4})-(\d{1,2})-(\d{1,2})$").unwrap());
    let mut day = zone.date(now);
    if let Some(c) = ymd.captures(v) {
        day = NaiveDate::from_ymd_opt(c[1].parse().ok()?, c[2].parse().ok()?, c[3].parse().ok()?)?;
    } else if v.eq_ignore_ascii_case("yesterday") {
        day -= Duration::days(1);
    } else if !v.eq_ignore_ascii_case("today") {
        return None;
    }
    Some((zone.midnight(day), zone.midnight(day + Duration::days(1))))
}

/// Split `from:`, `in:`, `before:`, `after:` and `on:` filters out of a query.
pub fn parse_query(q: &str, now: i64, zone: Zone) -> SearchFilters {
    static FILTER: OnceLock<Regex> = OnceLock::new();
    let re = FILTER.get_or_init(|| Regex::new(r"(?i)^(from|in|before|after|on|during):(.+)$").unwrap());
    let mut out = SearchFilters::default();
    let mut words: Vec<&str> = Vec::new();
    for w in q.split_whitespace() {
        let caps = re.captures(w);
        let key = caps.as_ref().map(|c| c[1].to_lowercase());
        let value = caps.as_ref().map(|c| c.get(2).unwrap().as_str());
        let day = match (&key, value) {
            (Some(k), Some(v)) if k != "from" && k != "in" => parse_day(v, now, zone),
            _ => None,
        };
        match (key.as_deref(), value, day) {
            (Some("from"), Some(v), _) => out.from = Some(v.strip_prefix('@').unwrap_or(v).to_string()),
            (Some("in"), Some(v), _) => out.in_ = Some(v.to_string()),
            (Some("before"), _, Some(d)) => out.before = Some(d.0),
            (Some("after"), _, Some(d)) => out.after = Some(d.1 - 1),
            (_, _, Some(d)) => {
                out.after = Some(d.0 - 1);
                out.before = Some(d.1);
            }
            _ => words.push(w),
        }
    }
    out.text = words.join(" ");
    out
}

pub fn hit_time(h: &SearchHit) -> i64 {
    h.msg.tags.get("time").and_then(|t| parse_time(t)).unwrap_or(0)
}

/// Drop hits found by both the log and the bouncer, then sort newest first.
pub fn dedupe_hits(all: Vec<SearchHit>) -> Vec<SearchHit> {
    let mut seen = HashSet::new();
    let mut out: Vec<SearchHit> = all
        .into_iter()
        .filter(|h| {
            let k = match h.msg.tags.get("msgid") {
                Some(id) => format!("{}|{id}", h.network),
                None => format!("{}|{}|{}|{}", h.network, h.target, h.msg.tags.get("time").map(String::as_str).unwrap_or(""), h.msg.param(1)),
            };
            seen.insert(k)
        })
        .collect();
    out.sort_by_key(|h| std::cmp::Reverse(hit_time(h)));
    out
}

/// One source's newest-first page; `full` when it may have older hits.
#[derive(Debug, Clone)]
pub struct SourceResult {
    pub hits: Vec<SearchHit>,
    pub full: bool,
}

/// Merge pages from several sources. A source that filled its page may have
/// more hits older than its oldest, so the merged page stops there, and the
/// next one starts from it.
pub fn merge_pages(results: Vec<SourceResult>) -> SearchPage {
    let cutoff = results.iter().filter(|r| r.full).filter_map(|r| r.hits.iter().map(hit_time).min()).max();
    let hits = dedupe_hits(results.into_iter().flat_map(|r| r.hits).collect());
    match cutoff {
        None => SearchPage { hits, next: None },
        Some(c) => SearchPage { hits: hits.into_iter().filter(|h| hit_time(h) >= c).collect(), next: Some(c) },
    }
}

const REMOTE_LIMIT: usize = 100;
const PER_TARGET_LIMIT: usize = 50;
const LOCAL_LIMIT: usize = 200;

/// Where a search looks. A buffer scopes it to that buffer; a connection to that network.
#[derive(Debug, Clone, Default)]
pub struct SearchScope {
    pub conn: Option<String>,
    pub buf: Option<BufId>,
}

pub(crate) struct Job {
    filters: SearchFilters,
    upper: Option<i64>,
    outstanding: usize,
    results: Vec<SourceResult>,
}

/// One request a search is waiting on.
pub(crate) enum SearchStep {
    Local { job: u64 },
    Remote { job: u64, conn: String, target: Option<String>, limit: usize, global: bool },
}

impl SearchStep {
    fn job(&self) -> u64 {
        match self {
            SearchStep::Local { job } | SearchStep::Remote { job, .. } => *job,
        }
    }
}

impl App {
    /// Search the local log and, where supported, the bouncer: one page,
    /// newest first, of hits older than `before` (from a previous page's
    /// `next`). Returns a handle; the page arrives via [`App::take_search`].
    pub fn search(&mut self, query: &str, scope: &SearchScope, before: Option<i64>) -> u64 {
        let job = self.env.next_req();
        let f = parse_query(query, self.now(), self.zone);
        if !f.is_filtered() {
            self.search_pages.insert(job, SearchPage::default());
            self.env.touch();
            return job;
        }
        let upper = match (before, f.before) {
            (Some(b), Some(x)) => Some(b.min(x)),
            (b, x) => b.or(x),
        };
        let buf = scope.buf.as_ref().filter(|b| !b.is_server()).and_then(|b| self.buffer(b));
        let target = f.in_.clone().or_else(|| buf.map(|b| b.name.clone()));
        self.searches.insert(job, Job { filters: f.clone(), upper, outstanding: 0, results: Vec::new() });

        if self.settings.store_logs {
            let key = target.as_ref().map(|t| match scope.conn.as_ref().and_then(|c| self.conns.get(c)) {
                Some(c) => c.fold(t),
                None => casefold(t, "rfc1459"),
            });
            let q = SearchQuery {
                text: f.text.clone(),
                network: scope.conn.clone(),
                target: key,
                from: f.from.clone(),
                before: upper,
                after: f.after,
                limit: LOCAL_LIMIT,
            };
            let req = self.env.next_req();
            self.pending.insert(req, Pending::Search(SearchStep::Local { job }));
            self.env.emit(Effect::Search { req, query: q });
            self.jobs_add(job, 1);
        }
        let remote: Vec<String> = match &scope.conn {
            Some(c) => vec![c.clone()],
            None => self.conns.keys().cloned().collect(),
        };
        for cid in remote {
            let Some(c) = self.conns.get(&cid) else { continue };
            if !c.has("soju.im/search") || c.status != crate::types::ConnStatus::Connected || c.is_control() {
                continue;
            }
            self.search_conn(job, &cid, target.clone());
        }
        self.finish_if_idle(job);
        job
    }

    /// Forget a search; answers still on their way are ignored.
    pub fn cancel_search(&mut self, job: u64) {
        self.searches.remove(&job);
        self.search_pages.remove(&job);
    }

    /// The finished page for a search handle, once every source has answered.
    pub fn take_search(&mut self, job: u64) -> Option<SearchPage> {
        self.search_pages.remove(&job)
    }

    fn jobs_add(&mut self, job: u64, n: usize) {
        if let Some(j) = self.searches.get_mut(&job) {
            j.outstanding += n;
        }
    }

    /// Search one bouncer network. Some (soju, for one) require `in`, so when a
    /// search everywhere fails, ask each open buffer instead, from then on.
    fn search_conn(&mut self, job: u64, cid: &str, target: Option<String>) {
        let per_target = self.conns.get(cid).is_some_and(|c| c.search_per_target);
        match (&target, per_target) {
            (Some(_), _) => self.remote_search(job, cid, target, true),
            (None, false) => self.remote_search(job, cid, None, true),
            (None, true) => self.search_each_buffer(job, cid),
        }
    }

    fn search_each_buffer(&mut self, job: u64, cid: &str) {
        let names: Vec<String> = self.conns.get(cid).map(|c| c.buffers_iter().map(|b| b.name.clone()).collect()).unwrap_or_default();
        for n in names {
            self.remote_search(job, cid, Some(n), false);
        }
    }

    fn remote_search(&mut self, job: u64, cid: &str, target: Option<String>, global: bool) {
        let Some(j) = self.searches.get(&job) else { return };
        let (f, upper) = (j.filters.clone(), j.upper);
        let limit = if target.is_some() { PER_TARGET_LIMIT } else { REMOTE_LIMIT };
        let mut attrs = format!("limit={limit}");
        if !f.text.is_empty() {
            attrs += &format!(";text={}", escape_attr(&f.text));
        }
        if let Some(from) = &f.from {
            attrs += &format!(";from={}", escape_attr(from));
        }
        if let Some(t) = &target {
            attrs += &format!(";in={}", escape_attr(t));
        }
        if let Some(b) = upper {
            attrs += &format!(";before={}", iso_time(b));
        }
        if let Some(a) = f.after {
            attrs += &format!(";after={}", iso_time(a));
        }
        let req = self.env.next_req();
        self.env.emit(Effect::History { req, conn: cid.to_string(), kind: HistoryKind::Search, target: "*".into(), line: format!("SEARCH {attrs}") });
        self.pending.insert(req, Pending::Search(SearchStep::Remote { job, conn: cid.to_string(), target, limit, global }));
        self.jobs_add(job, 1);
    }

    fn finish_if_idle(&mut self, job: u64) {
        if self.searches.get(&job).is_some_and(|j| j.outstanding == 0) {
            let j = self.searches.remove(&job).unwrap();
            self.search_pages.insert(job, merge_pages(j.results));
            self.env.touch();
        }
    }
}

pub(crate) fn step_done(app: &mut App, step: SearchStep, result: Result<Reply, String>) {
    let job = step.job();
    if !app.searches.contains_key(&job) {
        return;
    }
    match step {
        SearchStep::Local { .. } => {
            let r = match result {
                Ok(Reply::Hits(hits)) => SourceResult { full: hits.len() >= LOCAL_LIMIT, hits },
                _ => SourceResult { hits: Vec::new(), full: false },
            };
            push_result(app, job, r);
        }
        SearchStep::Remote { conn, target, limit, global, .. } => match result {
            Ok(Reply::Messages(msgs)) => {
                let full = msgs.len() >= limit;
                let hits = match app.conns.get(&conn) {
                    Some(c) => msgs
                        .into_iter()
                        .map(|msg| {
                            let to = msg.param(0).to_string();
                            let peer = match &target {
                                Some(t) => t.clone(),
                                None if c.is_channel(&to) || msg.source.is_none() || msg.source.as_ref().is_some_and(|s| c.is_me(&s.nick)) => to,
                                None => msg.source.as_ref().map(|s| s.nick.clone()).unwrap_or(to),
                            };
                            SearchHit { network: c.id.clone(), target: c.fold(&peer), msg }
                        })
                        .collect(),
                    None => Vec::new(),
                };
                push_result(app, job, SourceResult { hits, full });
            }
            Err(_) if target.is_none() && global => {
                // The bouncer wants `in`: ask each buffer, and remember to.
                if let Some(c) = app.conns.get_mut(&conn) {
                    c.search_per_target = true;
                }
                if let Some(j) = app.searches.get_mut(&job) {
                    j.outstanding -= 1;
                }
                app.search_each_buffer(job, &conn);
                app.finish_if_idle(job);
            }
            _ => push_result(app, job, SourceResult { hits: Vec::new(), full: false }),
        },
    }
}

fn push_result(app: &mut App, job: u64, r: SourceResult) {
    if let Some(j) = app.searches.get_mut(&job) {
        j.results.push(r);
        j.outstanding -= 1;
    }
    app.finish_if_idle(job);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::IrcMessage;

    const BASE: i64 = 1_767_225_600_000;
    fn t(s: i64) -> i64 {
        BASE + s * 1000
    }
    fn hit(s: i64) -> SearchHit {
        let line = format!("@time={} :ann!a@h PRIVMSG #a :m{s}", iso_time(t(s)));
        SearchHit { network: "c".into(), target: "#a".into(), msg: IrcMessage::parse(&line).unwrap() }
    }

    #[test]
    fn parses_filters_out_of_the_query() {
        let z = Zone::UTC;
        let now = 1_791_817_200_000; // 2026-10-12 15:00 UTC
        assert_eq!(
            parse_query("deploy from:@ann in:#ops failed", now, z),
            SearchFilters { text: "deploy failed".into(), from: Some("ann".into()), in_: Some("#ops".into()), ..Default::default() }
        );
        let day = z.midnight(NaiveDate::from_ymd_opt(2026, 10, 1).unwrap());
        let next = day + 86_400_000;
        let on = parse_query("on:2026-10-01", now, z);
        assert_eq!((on.after, on.before), (Some(day - 1), Some(next)));
        let b = parse_query("before:2026-10-01 after:2026-10-01", now, z);
        assert_eq!((b.before, b.after), (Some(day), Some(next - 1)));
        assert_eq!(parse_query("on:yesterday", now, z).before, Some(z.midnight(NaiveDate::from_ymd_opt(2026, 10, 12).unwrap())));
        assert_eq!(parse_query("before:soon http://x", now, z), SearchFilters { text: "before:soon http://x".into(), ..Default::default() });
    }

    #[test]
    fn stops_a_merged_page_where_a_full_source_runs_out() {
        let page = merge_pages(vec![SourceResult { hits: vec![hit(100), hit(50)], full: true }, SourceResult { hits: vec![hit(90), hit(10)], full: false }]);
        let texts: Vec<_> = page.hits.iter().map(|h| h.msg.param(1).to_string()).collect();
        assert_eq!(texts, ["m100", "m90", "m50"]);
        assert_eq!(page.next, Some(t(50)));
        assert_eq!(merge_pages(vec![SourceResult { hits: vec![hit(1)], full: false }]).next, None);
    }
}
