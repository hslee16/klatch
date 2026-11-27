//! The stop and reload flags and the signal handlers that set them (#1633, split from `daemon.rs`).

use std::sync::atomic::{AtomicBool, Ordering};

// stopping

pub(crate) static STOP: AtomicBool = AtomicBool::new(false);

/// Set by SIGHUP in the servers that install the hangup handler (the TLS listener reloads its files when it sees this;
/// the plain `http` server never reads it, so for it SIGHUP is caught and ignored). The handler does nothing else.
pub(crate) static RELOAD: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

extern "C" fn on_hangup(_: libc::c_int) {
    RELOAD.store(true, Ordering::SeqCst);
}

/// SIGTERM and SIGINT stop the server. The daemon keeps the default for SIGHUP.
pub(crate) fn install_signal_handlers() {
    install(false);
}

/// [`install_signal_handlers`], and SIGHUP sets [`RELOAD`] instead of ending the process (for the TLS listener, which
/// reloads, and the `http` listener, which has nothing to reload and so ignores it).
pub(crate) fn install_signal_handlers_with_hangup() {
    install(true);
}

fn install(hangup: bool) {
    let mut act: libc::sigaction = unsafe { std::mem::zeroed() };
    act.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as usize;
    // no SA_RESTART: the wait for a connection returns when a signal comes
    act.sa_flags = 0;
    unsafe {
        libc::sigemptyset(&mut act.sa_mask);
        libc::sigaction(libc::SIGTERM, &act, std::ptr::null_mut());
        libc::sigaction(libc::SIGINT, &act, std::ptr::null_mut());
        if hangup {
            // SA_RESTART: where the kernel honours it (macOS: every call) a read or write that the signal interrupts is
            // restarted. It is not honoured for a socket read with a timeout, poll or nanosleep on Linux, which return
            // EINTR whatever the flag says: this code does not rely on the flag there (a handshake read cut short that
            // way fails that one handshake, #1632). The flag is pinned by the test below.
            let mut hup = act;
            hup.sa_sigaction = on_hangup as extern "C" fn(libc::c_int) as usize;
            hup.sa_flags = libc::SA_RESTART;
            libc::sigaction(libc::SIGHUP, &hup, std::ptr::null_mut());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The handler of `signal` and its `SA_RESTART` bit. Other flag bits are left out on purpose: glibc (and musl on
    /// some architectures) add `SA_RESTORER` to every action it installs, so a whole-flags comparison never matches on
    /// Linux (#1708).
    fn flags_of(signal: libc::c_int) -> (usize, libc::c_int) {
        let mut cur: libc::sigaction = unsafe { std::mem::zeroed() };
        let r = unsafe { libc::sigaction(signal, std::ptr::null(), &mut cur) };
        assert_eq!(r, 0);
        (cur.sa_sigaction, cur.sa_flags & libc::SA_RESTART)
    }

    /// Puts the signal actions back that the test found, so the test binary keeps its own Ctrl-C (#1689).
    struct Restore([(libc::c_int, libc::sigaction); 3]);

    impl Restore {
        fn take() -> Self {
            let one = |signal| {
                let mut old: libc::sigaction = unsafe { std::mem::zeroed() };
                unsafe { libc::sigaction(signal, std::ptr::null(), &mut old) };
                (signal, old)
            };
            Restore([one(libc::SIGINT), one(libc::SIGTERM), one(libc::SIGHUP)])
        }
    }

    impl Drop for Restore {
        fn drop(&mut self) {
            for (signal, old) in &self.0 {
                unsafe { libc::sigaction(*signal, old, std::ptr::null_mut()) };
            }
        }
    }

    #[test]
    fn the_hangup_handler_restarts_calls_and_the_stop_handlers_do_not() {
        let before = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP].map(|s| (s, flags_of(s)));
        let restore = Restore::take();
        install_signal_handlers_with_hangup();
        let (handler, flags) = flags_of(libc::SIGHUP);
        assert_eq!(handler, on_hangup as extern "C" fn(libc::c_int) as usize);
        assert_ne!(flags & libc::SA_RESTART, 0, "SIGHUP must restart calls");
        for s in [libc::SIGTERM, libc::SIGINT] {
            let (handler, flags) = flags_of(s);
            assert_eq!(handler, on_signal as extern "C" fn(libc::c_int) as usize);
            assert_eq!(
                flags & libc::SA_RESTART,
                0,
                "stop must interrupt the wait for a connection"
            );
        }
        // the guard puts back what it found (#1689): the test binary keeps its own Ctrl-C
        drop(restore);
        for (s, was) in before {
            assert_eq!(flags_of(s), was, "signal {s} was not put back");
        }
    }
}
