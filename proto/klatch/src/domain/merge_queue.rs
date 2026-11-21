//! The merge queue on the board (#864, step 2 of epic #863; DESIGN item 84 is the model). Five log words and one
//! claim, derived like the open-PR section and stored nowhere: `ready: #N` enqueues PR N, and the integrator (#866)
//! says how it goes with `integrating: #N`, `checking: #N`, `merged: #N` and `dropped: #N <reason>`. klatch reads
//! nothing from the hosting service; the integrator posts these words from its own events, in these fixed templates.
//!
//! Who may post what. `ready:` is any member's, for any PR (a PR number is a name, not a verified PR, as item 84
//! says, and a reviewer or the owner may well be the one who says it is ready; the integrator checks the facts).
//! The state words are an integrator's or the human's (`admin`): the caller names who that is, and a state word from
//! anyone else is ignored. `merged:` keeps its item 84 meaning for everyone: it ends the PR, so from a plain member it
//! drops the PR from the queue silently, and only from an integrator does it show as `merged`.

use crate::domain::hook::clip;
use crate::domain::parked::span;
use crate::domain::pr_lifecycle::number_of;
use crate::domain::{MERGED_PREFIX, Message, flat};

/// A message starting with this text enqueues a pull request for merging.
pub const READY_PREFIX: &str = "ready:";
/// The integrator says it is merging the PR now.
pub const INTEGRATING_PREFIX: &str = "integrating:";
/// The integrator says the checks it waits for (on the PR head or on main after the merge) are running.
pub const CHECKING_PREFIX: &str = "checking:";
/// The integrator says the PR left the queue without merging; the first word after the number is the reason.
pub const DROPPED_PREFIX: &str = "dropped:";

/// The integrator says it merged the PR and then reverted it because main went red (#1094).
pub const REVERTED_PREFIX: &str = "reverted:";
/// The integrator says it stopped: `halted: main red after #N` or `halted: main not verified after #N` (#1094).
pub const HALTED_PREFIX: &str = "halted:";

/// The prefixes of every word that `merge_queue` and `merge_halt` fold, for a store query that reads only those messages.
pub const QUEUE_WORDS: [&str; 7] = [
    READY_PREFIX,
    INTEGRATING_PREFIX,
    CHECKING_PREFIX,
    DROPPED_PREFIX,
    MERGED_PREFIX,
    REVERTED_PREFIX,
    HALTED_PREFIX,
];

/// How many ended entries (merged or dropped) the board keeps showing, newest first.
pub const QUEUE_ENDED_SHOWN: usize = 5;

/// Why a PR left the queue: a fixed list, so the integrator's word is never echoed as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    /// A code conflict with main.
    Conflict,
    /// The checks on the PR head failed.
    ChecksFailed,
    /// The PR was closed without merging.
    Closed,
    /// It was merged and then reverted because main went red.
    Reverted,
    /// Any other word after the number.
    Other,
}

impl DropReason {
    /// The word the integrator posts after the number.
    pub fn word(self) -> &'static str {
        match self {
            Self::Conflict => "conflict",
            Self::ChecksFailed => "checks-failed",
            Self::Closed => "closed",
            Self::Reverted => "reverted",
            Self::Other => "other",
        }
    }

    /// What the board says.
    pub fn phrase(self) -> &'static str {
        match self {
            Self::Conflict => "code conflict with main",
            Self::ChecksFailed => "checks failed",
            Self::Closed => "closed without merging",
            Self::Reverted => "merge reverted after main went red",
            Self::Other => "other reason",
        }
    }

    fn parse(text: &str) -> Self {
        match text
            .split_whitespace()
            .next()
            .map(str::to_lowercase)
            .as_deref()
        {
            Some("conflict") => Self::Conflict,
            Some("checks-failed") => Self::ChecksFailed,
            Some("closed") => Self::Closed,
            Some("reverted") => Self::Reverted,
            _ => Self::Other,
        }
    }
}

