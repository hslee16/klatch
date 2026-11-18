//! Room settings (#242, DESIGN section 32 item 98): how long an ask may sit unclaimed before its asker is told, and
//! the lease an `ask:<N>` claim gets when none is asked for, each per room. Both are admin-set policy keys with
//! bounds, like `lease:cap` (#537): the human's alone, audited, and `off` puts the built-in value back. A stored
//! value outside its bound reads as the built-in one, so a hand-written key cannot lift a limit. Neither setting
//! reaches D5: the claim cap of 24 hours is checked on every claim, and an ask claim never lasts over
//! `ASK_LEASE_MAX_SECS` whatever the default is.

use super::*;
use crate::domain::{ASK_LEASE_DEFAULT_SECS, ASK_LEASE_MAX_SECS, STALE_AFTER_SECS, claim_ttl};

/// One room setting: the admin action name, the end of its policy key (`room:ROOM:<suffix>`), the whole seconds it
/// may take and the built-in value.
struct Knob {
    action: &'static str,
    suffix: &'static str,
    min: i64,
    max: i64,
    default: i64,
}

impl Knob {
    fn key(&self, room: &str) -> String {
        format!("room:{room}:{}", self.suffix)
    }

    /// A stored whole number of seconds inside the bounds, else the built-in value.
    fn read(&self, stored: Option<String>) -> i64 {
        stored
            .and_then(|v| v.trim().parse::<i64>().ok())
            .filter(|v| (self.min..=self.max).contains(v))
            .unwrap_or(self.default)
    }
}

/// How long an ask may go unclaimed before its asker is told: 10 seconds to an hour.
const STALE: Knob = Knob {
    action: "room-stale",
    suffix: "stale",
    min: 10,
    max: 3_600,
    default: STALE_AFTER_SECS,
};
/// The lease of an `ask:` claim with none requested: 10 seconds up to the cap an ask claim never passes.
const ASK_LEASE: Knob = Knob {
    action: "room-ask-lease",
    suffix: "ask-lease",
    min: 10,
    max: ASK_LEASE_MAX_SECS,
    default: ASK_LEASE_DEFAULT_SECS,
};

/// What a room's settings are now: the human's value, else the built-in one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomSettings {
    /// How long an open ask may go without a claim before its asker is told.
    pub stale_secs: i64,
    /// The lease an `ask:<N>` claim gets when none is asked for.
    pub ask_lease_secs: i64,
}

impl<S: Store> Gateway<S> {
    /// The settings in force for `room`. Internal: the digest and the claim default read it.
    pub(crate) fn settings_of(&self, room: &str) -> Result<RoomSettings> {
        Ok(RoomSettings {
            stale_secs: STALE.read(self.store.policy_get(&STALE.key(room))?),
            ask_lease_secs: ASK_LEASE.read(self.store.policy_get(&ASK_LEASE.key(room))?),
        })
    }

    /// Only the human: set how long an ask may sit unclaimed in `room` before its asker is told, 10 seconds to an
    /// hour, or with `None` put the built-in 60 seconds back.
    pub fn admin_set_room_stale(
        &self,
        who: &Principal,
        room: &str,
        secs: Option<i64>,
    ) -> Result<()> {
        self.set_room_key((who, room), &STALE, secs)
    }

    /// Only the human: set the lease an `ask:<N>` claim gets in `room` when none is asked for, 10 to 300 seconds, or
    /// with `None` put the built-in 90 seconds back. A longer lease than the cap is never allowed.
    pub fn admin_set_room_ask_lease(
        &self,
        who: &Principal,
        room: &str,
        secs: Option<i64>,
    ) -> Result<()> {
        self.set_room_key((who, room), &ASK_LEASE, secs)
    }

    fn set_room_key(
        &self,
        (who, room): (&Principal, &str),
        knob: &Knob,
        secs: Option<i64>,
    ) -> Result<()> {
        let Knob {
            action, min, max, ..
        } = *knob;
        check_room(room)?;
        if let Some(s) = secs
            && !(min..=max).contains(&s)
        {
            return Err(Error::Invalid(format!(
                "the {action} is {min} to {max} seconds, or off for the default"
            )));
        }
        let target = format!(
            "{room} {}",
            secs.map_or("off".to_string(), |s| s.to_string())
        );
        self.administer(who, (action, &target), |now| {
            self.store
                .policy_set(&knob.key(room), secs.map(|s| s.to_string()).as_deref(), now)?;
            Ok(String::new())
        })
    }

    /// The lease for a claim at `(room, path)`: the one asked for, else the default for the path (the room's setting
    /// for an `ask:<N>` claim, 10 minutes for any other). An ask claim is never longer than `ASK_LEASE_MAX_SECS`;
    /// the gateway still refuses any claim over 24 hours when it is taken.
    pub fn claim_ttl_for(
        &self,
        who: &Principal,
        (room, path): (&str, &str),
        requested: Option<i64>,
    ) -> Result<i64> {
        check_member(who)?;
        check_room(room)?;
        check_path(path)?;
        // the claim that follows asks again; a refusal is recorded here once because the claim is then never made
        self.authorize(
            who,
            &Operation::Claim {
                room: room.into(),
                path: path.into(),
            },
        )?;
        if requested.is_none() && path.starts_with("ask:") {
            return Ok(self.settings_of(room)?.ask_lease_secs);
        }
        Ok(claim_ttl(path, requested))
    }
}
