//! Sealed message text: what binds a sealed body to its message, and how it is stored and opened.

use super::SqliteStore;
use super::rows::message_from_row;
use crate::domain::{Error, Message, Result};
use crate::store::Store;

impl SqliteStore {
    /// What binds a sealed body to its message: moved to another message or label it no longer opens.
    pub(super) fn sealed_context(id: &str, label: &str, field: &str) -> String {
        format!("klatch message {id} under {label} field {field}")
    }

    /// The body to store: sealed under the label's key when the label is sealed. A sealed label without a key is
    /// refused (plain text must never be stored under it), and so is a plain body that looks like a sealed one.
    pub(super) fn seal_text(
        &self,
        (label, id, field): (&str, &str, &str),
        body: &str,
    ) -> Result<String> {
        let sealed = self.policy_get(&format!("sealed:{label}"))?.is_some();
        if !sealed {
            if crate::keys::is_sealed_text(body) {
                return Err(Error::Invalid(
                    "a message cannot start with klatch-sealed:1:, which is how sealed text is written".into(),
                ));
            }
            return Ok(body.to_string());
        }
        let ring = self.labels.borrow();
        let Some(key) = ring.get(label) else {
            return Err(Error::Refused {
                rule: "label-key".into(),
                reason: format!("label {label} is sealed and you hold no key for it"),
            });
        };
        crate::keys::protect_text(key, Self::sealed_context(id, label, field).as_bytes(), body)
            .map_err(|e| Error::Backend(format!("no random source: {e}")))
    }

    /// A stored text, opened if it is sealed and this principal holds the label's key, and replaced by a note if not.
    pub(super) fn open_text(&self, (id, label, field): (&str, &str, &str), text: String) -> String {
        if !crate::keys::is_sealed_text(&text) {
            return text;
        }
        let ctx = Self::sealed_context(id, label, field);
        self.labels
            .borrow()
            .get(label)
            .and_then(|k| crate::keys::reveal_text(k, ctx.as_bytes(), &text).ok())
            .unwrap_or_else(|| {
                format!(
                    "[sealed: this {label} message needs the {label} key, which you do not hold]"
                )
            })
    }

    /// A message row, with a sealed body opened if this principal holds the label's key and replaced by a note if not.
    pub(super) fn open_message(&self, r: &rusqlite::Row<'_>) -> rusqlite::Result<Message> {
        let mut m = message_from_row(r)?;
        m.body = self.open_text((&m.id, &m.label, "body"), m.body);
        m.payload = m
            .payload
            .map(|p| self.open_text((&m.id, &m.label, "payload"), p));
        Ok(m)
    }
}
