//! Reading a terminal's answer to XTVERSION. Pure string work, no I/O.

/// The terminal name inside an XTVERSION reply, or `None` when the bytes are not one.
///
/// Replies arrive wrapped in `DCS > | <payload> ST`, but the payload itself is not
/// standardised: measurement showed `ghostty 1.3.1` and `kitty(0.48.2)`, so the name
/// is the leading run up to the first space or `(`.
pub fn terminal_name(reply: &str) -> Option<&str> {
    let payload = payload(reply)?;
    let name = payload.split([' ', '(']).next().unwrap_or("").trim();
    (!name.is_empty()).then_some(name)
}

/// Strips the `DCS > |` prefix and the string terminator, tolerating whatever
/// unrelated bytes the terminal may have sent before or after.
fn payload(reply: &str) -> Option<&str> {
    // DCS is ESC P, or the single byte 0x90 in 8-bit form.
    let after_dcs = match reply.find('\u{1b}') {
        Some(i) if reply[i + 1..].starts_with('P') => &reply[i + 2..],
        _ => reply.strip_prefix('\u{90}')?,
    };
    let body = after_dcs.trim_start().strip_prefix('>')?.trim_start();
    let body = body.strip_prefix('|')?;
    // ST is ESC \, or 0x9c; some terminals close with BEL instead.
    let end = body
        .find(['\u{1b}', '\u{9c}', '\u{7}'])
        .unwrap_or(body.len());
    Some(&body[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_space_separated_reply() {
        assert_eq!(terminal_name("\x1bP>|ghostty 1.3.1\x1b\\"), Some("ghostty"));
    }

    /// kitty parenthesises its version instead of separating it with a space.
    #[test]
    fn reads_a_parenthesised_reply() {
        assert_eq!(terminal_name("\x1bP>|kitty(0.48.2)\x1b\\"), Some("kitty"));
    }

    #[test]
    fn reads_a_mixed_case_reply() {
        assert_eq!(terminal_name("\x1bP>|iTerm2 3.6.11\x1b\\"), Some("iTerm2"));
    }

    #[test]
    fn accepts_bel_as_the_terminator() {
        assert_eq!(terminal_name("\x1bP>|ghostty 1.3.1\x07"), Some("ghostty"));
    }

    #[test]
    fn accepts_the_eight_bit_form() {
        assert_eq!(
            terminal_name("\u{90}>|ghostty 1.3.1\u{9c}"),
            Some("ghostty")
        );
    }

    /// A terminal that ignores XTVERSION may still have answered an earlier query,
    /// so leftover bytes in the buffer must not be mistaken for a name.
    #[test]
    fn rejects_a_reply_to_some_other_query() {
        assert_eq!(terminal_name("\x1b[?62;4c"), None);
        assert_eq!(terminal_name(""), None);
    }

    #[test]
    fn rejects_an_empty_payload() {
        assert_eq!(terminal_name("\x1bP>|\x1b\\"), None);
        assert_eq!(terminal_name("\x1bP>|   \x1b\\"), None);
    }

    /// The read may return the reply with nothing following it, terminator included.
    #[test]
    fn tolerates_a_missing_terminator() {
        assert_eq!(terminal_name("\x1bP>|ghostty 1.3.1"), Some("ghostty"));
    }
}
