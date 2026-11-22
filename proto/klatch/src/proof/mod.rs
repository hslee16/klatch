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

mod limit;
mod tty;
mod verifier;
mod window;

pub use tty::TtyPrompt;
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
pub const DEFAULT_WINDOW_SECS: i64 = 300;
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
            ProofError::Locked(s) => write!(
                f,
                "too many wrong passphrases; try again in {s} seconds"
            ),
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
    fn secret(&mut self, label: &str) -> Result<Zeroizing<String>, ProofError>;
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
fn audit_throttled<S: PolicyStore + ?Sized>(store: &S, action: &str, target: &str, detail: &str, at: i64) {
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
pub fn prove<S: PolicyStore + ?Sized>(
    store: &S,
    prompt: &mut dyn Prompt,
    at: At,
    unlock: &Unlock,
) -> Result<AdminProof, ProofError> {
    let verifier = load(store)?;
    let id = verifier.id();
    if unlock.is_open(at.mono, &id, window_len(store)) {
        return Ok(AdminProof(()));
    }
    let wait = limit::wait_secs(store, at.unix)?;
    if wait > 0 {
        audit_throttled(store, "proof-locked", "", &format!("wait {wait}s"), at.unix);
        return Err(ProofError::Locked(wait));
    }
    let typed = match prompt.secret("admin passphrase") {
        Ok(t) => t,
        Err(ProofError::NoTerminal) => {
            audit_throttled(store, "proof-refused", "no-terminal", "", at.unix);
            return Err(ProofError::NoTerminal);
        }
        Err(e) => return Err(e),
    };
    let count = match limit::charge(store, at.unix)? {
        Ok(c) => c,
        Err(wait) => return Err(ProofError::Locked(wait)),
    };
    if !verifier.verify(&typed) {
        audit(store, "proof-failed", "", &format!("failures={count}"), at.unix);
        return Err(ProofError::Wrong);
    }
    limit::clear(store, at.unix)?;
    if verifier.is_stale()
        && let Ok(fresh) = Verifier::create(&typed, verifier::CURRENT)
    {
        store.policy_set(VERIFIER_KEY, Some(&fresh.encode()), at.unix)?;
        audit(store, "passphrase-rehashed", "", "cost raised", at.unix);
        unlock.open(at.mono, &fresh.id());
    } else {
        unlock.open(at.mono, &id);
    }
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
    store.policy_set(VERIFIER_KEY, Some(&verifier.encode()), at)?;
    Ok(())
}

/// Ask for a new passphrase twice and check it.
fn ask_new(prompt: &mut dyn Prompt) -> Result<Zeroizing<String>, ProofError> {
    let first = prompt.secret("new admin passphrase")?;
    check_new(&first).map_err(ProofError::Rejected)?;
    let second = prompt.secret("new admin passphrase again")?;
    if *first != *second {
        return Err(ProofError::Rejected(
            "the two passphrases differ; nothing was changed".into(),
        ));
    }
    Ok(first)
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
    let new = ask_new(prompt)?;
    let v = Verifier::create(&new, verifier::CURRENT).map_err(ProofError::Store)?;
    store_verifier(store, &v, at.unix)?;
    limit::clear(store, at.unix)?;
    audit(store, "passphrase-set", "", "first passphrase", at.unix);
    unlock.open(at.mono, &v.id());
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
    let _old = prove(store, prompt, at, unlock)?;
    let new = ask_new(prompt)?;
    let v = Verifier::create(&new, verifier::CURRENT).map_err(ProofError::Store)?;
    store_verifier(store, &v, at.unix)?;
    audit(store, "passphrase-changed", "", "", at.unix);
    unlock.open(at.mono, &v.id());
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
    let wait = limit::wait_secs(store, at.unix)?;
    if wait > 0 {
        audit_throttled(store, "proof-locked", "", &format!("wait {wait}s"), at.unix);
        return Err(ProofError::Locked(wait));
    }
    let typed = prompt.secret("recovery code")?;
    let count = match limit::charge(store, at.unix)? {
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
        audit(store, "proof-failed", "recovery", &format!("failures={count}"), at.unix);
        return Err(ProofError::Wrong);
    }
    let new = ask_new(prompt)?;
    let v = Verifier::create(&new, verifier::CURRENT).map_err(ProofError::Store)?;
    store_verifier(store, &v, at.unix)?;
    limit::clear(store, at.unix)?;
    audit(store, "passphrase-reset", "", "by recovery code", at.unix);
    unlock.open(at.mono, &v.id());
    Ok(AdminProof(()))
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
            store.policy_set(WINDOW_KEY, Some(&s.to_string()), at)?;
            audit(store, "unlock-window", "", &format!("{s}s"), at);
            Ok(())
        }
        None => {
            store.policy_set(WINDOW_KEY, None, at)?;
            audit(store, "unlock-window", "", "default", at);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
