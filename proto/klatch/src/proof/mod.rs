//! The human's proof for admin, launcher and keystore acts: the admin passphrase (#1233, option 1 of #318).
//!
//! `klatch admin init-passphrase` keeps an Argon2id verifier of a passphrase in policy (`admin-passphrase`), never the
//! passphrase. Every admin command, every launcher command (`worker`, `anchor`, `run`, `judge`) and every keystore
//! act then asks for it on a real terminal (`tty`), and only a correct answer builds an `AdminProof`. The proof is
//! the only way to the principal named `human` (`Attribution::Verified`) that the admin guard (`RequireHuman`)
//! accepts, and the functions that act take the proof as an argument, so an act with no proof does not compile.
//!
//! What it is and is not: it proves the caller KNOWS a secret typed at a terminal. It does not prove the caller
//! is a person, and it is no defence against a process of the same user that can read the store key and write the
//! store directly, that watches the keyboard, or that reads the memory of an unlocked process (SECURITY.md).
//!
//! Policy keys: `admin-passphrase` (the verifier), `admin-passphrase-fails` (the failure count, `limit`) and
//! `admin-unlock-secs` (the unlock window, default `DEFAULT_WINDOW_SECS`).

pub mod alarm;
mod filelimit;
pub(crate) mod limit;
mod terminal;
mod tty;
mod verifier;
mod window;

/// The token `PolicyStore::reserved_policy_set` asks for (#1588): only this module can build one, so "only the proof
/// writes the admin passphrase rows" is a compiler guarantee, not a convention.
pub struct ProofCap(());

impl ProofCap {
    fn new() -> Self {
        ProofCap(())
    }
}

pub use tty::{
    TtyPrompt, input_and_output_are_one_terminal, is_controlling_tty, output_is_the_prompt_terminal,
};
pub use verifier::{CURRENT as CURRENT_COST, Cost, MAX_LEN, MIN_LEN, Verifier, check_new};
pub use window::{PROCESS as PROCESS_WINDOW, Unlock};

use crate::domain::Principal;
use crate::store::NewAudit;
use crate::store::areas::PolicyStore;
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

pub const VERIFIER_KEY: &str = "admin-passphrase";
pub const WINDOW_KEY: &str = "admin-unlock-secs";
/// The member name of the person, once proved. Reserved: no member may take it.
pub const HUMAN: &str = "human";
pub const DEFAULT_WINDOW_SECS: i64 = 60;
pub const MIN_WINDOW_SECS: i64 = 10;
pub const MAX_WINDOW_SECS: i64 = 1800;

/// Why a proof, or a change of the passphrase, did not happen. No variant carries what was typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofError {
    /// No passphrase has been set yet.
    NotSet,
    /// The stored verifier is damaged; nothing is let in.
    Corrupt,
    /// There is no terminal to ask on.
    NoTerminal,
    /// The read was stopped (end of input or Ctrl-C).
    Aborted,
    /// Too many wrong attempts: wait this many seconds.
    Locked(i64),
    /// Wrong, or empty.
    Wrong,
    /// A new passphrase that is not acceptable, or the two typed did not match.
    Rejected(String),
    /// The store failed.
    Store(String),
}

