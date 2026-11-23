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

/// What waiting for the master side of a terminal came to.
enum Wait {
    Readable,
    Quiet,
    Failed,
}

/// Wait up to 100 ms for `fd` to have something to read.
fn wait_readable(fd: i32) -> Wait {
    let mut p = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one pollfd, live for the call.
    let n = unsafe { libc::poll(&mut p, 1, 100) };
    match n {
        n if n < 0 => Wait::Failed,
        0 => Wait::Quiet,
        _ => Wait::Readable,
    }
}

/// Read what is there on `fd` (without the carriage returns), or none when the terminal is closed.
fn read_chunk(fd: i32) -> Option<String> {
    let mut buf = [0u8; 4096];
    // SAFETY: reads into a live buffer of that length.
    let got = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
    if got <= 0 {
        return None;
    }
    Some(String::from_utf8_lossy(&buf[..got as usize]).replace('\r', ""))
}

/// The lines typed at the prompts of a terminal, one for each prompt.
struct Typist {
    lines: Vec<String>,
    next: usize,
    answered_at: usize,
}

impl Typist {
    /// Type the next line when `text` ends at a prompt that has not been answered yet.
    fn answer(&mut self, fd: i32, text: &str) {
        let prompting = text.ends_with(": ") || text.ends_with("> ");
        let unanswered = text.len() > self.answered_at && self.next < self.lines.len();
        if !(prompting && unanswered) {
            return;
        }
        let line = format!("{}\n", self.lines[self.next]);
        self.next += 1;
        self.answered_at = text.len();
        // SAFETY: writes a live slice to the master.
        unsafe { libc::write(fd, line.as_ptr().cast(), line.len()) };
    }
}

/// Everything the terminal showed, answering its prompts with `typed`, until it closes (or five minutes pass).
fn read_terminal(master: OwnedFd, typed: Vec<String>) -> String {
    let mfd = master.as_raw_fd();
    let started = Instant::now();
    let mut text = String::new();
    let mut typist = Typist {
        lines: typed,
        next: 0,
        answered_at: 0,
    };
    while started.elapsed() < Duration::from_secs(300) {
        match wait_readable(mfd) {
            Wait::Failed => break,
            Wait::Quiet => continue,
            Wait::Readable => {}
        }
        let Some(chunk) = read_chunk(mfd) else { break };
        text.push_str(&chunk);
        typist.answer(mfd, &text);
    }
    text
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
    let reader = std::thread::spawn(move || read_terminal(master, typed));
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

/// The terminal text without the passphrase prompt lines (they are the terminal's, not the command's output).
pub fn without_prompts(text: &str) -> String {
    text.split_inclusive('\n')
        .filter(|l| !l.starts_with("admin passphrase to approve"))
        .collect()
}
