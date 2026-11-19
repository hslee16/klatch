//! Scopes: where a member works (DESIGN section 27). A scope is an opaque string. A coding session gets
//! `git:<repository>` by itself; a bot, a service or a human declares its own (`bot:grok`, `project:acme`). Claims
//! conflict only inside one scope, so two repositories can each claim `src/lib.rs`.

/// The scope of a claim that conflicts with every scope: a shared claim, or a room-wide resource such as an ask.
pub const SHARED_SCOPE: &str = "*";

/// A scope name is 1 to 80 characters of letters, digits and `. _ : - /`, so it is safe in output and in a path.
pub fn valid_scope(scope: &str) -> bool {
    scope_bytes_ok(scope.as_bytes())
}

/// The rule behind `valid_scope`, over bytes (a byte fold, so the Kani proofs of #657 run it on arrays). Every accepted
/// byte is ASCII, so the byte length is the character count.
pub(crate) fn scope_bytes_ok(b: &[u8]) -> bool {
    !b.is_empty()
        && b.len() <= 80
        // Redundant today, by design (#664): `*` is not in the alphabet checked below, so that check refuses it too and no
        // test or proof can tell the two apart. It stays so that widening the alphabet to include `*` cannot let a member
        // into the shared scope.
        && b != SHARED_SCOPE.as_bytes()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b':' | b'_' | b'-' | b'/'))
}

/// The nouns of a work item name (`ticket:602`, `design-item:66`). Such a name is one thing in the whole room (#632).
pub const WORK_ITEM_NOUNS: [&str; 6] = ["ticket", "issue", "pr", "task", "item", "design-item"];

/// The canonical form of a work item name: trimmed and lower case, when it is `<noun>:<rest>` with a noun from
/// `WORK_ITEM_NOUNS` and no `/` (a path such as `src/ticket:1` is not one). `None` for anything else.
pub fn work_item_name(path: &str) -> Option<String> {
    let name = path.trim().to_lowercase();
    let (noun, rest) = name.split_once(':')?;
    (WORK_ITEM_NOUNS.contains(&noun) && !rest.is_empty() && !name.contains('/')).then_some(name)
}

/// The path as it is stored and compared: a work item name in its canonical form, anything else as typed.
pub fn canonical_claim_path(path: &str) -> String {
    work_item_name(path).unwrap_or_else(|| path.to_string())
}

/// The scope a claim is stored under: shared for an explicit `--shared`, for room-wide names (`ask:<N>`) and for work
/// item names (`ticket:<N>`, #632), otherwise the member's own scope (empty when it has none).
pub fn claim_scope(path: &str, shared: bool, member_scope: Option<&str>) -> String {
    if shared || path.starts_with("ask:") || work_item_name(path).is_some() {
        SHARED_SCOPE.to_string()
    } else {
        member_scope.unwrap_or("").to_string()
    }
}

