//! The open pull requests section of the board (item 84).

use super::*;

impl<S: Store> Gateway<S> {
    /// The pull requests the log says are open (item 84), each with the holder of its live `pr:N` claim. One that no
    /// message ended and whose claim was last released with a note that says merged is dropped: the claim's note is
    /// the other way to say a PR is over.
    pub(super) fn open_prs_of(&self, room: &str, now: i64) -> Result<Vec<crate::domain::OpenPr>> {
        let words = self.store.words(room, &crate::domain::PR_WORDS)?;
        let mut prs = crate::domain::open_prs(&words);
        if prs.is_empty() {
            return Ok(prs);
        }
        let claims = self.store.claims(room, now)?;
        for pr in &mut prs {
            let path = pr.path();
            pr.holder = claims
                .iter()
                .find(|c| c.path == path)
                .map(|c| c.owner.clone());
        }
        let mut open = Vec::with_capacity(prs.len());
        for pr in prs {
            let merged = pr.holder.is_none()
                && self
                    .store
                    .last_release(room, &pr.path())?
                    .is_some_and(|r| crate::domain::is_merged_note(&r.note));
            if !merged {
                open.push(pr);
            }
        }
        Ok(open)
    }
}
