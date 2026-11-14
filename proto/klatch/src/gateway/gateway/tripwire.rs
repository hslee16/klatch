//! Leak canaries, tripwires and the circuit breaker (D4, ticket #46). A canary is a fake secret planted in a room
//! that must not leak; text carrying it anywhere else is refused and pauses the room it was aimed at. The breaker
//! stops a member that keeps running into the safety rules. Both are paused until an admin lifts them, and both
//! record why. Neither echoes a canary or matched text.

use super::*;
use crate::domain::Attribution;
use crate::store::NewAudit;

const CANARY: &str = "canary:";
/// This many safety refusals within the window stop a member everywhere.
pub const BREAKER_REFUSALS: usize = 5;
pub const BREAKER_WINDOW_SECS: i64 = 300;

/// The rules that mean a member ran into a safety rule, as opposed to a limit or an admin stop.
fn is_safety_rule(rule: &str) -> bool {
    ["clearance", "label", "credentials", "canary", "dm-"]
        .iter()
        .any(|r| rule == *r || (r.ends_with('-') && rule.starts_with(r)))
        || rule.starts_with("scrub:")
}

impl<S: Store> Gateway<S> {
    /// Plant a canary in a room: a fake credential posted by the member `canary` at the room's label. Anything that
    /// copies it into another room trips the wire.
    pub fn admin_plant_canary(&self, who: &Principal, room: &str) -> Result<()> {
        check_room(room)?;
        self.administer(who, ("canary", room), |now| {
            let existing = self.store.policy_list(CANARY)?.len();
            let token = canary_token(room, now, existing);
            self.store
                .policy_set(&format!("{CANARY}{token}"), Some(room), now)?;
            let label = self.room_label(room)?;
            let body = format!("internal credential, never copy it out of this room: {token}");
            self.store.post_as(
                &NewMessage::new((room, "canary"), Attribution::Unattributed, &label, &body),
                now,
            )?;
            Ok(String::new())
        })
    }

    /// Refuse text that carries a canary into any room but its own, and pause the room it was aimed at.
    pub(super) fn check_canary(&self, who: &Principal, room: &str, text: &str) -> Result<()> {
        for (key, home) in self.store.policy_list(CANARY)? {
            if home == room || !text.contains(key.trim_start_matches(CANARY)) {
                continue;
            }
            let now = self.now();
            self.store.set_room_state(room, Some("tripped"), now)?;
            let why = format!("a canary from {home} was posted by {}", who.member);
            self.store
                .record_audit(&NewAudit::new("tripwire", room, &why), now)?;
            return Err(Error::Refused {
                rule: "canary".into(),
                reason: format!(
                    "that text came from a room it must not leave; room {room} is paused until an admin reopens it"
                ),
            });
        }
        Ok(())
    }

    /// After a refusal: stop the member everywhere when it has run into the safety rules too often.
    pub(super) fn check_breaker(&self, who: &Principal, now: i64) -> Result<()> {
        let key = format!("trip:member:{}", who.member);
        if self.store.policy_get(&key)?.is_some() {
            return Ok(());
        }
        let rules = self
            .store
            .refusal_rules_since(&who.member, now - BREAKER_WINDOW_SECS)?;
        let n = rules.iter().filter(|r| is_safety_rule(r)).count();
        if n < BREAKER_REFUSALS {
            return Ok(());
        }
        let why = format!(
            "you were stopped after {n} refused calls in {BREAKER_WINDOW_SECS} seconds; an admin must allow you again"
        );
        self.store.policy_set(&key, Some(&why), now)?;
        self.store.record_audit(
            &NewAudit::new("breaker", &who.member, &format!("{n} safety refusals")),
            now,
        )
    }
}

/// A token that looks like a credential and is unique to the plant (not secret: its use is to be recognised).
fn canary_token(room: &str, now: i64, n: usize) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for byte in room.bytes().chain(now.to_le_bytes()).chain(n.to_le_bytes()) {
        h = (h ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    format!("kc-canary-{h:016x}")
}
