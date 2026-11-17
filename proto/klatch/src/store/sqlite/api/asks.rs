//! The SQLite side of the `AskStore` area.

use super::super::{SqliteStore, asks};
use crate::domain::{AskRoom, OpenAsk, Result};
use crate::store::AskStore;
use crate::store::Seat;
use rusqlite::params;

impl AskStore for SqliteStore {
    fn rooms_with_prefix(&self, prefix: &str) -> Result<Vec<AskRoom>> {
        self.rows(
            "SELECT m.room, COUNT(DISTINCT m.name), MAX(m.joined),
                        COALESCE((SELECT MAX(x.ts) FROM messages x WHERE x.room=m.room), 0)
                 FROM members m WHERE substr(m.room,1,?2)=?1 GROUP BY m.room ORDER BY m.room",
            params![prefix, prefix.chars().count() as i64],
            |r| {
                let seen: i64 = r.get(2)?;
                let said: i64 = r.get(3)?;
                Ok(AskRoom {
                    name: r.get(0)?,
                    members: r.get::<_, i64>(1)? as usize,
                    last_activity: seen.max(said),
                })
            },
        )
    }

    fn lapsed_asks(&self, room: &str, member: &str, now: i64) -> Result<Vec<(OpenAsk, String)>> {
        self.immediate(|| {
            let mut out = Vec::new();
            for a in self.open_asks(room, now)? {
                if a.sender != member || a.claimed {
                    continue;
                }
                let Some((owner, expires)) = self
                    .ask_lease(room, a.seq)?
                    .filter(|(_, expires)| *expires <= now)
                else {
                    continue;
                };
                if self.report_lapse_once((room, member), a.seq, expires)? {
                    out.push((a, owner));
                }
            }
            Ok(out)
        })
    }

    fn stale_asks(&self, at: Seat, now: i64, after: i64) -> Result<Vec<OpenAsk>> {
        let Seat { room, member } = at;
        self.immediate(|| {
            let stale = self
                .open_asks(room, now)?
                .into_iter()
                .filter(|a| asks::is_stale(a, member, now, after))
                .collect();
            self.report_once("asks_reported", (room, member), stale)
        })
    }

    fn escalated_asks(&self, room: &str, member: &str, now: i64) -> Result<Vec<OpenAsk>> {
        self.immediate(|| {
            let escalated = self
                .open_asks(room, now)?
                .into_iter()
                .filter(|a| a.sender == member && a.no_capable_member())
                .collect();
            self.report_once("asks_escalated", (room, member), escalated)
        })
    }
}
