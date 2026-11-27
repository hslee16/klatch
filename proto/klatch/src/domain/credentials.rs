//! Recognising text that carries a credential, so the gateway can refuse to store it (R20, ticket #31).
//!
//! Only well known shapes are matched, each with a prefix and a minimum length, so ordinary prose, hashes and
//! identifiers pass. A match names the kind of credential and never the text.

//!
//! Mechanical decodings (#987) catch the same credential wrapped to dodge a scan: zero-width characters inside it,
//! a value spaced out or cut over lines, base64 or hex of it, or written backwards. Each decoded or joined form is
//! matched with the same rules as plain text, so nothing is flagged that is not credential-shaped after the
//! decoding. What stays out of reach on purpose: a passphrase of plain words and a secret that only the
//! surrounding talk marks as one; the rules judge does not guess at those, so they are left to the opt-in hosted
//! judge (the owner's own use).

/// Longest text the decodings are tried on; beyond it only the plain match runs.
const MAX_DECODE_TEXT: usize = 64 * 1024;

/// What kind of credential `text` appears to carry, if any.
pub fn credential_kind(text: &str) -> Option<&'static str> {
    if let Some(kind) = plain_kind(text) {
        return Some(kind);
    }
    if text.len() > MAX_DECODE_TEXT {
        return None;
    }
    let cleaned = strip_invisible(text);
    if cleaned != text
        && let Some(kind) = plain_kind(&cleaned)
    {
        return Some(kind);
    }
    if spelled_out_secret(&cleaned) {
        return Some("a spelled-out secret");
    }
    [joined_spaces(&cleaned), joined_lines(&cleaned)]
        .iter()
        .flatten()
        .find_map(|variant| plain_kind(variant))
        .or_else(|| decoded_kind(&cleaned))
}

/// The match on the text as it stands, with no decoding.
fn plain_kind(text: &str) -> Option<&'static str> {
    shape_kind(text)
        .or_else(|| url_or_command_password(&blank_templates(text)).then_some("a password"))
        .or_else(|| labelled_secret(text).then_some("a labelled secret"))
        .or_else(|| secret_field(text).then_some("a labelled secret"))
}

/// A key, token or web token by its fixed shape alone, with no label or context.
fn shape_kind(text: &str) -> Option<&'static str> {
    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY-----") {
        return Some("a private key");
    }
    SHAPES
        .iter()
        .find(|s| s.matches(text))
        .map(|s| s.kind)
        .or_else(|| bearer_token(text).then_some("a bearer token"))
        .or_else(|| json_web_token(text).then_some("a JSON web token"))
}

/// The text without zero-width and other invisible format characters.
pub(super) fn strip_invisible(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !matches!(
                c,
                '\u{200b}'..='\u{200f}'
                    | '\u{2060}'
                    | '\u{feff}'
                    | '\u{00ad}'
                    | '\u{202a}'..='\u{202e}'
            )
        })
        .collect()
}

/// Whitespace separated words with the byte offset each starts at.
fn words_at(text: &str) -> Vec<(usize, &str)> {
    let mut words = Vec::new();
    let mut start = None;
    for (i, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                words.push((s, &text[s..i]));
                start = None;
            }
            _ => {}
        }
    }
    words
}

/// A run of words of one equal width, the last one no wider (`ab cd ef gh`): a value spaced out. Returns the byte
/// range of the run, its word width and the joined value.
fn spaced_runs(text: &str) -> Vec<(std::ops::Range<usize>, usize, String)> {
    let words = words_at(text);
    let mut runs = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let width = words[i].1.chars().count();
        let j = run_end(text, &words, i, width);
        let joined: String = words[i..j].iter().map(|w| w.1).collect();
        if j - i >= 4 && joined.chars().count() >= 12 {
            let end = words[j - 1].0 + words[j - 1].1.len();
            runs.push((words[i].0..end, width, joined));
            i = j;
        } else {
            i += 1;
        }
    }
    runs
}

/// Whether word `j` follows word `j - 1` after exactly one space and has `fits` as its width.
fn next_in_run(
    text: &str,
    words: &[(usize, &str)],
    j: usize,
    fits: impl Fn(usize) -> bool,
) -> bool {
    j < words.len()
        && text[words[j - 1].0 + words[j - 1].1.len()..words[j].0] == *" "
        && fits(words[j].1.chars().count())
}

