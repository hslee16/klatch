//! The guard against a raw echo of a caller's text in an error (#1768).
//!
//! It reads the source (not the types), so it is deliberately strict: in the scanned directories, every value that
//! reaches the text of an error must pass through `shown(..)`, or be a number or constant, or be listed in `ALLOWED`
//! with the reason it is safe. An entry is keyed by the file, the exact format string and the exact argument, so a
//! second raw echo of the same name in the same file, with another message, is not covered by it, and a site that was
//! fixed is never listed.
//!
//! What it reads: the source is tokenised (strings with a `;` or a `{` in them stay one token, comments are gone,
//! balanced parentheses are followed), `#[cfg(test)]` items are skipped as items (not the rest of the file), and the
//! error sinks are the constructors and helpers that make an error text: `Error::Invalid`, `NotFound`, `Backend`,
//! `Busy` and `Bad`, `fail_with`, `rpc_error`, `invalid`, `refused`, `Err(..)`, the `reason` of a `Refused` and the
//! fields of a `Conflict`; inside `domain/error.rs` every `format!` and `write!` is a sink. A message built before the
//! constructor (`let msg = format!(..)` that a sink later uses) is read as if it stood in the sink.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Id(String),
    Str(String),
    Num,
    P(char),
}

fn is_id_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn tokenize(src: &str) -> Vec<Tok> {
    let c: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && c.get(i + 1) == Some(&'*') {
            let mut depth = 0;
            while i < c.len() {
                if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
        } else if ch == '"' {
            i += 1;
            let mut s = String::new();
            while i < c.len() && c[i] != '"' {
                if c[i] == '\\' && i + 1 < c.len() {
                    s.push(c[i]);
                    i += 1;
                }
                s.push(c[i]);
                i += 1;
            }
            i += 1;
            out.push(Tok::Str(s));
        } else if (ch == 'r' || ch == 'b')
            && (c.get(i + 1) == Some(&'"')
                || c.get(i + 1) == Some(&'#')
                || (ch == 'b' && c.get(i + 1) == Some(&'r')))
            && raw_string_at(&c, i).is_some()
        {
            let (s, next) = raw_string_at(&c, i).unwrap();
            out.push(Tok::Str(s));
            i = next;
        } else if ch == '\'' {
            // a char literal ('x', '\n', '\u{..}') or a lifetime ('a)
            if c.get(i + 1) == Some(&'\\') {
                i += 2;
                while i < c.len() && c[i] != '\'' {
                    i += 1;
                }
                i += 1;
                out.push(Tok::Num);
            } else if c.get(i + 2) == Some(&'\'') {
                i += 3;
                out.push(Tok::Num);
            } else {
                i += 1;
                while i < c.len() && (c[i].is_ascii_alphanumeric() || c[i] == '_') {
                    i += 1;
                }
            }
        } else if is_id_start(ch) {
            let st = i;
            while i < c.len() && (c[i].is_ascii_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            out.push(Tok::Id(c[st..i].iter().collect()));
        } else if ch.is_ascii_digit() {
            while i < c.len()
                && (c[i].is_ascii_alphanumeric()
                    || c[i] == '_'
                    || c[i] == '.' && c.get(i + 1).is_some_and(char::is_ascii_digit))
            {
                i += 1;
            }
            out.push(Tok::Num);
        } else {
            out.push(Tok::P(ch));
            i += 1;
        }
    }
    out
}

/// A raw string (`r"..."`, `r#"..."#`, `br"..."`) starting at `i`, and the index after it.
fn raw_string_at(c: &[char], i: usize) -> Option<(String, usize)> {
    let mut j = i;
    if c[j] == 'b' {
        j += 1;
    }
    if c.get(j) != Some(&'r') {
        return None;
    }
    j += 1;
    let mut hashes = 0;
    while c.get(j) == Some(&'#') {
        hashes += 1;
        j += 1;
    }
    if c.get(j) != Some(&'"') {
        return None;
    }
    j += 1;
    let st = j;
    while j < c.len() {
        if c[j] == '"' && (0..hashes).all(|k| c.get(j + 1 + k) == Some(&'#')) {
            return Some((c[st..j].iter().collect(), j + 1 + hashes));
        }
        j += 1;
    }
    None
}

/// The index of the token that closes the group opened at `open` (`(`, `[` or `{`).
fn close_of(t: &[Tok], open: usize) -> usize {
    let mut depth = 0;
    for (k, tok) in t.iter().enumerate().skip(open) {
        match tok {
            Tok::P('(' | '[' | '{') => depth += 1,
            Tok::P(')' | ']' | '}') => {
                depth -= 1;
                if depth == 0 {
                    return k;
                }
            }
            _ => {}
        }
    }
    t.len().saturating_sub(1)
}

/// The tokens without any `#[cfg(test)]` item (a module, function, use or impl): the attribute and the item after it.
/// A file that starts with `#![cfg(test)]` is test code as a whole.
fn without_test_items(t: &[Tok]) -> Vec<Tok> {
    let is = |k: usize, s: &str| matches!(t.get(k), Some(Tok::Id(x)) if x == s);
    let p = |k: usize, c: char| t.get(k) == Some(&Tok::P(c));
    if p(0, '#')
        && p(1, '!')
        && p(2, '[')
        && is(3, "cfg")
        && p(4, '(')
        && is(5, "test")
        && p(6, ')')
    {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut k = 0;
    while k < t.len() {
        if p(k, '#')
            && p(k + 1, '[')
            && is(k + 2, "cfg")
            && p(k + 3, '(')
            && is(k + 4, "test")
            && p(k + 5, ')')
            && p(k + 6, ']')
        {
            k += 7;
            // other attributes on the same item
            while p(k, '#') && p(k + 1, '[') {
                k = close_of(t, k + 1) + 1;
            }
            // the item: up to a `;` or past a `{..}` block at depth 0
            while k < t.len() {
                match &t[k] {
                    Tok::P(';') => {
                        k += 1;
                        break;
                    }
                    Tok::P('{') => {
                        k = close_of(t, k) + 1;
                        break;
                    }
                    Tok::P('(' | '[') => k = close_of(t, k) + 1,
                    _ => k += 1,
                }
            }
        } else {
            out.push(t[k].clone());
            k += 1;
        }
    }
    out
}

fn text_of(t: &[Tok]) -> String {
    t.iter()
        .map(|x| match x {
            Tok::Id(s) => s.clone(),
            Tok::Str(s) => format!("{s:?}"),
            Tok::Num => "0".into(),
            Tok::P(c) => c.to_string(),
        })
        .collect()
}

/// The top-level comma-separated pieces of the tokens between a group's brackets.
fn split_args(t: &[Tok]) -> Vec<&[Tok]> {
    let mut out = Vec::new();
    let (mut depth, mut st) = (0i32, 0);
    for (k, tok) in t.iter().enumerate() {
        match tok {
            Tok::P('(' | '[' | '{') => depth += 1,
            Tok::P(')' | ']' | '}') => depth -= 1,
            Tok::P(',') if depth == 0 => {
                out.push(&t[st..k]);
                st = k + 1;
            }
            _ => {}
        }
    }
    if st < t.len() {
        out.push(&t[st..]);
    }
    out
}

/// Whether the expression is exactly one call of `shown`: the identifier `shown` (alone or as the last path segment)
/// and then a parenthesis that closes at the very end. `xshown(..)` and `shown(a).b` are not.
fn is_shown_call(e: &[Tok]) -> bool {
    let Some(open) = e.iter().position(|x| *x == Tok::P('(')) else {
        return false;
    };
    if open == 0 || !matches!(&e[open - 1], Tok::Id(f) if f == "shown" || f == "audit_target") {
        return false;
    }
    // before `shown`: only path segments `a::b::`
    let mut k = 0;
    while k + 1 < open - 1 {
        match (&e[k], &e[k + 1]) {
            (Tok::Id(_), Tok::P(':')) => k += 2,
            (Tok::P(':'), Tok::Id(_)) => k += 1,
            _ => return false,
        }
        if k >= open - 1 {
            break;
        }
    }
    close_of(e, open) == e.len() - 1
}

/// An expression that is not caller text: a number, a literal, a constant, or a chain whose root is a counter.
fn is_plain(e: &[Tok]) -> bool {
    match e {
        [] => true,
        [Tok::Num | Tok::Str(_)] => true,
        _ => {
            let root = e.iter().find_map(|x| match x {
                Tok::Id(s) => Some(s.as_str()),
                _ => None,
            });
            let all_caps = |s: &str| {
                s.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            };
            let path_only = e.iter().all(|x| matches!(x, Tok::Id(_) | Tok::P(':')));
            let last = e.iter().rev().find_map(|x| match x {
                Tok::Id(s) => Some(s.as_str()),
                _ => None,
            });
            root.is_some_and(|r| NUMBERS.contains(&r)) || (path_only && last.is_some_and(all_caps))
        }
    }
}

/// Names that hold a count, a time or a sequence number wherever they are formatted into an error.
const NUMBERS: &[&str] = &[
    "seq",
    "ttl",
    "n",
    "secs",
    "secs_left",
    "held_secs",
    "min",
    "max",
    "limit",
    "count",
    "len",
    "next",
    "age",
    "left",
    "now",
    "ask",
    "cap",
];

/// One raw echo found: where, the format string and the argument.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Echo {
    pub file: String,
    pub template: String,
    pub arg: String,
}

/// The identifiers a format string reads, in order of the arguments: `(inline name)` for `{name}` and `None` for a
/// positional `{}`, `{:?}` or `{0}`.
fn placeholders(template: &str) -> Vec<Option<String>> {
    let c: Vec<char> = template.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '{' && c.get(i + 1) == Some(&'{') {
            i += 2;
        } else if c[i] == '{' {
            let st = i + 1;
            let mut j = st;
            while j < c.len() && c[j] != '}' {
                j += 1;
            }
            let body: String = c[st..j.min(c.len())].iter().collect();
            let name = body.split(':').next().unwrap_or("").trim().to_string();
            if name.is_empty() || name.chars().all(|x| x.is_ascii_digit()) {
                out.push(None);
            } else {
                out.push(Some(name));
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

const SINKS: &[&str] = &[
    "Invalid",
    "NotFound",
    "Backend",
    "Busy",
    "Bad",
    "fail_with",
    "rpc_error",
    "invalid",
    "refused",
    "reserved",
    "Err",
];

/// The raw echoes in the source of one file.
pub(super) fn raw_echoes(file: &str, src: &str) -> Vec<Echo> {
    let t = without_test_items(&tokenize(src));
    let in_error_file = file.ends_with("domain/error.rs");
    // spans (token ranges) of the sink calls
    let mut sink_spans: Vec<(usize, usize)> = Vec::new();
    for k in 0..t.len() {
        let Tok::Id(name) = &t[k] else { continue };
        if SINKS.contains(&name.as_str()) && t.get(k + 1) == Some(&Tok::P('(')) {
            sink_spans.push((k + 1, close_of(&t, k + 1)));
        }
        // the reason of a refusal, and the text fields of a conflict
        if name == "Refused" && t.get(k + 1) == Some(&Tok::P('{')) {
            let end = close_of(&t, k + 1);
            for piece in split_args(&t[k + 2..end]) {
                if piece.first() == Some(&Tok::Id("reason".into())) {
                    sink_spans.push((k + 1, end));
                }
            }
        }
        if name == "Conflict" && t.get(k + 1) == Some(&Tok::P('{')) {
            sink_spans.push((k + 1, close_of(&t, k + 1)));
        }
    }
    // a name that a sink reads: `let msg = format!(..)` is then part of a sink
    let mut read_by_sinks: Vec<&str> = Vec::new();
    for &(a, b) in &sink_spans {
        for x in &t[a..b] {
            if let Tok::Id(s) = x {
                read_by_sinks.push(s.as_str());
            }
        }
    }
    let mut found = Vec::new();
    let mut seen_format = Vec::new();
    for k in 0..t.len() {
        let is_macro = matches!(&t[k], Tok::Id(m) if m == "format" || m == "write" || m == "writeln")
            && t.get(k + 1) == Some(&Tok::P('!'));
        let open = k + 2;
        if !is_macro || t.get(open) != Some(&Tok::P('(')) {
            continue;
        }
        let end = close_of(&t, open);
        let in_sink = in_error_file
            || sink_spans.iter().any(|&(a, b)| a < k && k < b)
            || (k >= 3
                && t[k - 1] == Tok::P('=')
                && matches!(&t[k - 2], Tok::Id(v) if read_by_sinks.contains(&v.as_str())));
        if !in_sink {
            continue;
        }
        let args = split_args(&t[open + 1..end]);
        // write!(f, "..", args): the first piece is the writer
        let first = usize::from(matches!(&t[k], Tok::Id(m) if m == "write" || m == "writeln"));
        let Some(Tok::Str(template)) = args.get(first).and_then(|a| a.first()) else {
            continue;
        };
        let positional: Vec<&[Tok]> = args[first + 1..]
            .iter()
            .filter(|a| {
                !(a.len() > 1
                    && matches!(&a[0], Tok::Id(_))
                    && a[1] == Tok::P('=')
                    && a.get(2) != Some(&Tok::P('=')))
            })
            .copied()
            .collect();
        let named: Vec<(&str, &[Tok])> = args[first + 1..]
            .iter()
            .filter_map(|a| match (&a.first(), a.get(1)) {
                (Some(Tok::Id(n)), Some(Tok::P('='))) if a.get(2) != Some(&Tok::P('=')) => {
                    Some((n.as_str(), &a[2..]))
                }
                _ => None,
            })
            .collect();
        let mut next = 0;
        for ph in placeholders(template) {
            let arg: Vec<Tok> = match ph {
                Some(name) => match named.iter().find(|(n, _)| *n == name) {
                    Some((_, e)) => e.to_vec(),
                    None => vec![Tok::Id(name)],
                },
                None => {
                    let e = positional.get(next).map(|e| e.to_vec()).unwrap_or_default();
                    next += 1;
                    e
                }
            };
            if is_shown_call(&arg) || is_plain(&arg) {
                continue;
            }
            found.push(Echo {
                file: file.to_string(),
                template: template.clone(),
                arg: text_of(&arg),
            });
        }
        seen_format.push((k, end));
    }
    // a bare expression that is not a format: a concatenation or a call that builds text
    for &(a, b) in &sink_spans {
        let inner = &t[a + 1..b];
        let covered = |k: usize| seen_format.iter().any(|&(s, e)| s <= k && k <= e);
        // only a sink whose whole argument is such an expression: `+` outside any format!
        let plus = (a + 1..b).any(|k| t[k] == Tok::P('+') && !covered(k));
        if plus && !inner.is_empty() {
            found.push(Echo {
                file: file.to_string(),
                template: "<concatenation>".into(),
                arg: text_of(inner),
            });
        }
    }
    found.sort_by(|x, y| (&x.template, &x.arg).cmp(&(&y.template, &y.arg)));
    found.dedup();
    found
}

/// The sites where a raw echo is safe, one per line: `file<TAB>format string (Debug form)<TAB>argument<TAB>reason`.
/// The file is relative to `src`. An entry holds only a value that is validated before the error is built (and says
/// by what), a number, or text that is not a caller's; nothing that was fixed is listed, and an entry that matches no
/// echo fails the test, so the list cannot rot.
const ALLOWED_TSV: &str = include_str!("echo_guard_allowed.tsv");

fn allowed_entries() -> Vec<[&'static str; 4]> {
    ALLOWED_TSV
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            assert_eq!(
                f.len(),
                4,
                "a line of echo_guard_allowed.tsv needs four tab-separated fields: {l}"
            );
            [f[0], f[1], f[2], f[3]]
        })
        .collect()
}

/// The path of a file relative to `src`.
fn rel(file: &str) -> &str {
    file.split("/src/").nth(1).unwrap_or(file)
}

fn matches_entry(e: &Echo, a: &[&str; 4]) -> bool {
    rel(&e.file) == a[0] && format!("{:?}", e.template) == a[1] && e.arg == a[2]
}

fn allowed(e: &Echo) -> bool {
    allowed_entries().iter().any(|a| matches_entry(e, a))
}

fn source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if name == "tests"
                || name == "test_support"
                || name == "conformance"
                || name == "memory_spike"
            {
                continue;
            }
            source_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            let stem = p.file_stem().unwrap().to_string_lossy().to_string();
            if !(stem.ends_with("_tests") || stem == "tests") {
                out.push(p);
            }
        }
    }
}