impl std::fmt::Display for ProofError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProofError::NotSet => write!(
                f,
                "no admin passphrase is set; run `klatch admin init-passphrase` at a terminal of your own"
            ),
            ProofError::Corrupt => write!(
                f,
                "the stored admin passphrase verifier is damaged; nothing was done (restore it with `klatch admin reset-passphrase`)"
            ),
            ProofError::NoTerminal => write!(
                f,
                "this needs the admin passphrase, typed at a real terminal; run it yourself in a terminal"
            ),
            ProofError::Aborted => write!(f, "no passphrase was entered; nothing was done"),
            ProofError::Locked(s) => {
                write!(f, "too many wrong passphrases; try again in {s} seconds")
            }
            ProofError::Wrong => write!(f, "wrong passphrase; nothing was done"),
            ProofError::Rejected(why) => write!(f, "{why}"),
            ProofError::Store(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ProofError {}

impl From<crate::domain::Error> for ProofError {
    fn from(e: crate::domain::Error) -> Self {
        ProofError::Store(e.to_string())
    }
}

/// Where a secret comes from. The real one is `TtyPrompt`; a test supplies its own. A prompt that cannot reach a
/// person answers `NoTerminal`, and that is a refusal.
pub trait Prompt {
    /// A secret, typed with echo off.
    fn secret(&mut self, label: &str) -> Result<Zeroizing<String>, ProofError>;

    /// The terminal this prompt is on, as a short label that is not secret (#1274). The failure limit counts per
    /// terminal. The default names the process's controlling terminal; a test supplies its own.
    fn terminal(&self) -> String {
        terminal::current()
    }

    /// Tell the person why an entry was not taken (never the entry itself). The default says nothing.
    fn notice(&mut self, _text: &str) {}

    /// A line the person types back with echo on (the confirmation of a destructive act). It comes from the
    /// terminal and never from a pipe. A prompt that cannot reach a person answers `NoTerminal`.
    fn line(&mut self, _label: &str) -> Result<String, ProofError> {
        Err(ProofError::NoTerminal)
    }
}

/// Evidence that the caller typed the admin passphrase (or the recovery code) just now or inside the unlock window.
/// The only constructors are in this module, after a check. It is not `Clone`.
#[derive(Debug)]
pub struct AdminProof(());

impl AdminProof {
    /// The principal this proof makes: the person, `Verified`.
    pub fn principal(&self) -> Principal {
        Principal::verified(HUMAN)
    }
}

/// The time of a proof: unix seconds (for the audit and the failure limit) and the monotonic clock (for the window).
#[derive(Clone, Copy)]
pub struct At {
    pub unix: i64,
    pub mono: Instant,
}

/// The unlock window length, from policy (a damaged or missing value is the default).
pub fn window_len<S: PolicyStore + ?Sized>(store: &S) -> Duration {
    let secs = store
        .policy_get(WINDOW_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|s| (MIN_WINDOW_SECS..=MAX_WINDOW_SECS).contains(s))
        .unwrap_or(DEFAULT_WINDOW_SECS);
    Duration::from_secs(secs as u64)
}

fn audit<S: PolicyStore + ?Sized>(store: &S, action: &str, target: &str, detail: &str, at: i64) {
    let _ = store.record_audit(&NewAudit::new(action, target, detail), at);
}

/// An audit row of a refusal that anyone can cause by running a command: at most one a minute per kind, so a loop
/// of refused commands cannot fill the audit.
fn audit_throttled<S: PolicyStore + ?Sized>(
    store: &S,
    action: &str,
    target: &str,
    detail: &str,
    at: i64,
) {
    if store
        .last_audit_ts(action, target, at - 60)
        .ok()
        .flatten()
        .is_none()
    {
        audit(store, action, target, detail, at);
    }
}

fn load<S: PolicyStore + ?Sized>(store: &S) -> Result<Verifier, ProofError> {
    match store.policy_get(VERIFIER_KEY)? {
        None => Err(ProofError::NotSet),
        Some(v) => Verifier::parse(&v).ok_or(ProofError::Corrupt),
    }
}

/// Whether a passphrase is set.
pub fn is_set<S: PolicyStore + ?Sized>(store: &S) -> Result<bool, ProofError> {
    Ok(store.policy_get(VERIFIER_KEY)?.is_some())
}

/// Ask for the passphrase (unless the window of `unlock` is open for it) and build the proof.
///
/// Order: the window; the wait of the failure limit (nothing is asked while it runs); the prompt (no terminal is
/// a refusal that charges nothing); the attempt is charged; then it is checked in constant time. Success clears
/// the failures, opens the window and rewrites a verifier of weaker cost under the current one.
///
/// `what` is shown in the prompt: the passphrase approves exactly that command (`admin close lobby`), so a person
/// who is asked by a program that shares their terminal can see what they are about to allow.
pub fn prove<S: PolicyStore + ?Sized>(
    store: &S,
    prompt: &mut dyn Prompt,
    at: At,
    unlock: &Unlock,
    what: &str,
) -> Result<AdminProof, ProofError> {
    let term = prompt.terminal();
    let verifier = load(store)?;
    let id = verifier.id();
    if unlock.is_open(at, &id, window_len(store)) {
        return Ok(AdminProof(()));
    }
    let wait = limit::wait_secs(store, &term, at.unix)?;
    if wait > 0 {
        audit_throttled(
            store,
            "proof-locked",
            "",
            &format!("wait {wait}s terminal={term}"),
            at.unix,
        );
        return Err(ProofError::Locked(wait));
    }
    let typed = match prompt.secret(&format!("admin passphrase to approve `{what}`")) {
        Ok(t) => t,
        Err(ProofError::NoTerminal) => {
            audit_throttled(store, "proof-refused", "no-terminal", "", at.unix);
            return Err(ProofError::NoTerminal);
        }
        Err(e) => return Err(e),
    };
    let count = match limit::charge(store, &term, at.unix)? {
        Ok(c) => c,
        Err(wait) => return Err(ProofError::Locked(wait)),
    };
    // the verifier is read again now that the attempt is known: a passphrase changed while the person was typing
    // is checked against, never the old one
    let verifier = load(store)?;
    let id = verifier.id();
    if !verifier.verify(&typed) {
        alarm::failed(
            store,
            "",
            &format!("failures={count} terminal={term}"),
            at.unix,
        );
        return Err(ProofError::Wrong);
    }
    limit::clear(store, &term, at.unix)?;
    let mut opened_for = id;
    if verifier.is_stale()
        && let Ok(fresh) = Verifier::create(&typed, verifier::CURRENT)
    {
        // rewritten only if the row is still exactly the one that was verified, in one transaction
        let old = verifier.encode();
        let swapped = std::cell::Cell::new(false);
        store.atomically(&mut || {
            if store.policy_get(VERIFIER_KEY)?.as_deref() == Some(old.as_str()) {
                store.reserved_policy_set(
                    &ProofCap::new(),
                    VERIFIER_KEY,
                    Some(&fresh.encode()),
                    at.unix,
                )?;
                swapped.set(true);
            }
            Ok(())
        })?;
        if swapped.get() {
            audit(store, "passphrase-rehashed", "", "cost raised", at.unix);
            opened_for = fresh.id();
        }
    }
    unlock.open(at, &opened_for);
    audit(
        store,
        "proof-ok",
        "",
        &format!("window {}s", window_len(store).as_secs()),
        at.unix,
    );
    Ok(AdminProof(()))
}

/// Store a verifier of a passphrase. Not guarded by itself: the callers (`init`, `change`, `reset`) are.
pub fn store_verifier<S: PolicyStore + ?Sized>(
    store: &S,
    verifier: &Verifier,
    at: i64,
) -> Result<(), ProofError> {
    store.reserved_policy_set(&ProofCap::new(), VERIFIER_KEY, Some(&verifier.encode()), at)?;
    Ok(())
}

/// How many times a new passphrase is asked for in all: a too-short or mismatched entry is said so and asked again
/// until this many attempts have been used. Choosing a new passphrase proves nothing about an old one, so these
/// attempts are never charged to the failure limit.
pub const NEW_ATTEMPTS: usize = 3;

/// Ask for a new passphrase twice and check it, up to `NEW_ATTEMPTS` times, saying why each attempt failed.
fn ask_new(prompt: &mut dyn Prompt) -> Result<Zeroizing<String>, ProofError> {
    let mut why = String::new();
    for attempt in 1..=NEW_ATTEMPTS {
        let first = prompt.secret("new admin passphrase")?;
        why = match check_new(&first) {
            Err(e) => e,
            Ok(()) => {
                let second = prompt.secret("new admin passphrase again")?;
                if *first == *second {
                    return Ok(first);
                }
                "the two passphrases differ".to_string()
            }
        };
        if attempt < NEW_ATTEMPTS {
            prompt.notice(&format!(
                "{why}; try again ({} of {NEW_ATTEMPTS} attempts left)",
                NEW_ATTEMPTS - attempt
            ));
        }
    }
    Err(ProofError::Rejected(format!("{why}; nothing was changed")))
}

/// Set the first passphrase. Refused when one is set (use `change`). This is the one act with no proof to check
/// (there is nothing yet to prove): it needs a terminal, is audited, and the caller adds what it can to keep a
/// program away from it. See SECURITY.md for what that leaves open on an install that has not set one.
pub fn init<S: PolicyStore + ?Sized>(
    store: &S,
    prompt: &mut dyn Prompt,
    at: At,
    unlock: &Unlock,
) -> Result<AdminProof, ProofError> {
    if is_set(store)? {
        return Err(ProofError::Rejected(
            "an admin passphrase is already set; change it with `klatch admin init-passphrase` (it asks for the old one first)"
                .into(),
        ));
    }
    let term = prompt.terminal();
    let new = ask_new(prompt)?;
    let v = Verifier::create(&new, verifier::CURRENT).map_err(ProofError::Store)?;
    // the check and the store are one write transaction: two processes setting the first passphrase at once, or
    // one set while this was asking, cannot both win and the second never replaces the first
    let taken = std::cell::Cell::new(false);
    store.atomically(&mut || {
        if store.policy_get(VERIFIER_KEY)?.is_some() {
            taken.set(true);
            return Ok(());
        }
        store.reserved_policy_set(&ProofCap::new(), VERIFIER_KEY, Some(&v.encode()), at.unix)
    })?;
    if taken.get() {
        return Err(ProofError::Rejected(
            "an admin passphrase was set meanwhile; nothing was changed".into(),
        ));
    }
    limit::clear(store, &term, at.unix)?;
    audit(store, "passphrase-set", "", "first passphrase", at.unix);
    unlock.open(at, &v.id());
    Ok(AdminProof(()))
}

/// Change the passphrase: the old one first (a proof, with the failure limit), then the new one twice. The old
/// unlock is void (the verifier's salt changes) and the new one opens the window.
pub fn change<S: PolicyStore + ?Sized>(
    store: &S,
    prompt: &mut dyn Prompt,
    at: At,
    unlock: &Unlock,
) -> Result<AdminProof, ProofError> {
    // the old passphrase is asked for even inside the window: a change is never done on an old entry
    unlock.close();
    let _old = prove(store, prompt, at, unlock, "change the admin passphrase")?;
    let new = ask_new(prompt)?;
    let v = Verifier::create(&new, verifier::CURRENT).map_err(ProofError::Store)?;
    store_verifier(store, &v, at.unix)?;
    audit(store, "passphrase-changed", "", "", at.unix);
    unlock.open(at, &v.id());
    Ok(AdminProof(()))
}

/// Reset the passphrase with the store's recovery code, for a person who forgot it. `expected` is the code of the
/// store key as `klatch admin recovery` shows it. The attempt is charged like a passphrase attempt (the same
/// limit), the comparison is constant time, and what was typed is never kept. Whoever holds the recovery code can
/// already restore the key and so read the whole store: resetting the passphrase gives them nothing more.
pub fn reset<S: PolicyStore + ?Sized>(
    store: &S,
    expected: &str,
    prompt: &mut dyn Prompt,
    at: At,
    unlock: &Unlock,
) -> Result<AdminProof, ProofError> {
    use subtle::ConstantTimeEq;
    let term = prompt.terminal();
    let wait = limit::wait_secs(store, &term, at.unix)?;
    if wait > 0 {
        audit_throttled(
            store,
            "proof-locked",
            "",
            &format!("wait {wait}s terminal={term}"),
            at.unix,
        );
        return Err(ProofError::Locked(wait));
    }
    let typed = prompt.secret("recovery code")?;
    let count = match limit::charge(store, &term, at.unix)? {
        Ok(c) => c,
        Err(wait) => return Err(ProofError::Locked(wait)),
    };
    let norm = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_uppercase()
    };
    let (a, b) = (norm(&typed), norm(expected));
    let same = a.len() == b.len() && bool::from(a.as_bytes().ct_eq(b.as_bytes()));
    if !same {
        alarm::failed(
            store,
            "recovery",
            &format!("failures={count} terminal={term}"),
            at.unix,
        );
        return Err(ProofError::Wrong);
    }
    let new = ask_new(prompt)?;
    let v = Verifier::create(&new, verifier::CURRENT).map_err(ProofError::Store)?;
    store_verifier(store, &v, at.unix)?;
    limit::clear(store, &term, at.unix)?;
    audit(store, "passphrase-reset", "", "by recovery code", at.unix);
    unlock.open(at, &v.id());
    Ok(AdminProof(()))
}

