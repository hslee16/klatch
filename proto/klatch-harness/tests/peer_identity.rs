//! Experiment 14.3 (ticket #29): can a gateway that owns a Unix socket tell which process connected, and map it to
//! a coding-agent session? On macOS the answer is `getsockopt(LOCAL_PEERPID)`, on Linux `SO_PEERCRED`; both give the
//! pid of the connecting process, which the ancestry walk then maps to the session through the harness registry.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use klatch_harness::{FsRegistry, PsTable, find_session};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

unsafe extern "C" {
    fn getsockopt(
        fd: i32,
        level: i32,
        name: i32,
        value: *mut std::ffi::c_void,
        len: *mut u32,
    ) -> i32;
}

/// The pid of the process at the other end of a Unix socket, as the kernel reports it.
fn peer_pid(stream: &UnixStream) -> Option<u32> {
    #[cfg(target_os = "macos")]
    let (level, name) = (0, 0x002); // SOL_LOCAL, LOCAL_PEERPID
    #[cfg(target_os = "linux")]
    let (level, name) = (1, 17); // SOL_SOCKET, SO_PEERCRED
    let mut value = [0i32; 3]; // a pid_t on macOS; struct ucred { pid, uid, gid } on Linux
    let mut len = std::mem::size_of_val(&value) as u32;
    // SAFETY: `value` and `len` are valid for the call and `len` is the size of `value`.
    let rc = unsafe {
        getsockopt(
            stream.as_raw_fd(),
            level,
            name,
            value.as_mut_ptr().cast(),
            &mut len,
        )
    };
    (rc == 0).then_some(value[0] as u32)
}

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("klatch-peer-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A program named `claude` (a symlink to the shell, as the bench does) that runs `script`, so it is the "session"
/// and its child, the script's process, is the connecting client.
fn fake_session(dir: &Path, script: &str) -> Child {
    let claude = dir.join("claude");
    std::os::unix::fs::symlink("/bin/sh", &claude).unwrap();
    Command::new(claude)
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .spawn()
        .unwrap()
}

fn client_script(socket: &Path) -> String {
    let py = format!(
        "import socket,time;s=socket.socket(socket.AF_UNIX);s.connect('{}');time.sleep(3)",
        socket.display()
    );
    format!("python3 -c \"{py}\"; true")
}

/// The start time as the registry records it: `ps -o lstart=` in UTC with the whitespace collapsed.
fn lstart(pid: u32) -> String {
    let out = Command::new("ps")
        .env("TZ", "UTC")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn domain() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else {
        std::env::consts::OS
    }
}

#[test]
fn the_server_learns_the_pid_of_the_process_that_connected_and_maps_it_to_its_session() {
    let dir = temp("map");
    let socket = dir.join("gateway.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let mut session = fake_session(&dir, &client_script(&socket));
    let (stream, _) = listener.accept().unwrap();
    let client = peer_pid(&stream).expect("the kernel reports the peer pid");
    assert_ne!(
        client,
        session.id(),
        "the client is the session's child, not the session"
    );
    // the registry record the harness would have written for the session
    let registry = dir.join("sessions");
    std::fs::create_dir_all(&registry).unwrap();
    let record = format!(
        r#"{{"pid":{},"sessionId":"peer-session-0001","cwd":"/w","name":"peer","procStart":"{}","pidDomain":"{}"}}"#,
        session.id(),
        lstart(session.id()),
        domain()
    );
    std::fs::write(registry.join(format!("{}.json", session.id())), record).unwrap();
    let found = find_session(client, &PsTable, &FsRegistry { dir: registry }, domain())
        .expect("the connecting process maps to its session");
    assert_eq!(found.session_id, "peer-session-0001");
    assert_eq!(found.pid, session.id());
    let _ = session.kill();
    let _ = session.wait();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_process_with_no_session_above_it_maps_to_nothing() {
    let dir = temp("none");
    let socket = dir.join("gateway.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let mut client = Command::new("sh")
        .arg("-c")
        .arg(client_script(&socket))
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let (stream, _) = listener.accept().unwrap();
    let pid = peer_pid(&stream).unwrap();
    let registry = dir.join("sessions");
    std::fs::create_dir_all(&registry).unwrap();
    assert!(find_session(pid, &PsTable, &FsRegistry { dir: registry }, domain()).is_err());
    let _ = client.kill();
    let _ = client.wait();
    let _ = std::fs::remove_dir_all(dir);
}
