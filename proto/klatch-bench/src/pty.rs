//! Run klatch on a real pseudo-terminal and type at its prompts (#1233). An admin or launcher command asks for the
//! admin passphrase at a terminal, never from the environment, so the bench gives each such command a terminal of its
//! own (and makes it the controlling terminal, so `/dev/tty` opens) and a thread that types the answers: one line
//! each time the output ends in a prompt (`: ` or `> `), after the prompt, because a terminal throws away input that
//! arrives before the program turns echo off.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The passphrase the bench sets for every store it makes.
pub const PASS: &str = "bench passphrase for the admin proof";

fn pty() -> Option<(OwnedFd, OwnedFd)> {
    let (mut m, mut s) = (0, 0);
    // SAFETY: openpty fills two descriptors; no termios or winsize is passed.
    let r = unsafe {
        libc::openpty(
            &mut m,
            &mut s,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if r != 0 {
        return None;
    }
    // SAFETY: both are fresh descriptors we own.
    Some(unsafe { (OwnedFd::from_raw_fd(m), OwnedFd::from_raw_fd(s)) })
}

/// Start `cmd` on a pseudo-terminal. The child and a thread that answers its prompts with `typed` and returns
/// everything the terminal showed once the child has closed it.
pub fn spawn_on_pty(
    mut cmd: Command,
    typed: Vec<String>,
) -> std::io::Result<(Child, JoinHandle<String>)> {
    let (master, slave) = pty().ok_or_else(|| std::io::Error::other("openpty failed"))?;
    cmd.stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave.try_clone()?));
    // SAFETY: setsid and ioctl are async-signal-safe and touch nothing of ours.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            libc::ioctl(0, libc::TIOCSCTTY as _, 0);
            Ok(())
        });
    }
    let child = cmd.spawn()?;
    // the command holds clones of the slave until it is dropped; the master reads end only when none is left
    drop(cmd);
    drop(slave);
    let reader = std::thread::spawn(move || {
        let mfd = master.as_raw_fd();
        let started = Instant::now();
        let mut text = String::new();
        let mut next = 0;
        let mut answered_at = 0;
        while started.elapsed() < Duration::from_secs(300) {
            let mut p = libc::pollfd {
                fd: mfd,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one pollfd, live for the call.
            let n = unsafe { libc::poll(&mut p, 1, 100) };
            if n < 0 {
                break;
            }
            if n == 0 {
                continue;
            }
            let mut buf = [0u8; 4096];
            // SAFETY: reads into a live buffer of that length.
            let got = unsafe { libc::read(mfd, buf.as_mut_ptr().cast(), buf.len()) };
            if got <= 0 {
                break;
            }
            text.push_str(&String::from_utf8_lossy(&buf[..got as usize]).replace('\r', ""));
            let prompting = text.ends_with(": ") || text.ends_with("> ");
            if prompting && text.len() > answered_at && next < typed.len() {
                let line = format!("{}\n", typed[next]);
                next += 1;
                answered_at = text.len();
                // SAFETY: writes a live slice to the master.
                unsafe { libc::write(mfd, line.as_ptr().cast(), line.len()) };
            }
        }
        text
    });
    Ok((child, reader))
}

/// Run to the end on a pseudo-terminal: whether it succeeded, and what the terminal showed.
pub fn run_on_pty(cmd: Command, typed: Vec<String>) -> (bool, String) {
    match spawn_on_pty(cmd, typed) {
        Ok((mut child, reader)) => {
            let ok = child.wait().map(|s| s.success()).unwrap_or(false);
            (ok, reader.join().unwrap_or_default())
        }
        Err(e) => (false, format!("could not start klatch: {e}")),
    }
}

/// A command running on a pseudo-terminal that has not finished: wait for it like a `Child`, and its output is
/// what the terminal showed.
pub struct TtyChild {
    child: Child,
    tty: JoinHandle<String>,
}

impl TtyChild {
    pub fn start(cmd: Command, typed: Vec<String>) -> std::io::Result<Self> {
        let (child, tty) = spawn_on_pty(cmd, typed)?;
        Ok(Self { child, tty })
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    /// Wait to the end; `stdout` holds the whole terminal text and `stderr` is empty.
    pub fn wait_with_output(mut self) -> std::io::Result<std::process::Output> {
        let status = self.child.wait()?;
        Ok(std::process::Output {
            status,
            stdout: self.tty.join().unwrap_or_default().into_bytes(),
            stderr: Vec::new(),
        })
    }
}
