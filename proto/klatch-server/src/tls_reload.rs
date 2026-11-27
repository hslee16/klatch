//! Reload of the TLS files on SIGHUP (#1492, refs #108, #1491), without a restart and without cutting an open stream.
//!
//! **What is read again.** The server certificate chain and key, the client CA file and, if `--tls-client-crl` was given,
//! the client CRL file: through the same functions as at start (`tls::client_roots`, `tls::build_config`,
//! `tls_crl::read_crls`), so every check of start applies and none is copied here: the key is a regular file, not a link,
//! owned by this user, mode 0600, and holds a key; the certificate file holds a certificate and fits the key; the CA file
//! holds at least one certificate (an empty root store is refused as the bind policy refuses it at start); the CRL
//! file is a regular file owned by this user or root, not writable by group or other, at most 4 MiB, holds CRLs, one per
//! issuer, each with a readable issuer and a `nextUpdate`, and the CRL warnings are printed again for the new file.
//! Nothing a flag decided changes: the listen address, the limits and the file names are those of start.
//!
//! **All or nothing.** A new `ServerConfig` is built completely, off the accept loop's thread, and only a finished one is
//! offered to [`Current::swap`], which replaces one `Arc` behind a `Mutex` (a pointer assignment; the lock is held for no
//! other work). A connection takes `Arc::clone` of the current one when it is accepted and keeps it until it ends, so a
//! reload never changes a connection that is open and never shows a new connection a half-built configuration. On ANY
//! failure the old configuration stays and one fixed line says which kind of failure it was (never a file's content, a
//! subject or a serial).
//!
//! **Bounded.** The signal handler only sets a flag (`daemon::RELOAD`; no allocation, no lock, no I/O). One watcher thread
//! reads the flag about every 100 ms and clears it as it starts a reload, so a flood of signals is one reload in progress
//! and at most one more pending, and a reload starts at most once in [`SPACING`] (a sustained flood is a few reloads a
//! second, each a few file reads). The build runs on its own thread and the watcher waits for it at most [`DEADLINE`]: a
//! file that blocks (the files are opened without blocking, so a FIFO is refused at once; a file on a stalled mount can
//! still block) costs one abandoned thread and a failure line, never the accept loop, never a swap that arrives late, and
//! while it is stuck no other reload starts until it has been stuck for [`ABANDON_AFTER`] (#1637): then the next reload gives it up, says so in the log (a restart clears the thread), and runs; the abandoned build has no receiver and its own busy flag, so its late answer is never swapped in and never clears the flag of the build that replaced it. At most [`MAX_ABANDONED`] builds are given up in a run, after which a stuck build keeps the flag. Stopping the server does not wait for it.
//!
//! **A stale CRL file.** A reload refuses a CRL file whose `nextUpdate` has passed while the CRLs in force are in date (the
//! old configuration stays, the expiry is an ALERT line): see `tls_crl::Stage`.

use crate::tls::{self, SharedConfig, TlsFiles};
use crate::tls_crl::Stage;
use crate::wire_https::Options;
use anyhow::{Result, bail};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A reload starts at most this often.
pub const SPACING: Duration = Duration::from_millis(500);
/// The longest a build may take before the reload is given up.
pub const DEADLINE: Duration = Duration::from_secs(5);
/// How often the watcher looks at the flags.
const TICK: Duration = Duration::from_millis(100);

/// The configuration new connections use.
pub struct Current<T>(Mutex<Arc<T>>);

impl<T> Current<T> {
    pub fn new(config: Arc<T>) -> Self {
        Current(Mutex::new(config))
    }

    /// The configuration for a connection that is just accepted: a clone of the pointer, never a copy.
    pub fn get(&self) -> Arc<T> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Replace the configuration with a complete one (one pointer assignment under the lock).
    fn swap(&self, config: Arc<T>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = config;
    }
}

/// The kinds of failure a log line may name: fixed words, nothing from the files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    CaFile,
    CrlFile,
    IntermediatesFile,
    KeyFile,
    Certificate,
    NoFit,
    Other,
    TimedOut,
    Busy,
    Restart,
}

