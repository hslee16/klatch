//! The source scanner behind the no-network guards of `klatch` (`no_telemetry.rs`) and `klatch-server`
//! (`no_network.rs`, which includes this file with `#[path]`; a test helper shared between the two crates without a
//! dependency). Not a test file.
//!
//! It is a real tokenizer, not a line search: comments (line, block, nested) are removed and string, raw string and
//! char literals stay single tokens, so a `//` inside a string no longer hides the rest of a line, a path split over
//! lines or written with spaces reads as one path, and `use` trees (braces, `self`, `as`, globs) are expanded to full
//! paths. It does not parse anything else and it cannot see code a macro writes. `syn` was not used: it is in the lock
//! file only as the build-time dependency of the derive macros, a dev-dependency on it would turn on parser features for
//! it and add a node to `THIRD-PARTY-LICENSES.txt` and the audit, for an audit that needs about three hundred lines.
//!
//! What it reports, whatever the file: a `libc` name that makes, binds, connects, sends through or resolves for a socket,
//! raw `syscall`s and `SYS_` numbers, the `exec` family, `system`, `popen` and `posix_spawn`, however `libc` is spelled
//! (`libc::x`, `::libc::x`, `libc :: x`, an alias from `use libc as l`, `use libc::{self as l}` or `extern crate libc as
//! l`, a name imported alone, in braces or by glob); other network APIs by module path; resolvers; `extern` blocks that
//! declare such a function, with or without an ABI string; any `#[link_name]`; inline assembly; a symbol looked up at
//! run time; an alias of `Command` and a glob of `std::process`. The callers decide which findings a file may keep.
#![allow(dead_code)]

use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Id(String),
    Str(String),
    Num,
    P(char),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tok {
    pub kind: Kind,
    /// The line of the token, starting at 1.
    pub line: usize,
}

fn is_id_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_id_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The source with every comment replaced by spaces (newlines kept, so line numbers hold). Strings, raw strings and char
/// literals are left as they are, and a `//` inside one is not a comment.
pub fn strip_comments(src: &str) -> String {
    let c: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if ch == '/' && c.get(i + 1) == Some(&'*') {
            let mut depth = 0;
            while i < c.len() {
                if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(if c[i] == '\n' { '\n' } else { ' ' });
                    i += 1;
                }
            }
        } else if ch == '"' {
            out.push(ch);
            i += 1;
            while i < c.len() && c[i] != '"' {
                if c[i] == '\\' && i + 1 < c.len() {
                    out.push(c[i]);
                    i += 1;
                }
                out.push(c[i]);
                i += 1;
            }
            if i < c.len() {
                out.push('"');
                i += 1;
            }
        } else if let Some((end, _)) = raw_string_at(&c, i) {
            out.extend(&c[i..end]);
            i = end;
        } else if ch == '\'' {
            let end = char_literal_end(&c, i);
            out.extend(&c[i..end]);
            i = end;
        } else {
            out.push(ch);
            i += 1;
        }
    }
    out
}

/// The index after a char literal (`'x'`, `'\n'`, `'\u{..}'`) that starts at `i`, or after the lone quote of a lifetime.
fn char_literal_end(c: &[char], i: usize) -> usize {
    if c.get(i + 1) == Some(&'\\') {
        // an escape: the character after the backslash is part of it, whatever it is (`'\''`, `'\\'`, `'\u{..}'`)
        let mut j = i + 3;
        while j < c.len() && c[j] != '\'' {
            j += 1;
        }
        (j + 1).min(c.len())
    } else if c.get(i + 2) == Some(&'\'') {
        i + 3
    } else {
        i + 1
    }
}

