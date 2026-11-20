//! Leak canaries, tripwires and the circuit breaker (D4, ticket #46). A canary is a fake secret planted in a room
//! that must not leak; text carrying it anywhere else is refused and pauses the room it was aimed at. The breaker
//! stops a member that keeps running into the safety rules. Both are paused until an admin lifts them, and both
//! record why. Neither echoes a canary or matched text.

use super::*;
use crate::domain::{Attribution, NEXT_STEP};
use crate::store::NewAudit;

const CANARY: &str = "canary:";
/// The `kind` of a planted canary message.
pub(crate) const CANARY_KIND: &str = "canary";
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
            self.plant_canary(room, now)?;
            Ok(String::new())
        })
    }

    /// End the canary of a room (#347): the tripwire for it stops. A later label act by the human plants a new one.
    pub fn admin_retire_canary(&self, who: &Principal, room: &str) -> Result<()> {
        check_room(room)?;
        self.administer(who, ("canary-retire", room), |now| {
            let keys = self.canary_keys_of(room)?;
            if keys.is_empty() {
                return Err(Error::Invalid(format!("room {room} has no canary")));
            }
            for key in keys {
                self.store.policy_set(&key, None, now)?;
            }
            Ok(String::new())
        })
    }

    /// The label act of the human plants a canary when the room reaches the threshold (the default agent cap, which
    /// is `private` in the default chain) and has none. An admin agent's label act plants nothing, and a room
    /// labelled before this existed has none until the human labels it again.
    pub(super) fn plant_on_label(&self, who: &Principal, room: &str, label: &str) -> Result<()> {
        let chain = self.chain()?;
        let threshold = match chain.rank("private") {
            Some(_) => "private",
            None => chain.default_cap(),
        };
        if !chain.dominates(label, threshold)
            || self.is_admin_agent(&who.member)?
            || !self.canary_keys_of(room)?.is_empty()
        {
            return Ok(());
        }
        let now = self.now();
        self.plant_canary(room, now)?;
        self.store.record_audit(
            &NewAudit::new("canary", room, "planted with the label"),
            now,
        )
    }

    fn canary_keys_of(&self, room: &str) -> Result<Vec<String>> {
        Ok(self
            .store
            .policy_list(CANARY)?
            .into_iter()
            .filter(|(_, home)| home == room)
            .map(|(key, _)| key)
            .collect())
    }

    fn plant_canary(&self, room: &str, now: i64) -> Result<()> {
        let token = canary_token()?;
        self.store
            .policy_set(&format!("{CANARY}{token}"), Some(room), now)?;
        let label = self.room_label(room)?;
        let body = format!("internal credential, never copy it out of this room: {token}");
        self.store.post_as(
            &NewMessage::new((room, "canary"), Attribution::Unattributed, &label, &body)
                .with_kind(CANARY_KIND),
            now,
        )?;
        Ok(())
    }

    /// A message of a room's canary is shown once in the digest and nowhere else: every other view drops it.
    pub(super) fn without_canary(messages: Vec<Message>) -> Vec<Message> {
        messages
            .into_iter()
            .filter(|m| m.kind.as_deref() != Some(CANARY_KIND))
            .collect()
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
                    "that text came from a room it must not leave; room {room} is paused until an admin reopens it; {NEXT_STEP}do not post it again; ask an admin to look and reopen room {room}"
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

/// A token that looks like a credential: 128 random bits from the operating system, never derived from the room or
/// the time and never written in the source (not secret: its use is to be recognised).
fn canary_token() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|e| Error::Invalid(format!("no randomness for a canary: {e}")))?;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("kc-canary-{hex}"))
}

#[cfg(test)]
mod canary_token_tests {
    use super::canary_token;

    #[test]
    fn a_token_has_the_prefix_and_128_bits_and_never_repeats() {
        let a = canary_token().unwrap();
        let b = canary_token().unwrap();
        assert!(a.starts_with("kc-canary-"), "{a}");
        assert_eq!(a.len(), "kc-canary-".len() + 32);
        assert_ne!(a, b);
    }
}
