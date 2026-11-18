//! #181: `who` and the board show where each member was observed: the repository id and the short name of its
//! directory, never the directory path. They are observations by the host, for display and grouping; nothing reads
//! them as authority, and showing them is under the same guard as the reads that carry them.

use super::*;
use crate::domain::MemberAttrs;

fn lobby() -> Gateway<SpyStore> {
    let g = open();
    g.join(&alice(), "lobby").unwrap();
    g.join(&bob(), "lobby").unwrap();
    g.store
        .note_member_attrs(
            "alice",
            &MemberAttrs::new(
                "m-a",
                "git:proj-1",
                "/Users/secret-person/clients/acme/proj",
            ),
            1,
        )
        .unwrap();
    g
}

#[test]
fn who_shows_the_repository_and_only_the_short_directory_name() {
    let g = lobby();
    let ms = g.who(&bob(), "lobby").unwrap();
    let a = ms.iter().find(|m| m.name == "alice").unwrap();
    let place = a.place.as_ref().expect("alice has a place");
    assert_eq!(place.repository.as_deref(), Some("git:proj-1"));
    assert_eq!(place.directory.as_deref(), Some("proj"));
    let shown = crate::domain::render_who(&ms, g.now(), true);
    assert!(shown.contains("repo: git:proj-1"), "{shown}");
    assert!(shown.contains("dir: proj"), "{shown}");
    assert!(!shown.contains("secret-person"), "{shown}");
    assert!(!shown.contains("/Users"), "{shown}");
}

#[test]
fn a_member_with_no_observed_attributes_shows_none() {
    let g = lobby();
    let ms = g.who(&alice(), "lobby").unwrap();
    assert!(ms.iter().find(|m| m.name == "bob").unwrap().place.is_none());
}

#[test]
fn a_member_outside_a_repository_shows_a_directory_but_no_repository() {
    let g = lobby();
    g.store
        .note_member_attrs("bob", &MemberAttrs::new("m-b", "", "/tmp/scratch/"), 2)
        .unwrap();
    let ms = g.who(&alice(), "lobby").unwrap();
    let place = ms
        .iter()
        .find(|m| m.name == "bob")
        .and_then(|m| m.place.clone())
        .unwrap();
    assert_eq!(place.repository, None);
    assert_eq!(place.directory.as_deref(), Some("scratch"));
}

#[test]
fn the_board_shows_the_same_place_and_not_the_path() {
    let g = lobby();
    let board = g.board(&bob(), "lobby").unwrap();
    let a = board.members.iter().find(|m| m.name == "alice").unwrap();
    assert_eq!(
        a.place.as_ref().and_then(|p| p.repository.as_deref()),
        Some("git:proj-1")
    );
    let text = crate::domain::render_board(&board, g.now());
    assert!(text.contains("repo: git:proj-1"), "{text}");
    assert!(!text.contains("secret-person"), "{text}");
}

#[test]
fn a_reader_below_the_room_label_learns_nothing_of_the_places() {
    // who and the board are guarded like the other reads: a caller the room's label refuses sees no member at all
    let g = open();
    g.admin_set_room_label(&admin(), "vault", "private")
        .unwrap();
    g.admin_set_clearance(&admin(), "alice", "private").unwrap();
    g.join(&alice(), "vault").unwrap();
    g.store
        .note_member_attrs(
            "alice",
            &MemberAttrs::new("m-a", "git:proj-1", "/w/proj"),
            1,
        )
        .unwrap();
    assert!(g.who(&alice(), "vault").is_ok());
    assert!(g.who(&bob(), "vault").is_err());
    assert!(g.board(&bob(), "vault").is_err());
}

#[test]
fn a_shared_repository_value_changes_no_outcome() {
    // two members share a repository, a third does not: who lists all three the same way and the repository grants
    // nothing (the clearance test in labels.rs pins the access side)
    let g = lobby();
    g.store
        .note_member_attrs("bob", &MemberAttrs::new("m-b", "git:proj-1", "/w/other"), 2)
        .unwrap();
    let ms = g.who(&alice(), "lobby").unwrap();
    assert_eq!(ms.len(), 2);
    assert!(g.read(&bob(), "lobby", true).is_ok());
}
