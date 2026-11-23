//! A golden made of several blocks, each one a scenario: a label, then what the terminal or the command showed.

use super::{Scrub, check};
use std::path::Path;

/// A label with every character that is not printable ASCII written as `\u{..}`, so a hostile argument is readable.
fn visible(label: &str) -> String {
    let plain = |c: char| c.is_ascii_graphic() || c == ' ' || c == '\n';
    label
        .chars()
        .map(|c| {
            if plain(c) {
                c.to_string()
            } else {
                format!("\\u{{{:x}}}", c as u32)
            }
        })
        .collect()
}

/// A label and the text under it.
struct Entry<'a> {
    label: &'a str,
    text: &'a str,
}

pub struct Sheet {
    out: String,
    scrub: Scrub,
}

impl Sheet {
    pub fn new(scrub: Scrub) -> Self {
        Sheet {
            out: String::new(),
            scrub,
        }
    }

    /// Also scrub `path` to `word` from now on.
    pub fn scrub_path(&mut self, path: &Path, word: &'static str) {
        self.scrub.add(path, word);
    }

    /// `### label`, the scrubbed text, ending in a newline.
    fn push(&mut self, entry: Entry) {
        let body = self.scrub.apply(entry.text);
        let label = visible(&self.scrub.apply(entry.label));
        self.out.push_str(&format!("### {label}\n{body}"));
        if !body.is_empty() && !body.ends_with('\n') {
            self.out.push('\n');
        }
    }

    /// Add a block: the label, the scrubbed text, and how the command ended.
    pub fn block(&mut self, label: &str, ok: bool, text: &str) {
        self.push(Entry { label, text });
        let end = if ok { "ok" } else { "refused" };
        self.out.push_str(&format!("(ended: {end})\n\n"));
    }

    /// Add a block of text that is not a command's run (a file or a table), with no ending line.
    pub fn note(&mut self, label: &str, text: &str) {
        self.push(Entry { label, text });
        self.out.push('\n');
    }

    /// Compare the whole sheet to the golden `name`.
    #[track_caller]
    pub fn check(self, name: &str, forbidden: &[&str]) {
        check(name, &self.out, forbidden);
    }
}
