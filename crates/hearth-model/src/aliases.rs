//! User-defined slash commands: `/ns identify x` → `/msg NickServ identify x`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasVars {
    pub nick: String,
    pub channel: String,
    pub network: String,
}

/// Split a body into its `;`-separated commands. `\;` is a literal semicolon.
pub fn split_commands(body: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev: Option<char> = None;
    for c in body.chars() {
        if c == ';' && prev != Some('\\') {
            parts.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
        prev = Some(c);
    }
    parts.push(cur);
    parts.into_iter().map(|c| c.replace(r"\;", ";").trim().to_string()).filter(|c| !c.is_empty()).collect()
}

/// Substitute `$1`…`$9`, `$2-` (from the 2nd on), `$*`, `$nick`, `$channel` and `$network`.
pub fn expand_command(cmd: &str, args: &[String], vars: &AliasVars) -> String {
    let mut out = String::new();
    let mut rest = cmd;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let mut chars = after.chars();
        match chars.next() {
            Some('*') => {
                out.push_str(&args.join(" "));
                rest = &after[1..];
                continue;
            }
            Some(d @ '1'..='9') => {
                let n = d as usize - '0' as usize;
                if after[1..].starts_with('-') {
                    out.push_str(&args.iter().skip(n - 1).cloned().collect::<Vec<_>>().join(" "));
                    rest = &after[2..];
                } else {
                    out.push_str(args.get(n - 1).map(String::as_str).unwrap_or(""));
                    rest = &after[1..];
                }
                continue;
            }
            _ => {}
        }
        let named = [("nick", &vars.nick), ("channel", &vars.channel), ("network", &vars.network)]
            .into_iter()
            .find(|(name, _)| after.starts_with(name) && !after[name.len()..].starts_with(|c: char| c.is_ascii_alphabetic()));
        match named {
            Some((name, value)) => {
                out.push_str(value);
                rest = &after[name.len()..];
            }
            None => {
                out.push('$');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.trim_end().to_string()
}

/// `/J ` → `j`.
pub fn alias_name(raw: &str) -> String {
    raw.trim().trim_start_matches('/').to_lowercase()
}

pub const BODY_ERROR: &str = "Add at least one command";

/// An error for a name or body that can't work, else `None`.
pub fn alias_error(name: &str, body: &str) -> Option<&'static str> {
    if name.is_empty() {
        return Some("Give the alias a name");
    }
    if !name.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-')) {
        return Some("Names use letters, digits, - and _ only");
    }
    if split_commands(body).is_empty() {
        return Some(BODY_ERROR);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> AliasVars {
        AliasVars { nick: "me".into(), channel: "#a".into(), network: "Testnet".into() }
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn substitutes_positional_rest_and_context_variables() {
        assert_eq!(expand_command("/msg $1 $2-", &args(&["ann", "hi", "there"]), &vars()), "/msg ann hi there");
        assert_eq!(expand_command("/say $* in $channel on $network as $nick", &args(&["x", "y"]), &vars()), "/say x y in #a on Testnet as me");
        assert_eq!(expand_command("/kick $1 $3", &args(&["ann"]), &vars()), "/kick ann");
        assert_eq!(expand_command("$nickname costs $5", &[], &vars()), "$nickname costs");
    }

    #[test]
    fn splits_on_semicolons_with_an_escape() {
        assert_eq!(split_commands(r" /a ; /b x;; hi\; there "), ["/a", "/b x", "hi; there"]);
    }

    #[test]
    fn validates_names_and_bodies() {
        assert_eq!(alias_error("ns", "/msg NickServ $*"), None);
        assert!(alias_error("", "/x").unwrap().contains("name"));
        assert!(alias_error("n s", "/x").unwrap().contains("letters"));
        assert!(alias_error("ns", " ; ").unwrap().contains("command"));
        assert_eq!(alias_name(" //J "), "j");
    }
}
