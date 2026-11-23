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
    run_with(cmd, Setup::new(typed, Layout::Plain))
}

/// The terminal the human has (pty A, the child's controlling terminal, carrying standard ERROR) while standard input
/// and OUTPUT are a second pty B that the caller owns, as an agent that makes itself a pty for fd 0 and 1 without
/// `setsid` would: B's master output comes back as `piped_stdout` (#1233).
pub fn run_on_pty_split(cmd: Command, typed: &[&str]) -> PtyRun {
    run_with(cmd, Setup::new(typed, Layout::Split))
}

/// The human's terminal H is standard INPUT (the one the test types at), while the child is the leader of a session of
/// its own on a pty P that the caller owns, as an agent that forks a pty (`forkpty`) would be: standard output and
/// error are P, and P is the child's controlling terminal. P's master output comes back as `piped_stdout` (#1233).
pub fn run_on_pty_forked_session(cmd: Command, typed: &[&str]) -> PtyRun {
    run_with(cmd, Setup::new(typed, Layout::ForkedSession))
}

/// As `run_on_pty`, but standard ERROR is a pipe (the terminal is still the controlling terminal, input and output);
/// what it carried comes back as `piped_stdout` (#1233).
pub fn run_on_pty_stderr_piped(cmd: Command, typed: &[&str]) -> PtyRun {
    run_with(cmd, Setup::new(typed, Layout::StderrPiped))
}

/// As `run_on_pty`, but standard OUTPUT is a pipe (`klatch admin recovery | cat`) while the terminal stays the
/// controlling terminal, standard input and error: a program that shares the human's terminal and captures what a
/// command prints (#1233).
pub fn run_on_pty_stdout_piped(cmd: Command, typed: &[&str]) -> PtyRun {
    run_with(cmd, Setup::new(typed, Layout::Plain).stdout_piped())
}

/// As `run_on_pty`, but standard input is a PIPE carrying `stdin_text` while the terminal is still the child's
/// controlling terminal and its output: an agent that shares the human's terminal and pipes its own input (#1233).
pub fn run_on_pty_piped(cmd: Command, typed: &[&str], stdin_text: &str) -> PtyRun {
    run_with(cmd, Setup::new(typed, Layout::Plain).stdin_text(stdin_text))
}

/// Whether the output so far ends in a prompt the child is waiting at (see the module note).
///
/// `main` returning `Err` prints `Error: ` and the message as separate writes, so a read can end right after
/// `Error: ` (it ends in `: ` too). That half line is not a prompt; typing at it spends a typed line while echo is
/// on and leaves the real next prompt unanswered (#1316).
pub fn is_prompt(text: &str) -> bool {
    let last = text.rsplit('\n').next().unwrap_or("");
    (text.ends_with(": ") || text.ends_with("> ")) && !last.starts_with("Error:")
}

/// Which descriptors are the pty the test types at, and which are something else.
#[derive(Clone, Copy, PartialEq)]
enum Layout {
    /// Standard input, output and error are all the pty, which is also the controlling terminal.
    Plain,
    /// Standard input and output are a second pty B that the caller owns; error is the controlling terminal A.
    Split,
    /// Input is the pty H the test types at; output and error are a pty P of the child's own session.
    ForkedSession,
    /// Standard error is a pipe; the pty is the controlling terminal, input and output.
    StderrPiped,
}

impl Layout {
    /// The descriptor that becomes the controlling terminal: the one that is the terminal in this layout.
    fn tty_fd(self) -> i32 {
        match self {
            Layout::ForkedSession => 1,
            Layout::StderrPiped => 0,
            _ => 2,
        }
    }

    fn has_other_pty(self) -> bool {
        matches!(self, Layout::Split | Layout::ForkedSession)
    }
}

/// How a command is run on a pty: what is typed, and which of its descriptors are not the terminal.
struct Setup<'a> {
    typed: &'a [&'a str],
    stdin_text: Option<&'a str>,
    out_piped: bool,
    layout: Layout,
}

impl<'a> Setup<'a> {
    fn new(typed: &'a [&'a str], layout: Layout) -> Self {
        Setup {
            typed,
            stdin_text: None,
            out_piped: false,
            layout,
        }
    }

    fn stdout_piped(mut self) -> Self {
        self.out_piped = true;
        self
    }

    fn stdin_text(mut self, text: &'a str) -> Self {
        self.stdin_text = Some(text);
        self
    }
}

/// Point the child's standard descriptors at the terminals of this layout.
fn wire_stdio(
    cmd: &mut Command,
    setup: &Setup,
    slave: &OwnedFd,
    other: &Option<(OwnedFd, OwnedFd)>,
) {
    let clone = |fd: &OwnedFd| Stdio::from(fd.try_clone().unwrap());
    let b_slave = other.as_ref().map(|(_, s)| s);
    match (setup.layout, b_slave) {
        (Layout::ForkedSession, Some(p)) => cmd.stderr(clone(p)),
        (Layout::StderrPiped, _) => cmd.stderr(Stdio::piped()),
        _ => cmd.stderr(clone(slave)),
    };
    match (b_slave, setup.out_piped) {
        (Some(b), _) => cmd.stdout(clone(b)),
        (None, true) => cmd.stdout(Stdio::piped()),
        (None, false) => cmd.stdout(clone(slave)),
    };
    match (setup.layout, b_slave, setup.stdin_text) {
        (Layout::Split, Some(b), _) => cmd.stdin(clone(b)),
        (_, _, Some(_)) => cmd.stdin(Stdio::piped()),
        _ => cmd.stdin(clone(slave)),
    };
}

