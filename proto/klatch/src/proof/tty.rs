//! The one place a passphrase is read: a real terminal, with echo off (#1233).
//!
//! It reads standard input when that is a terminal, and otherwise the process's CONTROLLING terminal (`/dev/tty`),
//! so a person who pipes something into a command is still asked at their own terminal. It never reads the
//! environment, an argument, or standard input that is a file or a pipe: a process with no terminal at all (an
//! agent's command, a detached or double-forked process, `setsid`) has no controlling terminal and is refused.
//! A process that makes a pseudo-terminal of its own can be asked; it is then refused for not knowing the
//! passphrase, which is the point.
//!
//! What this does NOT do: a program started from the person's own terminal inherits it as its controlling
//! terminal, so it can trigger a prompt there even with a pipe (or nothing) on standard input. It still cannot
//! answer it. The defence is the person reading the prompt, which names the exact command the passphrase approves;
//! typed-back confirmations and the admin shell never take a pipe (`klatch admin shell` needs standard input itself
//! to be a terminal). SECURITY.md says the same.

use super::{Prompt, ProofError};
use std::os::fd::{AsRawFd, OwnedFd};
use zeroize::Zeroizing;

/// The longest line read; a longer one is refused without being kept.
const MAX_LINE: usize = 1024;

pub struct TtyPrompt;

/// Whether two descriptors are the same device node: the same device and inode and, for a terminal, the same
/// terminal. Two fds opened on one pty slave by different paths agree; a pty someone else made does not.
fn same_device(a: i32, b: i32) -> bool {
    // SAFETY: fstat fills a plain struct for a descriptor.
    unsafe {
        let mut sa: libc::stat = std::mem::zeroed();
        let mut sb: libc::stat = std::mem::zeroed();
        libc::fstat(a, &mut sa) == 0
            && libc::fstat(b, &mut sb) == 0
            && sa.st_dev == sb.st_dev
            && sa.st_ino == sb.st_ino
            && sa.st_rdev == sb.st_rdev
    }
}

/// Whether `fd` is `/dev/null`.
fn is_dev_null(fd: i32) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(null) = std::fs::metadata("/dev/null") else {
        return false;
    };
    // SAFETY: fstat fills a plain struct for a descriptor.
    unsafe {
        let mut s: libc::stat = std::mem::zeroed();
        libc::fstat(fd, &mut s) == 0
            && (s.st_mode & libc::S_IFMT) == libc::S_IFCHR
            && s.st_rdev as u64 == null.rdev()
    }
}

/// For an act that prints a secret or stored text: standard output must be this process's controlling terminal AND
/// the very terminal the passphrase and the confirmation are read from and written to (the same device, found with
/// `fstat`, so a pty in another session that merely IS a terminal does not count), and standard error must be that
/// terminal or `/dev/null`. It stops a program that gives its own pty to standard output while the person's terminal
/// is the prompt, or captures standard error through a pipe. It does NOT stop a program that relays or hosts the
/// WHOLE terminal the person types at (SECURITY.md).
pub fn output_is_the_prompt_terminal() -> bool {
    // With standard input a terminal, the prompt is on standard input, so standard output must be that very device.
    // Otherwise the prompt is on `/dev/tty`, the controlling terminal, and a descriptor that IS the controlling
    // terminal (`is_controlling_tty`, by session) is that same terminal; comparing inode numbers there would refuse
    // it, because `/dev/tty` is a different node from the real device (#1285).
    // SAFETY: isatty only inspects the descriptor.
    let stdin_is_tty = unsafe { libc::isatty(0) } == 1;
    is_controlling_tty(1)
        && (!stdin_is_tty || same_device(0, 1))
        && open_tty().is_ok()
        && (same_device(2, 1) || is_dev_null(2))
}

/// For the admin shell: standard input and output must both be this process's controlling terminal and one device.
pub fn input_and_output_are_one_terminal() -> bool {
    is_controlling_tty(0) && is_controlling_tty(1) && same_device(0, 1)
}

