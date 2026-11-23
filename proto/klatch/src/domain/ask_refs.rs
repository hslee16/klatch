//! References an ask carries to artifacts outside klatch, and to other klatch objects (#1318): a ticket, a pull
//! request, a commit, a path, a log, a tool, a message, a room. Pure and without I/O.
//!
//! The rules, all of them about what klatch does NOT do:
//! - klatch never fetches, opens, runs or resolves a ref. A `tool`, `mcp-tool` or `command` ref is a name or a line of
//!   text for a person or an agent to act on, never a thing klatch executes. The binary stays network-free and
//!   credential-free.
//! - A ref is data, not an instruction. It is shown as inert text: control characters are escaped, line breaks are
//!   folded, and a ref is always shown as the asker's, never in the voice of klatch.
//! - A ref carries no credential: a URL with userinfo or a token-looking query is refused here; the text of every
//!   field also goes through the same secret detector as message text (in the gateway).
//! - A ref never implies a scope; only the declared scope of an ask (#1171) does.
//! - Refusing, not truncating: a ref over a cap is refused whole, and nothing is cut silently.
//!
//! The kind is open: any token of `[a-z0-9_.-]{1,32}`. A kind klatch knows gets a check of its value; an unknown kind
//! is accepted as opaque text, so a new artifact type needs no wire change.

/// Most refs one ask may carry.
pub const MAX_REFS: usize = 32;
/// Longest value, in bytes.
pub const MAX_VALUE_BYTES: usize = 1024;
/// Longest label, in bytes.
pub const MAX_LABEL_BYTES: usize = 120;
/// Longest inline snippet, in bytes: a few KB of text, such as the failing output or the relevant lines.
pub const MAX_SNIPPET_BYTES: usize = 8192;
/// Largest sum of every kind, value, label and snippet of one ask, in bytes.
pub const MAX_TOTAL_BYTES: usize = 65_536;
/// Longest kind token.
pub const MAX_KIND_CHARS: usize = 32;

/// The kinds klatch knows and checks the value of. Any other kind is accepted as opaque text.
pub const KNOWN_KINDS: [&str; 23] = [
    "ticket", "pr", "commit", "branch", "tag", "release", "path", "diff", "test", "log", "ci-run",
    "doc", "url", "tool", "mcp-tool", "command", "dataset", "message", "ask", "room", "run",
    "member", "other",
];

/// One reference on an ask.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AskRef {
    /// An open-ended token of `[a-z0-9_.-]{1,32}`.
    pub kind: String,
    pub value: String,
    /// A short human label, optional.
    pub label: Option<String>,
    /// Bounded inline text that gives context without fetching anything, optional. Data, never an instruction.
    pub snippet: Option<String>,
}

/// Why a set of refs was refused: a plain reason, or a credential (so the gateway can answer with the credential
/// rule). Neither carries the offending content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefProblem {
    Invalid(String),
    Credential(String),
}

/// Whether `kind` is a ref kind token: 1 to 32 characters of `a-z 0-9 _ . -`.
pub fn valid_ref_kind(kind: &str) -> bool {
    (1..=MAX_KIND_CHARS).contains(&kind.len())
        && kind.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'.' | b'-')
        })
}

fn bad(why: String) -> RefProblem {
    RefProblem::Invalid(why)
}

/// Check every ref of one ask: the count, each kind, value, label and snippet, and the total size. The first
/// problem found is returned; nothing is changed.
pub fn check_refs(refs: &[AskRef]) -> Result<(), RefProblem> {
    if refs.len() > MAX_REFS {
        return Err(bad(format!(
            "an ask carries at most {MAX_REFS} refs and this one has {}; nothing was cut, send fewer",
            refs.len()
        )));
    }
    let mut total = 0usize;
    for (i, r) in refs.iter().enumerate() {
        let n = i + 1;
        if !valid_ref_kind(&r.kind) {
            return Err(bad(format!(
                "ref {n}: bad kind: 1 to {MAX_KIND_CHARS} characters of a-z 0-9 _ . -"
            )));
        }
        check_value(n, r)?;
        check_label(n, r)?;
        check_snippet(n, r)?;
        for text in [
            Some(r.value.as_str()),
            r.label.as_deref(),
            r.snippet.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            url_credential(text).map_or(Ok(()), |why| {
                Err(RefProblem::Credential(format!(
                    "ref {n} carries a credential ({why}); {}",
                    "remove it and say where it is kept instead"
                )))
            })?;
        }
        check_kind_value(n, r)?;
        total += r.kind.len()
            + r.value.len()
            + r.label.as_ref().map_or(0, String::len)
            + r.snippet.as_ref().map_or(0, String::len);
    }
    if total > MAX_TOTAL_BYTES {
        return Err(bad(format!(
            "the refs total {total} bytes and the most an ask carries is {MAX_TOTAL_BYTES}; nothing was cut, send less"
        )));
    }
    Ok(())
}

