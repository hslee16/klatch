//! Rewriting the stored text of one label under a key (#378): sealing a label over what is already in it, and moving a
//! sealed label to a new key. Both run in one transaction and keep the binding of a sealed text to its message, label
//! and field (`SqliteStore::sealed_context`), so a text moved between messages still fails to open.

use super::SqliteStore;
use super::be;
use crate::domain::{Error, Result};
use crate::keys::Key;
use crate::store::PolicyStore;

/// What moving one stored text to a new label key does with it.
pub(super) enum Moved {
    /// Plain text, or already under the new key: nothing to do.
    Keep,
    /// The same text sealed again under the new key, same message, label and field.
    To(String),
    /// Sealed, and neither key opens it.
    Stuck,
}

/// Decide what to do with `text` of `field` of message `mid` when `label`'s key moves from `old` to `new`. A text that
/// already opens under the new key is left as it is, which is what lets a repeated move finish a crashed one.
pub(super) fn move_text(
    (mid, label, field): (&str, &str, &str),
    text: &str,
    (old, new): (&Key, &Key),
) -> Result<Moved> {
    if !crate::keys::is_sealed_text(text) {
        return Ok(Moved::Keep);
    }
    let ctx = SqliteStore::sealed_context(mid, label, field);
    if crate::keys::reveal_text(new, ctx.as_bytes(), text).is_ok() {
        return Ok(Moved::Keep);
    }
    let Ok(plain) = crate::keys::reveal_text(old, ctx.as_bytes(), text) else {
        return Ok(Moved::Stuck);
    };
    crate::keys::protect_text(new, ctx.as_bytes(), &plain)
        .map(Moved::To)
        .map_err(|e| Error::Backend(format!("no random source: {e}")))
}

/// One message row of a label.
struct LabelRow {
    id: i64,
    mid: String,
    body: Option<String>,
    payload: Option<String>,
}

/// The new body and payload of the row with this id; `None` leaves a field as it is.
type Update = (i64, Option<String>, Option<String>);

impl SqliteStore {
    fn label_rows(&self, label: &str, live_only: bool) -> Result<Vec<LabelRow>> {
        let filter = if live_only {
            " AND COALESCE(kind,'')<>'archived'"
        } else {
            ""
        };
        self.rows(
            &format!("SELECT id, mid, body, payload FROM messages WHERE label=?1{filter}"),
            [label],
            |r| {
                Ok(LabelRow {
                    id: r.get(0)?,
                    mid: r.get(1)?,
                    body: r.get(2)?,
                    payload: r.get(3)?,
                })
            },
        )
    }

    /// Seal a label over what is already in it: mark it sealed and protect every earlier plain body and payload of
    /// the label under its key, in one transaction. Returns how many texts were converted. Archived tombstones are
    /// left alone, and nothing is marked or changed when there is something to convert and the key is not held.
    pub(super) fn mark_sealed_converting(&self, label: &str, now: i64) -> Result<usize> {
        self.immediate(|| {
            let mut todo: Vec<(i64, &str, String, String)> = Vec::new();
            for row in self.label_rows(label, true)? {
                for (field, text) in [("body", row.body), ("payload", row.payload)] {
                    if let Some(t) = text.filter(|t| !crate::keys::is_sealed_text(t)) {
                        todo.push((row.id, field, row.mid.clone(), t));
                    }
                }
            }
            let ring = self.labels.borrow();
            let key = ring.get(label);
            if key.is_none() && !todo.is_empty() {
                return Err(Error::Refused {
                    rule: "label-key".into(),
                    reason: format!(
                        "label {label} has earlier messages to seal and you hold no key for it"
                    ),
                });
            }
            self.policy_set(&format!("sealed:{label}"), Some("1"), now)?;
            let count = todo.len();
            let Some(key) = key else { return Ok(0) };
            for (id, field, mid, text) in todo {
                let sealed = crate::keys::protect_text(
                    key,
                    Self::sealed_context(&mid, label, field).as_bytes(),
                    &text,
                )
                .map_err(|e| Error::Backend(format!("no random source: {e}")))?;
                self.conn
                    .execute(
                        &format!("UPDATE messages SET {field}=?2 WHERE id=?1"),
                        rusqlite::params![id, sealed],
                    )
                    .map_err(be)?;
            }
            Ok(count)
        })
    }

