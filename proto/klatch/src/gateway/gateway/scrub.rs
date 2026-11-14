//! The scrubber (R9, ticket #44): private terms the administrator lists refuse a post, claim reason, release note
//! or status in any room below the agent cap. It is a backstop: the real control is labels and clearance, and a
//! scrubber only knows what a person listed. A refusal names the rule and never the matched text.

use super::*;

const MAX_TERM_BYTES: usize = 200;
const PREFIX: &str = "scrub:";

impl<S: Store> Gateway<S> {
    /// Refuse `text` if it matches a listed term and the room is below the agent cap; a room at or above the cap
    /// is held to the credential rule only.
    pub(super) fn check_scrub(&self, room: &str, text: &str) -> Result<()> {
        let terms = self.store.policy_list(PREFIX)?;
        if terms.is_empty() {
            return Ok(());
        }
        let chain = self.chain()?;
        let label = self.room_label(room)?;
        if chain.dominates(&label, chain.default_cap()) {
            return Ok(());
        }
        let lower = text.to_lowercase();
        let hit = terms
            .iter()
            .find(|(_, term)| lower.contains(&term.to_lowercase()));
        match hit {
            None => Ok(()),
            Some((key, _)) => {
                let name = key.trim_start_matches(PREFIX);
                Err(Error::Refused {
                    rule: format!("scrub:{name}"),
                    reason: format!(
                        "the text matches the private-term rule {name}, which keeps it out of room {room} (labelled {label})"
                    ),
                })
            }
        }
    }

    /// List a private term under a rule name. Matching ignores case.
    pub fn admin_add_scrub_term(&self, who: &Principal, name: &str, term: &str) -> Result<()> {
        if !valid_name(name) {
            return Err(Error::Invalid(format!("bad rule name: {name}")));
        }
        let ok = !term.trim().is_empty() && plain_text(term, MAX_TERM_BYTES);
        invalid(
            ok,
            format!("bad term: 1 to {MAX_TERM_BYTES} bytes, no control characters"),
        )?;
        self.administer(who, ("scrub-add", name), |now| {
            self.store
                .policy_set(&format!("{PREFIX}{name}"), Some(term.trim()), now)?;
            Ok(String::new())
        })
    }

    /// Stop refusing a rule's term.
    pub fn admin_remove_scrub_term(&self, who: &Principal, name: &str) -> Result<()> {
        if !valid_name(name) {
            return Err(Error::Invalid(format!("bad rule name: {name}")));
        }
        self.administer(who, ("scrub-remove", name), |now| {
            let key = format!("{PREFIX}{name}");
            if self.store.policy_get(&key)?.is_none() {
                return Err(Error::NotFound(format!("no scrub rule {name}")));
            }
            self.store.policy_set(&key, None, now)?;
            Ok(String::new())
        })
    }

    /// The listed rules and their terms, by name.
    pub fn admin_scrub_terms(&self, who: &Principal) -> Result<Vec<(String, String)>> {
        check_member(who)?;
        self.guard.check(
            who,
            &Operation::Admin {
                action: "scrub-list".into(),
                target: String::new(),
            },
        )?;
        Ok(self
            .store
            .policy_list(PREFIX)?
            .into_iter()
            .map(|(k, v)| (k.trim_start_matches(PREFIX).to_string(), v))
            .collect())
    }
}