/// One past the last word of the run that starts at word `i` of `width` characters: words of the same width, then
/// at most one narrower. A word of no width or more than eight characters starts no run (`i + 1`).
fn run_end(text: &str, words: &[(usize, &str)], i: usize, width: usize) -> usize {
    let mut j = i + 1;
    if !(1..=8).contains(&width) {
        return j;
    }
    while next_in_run(text, words, j, |w| w == width) {
        j += 1;
    }
    if next_in_run(text, words, j, |w| w < width) {
        j += 1;
    }
    j
}

/// The text with each spaced-out run joined into one value.
fn joined_spaces(text: &str) -> Option<String> {
    let runs = spaced_runs(text);
    if runs.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (range, _, joined) in runs {
        out.push_str(&text[at..range.start]);
        out.push_str(&joined);
        at = range.end;
    }
    out.push_str(&text[at..]);
    Some(out)
}

/// A value read out one character at a time (`Z q 8 r ...`): twelve or more single characters with a lower case
/// letter and at least two of a digit, an upper case letter and a symbol. Counting `1 2 3 ...` or spelling a word
/// does not qualify.
fn spelled_out_secret(text: &str) -> bool {
    spaced_runs(text).iter().any(|(_, width, joined)| {
        let classes = [
            joined.chars().any(|c| c.is_ascii_digit()),
            joined.chars().any(|c| c.is_ascii_uppercase()),
            joined.chars().any(|c| c.is_ascii_punctuation()),
        ];
        *width == 1
            && joined.chars().any(|c| c.is_ascii_lowercase())
            && classes.iter().filter(|c| **c).count() >= 2
    })
}

fn value_char(c: char) -> bool {
    token(c) || matches!(c, '.' | '~' | '+' | '/' | '=')
}

/// The text with a long bare token line glued to the long token that ends the line above it, so a value cut over
/// lines (a JWT in three pieces) is one value again.
fn joined_lines(text: &str) -> Option<String> {
    let long_token = |s: &str| s.chars().count() >= 12 && s.chars().all(value_char);
    let mut lines: Vec<String> = Vec::new();
    let mut changed = false;
    for line in text.lines() {
        let bare = line.trim();
        let glue = long_token(bare)
            && lines
                .last()
                .is_some_and(|prev| prev.split_whitespace().next_back().is_some_and(long_token));
        if glue && let Some(prev) = lines.last_mut() {
            prev.truncate(prev.trim_end().len());
            prev.push_str(bare);
            changed = true;
        } else {
            lines.push(line.to_string());
        }
    }
    changed.then(|| lines.join("\n"))
}

/// Tokens of 16 to 4096 bytes with the byte offset each starts at (at most 256 of them).
pub(super) fn long_tokens(text: &str) -> Vec<(usize, &str)> {
    let sep = |c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '<' | '>' | '[' | ']'
            )
    };
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        match (sep(c), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                let t = &text[s..i];
                if (16..=4096).contains(&t.len()) {
                    out.push((s, t));
                }
                start = None;
            }
            _ => {}
        }
    }
    out.truncate(256);
    out
}

pub(super) fn printable(bytes: Vec<u8>) -> Option<String> {
    let text = String::from_utf8(bytes).ok()?;
    (text.len() >= 12
        && text
            .chars()
            .all(|c| c == '\n' || c == '\t' || (' '..='~').contains(&c)))
    .then_some(text)
}

pub(super) fn base64_decode(v: &str) -> Option<Vec<u8>> {
    let v = v.trim_end_matches('=');
    if v.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(v.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in v.chars() {
        let d = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' | '-' => 62,
            '/' | '_' => 63,
            _ => return None,
        };
        acc = (acc << 6) | d;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

pub(super) fn hex_decode(v: &str) -> Option<Vec<u8>> {
    if !v.len().is_multiple_of(2) || !v.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    (0..v.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&v[i..i + 2], 16).ok())
        .collect()
}

/// A long token that is base64 or hex of a credential, or a credential written backwards: the token is swapped
/// for its decoding in place (so a label before it still counts) and matched as plain text.
fn decoded_kind(text: &str) -> Option<&'static str> {
    for (at, tok) in long_tokens(text) {
        let decodings = [
            base64_decode(tok).and_then(printable),
            hex_decode(tok).and_then(printable),
        ];
        for decoded in decodings.into_iter().flatten() {
            let variant = format!("{}{}{}", &text[..at], decoded, &text[at + tok.len()..]);
            if let Some(kind) = plain_kind(&variant) {
                return Some(kind);
            }
        }
        // Backwards: only a well known shape counts, never a label (the words around the token are not reversed).
        let reversed: String = tok.trim_end_matches('=').chars().rev().collect();
        if let Some(kind) = shape_kind(&reversed) {
            return Some(kind);
        }
    }
    None
}