/// The proof for `admin restore` when the store is lost or damaged (so the verifier cannot be read): the person types
/// the store's recovery code, which is compared in constant time with the code of the key in the keystore. The failure
/// limit (#1281) is kept in a file beside the store, since the store is unreadable (`filelimit`); `settle_recovery_attempts`
/// audits it once the store opens again. Private checks below build the proof; nothing outside this
/// module can build one from a bare yes.
pub fn recovery_proof_for_restore(
    db: &std::path::Path,
    prompt: &mut dyn Prompt,
    what: &str,
) -> Result<AdminProof, ProofError> {
    let key = crate::member::custody::current_key(db)
        .map_err(|e| ProofError::Store(e.to_string()))?
        .ok_or_else(|| {
            ProofError::Rejected(
                "this store has no key; restore the key first with `klatch admin recover`".into(),
            )
        })?;
    charge_file_limit(db)?;
    note_if_wrong(
        db,
        prove_by_recovery_code(&crate::keys::recovery_code(&key), prompt, what),
    )
}

/// A wrong code is said on stderr with the count on record, so failed guesses that never settle are not silent (#1636).
fn note_if_wrong(
    db: &std::path::Path,
    proved: Result<AdminProof, ProofError>,
) -> Result<AdminProof, ProofError> {
    if matches!(proved, Err(ProofError::Wrong)) {
        eprintln!("{}", failed_attempt_note(db));
    }
    proved
}

