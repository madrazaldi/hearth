//! One server connection (or one soju network behind a bouncer) and its buffers.

use crate::buffer::{collate, BufId, Buffer, BufferKind};
use crate::effect::{Effect, Secret};
use crate::env::{Env, Timer};
use crate::irc::{is_channel_name, parse_prefix, Fold, PrefixMap};
use crate::types::ConnStatus;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// A channel from `/list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelInfo {
    pub name: String,
    pub users: u32,
    pub topic: String,
}

#[derive(Debug, Clone)]
pub struct ChannelList {
    pub loading: bool,
    pub items: Vec<ChannelInfo>,
    /// When the list was started or finished (ms).
    pub at: i64,
}

/// What someone has published about themselves with draft/metadata-2.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile {
    /// An image URL; load it with `App::request_avatar`, never directly.
    pub avatar: Option<String>,
    pub display_name: Option<String>,
}

/// draft/account-registration credentials from `/register`, kept in memory
/// only until the server answers, so they can be offered for SASL.
#[derive(Debug, Clone)]
pub struct Registration {
    pub account: String,
    pub password: Secret<String>,
}

pub struct Conn {
    pub(crate) env: Env,
    pub id: String,
    pub server_id: String,
    pub parent_id: Option<String>,
    pub netid: Option<String>,
    pub status: ConnStatus,
    pub error: Option<String>,
    /// When the next reconnect attempt starts (ms), or 0.
    pub retry_at: i64,
    pub nick: String,
    pub caps: BTreeSet<String>,
    /// Values of enabled caps that have one, e.g. draft/multiline limits.
    pub cap_values: BTreeMap<String, String>,
    pub isupport: BTreeMap<String, String>,
    buffers: BTreeMap<String, Buffer>,
    pub server: Buffer,
    /// soju bouncer networks (on the control connection): netid → attributes.
    pub networks: BTreeMap<String, BTreeMap<String, String>>,
    pub away: bool,
    pub collapsed: bool,
    /// MONITOR state: case-folded nick → online.
    pub online: BTreeMap<String, bool>,
    pub channel_list: Option<ChannelList>,
    /// draft/metadata-2 key/values: case-folded nick (or channel) → key → value.
    pub metadata: BTreeMap<String, BTreeMap<String, String>>,
    pub(crate) whois: BTreeMap<String, BTreeMap<String, String>>,
    pub(crate) motd: Vec<String>,
    pub(crate) list_acc: Option<Vec<ChannelInfo>>,
    /// We sent AWAY because the window sat in the background.
    pub auto_away: bool,
    /// Saved buffers have been restored, so saving won't clobber them.
    pub buffers_loaded: bool,
    /// Channel keys from /join, applied when the JOIN comes back.
    pub(crate) pending_keys: BTreeMap<String, String>,
    /// labeled-response: label → key of the buffer that issued the command.
    labels: BTreeMap<String, String>,
    label_order: VecDeque<String>,
    pub(crate) ctcp_times: Vec<i64>,
    /// The bouncer's SEARCH needs `in`, so search buffer by buffer.
    pub search_per_target: bool,
    pub(crate) ctcp_warned: i64,
    pub registration: Option<Registration>,
    name: String,
    prefix: PrefixMap,
    fold: Fold,
}

impl Conn {
    pub(crate) fn new(env: &Env, id: &str, server_id: &str, parent_id: Option<String>, netid: Option<String>) -> Conn {
        let pref = format!("{}|{}", server_id, netid.as_deref().unwrap_or(""));
        let fold = Fold::default();
        Conn {
            env: env.clone(),
            id: id.into(),
            server_id: server_id.into(),
            parent_id,
            netid,
            status: ConnStatus::Connecting,
            error: None,
            retry_at: 0,
            nick: String::new(),
            caps: BTreeSet::new(),
            cap_values: BTreeMap::new(),
            isupport: BTreeMap::new(),
            buffers: BTreeMap::new(),
            server: Buffer::new(env, id, &pref, fold, false, BufferKind::Server, ""),
            networks: BTreeMap::new(),
            away: false,
            collapsed: false,
            online: BTreeMap::new(),
            channel_list: None,
            metadata: BTreeMap::new(),
            whois: BTreeMap::new(),
            motd: Vec::new(),
            list_acc: None,
            auto_away: false,
            buffers_loaded: false,
            pending_keys: BTreeMap::new(),
            labels: BTreeMap::new(),
            label_order: VecDeque::new(),
            ctcp_times: Vec::new(),
            search_per_target: false,
            ctcp_warned: 0,
            registration: None,
            name: id.into(),
            prefix: PrefixMap::default(),
            fold,
        }
    }

