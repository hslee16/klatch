//! Durable tasks (DESIGN section 29 item 7, section 32 item 48): work that outlives a session, derived from the log
//! like asks and offers. Four words: `working: <what>` starts or renews a task, `checkpoint: <state needed to
//! resume>` records where it stands (the latest wins), `handoff: <task> to <member>` gives it to another member and
//! `finished: <task>` ends it. A task is keyed by its owner and a short title; nothing is stored.

use crate::domain::parked::span;
use crate::domain::{Message, valid_name};

/// A message starting with this text starts or renews its sender's task.
pub const WORKING_PREFIX: &str = "working:";
/// A message starting with this text records the state needed to resume a task.
pub const CHECKPOINT_PREFIX: &str = "checkpoint";
/// A message starting with this text gives a task to another member.
pub const HANDOFF_PREFIX: &str = "handoff:";
/// A message starting with this text ends a task.
pub const FINISHED_PREFIX: &str = "finished";
/// The prefixes of every word that `open_tasks` folds, for a store query that reads only those messages.
pub const TASK_WORDS: [&str; 4] = [
    WORKING_PREFIX,
    CHECKPOINT_PREFIX,
    HANDOFF_PREFIX,
    FINISHED_PREFIX,
];
/// The longest title:the text before the first colon, or this many characters.
pub const TITLE_MAX_CHARS: usize = 60;
/// How long after its last word a task keeps its owner from being pruned. A task whose owner went away for good
/// stays visible as orphaned from the log, but must not hold the owner's rows forever.
pub const TASK_HOLD_SECS: i64 = 7 * 86_400;

/// The latest recorded state of a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    pub text: String,
    pub ts: i64,
}

/// A task nobody has finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub owner: String,
    pub title: String,
    /// What the owner said it works on (the text after `working:`).
    pub what: String,
    pub started: i64,
    /// The time of the latest word about it (a renewal, a checkpoint or a handoff).
    pub last: i64,
    pub checkpoint: Option<Checkpoint>,
    /// The machine checkpoint a hook wrote for the owner (#1408), shown beneath the owner's own checkpoint, which wins.
    /// Set by the reader; the fold of messages leaves it `None`.
    pub machine: Option<Checkpoint>,
    /// Set by the reader: the owner declared no task, and this one stands only for its machine checkpoint.
    pub undeclared: bool,
    /// The member that handed it over, if it was handed over.
    pub from: Option<String>,
    /// Set by the reader: the owner is gone (not present, not parked, holds nothing), so the task waits for someone.
    pub orphaned: bool,
}

impl Task {
    /// One line for the board or `klatch tasks`. `full` keeps the whole checkpoint instead of a clipped one.
    pub fn line(&self, now: i64, full: bool) -> String {
        let flatten = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let clipped = |s: &str| {
            let text = flatten(s);
            if full || text.chars().count() <= 200 {
                text
            } else {
                format!("{}...", text.chars().take(200).collect::<String>())
            }
        };
        let mut out = format!("{}: {}", flatten(&self.owner), flatten(&self.title));
        let mut notes = Vec::new();
        if self.undeclared {
            notes.push("no task declared".to_string());
            if self.orphaned {
                notes.push("orphaned".to_string());
            }
        } else if self.orphaned {
            notes.push(format!("orphaned, last word {} ago", span(now - self.last)));
        } else {
            notes.push(format!("working {}", span(now - self.started)));
        }
        if let Some(from) = &self.from {
            notes.push(format!("handed over by {}", flatten(from)));
        }
        if let Some(c) = &self.checkpoint {
            notes.push(format!(
                "checkpoint {} ago: {}",
                span(now - c.ts),
                clipped(&c.text)
            ));
        }
        if let Some(c) = &self.machine {
            notes.push(format!(
                "{} {} ago: {}",
                crate::domain::MACHINE_MARK,
                span(now - c.ts),
                clipped(&c.text)
            ));
        }
        out.push_str(&format!(" ({})", notes.join(", ")));
        out
    }
}

/// The title of a task from the text after `working:`: `#N` for an ask number, else the text before the first colon
/// (or the first `TITLE_MAX_CHARS` characters).
fn title_of(what: &str) -> String {
    let what = what.trim();
    if let Some(rest) = what.strip_prefix('#') {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            return format!("#{digits}");
        }
    }
    let head = what.split(':').next().unwrap_or(what).trim();
    head.chars().take(TITLE_MAX_CHARS).collect()
}

