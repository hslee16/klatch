//! Anchored claims (#125, DESIGN D5): a claim may name the work item it is for, a factory's work order, mission or
//! ticket. The item being done releases the claim, so a path is free the moment the work is, instead of when the lease
//! runs out. Two ways in, and neither is an agent's: the factory reports the item done (`anchor_done`, the push or
//! webhook form), or the gateway asks an adapter which items are done (`reap_anchors`, the poll form). The lease
//! stays the safety net: an adapter that cannot answer leaves the claim, and the lease still ends it.

use super::*;
use crate::store::NewAudit;

/// Tells the gateway whether a work item is finished. An implementation reads the factory's own state (a REST API, a
/// directory the factory writes to); klatch never interprets the anchor.
pub trait AnchorAdapter {
    /// `Ok(true)` when the item is done. An error means the factory could not say; the claim stays.
    fn is_done(&self, anchor: &str) -> Result<bool>;
}

/// 1 to 80 characters of letters, digits and `. _ : / -`.
pub fn valid_anchor(anchor: &str) -> bool {
    !anchor.is_empty()
        && anchor.len() <= 80
        && anchor
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-'))
}

impl<S: Store> Gateway<S> {
    /// The factory reports `anchor` done: release every claim anchored to it. Returns how many were released. A
    /// launcher's act, not an agent's: it goes through the guard and is audited.
    pub fn anchor_done(&self, who: &Principal, anchor: &str) -> Result<usize> {
        if !valid_anchor(anchor) {
            return Err(Error::Invalid(format!("bad anchor: {anchor:?}")));
        }
        let mut released = 0;
        self.administer(who, ("anchor-done", anchor), |now| {
            released = self.release_anchor(anchor, now)?;
            Ok(format!("released {released} claim(s)"))
        })?;
        Ok(released)
    }

    /// Ask `adapter` about every anchor with a live claim and release those it says are done. Returns the total
    /// released. An anchor the adapter cannot answer for is left alone.
    pub fn reap_anchors(&self, who: &Principal, adapter: &dyn AnchorAdapter) -> Result<usize> {
        let mut total = 0;
        self.administer(who, ("reap-anchors", ""), |now| {
            for anchor in self.store.anchors_in_use(now)? {
                if matches!(adapter.is_done(&anchor), Ok(true)) {
                    let n = self.release_anchor(&anchor, now)?;
                    total += n;
                    self.store.record_audit(
                        &NewAudit::new(
                            "anchor-done",
                            &anchor,
                            &format!("released {n} claim(s) (polled)"),
                        ),
                        now,
                    )?;
                }
            }
            Ok(format!("released {total} claim(s)"))
        })?;
        Ok(total)
    }

    fn release_anchor(&self, anchor: &str, now: i64) -> Result<usize> {
        Ok(self.store.release_anchored(anchor, now)?.len())
    }
}