/// Whether descriptor `fd` IS this process's controlling terminal: a terminal, and the one whose session is ours.
/// A pty an agent made for itself and handed to a child as standard input or output is a terminal too, but not the
/// controlling one (the person's is), so this tells them apart: a person approving a prompt on their own terminal
/// must not have the answer written to a pty someone else holds the other end of.
pub fn is_controlling_tty(fd: i32) -> bool {
    // SAFETY: these only inspect the descriptor and the process.
    unsafe {
        libc::isatty(fd) == 1 && libc::tcgetpgrp(fd) >= 0 && libc::tcgetsid(fd) == libc::getsid(0)
    }
}

/// A terminal to ask on: standard input, or the controlling terminal.
struct Tty {
    fd: i32,
    // keeps a `/dev/tty` descriptor open, and closes it
    _own: Option<OwnedFd>,
}

fn open_tty() -> Result<Tty, ProofError> {
    // SAFETY: isatty only inspects the descriptor.
    if unsafe { libc::isatty(0) } == 1 {
        return Ok(Tty { fd: 0, _own: None });
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|_| ProofError::NoTerminal)?;
    // SAFETY: as above, on the descriptor just opened.
    if unsafe { libc::isatty(file.as_raw_fd()) } != 1 {
        return Err(ProofError::NoTerminal);
    }
    let own: OwnedFd = file.into();
    Ok(Tty {
        fd: own.as_raw_fd(),
        _own: Some(own),
    })
}

fn write_all(fd: i32, text: &str) {
    let mut bytes = text.as_bytes();
    while !bytes.is_empty() {
        // SAFETY: the pointer and length are of a live slice.
        let n = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
        if n <= 0 {
            return;
        }
        bytes = &bytes[n as usize..];
    }
}

/// Echo and signal keys off for the read, the old settings back on drop. Signals are off so that a Ctrl-C arrives
/// as a character (and aborts the read) instead of killing the process with the terminal still silent.
struct Silent {
    fd: i32,
    saved: libc::termios,
    old_actions: [libc::sigaction; SIGNALS.len()],
}

/// The signals that end the process from outside while the prompt is up (#1278). SIGKILL cannot be caught.
const SIGNALS: [i32; 4] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGQUIT];

/// The terminal settings the signal handler puts back. Written before `SIGNAL_FD` is set and read only after it is
/// seen set, so the handler never reads a half-written value; the prompt is one at a time per process.
struct SavedForSignal(std::cell::UnsafeCell<std::mem::MaybeUninit<libc::termios>>);
// SAFETY: see the ordering described above; there is no other access.
unsafe impl Sync for SavedForSignal {}
static SIGNAL_SAVED: SavedForSignal =
    SavedForSignal(std::cell::UnsafeCell::new(std::mem::MaybeUninit::uninit()));
static SIGNAL_FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Put the terminal back, then let the signal do what it would have done: default action, re-raised. Only
/// async-signal-safe calls (`tcsetattr`, `signal`, `raise`).
extern "C" fn restore_and_reraise(sig: libc::c_int) {
    use std::sync::atomic::Ordering;
    let fd = SIGNAL_FD.load(Ordering::Acquire);
    if fd >= 0 {
        // SAFETY: SIGNAL_SAVED was written before the fd was published.
        unsafe {
            libc::tcsetattr(fd, libc::TCSAFLUSH, (*SIGNAL_SAVED.0.get()).as_ptr());
            libc::signal(sig, libc::SIG_DFL);
            libc::raise(sig);
        }
    }
}