fn check_value(n: usize, r: &AskRef) -> Result<(), RefProblem> {
    if r.value.trim().is_empty() {
        return Err(bad(format!("ref {n}: the value is empty")));
    }
    if r.value.len() > MAX_VALUE_BYTES {
        return Err(bad(format!(
            "ref {n}: the value is over {MAX_VALUE_BYTES} bytes; nothing was cut, shorten it or put the text in a snippet"
        )));
    }
    if r.value.chars().any(char::is_control) {
        return Err(bad(format!(
            "ref {n}: the value has a control character; it is one line of text"
        )));
    }
    Ok(())
}

fn check_label(n: usize, r: &AskRef) -> Result<(), RefProblem> {
    let Some(label) = &r.label else {
        return Ok(());
    };
    if label.trim().is_empty() {
        return Err(bad(format!("ref {n}: the label is empty")));
    }
    if label.len() > MAX_LABEL_BYTES {
        return Err(bad(format!(
            "ref {n}: the label is over {MAX_LABEL_BYTES} bytes; nothing was cut, shorten it"
        )));
    }
    if label.chars().any(char::is_control) {
        return Err(bad(format!(
            "ref {n}: the label has a control character; it is one line of text"
        )));
    }
    Ok(())
}

fn check_snippet(n: usize, r: &AskRef) -> Result<(), RefProblem> {
    let Some(snippet) = &r.snippet else {
        return Ok(());
    };
    if snippet.is_empty() {
        return Err(bad(format!("ref {n}: the snippet is empty")));
    }
    if snippet.len() > MAX_SNIPPET_BYTES {
        return Err(bad(format!(
            "ref {n}: the snippet is over {MAX_SNIPPET_BYTES} bytes; nothing was cut, send the relevant part"
        )));
    }
    Ok(())
}

/// The value is shaped as its known kind says. A kind klatch does not know is opaque text and always passes.
fn check_kind_value(n: usize, r: &AskRef) -> Result<(), RefProblem> {
    let v = r.value.as_str();
    let ok = match r.kind.as_str() {
        "pr" => is_http_url(v) || issue_number(v),
        "ticket" => !v.contains(char::is_whitespace),
        "commit" => is_http_url(v) || commit_id(v),
        "path" => relative_path(v),
        "url" => is_http_url(v),
        "message" | "run" => opaque_id(v),
        "ask" => ask_ref(v),
        "room" | "member" => crate::domain::valid_name(v),
        _ => true,
    };
    if ok {
        return Ok(());
    }
    let want = match r.kind.as_str() {
        "pr" => "#N, owner/repo#N or an http(s) URL",
        "commit" => "7 to 64 hex digits, optionally `repo@` first, or an http(s) URL",
        "path" => {
            "a relative, cleaned path (no leading /, no `.` or `..` segment), optionally `:N` or `:N-M`"
        }
        "url" => "an http or https URL",
        "message" | "run" => "an id of letters, digits and . _ : -",
        "ask" => "N or ROOM#N",
        "room" => "a room name",
        "member" => "a member name",
        _ => "one token with no spaces",
    };
    Err(bad(format!("ref {n}: a {} ref is {want}", r.kind)))
}

fn is_http_url(v: &str) -> bool {
    let lower = v.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    rest.is_some_and(|r| !r.is_empty() && !r.starts_with('/')) && !v.contains(char::is_whitespace)
}