/// The directories the guard reads (relative to `src`).
const SCANNED: &[&str] = &["gateway", "session", "keys", "init"];

/// Single files read as well: the error text of the domain and the store sites that echo a caller.
const SCANNED_FILES: &[&str] = &[
    "domain/error.rs",
    "domain/labels.rs",
    "store/sqlite/releases.rs",
    "store/sqlite/api/policy.rs",
    "store/sqlite/credentials.rs",
];

fn scanned_files() -> Vec<(String, PathBuf)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut all = Vec::new();
    for d in SCANNED {
        let mut files = Vec::new();
        source_files(&root.join(d), &mut files);
        assert!(!files.is_empty(), "no source files under {d}");
        all.extend(files.into_iter().map(|f| (d.to_string(), f)));
    }
    for f in SCANNED_FILES {
        all.push(("files".to_string(), root.join(f)));
    }
    all
}

#[test]
fn no_error_repeats_a_raw_caller_value() {
    let mut found = Vec::new();
    for (_, f) in scanned_files() {
        let src = std::fs::read_to_string(&f).unwrap();
        let file = f.to_string_lossy().replace('\\', "/");
        found.extend(raw_echoes(&file, &src).into_iter().filter(|e| !allowed(e)));
    }
    let report: Vec<String> = found
        .iter()
        .map(|e| format!("{}\t{:?}\t{}\twhy", rel(&e.file), e.template, e.arg))
        .collect();
    assert!(
        found.is_empty(),
        "text of a caller is formatted raw into an error; wrap it in `shown(..)`, or if it was validated before, add \
         the site to ALLOWED with the reason:\n{}",
        report.join("\n")
    );
}

