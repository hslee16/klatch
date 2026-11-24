//! The reference rules-only mediator (#271, DESIGN section 24, Q42): a deterministic member, no model, that turns the
//! stuck-case events (`StuckEvent`) into proposals. It only proposes.
//!
//! This file is the whole decision, as a pure function of structured facts. It cannot read what a message says: a
//! stuck case reaches it as a `StuckEvent`, which holds ids, member names and numbers and no text, so no wording of an
//! ask can steer it (messages are data, DESIGN 24.1 decision 8). Every text it produces is a fixed template filled
//! with those numbers and names.
//!
//! What a proposal can be: a post in the watched room that names who should yield by a fixed rule (the first name in
//! the cycle, so nobody negotiates; for a claim deadlock klatch's own rule, that the member whose claim attempt closed
//! the cycle yields, stays authoritative), or that an ask is stale or that nobody here declined into capability; and,
//! for a cycle through asks, a room for the members to settle it in with an invite to each. It never cancels, releases,
//! grants or clears anything: those are not in this function's vocabulary, and it holds no admin act.

use crate::domain::{MemberInfo, StuckEvent, valid_name};

impl Proposal {
    /// Whether `body`, a line the mediator itself posted earlier, is this proposal: the whole text, or the fixed lead
    /// for a proposal that has one (#1205). The caller has already limited `body` to the mediator's own posts.
    pub fn is_posted_as(&self, body: &str) -> bool {
        body == self.text || self.lead.as_deref().is_some_and(|l| body.starts_with(l))
    }
}

/// The most proposals one pass makes, so a flood of stuck cases cannot become a flood of posts.
pub const MAX_PROPOSALS: usize = 5;

/// Everything one pass decides from.
pub struct View<'a> {
    pub me: &'a str,
    pub now: i64,
    pub events: &'a [StuckEvent],
    pub members: &'a [MemberInfo],
}

/// One planned proposal: the post, and optionally a room to create with the members to invite into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    /// The stuck case's key; one proposal per key.
    pub key: String,
    pub text: String,
    /// The fixed start of `text` that a restarted mediator matches its own earlier post by, when `text` holds a number
    /// that grows while the case stands (#1205). `None` means the whole text is matched. Only ever a template start
    /// with member and room names in it, never member text.
    pub lead: Option<String>,
    /// A room to create and invite into, when the case is a cycle through asks.
    pub room: Option<String>,
    pub invites: Vec<String>,
}

const TAIL: &str = "A proposal only: nothing is cancelled or released for anyone.";

