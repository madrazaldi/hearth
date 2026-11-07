//! draft/multiline: sending a composer message with line breaks as one batch.

use crate::irc::split_exact;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultilineLimits {
    pub max_bytes: usize,
    pub max_lines: usize,
}

/// One PRIVMSG in a batch; `concat` joins it to the previous one without a line break.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub text: String,
    pub concat: bool,
}

/// Parse a cap value like "max-bytes=4096,max-lines=24". max-bytes is mandatory.
pub fn parse_multiline_cap(value: Option<&str>) -> Option<MultilineLimits> {
    let mut max_bytes = 0usize;
    let mut max_lines = usize::MAX;
    for kv in value.unwrap_or("").split(',') {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        let n = v.parse::<usize>().ok().filter(|n| *n > 0);
        match (k, n) {
            ("max-bytes", Some(n)) => max_bytes = n,
            ("max-lines", Some(n)) => max_lines = n,
            _ => {}
        }
    }
    (max_bytes > 0).then_some(MultilineLimits { max_bytes, max_lines })
}

/// The batches to send `text` in: each is a list of parts that fits the
/// server's limits, with over-long lines split into concatenated pieces.
pub fn multiline_batches(text: &str, limits: MultilineLimits, line_bytes: usize) -> Vec<Vec<Part>> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = normalized.trim_matches('\n');
    let mut parts: Vec<Part> = Vec::new();
    for line in trimmed.split('\n') {
        if line.is_empty() {
            parts.push(Part { text: String::new(), concat: false });
            continue;
        }
        let cap = line_bytes.min(limits.max_bytes.saturating_sub(1)).max(1);
        for (i, t) in split_exact(line, cap).into_iter().enumerate() {
            parts.push(Part { text: t, concat: i > 0 });
        }
    }
    let mut batches: Vec<Vec<Part>> = Vec::new();
    let mut cur: Vec<Part> = Vec::new();
    let mut size = 0;
    for part in parts {
        // Count a line break for every part: conservative about how servers measure.
        let n = part.text.len() + 1;
        if !cur.is_empty() && (cur.len() >= limits.max_lines || size + n > limits.max_bytes) {
            batches.push(std::mem::take(&mut cur));
            size = 0;
        }
        // A batch can't open with a continuation; it starts a fresh line instead.
        cur.push(if cur.is_empty() { Part { concat: false, ..part } } else { part });
        size += n;
    }
    if !cur.is_empty() {
        batches.push(cur);
    }
    // A batch of nothing but blank lines is invalid.
    batches.into_iter().filter(|b| b.iter().any(|p| !p.text.trim().is_empty())).collect()
}

/// Join parts back into the text readers see.
pub fn join_parts(parts: &[Part]) -> String {
    let mut out = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 && !p.concat {
            out.push('\n');
        }
        out.push_str(&p.text);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_cap_value() {
        assert_eq!(parse_multiline_cap(Some("max-bytes=4096,max-lines=24")), Some(MultilineLimits { max_bytes: 4096, max_lines: 24 }));
        assert_eq!(parse_multiline_cap(Some("max-bytes=10")), Some(MultilineLimits { max_bytes: 10, max_lines: usize::MAX }));
        assert_eq!(parse_multiline_cap(Some("max-lines=3")), None);
        assert_eq!(parse_multiline_cap(None), None);
    }

    #[test]
    fn batches_by_lines_and_bytes_and_joins_back() {
        let limits = MultilineLimits { max_bytes: 4096, max_lines: 2 };
        let b = multiline_batches("a\nb\nc", limits, 400);
        assert_eq!(b.len(), 2);
        assert_eq!(join_parts(&b[0]), "a\nb");
        assert_eq!(join_parts(&b[1]), "c");
        let long = "word ".repeat(30);
        let b = multiline_batches(&format!("x\n{}", long.trim()), MultilineLimits { max_bytes: 4096, max_lines: 24 }, 50);
        assert_eq!(b.len(), 1);
        assert!(b[0].iter().skip(1).any(|p| p.concat));
        assert_eq!(join_parts(&b[0]), format!("x\n{}", long.trim()));
        assert!(multiline_batches("\n\n  \n", limits, 400).is_empty());
    }
}
