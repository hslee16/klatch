//! The unlock window (#1233): after one correct passphrase a process need not be asked again for a few minutes.
//!
//! It is held in the MEMORY of one process and nowhere else: no file, no environment variable, no token another
//! process could read or replay. A child process, another `klatch`, a detached process and a restarted process all
//! start locked. The window is measured from the moment the passphrase was typed (it does not slide with use) on
//! the monotonic and the wall clock (whichever passes first ends it; #1277), and it is tied to the verifier it
//! was earned under, so changing the passphrase voids it.
//!
//! A `klatch` command is one short-lived process, so for a command run on its own the window never outlives the
//! command and every command asks. The window matters for the one long-lived process there is, `klatch admin
//! shell`, which runs many commands under one proof.

use super::At;
use std::sync::Mutex;
use std::time::Duration;

pub struct Unlock {
    held: Mutex<Option<(At, String)>>,
}

/// The window of this process: the only one a command line ever uses.
pub static PROCESS: Unlock = Unlock::new();

impl Unlock {
    pub const fn new() -> Self {
        Self {
            held: Mutex::new(None),
        }
    }

    /// Whether the window is open at `now`: opened for the verifier `id`, less than `window` ago on BOTH clocks.
    /// The monotonic clock does not count a sleep of the machine (macOS), the wall clock does, so the window ends
    /// when either passes it (#1277). A wall clock set back cannot hold it open, because the monotonic clock still ends it.
    pub fn is_open(&self, now: At, id: &str, window: Duration) -> bool {
        let held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        match held.as_ref() {
            Some((at, held_id)) => {
                held_id == id
                    && now
                        .mono
                        .checked_duration_since(at.mono)
                        .is_some_and(|d| d < window)
                    && now.unix.saturating_sub(at.unix) < window.as_secs() as i64
            }
            None => false,
        }
    }

    /// Open it, for the verifier `id`, from `now`.
    pub fn open(&self, now: At, id: &str) {
        *self.held.lock().unwrap_or_else(|e| e.into_inner()) = Some((now, id.to_string()));
    }

    /// Close it.
    pub fn close(&self) {
        *self.held.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

impl Default for Unlock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn at(t: Instant, secs: u64) -> At {
        At {
            unix: 1000 + secs as i64,
            mono: t + Duration::from_secs(secs),
        }
    }

    #[test]
    fn open_inside_the_window_closed_after_and_never_for_another_verifier() {
        let w = Unlock::new();
        let t0 = Instant::now();
        let win = Duration::from_secs(300);
        assert!(!w.is_open(at(t0, 0), "a", win), "starts locked");
        w.open(at(t0, 0), "a");
        assert!(w.is_open(at(t0, 0), "a", win));
        assert!(w.is_open(at(t0, 299), "a", win));
        assert!(!w.is_open(at(t0, 300), "a", win));
        assert!(!w.is_open(at(t0, 9999), "a", win));
        assert!(
            !w.is_open(at(t0, 0), "b", win),
            "an unlock of an old verifier is void"
        );
        w.close();
        assert!(!w.is_open(at(t0, 0), "a", win));
    }

    #[test]
    fn the_window_does_not_slide_with_use() {
        let w = Unlock::new();
        let t0 = Instant::now();
        let win = Duration::from_secs(300);
        w.open(at(t0, 0), "a");
        for s in [100, 200, 299] {
            assert!(w.is_open(at(t0, s), "a", win));
        }
        assert!(!w.is_open(at(t0, 301), "a", win));
    }
}
