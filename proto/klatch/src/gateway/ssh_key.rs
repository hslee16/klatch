//! The ssh forced-command key (#107, DESIGN section 20 rule 11): ssh carries the bytes into `klatch serve --stdio`, and
//! sshd runs the command the administrator wrote beside the key in `authorized_keys`, so the caller cannot choose a
//! name. That command names a key label (`--ssh-key <label>`), never a member; the human maps the label to one member
//! at a terminal (`klatch admin ssh-key`, a floor act). This module holds what the mapping is made of: the policy key,
//! the label rule, the lookup and the `Identity` that turns the evidence into a principal. klatch opens no socket and
//! links no ssh code for any of it.

use crate::domain::{Error, Principal, Result};
use crate::gateway::{Identity, PeerEvidence};
use crate::store::PolicyStore;

/// The policy entry of a label: `ssh-key:<label>` holds the member.
pub(crate) const POLICY_PREFIX: &str = "ssh-key:";

/// The longest label.
const MAX_LABEL: usize = 40;

/// Whether `label` is one an `authorized_keys` command line can carry as a bare word: 1 to 40 letters, digits, `.`, `_`
/// and `-`. No space, quote or shell character, so a label cannot add an option to the command line.
pub fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= MAX_LABEL
        && label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The member a label is mapped to, if the human mapped it. A policy lookup that takes no principal: the decision to
/// serve a session is made by the caller (`klatch serve --ssh-key`), which refuses an unmapped label.
pub fn ssh_key_member<S: PolicyStore + ?Sized>(store: &S, label: &str) -> Result<Option<String>> {
    if !valid_label(label) {
        return Ok(None);
    }
    store.policy_get(&format!("{POLICY_PREFIX}{label}"))
}

/// Maps `SshForcedKey` evidence through the labels the human mapped. A mapped label is a verified member (sshd
/// authenticated the key and the human named the member); any other label is unattributed (R23) and names only the
/// kind of evidence. Evidence of another kind, a typed name above all, is not taken: the identity fails closed.
pub struct SshKeyIdentity<'a, S: PolicyStore + ?Sized> {
    store: &'a S,
}

impl<'a, S: PolicyStore + ?Sized> SshKeyIdentity<'a, S> {
    pub fn new(store: &'a S) -> Self {
        Self { store }
    }
}

impl<S: PolicyStore + ?Sized> Identity for SshKeyIdentity<'_, S> {
    fn resolve(&self, evidence: &PeerEvidence) -> Result<Principal> {
        let PeerEvidence::SshForcedKey { label } = evidence else {
            return Err(Error::Invalid(
                "an ssh session takes only the key label its forced command names, never a typed name".into(),
            ));
        };
        Ok(match ssh_key_member(self.store, label)? {
            Some(member) => Principal::verified(&member),
            None => Principal::unattributed(&format!("unmapped-{}", evidence.kind())),
        })
    }
}
