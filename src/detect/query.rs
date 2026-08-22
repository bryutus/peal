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

/// Whether a sequence goes straight to the terminal or has to be handed through tmux.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Route {
    #[default]
    Direct,
    ThroughTmux,
}

/// Wraps a sequence so that tmux passes it to the terminal it is drawn in instead of
/// interpreting it.
///
/// tmux drops escape sequences it does not recognise rather than forwarding them, which
/// is every notification dialect. The wrapper is `DCS tmux; ... ST` with each ESC in the
/// payload doubled, so that the inner sequence cannot terminate the outer one.
///
/// It only works where tmux's own `allow-passthrough` is on. Where it is off the wrapped
/// sequence goes nowhere, which is the same place an unwrapped one went, so wrapping
/// costs nothing and is not worth trying to detect first.
pub fn through_tmux(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 16);
    out.extend_from_slice(b"\x1bPtmux;");
    for byte in bytes {
        if *byte == 0x1b {
            out.push(0x1b);
        }
        out.push(*byte);
    }
    out.extend_from_slice(b"\x1b\\");
    out
}

/// Sends XTVERSION and returns whatever the terminal replied, raw.
///
/// A DA1 query rides along behind it. Terminals that ignore XTVERSION still answer DA1,
/// so its reply marks the end of the conversation and spares the common case from
/// waiting out the whole timeout. The timeout remains the backstop for terminals that
/// answer neither.
pub fn xtversion(tty: &mut File, timeout: Duration, route: Route) -> io::Result<String> {
    let _raw = RawMode::enter(tty.as_raw_fd(), timeout)?;

    let query: &[u8] = b"\x1b[>0q\x1b[c";
    let query = match route {
        Route::Direct => query.to_vec(),
        Route::ThroughTmux => through_tmux(query),
    };
    tty.write_all(&query)?;
    tty.flush()?;

    // The read blocks for at most VTIME, so the loop cannot spin; the deadline only
    // bounds a terminal that keeps dribbling bytes without ever finishing its reply.
    let deadline = Instant::now() + timeout;
    let mut reply = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        match tty.read(&mut chunk) {
            // With VMIN at zero an empty read means the read timed out, not that the
            // terminal went away. It is how a terminal that ignores both queries ends up
            // here, so it is the normal way out of this loop, not a failure.
            Ok(0) => break,
            Ok(n) => reply.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
        if ends_with_da1(&reply) || Instant::now() >= deadline {
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

/// Puts the terminal in raw mode and restores the previous settings on drop, including
/// when the read below fails partway.
struct RawMode {
    fd: RawFd,
    previous: libc::termios,
}

impl RawMode {
    fn enter(fd: RawFd, read_timeout: Duration) -> io::Result<Self> {
        let mut previous = unsafe { std::mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut previous) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = previous;
        // Without ICANON the reply arrives without waiting for a newline the terminal
        // will never send; without ECHO it is not painted onto the user's screen.
        raw.c_lflag &= !(libc::ICANON | libc::ECHO);
        // The read timeout lives here rather than in a poll() call: on macOS poll()
        // reports POLLNVAL for a tty while still claiming the descriptor is ready, so it
        // returns immediately and forever. termios does the waiting reliably instead.
        // VMIN zero means "return whatever has arrived", VTIME caps how long to wait for
        // it, counted in tenths of a second and stored in a single byte.
        raw.c_cc[libc::VMIN] = 0;
        raw.c_cc[libc::VTIME] = (read_timeout.as_millis() / 100).clamp(1, 255) as u8;
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

    /// The payload's own ESC bytes are doubled so that the first of them cannot be read
    /// as the end of the wrapper.
    #[test]
    fn wraps_a_sequence_for_tmux() {
        assert_eq!(through_tmux(b"\x1b[>0q"), b"\x1bPtmux;\x1b\x1b[>0q\x1b\\");
        assert_eq!(
            through_tmux(b"\x1b]9;done\x07"),
            b"\x1bPtmux;\x1b\x1b]9;done\x07\x1b\\"
        );
    }

    #[test]
    fn wraps_a_sequence_with_no_escapes_at_all() {
        assert_eq!(through_tmux(b"\x07"), b"\x1bPtmux;\x07\x1b\\");
    }

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