#[test]
fn the_allowlist_holds_no_unused_entry_and_every_entry_has_a_reason() {
    let entries = allowed_entries();
    let mut used = vec![false; entries.len()];
    for (_, f) in scanned_files() {
        let src = std::fs::read_to_string(&f).unwrap();
        let file = f.to_string_lossy().replace('\\', "/");
        for e in raw_echoes(&file, &src) {
            for (i, a) in entries.iter().enumerate() {
                if matches_entry(&e, a) {
                    used[i] = true;
                }
            }
        }
    }
    let unused: Vec<_> = entries
        .iter()
        .zip(&used)
        .filter(|(_, u)| !**u)
        .map(|(a, _)| (a[0], a[2]))
        .collect();
    assert!(
        unused.is_empty(),
        "entries that match no raw echo (remove them): {unused:?}"
    );
    for a in &entries {
        assert!(
            a[3].trim().len() > 5 && a[3] != "why",
            "an entry needs a reason: {a:?}"
        );
        assert!(!a[2].contains("shown("), "a fixed site is listed: {a:?}");
    }
}

fn echoes(src: &str) -> Vec<Echo> {
    raw_echoes("x/gateway/x.rs", src)
}

#[test]
fn the_scanner_flags_each_blind_spot_snippet() {
    let flagged: &[(&str, &str)] = &[
        (
            "plain inline",
            r#"fn f() { return Err(Error::Invalid(format!("bad: {member}"))); }"#,
        ),
        (
            "debug inline",
            r#"fn f() { Err(Error::Invalid(format!("bad: {label:?}"))) }"#,
        ),
        (
            "positional",
            r#"fn f() { Err(Error::Invalid(format!("bad: {}", name))) }"#,
        ),
        (
            "multi-line layout",
            "fn f() {\n    return Err(Error::Invalid(format!(\n        \"bad {} thing: {}\",\n        what,\n        member\n    )));\n}",
        ),
        (
            "semicolon in the literal",
            r#"fn f() { Err(Error::Invalid(format!("a; b {member}"))) }"#,
        ),
        (
            "brace in the literal",
            r#"fn f() { Err(Error::Invalid(format!("{{ok}} {member}"))) }"#,
        ),
        (
            "reference",
            r#"fn f() { Err(Error::Invalid(format!("bad {}", &member))) }"#,
        ),
        (
            "field",
            r#"fn f() { Err(Error::Invalid(format!("bad {}", spec.member))) }"#,
        ),
        (
            "index",
            r#"fn f() { Err(Error::Invalid(format!("bad {}", names[i]))) }"#,
        ),
        (
            "concatenation",
            r#"fn f() { Err(Error::Invalid("bad: ".to_string() + member)) }"#,
        ),
        (
            "message built before the constructor",
            "fn f() { let msg = format!(\"bad {}\", member); Err(Error::Invalid(msg)) }",
        ),
        (
            "refusal reason as a variable",
            "fn f() { let why = format!(\"no {act}\"); Err(Error::Refused { rule: \"r\".into(), reason: why }) }",
        ),
        (
            "refusal reason inline",
            "fn f() { Err(Error::Refused { rule: \"r\".into(), reason: format!(\"no {act}\") }) }",
        ),
        (
            "conflict",
            "fn f() { Err(Error::Conflict { path: format!(\"{path}\"), owner: o }) }",
        ),
        (
            "a local closure that makes the refusal",
            r#"fn f() { reserved(&format!("belongs to run {id}")) }"#,
        ),
        (
            "the invalid helper",
            r#"fn f() { invalid(ok, format!("bad {tool}"))?; }"#,
        ),
        (
            "Error::Bad",
            r#"fn f() { Err(Error::Bad(format!("bad key {name}"))) }"#,
        ),
        (
            "fail_with",
            r#"fn f() { vec![fail_with(id, "c", format!("unknown {method}"))] }"#,
        ),
        (
            "rpc_error",
            r#"fn f() { Err(rpc_error(C, format!("no tool {other}"))) }"#,
        ),
        (
            "plain Err(String)",
            r#"fn f() -> Result<(), String> { Err(format!("bad hook {event}")) }"#,
        ),
        (
            "a name outside any list",
            r#"fn f() { Err(Error::Invalid(format!("{act} is not an act"))) }"#,
        ),
        (
            "named argument",
            r#"fn f() { Err(Error::Invalid(format!("bad {x}", x = member))) }"#,
        ),
        (
            "xshown is not shown",
            r#"fn f() { Err(Error::Invalid(format!("bad {}", xshown(member)))) }"#,
        ),
        (
            "shown(..).field is not shown",
            r#"fn f() { Err(Error::Invalid(format!("bad {}", shown(member).len_x))) }"#,
        ),
        (
            "code after a cfg(test) module",
            "#[cfg(test)]\nmod tests { fn t() {} }\nfn f() { Err(Error::Invalid(format!(\"bad {member}\"))) }",
        ),
        (
            "code after a cfg(test) function",
            "#[cfg(test)]\nfn t() { let _ = 1; }\nfn f() { Err(Error::Invalid(format!(\"bad {member}\"))) }",
        ),
    ];
    for (what, src) in flagged {
        assert!(!echoes(src).is_empty(), "not flagged: {what}");
    }
    // Display of an error: every write! in domain/error.rs
    let display = r#"impl Display for Error { fn fmt(&self, f: &mut F) -> R { write!(f, "refused: {path}") } }"#;
    assert!(!raw_echoes("x/domain/error.rs", display).is_empty());
    assert!(raw_echoes("x/domain/other.rs", display).is_empty());
}