/// Charge one attempt to the file limit of a store that cannot be read; a wait not over is `Locked`.
fn charge_file_limit(db: &std::path::Path) -> Result<(), ProofError> {
    use filelimit::Charge;
    let now = crate::member::now();
    filelimit::charge(db, now).map_err(|c| match c {
        Charge::Wait(s) => ProofError::Locked(s),
        Charge::Io(e) => ProofError::Store(format!(
            "cannot keep the count of recovery attempts, so none is allowed: {e}"
        )),
    })
}

/// How many recovery attempts are recorded for the store at `db` and not yet settled (`klatch doctor` shows it).
pub fn recovery_attempts_recorded(db: &std::path::Path) -> u32 {
    filelimit::attempts(db)
}

/// "1 recovery attempt is" or "N recovery attempts are".
pub fn attempts_phrase(n: u32) -> String {
    if n == 1 {
        "1 recovery attempt is".into()
    } else {
        format!("{n} recovery attempts are")
    }
}

/// The file of recovery attempts for the store at `db`.
pub fn recovery_attempts_file(db: &std::path::Path) -> std::path::PathBuf {
    filelimit::path_for(db)
}

/// The line written to stderr when a recovery code was wrong: how many attempts the store has on record.
pub fn failed_attempt_note(db: &std::path::Path) -> String {
    format!(
        "klatch: that recovery code was wrong; {} on record in {} until a recovery succeeds (`klatch doctor` shows the count)",
        attempts_phrase(filelimit::attempts(db)),
        filelimit::path_for(db).display()
    )
}