/// The index of `owner`'s task a word names: by title (any case), by `#N`, or the latest when nothing is named.
fn find(tasks: &[Task], owner: &str, name: &str) -> Option<usize> {
    let name = name.trim();
    let mut mine = tasks.iter().enumerate().filter(|(_, t)| t.owner == owner);
    if name.is_empty() {
        return mine.max_by_key(|(i, t)| (t.last, *i)).map(|(i, _)| i);
    }
    let want = title_of(name).to_lowercase();
    mine.find(|(_, t)| t.title.to_lowercase() == want)
        .map(|(i, _)| i)
}

/// The text after a word that may carry `#N` before its colon: `checkpoint #12: x` gives (`#12`, `x`).
fn numbered<'a>(body: &'a str, prefix: &str) -> Option<(String, &'a str)> {
    let rest = body.strip_prefix(prefix)?;
    if let Some(r) = rest.strip_prefix(':') {
        return Some((String::new(), r.trim()));
    }
    let r = rest.strip_prefix(" #")?;
    let n: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
    let after = r[n.len()..]
        .strip_prefix(':')
        .or(if prefix == FINISHED_PREFIX {
            Some(&r[n.len()..])
        } else {
            None
        })?;
    (!n.is_empty()).then(|| (format!("#{n}"), after.trim()))
}

