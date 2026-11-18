//! Labels, clearance and the record of refused calls (epic #9, DESIGN D3 and D4). The administrator sets a label on
//! a room and a clearance on a member; the gateway refuses a member below a room's label and a message labelled
//! above its room, and keeps one record for each refusal. The record names the rule and never the text.

use super::*;
use crate::domain::{LabelChain, NEXT_STEP, RefusalRecord};
use crate::store::NewRefusal;

impl<S: Store> Gateway<S> {
    /// The label order: the default four levels unless the administrator set a chain.
    pub(super) fn chain(&self) -> Result<LabelChain> {
        match self.store.policy_get("chain")? {
            Some(text) => LabelChain::parse(&text).map_err(Error::Backend),
            None => Ok(LabelChain::default()),
        }
    }

    /// The highest clearance any agent may hold.
    fn cap(&self, chain: &LabelChain) -> Result<String> {
        Ok(self
            .store
            .policy_get("cap")?
            .unwrap_or_else(|| chain.default_cap().to_string()))
    }

    /// A room's label; the lowest level for a room nobody labelled. A direct message room's label is the lower of
    /// its two members' clearances.
    pub fn room_label(&self, room: &str) -> Result<String> {
        if let Some((a, b)) = self.dm_pair(room)? {
            let (ca, cb) = (self.clearance_of(&a)?, self.clearance_of(&b)?);
            let chain = self.chain()?;
            return Ok(chain.min(&ca, &cb).unwrap_or("unknown").to_string());
        }
        match self.store.policy_get(&format!("label:{room}"))? {
            Some(label) => Ok(label),
            None => Ok(self.chain()?.lowest().to_string()),
        }
    }

    /// A member's clearance; the lowest level for a member nobody cleared.
    pub fn clearance_of(&self, member: &str) -> Result<String> {
        match self.store.policy_get(&format!("clearance:{member}"))? {
            Some(label) => Ok(label),
            None => Ok(self.chain()?.lowest().to_string()),
        }
    }

    /// Refuse a member whose clearance is below the room's label. A label the chain does not have never
    /// qualifies, so a room or member left with a stale label fails closed.
    pub(super) fn check_clearance(&self, who: &Principal, room: &str) -> Result<()> {
        let chain = self.chain()?;
        let (label, held) = (self.room_label(room)?, self.clearance_of(&who.member)?);
        if chain.dominates(&held, &label) {
            return Ok(());
        }
        Err(Error::Refused {
            rule: "clearance".into(),
            reason: format!(
                "room {room} is labelled {label}, above your clearance {held}; {NEXT_STEP}ask the admin to raise your clearance or to relabel the room"
            ),
        })
    }

    /// The label a post goes under: the one asked for, else the room's. A post above its room's label is refused
    /// (R8): a private fact cannot enter a public room.
    pub(super) fn post_label(
        &self,
        who: &Principal,
        room: &str,
        asked: Option<&str>,
    ) -> Result<String> {
        let chain = self.chain()?;
        let room_label = self.room_label(room)?;
        let label = asked.unwrap_or(&room_label);
        known_label(&chain, label)?;
        if chain.dominates(&room_label, label) {
            return Ok(label.to_string());
        }
        let e = Error::Refused {
            rule: "label".into(),
            reason: format!(
                "a {label} message cannot go into room {room}, which is labelled {room_label}; {NEXT_STEP}post it to a room labelled {label} or higher; to release a message already posted, run `klatch cross SOURCE SEQ --to {room}`"
            ),
        };
        self.note_refusal(who, (room, "post"), &e);
        Err(e)
    }

    /// Keep a record of a refusal; any other error is not one.
    pub(super) fn note_refusal(&self, who: &Principal, (room, operation): (&str, &str), e: &Error) {
        if let Error::Refused { rule, .. } = e {
            // a refused call stays refused whether or not the record could be kept
            let now = self.now();
            let _ = self
                .store
                .record_refusal(&NewRefusal::new(&who.member, room, operation, rule), now);
            let _ = self.check_breaker(who, now);
        }
    }

    /// Refuse text that looks like a credential or matches a private term, and keep a record of the refusal.
    pub(super) fn refuse_text(
        &self,
        who: &Principal,
        (room, operation): (&str, &str),
        text: &str,
    ) -> Result<()> {
        check_no_credentials(text)
            .and_then(|()| self.check_canary(who, room, text))
            .and_then(|()| self.check_scrub(room, text))
            .inspect_err(|e| self.note_refusal(who, (room, operation), e))
    }

    /// An administrative act that sets one policy `(key, value)`; the act is `(action, target)` as `administer` has it.
    fn administer_policy(
        &self,
        who: &Principal,
        act: (&str, &str),
        (key, value): (&str, Option<&str>),
    ) -> Result<()> {
        self.administer(who, act, |now| {
            self.store.policy_set(key, value, now)?;
            Ok(String::new())
        })
    }

    /// Set a room's label.
    pub fn admin_set_room_label(&self, who: &Principal, room: &str, label: &str) -> Result<()> {
        check_room(room)?;
        known_label(&self.chain()?, label)?;
        self.administer_policy(
            who,
            ("label", &format!("{room}={label}")),
            (&format!("label:{room}"), Some(label)),
        )
    }

