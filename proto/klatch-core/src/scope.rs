//! Scopes: where a member works (DESIGN section 27). A scope is an opaque string. A coding session gets
//! `git:<repository>` by itself; a bot, a service or a human declares its own (`bot:grok`, `project:acme`). Claims
//! conflict only inside one scope, so two repositories can each claim `src/lib.rs`.

/// The scope of a claim that conflicts with every scope: a shared claim, or a room-wide resource such as an ask.
pub const SHARED_SCOPE: &str = "*";

/// A scope name is 1 to 80 characters of letters, digits and `. _ : - /`, so it is safe in output and in a path.
pub fn valid_scope(scope: &str) -> bool {
    !scope.is_empty()
        && scope.chars().count() <= 80
        && scope != SHARED_SCOPE
        && scope
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".:_-/".contains(c))
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
