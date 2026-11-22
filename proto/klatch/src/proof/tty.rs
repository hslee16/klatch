//! The one place a passphrase is read: a real terminal, with echo off (#1233).
//!
//! It reads standard input when that is a terminal, and otherwise the process's CONTROLLING terminal (`/dev/tty`),
//! so a person who pipes something into a command is still asked at their own terminal. It never reads the
//! environment, an argument, or standard input that is a file or a pipe: a process with no terminal at all (an
//! agent's command, a detached or double-forked process, `setsid`) has no controlling terminal and is refused.
//! A process that makes a pseudo-terminal of its own can be asked; it is then refused for not knowing the
//! passphrase, which is the point.

use super::{Prompt, ProofError};
use std::os::fd::{AsRawFd, OwnedFd};
use zeroize::Zeroizing;

/// The longest line read; a longer one is refused without being kept.
const MAX_LINE: usize = 1024;

pub struct TtyPrompt;

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
        Ok(Self { fd, saved })
    }
}

impl Drop for Silent {
    fn drop(&mut self) {
        // SAFETY: restores the settings read in `on`.
        unsafe { libc::tcsetattr(self.fd, libc::TCSAFLUSH, &self.saved) };
    }
}

impl Prompt for TtyPrompt {
    fn secret(&mut self, label: &str) -> Result<Zeroizing<String>, ProofError> {
        let tty = open_tty()?;
        let mut line = Zeroizing::new(Vec::<u8>::with_capacity(128));
        let too_long;
        {
            let _silent = Silent::on(tty.fd)?;
            write_all(tty.fd, &format!("{label}: "));
            let mut overflow = false;
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
                    3 | 4 => {
                        write_all(tty.fd, "\n");
                        return Err(ProofError::Aborted);
                    }
                    _ if line.len() >= MAX_LINE => overflow = true,
                    _ => line.push(b),
                }
            }
            too_long = overflow;
            write_all(tty.fd, "\n");
        }
        if too_long {
            return Err(ProofError::Wrong);
        }
        while line.last() == Some(&b'\r') {
            line.pop();
        }
        let text = String::from_utf8(std::mem::take(&mut *line)).map_err(|_| ProofError::Wrong)?;
        Ok(Zeroizing::new(text))
    }
}
