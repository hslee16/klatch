/// Names are restricted so they cannot carry newlines or fake headers into a model's context.
pub fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')
}

/// Whether `body` mentions `member` as `@member`: the name stands alone, so `@bob` does not mention `bob-1` or
/// `bob.smith`, a name inside another word (an address) does not count, and a full stop that ends a sentence does not
/// belong to the name (#20).
pub fn mentions(body: &str, member: &str) -> bool {
    let target = format!("@{member}");
    body.match_indices(&target).any(|(i, _)| {
        let before = body[..i].chars().next_back();
        let rest = &body[i + target.len()..];
        let after_ok = match rest.chars().next() {
            None => true,
            Some('.') => rest[1..].chars().next().is_none_or(|n| !name_char(n)),
            Some(c) => !name_char(c),
        };
        before.is_none_or(|c| !name_char(c)) && after_ok
    })
}

/// Seconds from `earlier` to `later`, never negative: a clock set back, or a time written by a machine whose clock
/// is ahead, must not print as a negative age (#295). Decisions that compare times keep the raw difference.
pub fn secs_between(later: i64, earlier: i64) -> i64 {
    (later - earlier).max(0)
}

/// The longest payload a message may carry, in bytes. A payload is an opaque handoff for another member; it is not a
/// file transfer.
pub const MAX_PAYLOAD_BYTES: usize = 16 * 1024;

/// A handoff kind: 1 to 40 characters of letters, digits, dot, dash and underscore. klatch stores and delivers it and
/// never interprets it.
pub fn valid_kind(kind: &str) -> bool {
    !kind.is_empty()
        && kind.len() <= 40
        && kind
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Collapse line breaks so one message stays on one line.
pub fn flat(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
}

/// Strip trailing slashes; the empty path becomes "/".
pub fn norm(p: &str) -> String {
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        "/".into()
    } else {
        t.to_string()
    }
}

/// a overlaps b if either is a path-component prefix of the other.
pub fn overlaps(a: &str, b: &str) -> bool {
    let (a, b) = (norm(a), norm(b));
    let pre = |x: &str, y: &str| x == y || y.starts_with(&format!("{}/", x.trim_end_matches('/')));
    pre(&a, &b) || pre(&b, &a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_accept_the_safe_alphabet_only() {
        assert!(valid_name("alice-1_x.y"));
        assert!(!valid_name(""));
        assert!(!valid_name("a b"));
        assert!(!valid_name("a\nb"));
        assert!(!valid_name("a/b"));
        assert!(!valid_name(&"x".repeat(41)));
        assert!(valid_name(&"x".repeat(40)));
    }

    #[test]
    fn a_handoff_kind_is_a_short_safe_name() {
        for ok in ["plan", "work-order.v2", "a", "Review_1", &"k".repeat(40)] {
            assert!(valid_kind(ok), "{ok:?}");
        }
        for bad in [
            "",
            "has space",
            "a/b",
            "new\nline",
            "x:y",
            &"k".repeat(41),
            "é",
        ] {
            assert!(!valid_kind(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_mention_is_the_exact_name_after_an_at_sign() {
        let yes = [
            "@alice please look",
            "hi @alice.",
            "hi @alice, ok?",
            "(@alice)",
            "first line\n@alice second",
            "@bob and @alice",
        ];
        for body in yes {
            assert!(mentions(body, "alice"), "{body:?}");
        }
    }

    #[test]
    fn a_name_inside_another_word_or_name_is_not_a_mention() {
        let no = [
            "@alice2 x",
            "@alice-1 x",
            "@alice.smith x",
            "me@alice",
            "@Alice",
            "alice",
            "@ alice",
            "",
        ];
        for body in no {
            assert!(!mentions(body, "alice"), "{body:?}");
        }
    }

    #[test]
    fn flat_removes_line_breaks() {
        assert_eq!(flat("a\r\nb\nc"), "a  b c");
    }

    #[test]
    fn norm_strips_trailing_slashes() {
        assert_eq!(norm("src/"), "src");
        assert_eq!(norm("src//"), "src");
        assert_eq!(norm("/"), "/");
        assert_eq!(norm(""), "/");
    }

    #[test]
    fn overlap_is_by_path_component() {
        assert!(overlaps("src", "src/a.rs"));
        assert!(overlaps("src/a.rs", "src"));
        assert!(overlaps("src/", "src"));
        assert!(overlaps("/", "/abs/here"));
        assert!(!overlaps("/", "relative/here")); // root covers absolute paths only
        assert!(!overlaps("src", "srcs/a.rs"));
        assert!(!overlaps("src/a", "src/b"));
    }
}
