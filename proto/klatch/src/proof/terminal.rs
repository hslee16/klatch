//! Which terminal an admin proof is attempted from (#1274): a short label that is not secret, the controlling
//! terminal's device and its session. The human's terminal gives the same label for every command typed in it; a
//! program that makes a pseudo-terminal of its own gets a different one, so its wrong guesses are counted apart.

/// Keep a label to plain characters and a short length, so it can sit in a policy row and an audit row.
pub(super) fn clean(raw: &str) -> String {
    let s: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/') {
                c
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    if s.is_empty() { "none".into() } else { s }
}

/// The label of this process's controlling terminal: `<device>/s<session id>` (`ttys003/s4121`), or `none` when
/// the process has no terminal (it cannot be asked anyway, so nothing is charged to it).
pub fn current() -> String {
    clean(&super::tty::terminal_label())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_is_plain_and_short() {
        assert_eq!(clean("ttys003/s4121"), "ttys003/s4121");
        assert_eq!(clean("a;b=c d"), "a_b_c_d");
        assert_eq!(clean(""), "none");
        assert_eq!(clean(&"x".repeat(200)).len(), 48);
    }
}
