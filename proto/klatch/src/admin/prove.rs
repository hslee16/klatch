//! The command line side of the admin proof (#1233): ask for the passphrase at the terminal, set it, change it, reset
//! it. Every admin command, every launcher command (`worker`, `anchor`, `run`, `judge`) and every keystore act calls
//! `require` first and holds the `AdminProof` it returns; the functions that act take it as an argument.
//!
//! The logic (the verifier, the failure limit, the window) is `klatch::proof`; this file only connects it to the
//! terminal, the store and the clock.

use anyhow::{Result, bail};
use klatch::member::{custody, now};
use klatch::proof::{self, AdminProof, At, TtyPrompt};
use klatch::store::SqliteStore;
use std::path::Path;
use std::time::Instant;

fn at() -> At {
    At {
        unix: now(),
        mono: Instant::now(),
    }
}

/// The command line the person is being asked to approve, set by the shell for each line it runs.
static SHELL_LINE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub(crate) fn set_shell_line(line: Option<String>) {
    *SHELL_LINE.lock().unwrap_or_else(|e| e.into_inner()) = line;
}

/// Text that comes from an argument, for a prompt: printable ASCII only. Every other character (a control or escape
/// character, a bidi override, a zero-width or invisible mark, a separator, a tag character, a variation selector, a
/// filler, a combining mark, any non-ASCII letter) is shown as `?`, so nothing in an argument can change what the
/// terminal draws or pass for something else. A whitelist on purpose: a blacklist of bad characters is never done.
pub(crate) fn plain(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '?'
            }
        })
        .collect()
}

/// The longest a word after the command is shown; a longer one is cut and says how long it was and a short digest
/// (of the whole word, which the person cannot check against the part they see: they should read the cut as unseen).
const WORD_LIMIT: usize = 48;

/// The families of command this approves, and so where the command word is.
const FAMILIES: [&str; 5] = ["admin", "worker", "anchor", "run", "judge"];

/// What the passphrase approves, for the prompt: the command being run. Text is whitelisted (`plain`), the family
/// and command words are never cut, a long word is cut with its length and a digest, and a very long command is
/// cut at a word with how many words went. For `admin recover` EVERY word after the command word is hidden
/// (a recovery code however it is split, whatever flags or `--` come before it); a room or member that happens to
/// be named `recover` hides nothing, because only the command word is the trigger.
pub(crate) fn describe_words(words: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let family = words.iter().position(|w| FAMILIES.contains(&w.as_str()));
    let command = family.map(|f| f + 1);
    let hide_from = match (family, command) {
        (Some(f), Some(c))
            if words[f] == "admin" && words.get(c).map(String::as_str) == Some("recover") =>
        {
            Some(c + 1)
        }
        _ => None,
    };
    let protected =
        |i: usize| Some(i) == family || Some(i) == command || (family.is_none() && i < 2);
    let mut out: Vec<String> = Vec::new();
    for (i, w) in words.iter().enumerate() {
        let w = plain(w);
        let shown = if hide_from.is_some_and(|h| i >= h) {
            "<code>".to_string()
        } else if !protected(i) && w.chars().count() > WORD_LIMIT {
            let digest = data_encoding::HEXLOWER.encode(&Sha256::digest(w.as_bytes()));
            format!(
                "{}...({} chars, sha256 {})",
                w.chars().take(WORD_LIMIT - 8).collect::<String>(),
                w.chars().count(),
                &digest[..8]
            )
        } else {
            w
        };
        out.push(shown);
    }
    let first_free = command.map_or(1, |c| c.max(1)) + 1;
    let mut text = String::new();
    for (i, w) in out.iter().enumerate() {
        if i >= first_free && text.chars().count() + w.chars().count() > 200 {
            text.push_str(&format!(" ...(+{} more words)", out.len() - i));
            break;
        }
        if i > 0 {
            text.push(' ');
        }
        text.push_str(w);
    }
    text
}

/// `describe_words` for arguments as the OS gives them: a byte sequence that is not UTF-8 is shown lossily.
pub(crate) fn describe_os(args: &[std::ffi::OsString]) -> String {
    describe_words(
        &args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
    )
}

/// What the passphrase approves right now: the line the shell is running, else this process's own command line.
pub(crate) fn describe() -> String {
    if let Some(line) = SHELL_LINE.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return line;
    }
    describe_os(&std::env::args_os().skip(1).collect::<Vec<_>>())
}

/// The proof of the person, asked for on the terminal unless this process holds an open unlock window. A refusal
/// is an error whose text says what to do and never holds what was typed. The prompt names `what`.
fn require_for(store: &SqliteStore, db: &Path, what: &str) -> Result<AdminProof> {
    let what = format!("{what} [store {}]", plain(&db.to_string_lossy()));
    proof::prove(store, &mut TtyPrompt, at(), &proof::PROCESS_WINDOW, &what).map_err(Into::into)
}

