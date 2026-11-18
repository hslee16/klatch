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

/// The scope a claim is stored under: shared for an explicit `--shared` and for room-wide names (`ask:<N>`),
/// otherwise the member's own scope (empty when it has none).
pub fn claim_scope(path: &str, shared: bool, member_scope: Option<&str>) -> String {
    if shared || path.starts_with("ask:") {
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
