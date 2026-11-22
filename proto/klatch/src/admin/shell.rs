//! `klatch admin shell` (#1233): the one long-lived process there is. It was started with the passphrase (the
//! proof of `run`), and it reads `admin`, `worker`, `anchor`, `run` and `judge` commands from the terminal, each
//! going through the same `prove::require` as on a command line. The unlock window (`klatch::proof::PROCESS_WINDOW`)
//! is this process's memory, so inside it those commands do not ask again and after it they do. Nothing is written
//! to disk and no other process can use it: it is not inherited by a child, and a second `klatch` starts locked.
//!
//! It runs nothing else: no other command, no shell escape, no environment or file read of a passphrase.

use super::*;
use crate::{Cli, Cmd};
use clap::Parser;

/// Split a line into words: spaces separate, single quotes keep everything, double quotes keep everything but `\"`
/// and `\\`, and a backslash outside quotes escapes one character. An unterminated quote is an error.
pub(super) fn split_words(line: &str) -> std::result::Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            '\'' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(ch) => cur.push(ch),
                        None => return Err("a ' was never closed".into()),
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(e @ ('"' | '\\')) => cur.push(e),
                            Some(e) => {
                                cur.push('\\');
                                cur.push(e);
                            }
                            None => return Err("a \" was never closed".into()),
                        },
                        Some(ch) => cur.push(ch),
                        None => return Err("a \" was never closed".into()),
                    }
                }
            }
            '\\' => {
                started = true;
                match chars.next() {
                    Some(e) => cur.push(e),
                    None => return Err("a line cannot end in a backslash".into()),
                }
            }
            c => {
                started = true;
                cur.push(c);
            }
        }
    }
    if started {
        words.push(cur);
    }
    Ok(words)
}

/// What one line of the shell does.
pub(super) enum Line {
    Nothing,
    Leave,
    Run(Vec<String>),
}

pub(super) fn read_line(line: &str) -> std::result::Result<Line, String> {
    let words = split_words(line)?;
    Ok(match words.first().map(String::as_str) {
        None => Line::Nothing,
        Some("exit" | "quit") if words.len() == 1 => Line::Leave,
        Some(_) => Line::Run(words),
    })
}

/// Run one command line (the words after `klatch`) if it is one of the five families.
fn dispatch(words: Vec<String>) -> Result<()> {
    let mut argv = vec!["klatch".to_string()];
    argv.extend(words);
    let cli = match Cli::try_parse_from(argv) {
        Ok(c) => c,
        Err(e) => {
            // help and usage errors print and the shell goes on
            let _ = e.print();
            return Ok(());
        }
    };
    match cli.cmd {
        Cmd::Admin {
            cmd: AdminCmd::Shell,
        } => bail!("already in the shell"),
        Cmd::Admin { cmd } => super::run(cmd),
        Cmd::Worker { cmd } => crate::worker::run(cmd),
        Cmd::Anchor { cmd } => crate::anchor::run(cmd),
        Cmd::Run { cmd } => crate::run::run(cmd),
        Cmd::Judge { cmd } => crate::judge::run(cmd),
        _ => bail!("only admin, worker, anchor, run and judge commands run in the shell"),
    }
}

/// The shell loop. The caller already holds a proof, so the window is open when this starts.
pub(super) fn run() -> Result<()> {
    println!(
        "klatch admin shell: admin, worker, anchor, run and judge commands; the passphrase is asked again when the unlock window ends; `exit` leaves"
    );
    let stdin = std::io::stdin();
    loop {
        print!("klatch> ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            println!();
            break;
        }
        match read_line(&line) {
            Ok(Line::Nothing) => {}
            Ok(Line::Leave) => break,
            Ok(Line::Run(words)) => {
                if let Err(e) = dispatch(words) {
                    eprintln!("{e:#}");
                }
            }
            Err(e) => eprintln!("{e}"),
        }
    }
    klatch::proof::PROCESS_WINDOW.close();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        split_words(s).unwrap()
    }

    #[test]
    fn words_split_on_spaces_and_keep_quoted_text_whole() {
        assert_eq!(words("close lobby"), ["close", "lobby"]);
        assert_eq!(words("  a   b  "), ["a", "b"]);
        assert_eq!(
            words("scrub-add n 'two words' \"and more\""),
            ["scrub-add", "n", "two words", "and more"]
        );
        assert_eq!(words(r#"x "a \"b\" \\ c""#), ["x", r#"a "b" \ c"#]);
        assert_eq!(words(r"a\ b"), ["a b"]);
        assert_eq!(words("''"), [""]);
        assert!(words("").is_empty());
    }

    #[test]
    fn an_open_quote_or_a_trailing_backslash_is_an_error() {
        assert!(split_words("a 'b").is_err());
        assert!(split_words("a \"b").is_err());
        assert!(split_words("a\\").is_err());
    }

    #[test]
    fn exit_and_quit_leave_and_a_blank_line_does_nothing() {
        assert!(matches!(read_line("exit\n"), Ok(Line::Leave)));
        assert!(matches!(read_line(" quit "), Ok(Line::Leave)));
        assert!(matches!(read_line("   \n"), Ok(Line::Nothing)));
        assert!(matches!(read_line("exit now"), Ok(Line::Run(_))));
        assert!(matches!(read_line("pause"), Ok(Line::Run(_))));
    }

    #[test]
    fn the_shell_runs_only_the_five_families_and_never_itself() {
        // each of these parses as a command that is not one of the five (or is the shell) and so is refused
        for line in [
            "post r hello",
            "claim r src",
            "init",
            "doctor",
            "admin shell",
        ] {
            let w = split_words(line).unwrap();
            let err = dispatch(w).unwrap_err().to_string();
            assert!(
                err.contains("only admin, worker, anchor, run and judge")
                    || err.contains("already in the shell"),
                "{line}: {err}"
            );
        }
    }
}
