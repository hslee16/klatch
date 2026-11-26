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
//! while it is stuck no other reload starts (a restart clears it). Stopping the server does not wait for it.
//!
//! **A stale CRL file.** A reload refuses a CRL file whose `nextUpdate` has passed while the CRLs in force are in date (the
//! old configuration stays, the expiry is an ALERT line): see `tls_crl::Stage`.

use crate::tls::{self, SharedConfig, TlsFiles};
use crate::tls_crl::Stage;
use crate::wire_https::Options;
use anyhow::{Result, bail};
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Clears `busy` when the build thread ends, however it ends.
struct Done(Arc<AtomicBool>);

impl Drop for Done {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

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
    busy: &Arc<AtomicBool>,
    stop: &AtomicBool,
    in_force_due: &mut Option<i64>,
) -> Option<String> {
    if busy.swap(true, Ordering::SeqCst) {
        return Some(failure_line(Kind::Busy));
    }
    let (tx, rx) = channel();
    let owned = opts.clone();
    let due = *in_force_due;
    let guard = Done(busy.clone());
    let spawned = std::thread::Builder::new()
        .name("tls-reload".into())
        .spawn(move || {
            let _done = guard;
            let result = build(&owned, due).map_err(|e| classify(&e, &owned.files));
            let _ = tx.send(result);
        });
    if spawned.is_err() {
        busy.store(false, Ordering::SeqCst);
        return Some(failure_line(Kind::Other));
    }
    match wait_for(&rx, stop, DEADLINE) {
        Wait::Got(Ok((config, next_due))) => {
            current.swap(config);
            *in_force_due = next_due;
            Some(success_line(&opts.files))
        }
        Wait::Got(Err(kind)) => Some(failure_line(kind)),
        Wait::TimedOut => Some(failure_line(Kind::TimedOut)),
        Wait::Gone => Some(failure_line(Kind::Other)),
        Wait::Stopped => None,
    }
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
    let busy = Arc::new(AtomicBool::new(false));
    let mut last: Option<Instant> = None;
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(TICK);
        let due = last.is_none_or(|t| t.elapsed() >= SPACING);
        // taking the flag clears it: signals that arrive during the reload set it again, and are one pending reload
        if due && flag.swap(false, Ordering::SeqCst) {
            last = Some(Instant::now());
            if let Some(line) = reload_once(opts, current, &busy, stop, &mut in_force_due) {
                eprintln!("{line}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn files() -> TlsFiles {
        TlsFiles {
            cert: "/d/client CA file/c.pem".into(),
            key: PathBuf::from("/d/k.pem"),
            client_ca: PathBuf::from("/d/ca.pem"),
            client_crl: Some(PathBuf::from("/d/crl.pem")),
            client_intermediates: None,
        }
    }

    #[test]
    fn a_failure_is_classified_by_its_words_and_not_by_a_directory_name() {
        let f = files();
        let e = |m: &str| anyhow::anyhow!("{m}");
        let cert = e("cannot read the server certificate /d/client CA file/c.pem: NotFound");
        assert_eq!(classify(&cert, &f), Kind::Certificate);
        let key = e("the key file /d/k.pem must be mode 0600");
        assert_eq!(classify(&key, &f), Kind::KeyFile);
        let ca = e("the client CA file /d/ca.pem holds no certificate");
        assert_eq!(classify(&ca, &f), Kind::CaFile);
        let crl = e("the client CRL file /d/crl.pem is empty");
        assert_eq!(classify(&crl, &f), Kind::CrlFile);
        let fit =
            e("the server certificate and key do not fit together, or the certificate is unusable");
        assert_eq!(classify(&fit, &f), Kind::NoFit);
        assert_eq!(classify(&e("something new"), &f), Kind::Other);
    }

    #[test]
    fn every_line_is_a_fixed_template_with_no_file_name() {
        let f = files();
        for kind in [
            Kind::CaFile,
            Kind::CrlFile,
            Kind::KeyFile,
            Kind::Certificate,
            Kind::NoFit,
            Kind::Other,
            Kind::TimedOut,
            Kind::Busy,
        ] {
            let line = failure_line(kind);
            assert!(!line.contains("/d/"), "{line}");
            assert!(line.starts_with("klatch-server: https: reload failed: "));
        }
        assert!(!success_line(&f).contains("/d/"));
    }

    #[test]
    fn a_swap_replaces_the_value_new_readers_get_and_leaves_the_old_one_alive_for_its_holders() {
        let (a, b) = (Arc::new(1), Arc::new(2));
        let current = Current::new(a.clone());
        let held = current.get();
        current.swap(b.clone());
        assert!(Arc::ptr_eq(&current.get(), &b));
        assert!(Arc::ptr_eq(&held, &a));
    }

    #[test]
    fn a_wait_for_a_build_ends_at_the_deadline_on_stop_and_when_the_builder_is_gone() {
        let stop = AtomicBool::new(false);
        let (tx, rx) = channel::<u8>();
        assert_eq!(
            wait_for(&rx, &stop, Duration::from_millis(150)),
            Wait::TimedOut
        );
        tx.send(7).unwrap();
        assert_eq!(wait_for(&rx, &stop, Duration::from_secs(5)), Wait::Got(7));
        // stopping ends a long wait within a tick, not at the deadline
        let started = Instant::now();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(200));
                stop.store(true, Ordering::SeqCst);
            });
            assert_eq!(wait_for(&rx, &stop, Duration::from_secs(30)), Wait::Stopped);
        });
        assert!(started.elapsed() < Duration::from_secs(5));
        drop(tx);
        stop.store(false, Ordering::SeqCst);
        assert_eq!(wait_for(&rx, &stop, Duration::from_secs(5)), Wait::Gone);
    }
}