/// Field names that hold a secret in JSON, YAML and env files.
const SECRET_FIELDS: [&str; 8] = [
    "refresh_token",
    "access_token",
    "id_token",
    "auth_token",
    "session_token",
    "bearer_token",
    "client_secret",
    "secret_key",
];

/// `"refresh_token":"VALUE"`, `access_token=VALUE` or `id_token: VALUE` with a long value that has a digit and is
/// not a placeholder. The field name alone (`"grant_type":"refresh_token"`) or prose about it is not a match.
fn secret_field(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    SECRET_FIELDS.iter().any(|field| {
        lower.match_indices(field).any(|(i, _)| {
            if lower[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphabetic())
            {
                return false;
            }
            let rest = text[i + field.len()..].trim_start_matches(['"', '\'', '`']);
            let rest = rest.trim_start_matches([' ', '\t']);
            let Some(rest) = rest.strip_prefix([':', '=']) else {
                return false;
            };
            let value: String = rest
                .trim_start_matches([' ', '\t', '"', '\'', '`'])
                .chars()
                .take_while(|c| value_char(*c))
                .collect();
            value.len() >= 20 && value.chars().any(|c| c.is_ascii_digit()) && !placeholder(&value)
        })
    })
}

/// A credential that starts with a fixed prefix and then runs for at least `min` characters of a class.
struct Shape {
    kind: &'static str,
    prefix: &'static str,
    min: usize,
    class: fn(char) -> bool,
}

fn alnum(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

fn upper_alnum(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit()
}

fn token(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-')
}

const SHAPES: [Shape; 14] = [
    Shape {
        kind: "an npm token",
        prefix: "npm_",
        min: 36,
        class: alnum,
    },
    Shape {
        kind: "an AWS access key",
        prefix: "AKIA",
        min: 16,
        class: upper_alnum,
    },
    Shape {
        kind: "an AWS access key",
        prefix: "ASIA",
        min: 16,
        class: upper_alnum,
    },
    Shape {
        kind: "a GitHub token",
        prefix: "ghp_",
        min: 30,
        class: alnum,
    },
    Shape {
        kind: "a GitHub token",
        prefix: "gho_",
        min: 30,
        class: alnum,
    },
    Shape {
        kind: "a GitHub token",
        prefix: "ghu_",
        min: 30,
        class: alnum,
    },
    Shape {
        kind: "a GitHub token",
        prefix: "ghs_",
        min: 30,
        class: alnum,
    },
    Shape {
        kind: "a GitHub token",
        prefix: "github_pat_",
        min: 22,
        class: token,
    },
    Shape {
        kind: "a Slack token",
        prefix: "xoxb-",
        min: 10,
        class: token,
    },
    Shape {
        kind: "a Slack token",
        prefix: "xoxp-",
        min: 10,
        class: token,
    },
    Shape {
        kind: "an API key",
        prefix: "sk-",
        min: 24,
        class: token,
    },
    Shape {
        kind: "a Stripe key",
        prefix: "sk_live_",
        min: 16,
        class: alnum,
    },
    Shape {
        kind: "a Stripe key",
        prefix: "rk_live_",
        min: 16,
        class: alnum,
    },
    Shape {
        kind: "a Google API key",
        prefix: "AIza",
        min: 35,
        class: token,
    },
];

impl Shape {
    fn matches(&self, text: &str) -> bool {
        text.match_indices(self.prefix).any(|(i, _)| {
            let standalone = text[..i].chars().next_back().is_none_or(|c| !alnum(c));
            let run = text[i + self.prefix.len()..]
                .chars()
                .take_while(|c| (self.class)(*c))
                .count();
            standalone && run >= self.min
        })
    }
}

/// `Authorization: Bearer <token>` or `Bearer <token>` with a long token.
fn bearer_token(text: &str) -> bool {
    text.match_indices("Bearer ").any(|(i, _)| {
        let run = text[i + 7..]
            .chars()
            .take_while(|c| token(*c) || matches!(c, '.' | '~' | '+' | '/' | '='))
            .count();
        run >= 20
    })
}

/// Three dot separated base64url parts, the first starting with the encoded `{"`: a JSON web token.
fn json_web_token(text: &str) -> bool {
    text.split(|c: char| !(token(c) || c == '.')).any(|word| {
        let parts: Vec<&str> = word.split('.').collect();
        parts.len() == 3 && word.starts_with("eyJ") && parts.iter().all(|p| p.len() >= 8)
    })
}

/// A value that is plainly a placeholder or a reference rather than a secret.
fn placeholder(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    value.starts_with(['<', '{', '$', '*', '[', '(', '.', '>', '}'])
        || (value.starts_with('%') && (value.ends_with('%') || value.len() < 3))
        || value
            .chars()
            .all(|c| matches!(c, '*' | 'x' | 'X' | '.' | '#'))
        || matches!(
            lower.as_str(),
            "pass" | "password" | "secret" | "passwd" | "changeme" | "redacted"
        )
}

fn trim_value(word: &str) -> &str {
    word.trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | ',' | ';' | ')' | '(' | '.'))
}

