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

    fn set_ask_scope(&self, room: &str, seq: i64, scope: &str) -> Result<()> {
        self.exec(
            "INSERT INTO ask_scopes(room,seq,scope) VALUES(?1,?2,?3) \
             ON CONFLICT(room,seq) DO UPDATE SET scope=excluded.scope",
            params![room, seq, scope],
        )
        .map(|_| ())
    }

    fn ask_scopes(&self, room: &str) -> Result<Vec<(i64, String)>> {
        self.rows(
            "SELECT seq,scope FROM ask_scopes WHERE room=?1 ORDER BY seq",
            params![room],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    }

    fn set_ask_refs(&self, room: &str, seq: i64, refs: &[crate::domain::AskRef]) -> Result<()> {
        self.immediate(|| {
            self.exec(
                "DELETE FROM ask_refs WHERE room=?1 AND seq=?2",
                params![room, seq],
            )?;
            for (pos, r) in refs.iter().enumerate() {
                self.exec(
                    "INSERT INTO ask_refs(room,seq,pos,kind,value,label,snippet) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                    params![room, seq, pos as i64, r.kind, r.value, r.label, r.snippet],
                )?;
            }
            Ok(())
        })
    }

    fn ask_refs(&self, room: &str) -> Result<Vec<(i64, Vec<crate::domain::AskRef>)>> {
        let rows = self.rows(
            "SELECT seq,kind,value,label,snippet FROM ask_refs WHERE room=?1 ORDER BY seq,pos",
            params![room],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    crate::domain::AskRef {
                        kind: r.get(1)?,
                        value: r.get(2)?,
                        label: r.get(3)?,
                        snippet: r.get(4)?,
                    },
                ))
            },
        )?;
        let mut out: Vec<(i64, Vec<crate::domain::AskRef>)> = Vec::new();
        for (seq, r) in rows {
            match out.last_mut() {
                Some((last, refs)) if *last == seq => refs.push(r),
                _ => out.push((seq, vec![r])),
            }
        }
        Ok(out)
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

    fn route_ask(
        &self,
        room: &str,
        seq: i64,
        to: &[(String, crate::domain::RouteKind)],
        now: i64,
    ) -> Result<bool> {
        self.do_route_ask(room, seq, to, now)
    }

    fn add_ask_routes(
        &self,
        room: &str,
        seq: i64,
        to: &[(String, crate::domain::RouteKind)],
        now: i64,
    ) -> Result<()> {
        self.immediate(|| self.insert_routes(room, seq, to, now))
    }

    fn ask_routes(&self, room: &str) -> Result<Vec<crate::domain::AskRoute>> {
        self.do_ask_routes(room)
    }

    fn note_route_outcome(
        &self,
        room: &str,
        seq: i64,
        outcome: &str,
        n: i64,
        now: i64,
    ) -> Result<bool> {
        self.do_note_route_outcome(room, seq, outcome, n, now)
    }

    fn route_outcomes(&self, room: &str, seq: i64) -> Result<Vec<String>> {
        self.rows(
            "SELECT outcome FROM ask_route_events WHERE room=?1 AND seq=?2 ORDER BY ts, outcome",
            params![room, seq],
            |r| r.get(0),
        )
    }

    fn untold_routes(&self, room: &str, member: &str, bit: i64) -> Result<Vec<i64>> {
        self.rows(
            "SELECT seq FROM ask_routes WHERE room=?1 AND member=?2 AND kind='coordinator' AND (told & ?3)=0 ORDER BY seq",
            params![room, member, bit],
            |r| r.get(0),
        )
    }

    fn mark_told(&self, room: &str, member: &str, seqs: &[i64], bit: i64) -> Result<()> {
        self.immediate(|| {
            for seq in seqs {
                self.exec(
                    "UPDATE ask_routes SET told = told | ?4 WHERE room=?1 AND member=?2 AND seq=?3",
                    params![room, member, seq, bit],
                )?;
            }
            Ok(())
        })
    }

    fn route_events(&self, room: &str) -> Result<Vec<(i64, String, i64)>> {
        self.rows(
            "SELECT seq,outcome,ts FROM ask_route_events WHERE room=?1 ORDER BY seq,ts,outcome",
            params![room],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
    }
}
