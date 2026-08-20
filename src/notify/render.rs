//! Turning a notification into the bytes a dialect expects. Pure, so every dialect can
//! be checked byte for byte without a terminal.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::Sequence;

/// An identifier no other notification will reuse.
///
/// OSC 99 sends a title and a body as two separate escapes and joins them into one
/// notification by their shared id, so an id is unavoidable even when the caller named
/// none. That id has to differ every time: kitty replaces a notification arriving under
/// an id it has already seen, and replacing is not what an unnamed notification should
/// do.
fn anonymous_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("peal-{}-{serial}", std::process::id())
}

/// The caller's name for a notification, encoded so it cannot be read as anything but a
/// name.
///
/// Metadata fields are separated by `:` and closed by `;`, so a name containing either
/// would rewrite the escape around it. Encoding sidesteps the question entirely and
/// leaves the caller free to use whatever names its own code already has — a path, a job
/// name, a command line. The URL alphabet, unpadded, because `+`, `/` and `=` have no
/// attested meaning inside this field.
fn encode_id(id: &str) -> String {
    base64(id.as_bytes())
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_")
}

/// Text with anything that could steer the terminal removed.
///
/// A notification body is often a fragment of someone else's output — a build log, a
/// command's last line — so it has to be assumed hostile. Control characters would end
/// the escape sequence early and leave the rest to be executed as commands. Newlines and
/// tabs become spaces because a notification is a line of text, not a document.
pub fn sanitize(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect();
    cleaned.trim().to_owned()
}

/// The bytes that ask the terminal to raise this notification.
///
/// `title` and `id` are only honoured by dialects that can carry them; the caller is
/// expected to have folded the title into the body and given up on the id otherwise.
/// Title and body are sanitised here rather than at the boundary so no route to the wire
/// can skip it.
pub fn bytes(sequence: Sequence, title: Option<&str>, body: &str, id: Option<&str>) -> Vec<u8> {
    let title = title.map(sanitize).filter(|t| !t.is_empty());
    let body = sanitize(body);

    match sequence {
        // OSC 9 ; <body> BEL
        Sequence::Osc9 => format!("\x1b]9;{body}\x07").into_bytes(),

        // OSC 777 ; notify ; <title> ; <body> BEL
        //
        // With no title the body takes the title's place: the field is what every
        // implementation displays most prominently, and a notification whose loudest
        // line is empty reads as broken.
        //
        // No terminal in the table reaches this: an untitled notification is sent as
        // OSC 9 wherever OSC 9 is accepted, which so far is everywhere OSC 777 is. The
        // shape is therefore unmeasured — Ghostty ignores this two-field form, and the
        // alternative of a blank third field is no more attested — so it stays at the
        // shorter of the two until a terminal exists to measure it on.
        Sequence::Osc777 => match &title {
            Some(title) => format!("\x1b]777;notify;{title};{body}\x07").into_bytes(),
            None => format!("\x1b]777;notify;{body}\x07").into_bytes(),
        },

        // OSC 99 ; <metadata> ; <payload> ST, once per field.
        //
        // The payload is base64 so that no byte of it can be read as a metadata
        // separator, and `d=0` on the first chunk tells the terminal to hold the
        // notification until the second one arrives.
        Sequence::Osc99 => {
            let id = id.map_or_else(anonymous_id, encode_id);
            match &title {
                Some(title) => {
                    let mut out = osc99_chunk(&id, "d=0:p=title", title);
                    out.extend_from_slice(&osc99_chunk(&id, "d=1:p=body", &body));
                    out
                }
                // `p` defaults to title, which is the field to use when there is only one.
                None => osc99_chunk(&id, "d=1", &body),
            }
        }
    }
}

fn osc99_chunk(id: &str, metadata: &str, payload: &str) -> Vec<u8> {
    format!(
        "\x1b]99;i={id}:{metadata}:e=1;{}\x1b\\",
        base64(payload.as_bytes())
    )
    .into_bytes()
}

