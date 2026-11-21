//! The scope id hash, the repository root of a git directory, and the same-scope no-op (#725).

use super::*;
use crate::store::SqliteStore;
use std::path::Path;

#[test]
fn the_hash_is_fnv1a_64_so_a_scope_id_is_the_same_on_every_platform() {
    // the published test vectors of 64-bit FNV-1a
    assert_eq!(fnv1a(""), 0xcbf29ce484222325);
    assert_eq!(fnv1a("a"), 0xaf63dc4c8601ec8c);
    assert_eq!(fnv1a("foobar"), 0x85944171f73967e8);
}

#[test]
fn different_directories_hash_apart_and_order_matters() {
    assert_ne!(fnv1a("/w/repo-a/.git"), fnv1a("/w/repo-b/.git"));
    assert_ne!(fnv1a("ab"), fnv1a("ba"));
    assert_ne!(fnv1a("a"), fnv1a("b"));
}

#[test]
fn the_root_of_a_dot_git_directory_is_its_parent_and_of_a_bare_one_itself() {
    assert_eq!(
        repository_root(Path::new("/w/repo/.git")),
        Path::new("/w/repo")
    );
    assert_eq!(
        repository_root(Path::new("/w/repo.git")),
        Path::new("/w/repo.git"),
        "a bare repository is the directory itself, though it has a parent"
    );
    assert_eq!(
        repository_root(Path::new("/w/x/gitdir")),
        Path::new("/w/x/gitdir")
    );
    assert_eq!(repository_root(Path::new("/")), Path::new("/"));
}

fn store(tag: &str) -> crate::store::test_dir::In<SqliteStore> {
    crate::store::test_dir::In::new(&format!("scope-edge-{tag}"), |d| {
        SqliteStore::open(&d.join("t.db"), &crate::store::insecure_test_key()).unwrap()
    })
}

#[test]
fn declaring_the_scope_a_member_already_has_is_a_no_op_even_when_it_would_not_pass_today() {
    let s = store("same");
    // a scope stored under an older, looser rule
    s.set_member_scope("alice", "has space", 5).unwrap();
    ensure_member_scope(&*s, "alice", (Some("has space"), None), 9).unwrap();
    assert_eq!(
        s.member_scope("alice").unwrap().as_deref(),
        Some("has space")
    );
    // a different declared scope is still checked and replaces it
    assert!(ensure_member_scope(&*s, "alice", (Some("also bad"), None), 9).is_err());
    ensure_member_scope(&*s, "alice", (Some("good"), None), 9).unwrap();
    assert_eq!(s.member_scope("alice").unwrap().as_deref(), Some("good"));
}
