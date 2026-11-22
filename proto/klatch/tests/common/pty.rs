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
    run_with(cmd, typed, None, false, 0)
}

/// The terminal the human has (pty A, the child's controlling terminal, carrying standard ERROR) while standard input
/// and OUTPUT are a second pty B that the caller owns, as an agent that makes itself a pty for fd 0 and 1 without
/// `setsid` would: B's master output comes back as `piped_stdout` (#1233).
pub fn run_on_pty_split(cmd: Command, typed: &[&str]) -> PtyRun {
    run_with(cmd, typed, None, false, 1)
}

/// The human's terminal H is standard INPUT (the one the test types at), while the child is the leader of a session of
/// its own on a pty P that the caller owns, as an agent that forks a pty (`forkpty`) would be: standard output and
/// error are P, and P is the child's controlling terminal. P's master output comes back as `piped_stdout` (#1233).
pub fn run_on_pty_forked_session(cmd: Command, typed: &[&str]) -> PtyRun {
    run_with(cmd, typed, None, false, 2)
}

/// As `run_on_pty`, but standard ERROR is a pipe (the terminal is still the controlling terminal, input and output);
/// what it carried comes back as `piped_stdout` (#1233).
pub fn run_on_pty_stderr_piped(cmd: Command, typed: &[&str]) -> PtyRun {
    run_with(cmd, typed, None, false, 3)
}

/// As `run_on_pty`, but standard OUTPUT is a pipe (`klatch admin recovery | cat`) while the terminal stays the
/// controlling terminal, standard input and error: a program that shares the human's terminal and captures what a
/// command prints (#1233).
pub fn run_on_pty_stdout_piped(cmd: Command, typed: &[&str]) -> PtyRun {
    run_with(cmd, typed, None, true, 0)
}

/// As `run_on_pty`, but standard input is a PIPE carrying `stdin_text` while the terminal is still the child's
/// controlling terminal and its output: an agent that shares the human's terminal and pipes its own input (#1233).
pub fn run_on_pty_piped(cmd: Command, typed: &[&str], stdin_text: &str) -> PtyRun {
    run_with(cmd, typed, Some(stdin_text), false, 0)
}

fn run_with(
    mut cmd: Command,
    typed: &[&str],
    piped: Option<&str>,
    out_piped: bool,
    layout: u8,
) -> PtyRun {
    let (master, slave) = pty();
    let other = (layout == 1 || layout == 2).then(pty);
    match (layout, &other) {
        (2, Some((_, p))) => cmd.stderr(Stdio::from(p.try_clone().unwrap())),
        (3, _) => cmd.stderr(Stdio::piped()),
        _ => cmd.stderr(Stdio::from(slave.try_clone().unwrap())),
    };
    if let Some((_, b_slave)) = &other {
        cmd.stdout(Stdio::from(b_slave.try_clone().unwrap()));
    } else if out_piped {
        cmd.stdout(Stdio::piped());
    } else {
        cmd.stdout(Stdio::from(slave.try_clone().unwrap()));
    }
    if let (1, Some((_, b_slave))) = (layout, &other) {
        cmd.stdin(Stdio::from(b_slave.try_clone().unwrap()));
    } else if piped.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::from(slave.try_clone().unwrap()));
    }
    // the descriptor that becomes the controlling terminal: the one that is the terminal in this layout
    let tty_fd: i32 = match layout {
        2 => 1,
        3 => 0,
        _ => 2,
    };
    // SAFETY: setsid and ioctl are async-signal-safe and touch nothing of ours.
    unsafe {
        cmd.pre_exec(move || {
            libc::setsid();
            libc::ioctl(tty_fd, libc::TIOCSCTTY as _, 0);
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
    // The parent keeps its own slave descriptors open until everything is read (#1298). When the child exits and the
    // LAST slave descriptor closes, a pseudo-terminal master can lose output the child wrote just before it exited
    // (seen on macOS under load: the whole text came back empty). With one slave held here the master never hangs
    // up, so the end is the child's exit and the output is then drained to the end, not a race with the close.
    let _held_slave = slave;
    let (b_master, _held_b_slave) = match other {
        Some((m, s)) => (Some(m), Some(s)),
        None => (None, None),
    };

    let mfd = master.as_raw_fd();
    let mut piped_stdout = String::new();
    let started = Instant::now();
    let mut text = String::new();
    let mut next = 0;
    let mut answered_at = 0;
    loop {
        if started.elapsed() > Duration::from_secs(120) {
            let _ = child.kill();
            panic!("the command did not finish on its pty; so far:\n{text}");
        }
        // the other terminal is read as it goes (some systems drop what is unread when the child closes it)
        if let Some(b) = &b_master {
            drain(b.as_raw_fd(), &mut piped_stdout, 0);
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
            // The child has exited. Whatever it wrote is already in the master (the slave is held open), so drain
            // it to the end; a prompt in it can no longer be answered.
            drain(mfd, &mut text, 0);
            text = text.replace('\r', "");
            break;
        }
    }
    if let Some(b) = &b_master {
        drain(b.as_raw_fd(), &mut piped_stdout, 200);
    }
    if let Some(mut out) = child.stdout.take() {
        use std::io::Read;
        let _ = out.read_to_string(&mut piped_stdout);
    }
    if let Some(mut err) = child.stderr.take() {
        use std::io::Read;
        let _ = err.read_to_string(&mut piped_stdout);
    }
    let status = child.wait().expect("wait");
    PtyRun {
        status,
        text,
        piped_stdout,
    }
}

/// Read what is waiting on a master descriptor into `into`, waiting up to `wait_ms` for the first byte.
fn drain(fd: i32, into: &mut String, wait_ms: i32) {
    let mut wait = wait_ms;
    loop {
        let mut p = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one pollfd, live for the call.
        if unsafe { libc::poll(&mut p, 1, wait) } <= 0 {
            return;
        }
        wait = 0;
        let mut buf = [0u8; 4096];
        // SAFETY: reads into a live buffer of that length.
        let got = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if got <= 0 {
            return;
        }
        into.push_str(&String::from_utf8_lossy(&buf[..got as usize]));
    }
}
