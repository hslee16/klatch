//! The delivery view (#186, DESIGN 24.1 decision 5): what hooks and the wake count show a member. `all` is the default
//! and shows everything. A member may narrow its own view to `scope`: messages from members in its own scope (its
//! repository), messages addressed to it (`@name`, or any message in a direct-message room it is in) and broadcasts
//! (`@all`). It is a view setting, not a room concept: nothing is deleted or marked read, and an explicit `read`,
//! `history` or `wait` is never narrowed, so narrowing can only delay news that is not for the member, never hide it.

use super::*;
use std::collections::HashMap;

const KEY: &str = "view:";
/// The word that addresses every member (`@all`).
const BROADCAST: &str = "all";

/// What a member is shown without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryView {
    All,
    Scope,
}

impl<S: Store> Gateway<S> {
    /// The caller's delivery view (`all` unless it narrowed it).
    pub fn delivery_view(&self, who: &Principal) -> Result<DeliveryView> {
        check_member(who)?;
        Ok(
            match self.store.policy_get(&format!("{KEY}{}", who.member))? {
                Some(v) if v == "scope" => DeliveryView::Scope,
                _ => DeliveryView::All,
            },
        )
    }

    /// Set the caller's own delivery view. Only its own: the key is built from the caller's name.
    pub fn set_delivery_view(&self, who: &Principal, view: DeliveryView) -> Result<()> {
        check_member(who)?;
        let value = (view == DeliveryView::Scope).then_some("scope");
        self.store
            .policy_set(&format!("{KEY}{}", who.member), value, self.now())
    }

    /// The messages the caller's view lets through. Addressed messages always pass, so a narrowed view never hides
    /// what is for the member.
    pub(super) fn in_view(
        &self,
        who: &Principal,
        room: &str,
        messages: Vec<Message>,
    ) -> Result<Vec<Message>> {
        if self.delivery_view(who)? == DeliveryView::All || self.dm_pair(room)?.is_some() {
            return Ok(messages);
        }
        let mine = self.store.member_scope(&who.member)?;
        let mut scopes: HashMap<String, Option<String>> = HashMap::new();
        let mut kept = Vec::new();
        for m in messages {
            let addressed = crate::domain::mentions(&m.body, &who.member)
                || crate::domain::mentions(&m.body, BROADCAST);
            let same_scope = match scopes.get(&m.sender) {
                Some(s) => *s == mine,
                None => {
                    let theirs = self.store.member_scope(&m.sender)?;
                    let same = theirs == mine;
                    scopes.insert(m.sender.clone(), theirs);
                    same
                }
            };
            if addressed || same_scope {
                kept.push(m);
            }
        }
        Ok(kept)
    }
}
