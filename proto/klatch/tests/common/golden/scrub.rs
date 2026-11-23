//! What changes from run to run in the text of a command, rewritten to fixed words: the throwaway home and the
//! binary path (given by the test), the terminal label, process ids, ages, waits and lifetimes of a listing.

use std::path::Path;

/// A value that follows a key in the text (`terminal=ttys003/s4121`): everything up to the end of the token.
struct Keyed {
    key: &'static str,
    word: &'static str,
}

/// Digits between two markers (`wait 29s terminal=`): the digits only.
struct Between {
    open: &'static str,
    close: &'static str,
    word: &'static str,
}

const fn keyed(key: &'static str, word: &'static str) -> Keyed {
    Keyed { key, word }
}

const KEYED: [Keyed; 7] = [
    keyed("terminal=", "<TTY>"),
    keyed("pid ", "<PID>"),
    keyed("pid=", "<PID>"),
    keyed("now=", "<UNIX>"),
    keyed("try again in ", "<N>"),
    keyed("expires in ", "<N>"),
    keyed("cap in ", "<N>"),
];

const BETWEEN: [Between; 2] = [
    Between {
        open: "opened ",
        close: "s ago",
        word: "<N>",
    },
    Between {
        open: "wait ",
        close: "s ",
        word: "<N>",
    },
];

/// What to replace in a text, built per test from the throwaway paths it used.
#[derive(Default)]
pub struct Scrub {
    pairs: Vec<(String, &'static str)>,
}

impl Scrub {
    pub fn new() -> Self {
        Scrub::default()
    }

    /// Replace every `path` (and its `/private` twin, which macOS gives a temp directory) with `word`.
    pub fn path(mut self, path: &Path, word: &'static str) -> Self {
        self.add(path, word);
        self
    }

    /// As `path`, on a scrub already built (a test that makes a new throwaway directory per scenario).
    pub fn add(&mut self, path: &Path, word: &'static str) {
        let p = path.to_string_lossy().into_owned();
        if let Ok(real) = path.canonicalize() {
            self.pairs.push((real.to_string_lossy().into_owned(), word));
        }
        self.pairs.push((format!("/private{p}"), word));
        self.pairs.push((p, word));
    }

    /// Replace the product binary's path.
    pub fn binary(self) -> Self {
        self.path(Path::new(env!("CARGO_BIN_EXE_klatch")), "<KLATCH>")
    }

    /// Apply the path replacements (longest first) and then the volatile-token rules.
    pub fn apply(&self, text: &str) -> String {
        let mut pairs: Vec<&(String, &'static str)> = self.pairs.iter().collect();
        pairs.sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
        let mut out = text.replace('\r', "").replace('\u{1b}', "<ESC>");
        for (from, to) in pairs {
            out = out.replace(from.as_str(), to);
        }
        out.split_inclusive('\n').map(volatile_line).collect()
    }
}

/// A character that ends a token: whitespace or a character that cannot be part of a label.
fn is_end(c: char) -> bool {
    c.is_whitespace() || matches!(c, ',' | ';' | ')' | '"' | '`' | '\'')
}

/// Replace the value after each key (up to the end of the token) with the word.
fn after_key(line: &str, rule: &Keyed) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = rest.find(rule.key) {
        out.push_str(&rest[..i + rule.key.len()]);
        let tail = &rest[i + rule.key.len()..];
        let end = tail.find(is_end).unwrap_or(tail.len());
        if end > 0 {
            out.push_str(rule.word);
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Replace the digits that sit between the markers with the word.
fn digits_between(line: &str, rule: &Between) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = rest.find(rule.open) {
        let tail = &rest[i + rule.open.len()..];
        let n = tail.chars().take_while(char::is_ascii_digit).count();
        out.push_str(&rest[..i + rule.open.len()]);
        if n > 0 && tail[n..].starts_with(rule.close) {
            out.push_str(rule.word);
        } else {
            out.push_str(&tail[..n]);
        }
        rest = &tail[n..];
    }
    out.push_str(rest);
    out
}

/// A listing line starts with its age (`12s ago<TAB>`): replaced by `<AGE>s ago`.
fn age_of(line: &str) -> String {
    let n = line.chars().take_while(char::is_ascii_digit).count();
    if n > 0 && line[n..].starts_with("s ago\t") {
        format!("<AGE>{}", &line[n..])
    } else {
        line.to_string()
    }
}

/// One line with the volatile tokens replaced: ages, terminal labels, process ids, waits and lifetimes.
fn volatile_line(line: &str) -> String {
    let mut s = age_of(line);
    for rule in &KEYED {
        s = after_key(&s, rule);
    }
    for rule in &BETWEEN {
        s = digits_between(&s, rule);
    }
    s
}