/// The store opens again after a recovery or restore: audit the attempts made while it could not be read.
pub fn settle_recovery_attempts<S: PolicyStore + ?Sized>(db: &std::path::Path, store: &S) {
    filelimit::settle(db, store, crate::member::now());
}

/// The proof for `admin recover` (the store key) when the key is lost: the recovery code given is the credential,
/// and it counts only if the key it makes opens an existing ENCRYPTED store file. No store, an empty or a plaintext
/// one, or a key that does not open it, is refused: no code is good where there is nothing to check it against.
pub fn recovery_proof_for_recover(
    db: &std::path::Path,
    code: &str,
) -> Result<AdminProof, ProofError> {
    let key = crate::keys::restore(code).map_err(|e| ProofError::Rejected(e.to_string()))?;
    charge_file_limit(db)?;
    note_if_wrong(
        db,
        prove_by_key_that_opens(crate::member::custody::opens_encrypted_store(db, &key)),
    )
}

fn prove_by_recovery_code(
    expected: &str,
    prompt: &mut dyn Prompt,
    what: &str,
) -> Result<AdminProof, ProofError> {
    use subtle::ConstantTimeEq;
    let typed = prompt.secret(&format!("recovery code to approve `{what}`"))?;
    let norm = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_uppercase()
    };
    let (a, b) = (norm(&typed), norm(expected));
    if a.len() == b.len() && bool::from(a.as_bytes().ct_eq(b.as_bytes())) {
        Ok(AdminProof(()))
    } else {
        Err(ProofError::Wrong)
    }
}

fn prove_by_key_that_opens(opens: bool) -> Result<AdminProof, ProofError> {
    if opens {
        Ok(AdminProof(()))
    } else {
        Err(ProofError::Wrong)
    }
}

/// Set the unlock window, in seconds (`None` puts back the default). Needs a proof.
pub fn set_window<S: PolicyStore + ?Sized>(
    store: &S,
    _proof: &AdminProof,
    secs: Option<i64>,
    at: i64,
) -> Result<(), ProofError> {
    match secs {
        Some(s) if !(MIN_WINDOW_SECS..=MAX_WINDOW_SECS).contains(&s) => Err(ProofError::Rejected(
            format!("the unlock window is {MIN_WINDOW_SECS} to {MAX_WINDOW_SECS} seconds"),
        )),
        Some(s) => {
            store.reserved_policy_set(&ProofCap::new(), WINDOW_KEY, Some(&s.to_string()), at)?;
            audit(store, "unlock-window", "", &format!("{s}s"), at);
            Ok(())
        }
        None => {
            store.reserved_policy_set(&ProofCap::new(), WINDOW_KEY, None, at)?;
            audit(store, "unlock-window", "", "default", at);
            Ok(())
        }
    }
}

#[cfg(test)]
mod alarm_faults;
#[cfg(test)]
mod alarm_recovery_tests;
#[cfg(test)]
mod alarm_tests;
#[cfg(test)]
mod refusal_tests;
#[cfg(test)]
mod tests;
