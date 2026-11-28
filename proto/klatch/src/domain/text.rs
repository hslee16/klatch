/// Names are restricted so they cannot carry newlines or fake headers into a model's context.
pub fn valid_name(s: &str) -> bool {
    safe_name_bytes(s.as_bytes())
}

/// The names the system acts under (#523): `admin` is the launcher and the audit's name for the person, `human` is
/// the principal a proved admin passphrase makes (#1233), and `canary` the tripwire; `all` is the broadcast word
/// (`@all`, #674). No member, worker or session may be named one, so nothing that later keys on a member name can be
/// forged by typing it.
pub const RESERVED_NAMES: [&str; 4] = ["admin", "human", "canary", "all"];

/// Whether `name` is one the system acts under and no member may take.
pub fn is_reserved_name(name: &str) -> bool {
    RESERVED_NAMES.contains(&name)
}

/// The rule behind `valid_name` and `valid_kind`, over bytes: 1 to 40 bytes, each a letter, digit, dot, dash or
/// underscore. A byte fold, so the Kani proofs (#657) can run it on arrays; a multi-byte character has only bytes of
/// 0x80 and up, none of them in the set, so refusing by byte refuses the same text as refusing by character.
pub(crate) fn safe_name_bytes(b: &[u8]) -> bool {
    !b.is_empty()
        && b.len() <= 40
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
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
    safe_name_bytes(kind.as_bytes())
}