    /// Stable across restarts; used for saved preferences.
    pub fn pref_key(&self) -> String {
        format!("{}|{}", self.server_id, self.netid.as_deref().unwrap_or(""))
    }

    pub fn casemapping(&self) -> &str {
        self.isupport.get("CASEMAPPING").map(String::as_str).unwrap_or("rfc1459")
    }

    pub fn fold(&self, s: &str) -> String {
        self.fold.apply(s)
    }

    pub fn fold_kind(&self) -> Fold {
        self.fold
    }

    pub fn prefix(&self) -> &PrefixMap {
        &self.prefix
    }

    pub(crate) fn set_isupport(&mut self, isupport: BTreeMap<String, String>) {
        self.isupport = isupport;
        self.prefix = parse_prefix(&self.isupport);
        self.fold = Fold::from_isupport(self.isupport.get("CASEMAPPING").map(String::as_str));
        let fold = self.fold;
        self.server.set_fold(fold);
        for b in self.buffers.values_mut() {
            b.set_fold(fold);
        }
        self.env.touch();
    }

    /// The soju control connection (the bouncer itself, not one of its networks).
    pub fn is_control(&self) -> bool {
        self.netid.is_none() && self.caps.contains("soju.im/bouncer-networks")
    }

    /// Display name: the bouncer's name for the network, else ISUPPORT NETWORK, else the config.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn set_name(&mut self, name: String) {
        self.name = name;
    }

    pub fn can_monitor(&self) -> bool {
        self.isupport.contains_key("MONITOR") && !self.is_control()
    }

    pub fn is_channel(&self, name: &str) -> bool {
        is_channel_name(name, self.isupport.get("CHANTYPES").map(String::as_str).unwrap_or("#&"))
    }

    pub fn is_me(&self, nick: &str) -> bool {
        !nick.is_empty() && self.fold(nick) == self.fold(&self.nick)
    }

    pub fn has(&self, cap: &str) -> bool {
        self.caps.contains(cap)
    }

    pub fn can_tag(&self, tag: &str) -> bool {
        if !self.has("message-tags") {
            return false;
        }
        let deny: Vec<&str> = self.isupport.get("CLIENTTAGDENY").map(String::as_str).unwrap_or("").split(',').filter(|s| !s.is_empty()).collect();
        if deny.contains(&"*") {
            return deny.iter().any(|d| d.strip_prefix('-') == Some(tag));
        }
        !deny.contains(&tag)
    }

    pub fn isupport_value(&self, key: &str) -> Option<&str> {
        self.isupport.get(key).map(String::as_str)
    }

    // ── Buffers ──────────────────────────────────────────────

    /// A buffer by its key (`""` is the server buffer).
    pub fn buf(&self, key: &str) -> Option<&Buffer> {
        if key.is_empty() {
            Some(&self.server)
        } else {
            self.buffers.get(key)
        }
    }

    pub fn buf_mut(&mut self, key: &str) -> Option<&mut Buffer> {
        if key.is_empty() {
            Some(&mut self.server)
        } else {
            self.buffers.get_mut(key)
        }
    }

    /// A buffer by display name.
    pub fn buffer(&self, name: &str) -> Option<&Buffer> {
        self.buffers.get(&self.fold(name))
    }

    pub fn buffer_mut(&mut self, name: &str) -> Option<&mut Buffer> {
        let k = self.fold(name);
        self.buffers.get_mut(&k)
    }

    /// The key a buffer of that name has (or would have), if it exists.
    pub fn key_of(&self, name: &str) -> Option<String> {
        let k = self.fold(name);
        self.buffers.contains_key(&k).then_some(k)
    }

    /// Finds or opens a buffer and returns its key.
    pub fn ensure(&mut self, name: &str, kind: Option<BufferKind>) -> String {
        let key = self.fold(name);
        if self.buffers.contains_key(&key) {
            return key;
        }
        let kind = kind.unwrap_or(if self.is_channel(name) { BufferKind::Channel } else { BufferKind::Query });
        let pref = self.pref_key();
        let b = Buffer::new(&self.env, &self.id, &pref, self.fold, self.has("draft/read-marker"), kind, name);
        self.buffers.insert(key.clone(), b);
        self.env.touch();
        if kind == BufferKind::Query {
            self.save_buffers_soon();
            let connected = self.status == ConnStatus::Connected;
            if self.can_monitor() && connected {
                self.send(format!("MONITOR + {name}"));
            }
            // Servers only push markers for joined channels; queries must ask.
            if self.has("draft/read-marker") && connected {
                self.send(format!("MARKREAD {name}"));
            }
        }
        key
    }

    /// Removes a buffer, returning it.
    pub(crate) fn remove_buffer(&mut self, key: &str) -> Option<Buffer> {
        let b = self.buffers.remove(key);
        if b.is_some() {
            self.env.touch();
        }
        b
    }

    /// Re-files a buffer under a new name (nick change, channel rename).
    pub(crate) fn rename_buffer(&mut self, key: &str, new_name: &str) -> Option<String> {
        let mut b = self.buffers.remove(key)?;
        b.name = new_name.to_string();
        let new_key = self.fold(new_name);
        self.buffers.insert(new_key.clone(), b);
        self.env.touch();
        Some(new_key)
    }

    /// Server buffer first, then channels and queries as the sidebar lists them.
    pub fn all_buffers(&self) -> impl Iterator<Item = &Buffer> {
        std::iter::once(&self.server).chain(self.buffers.values())
    }

    pub(crate) fn buffers_iter(&self) -> impl Iterator<Item = &Buffer> {
        self.buffers.values()
    }

    pub(crate) fn buffers_iter_mut(&mut self) -> impl Iterator<Item = &mut Buffer> {
        self.buffers.values_mut()
    }

    /// Keys of every buffer except the server buffer, for loops that mutate buffers.
    pub(crate) fn keys(&self) -> Vec<String> {
        self.buffers.keys().cloned().collect()
    }

    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
    }

    /// Channels then queries, each sorted by name without the `#`.
    pub fn channel_buffers(&self) -> Vec<&Buffer> {
        let sort_key = |b: &Buffer| self.fold(&b.name).trim_start_matches(['#', '&', '+', '!']).to_string();
        let mut chans: Vec<&Buffer> = self.buffers.values().filter(|b| b.kind == BufferKind::Channel).collect();
        let mut queries: Vec<&Buffer> = self.buffers.values().filter(|b| b.kind == BufferKind::Query).collect();
        chans.sort_by(|a, b| collate(&sort_key(a), &sort_key(b)));
        queries.sort_by(|a, b| collate(&sort_key(a), &sort_key(b)));
        chans.extend(queries);
        chans
    }

    pub fn buf_id(&self, key: &str) -> BufId {
        BufId::new(self.id.clone(), key)
    }

    // ── Sending ──────────────────────────────────────────────

    pub fn send(&self, line: impl Into<String>) {
        self.send_lines(vec![line.into()]);
    }

    pub fn send_lines(&self, lines: Vec<String>) {
        self.env.emit(Effect::Send { conn: self.id.clone(), lines });
    }

    /// Send a command whose replies should land in the buffer `key` (via labeled-response).
    pub fn send_from(&mut self, key: &str, line: &str) {
        let label = if line.starts_with('@') { None } else { self.label_for(key) };
        match label {
            Some(l) => self.send(format!("@label={l} {line}")),
            None => self.send(line),
        }
    }

    /// A fresh label whose replies go to the buffer `key`, if the server supports labels.
    pub fn label_for(&mut self, key: &str) -> Option<String> {
        if !self.has("labeled-response") {
            return None;
        }
        let label = format!("f{}", self.env.next_seq());
        self.labels.insert(label.clone(), key.to_string());
        self.label_order.push_back(label.clone());
        if self.labels.len() > 200 {
            if let Some(old) = self.label_order.pop_front() {
                self.labels.remove(&old);
            }
        }
        Some(label)
    }

    pub(crate) fn label_target(&self, label: &str) -> Option<&String> {
        self.labels.get(label)
    }

    pub fn save_buffers_soon(&self) {
        if !self.buffers_loaded || self.is_control() {
            return;
        }
        self.env.schedule(500, Timer::SaveBuffers { conn: self.id.clone() });
    }

    // ── Metadata ─────────────────────────────────────────────

    pub fn profile(&self, nick: &str) -> Profile {
        let m = self.metadata.get(&self.fold(nick));
        let get = |k: &str| m.and_then(|m| m.get(k)).filter(|v| !v.is_empty()).cloned();
        Profile { avatar: get("avatar"), display_name: get("display-name") }
    }

    /// Record a metadata key; `None` means it was removed.
    pub fn set_metadata(&mut self, target: &str, key: &str, value: Option<&str>) {
        let k = self.fold(target);
        let mut next = self.metadata.get(&k).cloned().unwrap_or_default();
        match value {
            None => {
                next.remove(key);
            }
            Some(v) => {
                next.insert(key.to_string(), v.to_string());
            }
        }
        if next.is_empty() {
            self.metadata.remove(&k);
        } else {
            self.metadata.insert(k, next);
        }
        self.env.touch();
    }

    /// Whether a buffer key is still bound (used to validate remembered labels).
    pub(crate) fn has_buffer_key(&self, key: &str) -> bool {
        key.is_empty() || self.buffers.contains_key(key)
    }
}
