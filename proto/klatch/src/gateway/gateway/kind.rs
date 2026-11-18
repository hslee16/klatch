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
        if self.store.session_of_member(&who.member)?.is_some() {
            return Ok(Some(MemberKind::Session));
        }
        Ok(self
            .store
            .policy_get(&format!("{KIND_KEY}{}", who.member))?
            .as_deref()
            .and_then(declared))
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