/// Collapse line breaks so one message stays on one line: every line separator (CR, LF, U+0085, U+2028, U+2029) and every
/// other control character (a terminal escape, a tab, a bell) becomes a space, so text shown in a listing cannot start a
/// line of its own or move a terminal's cursor. It returns a copy; where a caller stores the result (a release note is
/// stored after `flat`), the stored text has these characters as spaces.
pub fn flat(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect()
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

/// `norm` without allocating: the path without trailing slashes, `/` for the root. A byte loop, not
/// `trim_end_matches`, so the Kani proofs can cover it (#404).
fn trim_slashes(p: &str) -> &str {
    let bytes = p.as_bytes();
    let mut n = bytes.len();
    while n > 0 && bytes[n - 1] == b'/' {
        n -= 1;
    }
    if n == 0 { "/" } else { &p[..n] }
}

/// a overlaps b if either is a path-component prefix of the other.
pub fn overlaps(a: &str, b: &str) -> bool {
    let (a, b) = (trim_slashes(a), trim_slashes(b));
    // `y` is under `x` when it continues with a `/` right after `x`; the root's prefix is the empty string
    let pre = |x: &str, y: &str| {
        let x = if x == "/" { "" } else { x };
        x == y || (y.starts_with(x) && y.as_bytes().get(x.len()) == Some(&b'/'))
    };
    pre(a, b) || pre(b, a)
}

/// The marker that ends a refusal audit target cut to `MAX_AUDIT_TARGET` (#1755).
/// It is pure ASCII and starts with a bare backslash. `audit_target` writes every backslash of a key as the escape
/// `\u{5c}`, and every other escape is `\u{..}` too, so an escaped body never contains a backslash that is not
/// followed by `u{`: no key can end with the marker by itself and a cut target is never byte-identical to an uncut one.
pub const TRUNCATED: &str = "\\...[truncated]";

/// The most bytes a shown value holds, marker included.
pub const MAX_AUDIT_TARGET: usize = 128;

/// Whether a character is shown as an escape in an audit target: everything but printable ASCII (letters, digits,
/// punctuation, the plain space), and the backslash itself (so a typed `\u{202e}` cannot pass for an escape). Real
/// policy keys are ASCII, so any other character is junk that must not render like a real key: invisible format
/// characters, fillers, tags, variation selectors, combining marks, other spaces, private-use, unassigned and
/// noncharacter code points are all escaped, not picked from a list.
fn needs_escape(c: char) -> bool {
    !(c.is_ascii_graphic() || c == ' ') || c == '\\'
}

/// A caller-supplied value as it may appear in text a person or a model reads: error text, a refusal, an audit target
/// (#1755, #1763, #1768). Every character that is not printable ASCII, and the backslash, is written as a `\u{..}`
/// escape, and what is longer than `MAX_AUDIT_TARGET` bytes is cut at a whole character or escape and ends with
/// `TRUNCATED`. The result is pure ASCII with no control character, so no name can steer a terminal, hide text with a
/// bidi or tag character, or flood a reader. A plain name of printable ASCII without a backslash (letters, digits,
/// punctuation, space) up to the cap comes back unchanged. This is the one helper for it: `store::area::audit_target` is the same function.
pub fn shown(nk: &str) -> String {
    let room = MAX_AUDIT_TARGET - TRUNCATED.len();
    let mut out = String::new();
    // the length of `out` at the last piece boundary that still leaves room for the marker
    let mut cut = 0;
    for c in nk.chars() {
        if needs_escape(c) {
            out.push_str(&format!("\\u{{{:x}}}", c as u32));
        } else {
            out.push(c);
        }
        if out.len() <= room {
            cut = out.len();
        }
        if out.len() > MAX_AUDIT_TARGET {
            // the rest is not looked at, so a 100 KB key costs no more than a short one
            out.truncate(cut);
            out.push_str(TRUNCATED);
            return out;
        }
    }
    out
}

/// Every string of 0 to 3 pieces drawn from `pieces` (multi-byte characters included).
#[cfg(test)]
pub(crate) fn short_strings(pieces: &[&str]) -> Vec<String> {
    let mut all = vec![String::new()];
    let mut layer = vec![String::new()];
    for _ in 0..3 {
        layer = layer
            .iter()
            .flat_map(|s| pieces.iter().map(move |p| format!("{s}{p}")))
            .collect();
        all.extend(layer.iter().cloned());
    }
    all
}

#[cfg(test)]
mod tests {
    #[test]
    fn shown_leaves_a_plain_name_alone_and_escapes_the_rest() {
        for plain in [
            "",
            "alice",
            "a-b_c.d",
            "bad name",
            "x:y/z*",
            &"n".repeat(128),
        ] {
            assert_eq!(super::shown(plain), plain);
        }
        assert_eq!(super::shown("a\u{1b}[31m"), "a\\u{1b}[31m");
        assert_eq!(super::shown("a\u{202e}b\u{e0041}"), "a\\u{202e}b\\u{e0041}");
        assert_eq!(super::shown("a\nb\r\t\0"), "a\\u{a}b\\u{d}\\u{9}\\u{0}");
        assert_eq!(super::shown("\u{e9}\u{ff41}"), "\\u{e9}\\u{ff41}");
        assert_eq!(super::shown("a\\b"), "a\\u{5c}b");
    }

    #[test]
    fn shown_is_capped_pure_ascii_whatever_the_input() {
        let hostile = format!("{}\u{1b}[31m\u{202e}\u{e0041}\r\n", "n".repeat(100 * 1024));
        let t = super::shown(&hostile);
        assert!(t.is_ascii() && !t.chars().any(char::is_control));
        assert_eq!(t.len(), super::MAX_AUDIT_TARGET);
        assert!(t.ends_with(super::TRUNCATED));
        // exactly at the cap is not cut; one over is
        assert_eq!(super::shown(&"a".repeat(128)).len(), 128);
        assert!(super::shown(&"a".repeat(129)).ends_with(super::TRUNCATED));
        // a cut never splits an escape, even when the cap falls inside one
        for pad in 0..40 {
            let t = super::shown(&format!(
                "{}{}",
                "a".repeat(pad + 90),
                "\u{202e}".repeat(20)
            ));
            assert!(t.len() <= super::MAX_AUDIT_TARGET, "{t}");
            assert!(
                !t.trim_end_matches(super::TRUNCATED).ends_with("\\u{20"),
                "{t}"
            );
        }
    }

    use super::*;

    /// The old character-based form of `valid_name` and `valid_kind`, kept as the reference the byte form must match
    /// (the Kani proofs of #657 are over the byte form).
    fn valid_name_by_chars(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= 40
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    }

    #[test]
    fn the_byte_form_of_valid_name_and_kind_matches_the_character_form_on_every_short_string() {
        let pieces = [
            "a", "Z", "0", "-", "_", ".", " ", "/", ":", "*", "\n", "\u{e9}", "\u{ff41}",
        ];
        for s in short_strings(&pieces) {
            assert_eq!(valid_name(&s), valid_name_by_chars(&s), "{s:?}");
            assert_eq!(valid_kind(&s), valid_name_by_chars(&s), "{s:?}");
        }
        for n in 0..=45 {
            let s = "a".repeat(n);
            assert_eq!(valid_name(&s), valid_name_by_chars(&s), "{n}");
        }
        // a multi-byte character near the limit: 20 two-byte characters are 40 bytes but not in the alphabet
        assert!(!valid_name(&"\u{e9}".repeat(20)));
    }

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

    fn overlaps_reference(a: &str, b: &str) -> bool {
        let (a, b) = (norm(a), norm(b));
        let pre =
            |x: &str, y: &str| x == y || y.starts_with(&format!("{}/", x.trim_end_matches('/')));
        pre(&a, &b) || pre(&b, &a)
    }

    #[test]
    fn overlaps_equals_its_reference_on_every_short_input() {
        // every string of up to 5 characters from / a b (the first form of `overlaps`, before the Kani rewrite, #404)
        let mut all = vec![String::new()];
        let mut layer = vec![String::new()];
        for _ in 0..5 {
            layer = layer
                .iter()
                .flat_map(|s| ['/', 'a', 'b'].map(move |c| format!("{s}{c}")))
                .collect();
            all.extend(layer.iter().cloned());
        }
        for a in &all {
            for b in &all {
                assert_eq!(overlaps(a, b), overlaps_reference(a, b), "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn flat_leaves_no_line_separator_or_control_character() {
        for bad in [
            "\r", "\n", "\u{2028}", "\u{2029}", "\u{85}", "\u{1b}", "\t", "\u{7}",
        ] {
            let out = flat(&format!("a{bad}b"));
            assert_eq!(out, "a b", "{bad:?}");
        }
        let out = flat("x\u{1b}[2K\ry\u{2028}z\r\nw");
        assert!(
            out.chars()
                .all(|c| !c.is_control() && c != '\u{2028}' && c != '\u{2029}')
        );
        assert_eq!(
            flat("plain text, unicode: \u{e9}\u{4e2d}"),
            "plain text, unicode: \u{e9}\u{4e2d}"
        );
    }
}