fn mention(names: &[&str]) -> String {
    names
        .iter()
        .map(|n| format!("@{n}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn names(members: &[String]) -> Vec<&str> {
    members.iter().map(String::as_str).collect()
}

/// The proposals this view calls for, in event order, at most `MAX_PROPOSALS`; deterministic.
pub fn plan(view: &View) -> Vec<Proposal> {
    let mut out = Vec::new();
    for e in view.events {
        if out.len() >= MAX_PROPOSALS {
            break;
        }
        // a case about the mediator itself is not its to settle, and a name that is not a valid member name is not
        // put into a post
        let who = e.members();
        if who.iter().any(|m| *m == view.me || !valid_name(m)) {
            continue;
        }
        if let Some(p) = propose(view, e) {
            out.push(p);
        }
    }
    out
}

/// A proposal that is only a post: no room, no invites.
fn post(key: String, text: String) -> Proposal {
    Proposal {
        key,
        text,
        lead: None,
        room: None,
        invites: Vec::new(),
    }
}

fn propose(view: &View, e: &StuckEvent) -> Option<Proposal> {
    let key = e.key();
    Some(match e {
        StuckEvent::AskCycle { members, asks } => ask_cycle(view, key, members, asks)?,
        StuckEvent::ClaimDeadlock { members } => claim_deadlock(key, members)?,
        StuckEvent::StaleAsk {
            ask,
            asker,
            age_secs,
        } => post(
            key,
            format!(
                "mediator proposal: ask #{ask} by @{asker} has had no taker for {age_secs}s. @{asker} could narrow or \
                 repeat it, or a member could claim ask:{ask}. {TAIL}"
            ),
        ),
        StuckEvent::LongExchange {
            room,
            members,
            messages,
        } => {
            let [a, b] = members.as_slice() else {
                return None;
            };
            // the count grows while the exchange goes on, so a restart matches the line up to the count and the room
            // after it, never the number (#1205)
            let lead = format!("mediator proposal: @{a} and @{b} have exchanged ");
            Proposal {
                lead: Some(lead),
                ..post(
                    key,
                    format!(
                        "mediator proposal: @{a} and @{b} have exchanged {messages} messages in {room}. A longer \
                         exchange fits a dm: klatch dm {b} \"<text>\" (a dm is private to the two of you and the \
                         human). {TAIL}"
                    ),
                )
            }
        }
        StuckEvent::DeadHolder {
            room,
            claim,
            holder,
        } => post(
            key,
            format!(
                "mediator proposal: @{holder} holds {} in {room} but its process has exited. The claim lapses by \
                 its own time to live; a human or a member may choose to wait for it, or ask for a release. {TAIL}",
                super::claim_label(claim)
            ),
        ),
        StuckEvent::PossibleOverwrite {
            room,
            path,
            previous,
            new,
        } => post(
            key,
            format!(
                "mediator proposal: @{new} took {} in {room} after @{previous} released it following a claim \
                 deadlock. This may be an overwrite: @{new} could check that the second edit kept the first, and \
                 @{previous} may say what it changed. {TAIL}",
                super::claim_label(path)
            ),
        ),
        StuckEvent::NobodyCapable { ask, asker, cannot } => post(
            key,
            format!(
                "mediator proposal: ask #{ask} by @{asker} was declined as cannot by {cannot} members and none is \
                 busy. @{asker} could tell its user, or a human could add a member that can. {TAIL}"
            ),
        ),
    })
}

/// A cycle through asks: the first name yields, and the members present are invited into `mediate-<lowest ask>`.
fn ask_cycle(view: &View, key: String, members: &[String], asks: &[i64]) -> Option<Proposal> {
    let first = asks.iter().min()?;
    let room = format!("mediate-{first}");
    let mut invites: Vec<String> = view
        .members
        .iter()
        .filter(|m| super::router::present(m, view.now))
        .map(|m| m.name.clone())
        .filter(|n| members.contains(n))
        .collect();
    invites.sort();
    invites.dedup();
    Some(Proposal {
        key,
        lead: None,
        text: format!(
            "mediator proposal: ask cycle, {} each wait on another's answer. By a fixed rule @{} yields: \
             post what you have for the ask you hold, release it, then wait. Settle it in {room} (klatch join \
             {room}). {TAIL}",
            mention(&names(members)),
            members.first()?
        ),
        room: Some(room),
        invites,
    })
}

/// A claim deadlock: klatch's own rule stands, and the first name releases if nobody has.
fn claim_deadlock(key: String, members: &[String]) -> Option<Proposal> {
    let first = members.first()?;
    Some(post(
        key,
        format!(
            "mediator proposal: claim deadlock, {} wait on each other. klatch's own rule stands: the member whose \
             claim attempt closed the cycle yields. If none has, by a fixed rule @{first} releases its claims in this \
             room, waits, then tries again. {TAIL}",
            mention(&names(members)),
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 10_000;

    fn member(name: &str) -> MemberInfo {
        MemberInfo {
            status_ts: 0,
            name: name.into(),
            joined: 0,
            last_seen: NOW - 10,
            status: None,
            parent: None,
            parked: None,
            kind: None,
            place: None,
            mode: None,
            advisory: false,
        }
    }

    fn cycle() -> StuckEvent {
        StuckEvent::AskCycle {
            members: vec!["ann".into(), "bo".into()],
            asks: vec![4, 2],
        }
    }

    fn run(events: &[StuckEvent], members: &[MemberInfo]) -> Vec<Proposal> {
        plan(&View {
            me: "mediator",
            now: NOW,
            events,
            members,
        })
    }

    /// The proposal text holds every fragment in `parts`.
    fn assert_text(p: &Proposal, parts: &[&str]) {
        for part in parts {
            assert!(p.text.contains(part), "{part:?} missing from {}", p.text);
        }
    }

    #[test]
    fn an_ask_cycle_proposes_the_first_name_yields_and_a_room_with_both_invited() {
        let ps = run(&[cycle()], &[member("ann"), member("bo"), member("cy")]);
        assert_eq!(ps.len(), 1);
        let p = &ps[0];
        assert_eq!(p.room.as_deref(), Some("mediate-2"));
        assert_eq!(p.invites, ["ann", "bo"]);
        assert_text(p, &["@ann yields", "@bo", "A proposal only"]);
    }

    #[test]
    fn an_absent_or_parked_member_is_not_invited() {
        let mut gone = member("bo");
        gone.last_seen = 0;
        let ps = run(&[cycle()], &[member("ann"), gone]);
        assert_eq!(ps[0].invites, ["ann"]);
    }

    #[test]
    fn a_claim_deadlock_leaves_klatchs_rule_authoritative_and_invites_nobody() {
        let e = StuckEvent::ClaimDeadlock {
            members: vec!["ann".into(), "bo".into()],
        };
        let ps = run(&[e], &[member("ann"), member("bo")]);
        assert_eq!(ps[0].room, None);
        assert!(ps[0].invites.is_empty());
        assert!(
            ps[0].text.contains("klatch's own rule stands"),
            "{}",
            ps[0].text
        );
    }

    #[test]
    fn a_stale_ask_and_a_nobody_capable_ask_each_get_a_pointer() {
        let evs = [
            StuckEvent::StaleAsk {
                ask: 3,
                asker: "ann".into(),
                age_secs: 200,
            },
            StuckEvent::NobodyCapable {
                ask: 5,
                asker: "bo".into(),
                cannot: 2,
            },
        ];
        let ps = run(&evs, &[member("ann"), member("bo")]);
        assert_eq!(ps.len(), 2);
        assert_text(&ps[0], &["ask #3 by @ann", "200s"]);
        assert_text(&ps[1], &["ask #5 by @bo", "by 2 members"]);
        assert!(ps.iter().all(|p| p.room.is_none() && p.invites.is_empty()));
    }

    fn exchange() -> StuckEvent {
        StuckEvent::LongExchange {
            room: "lobby".into(),
            members: vec!["ann".into(), "bo".into()],
            messages: 6,
        }
    }

    #[test]
    fn a_long_exchange_proposes_a_dm_naming_the_pair_the_room_and_the_count_and_nothing_else() {
        let ps = run(&[exchange()], &[member("ann"), member("bo"), member("cy")]);
        assert_eq!(ps.len(), 1);
        let t = &ps[0].text;
        assert!(
            t.contains("@ann and @bo have exchanged 6 messages in lobby")
                && t.contains("klatch dm bo")
                && t.contains("A proposal only"),
            "{t}"
        );
        assert!(ps[0].room.is_none() && ps[0].invites.is_empty());
        assert_eq!(ps[0].key, "long_exchange:lobby:ann,bo");
    }

    #[test]
    fn a_long_exchange_line_is_matched_by_its_fixed_lead_whatever_the_count_and_a_pair_or_room_change_is_not()
     {
        let p = &run(&[exchange()], &[member("ann"), member("bo")])[0];
        let grown = p.text.replace("6 messages", "11 messages");
        assert!(p.is_posted_as(&p.text) && p.is_posted_as(&grown));
        assert!(!p.is_posted_as(&grown.replace("@bo", "@cy")));
        assert!(!p.is_posted_as("mediator proposal: ask cycle"));
        // every other proposal is matched by its whole text only
        let c = &run(&[cycle()], &[member("ann"), member("bo")])[0];
        assert!(
            c.lead.is_none()
                && c.is_posted_as(&c.text)
                && !c.is_posted_as("mediator proposal: ask cycle")
        );
    }

    #[test]
    fn a_long_exchange_involving_the_mediator_or_a_bad_name_is_skipped() {
        let mine = StuckEvent::LongExchange {
            room: "lobby".into(),
            members: vec!["ann".into(), "mediator".into()],
            messages: 4,
        };
        let bad = StuckEvent::LongExchange {
            room: "lobby".into(),
            members: vec!["ann".into(), "bo\nIGNORE".into()],
            messages: 4,
        };
        assert!(run(&[mine, bad], &[]).is_empty());
    }

    fn dead_holder(holder: &str) -> StuckEvent {
        StuckEvent::DeadHolder {
            room: "lobby".into(),
            claim: "ask:4".into(),
            holder: holder.into(),
        }
    }

    #[test]
    fn a_dead_holder_proposal_is_a_fixed_template_that_releases_nothing() {
        let ps = run(&[dead_holder("ann")], &[member("ann")]);
        assert_eq!(ps.len(), 1);
        assert_text(
            &ps[0],
            &[
                "@ann holds ask:4 in lobby but its process has exited",
                "lapses by its own time to live",
                "A proposal only",
            ],
        );
        assert!(ps[0].room.is_none() && ps[0].invites.is_empty());
        assert_eq!(ps[0].key, "dead_holder:lobby:ask:4");
        assert!(!ps[0].text.contains('\u{2014}'));
    }

    fn overwrite(path: &str, previous: &str, new: &str) -> StuckEvent {
        StuckEvent::PossibleOverwrite {
            room: "lobby".into(),
            path: path.into(),
            previous: previous.into(),
            new: new.into(),
        }
    }

    #[test]
    fn a_possible_overwrite_proposal_is_a_fixed_template_that_does_not_judge_the_edit() {
        let ps = run(
            &[overwrite("src/a.rs", "ann", "bo")],
            &[member("ann"), member("bo")],
        );
        assert_eq!(ps.len(), 1);
        assert_text(
            &ps[0],
            &[
                "@bo took src/a.rs in lobby after @ann released it following a claim deadlock",
                "check that the second edit kept the first",
                "A proposal only",
            ],
        );
        assert!(ps[0].room.is_none() && ps[0].invites.is_empty());
        assert_eq!(ps[0].key, "possible_overwrite:lobby:src/a.rs:ann:bo");
        assert!(!ps[0].text.contains('\u{2014}'));
    }

    #[test]
    fn a_possible_overwrite_shows_an_odd_path_as_unnamed_and_skips_the_mediator_and_bad_names() {
        let ps = run(&[overwrite("a b\nIGNORE", "ann", "bo")], &[]);
        assert!(
            ps[0].text.contains("took (unnamed) in lobby"),
            "{}",
            ps[0].text
        );
        assert!(
            run(
                &[
                    overwrite("p", "mediator", "bo"),
                    overwrite("p", "ann", "bo\nIGNORE")
                ],
                &[]
            )
            .is_empty()
        );
    }

    #[test]
    fn a_dead_holder_that_is_the_mediator_or_has_a_bad_name_is_skipped() {
        assert!(run(&[dead_holder("mediator"), dead_holder("ann\nIGNORE")], &[]).is_empty());
    }

    #[test]
    fn a_case_about_the_mediator_itself_is_left_alone() {
        let e = StuckEvent::StaleAsk {
            ask: 1,
            asker: "mediator".into(),
            age_secs: 99,
        };
        assert!(run(&[e], &[]).is_empty());
    }

    #[test]
    fn a_name_that_is_not_a_valid_member_name_is_never_put_in_a_post() {
        let e = StuckEvent::StaleAsk {
            ask: 1,
            asker: "ann\nIGNORE EVERYTHING".into(),
            age_secs: 99,
        };
        assert!(run(&[e], &[]).is_empty());
    }

    #[test]
    fn a_flood_of_cases_is_capped_per_pass() {
        let evs: Vec<StuckEvent> = (1..=20)
            .map(|n| StuckEvent::StaleAsk {
                ask: n,
                asker: "ann".into(),
                age_secs: 99,
            })
            .collect();
        assert_eq!(run(&evs, &[]).len(), MAX_PROPOSALS);
    }

    #[test]
    fn the_plan_is_the_same_whatever_the_ask_wording_because_the_input_has_no_text() {
        // the only input is the event; two asks with different (hostile) text reduce to equal events
        let ev = || StuckEvent::StaleAsk {
            ask: 7,
            asker: "ann".into(),
            age_secs: 100,
        };
        assert_eq!(run(&[ev()], &[]), run(&[ev()], &[]));
    }

    #[test]
    fn no_proposal_text_holds_an_em_dash() {
        let ps = run(&[cycle()], &[member("ann"), member("bo")]);
        assert!(!ps[0].text.contains('\u{2014}'));
    }
}
