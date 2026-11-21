//! `admin export`: a room's messages as readable text or JSON lines. Plaintext leaves on purpose, so it is audited.

use super::keys::KeyCtx;
use super::*;

/// One message as a line of JSON: the derived, regenerable transcript form (#71). Every field the store keeps.
pub(super) fn export_json_line(room: &str, m: &klatch::domain::Message) -> String {
    serde_json::json!({
        "room": room,
        "id": m.id,
        "seq": m.seq,
        "sender": m.sender,
        "attribution": match m.attribution {
            klatch::domain::Attribution::Verified => "verified",
            klatch::domain::Attribution::Unattributed => "unattributed",
        },
        "label": m.label,
        "ts": m.ts,
        "body": m.body,
        "kind": m.kind,
        "payload": m.payload,
    })
    .to_string()
}

/// One message as a line of readable text.
fn export_text_line(m: &klatch::domain::Message) -> String {
    format!(
        "[{} {} {} {}] {}",
        m.seq,
        m.ts,
        m.sender_label(),
        m.label,
        flat(&m.body)
    )
}

pub(super) fn export(ctx: &KeyCtx, room: &str, json: bool) -> Result<()> {
    let gateway = Gateway::new(custody::open_store(ctx.db)?, CliClock, AllowAll);
    gateway.use_label_keys(custody::label_ring(ctx.db, &gateway.sealed_labels()?)?);
    // audited before anything is printed: the act is recorded even if the output is cut short
    gateway.admin_note(&ctx.who, ("export", room), "readable transcript printed")?;
    // the human's own named path (#1196): a direct message is exportable here and through no member-reachable reader
    for m in gateway.admin_tail_scan(&ctx.who, room, (0, 1_000_000))?.0 {
        if json {
            println!("{}", export_json_line(room, &m));
        } else {
            println!("{}", export_text_line(&m));
        }
    }
    Ok(())
}
