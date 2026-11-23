//! The reference rules-based router (#189, DESIGN section 24.1 decision 7): a deterministic member, no model, that
//! turns open asks, live claims and presence into invites to a topical room.
//!
//! This file is the whole decision, as a pure function of structured facts. It cannot read what a message says: an ask
//! reaches it as `AskFacts`, which has a number, a sender, whether a helper holds it and who declined it as `cannot`;
//! there is no text field to read, so no wording of an ask can steer it (messages are data, DESIGN 24.1 decision 8).
//! The only text it reads is a claim path, which klatch compares by path component everywhere else too.
//!
//! The rule: the topic of an ask is the asker's own first live claim (by path order), other than the bookkeeping
//! claims `ask:` and `pr:`. The members to invite are the present members who hold a live claim that overlaps the topic,
//! or that an operator rule (`prefix=member`) names for it; never the asker, the router, a member that declined that ask
//! as `cannot`, one that is done or parked or has not been seen lately. An ask with no topic, or no such member, is
//! left alone: nothing is guessed from words.

use crate::domain::{
    Claim, MEMBER_ACTIVE_SECS, MemberInfo, OpenAsk, OutcomeKind, norm, overlaps, valid_name,
};

/// The most members one ask brings into its room.
pub const MAX_INVITES: usize = 3;

/// An operator rule: a member to invite for any topic that overlaps `prefix` (a claim path), from `--rule prefix=member`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub prefix: String,
    pub member: String,
}

impl Rule {
    /// `prefix=member`; the member must be a valid name and the prefix not empty.
    pub fn parse(text: &str) -> Result<Rule, String> {
        let (prefix, member) = text
            .split_once('=')
            .ok_or_else(|| format!("a rule is prefix=member, not {text}"))?;
        if prefix.is_empty() || !valid_name(member) {
            return Err(format!("a rule is prefix=member, not {text}"));
        }
        Ok(Rule {
            prefix: norm(prefix),
            member: member.to_string(),
        })
    }
}

/// What the router may know about an ask: no text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskFacts {
    pub seq: i64,
    pub sender: String,
    /// A helper holds a live `ask:<seq>` claim.
    pub claimed: bool,
    /// Helpers that declined it as `cannot`.
    pub cannot: Vec<String>,
}

impl AskFacts {
    pub fn of(ask: &OpenAsk) -> AskFacts {
        AskFacts {
            seq: ask.seq,
            sender: ask.sender.clone(),
            claimed: ask.claimed,
            cannot: ask
                .outcomes
                .iter()
                .filter(|o| o.kind == OutcomeKind::Cannot)
                .map(|o| o.by.clone())
                .collect(),
        }
    }
}

/// Everything one pass decides from.
pub struct View<'a> {
    pub me: &'a str,
    pub now: i64,
    pub asks: &'a [AskFacts],
    pub claims: &'a [Claim],
    pub members: &'a [MemberInfo],
    pub rules: &'a [Rule],
}

/// One planned room: who to invite into which room for which ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub ask: i64,
    pub topic: String,
    pub room: String,
    pub invites: Vec<String>,
}

/// The room for a topic: `topic-` and the topic with every run of other characters as one dash, at most 40 bytes;
/// `None` when nothing is left.
pub fn topic_room(topic: &str) -> Option<String> {
    let mut slug = String::new();
    for c in topic.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    let room: String = format!("topic-{slug}").chars().take(40).collect();
    let room = room.trim_end_matches('-').to_string();
    (!slug.is_empty() && valid_name(&room)).then_some(room)
}

/// The routes this view calls for, oldest ask first; deterministic.
pub fn plan(view: &View) -> Vec<Route> {
    let mut asks: Vec<&AskFacts> = view
        .asks
        .iter()
        .filter(|a| !a.claimed && a.sender != view.me)
        .collect();
    asks.sort_by_key(|a| a.seq);
    asks.into_iter()
        .filter_map(|a| route_for(view, a))
        .collect()
}

/// A claim that has not expired.
fn is_live(view: &View, c: &Claim) -> bool {
    c.expires > view.now
}