impl Kind {
    fn words(self) -> &'static str {
        match self {
            Kind::CaFile => "client CA file",
            Kind::CrlFile => "client CRL file",
            Kind::IntermediatesFile => "client intermediates file",
            Kind::KeyFile => "server key file",
            Kind::Certificate => "server certificate",
            Kind::NoFit => "key and certificate do not fit",
            Kind::Other => "other",
            Kind::TimedOut => "timeout (a file did not answer in time)",
            Kind::Busy => "an earlier reload has not finished",
            Kind::Restart => {
                "an earlier reload is stuck and no more may be abandoned: restart the server (reloads also resume if the stuck build finishes)"
            }
        }
    }
}

/// The line of a failed reload: a fixed template, one kind.
fn failure_line(kind: Kind) -> String {
    format!(
        "klatch-server: https: reload failed: {}; keeping the previous TLS configuration",
        kind.words()
    )
}

/// The line of a reload that was swapped in.
fn success_line(files: &TlsFiles) -> String {
    format!(
        "klatch-server: https: reload ok: new connections use the new server certificate and key, the client CA file{}; connections already open keep theirs",
        match (
            files.client_crl.is_some(),
            files.client_intermediates.is_some()
        ) {
            (true, true) => ", the client CRL file and the client intermediates file",
            (true, false) => " and the client CRL file",
            _ => "",
        }
    )
}

/// Which file a failure of the build is about, from its message with the operator's file names taken out (a directory
/// named like a message must not change the answer).
fn classify(error: &anyhow::Error, files: &TlsFiles) -> Kind {
    let mut text = error.to_string();
    let paths = [
        Some(&files.cert),
        Some(&files.key),
        Some(&files.client_ca),
        files.client_crl.as_ref(),
        files.client_intermediates.as_ref(),
    ];
    for p in paths.into_iter().flatten() {
        text = text.replace(&p.display().to_string(), "");
    }
    if text.contains("do not fit together") {
        Kind::NoFit
    } else if text.contains("client CRL file") {
        Kind::CrlFile
    } else if text.contains("client intermediates file") {
        Kind::IntermediatesFile
    } else if text.contains("client CA file") {
        Kind::CaFile
    } else if text.contains("key file") {
        Kind::KeyFile
    } else if text.contains("server certificate") {
        Kind::Certificate
    } else {
        Kind::Other
    }
}

/// What a build of the new configuration gives: the configuration and the earliest `nextUpdate` of its CRLs.
type Built = (SharedConfig, Option<i64>);

/// The new configuration for the files and flags of start, or why not: every check of start, nothing swapped. A CRL file
/// that has expired is refused while the CRLs in force (`in_force_due`) have not.
fn build(opts: &Options, in_force_due: Option<i64>) -> Result<Built> {
    let roots = tls::client_roots(&opts.files.client_ca)?;
    if roots.is_empty() {
        bail!(
            "the client CA file {} holds no certificate",
            opts.files.client_ca.display()
        );
    }
    tls::build_config_at(&opts.files, roots, Stage::Reload { in_force_due })
}

/// Held by the build thread: when it ends, however it ends, it clears the flag of ITS build (a build that was abandoned
/// clears only its own, never the flag of the build that replaced it) and, if it had been given up meanwhile, gives its
/// place back to the abandon cap (#1694), so the cap means "abandoned and still stuck".
struct Done {
    busy: Arc<AtomicBool>,
    /// Set by whichever of the two comes second, the abandoning reload or the end of the build: the second one to set it
    /// knows the other has happened, so the count is raised and lowered exactly once and never goes below zero.
    ended: Arc<AtomicBool>,
    abandoned: Arc<AtomicUsize>,
}

/// Lower the count by one, never below zero.
fn give_back(count: &AtomicUsize) {
    let mut seen = count.load(Ordering::SeqCst);
    while seen > 0 {
        match count.compare_exchange(seen, seen - 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return,
            Err(now) => seen = now,
        }
    }
}

