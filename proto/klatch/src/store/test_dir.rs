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
        }
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
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn the_directory_is_removed_when_the_value_is_dropped() {
        let t = TestDir::new("drop-removes");
        let dir = t.to_path_buf();
        std::fs::write(dir.join("rooms.db"), b"data").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("x"), b"more").unwrap();
        assert!(dir.exists());
        drop(t);
        assert!(!dir.exists(), "the directory was left behind: {dir:?}");
    }

    #[test]
    fn the_directory_is_removed_when_the_test_panics() {
        let seen = Arc::new(std::sync::Mutex::new(PathBuf::new()));
        let inner = seen.clone();
        let result = std::panic::catch_unwind(move || {
            let t = TestDir::new("panic-removes");
            *inner.lock().unwrap() = t.to_path_buf();
            std::fs::write(t.join("rooms.db"), b"data").unwrap();
            panic!("a failing test");
        });
        assert!(result.is_err());
        let dir = seen.lock().unwrap().clone();
        assert!(!dir.as_os_str().is_empty());
        assert!(!dir.exists(), "a panic left the directory behind: {dir:?}");
    }

    #[test]
    fn two_directories_with_one_tag_are_different() {
        let (a, b) = (TestDir::new("same"), TestDir::new("same"));
        assert_ne!(&*a, &*b);
        drop(a);
        assert!(b.exists(), "dropping one removed the other");
    }

    #[test]
    fn the_guard_fires_when_the_directory_passes_the_cap_and_not_before() {
        let fired = Arc::new(AtomicBool::new(false));
        let flag = fired.clone();
        let t = TestDir::with_cap("guard-fires", 1024 * 1024, move |_| {
            flag.store(true, Ordering::SeqCst)
        });
        std::fs::write(t.join("under"), vec![0u8; 512 * 1024]).unwrap();
        std::thread::sleep(POLL * 5);
        assert!(!fired.load(Ordering::SeqCst), "fired under the cap");
        std::fs::create_dir(t.join("deep")).unwrap();
        std::fs::write(t.join("deep").join("over"), vec![0u8; 1024 * 1024]).unwrap();
        for _ in 0..200 {
            if fired.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(POLL);
        }
        panic!("the guard did not fire on a directory over the cap");
    }
}