/// A password in a connection URL (`scheme://user:PASSWORD@host`), after `-u user:PASSWORD` or `--user` of a
/// curl command, in `mysql -pPASSWORD`, or in `--password=PASSWORD`.
fn url_or_command_password(text: &str) -> bool {
    if url_password(text) {
        return true;
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    curl_user_password(text, &words) || mysql_flag_password(text, &words) || password_flag(&words)
}

/// A password that is long enough and not a placeholder.
fn real_password(p: &str) -> bool {
    p.len() >= 3 && !placeholder(p)
}

/// `scheme://user:PASSWORD@host` anywhere in the text.
fn url_password(text: &str) -> bool {
    text.match_indices("://").any(|(i, _)| {
        let rest = &text[i + 3..];
        let authority = rest
            .split(|c: char| matches!(c, '/' | '?' | '#') || c.is_whitespace())
            .next()
            .unwrap_or("");
        authority
            .rsplit_once('@')
            .and_then(|(info, _)| info.split_once(':'))
            .is_some_and(|(_, pw)| real_password(pw))
    })
}

/// `-u user:PASSWORD` or `--user user:PASSWORD` in a text that mentions curl.
fn curl_user_password(text: &str, words: &[&str]) -> bool {
    text.contains("curl")
        && words.windows(2).any(|w| {
            matches!(w[0], "-u" | "--user")
                && w[1]
                    .split_once(':')
                    .is_some_and(|(_, pw)| real_password(trim_value(pw)))
        })
}

/// `-pPASSWORD` (four characters or more) in a text that mentions mysql.
fn mysql_flag_password(text: &str, words: &[&str]) -> bool {
    text.contains("mysql")
        && words.iter().any(|w| {
            w.strip_prefix("-p").is_some_and(|pw| {
                !pw.starts_with('-') && pw.len() >= 4 && real_password(trim_value(pw))
            })
        })
}

/// `--password=PASSWORD`.
fn password_flag(words: &[&str]) -> bool {
    words.iter().any(|w| {
        w.strip_prefix("--password=")
            .is_some_and(|pw| real_password(trim_value(pw)))
    })
}

/// A value that reads as a secret after a label: long enough, not a placeholder, and with a digit or a symbol
/// (so a hyphenated name or a path such as `billing-db` or `secret/prod/db` is not one). A URL is not a value
/// either: `url_or_command_password` judges the password inside it.
fn secret_looking(value: &str) -> bool {
    value.len() >= 8
        && !value.contains("://")
        && !placeholder(value)
        && value
            .chars()
            .any(|c| c.is_ascii_digit() || "!#%^&*+=~@$".contains(c))
}

/// Words that label a secret in prose, configuration and logs.
const LABELS: [&str; 8] = [
    "pass",
    "password",
    "passphrase",
    "passwd",
    "passwort",
    "pw",
    "sessionid",
    "secret",
];

/// Labels after which only a long base64 value counts (a bare word like `token` is too common in prose).
const BLOB_LABELS: [&str; 3] = ["token", "apikey", "api_key"];

/// The label starts a word (an `_` or `.` before it is fine) and is not the front of a longer word or a compound
/// name (`secret-marker-7731`: a hyphen right after it is not a separator).
fn whole_label(lower: &str, at: usize, len: usize) -> bool {
    let before = lower[..at].chars().next_back();
    let after = lower[at + len..].chars().next();
    !before.is_some_and(|c| c.is_ascii_alphabetic())
        && !after.is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '@' | '-'))
}

/// The word right after a label and its separator (`:`, `=`, spaces, an opening quote).
fn word_after_label(rest: &str) -> &str {
    let first = rest
        .trim_start_matches([':', '=', ' ', '\t', '|', '"', '\'', '`'])
        .split_whitespace()
        .next()
        .unwrap_or("");
    trim_value(first)
}

