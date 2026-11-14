//! Direct messages (D3, ticket #43): a two-member room made on first use. Its label is the lower of the two
//! members' clearances, worked out at each call so a change of clearance applies at once. It goes through the same
//! post path as any room: same checks, audit and receipts.

use super::*;
use klatch_core::{DM_PREFIX, dm_room};

impl<S: Store> Gateway<S> {
    /// The two members of a direct-message room, if `room` is one.
    pub(super) fn dm_pair(&self, room: &str) -> Result<Option<(String, String)>> {
        let pair = self.store.policy_get(&format!("dm:{room}"))?;
        Ok(pair.and_then(|p| {
            let (a, b) = p.split_once(',')?;
            Some((a.to_string(), b.to_string()))
        }))
    }

    /// Only the two members use a direct-message room, and a name made like one that `dm` did not make is reserved.
    pub(super) fn check_dm(&self, who: &Principal, room: &str) -> Result<()> {
        match self.dm_pair(room)? {
            Some((a, b)) if who.member == a || who.member == b => Ok(()),
            Some(_) => Err(Error::Refused {
                rule: "dm-members".into(),
                reason: format!("room {room} is a direct message between two other members"),
            }),
            None if room.starts_with(DM_PREFIX) => Err(Error::Refused {
                rule: "dm-reserved".into(),
                reason: format!("room names starting {DM_PREFIX} are for direct messages"),
            }),
            None => Ok(()),
        }
    }

    /// Send `body` to member `to` in the room the two share, made on first use. The room's ceiling is the lower of
    /// their clearances; `label` may go lower. Returns the room, which both members can `read` and `wait` on.
    pub fn dm(
        &self,
        who: &Principal,
        to: &str,
        body: &str,
        label: Option<&str>,
    ) -> Result<(String, Posted)> {
        check_member(who)?;
        if !valid_name(to) || to == who.member {
            return Err(Error::Invalid(
                "a direct message goes to one other member by name".into(),
            ));
        }
        let room = dm_room(&who.member, to);
        let key = format!("dm:{room}");
        if self.store.policy_get(&key)?.is_none() {
            let (a, b) = if who.member.as_str() <= to {
                (who.member.as_str(), to)
            } else {
                (to, who.member.as_str())
            };
            self.store
                .policy_set(&key, Some(&format!("{a},{b}")), self.now())?;
        }
        let posted = self.post_labeled(who, &room, body, label)?;
        self.store.join(&room, &who.member, self.now())?;
        self.store.join(&room, to, self.now())?;
        Ok((room, posted))
    }
}
