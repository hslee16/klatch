//! A scratch directory for store tests (#683): removed when the value is dropped (also when the test panics), and
//! watched while it lives, so a test, or code a mutant has changed to write without end, cannot fill the disk.
//!
//! The watch is a size guard: a thread adds up the files under the directory every few milliseconds and, past the cap,
//! stops the whole test process. A runaway writer made about 3 GB every 10 seconds in the #407 mutants run, so a
//! check every 20 ms overshoots the cap by a few megabytes at most. Stopping the process is deliberate: a thread that
//! has lost its way cannot be stopped from outside, and a test process that dies counts as a failed test (a caught
//! mutant), which is the right verdict for code that writes without end.

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

/// What a test directory may hold before the guard stops the process. The biggest fixtures write a few megabytes.
pub(crate) const SIZE_CAP: u64 = 256 * 1024 * 1024;

/// How often the guard adds up the directory.
const POLL: Duration = Duration::from_millis(20);

/// A directory under the temp directory that is deleted on drop and capped in size while it lives.
pub(crate) struct TestDir {
    dir: PathBuf,
    stop: Option<mpsc::Sender<()>>,
    guard: Option<JoinHandle<()>>,
    keep: bool,
}

impl TestDir {
    /// A new empty directory named `klatch-store-{tag}-{pid}-{n}` (the number keeps two calls with the same tag apart).
    pub(crate) fn new(tag: &str) -> Self {
        Self::with_cap(tag, SIZE_CAP, |dir| {
            // delete the runaway's files first: an abort runs no destructors, and the files are the point
            let _ = std::fs::remove_dir_all(dir);
            eprintln!("a test directory passed {SIZE_CAP} bytes: stopping the test process (#683)");
            std::process::abort()
        })
    }

    /// As `new`, with another cap and another action when the cap is passed (the guard's own test uses this).
    pub(crate) fn with_cap(
        tag: &str,
        cap: u64,
        on_exceed: impl Fn(&Path) + Send + 'static,
    ) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("klatch-store-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (stop, stopped) = mpsc::channel::<()>();
        let watched = dir.clone();
        let guard = std::thread::spawn(move || {
            // recv_timeout is the sleep: a drop (the sender goes away) ends the wait at once
            while stopped.recv_timeout(POLL) == Err(mpsc::RecvTimeoutError::Timeout) {
                if size_of(&watched) > cap {
                    on_exceed(&watched);
                    return;
                }
            }
        });
        TestDir {
            dir,
            stop: Some(stop),
            guard: Some(guard),
            keep: false,
        }
    }

    /// Leave the directory behind when this value is dropped (the bench keeps the store of a round that found a
    /// violation, for a look) and return its path. The size guard still stops with the drop.
    #[allow(dead_code)] // only the bench keeps a directory
    pub(crate) fn keep(&mut self) -> PathBuf {
        self.keep = true;
        self.dir.clone()
    }
}

/// A database path (`rooms.db`, or another file name) inside a `TestDir`, for the SQLite tests. It derefs to the path, so
/// it is passed where a `&Path` goes, and the directory goes when it is dropped.
pub(crate) struct TempDb {
    path: PathBuf,
    _dir: TestDir,
}

pub(crate) fn temp_db(tag: &str) -> TempDb {
    let dir = TestDir::new(tag);
    TempDb {
        path: dir.join("rooms.db"),
        _dir: dir,
    }
}

impl TempDb {
    /// The same scratch directory with another file name for the path.
    pub(crate) fn named(mut self, file: impl AsRef<Path>) -> Self {
        self.path = self.path.with_file_name(file.as_ref());
        self
    }
}

impl Deref for TempDb {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for TempDb {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

/// A value that lives in a scratch directory (an open store, say), with the directory held until the value is gone. It
/// derefs to the value, and the value is dropped before the directory is removed.
pub(crate) struct In<T> {
    value: T,
    _dir: TestDir,
}

impl<T> In<T> {
    /// Build the value from a new scratch directory tagged `tag`.
    pub(crate) fn new(tag: &str, make: impl FnOnce(&Path) -> T) -> Self {
        let dir = TestDir::new(tag);
        In {
            value: make(&dir),
            _dir: dir,
        }
    }
}

impl<T> Deref for In<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> std::ops::DerefMut for In<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

/// The bytes in every file under `dir`; a file that vanishes while it is counted counts as nothing.
fn size_of(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => size_of(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

impl Deref for TestDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.dir
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        &self.dir
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(g) = self.guard.take() {
            let _ = g.join();
        }
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}
