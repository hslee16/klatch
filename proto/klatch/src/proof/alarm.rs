//! The global alarm on failed proofs (#1310): an audit row and a doctor warning, never a lockout and never a delay.
//!
//! The failure limit counts per terminal (#1274), so a program that opens new terminals gets three free guesses on
//! each. The human decided that the backstop is an alarm only, because any ceiling low enough to matter is again a
//! way for an agent to lock the human out. When the `proof-failed` rows of a rolling 24 hours reach `CEILING`, the
//! proof path writes ONE `proof-failed-many` row: the count, the number of distinct terminals and the window, never
//! a label and never anything typed. `klatch doctor` warns while the count is at or above `CEILING`.
//!
//! The count is read from the audit itself (`PolicyStore::audit_census`), so there is no new table and no policy key
//! an agent could write, and this module is the only code that writes the action. The `proof-failed` row and the
//! check are one write transaction: two processes failing at once cannot both miss the crossing. The row is written
//! when the count after this failure is exactly `CEILING`: later failures make it more and write nothing, and when
//! old failures roll out of the window and the count comes back to `CEILING` the alarm fires again. A success does
//! not reset the count. A census that cannot be read skips the alarm; the failure row is still written, and nothing
//! about what a proof does changes.
//!
//! Why 50: one terminal that never gives up makes about 103 guesses a day (3 free, waits of 30 to 480 seconds, then
//! 900 each), a program with a new terminal per three guesses is limited only by how fast it verifies, and a person
//! mistypes a handful of times a day. See the design item for #1310.

use crate::store::NewAudit;
use crate::store::areas::PolicyStore;

/// Failed proofs inside the window at which the alarm fires.
pub const CEILING: u64 = 50;
/// The rolling window, in seconds: a failure counts while `now - ts < WINDOW_SECS`.
pub const WINDOW_SECS: i64 = 24 * 3600;
/// The audit action of the alarm row. Written here and nowhere else.
pub const ACTION: &str = "proof-failed-many";
/// The audit action of a failed proof.
pub const FAILED: &str = "proof-failed";

/// The first moment that still counts at `now`.
pub fn since(now: i64) -> i64 {
    now - WINDOW_SECS + 1
}

/// Failed proofs in the last 24 hours at `now` and the distinct terminals among them. Counts only.
pub fn recent<S: PolicyStore + ?Sized>(store: &S, now: i64) -> crate::domain::Result<(u64, u64)> {
    store.audit_census(FAILED, since(now))
}

/// The text of the alarm row's detail.
pub fn detail(failures: u64, terminals: u64) -> String {
    format!("failures={failures} terminals={terminals} window=24h")
}

/// Write the `proof-failed` row of a wrong passphrase (`target` ""), or of a wrong recovery code ("recovery"), and,
/// when this failure brings the count of the window to `CEILING`, the alarm row, in one transaction. Best effort like
/// every audit row: an error loses the row, never changes the proof's answer.
pub(super) fn failed<S: PolicyStore + ?Sized>(store: &S, target: &str, text: &str, at: i64) {
    let row = NewAudit::new(FAILED, target, text);
    let done = store.atomically(&mut || {
        store.record_audit(&row, at)?;
        if let Ok((n, terminals)) = recent(store, at)
            && n == CEILING
        {
            store.record_audit(&NewAudit::new(ACTION, "alarm", &detail(n, terminals)), at)?;
        }
        Ok(())
    });
    if done.is_err() {
        // the transaction rolled back whole: write the failure row alone, as before the alarm existed
        let _ = store.record_audit(&row, at);
    }
}
