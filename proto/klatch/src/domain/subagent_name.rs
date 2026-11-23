//! Does a shell command run klatch without naming itself (#1152)? A subagent the harness runs inside the session's own
//! process has no process of its own and no agent id in its environment, so a klatch command it runs would resolve to
//! the parent's member. The PreToolUse hook does see `agent_id`, so it refuses such a command from a subagent unless
//! the command names the subagent itself. The command text is data: it is tokenized, never run.

/// True when `command` invokes klatch and sets no name for itself: no `KLATCH_NAME=<name>` and no `--as-member`.
pub fn invokes_klatch_unnamed(command: &str) -> bool {
    let (klatch, named) = scan(command, 0);
    klatch && !named
}

/// How deep `sh -c '...'` strings are looked into.
const MAX_DEPTH: usize = 4;

/// Words that may stand before the real command word.
const WRAPPERS: [&str; 10] = [
    "env", "command", "exec", "time", "nohup", "nice", "sudo", "builtin", "!", "xargs",
];

/// Shells whose `-c` string is looked into.
const SHELLS: [&str; 5] = ["sh", "bash", "zsh", "dash", "ksh"];

/// (a command word is klatch, a word names the member) over the whole text.
fn scan(command: &str, depth: usize) -> (bool, bool) {
    let (mut klatch, mut named) = (false, false);
    for words in segments(command) {
        named |= words.iter().any(|w| names_member(w));
        let (k, n) = scan_simple(&words, depth);
        klatch |= k;
        named |= n;
    }
    (klatch, named)
}

/// A word that may stand before the real command word: an assignment, a wrapper or a flag of a wrapper.
fn is_prefix_word(word: &str) -> bool {
    is_assignment(word) || WRAPPERS.contains(&word) || (word.starts_with('-') && word.len() > 1)
}

/// (the command word is klatch, a shell string it runs names the member) for one simple command.
fn scan_simple(words: &[String], depth: usize) -> (bool, bool) {
    let mut rest = words
        .iter()
        .map(String::as_str)
        .skip_while(|w| is_prefix_word(w));
    let Some(cmd) = rest.next() else {
        return (false, false);
    };
    let base = cmd.rsplit('/').next().unwrap_or(cmd);
    if base == "klatch" {
        (true, false)
    } else if depth < MAX_DEPTH && SHELLS.contains(&base) {
        scan_dash_c(rest, depth)
    } else {
        (false, false)
    }
}

/// `bash -c 'klatch ...'`: the command is the word after the flags that include c.
fn scan_dash_c<'a>(args: impl Iterator<Item = &'a str>, depth: usize) -> (bool, bool) {
    let mut flags_have_c = false;
    for w in args {
        if !w.starts_with('-') {
            return if flags_have_c {
                scan(w, depth + 1)
            } else {
                (false, false)
            };
        }
        flags_have_c |= w.len() > 1 && !w.starts_with("--") && w.contains('c');
    }
    (false, false)
}

/// `KLATCH_NAME=<name>` (a non-empty name) or `--as-member <name>` / `--as-member=<name>`; the word after a bare
/// `--as-member` is not checked, a typed flag with no name fails in klatch itself.
fn names_member(word: &str) -> bool {
    word.strip_prefix("KLATCH_NAME=")
        .is_some_and(|n| !n.is_empty())
        || word == "--as-member"
        || word
            .strip_prefix("--as-member=")
            .is_some_and(|n| !n.is_empty())
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        name.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// The simple commands of a shell text as lists of words with quotes removed. Quoted text is one word and its
/// separators do not split; an unquoted `;`, `&`, `|`, newline, parenthesis or backtick ends a command. Not a shell:
/// it never expands anything and reads only enough to find the command word of each part.
fn segments(command: &str) -> Vec<Vec<String>> {
    let mut split = Splitter::default();
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        split.feed(c, &mut chars);
    }
    split.finish()
}

/// The state of `segments`: finished commands, the words of the open one, the open word and the open quote.
#[derive(Default)]
struct Splitter {
    out: Vec<Vec<String>>,
    words: Vec<String>,
    word: String,
    in_word: bool,
    quote: Option<char>,
}

impl Splitter {
    fn end_word(&mut self) {
        if self.in_word {
            self.words.push(std::mem::take(&mut self.word));
            self.in_word = false;
        }
    }

