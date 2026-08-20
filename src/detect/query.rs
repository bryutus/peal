//! Asking the terminal to name itself. The only part of detection that touches the tty.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::time::{Duration, Instant};

/// The controlling terminal, or `None` when the process has none — a pipe, a CI job.
///
/// `/dev/tty` rather than stdout: a caller may legitimately redirect stdout while still
/// running under a terminal, and the query has to be both written and read on the same
/// device anyway.
pub fn open_tty() -> io::Result<Option<File>> {
    match OpenOptions::new().read(true).write(true).open("/dev/tty") {
        Ok(file) => Ok(Some(file)),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
            ) =>
        {
            Ok(None)
        }
        // ENXIO is what a process with no controlling terminal gets, and it has no
        // ErrorKind of its own.
        Err(e) if e.raw_os_error() == Some(libc::ENXIO) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Sends XTVERSION and returns whatever the terminal replied, raw.
///
/// A DA1 query rides along behind it. Terminals that ignore XTVERSION still answer DA1,
/// so its reply marks the end of the conversation and spares the common case from
/// waiting out the whole timeout. The timeout remains the backstop for terminals that
/// answer neither.
pub fn xtversion(tty: &mut File, timeout: Duration) -> io::Result<String> {
    let _raw = RawMode::enter(tty.as_raw_fd())?;

    tty.write_all(b"\x1b[>0q\x1b[c")?;
    tty.flush()?;

    let deadline = Instant::now() + timeout;
    let mut reply = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || !wait_readable(tty.as_raw_fd(), remaining)? {
            break;
        }
        match tty.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => reply.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
        if ends_with_da1(&reply) {
            break;
        }
    }

    // Terminal replies are ASCII in practice; a stray byte should not cost us the reply.
    Ok(String::from_utf8_lossy(&reply).into_owned())
}

/// Whether the buffer ends in a primary device attributes reply, `CSI ? ... c`.
fn ends_with_da1(reply: &[u8]) -> bool {
    if reply.last() != Some(&b'c') {
        return false;
    }
    let head = &reply[..reply.len() - 1];
    let Some(start) = head.iter().rposition(|b| *b == 0x1b) else {
        return false;
    };
    head[start..].starts_with(b"\x1b[")
        && head[start + 2..]
            .iter()
            .all(|b| b.is_ascii_digit() || matches!(b, b';' | b'?'))
}

fn wait_readable(fd: RawFd, timeout: Duration) -> io::Result<bool> {
    let mut pollfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let millis = timeout.as_millis().min(i32::MAX as u128) as i32;
    loop {
        let ready = unsafe { libc::poll(&mut pollfd, 1, millis) };
        if ready >= 0 {
            return Ok(ready > 0);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

/// Puts the terminal in raw mode and restores the previous settings on drop, including
/// when the read below fails partway.
struct RawMode {
    fd: RawFd,
    previous: libc::termios,
}

impl RawMode {
    fn enter(fd: RawFd) -> io::Result<Self> {
        let mut previous = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut previous) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = previous;
        // Without ICANON the reply arrives without waiting for a newline the terminal
        // will never send; without ECHO it is not painted onto the user's screen.
        raw.c_lflag &= !(libc::ICANON | libc::ECHO);
        raw.c_cc[libc::VMIN] = 0;
        raw.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { fd, previous })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &self.previous) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_a_da1_reply() {
        assert!(ends_with_da1(b"\x1b[?62;4c"));
        assert!(ends_with_da1(b"\x1bP>|ghostty 1.3.1\x1b\\\x1b[?62;4c"));
    }

    #[test]
    fn does_not_mistake_other_bytes_for_da1() {
        assert!(!ends_with_da1(b""));
        assert!(!ends_with_da1(b"\x1bP>|ghostty 1.3.1\x1b\\"));
        assert!(!ends_with_da1(b"c"));
        // A truncated reply must not end the read early.
        assert!(!ends_with_da1(b"\x1b[?62;"));
    }
}