impl Silent {
    fn on(fd: i32) -> Result<Self, ProofError> {
        // SAFETY: termios is plain data; tcgetattr fills it.
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
            return Err(ProofError::NoTerminal);
        }
        let mut quiet = saved;
        quiet.c_lflag &= !(libc::ECHO | libc::ISIG);
        quiet.c_lflag |= libc::ICANON;
        // TCSAFLUSH also throws away anything typed (or injected) before this moment
        if unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &quiet) } != 0 {
            return Err(ProofError::NoTerminal);
        }
        // SAFETY: published before the handlers can run; see `SavedForSignal`.
        unsafe { (*SIGNAL_SAVED.0.get()).write(saved) };
        SIGNAL_FD.store(fd, std::sync::atomic::Ordering::Release);
        // SAFETY: sigaction takes plain structs; the handler is async-signal-safe.
        let old_actions = unsafe {
            let mut act: libc::sigaction = std::mem::zeroed();
            act.sa_sigaction = restore_and_reraise as extern "C" fn(libc::c_int) as usize;
            libc::sigemptyset(&mut act.sa_mask);
            let mut old = [std::mem::zeroed::<libc::sigaction>(); SIGNALS.len()];
            for (i, sig) in SIGNALS.iter().enumerate() {
                libc::sigaction(*sig, &act, &mut old[i]);
            }
            old
        };
        Ok(Self {
            fd,
            saved,
            old_actions,
        })
    }
}

impl Drop for Silent {
    fn drop(&mut self) {
        // SAFETY: restores the settings and the signal actions read in `on`.
        unsafe {
            for (i, sig) in SIGNALS.iter().enumerate() {
                libc::sigaction(*sig, &self.old_actions[i], std::ptr::null_mut());
            }
            SIGNAL_FD.store(-1, std::sync::atomic::Ordering::Release);
            libc::tcsetattr(self.fd, libc::TCSAFLUSH, &self.saved);
        };
    }
}

/// Read one line from the terminal. With `echo` off the typed text is not shown (a secret); on, it is (a confirmation).
/// The buffer is allocated to its largest size first, so it never reallocates and leaves a copy behind, and it is
/// wiped when dropped.
fn read_tty_line(label: &str, echo: bool) -> Result<Zeroizing<Vec<u8>>, ProofError> {
    let tty = open_tty()?;
    let mut line = Zeroizing::new(Vec::<u8>::with_capacity(MAX_LINE));
    let mut overflow = false;
    {
        let _quiet = if echo {
            None
        } else {
            Some(Silent::on(tty.fd)?)
        };
        if echo {
            // anything typed ahead, or injected, before the prompt is thrown away either way
            // SAFETY: flushes the input queue of a terminal descriptor.
            unsafe { libc::tcflush(tty.fd, libc::TCIFLUSH) };
        }
        write_all(tty.fd, &format!("{label}: "));
        loop {
            let mut b = 0u8;
            // SAFETY: reads one byte into a live local.
            let n = unsafe { libc::read(tty.fd, (&mut b as *mut u8).cast(), 1) };
            if n <= 0 {
                write_all(tty.fd, "\n");
                return Err(ProofError::Aborted);
            }
            match b {
                b'\n' => break,
                // Ctrl-C or Ctrl-D typed as a character: stop
                3 | 4 if !echo => {
                    write_all(tty.fd, "\n");
                    return Err(ProofError::Aborted);
                }
                _ if line.len() >= MAX_LINE => overflow = true,
                _ => line.push(b),
            }
        }
        if !echo {
            write_all(tty.fd, "\n");
        }
    }
    if overflow {
        return Err(ProofError::Wrong);
    }
    while line.last() == Some(&b'\r') {
        line.pop();
    }
    Ok(line)
}

impl Prompt for TtyPrompt {
    fn secret(&mut self, label: &str) -> Result<Zeroizing<String>, ProofError> {
        let mut line = read_tty_line(label, false)?;
        // the Vec moves into the String (no copy) and the String is wiped when dropped
        let text = String::from_utf8(std::mem::take(&mut *line)).map_err(|_| ProofError::Wrong)?;
        Ok(Zeroizing::new(text))
    }

    fn notice(&mut self, text: &str) {
        eprintln!("{text}");
    }

    fn line(&mut self, label: &str) -> Result<String, ProofError> {
        let line = read_tty_line(label, true)?;
        String::from_utf8(line.to_vec()).map_err(|_| ProofError::Wrong)
    }
}