/// `require` for a command that has not opened the store yet: the store and the proof.
pub(crate) fn open_and_require(db: &Path) -> Result<(SqliteStore, AdminProof)> {
    open_and_require_for(db, &describe())
}

pub(crate) fn open_and_require_for(db: &Path, what: &str) -> Result<(SqliteStore, AdminProof)> {
    let store = custody::open_store(db)?;
    let proof = require_for(&store, db, what)?;
    Ok((store, proof))
}

/// Whether the store can be read with the key the keystore holds (without making anything).
pub(crate) fn store_readable(db: &Path) -> bool {
    db.exists()
        && matches!(custody::current_key(db), Ok(Some(k)) if SqliteStore::open(db, &k).is_ok())
}

/// The proof for `recover` (store key) and `restore` when the store cannot be read, so the passphrase verifier
/// cannot either (`klatch::proof` builds it: the recovery code approves, never a bare yes). Both are followed by the
/// typed confirmation. The prompt of `restore` names the backup file and the store.
pub(super) fn recovery_proof(db: &Path, act: &super::route::Keystore) -> Result<AdminProof> {
    use super::route::{KeyAct, Keystore};
    eprintln!(
        "the store cannot be read, so the admin passphrase cannot be checked; this act is approved by the recovery code and the typed confirmation instead"
    );
    match act {
        Keystore::Key(KeyAct::Recover { code, .. }) => {
            Ok(proof::recovery_proof_for_recover(db, &code.join(" "))?)
        }
        Keystore::Key(KeyAct::Restore { file }) => {
            let what = format!(
                "admin restore {} [store {}]",
                plain(&file.to_string_lossy()),
                plain(&db.to_string_lossy())
            );
            Ok(proof::recovery_proof_for_restore(
                db,
                &mut TtyPrompt,
                &what,
            )?)
        }
        _ => anyhow::bail!("only recover and restore can be approved without the store"),
    }
}

/// `klatch admin init-passphrase`: set the first passphrase, or change the one set (the old one is asked for first).
pub(crate) fn init_passphrase(db: &Path) -> Result<()> {
    let store = custody::open_store(db)?;
    if proof::is_set(&store).map_err(anyhow::Error::from)? {
        proof::change(&store, &mut TtyPrompt, at(), &proof::PROCESS_WINDOW)?;
        println!("the admin passphrase was changed");
        return Ok(());
    }
    // The first passphrase has nothing to prove against, so it cannot tell a person from a program that has a
    // terminal. Before anything else: a person sets it right after installing, on a terminal of their own.
    if klatch::member::discover_session().is_ok() {
        bail!(
            "the first admin passphrase is set by a person, not from inside a coding-agent session; run `klatch admin init-passphrase` in a terminal of your own"
        );
    }
    proof::init(&store, &mut TtyPrompt, at(), &proof::PROCESS_WINDOW)?;
    println!(
        "the admin passphrase is set; every admin, worker, anchor, run and judge command now asks for it at the terminal"
    );
    Ok(())
}

/// `klatch admin reset-passphrase`: a new passphrase for a person who forgot the old one, proved by the store's
/// recovery code (`klatch admin recovery` showed it).
pub(crate) fn reset_passphrase(db: &Path) -> Result<()> {
    let store = custody::open_store(db)?;
    let key = custody::current_key(db)?
        .ok_or_else(|| anyhow::anyhow!("this store has no key, so it has no recovery code"))?;
    let expected = klatch::keys::recovery_code(&key);
    proof::reset(
        &store,
        &expected,
        &mut TtyPrompt,
        at(),
        &proof::PROCESS_WINDOW,
    )?;
    println!("the admin passphrase was reset");
    Ok(())
}

/// `klatch admin unlock-window`: show the window, or set it (or put the default back).
pub(crate) fn unlock_window(
    store: &SqliteStore,
    proof: &AdminProof,
    secs: Option<i64>,
    off: bool,
) -> Result<()> {
    if secs.is_none() && !off {
        println!(
            "the unlock window is {} seconds (it applies to the commands of one long-lived process, `klatch admin shell`)",
            proof::window_len(store).as_secs()
        );
        return Ok(());
    }
    proof::set_window(store, proof, secs, now())?;
    println!(
        "the unlock window is now {} seconds",
        proof::window_len(store).as_secs()
    );
    Ok(())
}

