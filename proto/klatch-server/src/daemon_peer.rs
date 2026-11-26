//! Who is on the other end of a daemon connection (#102): the kernel's peer credentials. Split from `daemon.rs` (#1633).

use klatch::gateway::PeerEvidence;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

/// What the kernel reports about the process at the other end of a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerCred {
    pub pid: u32,
    pub uid: u32,
}

/// The evidence a local peer gives, or why it is refused: its uid must be the daemon's own. The pid goes into the
/// evidence and nowhere else; it is never a name.
pub fn local_evidence(peer: PeerCred, own_uid: u32) -> Result<PeerEvidence, WrongUid> {
    if peer.uid != own_uid {
        return Err(WrongUid);
    }
    Ok(PeerEvidence::Process {
        pid: peer.pid,
        uid: peer.uid,
    })
}

/// A peer that runs as another user.
#[derive(Debug, PartialEq, Eq)]
pub struct WrongUid;

#[cfg(target_os = "linux")]
pub(super) fn peer_cred(stream: &UnixStream) -> io::Result<PeerCred> {
    let mut c: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let r = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut c as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(PeerCred {
        pid: c.pid as u32,
        uid: c.uid,
    })
}

#[cfg(target_os = "macos")]
pub(super) fn peer_cred(stream: &UnixStream) -> io::Result<PeerCred> {
    let fd = stream.as_raw_fd();
    let (mut uid, mut gid): (libc::uid_t, libc::gid_t) = (0, 0);
    if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    let r = unsafe {
        libc::getsockopt(
            fd,
            0, // SOL_LOCAL
            libc::LOCAL_PEERPID,
            (&mut pid as *mut libc::pid_t).cast(),
            &mut len,
        )
    };
    // the pid is for the record only: a peer that has gone since it connected has none
    Ok(PeerCred {
        pid: if r == 0 { pid as u32 } else { 0 },
        uid,
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) fn peer_cred(_stream: &UnixStream) -> io::Result<PeerCred> {
    Err(io::Error::other(
        "no peer credentials on this system; the daemon cannot tell who connects",
    ))
}