/// `#N` or `owner/repo#N`.
fn issue_number(v: &str) -> bool {
    let Some((repo, n)) = v.rsplit_once('#') else {
        return false;
    };
    let n_ok = !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit());
    let repo_ok = repo.is_empty()
        || (repo.split('/').count() == 2
            && repo.split('/').all(|p| {
                !p.is_empty()
                    && p.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
            }));
    n_ok && repo_ok
}

/// 7 to 64 hex digits, optionally after `repo@`.
fn commit_id(v: &str) -> bool {
    let id = v.rsplit_once('@').map_or(v, |(_, id)| id);
    (7..=64).contains(&id.len()) && id.bytes().all(|c| c.is_ascii_hexdigit())
}

/// A line range suffix: `10`, `10-20`, never `-5` or `5-`.
fn is_line_range(range: &str) -> bool {
    !range.is_empty()
        && range.bytes().all(|c| c.is_ascii_digit() || c == b'-')
        && !range.starts_with('-')
        && !range.ends_with('-')
}

/// `v` without a trailing `:LINES` range, when it has one.
fn without_line_range(v: &str) -> &str {
    match v.rsplit_once(':') {
        Some((p, range)) if is_line_range(range) => p,
        _ => v,
    }
}

fn plain_segments(path: &str) -> bool {
    let body = path.strip_suffix('/').unwrap_or(path);
    !body.is_empty()
        && body
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

fn relative_path(v: &str) -> bool {
    let path = without_line_range(v);
    let rooted = path.is_empty() || path.starts_with('/') || path.contains('\\');
    !rooted && plain_segments(path)
}

fn opaque_id(v: &str) -> bool {
    (1..=128).contains(&v.len())
        && v.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b':' | b'-'))
}

/// `N` (an ask of this room) or `ROOM#N`.
fn ask_ref(v: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.len() <= 18 && s.bytes().all(|c| c.is_ascii_digit());
    match v.split_once('#') {
        Some((room, n)) => crate::domain::valid_name(room) && digits(n),
        None => digits(v),
    }
}

/// The room a `room` or `ask` ref names, when it names one: `lobby` for `room lobby` and `ask lobby#7`. An `ask` with
/// no room names the ask's own room, which is not another room, so it is `None`.
pub fn named_room(kind: &str, value: &str) -> Option<String> {
    match kind {
        "room" => Some(value.to_string()),
        "ask" => value.split_once('#').map(|(room, _)| room.to_string()),
        _ => None,
    }
}

/// Why some URL in `text` carries a credential, in words that never repeat it; `None` when none does. Every
/// `scheme://` in the text is looked at, whatever the scheme: userinfo in the authority, a query or fragment
/// parameter with a name that holds secrets, or a value shaped like a token.
fn url_credential(text: &str) -> Option<&'static str> {
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let after = &rest[at + 3..];
        let end = after
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')'))
            .unwrap_or(after.len());
        if let Some(why) = credential_in_url(&after[..end]) {
            return Some(why);
        }
        rest = &after[end..];
    }
    None
}

/// Why one URL (the text after `://`) carries a credential, if it does.
fn credential_in_url(url: &str) -> Option<&'static str> {
    let authority_end = url.find(['/', '?', '#']).unwrap_or(url.len());
    if url[..authority_end].contains('@') {
        return Some("a user name or password in the URL");
    }
    let tail = &url[authority_end..];
    let query = tail
        .split_once('?')
        .map(|(_, q)| q.split_once('#').map_or(q, |(q, _)| q));
    let fragment = tail.split_once('#').map(|(_, f)| f);
    [query, fragment]
        .into_iter()
        .flatten()
        .find_map(secret_param)
}

/// Words that mark a query or fragment parameter name as holding a secret.
const SECRET_WORDS: [&str; 13] = [
    "token",
    "secret",
    "passw",
    "pwd",
    "apikey",
    "api_key",
    "api-key",
    "auth",
    "signature",
    "credential",
    "session",
    "jwt",
    "bearer",
];

fn key_names_a_secret(key: &str) -> bool {
    SECRET_WORDS.iter().any(|w| key.contains(w))
        || matches!(key, "key" | "sig" | "sid" | "code" | "password")
        || key.ends_with("_key")
        || key.ends_with("-key")
}

