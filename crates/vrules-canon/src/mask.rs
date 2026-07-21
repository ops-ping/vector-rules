//! Stateless, deterministic masking of variable substrings.
//!
//! [`LogMask`] replaces the high-cardinality, semantically-empty parts of a
//! line — integers, floats, hex blobs, UUIDs, IPv4/IPv6 addresses, ISO-8601
//! timestamps, and filesystem paths — with the placeholder [`MASK`]. The
//! surrounding words (which carry the meaning) are preserved, so the canonical
//! form still embeds to a useful vector while every variant of a recurring
//! line collapses to one cache key.
//!
//! Everything here is a pure function of its input: no regex engine, no global
//! state, no allocation-heavy backtracking — just per-token classification.

use crate::{CanonMode, CanonResult, Canonicalizer};

/// Placeholder substituted for every masked variable.
pub const MASK: &str = "<*>";

/// Stateless log/text canonicalizer. See the [module docs](self).
#[derive(Debug, Default, Clone, Copy)]
pub struct LogMask;

impl Canonicalizer for LogMask {
    fn id(&self) -> &str {
        "log-mask"
    }

    fn version(&self) -> u32 {
        1
    }

    fn canon(&self, input: &str) -> CanonResult {
        let mut out = String::with_capacity(input.len());
        let mut vars: Vec<String> = Vec::new();
        let mut first = true;
        for token in input.split_whitespace() {
            if !first {
                out.push(' ');
            }
            first = false;
            mask_token(token, &mut out, &mut vars);
        }
        CanonResult::new(out, vars, CanonMode::Log)
    }
}

/// Mask one whitespace-delimited token, appending the (possibly rewritten)
/// token to `out` and any extracted originals to `vars`.
///
/// Handles three shapes:
/// 1. `key=value` / `key:value` — mask only the value if it's variable.
/// 2. a bare value wrapped in leading/trailing punctuation — mask the core.
/// 3. anything else — passed through unchanged.
fn mask_token(token: &str, out: &mut String, vars: &mut Vec<String>) {
    // Shape 1: key=value or key:value (single separator, alpha-ish key).
    for sep in ['=', ':'] {
        if let Some(pos) = token.find(sep) {
            let (key, rest) = token.split_at(pos);
            let value = &rest[1..];
            if !key.is_empty()
                && !value.is_empty()
                && key_is_label(key)
                && pos == token.rfind(sep).unwrap_or(pos)
            {
                out.push_str(key);
                out.push(sep);
                mask_core_with_punct(value, out, vars);
                return;
            }
        }
    }
    // Shape 2/3: strip surrounding punctuation, classify the core.
    mask_core_with_punct(token, out, vars);
}

/// Strip leading/trailing punctuation, classify the core; if variable, emit the
/// punctuation around a [`MASK`] and record the original core.
fn mask_core_with_punct(token: &str, out: &mut String, vars: &mut Vec<String>) {
    let start = token
        .char_indices()
        .find(|&(_, c)| !is_edge_punct(c))
        .map_or(token.len(), |(i, _)| i);
    let end = token
        .char_indices()
        .rev()
        .find(|&(_, c)| !is_edge_punct(c))
        .map_or(0, |(i, c)| i + c.len_utf8());

    if start >= end {
        out.push_str(token); // pure punctuation / empty
        return;
    }
    let (prefix, core, suffix) = (&token[..start], &token[start..end], &token[end..]);
    if is_variable(core) {
        out.push_str(prefix);
        out.push_str(MASK);
        out.push_str(suffix);
        vars.push(core.to_owned());
    } else {
        out.push_str(token);
    }
}

/// Punctuation that may wrap a value: brackets, quotes, commas, etc. Note `.`
/// and `:` are NOT edge punctuation because they're significant inside IPs,
/// timestamps, and floats; trailing sentence dots are rare in structured logs.
fn is_edge_punct(c: char) -> bool {
    matches!(
        c,
        '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '"' | '\'' | '`' | ',' | ';'
    )
}

/// A token key is a "label" if it's a plausible field name (letters, digits,
/// `_`, `-`, `.`) starting with a letter or `_`. Guards against treating a
/// `host:port` or `1:2` as key=value.
fn key_is_label(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    key.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Whether a bare core token is a variable that should be masked.
#[must_use]
pub fn is_variable(s: &str) -> bool {
    is_number(s)
        || is_hex_blob(s)
        || is_uuid(s)
        || is_ipv4(s)
        || is_ipv6(s)
        || is_iso8601(s)
        || is_path(s)
        || is_number_with_unit(s)
}

fn is_number(s: &str) -> bool {
    let body = s.strip_prefix(['-', '+']).unwrap_or(s);
    if body.is_empty() {
        return false;
    }
    let mut dots = 0;
    for c in body.chars() {
        if c == '.' {
            dots += 1;
        } else if !c.is_ascii_digit() {
            return false;
        }
    }
    dots <= 1
}

/// `123ms`, `4KB`, `1.5s` — digits then letters (a unit suffix).
fn is_number_with_unit(s: &str) -> bool {
    let split = s.char_indices().find(|&(_, c)| c.is_ascii_alphabetic());
    let Some((i, _)) = split else { return false };
    if i == 0 {
        return false;
    }
    let (num, unit) = s.split_at(i);
    is_number(num) && unit.chars().all(|c| c.is_ascii_alphabetic())
}

/// `0x…` prefixed hex, or a long (>=8) bare hex run with at least one
/// non-decimal hex digit (so plain decimal ids fall through to [`is_number`]).
fn is_hex_blob(s: &str) -> bool {
    if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_hexdigit());
    }
    s.len() >= 8
        && s.chars().all(|c| c.is_ascii_hexdigit())
        && s.chars().any(|c| c.is_ascii_alphabetic())
}

fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 36 {
        return false;
    }
    for (i, &c) in b.iter().enumerate() {
        let expect_dash = matches!(i, 8 | 13 | 18 | 23);
        if expect_dash {
            if c != b'-' {
                return false;
            }
        } else if !c.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

fn is_ipv4(s: &str) -> bool {
    let mut octets = 0;
    for part in s.split('.') {
        octets += 1;
        if octets > 4 || part.is_empty() || part.len() > 3 {
            return false;
        }
        match part.parse::<u16>() {
            Ok(n) if n <= 255 && part.chars().all(|c| c.is_ascii_digit()) => {}
            _ => return false,
        }
    }
    octets == 4
}

fn is_ipv6(s: &str) -> bool {
    // Heuristic: at least two colons and only hex digits / colons. Good enough
    // to catch addresses without a full RFC parser.
    s.matches(':').count() >= 2
        && s.chars().all(|c| c.is_ascii_hexdigit() || c == ':')
        && s.chars().any(|c| c.is_ascii_hexdigit())
}

/// ISO-8601-ish: `YYYY-MM-DD` optionally followed by `T`/space and a time.
fn is_iso8601(s: &str) -> bool {
    let date = &s[..s.len().min(10)];
    let b = date.as_bytes();
    if b.len() != 10 {
        return false;
    }
    let digits_ok = (0..4)
        .chain(5..7)
        .chain(8..10)
        .all(|i| b[i].is_ascii_digit());
    if !(digits_ok && b[4] == b'-' && b[7] == b'-') {
        return false;
    }
    // Either exactly a date, or a date followed by a 'T'/space separator.
    s.len() == 10 || matches!(s.as_bytes().get(10), Some(b'T' | b' '))
}

/// Unix absolute path: starts with `/` and has a second `/` or a `.` segment,
/// and contains no whitespace (already guaranteed — token is whitespace-free).
fn is_path(s: &str) -> bool {
    s.starts_with('/') && s.len() > 1 && (s[1..].contains('/') || s[1..].contains('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str) -> String {
        LogMask.canon(s).canonical
    }

    #[test]
    fn masks_integers_and_keeps_words() {
        assert_eq!(c("User 42 logged in"), "User <*> logged in");
    }

    #[test]
    fn variable_lines_collapse_to_one_template() {
        let a = LogMask.canon("User 42 login from 10.0.0.1");
        let b = LogMask.canon("User 9999 login from 192.168.1.7");
        assert_eq!(a.canonical, b.canonical);
        assert_eq!(a.id, b.id);
        assert_eq!(a.canonical, "User <*> login from <*>");
    }

    #[test]
    fn captures_vars_in_order() {
        let r = LogMask.canon("id 7 ip 10.0.0.1");
        assert_eq!(r.vars, vec!["7", "10.0.0.1"]);
    }

    #[test]
    fn key_value_masks_only_value() {
        assert_eq!(c("user_id=12345 status=ok"), "user_id=<*> status=ok");
        assert_eq!(c("latency=123ms"), "latency=<*>");
    }

    #[test]
    fn masks_uuid_hex_path_timestamp() {
        assert_eq!(
            c("req 550e8400-e29b-41d4-a716-446655440000 done"),
            "req <*> done"
        );
        assert_eq!(c("addr 0xDEADBEEF"), "addr <*>");
        assert_eq!(c("open /var/log/app.log ok"), "open <*> ok");
        assert_eq!(c("at 2026-06-23T13:00:00 boom"), "at <*> boom");
    }

    #[test]
    fn punctuation_wrapped_values_masked() {
        assert_eq!(c("ping (1.2.3.4) ok"), "ping (<*>) ok");
        assert_eq!(c("code [42]"), "code [<*>]");
    }

    #[test]
    fn host_port_not_treated_as_key_value() {
        // "localhost:8080" — key is a label, value 8080 is a number → masked.
        assert_eq!(c("localhost:8080"), "localhost:<*>");
        // "1:2" — key not a label → core "1:2" not variable → unchanged.
        assert_eq!(c("ratio 1:2"), "ratio 1:2");
    }

    #[test]
    fn decimal_ids_are_numbers_not_hex() {
        assert_eq!(c("12345678"), MASK);
    }

    #[test]
    fn idempotent_and_deterministic() {
        let once = c("User 42 login from 10.0.0.1");
        let twice = c(&once);
        assert_eq!(once, twice); // masking already-masked text is stable
        assert_eq!(c("a 1"), c("a 1"));
    }

    #[test]
    fn pure_words_unchanged() {
        assert_eq!(
            c("disk full please clear space"),
            "disk full please clear space"
        );
    }
}
