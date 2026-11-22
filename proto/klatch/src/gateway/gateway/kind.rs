//! A member's kind (DESIGN section 32 item 103, #301). A kind is display only and grants nothing. Where it comes
//! from: a member with a session record is a `session` (klatch knows it; no declaration changes that); any other
//! member shows what it declared, if it declared a kind that a member may declare. The declaration is the policy
//! entry `kind:NAME`, written only for the caller's own name, so no member can set another's.

use super::*;
use crate::domain::{KIND_KEY, MemberKind};
use std::collections::HashMap;

impl<S: Store> Gateway<S> {
    /// Declare the caller's own kind, or with `None` withdraw it. `session` is refused: it is derived from a session
    /// record, not claimed.
    pub fn set_kind(&self, who: &Principal, kind: Option<MemberKind>) -> Result<()> {
        check_member(who)?;
        self.check_own_record_live(who)?;
        self.refuse_facilitator(who, "kind")?;
        if kind.is_some_and(|k| !k.declarable()) {
            return Err(Error::Invalid(
                "a member cannot declare its kind as session: klatch derives that from the session record; \
                 declare bot, service or human"
                    .into(),
            ));
        }
        self.store.policy_set(
            &format!("{KIND_KEY}{}", who.member),
            kind.map(MemberKind::word),
            self.now(),
        )
    }

    /// The caller's kind as others see it: `session` when it has a session record, else its declaration.
    pub fn kind_of(&self, who: &Principal) -> Result<Option<MemberKind>> {
        check_member(who)?;
        self.check_own_record_live(who)?;
        if self.store.session_of_member(&who.member)?.is_some() {
            return Ok(Some(MemberKind::Session));
        }
        Ok(self
            .store
            .policy_get(&format!("{KIND_KEY}{}", who.member))?
            .as_deref()
            .and_then(declared))
    }

    /// Fill in where each member listed was observed (#181): the repository id and the short directory name, from
    /// the host's observations. Display only; the full directory path is dropped here and never leaves the store
    /// through `who` or the board.
    pub(super) fn with_places(&self, members: &mut [MemberInfo]) -> Result<()> {
        for m in members {
            let scope = self.store.member_scope(&m.name)?;
            m.place = self.store.member_attrs(&m.name)?.and_then(|a| {
                let repository = (!a.repository.is_empty()).then_some(a.repository);
                // #894: a declared scope naming a repository that is not the derived one. A scope that names no
                // repository (a role, a bot's area) never differs.
                let scope_differs = match (&scope, &repository) {
                    (Some(s), Some(r)) => s.starts_with("git:") && s != r,
                    _ => false,
                };
                let place = crate::domain::MemberPlace {
                    repository,
                    directory: crate::domain::short_dir_name(&a.directory),
                    scope_differs,
                };
                (place.repository.is_some() || place.directory.is_some()).then_some(place)
            });
        }
        Ok(())
    }

    /// Fill in the permission mode each member last reported (#677), for display and for choosing a wake path. One
    /// read of the recorded modes for all of them; a member whose hooks never ran keeps `None`.
    pub(super) fn with_modes(&self, members: &mut [MemberInfo]) -> Result<()> {
        let modes: HashMap<String, String> = self
            .store
            .member_modes()?
            .into_iter()
            .map(|(member, mode, _)| (member, mode))
            .collect();
        for m in members {
            m.mode = modes.get(&m.name).cloned();
        }
        Ok(())
    }

    /// Mark each member that was admitted in flag mode of the room's requirement and has not since been admitted
    /// cleanly (#878). Display only: a mark is a note that the member's harness lacks a capability, not authority.
    pub(super) fn with_advisory(&self, room: &str, members: &mut [MemberInfo]) -> Result<()> {
        let prefix = super::room_require::advisory_prefix(room);
        let flagged = self.store.policy_list(&prefix)?;
        for m in members {
            m.advisory = flagged
                .iter()
                .any(|(key, _)| key.strip_prefix(&prefix) == Some(m.name.as_str()));
        }
        Ok(())
    }

    /// The one overlay that marks flagged senders on what a reader is returned (#990, #878): `read`, `wait` and the
    /// hook digest pass their messages through here. A message is marked when its sender holds the room's advisory
    /// policy key, so the mark comes from the gateway's record and never from the message, and a stored message is
    /// never changed. One prefix read, skipped when there is nothing to mark.
    pub(super) fn mark_advisory(
        &self,
        room: &str,
        mut messages: Vec<Message>,
    ) -> Result<Vec<Message>> {
        if messages.is_empty() {
            return Ok(messages);
        }
        let prefix = super::room_require::advisory_prefix(room);
        let flagged = self.store.policy_list(&prefix)?;
        for m in &mut messages {
            m.advisory = flagged
                .iter()
                .any(|(key, _)| key.strip_prefix(&prefix) == Some(m.sender.as_str()));
        }
        self.mark_ask_scopes(room, &mut messages)?;
        Ok(messages)
    }

    /// The same overlay shows the scope an asker declared on an ask (#1171): an ask message (`seeking:`) gets the
    /// scope stored for its seq. The scope comes from the ask's record, never from the text, and a stored message is
    /// never changed. One read, skipped when no message is an ask.
    fn mark_ask_scopes(&self, room: &str, messages: &mut [Message]) -> Result<()> {
        if !messages
            .iter()
            .any(|m| m.body.starts_with(crate::domain::ASK_PREFIX))
        {
            return Ok(());
        }
        let scopes = self.store.ask_scopes(room)?;
        let refs = self.store.ask_refs(room)?;
        for m in messages {
            if m.body.starts_with(crate::domain::ASK_PREFIX) {
                m.scope = scopes
                    .iter()
                    .find(|(seq, _)| *seq == m.seq)
                    .map(|(_, scope)| scope.clone());
                // the refs of #1318 come from the same record: never from the text, and never changed by it
                m.refs = refs
                    .iter()
                    .find(|(seq, _)| *seq == m.seq)
                    .map(|(_, r)| r.clone())
                    .unwrap_or_default();
            }
        }
        Ok(())
    }

    /// Fill in the kind of each member listed: a session record first, else the member's declaration. One read of
    /// the declarations for all of them.
    pub(super) fn with_kinds(&self, members: &mut [MemberInfo]) -> Result<()> {
        let declared_by: HashMap<String, MemberKind> = self
            .store
            .policy_list(KIND_KEY)?
            .into_iter()
            .filter_map(|(key, value)| {
                let name = key.strip_prefix(KIND_KEY)?.to_string();
                Some((name, declared(&value)?))
            })
            .collect();
        for m in members {
            m.kind = if self.store.session_of_member(&m.name)?.is_some() {
                Some(MemberKind::Session)
            } else {
                declared_by.get(&m.name).copied()
            };
        }
        Ok(())
    }
}

/// A stored declaration that is in the vocabulary and is one a member may make; anything else reads as no kind.
fn declared(value: &str) -> Option<MemberKind> {
    MemberKind::parse(value).filter(|k| k.declarable())
}
