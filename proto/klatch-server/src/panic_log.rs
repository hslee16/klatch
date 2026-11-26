//! What a panic writes to the log (#1645): a handler panic gives one fixed line, paced, and the panic hook prints a place
//! that is crate-relative. Neither ever carries the panic message, an address, or an absolute build path.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The line a panicking handler leaves in the log. Fixed: no message, no address, no data.
pub const PANIC_LINE: &str = "klatch-server: handler panicked";

/// The least time between two lines about handler panics.
pub const PANIC_LINE_EVERY: Duration = Duration::from_secs(10);

/// Paces the handler-panic line: the first panic is said at once; later ones are counted, and once the interval has passed
/// a panic brings one summary line with the count since the last line (that panic included). The clock is an argument,
/// so a test never sleeps.
pub struct Pacer {
    /// No line before this; `None` until the first panic.
    next_line: Option<Instant>,
    /// Panics since the last line was handed out.
    unsaid: u64,
}

impl Pacer {
    pub const fn new() -> Pacer {
        Pacer {
            next_line: None,
            unsaid: 0,
        }
    }

    /// A handler panicked at `now`: the line to print, if one is due.
    pub fn panicked(&mut self, now: Instant) -> Option<String> {
        self.unsaid = self.unsaid.saturating_add(1);
        let due = self.next_line.is_none_or(|at| now >= at);
        if !due {
            return None;
        }
        let first = self.next_line.is_none();
        let count = std::mem::take(&mut self.unsaid);
        self.next_line = Some(now + PANIC_LINE_EVERY);
        Some(if first {
            PANIC_LINE.to_string()
        } else {
            format!("{PANIC_LINE} ({count} since the last line)")
        })
    }
}

static PACER: Mutex<Pacer> = Mutex::new(Pacer::new());

/// Log that a handler panicked, at most one line per [`PANIC_LINE_EVERY`].
pub fn handler_panicked() {
    let line = PACER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .panicked(Instant::now());
    if let Some(line) = line {
        eprintln!("{line}");
    }
}

/// `file` as the crate-relative form: the crate's directory and what is below it (`klatch-server/src/wire_http.rs`,
/// `serde-1.0.200/src/de.rs`, `core/src/option.rs`), never the absolute build path before it. A path with no `src`
/// directory shows only its file name.
pub fn crate_relative(file: &str) -> String {
    let parts: Vec<&str> = file.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    match parts.iter().rposition(|p| *p == "src") {
        Some(0) => parts.join("/"),
        Some(i) => parts[i - 1..].join("/"),
        None => parts.last().map_or_else(String::new, |p| p.to_string()),
    }
}

/// The line the panic hook prints: where the panic happened (crate-relative) and nothing it carried.
pub fn internal_error_line(at: Option<(&str, u32)>) -> String {
    let at = at.map_or_else(
        || "unknown place".to_string(),
        |(file, line)| format!("{}:{line}", crate_relative(file)),
    );
    format!("klatch-server: internal error at {at}; this is a bug (nothing you sent is shown here)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hook_line_has_the_crate_relative_place_and_no_build_path() {
        let line = internal_error_line(Some((
            "/Users/someone/.cargo/registry/src/index.crates.io-6f17d22bba15001f/tokio-1.0.0/src/io.rs",
            42,
        )));
        assert_eq!(
            line,
            "klatch-server: internal error at tokio-1.0.0/src/io.rs:42; this is a bug (nothing you sent is shown here)"
        );
        assert!(internal_error_line(None).contains("unknown place"));
    }

    #[test]
    fn a_burst_of_panics_gives_one_line_and_then_one_summary_with_the_exact_count() {
        let t0 = Instant::now();
        let mut pacer = Pacer::new();
        let mut lines = Vec::new();
        for i in 0..1000u64 {
            // the whole burst fits in the first second
            lines.extend(pacer.panicked(t0 + Duration::from_micros(i)));
        }
        assert_eq!(lines, vec![PANIC_LINE.to_string()], "a burst was not paced");
        // still inside the interval: nothing
        assert_eq!(pacer.panicked(t0 + Duration::from_secs(9)), None);
        // 1000 in the burst less the one said, plus the one at 9 s, plus this one
        let summary = pacer.panicked(t0 + PANIC_LINE_EVERY).unwrap();
        assert_eq!(summary, format!("{PANIC_LINE} (1001 since the last line)"));
        // and the count starts again
        assert_eq!(
            pacer.panicked(t0 + PANIC_LINE_EVERY + Duration::from_secs(1)),
            None
        );
        let next = pacer.panicked(t0 + PANIC_LINE_EVERY * 2).unwrap();
        assert_eq!(next, format!("{PANIC_LINE} (2 since the last line)"));
    }

    #[test]
    fn a_lone_panic_after_a_quiet_spell_is_a_summary_of_one() {
        let t0 = Instant::now();
        let mut pacer = Pacer::new();
        assert_eq!(pacer.panicked(t0).as_deref(), Some(PANIC_LINE));
        assert_eq!(
            pacer.panicked(t0 + Duration::from_secs(3600)).unwrap(),
            format!("{PANIC_LINE} (1 since the last line)")
        );
    }

    #[test]
    fn the_lines_are_a_fixed_template_and_a_number() {
        let t0 = Instant::now();
        let mut pacer = Pacer::new();
        let mut all = vec![pacer.panicked(t0).unwrap()];
        all.push(pacer.panicked(t0 + PANIC_LINE_EVERY).unwrap());
        for line in all {
            let rest = line.strip_prefix(PANIC_LINE).unwrap();
            let number = rest
                .strip_prefix(" (")
                .and_then(|r| r.strip_suffix(" since the last line)"));
            assert!(
                rest.is_empty() || number.is_some_and(|n| n.parse::<u64>().is_ok()),
                "{line}"
            );
        }
    }

    #[test]
    fn an_absolute_build_path_is_cut_to_the_crate_relative_form() {
        let cases = [
            (
                "/Users/someone/.cargo/registry/src/index.crates.io-6f17d22bba15001f/serde-1.0.200/src/de.rs",
                "serde-1.0.200/src/de.rs",
            ),
            (
                "/Users/someone/klatch/proto/klatch-server/src/wire_http.rs",
                "klatch-server/src/wire_http.rs",
            ),
            (
                "/rustc/0123456789abcdef/library/core/src/option.rs",
                "core/src/option.rs",
            ),
            ("src/main.rs", "src/main.rs"),
            ("/build/klatch-server/tests/x.rs", "x.rs"),
            (
                "C:\\work\\klatch-server\\src\\main.rs",
                "klatch-server/src/main.rs",
            ),
            ("", ""),
        ];
        for (file, want) in cases {
            assert_eq!(crate_relative(file), want, "{file}");
        }
        for (file, _) in cases {
            let out = crate_relative(file);
            assert!(
                !out.starts_with('/') && !out.contains("Users") && !out.contains(".cargo"),
                "{out}"
            );
        }
    }
}