    /// The rows of `label` that move from `old` to `new` and how many texts that is; an error naming the messages
    /// when a sealed text opens with neither key.
    fn plan_reseal(&self, label: &str, keys: (&Key, &Key)) -> Result<(Vec<Update>, usize)> {
        let mut plan = Vec::new();
        let mut stuck = Vec::new();
        let mut texts = 0;
        for row in self.label_rows(label, false)? {
            let mut next = [None, None];
            for (i, (field, text)) in [("body", &row.body), ("payload", &row.payload)]
                .into_iter()
                .enumerate()
            {
                let Some(text) = text else { continue };
                match move_text((&row.mid, label, field), text, keys)? {
                    Moved::Keep => {}
                    Moved::To(t) => {
                        next[i] = Some(t);
                        texts += 1;
                    }
                    Moved::Stuck => stuck.push(format!("{} {field}", row.mid)),
                }
            }
            let [body, payload] = next;
            if body.is_some() || payload.is_some() {
                plan.push((row.id, body, payload));
            }
        }
        if !stuck.is_empty() {
            return Err(Error::Invalid(format!(
                "{} sealed text(s) of label {label} open with neither its current key nor the new one, so the key was not changed (the key and the store disagree; restore the label's recovery code or a backup): {}",
                stuck.len(),
                stuck.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
            )));
        }
        Ok((plan, texts))
    }

    /// Whether every sealed text of `label` opens with `old` or `new`, changing nothing.
    pub fn check_label_reseal(&self, label: &str, old: &Key, new: &Key) -> Result<()> {
        self.plan_reseal(label, (old, new)).map(|_| ())
    }

    /// Move every sealed text of `label` from `old` to `new` in one transaction: reveal with the old key and protect
    /// with the new one, same message, label and field, so the binding does not change. A text that already opens
    /// with the new key is skipped. All or nothing: a text that opens with neither refuses the whole move. Returns
    /// how many texts were moved.
    pub fn reseal_label(&self, label: &str, old: &Key, new: &Key) -> Result<usize> {
        self.reseal_label_hooked(label, old, new, &|_| Ok(()))
    }

    /// `reseal_label` with a hook called after each row is rewritten, with the count so far; an error from it rolls
    /// the transaction back. A test uses it to stand in for a crash inside the transaction.
    pub(crate) fn reseal_label_hooked(
        &self,
        label: &str,
        old: &Key,
        new: &Key,
        after_each: &dyn Fn(usize) -> Result<()>,
    ) -> Result<usize> {
        self.immediate(|| {
            let (plan, texts) = self.plan_reseal(label, (old, new))?;
            for (done, (id, body, payload)) in plan.into_iter().enumerate() {
                self.conn
                    .execute(
                        "UPDATE messages SET body=COALESCE(?2, body), payload=COALESCE(?3, payload) WHERE id=?1",
                        rusqlite::params![id, body, payload],
                    )
                    .map_err(be)?;
                after_each(done + 1)?;
            }
            Ok(texts)
        })
    }

    /// How many sealed texts `label` has in all, and how many of them open with `a` and with `b`. A move that died
    /// after its transaction shows as none opening with the main key and all with the second one.
    pub fn label_open_counts(
        &self,
        label: &str,
        a: &Key,
        b: &Key,
    ) -> Result<(usize, usize, usize)> {
        let (mut total, mut with_a, mut with_b) = (0, 0, 0);
        for row in self.label_rows(label, false)? {
            for (field, text) in [("body", &row.body), ("payload", &row.payload)] {
                let Some(text) = text.as_deref().filter(|t| crate::keys::is_sealed_text(t)) else {
                    continue;
                };
                total += 1;
                let ctx = Self::sealed_context(&row.mid, label, field);
                with_a += usize::from(crate::keys::reveal_text(a, ctx.as_bytes(), text).is_ok());
                with_b += usize::from(crate::keys::reveal_text(b, ctx.as_bytes(), text).is_ok());
            }
        }
        Ok((total, with_a, with_b))
    }
}