/// Where a PR is in the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueState {
    Waiting,
    Integrating,
    Checking,
    Merged,
    /// Merged and then reverted by the integrator because main went red (#1094).
    Reverted,
    Dropped(DropReason),
}

impl QueueState {
    /// Still in the queue: waiting for a turn, being merged, or being checked.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Waiting | Self::Integrating | Self::Checking)
    }
}

/// One PR in the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueEntry {
    pub number: u64,
    /// The member whose `ready:` word enqueued it (the first one since it last left the queue).
    pub enqueued_by: String,
    pub state: QueueState,
    /// When it was enqueued.
    pub ts: i64,
    /// When its state last changed (the enqueue time while it waits).
    pub changed: i64,
    /// 1-based place among the active entries, in the order they were enqueued; `None` once it has ended.
    pub position: Option<usize>,
    /// The member holding a live claim on `pr:N`, if any (set by the reader).
    pub owner: Option<String>,
    seq: i64,
}

impl QueueEntry {
    /// The claim path of the PR itself.
    pub fn path(&self) -> String {
        format!("pr:{}", self.number)
    }

    /// One line for the board.
    pub fn line(&self, now: i64) -> String {
        let owner = self
            .owner
            .as_deref()
            .map(|o| format!(", owner {}", flat(o)))
            .unwrap_or_default();
        let ready = format!(
            "ready by {} {} ago{owner}",
            flat(&self.enqueued_by),
            span(now - self.ts)
        );
        let (n, p) = (self.number, self.position.unwrap_or(0));
        match self.state {
            QueueState::Waiting => format!("{p}. #{n} waiting ({ready})"),
            QueueState::Integrating => format!("{p}. #{n} integrating ({ready})"),
            QueueState::Checking => format!("{p}. #{n} checking ({ready})"),
            QueueState::Dropped(r) => format!(
                "#{n} dropped: {} ({} ago; `ready: #{n}` enqueues it again)",
                clip(r.phrase()),
                span(now - self.changed)
            ),
            QueueState::Merged => format!("#{n} merged ({} ago)", span(now - self.changed)),
            QueueState::Reverted => format!(
                "#{n} reverted, main went red ({} ago)",
                span(now - self.changed)
            ),
        }
    }
}

/// The word `body` is, with the PR number it names and the text after the number.
fn parsed(body: &str) -> Option<(&'static str, u64, &str)> {
    QUEUE_WORDS.iter().find_map(|p| {
        let (n, rest) = body.strip_prefix(p).and_then(number_of)?;
        Some((*p, n, rest))
    })
}