impl Drop for Done {
    fn drop(&mut self) {
        self.busy.store(false, Ordering::SeqCst);
        if self.ended.swap(true, Ordering::SeqCst) {
            // the abandoning reload got here first and counted this build: it is no longer stuck
            give_back(&self.abandoned);
        }
    }
}

/// The bounds of one reload.
struct Limits {
    /// How long the watcher waits for a build's answer.
    deadline: Duration,
    /// How long a build may stay unfinished before the next reload gives it up (a stalled mount; a restart is the cure).
    abandon_after: Duration,
    /// The most abandoned builds that may be stuck at once: past it a stuck build keeps the flag (a few threads, never one
    /// per reload).
    max_abandoned: usize,
}

/// How long a build may be stuck before it is abandoned (#1637): far past [`DEADLINE`], so only a hung read reaches it.
pub const ABANDON_AFTER: Duration = Duration::from_secs(120);
/// The most abandoned builds that may be stuck at one time (one that ends gives its place back).
const MAX_ABANDONED: usize = 3;

/// The state of the build in flight: its own busy flag, when it started, and how many given-up builds are still stuck.
struct Slot {
    busy: Arc<AtomicBool>,
    /// The `ended` marker of the build in flight (see [`Done`]).
    ended: Arc<AtomicBool>,
    started: Instant,
    abandoned: Arc<AtomicUsize>,
}