/// A raw string (`r"..."`, `r#"..."#`, `br"..."`) starting at `i`: the index after it and its text.
fn raw_string_at(c: &[char], i: usize) -> Option<(usize, String)> {
    if i > 0 && is_id_char(c[i - 1]) {
        return None;
    }
    let mut j = i;
    if matches!(c.get(j), Some('b' | 'c')) {
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
    let start = j;
    while j < c.len() {
        if c[j] == '"' && (0..hashes).all(|k| c.get(j + 1 + k) == Some(&'#')) {
            return Some((j + 1 + hashes, c[start..j].iter().collect()));
        }
        j += 1;
    }
    None
}

/// The tokens of Rust source: identifiers (a raw identifier `r#name` reads as `name`), strings, numbers and every other
/// character on its own. Comments are gone.
pub fn tokenize(src: &str) -> Vec<Tok> {
    let c: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line) = (0, 1);
    let mut push = |kind: Kind, line: usize| out.push(Tok { kind, line });
    while i < c.len() {
        let ch = c[i];
        if ch == '\n' {
            line += 1;
            i += 1;
        } else if ch.is_whitespace() {
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
                    if c[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
        } else if let Some((end, text)) = raw_string_at(&c, i) {
            push(Kind::Str(text), line);
            line += c[i..end].iter().filter(|x| **x == '\n').count();
            i = end;
        } else if ch == '"' || (matches!(ch, 'b' | 'c') && c.get(i + 1) == Some(&'"')) {
            let start_line = line;
            i += if ch == '"' { 1 } else { 2 };
            let mut s = String::new();
            while i < c.len() && c[i] != '"' {
                if c[i] == '\\' && i + 1 < c.len() {
                    s.push(c[i]);
                    i += 1;
                }
                if c[i] == '\n' {
                    line += 1;
                }
                s.push(c[i]);
                i += 1;
            }
            i += 1;
            push(Kind::Str(s), start_line);
        } else if ch == 'b' && c.get(i + 1) == Some(&'\'') && char_literal_end(&c, i + 1) > i + 2 {
            let end = char_literal_end(&c, i + 1);
            push(Kind::Num, line);
            i = end;
        } else if ch == '\'' {
            let end = char_literal_end(&c, i);
            if end > i + 1 {
                push(Kind::Num, line);
                i = end;
            } else {
                // a lifetime: the quote and its name are not a path segment
                i += 1;
                while i < c.len() && is_id_char(c[i]) {
                    i += 1;
                }
            }
        } else if is_id_start(ch) {
            let start = i;
            while i < c.len() && is_id_char(c[i]) {
                i += 1;
            }
            let word: String = c[start..i].iter().collect();
            if word == "r"
                && c.get(i) == Some(&'#')
                && c.get(i + 1).is_some_and(|x| is_id_start(*x))
            {
                i += 1;
                let s2 = i;
                while i < c.len() && is_id_char(c[i]) {
                    i += 1;
                }
                push(Kind::Id(c[s2..i].iter().collect()), line);
            } else {
                push(Kind::Id(word), line);
            }
        } else if ch.is_ascii_digit() {
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            push(Kind::Num, line);
        } else {
            push(Kind::P(ch), line);
            i += 1;
        }
    }
    out
}

/// One use of a path: its segments (`libc`, `socket`), the line, whether it ends in `*`, and the name it is given with
/// `as`.
#[derive(Debug, Clone, PartialEq)]
pub struct PathUse {
    pub segs: Vec<String>,
    pub line: usize,
    pub glob: bool,
    pub alias: Option<String>,
    /// Whether the path stands in a `use` or `extern crate` statement.
    pub import: bool,
}

fn id(t: &[Tok], k: usize) -> Option<&str> {
    match t.get(k) {
        Some(Tok {
            kind: Kind::Id(s), ..
        }) => Some(s.as_str()),
        _ => None,
    }
}

fn is_p(t: &[Tok], k: usize, c: char) -> bool {
    matches!(t.get(k), Some(Tok { kind: Kind::P(x), .. }) if *x == c)
}

fn is_sep(t: &[Tok], k: usize) -> bool {
    is_p(t, k, ':') && is_p(t, k + 1, ':')
}

/// The index of the token that closes the group opened at `open`.
fn close_of(t: &[Tok], open: usize) -> usize {
    let mut depth = 0i32;
    for (k, tok) in t.iter().enumerate().skip(open) {
        match tok.kind {
            Kind::P('(' | '[' | '{') => depth += 1,
            Kind::P(')' | ']' | '}') => {
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

/// Expand one `use` tree that starts at `k` (after `use`, or after a `,` or `{` inside one) with `prefix`; returns the
/// index after the tree.
fn use_tree(t: &[Tok], mut k: usize, prefix: &[String], out: &mut Vec<PathUse>) -> usize {
    let mut segs: Vec<String> = prefix.to_vec();
    let line = t.get(k).map_or(0, |x| x.line);
    if is_sep(t, k) {
        k += 2; // a leading `::`
    }
    loop {
        if is_p(t, k, '{') {
            let close = close_of(t, k);
            let mut j = k + 1;
            while j < close {
                j = use_tree(t, j, &segs, out);
                if is_p(t, j, ',') {
                    j += 1;
                }
            }
            return close + 1;
        }
        if is_p(t, k, '*') {
            out.push(PathUse {
                segs,
                line,
                glob: true,
                alias: None,
                import: true,
            });
            return k + 1;
        }
        let Some(name) = id(t, k) else { return k + 1 };
        k += 1;
        if name == "self" {
            // `{self}` or `{self as x}` is the prefix itself
        } else {
            segs.push(name.to_string());
        }
        if is_sep(t, k) {
            k += 2;
            continue;
        }
        let alias = if id(t, k) == Some("as") {
            let a = id(t, k + 1).map(str::to_string);
            k += 2;
            a
        } else {
            None
        };
        out.push(PathUse {
            segs,
            line,
            glob: false,
            alias,
            import: true,
        });
        return k;
    }
}

/// Every path of the source (see `paths`), from the source text.
pub fn path_uses(src: &str) -> Vec<PathUse> {
    paths(&tokenize(src))
}

/// Every path in the source: the `use` trees expanded to full paths, `extern crate x as y`, and the paths written inline
/// (`libc::socket`, `:: libc :: socket`, `l::socket`).
pub fn paths(t: &[Tok]) -> Vec<PathUse> {
    let mut out = Vec::new();
    let mut k = 0;
    while k < t.len() {
        match id(t, k) {
            Some("use") if k == 0 || !is_p(t, k - 1, '.') => {
                let end = (k..t.len()).find(|&j| is_p(t, j, ';')).unwrap_or(t.len());
                use_tree(&t[..end], k + 1, &[], &mut out);
                k = end;
            }
            Some("extern") if id(t, k + 1) == Some("crate") => {
                if let Some(name) = id(t, k + 2) {
                    let alias = (id(t, k + 3) == Some("as"))
                        .then(|| id(t, k + 4).map(str::to_string))
                        .flatten();
                    out.push(PathUse {
                        segs: vec![name.to_string()],
                        line: t[k].line,
                        glob: false,
                        alias,
                        import: true,
                    });
                }
                k += 3;
            }
            Some(first) => {
                // an inline path: `a :: b :: c`, not the tail of a longer one
                let preceded =
                    k >= 3 && is_sep(t, k - 2) && (id(t, k - 3).is_some() || is_p(t, k - 3, '$'));
                if !preceded {
                    let mut segs = vec![first.to_string()];
                    let mut j = k + 1;
                    while is_sep(t, j) {
                        if let Some(n) = id(t, j + 2) {
                            segs.push(n.to_string());
                            j += 3;
                        } else if is_p(t, j + 2, '$') && id(t, j + 3).is_some() {
                            segs.push(format!("${}", id(t, j + 3).unwrap()));
                            j += 4;
                        } else {
                            break;
                        }
                    }
                    if segs.len() > 1 {
                        out.push(PathUse {
                            segs,
                            line: t[k].line,
                            glob: false,
                            alias: None,
                            import: false,
                        });
                    }
                    k = j.max(k + 1);
                    continue;
                }
                k += 1;
            }
            None => k += 1,
        }
    }
    out
}

/// The `libc` calls that make, connect, bind, listen on, accept on, send or receive through a socket, or resolve a name.
pub const LIBC_NET_CALLS: &[&str] = &[
    "socket",
    "socketpair",
    "connect",
    "connectx",
    "bind",
    "listen",
    "accept",
    "accept4",
    "paccept",
    "sendto",
    "sendmsg",
    "sendmsg_x",
    "sendmmsg",
    "recvfrom",
    "recvmsg",
    "recvmsg_x",
    "recvmmsg",
    "send",
    "recv",
    "getaddrinfo",
    "getnameinfo",
    "gethostbyname",
    "gethostbyaddr",
    "getsockname",
    "getpeername",
    "getpeereid",
    "setsockopt",
    "getsockopt",
    "shutdown",
    "getifaddrs",
    "freeifaddrs",
    "if_nameindex",
    "res_init",
    "res_query",
    "res_search",
    "syscall",
];

/// The `libc` calls that start a program.
pub const LIBC_SPAWN_CALLS: &[&str] = &[
    "execv",
    "execve",
    "execvp",
    "execvpe",
    "execl",
    "execle",
    "execlp",
    "fexecve",
    "system",
    "popen",
    "posix_spawn",
    "posix_spawnp",
];

/// `libc` names that only a socket user needs, by prefix: address families, socket types, protocols, option levels and
/// options, message flags, interface requests, netlink, system call numbers, and the address and message structs.
pub const LIBC_NET_PREFIXES: &[&str] = &[
    "AF_",
    "PF_",
    "SOCK_",
    "IPPROTO_",
    "SOL_",
    "SO_",
    "MSG_",
    "INADDR_",
    "NETLINK_",
    "SIOC",
    "SYS_",
    "sockaddr",
    "in_addr",
    "in6_addr",
    "addrinfo",
    "msghdr",
    "cmsghdr",
    "ip_mreq",
    "ipv6_mreq",
    "ifaddrs",
    "ifreq",
];

/// `libc` names that only a socket user needs, whole.
pub const LIBC_NET_NAMES: &[&str] = &["socklen_t", "sa_family_t", "in_port_t", "ucred"];

/// Modules and crates of other network APIs: a path starting with these segments.
pub const NET_MODULES: &[&[&str]] = &[
    &["nix", "sys", "socket"],
    &["rustix", "net"],
    &["socket2"],
    &["tokio", "net"],
    &["mio", "net"],
    &["async_std", "net"],
    &["smol", "net"],
];

/// Names that look a host up or turn a name into an address.
pub const RESOLVERS: &[&str] = &[
    "getaddrinfo",
    "gethostbyname",
    "getnameinfo",
    "to_socket_addrs",
    "ToSocketAddrs",
    "lookup_host",
];

/// Names that look a symbol up at run time (a way to reach a call no path names).
pub const DYNAMIC_LOOKUP: &[&str] = &["dlsym", "dlvsym", "dlopen"];

fn dangerous_libc_name(name: &str) -> Option<&'static str> {
    if LIBC_NET_CALLS.contains(&name) {
        Some("a network or raw system call")
    } else if LIBC_SPAWN_CALLS.contains(&name) {
        Some("a program start")
    } else if LIBC_NET_NAMES.contains(&name)
        || LIBC_NET_PREFIXES.iter().any(|p| name.starts_with(p))
    {
        Some("a socket name")
    } else if name.starts_with('$') {
        Some("a name a macro fills in")
    } else {
        None
    }
}

/// One thing the scan found: the line and what.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub line: usize,
    pub what: String,
}

/// The names `libc` goes by in this file: `libc` itself and every alias.
fn libc_roots(paths: &[PathUse]) -> BTreeSet<String> {
    let mut roots: BTreeSet<String> = BTreeSet::new();
    roots.insert("libc".to_string());
    for p in paths {
        if p.import
            && p.segs.len() == 1
            && p.segs[0] == "libc"
            && let Some(a) = &p.alias
        {
            roots.insert(a.clone());
        }
    }
    roots
}

/// The `extern` blocks of the token stream: for each, the index of its `{` and of its `}`. A block has an ABI string or
/// none (`extern {`, `unsafe extern {`, `extern "C" {`); `extern "C" fn` and `extern crate` are not blocks.
fn extern_blocks(t: &[Tok]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for k in 0..t.len() {
        if id(t, k) != Some("extern") {
            continue;
        }
        let mut j = k + 1;
        if matches!(
            t.get(j),
            Some(Tok {
                kind: Kind::Str(_),
                ..
            })
        ) {
            j += 1;
        }
        if is_p(t, j, '{') {
            out.push((j, close_of(t, j)));
        }
    }
    out
}

/// Everything the scan reports about one file (see the module comment).
pub fn findings(src: &str) -> Vec<Finding> {
    let t = tokenize(src);
    let all = paths(&t);
    let roots = libc_roots(&all);
    let mut out = Vec::new();
    let mut add = |line: usize, what: String| out.push(Finding { line, what });
    for p in &all {
        let first = p.segs.first().map(String::as_str).unwrap_or("");
        if roots.contains(first) && !(p.import && p.segs.len() == 1 && !p.glob) {
            if p.glob && p.segs.len() == 1 {
                add(p.line, "a glob import of libc".to_string());
            } else if let Some(name) = p.segs.get(1)
                && let Some(kind) = dangerous_libc_name(name)
            {
                add(p.line, format!("libc::{name}, {kind}"));
            }
        }
        for module in NET_MODULES {
            if p.segs.len() >= module.len() && p.segs.iter().zip(module.iter()).all(|(a, b)| a == b)
            {
                add(p.line, format!("the network API {}", module.join("::")));
            }
        }
        if p.segs.len() >= 4 && p.segs[..4] == ["std", "os", "unix", "net"] {
            add(p.line, "std::os::unix::net".to_string());
        }
        // a glob of std::process, or Command under another name
        if p.segs.len() >= 2 && p.segs[..2] == ["std", "process"] {
            if p.glob && p.segs.len() == 2 {
                add(p.line, "a glob import of std::process".to_string());
            }
            if p.segs.last().map(String::as_str) == Some("Command") && p.alias.is_some() {
                add(p.line, "an alias of Command".to_string());
            }
        }
    }
    for (k, tok) in t.iter().enumerate() {
        let Kind::Id(name) = &tok.kind else { continue };
        if RESOLVERS.contains(&name.as_str()) {
            add(tok.line, format!("the resolver {name}"));
        }
        if DYNAMIC_LOOKUP.contains(&name.as_str()) {
            add(tok.line, format!("{name}: a symbol looked up at run time"));
        }
        if matches!(name.as_str(), "asm" | "global_asm" | "llvm_asm") && is_p(&t, k + 1, '!') {
            add(tok.line, format!("{name}!: inline assembly"));
        }
        if name == "link_name" && is_p(&t, k.wrapping_sub(1), '[') {
            add(tok.line, "#[link_name]".to_string());
        }
    }
    for (open, close) in extern_blocks(&t) {
        for k in open..close {
            if id(&t, k) == Some("fn")
                && let Some(name) = id(&t, k + 1)
                && dangerous_libc_name(name).is_some_and(|kind| {
                    kind != "a socket name" && kind != "a name a macro fills in"
                })
            {
                add(t[k].line, format!("an extern declaration of {name}"));
            }
        }
    }
    out.sort_by(|a, b| (a.line, &a.what).cmp(&(b.line, &b.what)));
    out.dedup();
    out
}

/// The files a `#[path = "..."]` attribute or an `include!("...")` names, as written, with the line.
pub fn path_targets(src: &str) -> Vec<(usize, String)> {
    let t = tokenize(src);
    let mut out = Vec::new();
    for k in 0..t.len() {
        let lit = |j: usize| match t.get(j) {
            Some(Tok {
                kind: Kind::Str(s), ..
            }) => Some(s.clone()),
            _ => None,
        };
        if id(&t, k) == Some("path")
            && is_p(&t, k + 1, '=')
            && is_p(&t, k.wrapping_sub(1), '[')
            && let Some(s) = lit(k + 2)
        {
            out.push((t[k].line, s));
        }
        if id(&t, k) == Some("include")
            && is_p(&t, k + 1, '!')
            && is_p(&t, k + 2, '(')
            && let Some(s) = lit(k + 3)
        {
            out.push((t[k].line, s));
        }
    }
    out
}

/// A relative file name resolved against the directory of the file that names it, with `.` and `..` folded.
pub fn resolve_relative(from_rel: &str, target: &str) -> String {
    let mut parts: Vec<&str> = from_rel.split('/').collect();
    parts.pop();
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            // a `..` out of the crate stays a `..` (the shared test helper of a sibling crate is reached that way)
            ".." => {
                if parts.last().is_some_and(|l| *l != "..") {
                    parts.pop();
                } else {
                    parts.push("..");
                }
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// The identifiers of the source with their lines (strings and comments are not identifiers).
pub fn ids(src: &str) -> Vec<(usize, String)> {
    tokenize(src)
        .into_iter()
        .filter_map(|t| match t.kind {
            Kind::Id(s) => Some((t.line, s)),
            _ => None,
        })
        .collect()
}

/// The text of each `Command::new(...)` (also `process::Command::new(` and `std::process::Command::new(`) in the source,
/// with its line: `Command::new("git")` for a literal program and `Command::new(program)` for anything else.
pub fn command_new_calls(src: &str) -> Vec<(usize, String)> {
    let t = tokenize(src);
    let mut out = Vec::new();
    for k in 0..t.len() {
        if id(&t, k) == Some("Command")
            && is_sep(&t, k + 1)
            && id(&t, k + 3) == Some("new")
            && is_p(&t, k + 4, '(')
        {
            let arg = match t.get(k + 5) {
                Some(Tok {
                    kind: Kind::Str(s), ..
                }) if is_p(&t, k + 6, ')') => format!("\"{s}\""),
                _ => "argument".to_string(),
            };
            out.push((t[k].line, format!("Command::new({arg})")));
        }
    }
    out
}

/// One throwaway source per way past the first version of the guards (#1783): `(what, file, source)`. The guards must
/// report each of them in a file that has no allowance. `file` is where the source pretends to live (a `#[path]` is
/// read against it).
pub fn evasions() -> Vec<(&'static str, &'static str, String)> {
    let new = "src/some/new_file.rs";
    let s = |x: &str| x.to_string();
    let mut v: Vec<(&'static str, &'static str, String)> = vec![
        (
            "a // inside a string before the call",
            new,
            s("let _u = \"//\"; unsafe { libc::connect(fd, a, 3) };\n"),
        ),
        (
            "a URL string before the call",
            new,
            s("let u = \"https://example.com\"; let r = libc::socket(2, 1, 0);\n"),
        ),
        (
            "a block comment holding // before the call",
            new,
            s("/* // */ unsafe { libc::connect(1, 2, 3) };\n"),
        ),
        (
            "a char literal of a slash then //",
            new,
            s("let c = '/'; let u = \"//\"; libc::bind(1, 2, 3);\n"),
        ),
        (
            "a raw string holding //",
            new,
            s("let u = r#\"//\"#; libc::listen(1, 2);\n"),
        ),
        (
            "syscall with SYS_socket",
            new,
            s("unsafe { libc::syscall(libc::SYS_socket, 2, 1, 0) };\n"),
        ),
        (
            "syscall with a number",
            new,
            s("unsafe { libc::syscall(41, 2, 1, 0) };\n"),
        ),
        ("SYS_connect alone", new, s("let n = libc::SYS_connect;\n")),
        (
            "an alias from use libc as",
            new,
            s("use libc as l;\nfn f() { unsafe { l::socket(1, 2, 3) }; }\n"),
        ),
        (
            "an alias from extern crate",
            new,
            s("extern crate libc as l;\nfn f() { unsafe { l::connect(1, 2, 3) }; }\n"),
        ),
        (
            "an alias from use libc::{self as l}",
            new,
            s("use libc::{self as l};\nfn f() { unsafe { l::bind(1, 2, 3) }; }\n"),
        ),
        (
            "a leading :: path",
            new,
            s("fn f() { unsafe { ::libc::connect(1, 2, 3) }; }\n"),
        ),
        ("a leading :: in a use", new, s("use ::libc::{socket};\n")),
        ("two spaces after use", new, s("use  libc::{socket};\n")),
        (
            "spaces around ::",
            new,
            s("fn f() { unsafe { libc :: socket(1, 2, 3) }; }\n"),
        ),
        (
            "a path split over lines",
            new,
            s("fn f() { unsafe { libc\n        ::\n        socket(1, 2, 3) }; }\n"),
        ),
        (
            "a use list with a trailing comma",
            new,
            s("use libc::{ socket , };\n"),
        ),
        (
            "a nested use list",
            new,
            s("use libc::{c_int, {connect, pid_t}};\n"),
        ),
        ("an import with as", new, s("use libc::socket as mk;\n")),
        ("a glob import", new, s("use libc::*;\n")),
        (
            "extern with no ABI",
            new,
            s("extern {\n    fn socket(a: i32, b: i32, c: i32) -> i32;\n}\n"),
        ),
        (
            "unsafe extern with no ABI",
            new,
            s("unsafe extern {\n    fn connect(a: i32, b: *const u8, c: u32) -> i32;\n}\n"),
        ),
        (
            "extern with two spaces",
            new,
            s("extern  \"C\"  {\n    fn\tsocket(a: i32) -> i32;\n}\n"),
        ),
        (
            "extern with a system ABI",
            new,
            s("unsafe extern \"system\" {\n    fn bind(a: i32) -> i32;\n}\n"),
        ),
        (
            "a link_name",
            new,
            s(
                "extern \"C\" {\n    #[link_name = \"socket\"]\n    fn mk(a: i32, b: i32, c: i32) -> i32;\n}\n",
            ),
        ),
        (
            "Command under another name",
            new,
            s("use std::process::Command as C;\nfn f() { C::new(\"curl\"); }\n"),
        ),
        ("a glob of std::process", new, s("use std::process::*;\n")),
        (
            "Command in a nested use",
            new,
            s("use std::process::{Child, Command as Run};\n"),
        ),
        (
            "libc::execvp",
            new,
            s("fn f() { unsafe { libc::execvp(a, b) }; }\n"),
        ),
        (
            "libc::execve",
            new,
            s("fn f() { unsafe { libc::execve(a, b, c) }; }\n"),
        ),
        (
            "libc::system",
            new,
            s("fn f() { unsafe { libc::system(c) }; }\n"),
        ),
        (
            "libc::popen",
            new,
            s("fn f() { unsafe { libc::popen(a, b) }; }\n"),
        ),
        (
            "libc::posix_spawnp",
            new,
            s("fn f() { unsafe { libc::posix_spawnp(a, b, c, d, e, f) }; }\n"),
        ),
        ("nix in braces", new, s("use nix::{sys::socket};\n")),
        ("rustix in braces", new, s("use rustix::{net::connect};\n")),
        (
            "nix by path",
            new,
            s("fn f() { nix::sys::socket::socket(a, b, c, d); }\n"),
        ),
        (
            "std::os::unix::net in braces",
            new,
            s("use std::os::unix::{net};\n"),
        ),
        (
            "std::os::unix::net with a type",
            new,
            s("use std::os::unix::net::{UnixStream};\n"),
        ),
        ("tokio in braces", new, s("use tokio::{net::TcpStream};\n")),
        (
            "libc connectx",
            new,
            s("fn f() { unsafe { libc::connectx(1, 2, 3) }; }\n"),
        ),
        (
            "libc sendmsg_x",
            new,
            s("fn f() { unsafe { libc::sendmsg_x(1, 2, 3) }; }\n"),
        ),
        (
            "libc paccept",
            new,
            s("fn f() { unsafe { libc::paccept(1, 2, 3, 4) }; }\n"),
        ),
        ("libc socklen_t", new, s("let n: libc::socklen_t = 0;\n")),
        ("libc INADDR_ANY", new, s("let a = libc::INADDR_ANY;\n")),
        (
            "libc NETLINK_ROUTE",
            new,
            s("let p = libc::NETLINK_ROUTE;\n"),
        ),
        (
            "libc getifaddrs",
            new,
            s("fn f() { unsafe { libc::getifaddrs(p) }; }\n"),
        ),
        (
            "libc res_init",
            new,
            s("fn f() { unsafe { libc::res_init() }; }\n"),
        ),
        ("libc SIOCGIFADDR", new, s("let r = libc::SIOCGIFADDR;\n")),
        (
            "a macro that fills in the libc name",
            new,
            s("macro_rules! m { ($f:ident) => { unsafe { libc::$f(1, 2, 3) } }; }\n"),
        ),
        (
            "inline assembly",
            new,
            s("fn f() { unsafe { std::arch::asm!(\"syscall\"); } }\n"),
        ),
        (
            "dlsym",
            new,
            s("fn f() { let p = unsafe { libc::dlsym(h, name) }; }\n"),
        ),
        (
            "dlopen",
            new,
            s("fn f() { let h = unsafe { libc::dlopen(p, 1) }; }\n"),
        ),
    ];
    for call in LIBC_NET_CALLS {
        v.push((
            "every libc call by path",
            new,
            format!("fn f() {{ unsafe {{ libc::{call}(1, 2, 3) }}; }}\n"),
        ));
        v.push((
            "every libc call by import",
            new,
            format!("use libc::{{c_int, {call}}};\n"),
        ));
        v.push((
            "every libc call by import over lines",
            new,
            format!("use libc::{{\n    c_int,\n    {call} as raw,\n}};\n"),
        ));
        v.push((
            "every libc call by extern declaration",
            new,
            format!("unsafe extern \"C\" {{\n    fn {call}(a: i32) -> i32;\n}}\n"),
        ));
    }
    for call in LIBC_SPAWN_CALLS {
        v.push((
            "every libc spawn call by path",
            new,
            format!("fn f() {{ unsafe {{ libc::{call}(1, 2, 3) }}; }}\n"),
        ));
        v.push((
            "every libc spawn call by import",
            new,
            format!("use libc::{call};\n"),
        ));
    }
    for p in LIBC_NET_PREFIXES {
        v.push((
            "every libc socket prefix",
            new,
            format!("let x = libc::{p}FOO;\n"),
        ));
    }
    for n in LIBC_NET_NAMES {
        v.push((
            "every libc socket name",
            new,
            format!("let x: libc::{n} = 0;\n"),
        ));
    }
    for m in NET_MODULES {
        v.push((
            "every network module",
            new,
            format!("fn f() {{ {}::thing(); }}\n", m.join("::")),
        ));
        v.push((
            "every network module in a use",
            new,
            format!("use {}::thing;\n", m.join("::")),
        ));
    }
    for r in RESOLVERS {
        v.push(("every resolver", new, format!("fn f() {{ x.{r}(); }}\n")));
    }
    v
}

/// Throwaway sources that pull the protected file `target` (a path relative to the crate, like `src/a/tests.rs`) into
/// another file with `#[path]` or `include!`: from its own directory, and from a sibling directory with `..`.
pub fn path_evasions(target: &str) -> Vec<(&'static str, String, String)> {
    let (dir, name) = target.rsplit_once('/').unwrap();
    let root = target.split('/').next().unwrap();
    let below = &target[root.len() + 1..];
    vec![
        (
            "a #[path] to a protected file from its own directory",
            format!("{dir}/zz_includer.rs"),
            format!("#[path = \"{name}\"]\nmod shipped_copy;\n"),
        ),
        (
            "a #[path] with .. to a protected file",
            format!("{root}/zz_other/zz_includer.rs"),
            format!("#[path = \"../{below}\"] mod shipped_copy;\n"),
        ),
        (
            "an include! of a protected file",
            format!("{dir}/zz_includer.rs"),
            format!("include!(\"{name}\");\n"),
        ),
    ]
}

/// Sources the guards must not report: the same words as strings and comments, and libc names that make no network use.
pub fn clean_sources() -> Vec<(&'static str, String)> {
    let s = |x: &str| x.to_string();
    vec![
        (
            "a comment",
            s("// libc::socket(a, b, c) is what the guard looks for\nfn f() {}\n"),
        ),
        ("a doc comment", s("/// says libc::connect\nfn f() {}\n")),
        (
            "a block comment",
            s("/* libc::bind(1, 2, 3) */ fn f() {}\n"),
        ),
        (
            "a nested block comment",
            s("/* a /* libc::listen(1, 2) */ b */ fn f() {}\n"),
        ),
        (
            "a string",
            s("fn f() { let s = \"libc::socket(1, 2, 3)\"; }\n"),
        ),
        (
            "a raw string",
            s("fn f() { let s = r#\"use libc::*; libc::connect\"#; }\n"),
        ),
        (
            "a URL",
            s("fn f() { let u = \"https://example.com/path\"; }\n"),
        ),
        (
            "mkfifo",
            s("fn f() { unsafe { libc::mkfifo(p, 0o600) }; }\n"),
        ),
        ("tcgetsid", s("fn f() { unsafe { libc::tcgetsid(fd) }; }\n")),
        ("plain libc imports", s("use libc::{c_int, pid_t};\n")),
        (
            "an alias used for a plain call",
            s("use libc as l;\nfn f() { unsafe { l::mkfifo(p, 0o600) }; }\n"),
        ),
        (
            "an extern callback",
            s("extern \"C\" fn restore(sig: libc::c_int) {}\n"),
        ),
        (
            "an extern block of other functions",
            s("unsafe extern \"C\" {\n    fn getpid() -> i32;\n}\n"),
        ),
        (
            "names that only look alike",
            s("fn socket_path() {}\nfn connect_timeout() {}\nlet bind_address = 1;\n"),
        ),
        (
            "std::net addresses",
            s("use std::net::{IpAddr, SocketAddr};\n"),
        ),
        (
            "Command with its own name",
            s("use std::process::{Child, Command, Stdio};\n"),
        ),
        (
            "a constant of a plain type",
            s("let m = libc::c_int::MAX;\n"),
        ),
    ]
}

/// The strings of the source (the text between the quotes, as written).
pub fn strings(src: &str) -> Vec<String> {
    tokenize(src)
        .into_iter()
        .filter_map(|t| match t.kind {
            Kind::Str(s) => Some(s),
            _ => None,
        })
        .collect()
}

/// Lexer edge cases with what rustc reads in them: `(what, source, identifiers, strings)`. Every string and char literal
/// form, nested comments, raw identifiers, lifetimes against chars. A lexer that gets out of step after one of them hides
/// the code that follows (#1783).
pub fn lexer_cases() -> Vec<(
    &'static str,
    &'static str,
    Vec<&'static str>,
    Vec<&'static str>,
)> {
    vec![
        (
            "an escaped quote in a string",
            "let a = \"x\\\"y\"; let z = 1;",
            vec!["let", "a", "let", "z"],
            vec!["x\\\"y"],
        ),
        (
            "a backslash at the end of a string",
            "let a = \"x\\\\\"; tail",
            vec!["let", "a", "tail"],
            vec!["x\\\\"],
        ),
        ("a raw string", "r\"a\\\"; tail", vec!["tail"], vec!["a\\"]),
        (
            "a raw string with one hash holding a quote",
            "r#\"a\"b\"#; tail",
            vec!["tail"],
            vec!["a\"b"],
        ),
        (
            "a raw string with two hashes holding a quote and one hash",
            "r##\"a\"#b\"##; tail",
            vec!["tail"],
            vec!["a\"#b"],
        ),
        (
            "a raw string with three hashes",
            "r###\"a\"##b\"###; tail",
            vec!["tail"],
            vec!["a\"##b"],
        ),
        (
            "a byte string",
            "b\"x\\\"y\"; tail",
            vec!["tail"],
            vec!["x\\\"y"],
        ),
        (
            "a raw byte string",
            "br#\"a\"b\"#; tail",
            vec!["tail"],
            vec!["a\"b"],
        ),
        (
            "a C string",
            "c\"x\\\"y\"; tail",
            vec!["tail"],
            vec!["x\\\"y"],
        ),
        (
            "an empty C string",
            "let _b = c\"\"; tail",
            vec!["let", "_b", "tail"],
            vec![""],
        ),
        (
            "a raw C string ending in a backslash",
            "let _a = cr\"\\\"; let _b = c\"\"; tail",
            vec!["let", "_a", "let", "_b", "tail"],
            vec!["\\", ""],
        ),
        (
            "a raw C string with a hash",
            "cr#\"a\"b\"#; tail",
            vec!["tail"],
            vec!["a\"b"],
        ),
        ("a byte char", "b'\\''; tail", vec!["tail"], vec![]),
        (
            "a byte char holding a quote",
            "b'\"'; tail",
            vec!["tail"],
            vec![],
        ),
        (
            "a char holding an escaped quote",
            "quotes! { '\\'', '\"' } after",
            vec!["quotes", "after"],
            vec![],
        ),
        (
            "an escaped quote char, a comma and a double quote char, then a string",
            "quotes!{'\\'','\"'} libc_after; let s = \"str\"; last",
            vec!["quotes", "libc_after", "let", "s", "last"],
            vec!["str"],
        ),
        (
            "a char holding a double quote",
            "let c = '\"'; after",
            vec!["let", "c", "after"],
            vec![],
        ),
        (
            "a char holding a backslash",
            "let b = '\\\\'; next",
            vec!["let", "b", "next"],
            vec![],
        ),
        (
            "a unicode escape char",
            "let u = '\\u{1F600}'; next",
            vec!["let", "u", "next"],
            vec![],
        ),
        (
            "a multi-byte char",
            "let e = 'é'; next",
            vec!["let", "e", "next"],
            vec![],
        ),
        (
            "a char holding a slash then a comment",
            "let c = '/'; // x\n y",
            vec!["let", "c", "y"],
            vec![],
        ),
        (
            "lifetimes are not chars",
            "fn f<'a>(x: &'a str) -> char { 'x' }",
            vec!["fn", "f", "x", "str", "char"],
            vec![],
        ),
        (
            "a lifetime before a char",
            "fn f<'a, 'b>() { let c = 'b'; d }",
            vec!["fn", "f", "let", "c", "d"],
            vec![],
        ),
        (
            "a static lifetime",
            "let s: &'static str = \"s\"; after",
            vec!["let", "s", "str", "after"],
            vec!["s"],
        ),
        (
            "a loop label",
            "'outer: loop { break 'outer; } after",
            vec!["loop", "break", "after"],
            vec![],
        ),
        (
            "a raw identifier",
            "let r#type = 1; r#loop(); after",
            vec!["let", "type", "loop", "after"],
            vec![],
        ),
        (
            "a nested block comment",
            "/* a /* b */ still a comment */ real",
            vec!["real"],
            vec![],
        ),
        (
            "a deeper block comment",
            "/* 1 /* 2 /* 3 */ 2 */ 1 */ real",
            vec!["real"],
            vec![],
        ),
        (
            "a block comment with a quote",
            "/* \" */ real",
            vec!["real"],
            vec![],
        ),
        (
            "a line comment with a quote",
            "// \" x\n real",
            vec!["real"],
            vec![],
        ),
        (
            "// in a string",
            "let s = \"//\"; after",
            vec!["let", "s", "after"],
            vec!["//"],
        ),
        (
            "/* in a string",
            "let s = \"/*\"; after",
            vec!["let", "s", "after"],
            vec!["/*"],
        ),
        (
            "/* in a raw string",
            "let s = r#\"/*\"#; after",
            vec!["let", "s", "after"],
            vec!["/*"],
        ),
        (
            "a number with a suffix",
            "let n = 0x1F_u32; after",
            vec!["let", "n", "after"],
            vec![],
        ),
        (
            "a float",
            "let n = 1.5e3; after",
            vec!["let", "n", "after"],
            vec![],
        ),
        (
            "a string over lines",
            "let s = \"a\\nb\"; after",
            vec!["let", "s", "after"],
            vec!["a\\nb"],
        ),
    ]
}

/// Run every lexer case: the identifiers and the strings must be exactly what rustc reads.
pub fn check_lexer_cases() {
    for (what, src, want_ids, want_strs) in lexer_cases() {
        let got: Vec<String> = ids(src).into_iter().map(|(_, s)| s).collect();
        assert_eq!(got, want_ids, "identifiers of {what}: {src}");
        assert_eq!(strings(src), want_strs, "strings of {what}: {src}");
        // strip_comments keeps the strings and loses the comments, on the same lines
        let stripped = strip_comments(src);
        assert_eq!(
            stripped.lines().count(),
            src.lines().count(),
            "lines of {what}"
        );
        let got2: Vec<String> = ids(&stripped).into_iter().map(|(_, s)| s).collect();
        assert_eq!(got2, want_ids, "identifiers of the stripped {what}: {src}");
    }
}

/// The brackets of a Rust source balance token by token; a lexer that is out of step hides or invents some. The token
/// counts of `{`, `(` and `[` must equal those of their closers.
pub fn brackets_balance(src: &str) -> bool {
    let (mut c, mut p, mut b) = (0i64, 0i64, 0i64);
    for t in tokenize(src) {
        match t.kind {
            Kind::P('{') => c += 1,
            Kind::P('}') => c -= 1,
            Kind::P('(') => p += 1,
            Kind::P(')') => p -= 1,
            Kind::P('[') => b += 1,
            Kind::P(']') => b -= 1,
            _ => {}
        }
    }
    c == 0 && p == 0 && b == 0
}

/// A `mod name;` declaration with the attributes that stand directly before it, each as its tokens written without
/// spaces (`cfg(test)`, `path="daemon_tests.rs"`).
#[derive(Debug, Clone, PartialEq)]
pub struct ModDecl {
    pub name: String,
    pub line: usize,
    pub attrs: Vec<String>,
}

fn kind_text(k: &Kind) -> String {
    match k {
        Kind::Id(s) => s.clone(),
        Kind::Str(s) => format!("\"{s}\""),
        Kind::Num => "0".to_string(),
        Kind::P(c) => c.to_string(),
    }
}

/// The `mod name;` declarations of the source (not `mod name { .. }`), read from tokens, so a comment, a string or the
/// spacing of the attributes cannot fake one.
pub fn mod_decls(src: &str) -> Vec<ModDecl> {
    let t = tokenize(src);
    let mut out = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    let mut k = 0;
    while k < t.len() {
        if is_p(&t, k, '#') && is_p(&t, k + 1, '[') {
            let close = close_of(&t, k + 1);
            pending.push(t[k + 2..close].iter().map(|x| kind_text(&x.kind)).collect());
            k = close + 1;
            continue;
        }
        match id(&t, k) {
            Some("mod") if id(&t, k + 1).is_some() && is_p(&t, k + 2, ';') => {
                out.push(ModDecl {
                    name: id(&t, k + 1).unwrap().to_string(),
                    line: t[k].line,
                    attrs: std::mem::take(&mut pending),
                });
                k += 3;
                continue;
            }
            // visibility and `unsafe` may stand between the attributes and `mod`
            Some("pub" | "unsafe" | "crate" | "super" | "self" | "in") => {}
            _ if is_p(&t, k, '(') || is_p(&t, k, ')') => {}
            _ => pending.clear(),
        }
        k += 1;
    }
    out
}

/// Whether the source declares `mod name;` exactly once, directly under `#[cfg(test)]` (other attributes may sit
/// between them), by tokens. A second declaration of the same module, or one without the attribute, is not accepted.
pub fn declared_under_cfg_test(src: &str, name: &str) -> bool {
    declared_with_attr(src, name, "cfg(test)")
}

/// Whether the source declares `mod name;` exactly once and with the attribute `attr` (written without spaces, like
/// `cfg(all(unix,feature="client-socket"))`) among those directly before it.
pub fn declared_with_attr(src: &str, name: &str, attr: &str) -> bool {
    let want: String = tokenize(attr).iter().map(|x| kind_text(&x.kind)).collect();
    let decls: Vec<ModDecl> = mod_decls(src)
        .into_iter()
        .filter(|d| d.name == name)
        .collect();
    decls.len() == 1 && decls[0].attrs.contains(&want)
}

/// The `#[path = "file"] mod name;` declarations of the source: the target as written and the attributes before it.
pub fn path_decls(src: &str) -> Vec<(String, Vec<String>)> {
    mod_decls(src)
        .into_iter()
        .filter_map(|d| {
            let target = d.attrs.iter().find_map(|a| {
                a.strip_prefix("path=\"")
                    .and_then(|r| r.strip_suffix('"'))
                    .map(str::to_string)
            })?;
            Some((target, d.attrs))
        })
        .collect()
}

/// How a program is started in the source.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandUse {
    /// `Command::new(...)`, also `process::Command::new(` and `<Command>::new(`: the literal program, if it is one.
    Call {
        line: usize,
        program: Option<String>,
        in_test: bool,
    },
    /// Anything else that makes or passes a program starter: `Command::new` not called at once (`.map(Command::new)`,
    /// `let mk = Command::new;`), and a `type` item that names `Command`.
    Indirect { line: usize, what: String },
}

/// Whether token `k` is inside an item marked `#[cfg(test)]` (a module, a function, an impl): the attribute, then the item
/// up to its `;` or past its `{ .. }`.
pub fn in_test_item(t: &[Tok], k: usize) -> bool {
    let mut j = 0;
    while j < t.len() {
        if is_p(t, j, '#')
            && is_p(t, j + 1, '[')
            && id(t, j + 2) == Some("cfg")
            && is_p(t, j + 3, '(')
            && id(t, j + 4) == Some("test")
            && is_p(t, j + 5, ')')
            && is_p(t, j + 6, ']')
        {
            let start = j;
            let mut m = j + 7;
            while is_p(t, m, '#') && is_p(t, m + 1, '[') {
                m = close_of(t, m + 1) + 1;
            }
            let mut end = t.len();
            let mut q = m;
            while q < t.len() {
                match t[q].kind {
                    Kind::P(';') => {
                        end = q;
                        break;
                    }
                    Kind::P('{') => {
                        end = close_of(t, q);
                        break;
                    }
                    Kind::P('(' | '[') => q = close_of(t, q) + 1,
                    _ => q += 1,
                }
            }
            if start <= k && k <= end {
                return true;
            }
            j = end.max(j + 1);
        } else {
            j += 1;
        }
    }
    false
}

/// Every place the source starts, or could start, a program through `Command` (by identifier, not by text).
pub fn command_uses(src: &str) -> Vec<CommandUse> {
    let t = tokenize(src);
    let mut out = Vec::new();
    for k in 0..t.len() {
        if id(&t, k) != Some("Command") {
            continue;
        }
        // `Command :: new`, or `< Command > :: new`
        let after = if is_p(&t, k + 1, '>') { k + 2 } else { k + 1 };
        if is_sep(&t, after) && id(&t, after + 2) == Some("new") {
            if is_p(&t, after + 3, '(') {
                let program = match t.get(after + 4) {
                    Some(Tok {
                        kind: Kind::Str(s), ..
                    }) if is_p(&t, after + 5, ')') => Some(s.clone()),
                    _ => None,
                };
                out.push(CommandUse::Call {
                    line: t[k].line,
                    program,
                    in_test: in_test_item(&t, k),
                });
            } else {
                out.push(CommandUse::Indirect {
                    line: t[k].line,
                    what: "Command::new not called at once".to_string(),
                });
            }
        }
    }
    // a `type X = ... Command ...;`
    for k in 0..t.len() {
        if id(&t, k) == Some("type")
            && !is_p(&t, k.wrapping_sub(1), '.')
            && let Some(end) = (k..t.len()).find(|&j| is_p(&t, j, ';'))
            && (k..end).any(|j| id(&t, j) == Some("Command"))
        {
            out.push(CommandUse::Indirect {
                line: t[k].line,
                what: "a type that names Command".to_string(),
            });
        }
    }
    out
}

/// The lines where `ty` (one of `types`, also written `<ty>`) is followed by `::method`, with any spacing and over any
/// number of lines: `TcpStream::connect`, `<TcpListener>::bind`, `loopback::Stream :: connect`.
pub fn assoc_calls(src: &str, types: &[&str], method: &str) -> Vec<usize> {
    let t = tokenize(src);
    let mut out = Vec::new();
    for k in 0..t.len() {
        let Some(name) = id(&t, k) else { continue };
        if !types.contains(&name) {
            continue;
        }
        let after = if is_p(&t, k + 1, '>') { k + 2 } else { k + 1 };
        if is_sep(&t, after) && id(&t, after + 2) == Some(method) {
            out.push(t[k].line);
        }
    }
    out
}

/// The crate's own Rust sources to scan, by the rule of the guards: everything under `src`, `tests` and `fuzz`, and
/// `build.rs`. Only the build output is skipped, and only where cargo puts it: `<root>/target`, and the cargo-fuzz
/// directories `<root>/fuzz/{target,corpus,artifacts}`. A directory of that name anywhere else (under `src`, say) is
/// ordinary source and is read.
pub fn crate_sources(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    fn walk(
        dir: &std::path::Path,
        skip: &dyn Fn(&std::path::Path) -> bool,
        out: &mut Vec<std::path::PathBuf>,
    ) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        for e in read.flatten() {
            let p = e.path();
            if p.is_dir() {
                if !skip(&p) {
                    walk(&p, skip, out);
                }
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let skip = |p: &std::path::Path| {
        p == root.join("target")
            || ["target", "corpus", "artifacts"]
                .iter()
                .any(|n| p == root.join("fuzz").join(n))
    };
    let mut files = Vec::new();
    for d in ["src", "tests", "fuzz"] {
        walk(&root.join(d), &skip, &mut files);
    }
    if root.join("build.rs").is_file() {
        files.push(root.join("build.rs"));
    }
    files.sort();
    files
}
