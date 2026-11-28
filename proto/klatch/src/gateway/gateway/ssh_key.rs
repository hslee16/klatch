//! Mapping an ssh forced-command key label to a member (#107): a floor act of the human at a terminal, kept in the
//! policy table as `ssh-key:<label>` and audited. See `gateway/ssh_key.rs` for the identity built from it.

use super::*;
use crate::domain::is_reserved_name;
use crate::gateway::ssh_key::{POLICY_PREFIX, valid_label};

impl<S: Store> Gateway<S> {
    /// Only the human: map the key label `label` to `member`, or with `on` false take the mapping away (the label must
    /// be mapped to that member). A label has one member: mapping it to another is refused until it is unmapped, so a
    /// key never moves between members by accident. A floor act: no agent, the admin agent included.
    pub fn admin_set_ssh_key(
        &self,
        who: &Principal,
        label: &str,
        member: &str,
        on: bool,
    ) -> Result<()> {
        if !valid_label(label) {
            return Err(Error::Invalid(format!(
                "bad ssh key label: \"{}\"; use 1 to 40 letters, digits, '.', '_' or '-'",
                shown(label)
            )));
        }
        if !valid_name(member) || is_reserved_name(member) {
            return Err(Error::Invalid(format!(
                "bad member name: {}",
                shown(member)
            )));
        }
        self.administer(who, ("ssh-key", &format!("{label}/{member}={on}")), |now| {
            let key = format!("{POLICY_PREFIX}{label}");
            let held = self.store.policy_get(&key)?;
            match (on, held.as_deref()) {
                (true, Some(other)) if other != member => {
                    let e = Error::Refused {
                        rule: "ssh-key-mapped".into(),
                        reason: format!(
                            "key {label} is already mapped to {other}; unmap it first with `klatch admin ssh-key {label} {other} --off`"
                        ),
                    };
                    self.note_refusal(who, ("*", "admin:ssh-key"), &e);
                    return Err(e);
                }
                (true, _) => self.store.policy_set(&key, Some(member), now)?,
                (false, Some(h)) if h == member => self.store.policy_set(&key, None, now)?,
                (false, _) => {
                    return Err(Error::Invalid(format!(
                        "key {label} is not mapped to {member}; nothing was changed"
                    )));
                }
            }
            Ok(String::new())
        })
    }
}
