//! How a message is stored and read back: the attribution text, the columns every message query selects and the
//! row mapping.

use crate::domain::Message;

pub(super) fn attribution_text(a: crate::domain::Attribution) -> &'static str {
    match a {
        crate::domain::Attribution::Verified => "verified",
        crate::domain::Attribution::Unattributed => "unattributed",
    }
}

/// Anything other than `verified` reads as unattributed: an unknown value must never look proven.
pub(super) fn attribution_from(text: &str) -> crate::domain::Attribution {
    if text == "verified" {
        crate::domain::Attribution::Verified
    } else {
        crate::domain::Attribution::Unattributed
    }
}

/// The columns every message query selects, in the order `message_from_row` reads them.
pub(super) const MESSAGE_COLUMNS: &str = "mid,seq,sender,attribution,label,body,ts,kind,payload";

pub(super) fn message_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Message> {
    Ok(Message {
        id: r.get(0)?,
        seq: r.get(1)?,
        sender: r.get(2)?,
        attribution: attribution_from(&r.get::<_, String>(3)?),
        label: r.get(4)?,
        body: r.get(5)?,
        ts: r.get(6)?,
        kind: r.get(7)?,
        payload: r.get(8)?,
    })
}