    /// Seal a label: from now on a message under it is stored encrypted under the label's own key, and a principal
    /// without that key can neither read nor write it (#160). The lowest label cannot be sealed, because everyone
    /// holds it. Existing messages under the label stay as they were.
    pub fn admin_seal_label(&self, who: &Principal, label: &str) -> Result<()> {
        let chain = self.chain()?;
        known_label(&chain, label)?;
        if label == chain.lowest() {
            return Err(Error::Invalid(format!(
                "{label} is the lowest label and everyone holds it, so it cannot be sealed"
            )));
        }
        self.administer_policy(
            who,
            ("seal-label", label),
            (&format!("sealed:{label}"), Some("1")),
        )
    }

    /// Every sealed label, lowest first.
    pub fn sealed_labels(&self) -> Result<Vec<String>> {
        let chain = self.chain()?;
        let mut out = Vec::new();
        for label in chain.names() {
            if self.store.policy_get(&format!("sealed:{label}"))?.is_some() {
                out.push(label.clone());
            }
        }
        Ok(out)
    }

    /// The sealed labels `member`'s clearance reaches: the keys it may be given.
    pub fn sealed_labels_within(&self, member: &str) -> Result<Vec<String>> {
        let chain = self.chain()?;
        let clearance = self.clearance_of(member)?;
        Ok(self
            .sealed_labels()?
            .into_iter()
            .filter(|l| chain.dominates(&clearance, l))
            .collect())
    }

    /// Hand this process the label keys it holds (in memory only). A sealed label it has no key for stays sealed.
    pub fn use_label_keys(&self, keys: crate::keys::LabelKeys) {
        self.store.set_label_keys(keys);
    }

    /// Whether an agent may be given `label` as its clearance: a known label no higher than the agent cap.
    pub(super) fn check_grantable(&self, label: &str) -> Result<()> {
        let chain = self.chain()?;
        known_label(&chain, label)?;
        let cap = self.cap(&chain)?;
        if !chain.dominates(&cap, label) {
            return Err(Error::Refused {
                rule: "agent-cap".into(),
                reason: format!("no agent may hold {label}: the agent cap is {cap}"),
            });
        }
        Ok(())
    }

    /// Set a member's clearance, at most the agent cap: `secret` is out of every agent's reach (D4).
    pub fn admin_set_clearance(&self, who: &Principal, member: &str, label: &str) -> Result<()> {
        if !valid_name(member) {
            return Err(Error::Invalid(format!("bad member name: {member}")));
        }
        self.check_grantable(label)?;
        self.administer_policy(
            who,
            ("clear", &format!("{member}={label}")),
            (&format!("clearance:{member}"), Some(label)),
        )
    }

    /// Set the highest clearance an agent may hold: below the top level of the chain.
    pub fn admin_set_cap(&self, who: &Principal, label: &str) -> Result<()> {
        let chain = self.chain()?;
        known_label(&chain, label)?;
        if label == chain.highest() {
            return Err(Error::Invalid(format!(
                "the agent cap must be below {label}, the top level"
            )));
        }
        self.administer_policy(who, ("cap", label), ("cap", Some(label)))
    }

    /// Replace the label order. The cap goes back to its default; a label stored under the old order that the new
    /// one lacks refuses everyone until it is set again.
    pub fn admin_set_label_chain(&self, who: &Principal, names: &[&str]) -> Result<()> {
        let chain = LabelChain::new(names).map_err(Error::Invalid)?;
        self.administer(who, ("chain", &chain.to_text()), |now| {
            self.store
                .policy_set("chain", Some(&chain.to_text()), now)?;
            self.store.policy_set("cap", None, now)?;
            Ok(String::new())
        })
    }

    /// The label order and the agent cap, read-only and any member's (decision on #740). It lists the levels and the
    /// cap, never a room, a clearance or a rule; setting either stays a floor act at a terminal.
    pub fn label_order(&self, who: &Principal) -> Result<(Vec<String>, String)> {
        check_member(who)?;
        self.check_guard(who, &Operation::admin("label-order", ""))?;
        let chain = self.chain()?;
        let cap = self.cap(&chain)?;
        Ok((chain.names().to_vec(), cap))
    }

    /// The caller's own latest refusals, newest first: rule, room, operation and time, never the text. It reads only
    /// the caller's rows, so no other member's refusal is ever returned, and it takes no guard: a member stopped by
    /// the breaker or a pause can still read why (#678).
    pub fn my_refusals(&self, who: &Principal, limit: usize) -> Result<Vec<RefusalRecord>> {
        check_member(who)?;
        self.store.refusals_of(&who.member, limit)
    }

    /// The latest refused calls, newest first.
    pub fn admin_refusals(&self, who: &Principal, limit: usize) -> Result<Vec<RefusalRecord>> {
        check_member(who)?;
        self.check_guard(
            who,
            &Operation::Admin {
                action: "refusals".into(),
                target: String::new(),
            },
        )?;
        self.store.refusals(limit)
    }
}

fn known_label(chain: &LabelChain, label: &str) -> Result<()> {
    match chain.rank(label) {
        Some(_) => Ok(()),
        None => Err(Error::Invalid(format!(
            "unknown label {label}; the levels are {}",
            chain.to_text()
        ))),
    }
}