/// Whether two claims in these scopes can conflict: the same scope, or either is shared.
pub fn scopes_collide(a: &str, b: &str) -> bool {
    a == b || a == SHARED_SCOPE || b == SHARED_SCOPE
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The old character-class form of `valid_scope`, the reference the byte form must match (#657).
    fn valid_scope_by_chars(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= 80
            && s != SHARED_SCOPE
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '_' | '-' | '/'))
    }

    #[test]
    fn the_byte_form_of_valid_scope_matches_the_character_form_on_every_short_string() {
        let pieces = [
            "a", "Z", "0", "-", "_", ".", " ", "/", ":", "*", "\n", "\u{e9}", "\u{ff41}",
        ];
        for s in crate::domain::text::short_strings(&pieces) {
            assert_eq!(valid_scope(&s), valid_scope_by_chars(&s), "{s:?}");
        }
        for n in 0..=85 {
            let s = "a".repeat(n);
            assert_eq!(valid_scope(&s), valid_scope_by_chars(&s), "{n}");
        }
    }

    /// The shared marker is refused (#664). The explicit check is redundant with the alphabet today, so this test
    /// cannot tell them apart either; it pins the result for a member that tries to claim the shared scope.
    #[test]
    fn the_shared_marker_is_never_a_valid_scope() {
        assert!(!valid_scope(SHARED_SCOPE));
        assert!(!valid_scope("*"));
        assert!(valid_scope("a*") == valid_scope_by_chars("a*"));
        assert!(!valid_scope("a*"));
    }

    #[test]
    fn a_work_item_name_is_canonical_and_shared_but_a_path_is_neither() {
        assert_eq!(
            work_item_name(" Design-Item:66 ").as_deref(),
            Some("design-item:66")
        );
        assert_eq!(work_item_name("PR:7").as_deref(), Some("pr:7"));
        for not in ["ticket", "ticket:", "src/ticket:1", "ticketing:1", "a:1", "ticket 1"] {
            assert_eq!(work_item_name(not), None, "{not}");
            assert_eq!(canonical_claim_path(not), not);
        }
        assert_eq!(claim_scope("Ticket:602", false, Some("git:a")), SHARED_SCOPE);
        assert_eq!(claim_scope("src/ticket:1", false, Some("git:a")), "git:a");
    }

    #[test]
    fn a_scope_is_a_short_safe_name() {
        for ok in [
            "git:klatch-3fa9c1d2",
            "bot:grok",
            "project:acme/web",
            "a",
            "dir:x_y.z",
        ] {
            assert!(valid_scope(ok), "{ok}");
        }
        for bad in [
            "",
            "has space",
            "new\nline",
            "tab\t",
            "quote\"",
            &"x".repeat(81),
            "semi;colon",
            "*",
        ] {
            assert!(!valid_scope(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_plain_claim_takes_the_members_scope_and_none_means_the_empty_scope() {
        assert_eq!(claim_scope("src/lib.rs", false, Some("git:a")), "git:a");
        assert_eq!(claim_scope("src/lib.rs", false, None), "");
    }

    #[test]
    fn a_shared_claim_and_a_room_wide_name_are_shared_whoever_claims() {
        assert_eq!(claim_scope("src/lib.rs", true, Some("git:a")), SHARED_SCOPE);
        assert_eq!(claim_scope("ask:12", false, Some("git:a")), SHARED_SCOPE);
        assert_eq!(claim_scope("ask:12", false, None), SHARED_SCOPE);
        assert_eq!(
            claim_scope("asked/thing", false, Some("git:a")),
            "git:a",
            "only the ask: prefix is room-wide"
        );
    }

    #[test]
    fn scopes_collide_when_equal_or_when_either_is_shared() {
        assert!(scopes_collide("git:a", "git:a"));
        assert!(scopes_collide("", ""));
        assert!(!scopes_collide("git:a", "git:b"));
        assert!(!scopes_collide("git:a", ""));
        assert!(scopes_collide(SHARED_SCOPE, "git:a"));
        assert!(scopes_collide("git:a", SHARED_SCOPE));
    }
}

/// What klatch observed about a member (DESIGN section 24, R41 and R42): a stable id that is never reused and does not
/// change with a rename, the repository id derived from the git common directory (empty when the directory is in no
/// repository), and the working directory.
///
/// These are observations by the host about where a process started, not authority: nothing may grant clearance, a
/// room or any access because two members share a repository value (a directory can name any repository's git
/// directory, so the value can be made to match). The member name, not this record, stays the key of every table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberAttrs {
    pub member_id: String,
    pub repository: String,
    pub directory: String,
}

impl MemberAttrs {
    pub fn new(
        member_id: impl Into<String>,
        repository: impl Into<String>,
        directory: impl Into<String>,
    ) -> Self {
        Self {
            member_id: member_id.into(),
            repository: repository.into(),
            directory: directory.into(),
        }
    }
}