/// The open tasks in `messages`, oldest first.
pub fn open_tasks(messages: &[Message]) -> Vec<Task> {
    let mut ordered: Vec<&Message> = messages.iter().collect();
    ordered.sort_by_key(|m| m.seq);
    let mut tasks: Vec<Task> = Vec::new();
    for m in ordered {
        let (who, body) = (m.sender.as_str(), m.body.as_str());
        if let Some(what) = body.strip_prefix(WORKING_PREFIX) {
            let what = what.trim();
            let title = title_of(what);
            if title.is_empty() {
                continue;
            }
            match find(&tasks, who, &title) {
                Some(i) => {
                    tasks[i].what = what.to_string();
                    tasks[i].last = m.ts;
                }
                None => tasks.push(Task {
                    owner: who.into(),
                    title,
                    what: what.to_string(),
                    started: m.ts,
                    last: m.ts,
                    checkpoint: None,
                    machine: None,
                    undeclared: false,
                    from: None,
                    orphaned: false,
                }),
            }
        } else if let Some((name, state)) = numbered(body, CHECKPOINT_PREFIX) {
            if let Some(i) = find(&tasks, who, &name) {
                tasks[i].checkpoint = Some(Checkpoint {
                    text: state.to_string(),
                    ts: m.ts,
                });
                tasks[i].last = m.ts;
            }
        } else if let Some(rest) = body.strip_prefix(HANDOFF_PREFIX) {
            let Some((name, to)) = rest.rsplit_once(" to ") else {
                continue;
            };
            let to = to.trim();
            let Some(i) = find(&tasks, who, name).filter(|_| valid_name(to) && to != who) else {
                continue;
            };
            let mut t = tasks.remove(i);
            tasks.retain(|o| !(o.owner == to && o.title == t.title));
            t.from = Some(t.owner.clone());
            t.owner = to.to_string();
            t.last = m.ts;
            tasks.push(t);
        } else if let Some((name, rest)) = numbered(body, FINISHED_PREFIX) {
            let name = if name.is_empty() { rest } else { &name };
            if let Some(i) = find(&tasks, who, name) {
                tasks.remove(i);
            }
        }
    }
    tasks.sort_by_key(|t| t.started);
    tasks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Attribution;

    fn msg(seq: i64, sender: &str, body: &str, ts: i64) -> Message {
        Message {
            id: format!("id{seq}"),
            seq,
            sender: sender.into(),
            attribution: Attribution::Unattributed,
            label: "public".into(),
            body: body.into(),
            ts,
            kind: None,
            payload: None,
            advisory: false,
            scope: None,
            refs: Vec::new(),
        }
    }

    fn tasks(log: &[(&str, &str)]) -> Vec<Task> {
        let messages: Vec<Message> = log
            .iter()
            .enumerate()
            .map(|(i, (who, body))| msg(i as i64 + 1, who, body, 100 * (i as i64 + 1)))
            .collect();
        open_tasks(&messages)
    }

    #[test]
    fn working_starts_a_task_a_checkpoint_records_state_and_finished_ends_it() {
        let t = tasks(&[
            ("alice", "working: nightly build: watching the pipeline"),
            ("alice", "checkpoint: build 41 queued, resume at step 3"),
            ("alice", "checkpoint: build 41 green, resume at step 4"),
        ]);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].owner, "alice");
        assert_eq!(t[0].title, "nightly build");
        assert_eq!(t[0].started, 100);
        assert_eq!(t[0].last, 300);
        let c = t[0].checkpoint.as_ref().unwrap();
        assert_eq!(
            (c.text.as_str(), c.ts),
            ("build 41 green, resume at step 4", 300)
        );
        let done = tasks(&[
            ("alice", "working: nightly build"),
            ("alice", "finished: nightly build"),
        ]);
        assert!(done.is_empty());
    }

    #[test]
    fn renewing_keeps_the_checkpoint_and_a_task_is_keyed_by_owner_and_title() {
        let t = tasks(&[
            ("alice", "working: migration"),
            ("alice", "checkpoint: half done"),
            ("alice", "working: migration"),
            ("bob", "working: migration"),
        ]);
        assert_eq!(t.len(), 2, "same title, different owner");
        let a = t.iter().find(|t| t.owner == "alice").unwrap();
        assert_eq!(a.checkpoint.as_ref().unwrap().text, "half done");
        assert_eq!(a.last, 300, "the renewal moved the latest word");
        assert_eq!(a.started, 100);
    }

    #[test]
    fn a_checkpoint_goes_to_the_latest_task_or_to_the_one_named_by_ask_number() {
        let t = tasks(&[
            ("alice", "working: #12 the review"),
            ("alice", "working: docs sweep"),
            ("alice", "checkpoint: chapter 2"),
            ("alice", "checkpoint #12: comments 1 to 7 done"),
        ]);
        let by = |title: &str| t.iter().find(|t| t.title == title).unwrap();
        assert_eq!(
            by("docs sweep").checkpoint.as_ref().unwrap().text,
            "chapter 2"
        );
        assert_eq!(
            by("#12").checkpoint.as_ref().unwrap().text,
            "comments 1 to 7 done"
        );
        assert!(tasks(&[("alice", "checkpoint: nothing to attach to")]).is_empty());
    }

    #[test]
    fn finished_names_a_title_a_number_or_nothing_for_the_latest_and_only_the_owner_ends_it() {
        let t = tasks(&[
            ("alice", "working: one"),
            ("alice", "working: #7 two"),
            ("alice", "working: three"),
            ("bob", "finished: one"),
            ("alice", "finished: ONE"),
            ("alice", "finished #7"),
        ]);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].title, "three");
        assert!(
            tasks(&[("alice", "working: x"), ("alice", "finished:")]).is_empty(),
            "an empty name ends the latest"
        );
    }

    #[test]
    fn a_handoff_moves_the_task_with_its_checkpoint_to_the_receiver() {
        let t = tasks(&[
            ("alice", "working: nightly build"),
            ("alice", "checkpoint: resume at step 4"),
            ("alice", "handoff: nightly build to bob"),
        ]);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].owner, "bob");
        assert_eq!(t[0].from.as_deref(), Some("alice"));
        assert_eq!(t[0].checkpoint.as_ref().unwrap().text, "resume at step 4");
        assert_eq!(t[0].last, 300);
        let t = tasks(&[
            ("alice", "working: x"),
            ("alice", "handoff: x to not a name!"),
            ("bob", "handoff: x to carol"),
        ]);
        assert_eq!(
            t[0].owner, "alice",
            "a bad name or a stranger moves nothing"
        );
        let t = tasks(&[
            ("alice", "working: x"),
            ("alice", "handoff: x to bob"),
            ("bob", "finished: x"),
        ]);
        assert!(t.is_empty(), "the receiver can finish it");
    }

    #[test]
    fn a_task_line_shows_its_state_and_age_and_marks_an_orphan() {
        let mut t = tasks(&[
            ("alice", "working: nightly build"),
            ("alice", "checkpoint: resume at step 4"),
        ])
        .remove(0);
        let line = t.line(200 + 7200, false);
        assert!(
            line.contains("alice")
                && line.contains("nightly build")
                && line.contains("checkpoint 2h ago: resume at step 4"),
            "{line}"
        );
        assert!(!line.contains("orphaned"));
        t.orphaned = true;
        let line = t.line(200 + 3 * 86_400, false);
        assert!(
            line.contains("orphaned") && line.contains("last word 3d ago"),
            "{line}"
        );
        t.checkpoint.as_mut().unwrap().text = "x".repeat(500);
        assert!(t.line(300, false).chars().count() < 300);
        assert!(t.line(300, true).contains(&"x".repeat(500)));
        t.checkpoint.as_mut().unwrap().text = "a\nb".into();
        assert!(
            !t.line(300, true).contains('\n'),
            "a checkpoint stays on one line"
        );
    }
}