/// The queue in `messages`: active entries in the order they were enqueued with their positions, then the last
/// `QUEUE_ENDED_SHOWN` ended ones, newest first. `trusted` says which senders may post the state words (the integrator
/// role and the human). The owner is left unset.
pub fn merge_queue(messages: &[Message], trusted: &dyn Fn(&str) -> bool) -> Vec<QueueEntry> {
    let mut ordered: Vec<&Message> = messages.iter().collect();
    ordered.sort_by_key(|m| m.seq);
    let mut entries: Vec<QueueEntry> = Vec::new();
    for m in ordered {
        let Some((word, number, rest)) = parsed(&m.body) else {
            continue;
        };
        let at = entries.iter().position(|e| e.number == number);
        if word == READY_PREFIX {
            match at {
                None => entries.push(enqueued(number, m)),
                Some(i) if matches!(entries[i].state, QueueState::Dropped(_)) => {
                    entries.remove(i);
                    entries.push(enqueued(number, m));
                }
                Some(_) => {}
            }
            continue;
        }
        let Some(i) = at else { continue };
        let vouched = trusted(&m.sender);
        if word == MERGED_PREFIX && !vouched {
            // item 84: anyone may say a PR is over, which takes it out of the queue without a claim of success
            entries.remove(i);
            continue;
        }
        let e = &mut entries[i];
        let next = match word {
            INTEGRATING_PREFIX if vouched && e.state.is_active() => Some(QueueState::Integrating),
            CHECKING_PREFIX if vouched && e.state.is_active() => Some(QueueState::Checking),
            DROPPED_PREFIX if vouched && e.state.is_active() => {
                Some(QueueState::Dropped(DropReason::parse(rest)))
            }
            MERGED_PREFIX if !matches!(e.state, QueueState::Merged | QueueState::Reverted) => {
                Some(QueueState::Merged)
            }
            REVERTED_PREFIX if vouched && e.state == QueueState::Merged => {
                Some(QueueState::Reverted)
            }
            _ => None,
        };
        if let Some(s) = next.filter(|s| *s != e.state) {
            e.state = s;
            e.changed = m.ts;
        }
    }
    let (mut active, mut ended): (Vec<_>, Vec<_>) =
        entries.into_iter().partition(|e| e.state.is_active());
    active.sort_by_key(|e| e.seq);
    for (i, e) in active.iter_mut().enumerate() {
        e.position = Some(i + 1);
    }
    ended.sort_by_key(|e| std::cmp::Reverse((e.changed, e.seq)));
    ended.truncate(QUEUE_ENDED_SHOWN);
    active.extend(ended);
    active
}

/// Why the integrator stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HaltReason {
    /// Main was red after the merge and could not be put right.
    MainRed,
    /// Main could not be verified after the merge.
    MainNotVerified,
}

/// The integrator's halt (#1094): it stopped after PR `number` and merges nothing more until a human starts it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Halt {
    pub number: u64,
    pub reason: HaltReason,
    /// When the halt word was posted.
    pub ts: i64,
}

impl Halt {
    /// One line for the board; fixed text, nothing of the message is echoed except the number.
    pub fn line(&self, now: i64) -> String {
        let why = match self.reason {
            HaltReason::MainRed => "main red",
            HaltReason::MainNotVerified => "main not verified",
        };
        format!(
            "integrator halted: {why} after #{} ({} ago; it merges nothing more until a human looks at main and starts it again)",
            self.number,
            span(now - self.ts)
        )
    }
}

fn halted(body: &str) -> Option<(HaltReason, u64)> {
    let rest = body.strip_prefix(HALTED_PREFIX)?.trim_start();
    let (reason, tail) = if let Some(t) = rest.strip_prefix("main red after ") {
        (HaltReason::MainRed, t)
    } else {
        (
            HaltReason::MainNotVerified,
            rest.strip_prefix("main not verified after ")?,
        )
    };
    Some((reason, number_of(tail)?.0))
}

/// The integrator's current halt in `messages`, if any: the newest `halted:` word from a trusted sender that no later
/// trusted state word (`integrating:`, `checking:`, `merged:`, `dropped:`, `reverted:`) has followed, since those
/// words mean the integrator is running again.
pub fn merge_halt(messages: &[Message], trusted: &dyn Fn(&str) -> bool) -> Option<Halt> {
    let mut ordered: Vec<&Message> = messages.iter().collect();
    ordered.sort_by_key(|m| m.seq);
    let mut halt = None;
    for m in ordered {
        if !trusted(&m.sender) {
            continue;
        }
        if let Some((reason, number)) = halted(&m.body) {
            halt = Some(Halt {
                number,
                reason,
                ts: m.ts,
            });
        } else if parsed(&m.body).is_some_and(|(w, _, _)| w != READY_PREFIX) {
            halt = None;
        }
    }
    halt
}

fn enqueued(number: u64, m: &Message) -> QueueEntry {
    QueueEntry {
        number,
        enqueued_by: m.sender.clone(),
        state: QueueState::Waiting,
        ts: m.ts,
        changed: m.ts,
        position: None,
        owner: None,
        seq: m.seq,
    }
}

#[cfg(test)]
mod tests;
