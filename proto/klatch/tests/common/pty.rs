//! Run a command on a real pseudo-terminal, the way a person (or an agent that opens one) would, and type at its
//! prompts (#1233). The child gets the terminal as its standard input, output and error AND as its controlling
//! terminal (`setsid` then `TIOCSCTTY`), so `/dev/tty` opens in it. Nothing is read from the environment: the
//! passphrase is typed into the terminal, which is the only way the product accepts one.
//!
//! A line is typed each time the output ends in a prompt (`: ` or `> `), after the prompt, because the terminal
//! throws away input that arrives before the program turns echo off.
#![allow(dead_code)]

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitStatus, Output, Stdio};
use std::time::{Duration, Instant};

pub struct PtyRun {
    pub status: ExitStatus,
    /// Everything the program wrote to the terminal, with carriage returns removed.
    pub text: String,
    /// What the child wrote to a PIPE standard output (only for `run_on_pty_stdout_piped`).
    pub piped_stdout: String,
}

impl PtyRun {
    pub fn ok(&self) -> bool {
        self.status.success()
    }

    /// As a plain `Output` (stdout holds the whole terminal text, stderr is empty), for the helpers that return one.
    pub fn into_output(self) -> Output {
        Output {
            status: self.status,
            stdout: self.text.into_bytes(),
            stderr: Vec::new(),
        }
    }
}

fn pty() -> (OwnedFd, OwnedFd) {
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
    assert_eq!(r, 0, "openpty failed");
    // SAFETY: both are fresh descriptors we own.
    unsafe { (OwnedFd::from_raw_fd(m), OwnedFd::from_raw_fd(s)) }
}

/// Run `cmd` on a pseudo-terminal, typing `typed` one line per prompt. Kills the child after 120 seconds.
pub fn run_on_pty(cmd: Command, typed: &[&str]) -> PtyRun {
    run_with(cmd, typed, None, false)
}

/// As `run_on_pty`, but standard OUTPUT is a pipe (`klatch admin recovery | cat`) while the terminal stays the
/// controlling terminal, standard input and error: a program that shares the human's terminal and captures what a
/// command prints (#1233).
pub fn run_on_pty_stdout_piped(cmd: Command, typed: &[&str]) -> PtyRun {
    run_with(cmd, typed, None, true)
}

/// As `run_on_pty`, but standard input is a PIPE carrying `stdin_text` while the terminal is still the child's
/// controlling terminal and its output: an agent that shares the human's terminal and pipes its own input (#1233).
pub fn run_on_pty_piped(cmd: Command, typed: &[&str], stdin_text: &str) -> PtyRun {
    run_with(cmd, typed, Some(stdin_text), false)
}

fn run_with(mut cmd: Command, typed: &[&str], piped: Option<&str>, out_piped: bool) -> PtyRun {
    let (master, slave) = pty();
    cmd.stderr(Stdio::from(slave.try_clone().unwrap()));
    if out_piped {
        cmd.stdout(Stdio::piped());
    } else {
        cmd.stdout(Stdio::from(slave.try_clone().unwrap()));
    }
    if piped.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::from(slave.try_clone().unwrap()));
    }
    // SAFETY: setsid and ioctl are async-signal-safe and touch nothing of ours. Descriptor 2 is always the terminal.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            libc::ioctl(2, libc::TIOCSCTTY as _, 0);
            Ok(())
        });
    }
    let mut child = cmd.spawn().expect("spawn on a pty");
    if let (Some(text), Some(mut stdin)) = (piped, child.stdin.take()) {
        use std::io::Write;
        let _ = stdin.write_all(text.as_bytes());
    }
    // the command holds clones of the slave until it is dropped; the master reads end only when none is left
    drop(cmd);
    drop(slave);

    let mfd = master.as_raw_fd();
    let started = Instant::now();
    let mut text = String::new();
    let mut next = 0;
    let mut answered_at = 0;
    loop {
        if started.elapsed() > Duration::from_secs(120) {
            let _ = child.kill();
            panic!("the command did not finish on its pty; so far:\n{text}");
        }
        let mut p = libc::pollfd {
            fd: mfd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one pollfd, live for the call.
        let n = unsafe { libc::poll(&mut p, 1, 100) };
        if n > 0 {
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
        } else if n < 0 {
            break;
        } else if child.try_wait().ok().flatten().is_some() {
            // exited with nothing left to read
            break;
        }
    }
    let mut piped_stdout = String::new();
    if let Some(mut out) = child.stdout.take() {
        use std::io::Read;
        let _ = out.read_to_string(&mut piped_stdout);
    }
    let status = child.wait().expect("wait");
    PtyRun {
        status,
        text,
        piped_stdout,
    }
}
