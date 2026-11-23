//! `admin stats`: a counts-only usage report (#1377). Reads the store file read-only; prints counts, timestamps and
//! room and member names, never message text, ask text, claim detail, labels or payloads. Gated like the other admin
//! read acts that print stored metadata (the passphrase, and standard output must be the person's own terminal).

use super::keys::KeyCtx;
use super::*;
use klatch::domain::{StatsOptions, start_of_day};

pub(super) fn stats(
    ctx: &KeyCtx,
    room: Option<String>,
    since: Option<String>,
    top: usize,
    json: bool,
) -> Result<()> {
    let since_ts = match since.as_deref() {
        Some(d) => Some(
            start_of_day(d)
                .ok_or_else(|| anyhow::anyhow!("--since takes a day as YYYY-MM-DD (UTC)"))?,
        ),
        None => None,
    };
    let key = ctx.key_or("this store has no key yet; run any klatch command to make one")?;
    let opts = StatsOptions {
        room: room.clone(),
        since: since_ts,
        top,
        now: now(),
    };
    let report = klatch::store::SqliteStore::read_usage_stats(ctx.db, &key, &opts)?;
    if json {
        println!("{}", report.to_json());
    } else {
        print!("{}", report.to_text());
    }
    ctx.note(
        "stats",
        &format!(
            "room {} since {}",
            room.as_deref().unwrap_or("all"),
            since.as_deref().unwrap_or("start")
        ),
    )?;
    Ok(())
}