impl Slot {
    fn new() -> Self {
        Slot {
            busy: Arc::new(AtomicBool::new(false)),
            ended: Arc::new(AtomicBool::new(false)),
            started: Instant::now(),
            abandoned: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// How many given-up builds are still unfinished.
    #[cfg(test)]
    fn abandoned_now(&self) -> usize {
        self.abandoned.load(Ordering::SeqCst)
    }

    /// Take the right to build: `Err(restart_needed)` while an earlier build is unfinished and not yet abandonable. Past the bound, and
    /// under the cap, the unfinished build is abandoned: the slot gets a NEW flag (the old build clears only the old one)
    /// and its answer, if it ever comes, has no receiver, so it can never be swapped in. `Ok(true)` says one was abandoned.
    fn claim(
        &mut self,
        abandon_after: Duration,
        max_abandoned: usize,
    ) -> Result<(bool, Done), bool> {
        let mut gave_up = false;
        if self.busy.load(Ordering::SeqCst) {
            let overdue = self.started.elapsed() >= abandon_after;
            if !overdue || self.abandoned.load(Ordering::SeqCst) >= max_abandoned {
                // `Err(true)`: given up but for the cap, so a restart clears it, as does the stuck build finishing (#1694, #1704)
                return Err(overdue);
            }
            // count it before the marker is set, and take the count back if the build had already ended
            self.abandoned.fetch_add(1, Ordering::SeqCst);
            if self.ended.swap(true, Ordering::SeqCst) {
                // it ended in the meantime: it never was stuck, and its end has already run
                give_back(&self.abandoned);
            } else {
                gave_up = true;
            }
            self.busy = Arc::new(AtomicBool::new(false));
        }
        // every build gets a marker of its own: one left over from a build that ended normally is already set, and would
        // make the next stuck build uncounted when abandoned and its end take a place from another
        self.ended = Arc::new(AtomicBool::new(false));
        self.busy.store(true, Ordering::SeqCst);
        self.started = Instant::now();
        Ok((
            gave_up,
            Done {
                busy: self.busy.clone(),
                ended: self.ended.clone(),
                abandoned: self.abandoned.clone(),
            },
        ))
    }
}

/// The line logged when a stuck build was given up.
const ABANDONED_LINE: &str = "klatch-server: https: reload: an earlier reload build is stuck (a file did not answer, perhaps a stalled mount) and was abandoned; its result will never be applied; restart the server to clear it";

/// How a wait for a build ended.
#[derive(Debug, PartialEq, Eq)]
enum Wait<T> {
    Got(T),
    TimedOut,
    Gone,
    Stopped,
}

/// Wait for the build's answer for at most `deadline`, looking at `stop` every [`TICK`]: stopping the server does not
/// wait for a build (the thread is abandoned, and the process ends).
fn wait_for<T>(rx: &Receiver<T>, stop: &AtomicBool, deadline: Duration) -> Wait<T> {
    let until = Instant::now() + deadline;
    loop {
        if stop.load(Ordering::SeqCst) {
            return Wait::Stopped;
        }
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Wait::TimedOut;
        }
        match rx.recv_timeout(left.min(TICK)) {
            Ok(v) => return Wait::Got(v),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Wait::Gone,
        }
    }
}

/// One reload: build on a thread of its own, wait at most [`DEADLINE`], swap only a finished configuration. Returns the
/// line to log (`None` when the server is stopping).
fn reload_once(
    opts: &Options,
    current: &Current<tls::Config>,
    slot: &mut Slot,
    stop: &AtomicBool,
    in_force_due: &mut Option<i64>,
) -> Option<String> {
    let owned = opts.clone();
    let due = *in_force_due;
    let limits = Limits {
        deadline: DEADLINE,
        abandon_after: ABANDON_AFTER,
        max_abandoned: MAX_ABANDONED,
    };
    reload_with(
        move || build(&owned, due).map_err(|e| classify(&e, &owned.files)),
        &success_line(&opts.files),
        current,
        slot,
        stop,
        in_force_due,
        &limits,
    )
}

/// [`reload_once`] with the build, the success line and the bounds given (so the bounds can be tested).
fn reload_with<T: Send + Sync + 'static>(
    run: impl FnOnce() -> Result<(Arc<T>, Option<i64>), Kind> + Send + 'static,
    ok_line: &str,
    current: &Current<T>,
    slot: &mut Slot,
    stop: &AtomicBool,
    in_force_due: &mut Option<i64>,
    limits: &Limits,
) -> Option<String> {
    let (gave_up, guard) = match slot.claim(limits.abandon_after, limits.max_abandoned) {
        Ok(taken) => taken,
        Err(true) => return Some(failure_line(Kind::Restart)),
        Err(false) => return Some(failure_line(Kind::Busy)),
    };
    let (tx, rx) = channel();
    let busy = guard.busy.clone();
    let spawned = std::thread::Builder::new()
        .name("tls-reload".into())
        .spawn(move || {
            let _done = guard;
            let _ = tx.send(run());
        });
    if spawned.is_err() {
        busy.store(false, Ordering::SeqCst);
        return Some(failure_line(Kind::Other));
    }
    let line = match wait_for(&rx, stop, limits.deadline) {
        Wait::Got(Ok((config, next_due))) => {
            current.swap(config);
            *in_force_due = next_due;
            ok_line.to_string()
        }
        Wait::Got(Err(kind)) => failure_line(kind),
        Wait::TimedOut => failure_line(Kind::TimedOut),
        Wait::Gone => failure_line(Kind::Other),
        Wait::Stopped => return None,
    };
    Some(if gave_up {
        format!("{ABANDONED_LINE}\n{line}")
    } else {
        line
    })
}

/// Watch `flag` (set by SIGHUP) until `stop`: each time it is set, and at least [`SPACING`] after the last start, reload
/// and print the line. Runs on its own thread, never on the accept loop's. `in_force_due` is the earliest `nextUpdate`
/// of the CRLs the server started with.
pub fn watch(
    stop: &AtomicBool,
    flag: &AtomicBool,
    opts: &Options,
    current: &Current<tls::Config>,
    mut in_force_due: Option<i64>,
) {
    let mut slot = Slot::new();
    let mut last: Option<Instant> = None;
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(TICK);
        let due = last.is_none_or(|t| t.elapsed() >= SPACING);
        // taking the flag clears it: signals that arrive during the reload set it again, and are one pending reload
        if due && flag.swap(false, Ordering::SeqCst) {
            last = Some(Instant::now());
            if let Some(line) = reload_once(opts, current, &mut slot, stop, &mut in_force_due) {
                eprintln!("{line}");
            }
        }
    }
}

#[cfg(test)]
#[path = "tls_reload_tests.rs"]
mod tests;
