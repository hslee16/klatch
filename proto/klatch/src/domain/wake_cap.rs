//! The bound on what one `klatch read`, `klatch wait` or MCP read prints for a room (#640, #1390, DESIGN item 73). A busy
//! lobby must not flood a session: one call shows whole messages from the oldest unread, up to a message count and a
//! byte count, and the rest stay unread and are only counted. Only what was shown is marked read, so the next call
//! continues in order and nothing is lost or repeated.

use crate::domain::Message;

/// How many of `sizes` (the printed bytes of each message, oldest first) one call shows: as many whole messages as fit
/// in `max_bytes` and `max_messages`, one newline counted after each. The first is always shown, even when it alone is
/// larger than `max_bytes` (the caller cuts it with `cut_to`), so a long message can never block the ones behind it.
pub fn fit_prefix(sizes: &[usize], max_messages: usize, max_bytes: usize) -> usize {
    let mut used = 0;
    let mut shown = 0;
    for size in sizes {
        let over_bytes = shown > 0 && used + size + 1 > max_bytes;
        if shown >= max_messages.max(1) || over_bytes {
            break;
        }
        used += size + 1;
        shown += 1;
    }
    shown
}

/// `text` cut to at most `max_bytes` bytes, on a character boundary, with `marker` (which says what was cut and where
/// to read all of it) after the kept part. The marker is part of the budget. Text that already fits is returned as is.
pub fn cut_to(text: &str, max_bytes: usize, marker: impl Fn(usize) -> String) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    // the marker names the bytes left out, which depend on how much is kept; shrink until kept part and marker fit
    let mut keep = max_bytes.saturating_sub(marker(text.len()).len());
    loop {
        while keep > 0 && !text.is_char_boundary(keep) {
            keep -= 1;
        }
        let tail = marker(text.len() - keep);
        if keep + tail.len() <= max_bytes || keep == 0 {
            return format!("{}{}", &text[..keep], tail);
        }
        keep -= (keep + tail.len() - max_bytes).min(keep);
    }
}

/// How many of `left` mention `me`, for the line that says some of what was not shown is for the reader.
pub fn mentioning(left: &[Message], me: &str) -> usize {
    left.iter()
        .filter(|m| crate::domain::mentions(&m.body, me))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_messages_fit_under_the_byte_cap_and_the_message_cap() {
        assert_eq!(fit_prefix(&[10, 10, 10], 40, 100), 3);
        assert_eq!(fit_prefix(&[10, 10, 10], 2, 100), 2, "message cap");
        assert_eq!(
            fit_prefix(&[40, 40, 40], 40, 100),
            2,
            "byte cap: 41 + 41 fit, 41 more do not"
        );
        assert_eq!(fit_prefix(&[], 40, 100), 0);
    }

    #[test]
    fn the_first_message_is_always_shown_however_large() {
        assert_eq!(fit_prefix(&[500, 1], 40, 100), 1);
        assert_eq!(fit_prefix(&[5, 5], 0, 100), 1, "a cap of 0 still shows one");
    }

    #[test]
    fn a_cut_text_stays_inside_the_budget_with_its_marker_and_on_a_character_boundary() {
        let text = "é".repeat(1000);
        let out = cut_to(&text, 200, |n| format!(" [cut {n} bytes]"));
        assert!(out.len() <= 200, "{}", out.len());
        assert!(out.contains("[cut "), "{out}");
        assert!(out.starts_with('é'));
        let kept = out.split(" [cut ").next().unwrap().len();
        let tail = format!(" [cut {} bytes]", text.len() - kept);
        assert_eq!(out, format!("{}{tail}", &text[..kept]));
    }

    #[test]
    fn text_that_fits_is_not_touched() {
        assert_eq!(cut_to("short", 100, |_| "!".into()), "short");
    }
}