/// `{{ ... }}` template spans (a placeholder such as `{{fake:password|pct}}`) replaced by `{{}}`, so a label word
/// inside one is not read as a label.
fn blank_templates(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        match rest[open..].find("}}") {
            Some(len) => {
                out.push_str(&rest[..open]);
                out.push_str("{{}}");
                rest = &rest[open + len + 2..];
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

/// A secret after a label such as `password`, `pw:`, `passphrase is`, `sessionid=` or `password = "..."`,
/// or a long base64 value after `token`.
fn labelled_secret(text: &str) -> bool {
    let text = &blank_templates(text);
    let lower = text.to_ascii_lowercase();
    let plain = LABELS.iter().any(|label| {
        lower
            .match_indices(label)
            .filter(|(i, _)| whole_label(&lower, *i, label.len()))
            .any(|(i, _)| {
                // Only the rest of the label's own line: `kind: Secret` in YAML is not the label of a later line.
                let rest = text[i + label.len()..].lines().next().unwrap_or("");
                if secret_looking(word_after_label(rest)) {
                    return true;
                }
                // `password for the vault until Friday: X`, `passphrase is X`, `Passwort ... ist X`.
                let words: Vec<&str> = rest.split_whitespace().take(14).collect();
                words.windows(2).any(|w| {
                    let k = w[0].to_ascii_lowercase();
                    let marks = k.ends_with(':') || matches!(k.as_str(), "is" | "ist" | "=" | ":=");
                    marks && secret_looking(trim_value(w[1]))
                })
            })
    });
    plain
        || BLOB_LABELS.iter().any(|label| {
            lower
                .match_indices(label)
                .filter(|(i, _)| whole_label(&lower, *i, label.len()))
                .any(|(i, _)| long_base64(word_after_label(&text[i + label.len()..])))
        })
}

/// At least 40 characters of base64 with upper case, lower case and a digit: an encoded secret value.
fn long_base64(v: &str) -> bool {
    v.len() >= 40
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
        && v.chars().any(|c| c.is_ascii_uppercase())
        && v.chars().any(|c| c.is_ascii_lowercase())
        && v.chars().any(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_credential_shapes_are_recognised_by_kind() {
        let cases = [
            ("-----BEGIN OPENSSH PRIVATE KEY-----\nabc", "a private key"),
            ("key AKIAIOSFODNN7EXAMPLE here", "an AWS access key"),
            (
                "token ghp_abcdefghijklmnopqrstuvwxyz0123456789",
                "a GitHub token",
            ),
            ("github_pat_11ABCDEFG0abcdefghijklmnop", "a GitHub token"),
            ("xoxb-1234567890-abcdefghij", "a Slack token"),
            ("use sk-abcdefghijklmnopqrstuvwxyz123456", "an API key"),
            ("sk-ant-api03-abcdefghijklmnopqrstuvwx", "an API key"),
            ("sk_live_abcdefghijklmnop1234", "a Stripe key"),
            (
                "AIzaSyA-abcdefghijklmnopqrstuvwxyz012345",
                "a Google API key",
            ),
            (
                "Authorization: Bearer abcdefghijklmnopqrstuvwxyz",
                "a bearer token",
            ),
            (
                "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghijklmnop",
                "a JSON web token",
            ),
        ];
        for (text, kind) in cases {
            assert_eq!(credential_kind(text), Some(kind), "{text:?}");
        }
    }

    #[test]
    fn ordinary_text_hashes_and_identifiers_are_not_credentials() {
        let cases = [
            "seeking: a slugify helper",
            "see commit 3f9c1d2a8b7e6f5d4c3b2a1908f7e6d5c4b3a291",
            "task-abcdefghijklmnopqrstuvwxyz12345",
            "the sk- prefix is short here: sk-abc",
            "AKIA is the prefix of an AWS key",
            "a Bearer of bad news",
            "eyJ is how base64 json starts",
            "id 123e4567-e89b-12d3-a456-426614174000",
            "",
        ];
        for text in cases {
            assert_eq!(credential_kind(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_match_names_the_kind_and_never_the_text() {
        let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        let kind = credential_kind(secret).unwrap();
        assert!(!kind.contains("ghp_") && !kind.contains("abcdef"));
    }

    #[test]
    fn half_a_private_key_marker_is_not_a_private_key() {
        for text in [
            "-----BEGIN PUBLIC KEY-----\nMFkw",
            "-----BEGIN CERTIFICATE-----",
            "ends with PRIVATE KEY-----",
        ] {
            let kind = credential_kind(text);
            assert_ne!(kind, Some("a private key"), "{text:?}");
        }
        assert_eq!(
            credential_kind("-----BEGIN RSA PRIVATE KEY-----"),
            Some("a private key")
        );
    }

    #[test]
    fn only_three_parts_starting_with_eyj_make_a_json_web_token() {
        let part = "abcdefghijkl";
        let cases = [
            (format!("eyJhbGciOi.{part}.{part}"), true),
            (format!("xyJhbGciOi.{part}.{part}"), false),
            (format!("eyJhbGciOi.{part}"), false),
            (format!("eyJhbGciOi.{part}.{part}.{part}"), false),
        ];
        for (text, jwt) in cases {
            assert_eq!(json_web_token(&text), jwt, "{text:?}");
        }
    }

    /// Obviously fake values built at runtime so no scanner sees a literal credential in the source.
    fn fake_password() -> String {
        ["Zq8", "rT2!", "vK9m", "_Lw4x"].concat()
    }

    #[test]
    fn a_password_inside_a_url_or_a_command_is_recognised() {
        let pw = fake_password();
        let cases = [
            format!("the db url is postgres://app:{pw}@db.internal.example:5432/app"),
            format!("redis is at redis://default:{pw}@cache.example.net:6379"),
            format!("Error: auth failed for postgres://svc:{pw}@10.0.3.4:5432/main"),
            format!("the failing curl:\ncurl -u deploy:{pw} https://artifacts.example.com/upload"),
            format!("curl --user deploy:{pw} https://x.example.com"),
            format!("mysql -h db.example.org -u root -p{pw} appdb"),
            format!("psql --password={pw} app"),
            "postgresql://reporter:%5A%71%38%72%54%32@203.0.113.40:5432/reports".to_string(),
        ];
        for text in cases {
            assert_eq!(credential_kind(&text), Some("a password"), "{text:?}");
        }
    }

    #[test]
    fn urls_and_commands_without_a_real_password_pass() {
        let cases = [
            "DATABASE_URL=postgres://localhost/app_test (no password, local dev only)",
            "postgres://user@host/db",
            "postgres://user:@host/db",
            "postgres://user:<password>@host/db",
            "postgres://user:${DB_PASSWORD}@host/db",
            "postgres://user:********@host/db",
            "postgres://user:password@host/db",
            "see https://example.com:8080/path for it",
            "curl -u deploy https://x.example.com",
            "curl -u deploy:$TOKEN https://x.example.com",
            "mysql -h db.example.org -u root -p appdb",
            "mysql -p",
            "run mysql -u root -p and type it at the prompt",
        ];
        for text in cases {
            assert_eq!(credential_kind(text), None, "{text:?}");
        }
    }

    #[test]
    fn an_npm_token_is_recognised() {
        let token = format!("npm_{}", "a1B2".repeat(9));
        assert_eq!(credential_kind(&token), Some("an npm token"));
        let line = format!("//registry.npmjs.org/:_authToken={token}");
        assert_eq!(credential_kind(&line), Some("an npm token"));
        assert_eq!(credential_kind("npm_config_cache is set"), None);
        assert_eq!(credential_kind("npm_short1234"), None);
    }

    #[test]
    fn a_keyword_labelled_secret_is_recognised() {
        let pw = fake_password();
        let hex = "9f2c".repeat(8);
        let cases = [
            format!("admin login for the staging dashboard: user ops, password {pw}"),
            format!("temporary password for the shared vault until Friday: {pw}"),
            format!("ssh in with the key I put at /tmp/k, passphrase is {pw}"),
            format!("pw: {pw}"),
            format!("terraform plan output:\n  + password = \"{pw}\"\n  + username = \"admin\""),
            format!("cookie for the admin session: sessionid={hex}; it's valid for a day"),
            format!("webhook signing secret: {hex}"),
            format!("Das Passwort für die Staging-Datenbank ist {pw}"),
            format!("DB_PASSWORD={pw}"),
            format!("PASSWORD: '{pw}'"),
            format!("| DB_PASSWORD | {pw} | prod |"),
            format!("[smtp]\nuser = \"alerts\"\npass = \"{pw}\""),
            format!(
                "webhook signing secret for the payments endpoint (we verify the HMAC with it): {hex}"
            ),
        ];
        for text in cases {
            assert_eq!(
                credential_kind(&text),
                Some("a labelled secret"),
                "{text:?}"
            );
        }
    }

    #[test]
    fn talk_about_passwords_without_one_passes() {
        let cases = [
            "password: ******** (masked in the log, as it should be)",
            "the password is required",
            "the password is in the vault",
            "password: <your password here>",
            "password = \"${DB_PASSWORD}\"",
            "password {{fake:password}}",
            "export GITHUB_TOKEN=${{ secrets.GITHUB_TOKEN }} in the workflow",
            "the TOTP secret goes in the authenticator app, never in chat",
            "rotate the passphrase every year",
            "pw: ...",
            "password reset link expired",
            "zq9-secret-marker-7731",
            "dsn = \"https://{{fake:hex32}}@o4508.ingest.example.io/77\"",
            "kind: Secret\nmetadata:\n  name: x\ndata:\n  username: YmlsbGluZw==\n",
            "the password is URL-encoded: postgresql://u:{{fake:password|pct}}@203.0.113.40:5432/db",
            "see client-secret-rotation-2026 for the plan",
            "the pass-through-2026 route",
            "password_hash: column added",
            "the token lives in the keychain under the item klatch-store; never paste it here",
            "test vector: HMAC-SHA256 of 'abc' with key 'key' is 9c196e32dc0175f86f4b1cb89289d6619de6bee699e4c378e68309ed97a1a6ab",
            "the release tarball sha256 is 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
            "the base64 of the test fixture is iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==; it's just the PNG header",
        ];
        for text in cases {
            assert_eq!(credential_kind(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_long_base64_value_labelled_token_or_secret_is_recognised() {
        let blob = format!("Qm{}", "x7Kp9Zr3Ld5Wn1Vb8Tc2Hg4Jf6Sa0Ye+/".repeat(2));
        for text in [
            format!(
                "kubectl get secret app -o yaml shows data.token: {blob} which decodes to the key"
            ),
            format!("secret = {blob}"),
        ] {
            assert_eq!(
                credential_kind(&text),
                Some("a labelled secret"),
                "{text:?}"
            );
        }
        assert_eq!(credential_kind("token: abcdefgh"), None);
    }

    // Mechanical decodings (#987): the same secret, wrapped so a plain match misses it.

    fn fake_github() -> String {
        format!("ghp_{}", "a1B2c3D4e5".repeat(4))
    }

    fn b64(bytes: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |a, (i, b)| a | (u32::from(*b) << (16 - 8 * i)));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn grouped(text: &str, size: usize, sep: &str) -> String {
        text.chars()
            .collect::<Vec<_>>()
            .chunks(size)
            .map(|c| c.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join(sep)
    }

    #[test]
    fn a_wrapped_credential_is_recognised() {
        let gh = fake_github();
        let stripe = format!("sk_live_{}", "Q7wE3rT9yU1i".repeat(2));
        let hex: String = stripe.bytes().map(|b| format!("{b:02x}")).collect();
        let reversed: String = gh.chars().rev().collect();
        let cut = gh.len() / 2;
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghijklmnop";
        let cases = [
            format!("paste it base64'd:\n{}\ndecode it", b64(gh.as_bytes())),
            format!("config value (hex): {hex}"),
            format!("written backwards: {reversed}"),
            format!(
                "new key, strip the invisible characters: {}",
                grouped(&gh, 3, "\u{200b}")
            ),
            format!("remove the spaces: {}", grouped(&gh, 4, " ")),
            format!("first half: {}\n{}", &gh[..cut], &gh[cut..]),
            format!("{}\n{}\n{}", &jwt[..30], &jwt[30..50], &jwt[50..]),
        ];
        for text in cases {
            assert!(credential_kind(&text).is_some(), "{text:?}");
        }
    }

    #[test]
    fn a_spelled_out_secret_is_recognised() {
        for pw in [fake_password(), "xOqZyFz*#OywdstkLm".to_string()] {
            let text = format!("reading it out: {}", grouped(&pw, 1, " "));
            assert!(credential_kind(&text).is_some(), "{text:?}");
        }
    }

    #[test]
    fn a_json_secret_field_is_recognised() {
        let tok = "Zq8rT2vK9mLw4xP7nB3cD5eF";
        let cases = [
            format!("body {{\"grant_type\":\"refresh_token\",\"refresh_token\":\"{tok}\"}}"),
            format!("{{\"access_token\": \"{tok}\", \"expires_in\": 3600}}"),
            format!("client_secret={tok}"),
            format!("id_token: {tok}"),
        ];
        for text in cases {
            assert_eq!(
                credential_kind(&text),
                Some("a labelled secret"),
                "{text:?}"
            );
        }
    }

    #[test]
    fn decodings_do_not_flag_ordinary_text() {
        let cases = [
            "the grant_type is refresh_token and the field is rotated hourly",
            "{\"grant_type\":\"refresh_token\",\"scope\":\"read\"}",
            "refresh_token: <your refresh token here>",
            "access_token=${ACCESS_TOKEN}",
            "client_secret: ********************",
            "a b c d e f g h i j k l m n o p",
            "1 2 3 4 5 6 7 8 9 10 11 12 13 14",
            "I have a b c plans, and so on and so forth, it is a to do",
            "aGVsbG8gd29ybGQgaGVsbG8gd29ybGQ= is just hello world twice",
            "the png header iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg== ok",
            "sha256 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
            "the file internationalization-configuration.yaml is long",
            "line one is a long sentence without any\nsecond line short\nthird",
        ];
        for text in cases {
            assert_eq!(credential_kind(text), None, "{text:?}");
        }
    }

    // Characterisation tests for the helpers #1446 refactors: they pass before and after (a pure refactor has no
    // red test).

    /// The joined values of the spaced-out runs in `text`, with their widths.
    fn runs(text: &str) -> Vec<(usize, String)> {
        spaced_runs(text)
            .into_iter()
            .map(|(_, w, j)| (w, j))
            .collect()
    }

    #[test]
    fn a_spaced_run_is_four_or_more_equal_words_with_a_shorter_last_and_twelve_characters() {
        // equal widths, the last one shorter
        assert_eq!(
            runs("ab cd ef gh ij kl mn o"),
            [(2, "abcdefghijklmno".to_string())]
        );
        // fewer than four words, or fewer than twelve characters joined: no run
        assert!(runs("abcd efgh ijkl").is_empty());
        assert!(runs("ab cd ef gh ij").is_empty());
        // a word wider than eight characters never starts a run
        assert!(runs("abcdefghi abcdefghi abcdefghi abcdefghi").is_empty());
        // a double space breaks the run; a longer last word is left out of it
        assert!(runs("ab cd  ef gh ij kl mn").is_empty());
        assert_eq!(
            runs("ab cd ef gh ij kl mnp"),
            [(2, "abcdefghijkl".to_string())]
        );
    }

    #[test]
    fn a_spaced_run_reports_its_byte_range_and_a_second_run_after_the_first() {
        let text = "x ab cd ef gh ij kl mn yyy";
        let found = spaced_runs(text);
        assert_eq!(found.len(), 1);
        assert_eq!(&text[found[0].0.clone()], "ab cd ef gh ij kl mn");
        assert_eq!(joined_spaces(text).as_deref(), Some("x abcdefghijklmn yyy"));
        let twice = "ab cd ef gh ij kl mn then 12 34 56 78 90 ab cd";
        assert_eq!(runs(twice).len(), 2);
    }

    #[test]
    fn a_command_password_must_be_real_and_long_enough() {
        let flagged = |t: &str| url_or_command_password(t);
        // the length floors: three characters in a url or after curl -u, four after mysql -p
        assert!(flagged("curl -u me:abc https://x.example"));
        assert!(!flagged("curl -u me:ab https://x.example"));
        assert!(flagged("mysql -u root -pabcd db"));
        assert!(!flagged("mysql -u root -pabc db"));
        assert!(!flagged("mysql -u root -p-abcd db"));
        assert!(flagged("psql --password=abc app"));
        assert!(!flagged("psql --password=ab app"));
        // the tool word has to be in the text
        assert!(!flagged("wget -u me:abcdef https://x.example"));
        assert!(!flagged("psql -pabcdef app"));
        // trailing punctuation is not part of the password
        assert!(!flagged("curl -u me:pass. https://x.example"));
        assert!(flagged("(curl -u me:s3cr3t) now"));
    }

    #[test]
    fn a_url_password_is_read_from_the_authority_only() {
        let flagged = |t: &str| url_or_command_password(t);
        assert!(flagged("db at postgres://u:s3cr3t@h/db now"));
        assert!(!flagged("db at postgres://h/path:s3cr3t@x"));
        assert!(!flagged("see postgres://u@h:5432/db"));
        assert!(flagged("a://b a://u:s3cr3t@h"));
    }
}