fn secret_param(params: &str) -> Option<&'static str> {
    for pair in params.split(['&', ';']) {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        if key_names_a_secret(&key.to_ascii_lowercase()) {
            return Some("a query value named like a secret");
        }
        if token_shaped(value) {
            return Some("a query value shaped like a token");
        }
    }
    None
}

/// 24 or more characters of a token alphabet with a letter and a digit in them, other than a plain git id.
fn token_shaped(value: &str) -> bool {
    if value.len() < 24
        || !value.bytes().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'+' | b'/' | b'=' | b'.' | b'%')
        })
    {
        return false;
    }
    let git_id = matches!(value.len(), 40 | 64) && value.bytes().all(|c| c.is_ascii_hexdigit());
    !git_id
        && value.bytes().any(|c| c.is_ascii_digit())
        && value.bytes().any(|c| c.is_ascii_alphabetic())
}

/// A character that cannot be shown as it is: a control character, or one that reorders, hides or breaks text.
fn unshowable(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{200b}'..='\u{200f}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{2069}'
                | '\u{feff}'
                | '\u{00ad}'
        )
}

/// The text of a ref as it is shown: control characters and bidirectional overrides escaped, nothing else changed.
/// A line break becomes the two characters `\n` unless `keep_lines` is set, which keeps `\n` and tab as they are.
pub fn inert(text: &str, keep_lines: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\n' if keep_lines => out.push('\n'),
            '\t' if keep_lines => out.push('\t'),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if unshowable(c) => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

impl AskRef {
    /// `kind value` and the label, on one line, inert: `pr o/r#12 (the fix)`.
    pub fn line(&self) -> String {
        let mut line = format!("{} {}", inert(&self.kind, false), inert(&self.value, false));
        if let Some(label) = &self.label {
            line.push_str(&format!(" ({})", inert(label, false)));
        }
        line
    }

    /// The text `klatch find` matches a query against: kind, value, label and snippet.
    pub fn searchable(&self) -> String {
        let mut parts = vec![self.kind.as_str(), self.value.as_str()];
        parts.extend(self.label.as_deref());
        parts.extend(self.snippet.as_deref());
        parts.join(" ")
    }
}

/// How many refs a message row names before it counts the rest.
const MARK_NAMED: usize = 3;
/// Longest value shown in a message row, in characters.
const MARK_VALUE_CHARS: usize = 60;

/// ` [refs pr o/r#12, ticket #4 +2]` for a one line message row: the first few refs and a count of the rest. Labels
/// and snippets are not in the row; `klatch read` shows them. Empty when there are none.
pub fn refs_mark(refs: &[AskRef]) -> String {
    if refs.is_empty() {
        return String::new();
    }
    let named: Vec<String> = refs
        .iter()
        .take(MARK_NAMED)
        .map(|r| {
            let value = inert(&r.value, false);
            let shown: String = if value.chars().count() > MARK_VALUE_CHARS {
                let cut: String = value.chars().take(MARK_VALUE_CHARS).collect();
                format!("{cut}...")
            } else {
                value
            };
            format!("{} {shown}", inert(&r.kind, false))
        })
        .collect();
    let rest = refs.len().saturating_sub(MARK_NAMED);
    let more = if rest > 0 {
        format!(" +{rest}")
    } else {
        String::new()
    };
    format!(" [refs {}{more}]", named.join(", "))
}

/// The lines shown under an ask in `read` and `wait`: each ref on its own line, a snippet's lines indented after it
/// with a `| ` margin, so a snippet cannot pass for a line of the log. Without snippets the ref line says how big
/// the snippet is.
pub fn ref_lines(refs: &[AskRef], with_snippets: bool) -> Vec<String> {
    let mut out = Vec::new();
    for r in refs {
        let mut line = format!("  ref {}", r.line());
        match (&r.snippet, with_snippets) {
            (Some(s), false) => {
                line.push_str(&format!(" [snippet {} bytes]", s.len()));
                out.push(line);
            }
            (Some(s), true) => {
                out.push(line);
                out.extend(s.lines().map(|l| format!("    | {}", inert(l, false))));
            }
            (None, _) => out.push(line),
        }
    }
    out
}

#[cfg(test)]
mod tests;