/// Types the next line each time the output ends in a prompt that is new since the last line typed.
struct Typist<'a> {
    typed: &'a [&'a str],
    next: usize,
    answered_at: usize,
}

impl Typist<'_> {
    fn has_something_to_answer(&self, text: &str) -> bool {
        let new_text = text.len() > self.answered_at;
        is_prompt(text) && new_text && self.next < self.typed.len()
    }

    fn answer(&mut self, mfd: i32, text: &str) {
        if !self.has_something_to_answer(text) {
            return;
        }
        let line = format!("{}\n", self.typed[self.next]);
        self.next += 1;
        self.answered_at = text.len();
        // SAFETY: writes a live slice to the master.
        unsafe { libc::write(mfd, line.as_ptr().cast(), line.len()) };
    }
}

/// Read once from the master, if it has anything: `None` when the terminal is done (an error or a hang up).
fn read_master(mfd: i32) -> Option<String> {
    let mut buf = [0u8; 4096];
    // SAFETY: reads into a live buffer of that length.
    let got = unsafe { libc::read(mfd, buf.as_mut_ptr().cast(), buf.len()) };
    (got > 0).then(|| String::from_utf8_lossy(&buf[..got as usize]).replace('\r', ""))
}

fn poll_in(fd: i32, wait_ms: i32) -> i32 {
    let mut p = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one pollfd, live for the call.
    unsafe { libc::poll(&mut p, 1, wait_ms) }
}

/// Read the output of the terminal until the child ends, typing at its prompts; returns the terminal text.
fn pump(
    child: &mut std::process::Child,
    terminals: (i32, &Option<OwnedFd>),
    typed: &[&str],
    piped_stdout: &mut String,
) -> String {
    let (mfd, b_master) = terminals;
    let started = Instant::now();
    let mut text = String::new();
    let mut typist = Typist {
        typed,
        next: 0,
        answered_at: 0,
    };
    loop {
        if started.elapsed() > Duration::from_secs(120) {
            let _ = child.kill();
            panic!("the command did not finish on its pty; so far:\n{text}");
        }
        // the other terminal is read as it goes (some systems drop what is unread when the child closes it)
        if let Some(b) = b_master {
            drain(b.as_raw_fd(), piped_stdout, 0);
        }
        let n = poll_in(mfd, 100);
        if n > 0 {
            let Some(got) = read_master(mfd) else { break };
            text.push_str(&got);
            typist.answer(mfd, &text);
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
    text
}

fn run_with(mut cmd: Command, setup: Setup) -> PtyRun {
    let (master, slave) = pty();
    let other = setup.layout.has_other_pty().then(pty);
    wire_stdio(&mut cmd, &setup, &slave, &other);
    let tty_fd = setup.layout.tty_fd();
    // SAFETY: setsid and ioctl are async-signal-safe and touch nothing of ours.
    unsafe {
        cmd.pre_exec(move || {
            libc::setsid();
            libc::ioctl(tty_fd, libc::TIOCSCTTY as _, 0);
            Ok(())
        });
    }
    let mut child = cmd.spawn().expect("spawn on a pty");
    if let (Some(text), Some(mut stdin)) = (setup.stdin_text, child.stdin.take()) {
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
    let mut piped_stdout = String::new();
    let text = pump(
        &mut child,
        (master.as_raw_fd(), &b_master),
        setup.typed,
        &mut piped_stdout,
    );
    if let Some(b) = &b_master {
        drain(b.as_raw_fd(), &mut piped_stdout, 200);
    }
    read_child_pipes(&mut child, &mut piped_stdout);
    let status = child.wait().expect("wait");
    PtyRun {
        status,
        text,
        piped_stdout,
    }
}

/// Whatever the child wrote to a pipe standard output or error joins `into`.
fn read_child_pipes(child: &mut std::process::Child, into: &mut String) {
    use std::io::Read;
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(into);
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(into);
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

/// Run `cmd` on a pty, wait for its first prompt, send the child `signal`, wait for it to end, and report the
/// terminal's local flags as they were left (#1278), with whether the child ended on that signal.
pub fn run_on_pty_signalled(mut cmd: Command, signal: i32) -> (bool, libc::tcflag_t) {
    let (master, slave) = pty();
    cmd.stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave.try_clone().unwrap()));
    // SAFETY: setsid and ioctl are async-signal-safe and touch nothing of ours.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            libc::ioctl(2, libc::TIOCSCTTY as _, 0);
            Ok(())
        });
    }
    let mut child = cmd.spawn().expect("spawn on a pty");
    drop(cmd);
    let mfd = master.as_raw_fd();
    let mut text = String::new();
    let started = Instant::now();
    // the prompt is up, and echo is off, once the output ends in ": "
    while !text.ends_with(": ") {
        if started.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            panic!("no prompt appeared; so far:\n{text}");
        }
        let mut p = libc::pollfd {
            fd: mfd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one pollfd, live for the call.
        if unsafe { libc::poll(&mut p, 1, 100) } > 0 {
            let mut buf = [0u8; 4096];
            // SAFETY: reads into a live buffer of that length.
            let got = unsafe { libc::read(mfd, buf.as_mut_ptr().cast(), buf.len()) };
            assert!(got > 0, "the command ended before it prompted:\n{text}");
            text.push_str(&String::from_utf8_lossy(&buf[..got as usize]));
        }
    }
    // SAFETY: kill on our own child.
    unsafe { libc::kill(child.id() as i32, signal) };
    let status = child.wait().expect("wait");
    // SAFETY: termios is plain data; tcgetattr fills it.
    let mut t: libc::termios = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::tcgetattr(mfd, &mut t) }, 0);
    use std::os::unix::process::ExitStatusExt;
    (status.signal() == Some(signal), t.c_lflag)
}
