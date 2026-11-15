//! Parked members (DESIGN section 29 item 5, section 32 item 45). A member that has declared a long wait, in a post
//! (`parked: <why>, back by <time>`) or through the Stop hook (background tasks or crons at the end of a turn), keeps
//! its claims, presence and bookkeeping until the return time. After it, with no sign of life, it is treated as
//! gone: the claims run out as they would have.

use crate::domain::asks::parse_utc;

/// A message whose body starts with this text parks its sender.
pub const PARKED_PREFIX: &str = "parked:";
/// How long a park lasts when its source names no return time (the Stop hook knows only that something runs). Each
/// turn that ends renews it.
pub const PARKED_DEFAULT_SECS: i64 = 3600;
/// The longest a park may last whatever the member says: a hold a member can raise at will would be no hold. A
/// member that really waits longer parks again when it wakes; a human can end any hold.
pub const PARKED_MAX_SECS: i64 = 7 * 86_400;

/// Why and until when a member is parked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parked {
    pub why: String,
    /// Unix seconds of the return time.
    pub back_by: i64,
}

impl Parked {
    /// The park holds at `now`: its return time has not come.
    pub fn holds(&self, now: i64) -> bool {
        self.back_by > now
    }

    /// `parked, back in 2h: waiting on CI`, or `parked, back time passed 1h ago: ...`, for the board and `who`.
    pub fn phrase(&self, now: i64) -> String {
        let left = self.back_by - now;
        let when = if left > 0 {
            format!("back in {}", span(left))
        } else {
            format!("back time passed {} ago", span(-left))
        };
        if self.why.is_empty() {
            format!("parked, {when}")
        } else {
            format!("parked, {when}: {}", self.why)
        }
    }
}

/// `3d`, `2h` or `5m`: the largest whole unit.
pub fn span(secs: i64) -> String {
    let n = secs.max(0);
    if n >= 86_400 {
        format!("{}d", n / 86_400)
    } else if n >= 3600 {
        format!("{}h", n / 3600)
    } else {
        format!("{}m", n / 60)
    }
}

/// The park a message declares: `parked: <why>, back by <time>` where the time is a length from now (`90m`, `2h`,
/// `3d`) or a UTC moment (`2026-11-20`, `2026-11-20 14:30`). A post that names no time parks for
/// `PARKED_DEFAULT_SECS`; a time already past, or one beyond `PARKED_MAX_SECS`, is not believed (the default and the
/// maximum apply).
pub fn parse_parked(body: &str, now: i64) -> Option<Parked> {
    let rest = body.strip_prefix(PARKED_PREFIX)?.trim();
    let (why, when) = match rest.rfind(", back by ") {
        Some(i) => (&rest[..i], Some(rest[i + ", back by ".len()..].trim())),
        None => (rest, None),
    };
    let wanted = when.and_then(|w| length(w).map(|n| now + n).or_else(|| parse_utc(w)));
    let back_by = match wanted {
        Some(t) if t > now => t.min(now + PARKED_MAX_SECS),
        _ => now + PARKED_DEFAULT_SECS,
    };
    Some(Parked {
        why: why.trim().to_string(),
        back_by,
    })
}

/// `90m`, `2h` or `3d` as seconds.
fn length(w: &str) -> Option<i64> {
    let unit = match w.chars().last()? {
        'm' => 60,
        'h' => 3600,
        'd' => 86_400,
        _ => return None,
    };
    let n: i64 = w[..w.len() - 1].parse().ok()?;
    (n > 0).then(|| n.saturating_mul(unit))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn a_parked_post_gives_the_reason_and_the_return_time() {
        let p = parse_parked("parked: waiting on the nightly build, back by 3h", NOW).unwrap();
        assert_eq!(p.why, "waiting on the nightly build");
        assert_eq!(p.back_by, NOW + 3 * 3600);
        let p = parse_parked("parked: review, back by 90m", NOW).unwrap();
        assert_eq!(p.back_by, NOW + 5400);
        // 2027-01-20 14:30 UTC
        let p = parse_parked("parked: x, back by 2027-01-20 14:30", NOW).unwrap();
        assert_eq!(p.back_by, 20_838 * 86_400 + 14 * 3600 + 30 * 60);
    }

    #[test]
    fn a_missing_or_unbelievable_time_uses_the_default_and_the_maximum_caps_it() {
        let d = NOW + PARKED_DEFAULT_SECS;
        assert_eq!(parse_parked("parked: waiting", NOW).unwrap().back_by, d);
        assert_eq!(
            parse_parked("parked: w, back by soon", NOW)
                .unwrap()
                .back_by,
            d
        );
        assert_eq!(
            parse_parked("parked: w, back by 2001-01-01", NOW)
                .unwrap()
                .back_by,
            d
        );
        assert_eq!(
            parse_parked("parked: w, back by 0m", NOW).unwrap().back_by,
            d
        );
        let far = parse_parked("parked: w, back by 400d", NOW).unwrap();
        assert_eq!(far.back_by, NOW + PARKED_MAX_SECS);
        let far = parse_parked("parked: w, back by 2099-01-01", NOW).unwrap();
        assert_eq!(far.back_by, NOW + PARKED_MAX_SECS);
    }

    #[test]
    fn only_a_body_that_starts_with_the_word_parks() {
        assert!(parse_parked("not parked: x", NOW).is_none());
        assert!(parse_parked(" parked: x", NOW).is_none());
        assert!(parse_parked("Parked: x", NOW).is_none());
    }

    #[test]
    fn the_phrase_says_how_long_is_left_or_how_late() {
        let p = Parked {
            why: "ci".into(),
            back_by: NOW + 7300,
        };
        assert!(p.holds(NOW) && !p.holds(NOW + 7300));
        assert_eq!(p.phrase(NOW), "parked, back in 2h: ci");
        assert_eq!(
            p.phrase(NOW + 7300 + 600),
            "parked, back time passed 10m ago: ci"
        );
        let bare = Parked {
            why: String::new(),
            back_by: NOW + 60,
        };
        assert_eq!(bare.phrase(NOW), "parked, back in 1m");
    }
}
