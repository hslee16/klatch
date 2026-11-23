//! The automatic machine checkpoint (#1408): what the opt-in Stop and SessionEnd hooks write so a successor, or the
//! same member after a resume, finds where the session stood. The trail it keeps between hooks holds tool classes and
//! repo-relative paths of edits and writes, never a command, content or output; the checkpoint goes only to rooms the
//! member can read and works in (a task, a claim or an open ask there; else the default lobby).

use super::*;
use crate::domain::{
    MachineState, TRAIL_PATH_MAX_BYTES, ToolClass, end_word, machine_body, status_word,
};

/// The room a member with no work of its own in any room still leaves its checkpoint in.
const DEFAULT_ROOM: &str = "lobby";

fn class_of(word: &str) -> Option<ToolClass> {
    [
        ToolClass::Edit,
        ToolClass::Write,
        ToolClass::Run,
        ToolClass::Read,
        ToolClass::Other,
    ]
    .into_iter()
    .find(|c| c.word() == word)
}

/// The first word of the member's last status on the board.
fn status_in(board: &crate::domain::Board, member: &str) -> Option<String> {
    board
        .members
        .iter()
        .find(|m| m.name == member)
        .and_then(|m| m.status.as_deref())
        .and_then(status_word)
}

/// Whether a state holds nothing a successor would need: no files, tools, claims or open asks.
fn is_quiet(state: &MachineState) -> bool {
    state.files.is_empty()
        && state.tools.is_empty()
        && state.claims.is_empty()
        && state.asks_open.is_empty()
}

/// A path a trail may keep: repo-relative, plain, short.
fn plain_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= TRAIL_PATH_MAX_BYTES
        && !p.starts_with('/')
        && !p.split('/').any(|s| s == "..")
        && !p.chars().any(char::is_control)
}

/// The caller's own state that every room's checkpoint is drawn from: who, the trail the hooks kept, the end word
/// and the time.
#[derive(Clone, Copy)]
struct Own<'a> {
    member: &'a str,
    trail: &'a crate::domain::Trail,
    ended: Option<&'static str>,
    now: i64,
}

impl<S: Store> Gateway<S> {
    /// Add one tool call to the caller's trail: its class word and, for an edit or a write, the repo-relative path.
    /// Anything else (a class outside the vocabulary, a path that is not plain) is refused, so the trail cannot carry
    /// text a hook did not mean to send.
    pub fn note_tool(&self, who: &Principal, class: &str, path: Option<&str>) -> Result<()> {
        check_member(who)?;
        self.check_own_record_live(who)?;
        let class = class_of(class).ok_or_else(|| Error::Invalid("bad tool class".into()))?;
        let paths: Vec<String> = match path {
            Some(p) if class.keeps_paths() => {
                if !plain_path(p) {
                    return Err(Error::Invalid("bad trail path".into()));
                }
                vec![p.to_string()]
            }
            _ => Vec::new(),
        };
        let mut trail = self.store.trail_of(&who.member)?;
        trail.push(class, &paths);
        self.store.save_trail(&who.member, &trail, self.now())
    }

    /// Write the caller's machine checkpoint (at a Stop, or at a SessionEnd with the end `reason`), replacing the
    /// one it wrote before. Rooms the caller cannot read are skipped. Returns how many were written.
    pub fn write_machine_checkpoint(&self, who: &Principal, ended: Option<&str>) -> Result<usize> {
        check_member(who)?;
        self.check_own_record_live(who)?;
        let now = self.now();
        let member = who.member.as_str();
        let trail = self.store.trail_of(member)?;
        let ended = ended.map(end_word);
        let own = Own {
            member,
            trail: &trail,
            ended,
            now,
        };
        let mut written = 0;
        for room in self.store.rooms_of(member)? {
            // the board is a read: a room the member cannot read refuses here and nothing of it is recorded
            let Ok(board) = self.board(who, &room) else {
                continue;
            };
            let Some(state) = self.room_machine_state(own, &room, &board)? else {
                continue;
            };
            let body = machine_body(&state);
            self.store
                .put_machine_checkpoint((&room, member), &body, ended.is_some(), now)?;
            written += 1;
        }
        if ended.is_some() {
            self.store.clear_trail(member)?;
        }
        Ok(written)
    }

    /// What the caller's checkpoint says for `room`, or `None` when none is written there: the member has no work
    /// in the room and it is not the default one, or there is nothing to say and the session did not end.
    fn room_machine_state(
        &self,
        own: Own,
        room: &str,
        board: &crate::domain::Board,
    ) -> Result<Option<MachineState>> {
        let member = own.member;
        let claims: Vec<String> = self
            .store
            .claims(room, own.now)?
            .into_iter()
            .filter(|c| c.owner == member)
            .map(|c| c.path)
            .collect();
        let mine: Vec<&crate::domain::OpenAsk> =
            board.asks.iter().filter(|a| a.sender == member).collect();
        let has_task = board
            .tasks
            .iter()
            .any(|t| t.owner == member && !t.undeclared);
        let works_here = has_task || !claims.is_empty() || !mine.is_empty();
        if !works_here && room != DEFAULT_ROOM {
            return Ok(None);
        }
        let state = MachineState {
            files: own.trail.files.clone(),
            tools: own.trail.tools.clone(),
            claims,
            asks_open: mine.iter().map(|a| a.seq).collect(),
            asks_waiting: mine
                .iter()
                .filter(|a| a.waiting_on_asker.is_some())
                .map(|a| a.seq)
                .collect(),
            status: status_in(board, member),
            ended: own.ended,
        };
        Ok((!is_quiet(&state) || own.ended.is_some()).then_some(state))
    }
}
