//! How a message is stored and read back: the attribution text, the columns every message query selects and the
//! row mapping.

use klatch_core::Message;

pub(super) fn attribution_text(a: klatch_core::Attribution) -> &'static str {
    match a {
        klatch_core::Attribution::Verified => "verified",
        klatch_core::Attribution::Unattributed => "unattributed",
    }
}

/// Anything other than `verified` reads as unattributed: an unknown value must never look proven.
pub(super) fn attribution_from(text: &str) -> klatch_core::Attribution {
    if text == "verified" {
        klatch_core::Attribution::Verified
    } else {
        klatch_core::Attribution::Unattributed
    }
}

/// The columns every message query selects, in the order `message_from_row` reads them.
pub(super) const MESSAGE_COLUMNS: &str = "mid,seq,sender,attribution,label,body,ts";

pub(super) fn message_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Message> {
    Ok(Message {
        id: r.get(0)?,
        seq: r.get(1)?,
        sender: r.get(2)?,
        attribution: attribution_from(&r.get::<_, String>(3)?),
        label: r.get(4)?,
        body: r.get(5)?,
        ts: r.get(6)?,
    })
}