#[test]
fn the_scanner_passes_what_is_safe() {
    let safe: &[(&str, &str)] = &[
        (
            "shown",
            r#"fn f() { Err(Error::Invalid(format!("bad {}", shown(member)))) }"#,
        ),
        (
            "shown by path",
            r#"fn f() { Err(Error::Invalid(format!("bad {}", crate::domain::shown(member)))) }"#,
        ),
        (
            "shown of a reference",
            r#"fn f() { Err(Error::Invalid(format!("bad {}", shown(&names[i])))) }"#,
        ),
        (
            "a number",
            r#"fn f() { Err(Error::Invalid(format!("no message {seq}"))) }"#,
        ),
        (
            "a constant",
            r#"fn f() { Err(Error::Invalid(format!("at most {MAX_POST_BYTES}"))) }"#,
        ),
        (
            "a literal",
            r#"fn f() { Err(Error::Invalid("bad name".into())) }"#,
        ),
        (
            "a format with no argument",
            r#"fn f() { Err(Error::Invalid(format!("bad name"))) }"#,
        ),
        (
            "a format outside any error",
            r#"fn f() { let s = format!("{member}"); log(s); }"#,
        ),
        (
            "a cfg(test) module",
            "#[cfg(test)]\nmod tests { fn t() { Err(Error::Invalid(format!(\"{member}\"))) } }",
        ),
        ("a cfg(test) use", "#[cfg(test)]\nuse a::b;\nfn f() {}"),
        (
            "a comment",
            "fn f() { // Err(Error::Invalid(format!(\"{member}\")))\n}",
        ),
        (
            "a string that looks like code",
            r#"fn f() { let _ = "Error::Invalid(format!(\"{member}\"))"; }"#,
        ),
    ];
    for (what, src) in safe {
        assert!(echoes(src).is_empty(), "flagged: {what}: {:?}", echoes(src));
    }
}

#[test]
fn a_throwaway_raw_echo_is_found_in_every_scanned_directory() {
    let mut reached = std::collections::BTreeSet::new();
    for (dir, f) in scanned_files() {
        if reached.contains(&dir) {
            continue;
        }
        let src = std::fs::read_to_string(&f).unwrap();
        let file = f.to_string_lossy().replace('\\', "/");
        let before = raw_echoes(&file, &src)
            .iter()
            .filter(|e| !allowed(e))
            .count();
        let mutant = format!(
            "{src}\nfn throwaway(member: &str) -> Result<(), Error> {{ Err(Error::Invalid(format!(\"bad {{member}}\"))) }}\n"
        );
        let after = raw_echoes(&file, &mutant)
            .iter()
            .filter(|e| !allowed(e))
            .count();
        assert_eq!(after, before + 1, "{file}");
        reached.insert(dir);
    }
    assert_eq!(reached.len(), SCANNED.len() + 1, "{reached:?}");
}

#[test]
fn code_after_the_test_module_in_gateway_rs_is_still_read() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/gateway/gateway.rs");
    let t = without_test_items(&tokenize(&std::fs::read_to_string(root).unwrap()));
    assert!(t.contains(&Tok::Id("check_ask_shape".into())));
}