/// Standard base64, written out rather than pulled in: it is the only encoding this
/// crate needs and it is shorter than the dependency would be.
fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let bits = u32::from(chunk[0]) << 16
            | u32::from(chunk.get(1).copied().unwrap_or(0)) << 8
            | u32::from(chunk.get(2).copied().unwrap_or(0));
        for (position, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            // Each output character covers six input bits; those past the end of a short
            // chunk are padding rather than data.
            out.push(if position > chunk.len() {
                '='
            } else {
                ALPHABET[(bits >> shift & 63) as usize] as char
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_escapes_that_would_end_the_sequence_early() {
        assert_eq!(sanitize("done\x07; rm -rf /"), "done; rm -rf /");
        assert_eq!(sanitize("done\x1b]9;gotcha\x07"), "done]9;gotcha");
    }

    #[test]
    fn folds_whitespace_onto_one_line() {
        assert_eq!(sanitize("build\nfailed\ttwice"), "build failed twice");
        assert_eq!(sanitize("  padded  "), "padded");
    }

    /// C1 controls are a second way to open an escape sequence, in a single byte.
    #[test]
    fn strips_eight_bit_controls() {
        assert_eq!(sanitize("done\u{9b}5m"), "done5m");
    }

    #[test]
    fn keeps_text_that_only_looks_alarming() {
        assert_eq!(sanitize("100% done — ✓ 日本語"), "100% done — ✓ 日本語");
    }

    #[test]
    fn writes_osc9() {
        assert_eq!(
            bytes(Sequence::Osc9, None, "build finished", None),
            b"\x1b]9;build finished\x07"
        );
    }

    /// OSC 9 carries no title, so a caller that failed to fold one in loses it here
    /// rather than corrupting the sequence with an extra field.
    #[test]
    fn osc9_ignores_a_title() {
        assert_eq!(
            bytes(Sequence::Osc9, Some("peal"), "build finished", None),
            b"\x1b]9;build finished\x07"
        );
    }

    #[test]
    fn writes_osc777() {
        assert_eq!(
            bytes(Sequence::Osc777, Some("peal"), "build finished", None),
            b"\x1b]777;notify;peal;build finished\x07"
        );
    }

    /// Unreachable through `notify`, which sends an untitled notification as OSC 9, and
    /// unmeasured for that reason. Pinned so the shape is at least deliberate.
    #[test]
    fn osc777_without_a_title_sends_the_body_as_the_title() {
        assert_eq!(
            bytes(Sequence::Osc777, None, "build finished", None),
            b"\x1b]777;notify;build finished\x07"
        );
    }

    #[test]
    fn writes_osc99_as_two_chunks_sharing_an_id() {
        let sent = String::from_utf8(bytes(Sequence::Osc99, Some("peal"), "done", None)).unwrap();
        let id = osc99_id_of(&sent);
        assert_eq!(
            sent,
            format!(
                "\x1b]99;i={id}:d=0:p=title:e=1;cGVhbA==\x1b\\\x1b]99;i={id}:d=1:p=body:e=1;ZG9uZQ==\x1b\\"
            )
        );
    }

    #[test]
    fn writes_osc99_without_a_title_as_one_chunk() {
        let sent = String::from_utf8(bytes(Sequence::Osc99, None, "done", None)).unwrap();
        let id = osc99_id_of(&sent);
        assert_eq!(sent, format!("\x1b]99;i={id}:d=1:e=1;ZG9uZQ==\x1b\\"));
    }

    /// kitty replaces a notification whose id it has seen before, so an unnamed one must
    /// never reuse an id or every notification would erase the last.
    #[test]
    fn each_anonymous_osc99_notification_gets_its_own_id() {
        let first = bytes(Sequence::Osc99, None, "done", None);
        let second = bytes(Sequence::Osc99, None, "done", None);
        assert_ne!(
            osc99_id_of(&String::from_utf8(first).unwrap()),
            osc99_id_of(&String::from_utf8(second).unwrap())
        );
    }

    /// The id the escape actually carries, so the tests above can assert on the rest of
    /// the bytes without pinning a value that must change every time.
    fn osc99_id_of(sent: &str) -> String {
        let after = sent.split_once("i=").expect("an id").1;
        after
            .split_once(':')
            .expect("a metadata separator")
            .0
            .to_owned()
    }

    /// An empty title is the same as none: sending the field empty would show a blank
    /// heading above the body.
    #[test]
    fn an_empty_title_is_no_title() {
        assert_eq!(
            bytes(Sequence::Osc777, Some("  "), "done", None),
            bytes(Sequence::Osc777, None, "done", None)
        );
    }

    /// A named notification keeps its name across sends, which is what makes the second
    /// one replace the first.
    #[test]
    fn a_named_osc99_notification_reuses_its_id() {
        let first =
            String::from_utf8(bytes(Sequence::Osc99, None, "first", Some("build"))).unwrap();
        let second =
            String::from_utf8(bytes(Sequence::Osc99, None, "second", Some("build"))).unwrap();
        let other =
            String::from_utf8(bytes(Sequence::Osc99, None, "first", Some("deploy"))).unwrap();
        assert_eq!(osc99_id_of(&first), osc99_id_of(&second));
        assert_ne!(osc99_id_of(&first), osc99_id_of(&other));
    }

    /// `:` ends a metadata field and `;` ends the metadata, so a name containing either
    /// would rewrite the escape around it if it went out as written.
    #[test]
    fn an_id_cannot_break_out_of_its_field() {
        let sent =
            String::from_utf8(bytes(Sequence::Osc99, None, "done", Some("a:b;p=body"))).unwrap();
        let id = osc99_id_of(&sent);
        assert!(!id.contains(':'), "{id}");
        assert!(!id.contains(';'), "{id}");
        assert_eq!(sent, format!("\x1b]99;i={id}:d=1:e=1;ZG9uZQ==\x1b\\"));
    }

    /// Unpadded, and using the two characters the URL alphabet substitutes.
    #[test]
    fn encodes_an_id_with_the_url_alphabet() {
        assert_eq!(encode_id(""), "");
        assert_eq!(encode_id("build"), "YnVpbGQ");
        assert_eq!(encode_id("~~~"), "fn5-");
        assert_eq!(encode_id("~~?"), "fn4_");
    }

    #[test]
    fn encodes_base64_with_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
        assert_eq!(base64(b"abcd"), "YWJjZA==");
        assert_eq!(base64("日本語".as_bytes()), "5pel5pys6Kqe");
    }
}