/// A proof for a test of a command function. It is made in a store of its own (in memory), so a test that needs the
/// directory to have no key still has none: the proof is a value, and the same path a person's typing takes makes it.
#[cfg(test)]
pub(crate) fn proof_for_tests() -> AdminProof {
    use klatch::proof::{Cost, ProofError, Verifier};
    struct Typed(&'static str);
    impl proof::Prompt for Typed {
        fn secret(&mut self, _: &str) -> Result<zeroize::Zeroizing<String>, ProofError> {
            Ok(zeroize::Zeroizing::new(self.0.to_string()))
        }
    }
    let store = SqliteStore::open_in_memory().unwrap();
    let cheap = Cost {
        m_kib: 64,
        t: 1,
        p: 1,
    };
    proof::store_verifier(
        &store,
        &Verifier::create("a test passphrase", cheap).unwrap(),
        1,
    )
    .unwrap();
    proof::prove(
        &store,
        &mut Typed("a test passphrase"),
        at(),
        &klatch::proof::Unlock::new(),
        "a test",
    )
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::describe_words;

    fn d(words: &[&str]) -> String {
        describe_words(&words.iter().map(|w| w.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn control_characters_escapes_bidi_overrides_and_zero_width_marks_are_stripped() {
        let shown = d(&[
            "admin",
            "close",
            "lob\u{202e}by\u{1b}[31m\u{200b}\r\nx\u{2066}y\u{feff}",
        ]);
        for bad in [
            '\u{202e}', '\u{1b}', '\u{200b}', '\r', '\n', '\u{2066}', '\u{feff}',
        ] {
            assert!(!shown.contains(bad), "{bad:?} in {shown:?}");
        }
        assert!(shown.starts_with("admin close lob"), "{shown}");
    }

    #[test]
    fn a_long_command_keeps_its_command_word_and_says_how_much_was_cut() {
        let long = "x".repeat(500);
        let shown = d(&["admin", "scrub-add", "name", &long]);
        assert!(shown.starts_with("admin scrub-add name"), "{shown}");
        assert!(shown.chars().count() < 260, "{}", shown.chars().count());
        assert!(shown.contains("chars"), "{shown}");
    }
}

#[cfg(test)]
mod describe_tests {
    use super::{describe_os, describe_words};
    use std::ffi::OsString;

    fn d(words: &[&str]) -> String {
        describe_words(&words.iter().map(|w| w.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn a_room_or_member_named_recover_hides_nothing() {
        let shown = d(&["admin", "close", "recover"]);
        assert_eq!(shown, "admin close recover");
        let shown = d(&["admin", "remove", "recover", "--room", "lobby"]);
        assert_eq!(shown, "admin remove recover --room lobby");
        // inside the shell the family is the first word too
        assert_eq!(
            d(&["admin", "label", "recover", "private"]),
            "admin label recover private"
        );
    }

    #[test]
    fn only_the_command_word_recover_hides_and_it_hides_every_word_after_it() {
        let shown = d(&[
            "admin",
            "recover",
            "--",
            "KLATCH-AAAA",
            "--label",
            "private",
            "BBBB",
        ]);
        for secret in ["AAAA", "BBBB", "private", "--label", "--"] {
            assert!(!shown.contains(secret), "{secret} in {shown}");
        }
        assert!(shown.starts_with("admin recover"), "{shown}");
        // global flags before the family do not move the command word
        let shown = d(&["--as-member", "x", "admin", "recover", "KLATCH-ZZZZ"]);
        assert!(!shown.contains("ZZZZ"), "{shown}");
    }

    #[test]
    fn only_printable_ascii_is_shown_and_every_other_class_becomes_a_placeholder() {
        for (name, ch) in [
            ("line separator", '\u{2028}'),
            ("paragraph separator", '\u{2029}'),
            ("tag character", '\u{e0041}'),
            ("variation selector", '\u{fe0f}'),
            ("hangul filler", '\u{3164}'),
            ("braille blank", '\u{2800}'),
            ("interlinear annotation", '\u{fff9}'),
            ("object replacement", '\u{fffc}'),
            ("combining mark", '\u{0301}'),
            ("right to left override", '\u{202e}'),
            ("zero width space", '\u{200b}'),
            ("escape", '\u{1b}'),
            ("newline", '\n'),
            ("carriage return", '\r'),
            ("a non-ASCII letter", 'é'),
        ] {
            let shown = d(&["admin", "close", &format!("a{ch}b")]);
            assert_eq!(shown, "admin close a?b", "{name}");
        }
        assert_eq!(
            d(&["admin", "close", "Room_1.x-y"]),
            "admin close Room_1.x-y"
        );
    }

    #[test]
    fn arguments_that_are_not_utf8_are_shown_lossily_and_never_panic() {
        use std::os::unix::ffi::OsStringExt;
        let bad = OsString::from_vec(vec![b'a', 0xff, 0xfe, b'b']);
        let shown = describe_os(&[OsString::from("admin"), OsString::from("close"), bad]);
        assert_eq!(shown, "admin close a??b");
    }
}
