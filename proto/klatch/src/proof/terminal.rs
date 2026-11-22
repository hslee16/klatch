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
    // SAFETY: these only inspect descriptors and the process; the name buffer is ours and NUL ended.
    unsafe {
        let mut fd = -1;
        let mut device = String::new();
        for candidate in [0, 1, 2] {
            if super::tty::is_controlling_tty(candidate) {
                let mut buf = [0 as libc::c_char; 128];
                if libc::ttyname_r(candidate, buf.as_mut_ptr(), buf.len()) == 0 {
                    device = std::ffi::CStr::from_ptr(buf.as_ptr())
                        .to_string_lossy()
                        .trim_start_matches("/dev/")
                        .to_string();
                }
                fd = candidate;
                break;
            }
        }
        if fd < 0 {
            // no standard descriptor is the terminal: the controlling terminal, if there is one
            let Ok(file) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/tty")
            else {
                return "none".into();
            };
            use std::os::fd::AsRawFd;
            let sid = libc::tcgetsid(file.as_raw_fd());
            return clean(&format!("tty/s{sid}"));
        }
        clean(&format!("{device}/s{}", libc::tcgetsid(fd)))
    }
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
