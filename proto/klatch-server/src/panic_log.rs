//! What a panic writes to the log (#1645): two kinds of line, each paced, none with the panic message, an address, or an
//! absolute build path. The panic hook says where the panic happened (crate-relative), once per panic; a handler that
//! panicked and was caught says one fixed line. Each kind has its own [`Pacer`]: the first is said at once, then at most one
//! summary per [`LINE_EVERY`] with the count since the last line. So a burst of panics prints at most two lines in all.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The line a panicking handler leaves in the log. Fixed: no message, no address, no data.
pub const PANIC_LINE: &str = "klatch-server: handler panicked";

/// The least time between two lines of one kind.
pub const LINE_EVERY: Duration = Duration::from_secs(10);

/// What a [`Pacer`] says is due.
#[derive(Debug, PartialEq, Eq)]
pub enum Due {
    /// The first event.
    First,
    /// A later one after the interval: the count since the last line, that event included.
    Summary(u64),
}

/// Paces one kind of line. The clock is an argument, so a test never sleeps.
pub struct Pacer {
    /// No line before this; `None` until the first event.
    next_line: Option<Instant>,
    /// Events since the last line was handed out.
    unsaid: u64,
}

impl Pacer {
    pub const fn new() -> Pacer {
        Pacer {
            next_line: None,
            unsaid: 0,
        }
    }

    /// An event at `now`: what is due, if anything.
    pub fn event(&mut self, now: Instant) -> Option<Due> {
        self.unsaid = self.unsaid.saturating_add(1);
        if self.next_line.is_some_and(|at| now < at) {
            return None;
        }
        let first = self.next_line.is_none();
        let count = std::mem::take(&mut self.unsaid);
        self.next_line = Some(now + LINE_EVERY);
        Some(if first {
            Due::First
        } else {
            Due::Summary(count)
        })
    }
}

/// The two pacers of the process.
pub struct Lines {
    hook: Pacer,
    handler: Pacer,
}

impl Lines {
    pub const fn new() -> Lines {
        Lines {
            hook: Pacer::new(),
            handler: Pacer::new(),
        }
    }

    /// The panic hook ran at `now` for a panic at `at`: its line, if one is due.
    pub fn hook(&mut self, now: Instant, at: Option<(&str, u32)>) -> Option<String> {
        Some(match self.hook.event(now)? {
            Due::First => internal_error_line(at),
            Due::Summary(n) => format!("klatch-server: internal error ({n} since the last line)"),
        })
    }

    /// A handler panic was caught at `now`: its line, if one is due.
    pub fn handler(&mut self, now: Instant) -> Option<String> {
        Some(match self.handler.event(now)? {
            Due::First => PANIC_LINE.to_string(),
            Due::Summary(n) => format!("{PANIC_LINE} ({n} since the last line)"),
        })
    }
}

static LINES: Mutex<Lines> = Mutex::new(Lines::new());

fn said(line: Option<String>) {
    if let Some(line) = line {
        eprintln!("{line}");
    }
}

/// The panic hook's whole job: say the place of a panic, paced.
pub fn on_panic(at: Option<(&str, u32)>) {
    let line = LINES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .hook(Instant::now(), at);
    said(line);
}

/// Log that a handler panicked, paced.
pub fn handler_panicked() {
    let line = LINES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .handler(Instant::now());
    said(line);
}

/// `file` as a form that shows no username or build directory: a cargo registry path keeps the crate and below
/// (`serde-1.0.200/src/de.rs`), a toolchain path keeps `rustc/...`, a relative path (what cargo gives a workspace crate)
/// is kept, and any other absolute path shows only its file name.
pub fn crate_relative(file: &str) -> String {
    let path = file.replace('\\', "/");
    let absolute = path.starts_with('/') || path.as_bytes().get(1) == Some(&b':');
    if !absolute {
        return path;
    }
    if let Some(i) = path.find("/registry/src/") {
        // skip the index directory (`index.crates.io-<hash>`)
        let below = &path[i + "/registry/src/".len()..];
        if let Some((_, rest)) = below.split_once('/') {
            return rest.to_string();
        }
    }
    if let Some(rest) = path.strip_prefix("/rustc/") {
        return format!("rustc/{rest}");
    }
    path.rsplit('/').next().unwrap_or_default().to_string()
}

