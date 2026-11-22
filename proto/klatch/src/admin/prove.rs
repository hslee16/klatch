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

/// A character that could dress up a prompt: a control character (newline, carriage return, escape), a bidi
/// override or isolate, a zero-width or invisible mark.
fn hostile(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{061c}' | '\u{180e}' | '\u{feff}' | '\u{00ad}')
}

/// Text from an argument, for a prompt: every hostile character removed.
pub(crate) fn plain(text: &str) -> String {
    text.chars().filter(|c| !hostile(*c)).collect()
}

/// The longest a word after the command is shown; a longer one is cut and says how long it was and a short digest.
const WORD_LIMIT: usize = 48;

/// What the passphrase approves, for the prompt: the command being run. Hostile characters are removed, the first
/// two words (the family and the command) are never cut, a long word is cut with its length and a digest, every
/// word after `recover` that is not a flag (a recovery code, however it is split) is hidden, and a very long
/// command is cut at a word with how many words went.
pub(crate) fn describe_words(words: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut out: Vec<String> = Vec::new();
    let mut after_recover = false;
    let mut value_next = false;
    for (i, w) in words.iter().enumerate() {
        let w = plain(w);
        let shown = if after_recover && !w.starts_with('-') && !value_next {
            "<code>".to_string()
        } else if i >= 2 && w.chars().count() > WORD_LIMIT {
            let digest = data_encoding::HEXLOWER.encode(&Sha256::digest(w.as_bytes()));
            format!(
                "{}...({} chars, sha256 {})",
                w.chars().take(WORD_LIMIT - 8).collect::<String>(),
                w.chars().count(),
                &digest[..8]
            )
        } else {
            w.clone()
        };
        value_next = after_recover && w.starts_with('-');
        if w == "recover" {
            after_recover = true;
        }
        out.push(shown);
    }
    let mut text = String::new();
    for (i, w) in out.iter().enumerate() {
        if i >= 2 && text.chars().count() + w.chars().count() > 200 {
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

fn describe() -> String {
    if let Some(line) = SHELL_LINE.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return line;
    }
    describe_words(&std::env::args().skip(1).collect::<Vec<_>>())
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
    fn every_word_after_recover_that_is_not_a_flag_is_hidden() {
        let shown = d(&[
            "admin",
            "recover",
            "KLATCH-AAAA",
            "BBBB-CCCC",
            "--label",
            "private",
        ]);
        assert!(
            !shown.contains("AAAA") && !shown.contains("BBBB") && !shown.contains("CCCC"),
            "{shown}"
        );
        assert!(shown.contains("--label"), "{shown}");
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