    fn end_command(&mut self) {
        self.end_word();
        if !self.words.is_empty() {
            self.out.push(std::mem::take(&mut self.words));
        }
    }

    /// The character after a backslash, taken literally.
    fn escaped(&mut self, chars: &mut std::str::Chars) {
        if let Some(n) = chars.next() {
            self.word.push(n);
        }
    }

    fn feed(&mut self, c: char, chars: &mut std::str::Chars) {
        match self.quote {
            Some(q) if c == q => self.quote = None,
            Some('"') if c == '\\' => self.escaped(chars),
            Some(_) => self.word.push(c),
            None => self.feed_unquoted(c, chars),
        }
    }

    fn feed_unquoted(&mut self, c: char, chars: &mut std::str::Chars) {
        match c {
            '\'' | '"' => {
                self.quote = Some(c);
                self.in_word = true;
            }
            '\\' => {
                self.in_word = true;
                self.escaped(chars);
            }
            c if c.is_whitespace() && c != '\n' => self.end_word(),
            ';' | '&' | '|' | '\n' | '(' | ')' | '`' => self.end_command(),
            c => {
                self.in_word = true;
                self.word.push(c);
            }
        }
    }

    fn finish(mut self) -> Vec<Vec<String>> {
        self.end_command();
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::invokes_klatch_unnamed as unnamed;

    /// Every command in `commands` is (or is not, per `want`) an unnamed klatch command.
    fn check(commands: &[&str], want: bool) {
        for c in commands {
            assert_eq!(unnamed(c), want, "{c}");
        }
    }

    #[test]
    fn a_bare_klatch_command_is_unnamed() {
        check(
            &[
                "klatch post lobby hi",
                "./proto/target/debug/klatch --discover join lobby",
                "/usr/local/bin/klatch who lobby",
            ],
            true,
        );
    }

    #[test]
    fn a_command_that_sets_its_name_is_named() {
        check(
            &[
                "KLATCH_NAME=t1-1 klatch post lobby hi",
                "env KLATCH_NAME=t1-1 klatch post lobby hi",
                "export KLATCH_NAME=t1-1 && klatch post lobby hi",
                "klatch --as-member t1-1 post lobby hi",
                "klatch --as-member=t1-1 post lobby hi",
            ],
            false,
        );
    }

    #[test]
    fn an_empty_name_does_not_count() {
        check(
            &[
                "KLATCH_NAME= klatch post lobby hi",
                "KLATCH_NAME=\"\" klatch post lobby hi",
            ],
            true,
        );
    }

    #[test]
    fn a_command_that_is_not_klatch_is_never_flagged() {
        check(
            &[
                "cargo test",
                "git commit -m 'fix klatch post'",
                "grep klatch README.md",
                "echo \"run klatch post; later\"",
                "ls proto/klatch",
                "cat klatch.toml",
                "",
            ],
            false,
        );
    }

    #[test]
    fn klatch_anywhere_in_a_pipeline_or_list_is_found() {
        check(
            &[
                "cd proto && klatch who lobby",
                "echo hi | klatch post lobby -",
                "true; klatch who lobby",
                "cd x\nklatch who lobby",
                "x=$(klatch who lobby)",
                "echo `klatch who lobby`",
                "(klatch who lobby)",
            ],
            true,
        );
    }

    #[test]
    fn a_name_anywhere_in_the_text_counts_as_the_subagent_naming_itself() {
        // conservative toward allowing a command that names itself somewhere: a subagent that wrote KLATCH_NAME=<name>
        // in its command has understood the rule
        check(&["KLATCH_NAME=a klatch who lobby; klatch who lobby"], false);
    }

    #[test]
    fn wrappers_and_shell_dash_c_are_looked_through() {
        check(
            &[
                "env FOO=1 klatch who lobby",
                "FOO=1 klatch who lobby",
                "time klatch who lobby",
                "command klatch who lobby",
                "bash -c 'klatch who lobby'",
                "sh -c \"cd x && klatch who lobby\"",
            ],
            true,
        );
        check(
            &[
                "bash -c 'KLATCH_NAME=a klatch who lobby'",
                "bash -c 'cargo test'",
            ],
            false,
        );
    }

    #[test]
    fn a_quoted_separator_does_not_start_a_command() {
        check(
            &["echo 'a; klatch post'", "echo \"a | klatch post\""],
            false,
        );
    }
}
