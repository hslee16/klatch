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

/// What the passphrase approves, for the prompt: the command being run, with no control characters, a recovery
/// code given as an argument left out, and a bounded length.
pub(crate) fn describe_words(words: &[String]) -> String {
    let mut out = Vec::new();
    let mut hide = false;
    for w in words {
        if hide {
            out.push("<code>".to_string());
            hide = false;
            continue;
        }
        hide = w == "recover";
        out.push(
            w.chars()
                .map(|c| if c.is_control() { '?' } else { c })
                .collect::<String>(),
        );
    }
    let text = out.join(" ");
    if text.chars().count() > 160 {
        format!("{}...", text.chars().take(157).collect::<String>())
    } else {
        text
    }
}

fn describe() -> String {
    if let Some(line) = SHELL_LINE.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return line;
    }
    describe_words(&std::env::args().skip(1).collect::<Vec<_>>())
}

/// The proof of the person, asked for on the terminal unless this process holds an open unlock window. A refusal
/// is an error whose text says what to do and never holds what was typed. The prompt names `what`.
fn require_for(store: &SqliteStore, what: &str) -> Result<AdminProof> {
    proof::prove(store, &mut TtyPrompt, at(), &proof::PROCESS_WINDOW, what).map_err(Into::into)
}

/// `require` for a command that has not opened the store yet: the store and the proof.
pub(crate) fn open_and_require(db: &Path) -> Result<(SqliteStore, AdminProof)> {
    open_and_require_for(db, &describe())
}

pub(crate) fn open_and_require_for(db: &Path, what: &str) -> Result<(SqliteStore, AdminProof)> {
    let store = custody::open_store(db)?;
    let proof = require_for(&store, what)?;
    Ok((store, proof))
}

/// Whether the store can be read with the key the keystore holds (without making anything).
pub(crate) fn store_readable(db: &Path) -> bool {
    db.exists()
        && matches!(custody::current_key(db), Ok(Some(k)) if SqliteStore::open(db, &k).is_ok())
}

/// The proof for `recover` (store key) and `restore` when the store cannot be read, so the passphrase verifier
/// cannot either. `recover`: the recovery code given is the credential and counts only if its key opens the store
/// file. `restore`: the person types the store's recovery code. Both are followed by the typed confirmation.
pub(super) fn recovery_proof(db: &Path, act: &super::route::Keystore) -> Result<AdminProof> {
    use super::route::{KeyAct, Keystore};
    eprintln!(
        "the store cannot be read, so the admin passphrase cannot be checked; this act is approved by the recovery code and the typed confirmation instead"
    );
    match act {
        Keystore::Key(KeyAct::Recover { code, .. }) => {
            let key = klatch::keys::restore(&code.join(" ")).map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(proof::prove_by_key_that_opens(custody::key_opens_store(
                db, &key,
            ))?)
        }
        Keystore::Key(KeyAct::Restore { .. }) => {
            let key = custody::current_key(db)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "this store has no key; restore the key first with `klatch admin recover`"
                )
            })?;
            Ok(proof::prove_by_recovery_code(
                &klatch::keys::recovery_code(&key),
                &mut TtyPrompt,
                "admin restore",
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
