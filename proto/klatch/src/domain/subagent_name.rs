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

/// (a command word is klatch, a word names the member) over the whole text.
fn scan(command: &str, depth: usize) -> (bool, bool) {
    let (mut klatch, mut named) = (false, false);
    for words in segments(command) {
        for w in &words {
            named |= names_member(w);
        }
        let mut rest = words.iter().map(String::as_str).peekable();
        while let Some(w) = rest.peek() {
            if is_assignment(w) || WRAPPERS.contains(w) || (w.starts_with('-') && w.len() > 1) {
                rest.next();
            } else {
                break;
            }
        }
        let Some(cmd) = rest.next() else { continue };
        let base = cmd.rsplit('/').next().unwrap_or(cmd);
        if base == "klatch" {
            klatch = true;
        } else if depth < MAX_DEPTH && ["sh", "bash", "zsh", "dash", "ksh"].contains(&base) {
            // `bash -c 'klatch ...'`: the command is the word after the flags that include c
            let mut flags_have_c = false;
            for w in rest {
                if w.starts_with('-') {
                    flags_have_c |= w.len() > 1 && !w.starts_with("--") && w.contains('c');
                } else {
                    if flags_have_c {
                        let (k, n) = scan(w, depth + 1);
                        klatch |= k;
                        named |= n;
                    }
                    break;
                }
            }
        }
    }
    (klatch, named)
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
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = command.chars();
    macro_rules! end_word {
        () => {
            if in_word {
                words.push(std::mem::take(&mut word));
                in_word = false;
            }
        };
    }
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some('"') if c == '\\' => {
                if let Some(n) = chars.next() {
                    word.push(n);
                }
            }
            Some(_) => word.push(c),
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    in_word = true;
                }
                '\\' => {
                    in_word = true;
                    if let Some(n) = chars.next() {
                        word.push(n);
                    }
                }
                c if c.is_whitespace() && c != '\n' => end_word!(),
                ';' | '&' | '|' | '\n' | '(' | ')' | '`' => {
                    end_word!();
                    if !words.is_empty() {
                        out.push(std::mem::take(&mut words));
                    }
                }
                c => {
                    in_word = true;
                    word.push(c);
                }
            },
        }
    }
    if in_word {
        words.push(word);
    }
    if !words.is_empty() {
        out.push(words);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::invokes_klatch_unnamed as unnamed;

    #[test]
    fn a_bare_klatch_command_is_unnamed() {
        assert!(unnamed("klatch post lobby hi"));
        assert!(unnamed("./proto/target/debug/klatch --discover join lobby"));
        assert!(unnamed("/usr/local/bin/klatch who lobby"));
    }

    #[test]
    fn a_command_that_sets_its_name_is_named() {
        assert!(!unnamed("KLATCH_NAME=t1-1 klatch post lobby hi"));
        assert!(!unnamed("env KLATCH_NAME=t1-1 klatch post lobby hi"));
        assert!(!unnamed("export KLATCH_NAME=t1-1 && klatch post lobby hi"));
        assert!(!unnamed("klatch --as-member t1-1 post lobby hi"));
        assert!(!unnamed("klatch --as-member=t1-1 post lobby hi"));
    }

    #[test]
    fn an_empty_name_does_not_count() {
        assert!(unnamed("KLATCH_NAME= klatch post lobby hi"));
        assert!(unnamed("KLATCH_NAME=\"\" klatch post lobby hi"));
    }

    #[test]
    fn a_command_that_is_not_klatch_is_never_flagged() {
        assert!(!unnamed("cargo test"));
        assert!(!unnamed("git commit -m 'fix klatch post'"));
        assert!(!unnamed("grep klatch README.md"));
        assert!(!unnamed("echo \"run klatch post; later\""));
        assert!(!unnamed("ls proto/klatch"));
        assert!(!unnamed("cat klatch.toml"));
        assert!(!unnamed(""));
    }

    #[test]
    fn klatch_anywhere_in_a_pipeline_or_list_is_found() {
        assert!(unnamed("cd proto && klatch who lobby"));
        assert!(unnamed("echo hi | klatch post lobby -"));
        assert!(unnamed("true; klatch who lobby"));
        assert!(unnamed("cd x\nklatch who lobby"));
        assert!(unnamed("x=$(klatch who lobby)"));
        assert!(unnamed("echo `klatch who lobby`"));
        assert!(unnamed("(klatch who lobby)"));
    }

    #[test]
    fn a_name_anywhere_in_the_text_counts_as_the_subagent_naming_itself() {
        // conservative toward allowing a command that names itself somewhere: a subagent that wrote KLATCH_NAME=<name>
        // in its command has understood the rule
        assert!(!unnamed("KLATCH_NAME=a klatch who lobby; klatch who lobby"));
    }

    #[test]
    fn wrappers_and_shell_dash_c_are_looked_through() {
        assert!(unnamed("env FOO=1 klatch who lobby"));
        assert!(unnamed("FOO=1 klatch who lobby"));
        assert!(unnamed("time klatch who lobby"));
        assert!(unnamed("command klatch who lobby"));
        assert!(unnamed("bash -c 'klatch who lobby'"));
        assert!(unnamed("sh -c \"cd x && klatch who lobby\""));
        assert!(!unnamed("bash -c 'KLATCH_NAME=a klatch who lobby'"));
        assert!(!unnamed("bash -c 'cargo test'"));
    }

    #[test]
    fn a_quoted_separator_does_not_start_a_command() {
        assert!(!unnamed("echo 'a; klatch post'"));
        assert!(!unnamed("echo \"a | klatch post\""));
    }
}