/// The route for one ask, when its asker holds a topic claim and someone relevant is present to invite.
fn route_for(view: &View, a: &AskFacts) -> Option<Route> {
    let topic = view
        .claims
        .iter()
        .filter(|c| is_live(view, c))
        .filter(|c| c.owner == a.sender)
        .map(|c| c.path.as_str())
        .filter(|p| !p.starts_with("ask:") && !p.starts_with("pr:"))
        .min()?;
    let room = topic_room(topic)?;
    let invites = invites_for(view, a, topic);
    (!invites.is_empty()).then(|| Route {
        ask: a.seq,
        topic: topic.to_string(),
        room,
        invites,
    })
}

/// Whether member `n` holds a live claim or a rule that overlaps `topic`.
fn works_on(view: &View, n: &str, topic: &str) -> bool {
    view.claims
        .iter()
        .filter(|c| is_live(view, c))
        .any(|c| c.owner == n && overlaps(&c.path, topic))
        || view
            .rules
            .iter()
            .any(|r| r.member == n && overlaps(&r.prefix, topic))
}

/// The members to invite for `topic`: present, not the asker or the router, not declined as cannot, and working on the
/// topic; at most `MAX_INVITES`, sorted.
fn invites_for(view: &View, a: &AskFacts, topic: &str) -> Vec<String> {
    let mut invites: Vec<String> = view
        .members
        .iter()
        .filter(|m| present(m, view.now))
        .map(|m| m.name.as_str())
        .filter(|n| *n != a.sender && *n != view.me && !a.cannot.iter().any(|c| c == n))
        .filter(|n| works_on(view, n, topic))
        .map(String::from)
        .collect();
    invites.sort();
    invites.dedup();
    // when more qualify than fit, members of the asker's repository come first (#890). The repository is an
    // observation used only to order candidates the claims and rules already chose; it adds nobody.
    let asker_repo = repository_of(view.members, &a.sender);
    invites.sort_by_key(|n| {
        let same = asker_repo.is_some() && repository_of(view.members, n) == asker_repo;
        !same
    });
    invites.truncate(MAX_INVITES);
    invites.sort();
    invites
}

/// The repository id observed for a member of the view, if any.
fn repository_of<'a>(members: &'a [MemberInfo], name: &str) -> Option<&'a str> {
    members
        .iter()
        .find(|m| m.name == name)
        .and_then(|m| m.place.as_ref())
        .and_then(|p| p.repository.as_deref())
}

