//! The checks on the refs an ask carries (#1318), all before anything is posted. klatch never fetches, opens, runs or
//! resolves a ref: these checks only read the ref's own text and the labels of the rooms it names.

use super::*;
use crate::domain::{AskRef, NEXT_STEP, RefProblem};

impl<S: Store> Gateway<S> {
    /// Refuse `refs` that are malformed or over a cap (`invalid`, never cut), that carry a credential, that match the
    /// secret, canary or private-term rules a message body meets (every value, label and snippet is judged as message
    /// text is), or that would launder a label (`check_refs_label`). `at` is the room and the label asked for.
    pub(super) fn check_refs_before_post(
        &self,
        who: &Principal,
        at: (&str, Option<&str>),
        refs: &[AskRef],
    ) -> Result<()> {
        let (room, asked) = at;
        crate::domain::check_refs(refs).map_err(|problem| match problem {
            RefProblem::Invalid(why) => Error::Invalid(why),
            RefProblem::Credential(why) => {
                let e = Error::Refused {
                    rule: "credentials".into(),
                    reason: why,
                };
                self.note_refusal(who, (room, "post"), &e);
                e
            }
        })?;
        for r in refs {
            for text in [Some(&r.value), r.label.as_ref(), r.snippet.as_ref()]
                .into_iter()
                .flatten()
            {
                self.refuse_text(who, (room, "post"), text)?;
            }
        }
        self.check_refs_label(who, (room, asked), refs)
    }

    /// A ref is posted under the ask's own label, so it cannot carry anything the ask could not: an ask under a sealed
    /// label keeps its text under the label's key and refs are stored beside it, so it carries none; and a ref that
    /// names another room (`room`, or `ask` as `ROOM#N`) must name one the ask's label covers, so a lower room cannot
    /// point at what a higher one holds. The reasons name no label.
    fn check_refs_label(
        &self,
        who: &Principal,
        (room, asked): (&str, Option<&str>),
        refs: &[AskRef],
    ) -> Result<()> {
        let label = self.post_label(who, room, asked)?;
        let chain = self.chain()?;
        let refuse = |reason: String| {
            let e = Error::Refused {
                rule: "label".into(),
                reason,
            };
            self.note_refusal(who, (room, "post"), &e);
            e
        };
        if self.store.policy_get(&format!("sealed:{label}"))?.is_some() {
            return Err(refuse(format!(
                "refs are stored beside the ask in the clear and this ask's label is sealed; {NEXT_STEP}ask without --ref and put what the helper needs in a room that holds it"
            )));
        }
        for r in refs {
            // a `message` ref is judged by the room that holds the message; an id no message has counts as a room
            // that does not exist, which is the lowest label (as a `room` ref to one does), so a refusal never
            // tells an asker which ids exist (#1328). A `run` has no room and stays a shape-only check.
            let Some(target) = (match r.kind.as_str() {
                "message" => self.store.message_room(&r.value)?,
                _ => crate::domain::named_room(&r.kind, &r.value),
            }) else {
                continue;
            };
            if target == room {
                continue;
            }
            let theirs = self.room_label(&target)?;
            if !chain.dominates(&label, &theirs) {
                return Err(refuse(format!(
                    "a ref names a room that the ask's label does not cover; {NEXT_STEP}drop that ref, or post the ask in a room that holds it"
                )));
            }
        }
        Ok(())
    }
}
