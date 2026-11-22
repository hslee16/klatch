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
pub fn run_on_pty(mut cmd: Command, typed: &[&str]) -> PtyRun {
    let (master, slave) = pty();
    cmd.stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave.try_clone().unwrap()));
    // SAFETY: setsid and ioctl are async-signal-safe and touch nothing of ours.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            libc::ioctl(0, libc::TIOCSCTTY as _, 0);
            Ok(())
        });
    }
    let mut child = cmd.spawn().expect("spawn on a pty");
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
    let status = child.wait().expect("wait");
    PtyRun { status, text }
}