/// A member worth inviting now: seen lately, not done, not parked.
pub(super) fn present(m: &MemberInfo, now: i64) -> bool {
    now - m.last_seen <= MEMBER_ACTIVE_SECS
        && !m.status.as_deref().is_some_and(|s| s.starts_with("done"))
        && !m.parked.as_ref().is_some_and(|p| p.holds(now))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 10_000;

    fn claim(owner: &str, path: &str) -> Claim {
        Claim {
            path: path.into(),
            owner: owner.into(),
            reason: String::new(),
            expires: NOW + 300,
            scope: String::new(),
        }
    }

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

    fn in_repo(mut m: MemberInfo, repo: &str) -> MemberInfo {
        m.place = Some(crate::domain::MemberPlace {
            repository: Some(repo.into()),
            directory: None,
            scope_differs: false,
        });
        m
    }

    fn ask(seq: i64, sender: &str) -> AskFacts {
        AskFacts {
            seq,
            sender: sender.into(),
            claimed: false,
            cannot: vec![],
        }
    }

    fn run(
        asks: &[AskFacts],
        claims: &[Claim],
        members: &[MemberInfo],
        rules: &[Rule],
    ) -> Vec<Route> {
        plan(&View {
            me: "router",
            now: NOW,
            asks,
            claims,
            members,
            rules,
        })
    }

    fn lobby() -> Vec<MemberInfo> {
        ["router", "ann", "bo", "cy", "di"].map(member).to_vec()
    }

    #[test]
    fn a_member_holding_a_claim_on_the_topic_is_invited_to_the_topic_room() {
        let claims = [
            claim("ann", "src/auth"),
            claim("bo", "src/auth/login.rs"),
            claim("cy", "docs"),
        ];
        let r = run(&[ask(1, "ann")], &claims, &lobby(), &[]);
        assert_eq!(
            r,
            vec![Route {
                ask: 1,
                topic: "src/auth".into(),
                room: "topic-src-auth".into(),
                invites: vec!["bo".into()],
            }]
        );
    }

    #[test]
    fn an_operator_rule_names_a_member_for_a_topic_it_holds_no_claim_on() {
        let claims = [claim("ann", "src/auth")];
        let rules = [Rule::parse("src=di").unwrap()];
        let r = run(&[ask(1, "ann")], &claims, &lobby(), &rules);
        assert_eq!(r[0].invites, vec!["di".to_string()]);
    }

    #[test]
    fn the_asker_the_router_and_members_that_declined_as_cannot_are_never_invited() {
        let claims = [
            claim("ann", "x"),
            claim("router", "x"),
            claim("bo", "x"),
            claim("cy", "x"),
        ];
        let mut a = ask(1, "ann");
        a.cannot = vec!["bo".into()];
        let r = run(&[a], &claims, &lobby(), &[]);
        assert_eq!(r[0].invites, vec!["cy".to_string()]);
    }

    #[test]
    fn members_that_are_done_parked_gone_or_not_in_the_lobby_are_not_invited() {
        let mut ms = lobby();
        ms[2].status = Some("done".into());
        ms[3].parked = Some(crate::domain::Parked {
            why: String::new(),
            back_by: NOW + 3600,
        });
        ms[4].last_seen = NOW - MEMBER_ACTIVE_SECS - 1;
        let claims = [
            claim("ann", "x"),
            claim("bo", "x"),
            claim("cy", "x"),
            claim("di", "x"),
            claim("eve", "x"),
        ];
        assert!(run(&[ask(1, "ann")], &claims, &ms, &[]).is_empty());
    }

    #[test]
    fn an_expired_claim_is_no_evidence() {
        let mut c = claim("bo", "x");
        c.expires = NOW;
        let claims = [claim("ann", "x"), c];
        assert!(run(&[ask(1, "ann")], &claims, &lobby(), &[]).is_empty());
    }

    #[test]
    fn an_ask_with_no_topic_or_that_a_helper_holds_is_left_alone() {
        let claims = [
            claim("bo", "x"),
            claim("ann", "ask:7"),
            claim("ann", "pr:3"),
        ];
        assert!(run(&[ask(1, "ann")], &claims, &lobby(), &[]).is_empty());
        let claims = [claim("ann", "x"), claim("bo", "x")];
        let mut held = ask(1, "ann");
        held.claimed = true;
        assert!(run(&[held], &claims, &lobby(), &[]).is_empty());
    }

    #[test]
    fn an_asker_with_no_claim_is_not_routed_even_with_a_rule_and_busy_members() {
        // the gap of #892: with no claim there is no topic, and an operator rule cannot supply one
        let claims = [claim("bo", "x"), claim("cy", "x/y")];
        let rules = [Rule::parse("x=bo").unwrap()];
        assert!(run(&[ask(1, "ann")], &claims, &lobby(), &rules).is_empty());
    }

    #[test]
    fn an_ask_of_the_router_itself_is_not_routed() {
        let claims = [claim("router", "x"), claim("bo", "x")];
        assert!(run(&[ask(1, "router")], &claims, &lobby(), &[]).is_empty());
    }

    #[test]
    fn at_most_three_members_come_in_and_they_come_in_name_order() {
        let ms: Vec<MemberInfo> = ["router", "ann", "m5", "m4", "m3", "m2", "m1"]
            .map(member)
            .to_vec();
        let claims: Vec<Claim> = ["ann", "m5", "m4", "m3", "m2", "m1"]
            .iter()
            .map(|o| claim(o, "x"))
            .collect();
        let r = run(&[ask(1, "ann")], &claims, &ms, &[]);
        assert_eq!(r[0].invites, vec!["m1", "m2", "m3"]);
    }

    #[test]
    fn asks_on_one_topic_share_one_room_and_the_plan_does_not_depend_on_input_order() {
        let claims = [claim("ann", "x"), claim("bo", "x"), claim("cy", "x")];
        let one = run(&[ask(1, "ann"), ask(2, "cy")], &claims, &lobby(), &[]);
        let two = run(&[ask(2, "cy"), ask(1, "ann")], &claims, &lobby(), &[]);
        assert_eq!(one, two);
        assert_eq!(one.len(), 2);
        assert_eq!(one[0].room, one[1].room);
        assert_eq!(one[0].ask, 1);
    }

    #[test]
    fn the_topic_is_the_first_claim_by_path_order_whatever_the_table_order() {
        let a = [
            claim("ann", "zeta"),
            claim("ann", "alpha"),
            claim("bo", "alpha"),
        ];
        let b = [
            claim("ann", "alpha"),
            claim("bo", "alpha"),
            claim("ann", "zeta"),
        ];
        assert_eq!(
            run(&[ask(1, "ann")], &a, &lobby(), &[]),
            run(&[ask(1, "ann")], &b, &lobby(), &[])
        );
        assert_eq!(run(&[ask(1, "ann")], &a, &lobby(), &[])[0].topic, "alpha");
    }

    #[test]
    fn a_topic_that_makes_no_room_name_routes_nothing() {
        assert_eq!(topic_room("///"), None);
        assert_eq!(
            topic_room("src/Auth Login.rs").as_deref(),
            Some("topic-src-auth-login-rs")
        );
        assert!(topic_room(&"a".repeat(200)).is_some_and(|r| r.len() <= 40));
    }

    #[test]
    fn rules_parse_strictly() {
        assert!(Rule::parse("src").is_err());
        assert!(Rule::parse("=bo").is_err());
        assert!(Rule::parse("src=bad name").is_err());
        assert_eq!(Rule::parse("src/=bo").unwrap().prefix, "src");
    }

    #[test]
    fn when_more_qualify_than_fit_the_askers_repository_comes_first_and_the_set_of_candidates_is_unchanged()
     {
        // everyone holds the topic; the asker and bo, cy, di are in repo r1 or r2; four candidates, three slots
        let ms = vec![
            member("router"),
            in_repo(member("ann"), "git:r1"),
            in_repo(member("bo"), "git:r2"),
            in_repo(member("cy"), "git:r1"),
            in_repo(member("di"), "git:r2"),
            in_repo(member("ed"), "git:r1"),
        ];
        let claims = [
            claim("ann", "x"),
            claim("bo", "x"),
            claim("cy", "x"),
            claim("di", "x"),
            claim("ed", "x"),
        ];
        let r = run(&[ask(1, "ann")], &claims, &ms, &[]);
        // cy and ed share ann's repository so they come first; bo fills the last slot
        assert_eq!(r[0].invites, vec!["bo", "cy", "ed"]);
    }

    #[test]
    fn a_member_in_the_askers_repository_with_no_claim_on_the_topic_is_not_invited() {
        // the repository attribute orders; it never adds a member the claims and rules would not
        let ms = vec![
            member("router"),
            in_repo(member("ann"), "git:r1"),
            in_repo(member("bo"), "git:r1"),
            in_repo(member("cy"), "git:r2"),
        ];
        let claims = [claim("ann", "x"), claim("cy", "x")];
        let r = run(&[ask(1, "ann")], &claims, &ms, &[]);
        assert_eq!(r[0].invites, vec!["cy"]);
    }

    #[test]
    fn with_no_attributes_known_the_order_is_by_name_as_before() {
        let claims = [
            claim("ann", "x"),
            claim("di", "x"),
            claim("bo", "x"),
            claim("cy", "x"),
        ];
        let r = run(&[ask(1, "ann")], &claims, &lobby(), &[]);
        assert_eq!(r[0].invites, vec!["bo", "cy", "di"]);
    }

    #[test]
    fn members_with_no_repository_are_never_treated_as_sharing_one() {
        let mut ann = in_repo(member("ann"), "");
        ann.place.as_mut().unwrap().repository = None;
        let mut ms = vec![member("router"), ann];
        for n in ["bo", "cy", "di", "ed"] {
            ms.push(member(n));
        }
        let claims = [
            claim("ann", "x"),
            claim("bo", "x"),
            claim("cy", "x"),
            claim("di", "x"),
            claim("ed", "x"),
        ];
        let r = run(&[ask(1, "ann")], &claims, &ms, &[]);
        assert_eq!(r[0].invites, vec!["bo", "cy", "di"]);
    }
}