/// The line for the first panic: where it happened (crate-relative) and nothing it carried.
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

    const SITE: Option<(&str, u32)> =
        Some(("/Users/alice/src/klatch/proto/klatch-server/src/x.rs", 7));

    #[test]
    fn a_burst_of_1000_panics_prints_two_lines_in_all_and_the_summaries_count_exactly() {
        let t0 = Instant::now();
        let mut lines = Lines::new();
        let mut said = Vec::new();
        for i in 0..1000u64 {
            // a handler panic runs the hook and then the handler's line; the burst fits in a millisecond
            let now = t0 + Duration::from_micros(i);
            said.extend(lines.hook(now, SITE));
            said.extend(lines.handler(now));
        }
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(
            said[0].starts_with("klatch-server: internal error at x.rs:7;"),
            "{said:?}"
        );
        assert_eq!(said[1], PANIC_LINE);
        // inside the interval: nothing from either
        let later = t0 + Duration::from_secs(9);
        assert_eq!(
            (lines.hook(later, SITE), lines.handler(later)),
            (None, None)
        );
        // 1000 in the burst less the one said, plus the one at 9 s, plus this one: 1001 of each
        let at = t0 + LINE_EVERY;
        assert_eq!(
            lines.hook(at, SITE).unwrap(),
            "klatch-server: internal error (1001 since the last line)"
        );
        assert_eq!(
            lines.handler(at).unwrap(),
            format!("{PANIC_LINE} (1001 since the last line)")
        );
        // and the count starts again
        assert_eq!(lines.handler(at + Duration::from_secs(1)), None);
        let next = at + LINE_EVERY;
        assert_eq!(
            lines.handler(next).unwrap(),
            format!("{PANIC_LINE} (2 since the last line)")
        );
    }

    #[test]
    fn a_lone_panic_after_a_quiet_spell_is_a_summary_of_one() {
        let t0 = Instant::now();
        let mut lines = Lines::new();
        assert_eq!(lines.handler(t0).as_deref(), Some(PANIC_LINE));
        assert_eq!(
            lines.handler(t0 + Duration::from_secs(3600)).unwrap(),
            format!("{PANIC_LINE} (1 since the last line)")
        );
    }

    #[test]
    fn the_hook_line_names_a_trimmed_place_and_nothing_else() {
        let line = internal_error_line(Some((
            "/Users/someone/.cargo/registry/src/index.crates.io-6f17d22bba15001f/tokio-1.0.0/src/io.rs",
            42,
        )));
        assert_eq!(
            line,
            "klatch-server: internal error at tokio-1.0.0/src/io.rs:42; this is a bug (nothing you sent is shown here)"
        );
        assert!(internal_error_line(None).contains("unknown place"));
        let first = Lines::new().hook(Instant::now(), SITE).unwrap();
        assert!(
            !first.contains("alice") && !first.contains("/Users"),
            "{first}"
        );
    }

    #[test]
    fn a_path_shows_no_username_whatever_its_shape() {
        let cases = [
            // cargo registry: the crate and below
            (
                "/home/alice/.cargo/registry/src/index.crates.io-6f17d22bba15001f/serde-1.0.200/src/de.rs",
                "serde-1.0.200/src/de.rs",
            ),
            // toolchain
            (
                "/rustc/0123456789abcdef/library/core/src/option.rs",
                "rustc/0123456789abcdef/library/core/src/option.rs",
            ),
            // relative: what cargo gives a workspace crate
            (
                "klatch-server/src/wire_http.rs",
                "klatch-server/src/wire_http.rs",
            ),
            ("src/main.rs", "src/main.rs"),
            // any other absolute path: the file name only
            (
                "/Users/alice/src/klatch/proto/klatch-server/src/wire_http.rs",
                "wire_http.rs",
            ),
            ("/home/alice/src/proj/build.rs", "build.rs"),
            (
                "/Users/alice/src/klatch/proto/target/release/build/foo/out/gen.rs",
                "gen.rs",
            ),
            (
                "C:\\Users\\alice\\work\\klatch-server\\src\\main.rs",
                "main.rs",
            ),
            ("", ""),
        ];
        for (file, want) in cases {
            let got = crate_relative(file);
            assert_eq!(got, want, "{file}");
            assert!(!got.contains("alice"), "{got}");
        }
    }
}
