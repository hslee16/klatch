//! A room can require a harness capability (#61, DESIGN section 20 rule 2). The human sets the requirement as the
//! policy key `room:ROOM:require` (`refuse:wake,pre-action-enforcement` or `flag:wake`); a join from a harness that
//! lacks a required capability is refused, or admitted and recorded, by the mode. A caller that names no harness is
//! the generic floor. A stored value this build cannot read (an unknown capability or mode, a hand-written key)
//! refuses every joiner whatever the mode: unknown is never satisfied.

use super::*;
use crate::domain::NEXT_STEP;
use crate::harness::{Admission, Harness, Requirement, negotiate};
use crate::store::NewRefusal;

const SUFFIX: &str = "require";
const REFUSE_RULE: &str = "harness-capability";
const FLAG_RULE: &str = "harness-flagged";

fn key(room: &str) -> String {
    format!("room:{room}:{SUFFIX}")
}

impl<S: Store> Gateway<S> {
    /// The room's requirement: `None` when it has none, an error when the stored text cannot be read.
    pub(crate) fn room_requirement(&self, room: &str) -> Result<Option<Requirement>> {
        match self.store.policy_get(&key(room))? {
            None => Ok(None),
            Some(text) => Requirement::parse(&text)
                .map(Some)
                .map_err(|why| Error::Refused {
                    rule: REFUSE_RULE.into(),
                    reason: format!(
                        "room {room} has a harness requirement this build cannot read ({why}), so no one can join; {NEXT_STEP}ask the admin to run room-require again"
                    ),
                }),
        }
    }

    /// Only the human: require harness capabilities of a room (`refuse:CAPS` or `flag:CAPS`), or with `None` lift the
    /// requirement. An unknown capability or mode is refused here as invalid.
    pub fn admin_set_room_require(
        &self,
        who: &Principal,
        room: &str,
        spec: Option<&str>,
    ) -> Result<()> {
        check_room(room)?;
        let parsed = spec
            .map(|s| Requirement::parse(s).map_err(Error::Invalid))
            .transpose()?;
        let target = format!(
            "{room} {}",
            parsed
                .as_ref()
                .map_or("off".to_string(), Requirement::render)
        );
        self.administer(who, ("room-require", &target), |now| {
            let text = parsed.as_ref().map(Requirement::render);
            self.store.policy_set(&key(room), text.as_deref(), now)?;
            Ok(String::new())
        })
    }

    /// Join `room` as a member whose harness is `harness`. Refused (rule `harness-capability`, recorded) when the
    /// room requires a capability the harness lacks in `refuse` mode; admitted and recorded (`harness-flagged`) in
    /// `flag` mode. The adapter is an untrusted conveyor: this checks what it declares, not whether it told the truth.
    pub fn join_with_harness(
        &self,
        who: &Principal,
        room: &str,
        harness: &dyn Harness,
    ) -> Result<Admission> {
        self.admit(who, room, Operation::Join { room: room.into() })?;
        let admission = self.harness_admission(who, room, harness)?;
        let created = self.check_creation(who, room)?;
        self.store.join(room, &who.member, self.now())?;
        self.note_creation(who, room, created)?;
        Ok(admission)
    }

    /// The decision for one joiner, with its record. Called after the generic checks and before the member is stored.
    fn harness_admission(
        &self,
        who: &Principal,
        room: &str,
        harness: &dyn Harness,
    ) -> Result<Admission> {
        let req = match self.room_requirement(room) {
            Ok(None) => return Ok(Admission::Admit),
            Ok(Some(req)) => req,
            Err(e) => {
                self.note_refusal(who, (room, "join"), &e);
                return Err(e);
            }
        };
        let admission = negotiate(&req, harness);
        match &admission {
            Admission::Admit => {}
            Admission::Flag(_) => {
                self.store.record_refusal(
                    &NewRefusal::new(&who.member, room, "join", FLAG_RULE),
                    self.now(),
                )?;
            }
            Admission::Refuse(_) => {
                let e = Error::Refused {
                    rule: REFUSE_RULE.into(),
                    reason: admission.say(room, harness, &req),
                };
                self.note_refusal(who, (room, "join"), &e);
                return Err(e);
            }
        }
        Ok(admission)
    }
}
