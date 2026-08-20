//! Working out which terminal we are talking to.
//!
//! Two routes, because neither covers the field on its own: XTVERSION is the only way
//! to recognise a terminal that sets no `TERM_PROGRAM` (kitty), and the environment is
//! the only way to recognise one that does not answer XTVERSION (Terminal.app).

pub mod env;
pub mod parse;
pub mod query;

use std::time::Duration;

use crate::{Terminal, database};

pub use env::Evidence;

/// How long to wait for a reply before giving up on the query.
///
/// Generous by machine standards and imperceptible by human ones. The DA1 sentinel means
/// a terminal that answers anything at all ends the wait long before this.
const QUERY_TIMEOUT: Duration = Duration::from_millis(500);

/// What detection concluded. Deliberately more than "a terminal or nothing": the caller
/// needs to distinguish a terminal we have measured from one we are extrapolating to,
/// and both from an environment where notifying makes no sense at all.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// A terminal listed in the capability table, with the evidence that identified it.
    Known {
        terminal: &'static Terminal,
        evidence: Evidence,
        /// The version this terminal reported just now, where it reported one.
        ///
        /// Not the same thing as [`Terminal::tested_version`], which records the version
        /// the table entry was measured against and may be several releases behind what
        /// is actually running.
        version: Option<String>,
    },
    /// The terminal named itself but is absent from the table.
    ///
    /// It answers XTVERSION, which every OSC-capable terminal measured so far also does,
    /// so OSC 9 is the reasonable guess. It stays a guess: the sample behind that
    /// reasoning is four terminals on one platform, and this variant exists so callers
    /// and `doctor` can say so rather than presenting it as fact.
    UnknownButModern {
        name: String,
        version: Option<String>,
    },
    /// A terminal is attached but would not identify itself. Only the bell is safe.
    Unknown,
    /// No controlling terminal — a pipe, a cron job, CI. Nothing to notify.
    NoTty,
}

/// Identifies the terminal attached to this process.
///
/// Errors are reserved for a tty that misbehaves once opened; a missing tty and an
/// unrecognised terminal are both answers, not failures.
pub fn resolve() -> std::io::Result<Resolution> {
    let term_program = std::env::var("TERM_PROGRAM").ok();
    let term = std::env::var("TERM").ok();
    let inside_tmux = std::env::var_os("TMUX").is_some();

    let Some(mut tty) = query::open_tty()? else {
        return Ok(Resolution::NoTty);
    };

    // Inside tmux the reply describes tmux, not the terminal it is drawn in, so asking
    // would identify the wrong program. Reaching past it needs a `DCS tmux;` passthrough
    // that depends on tmux's own `allow-passthrough` setting, which is untested here.
    let reply = if inside_tmux {
        String::new()
    } else {
        query::xtversion(&mut tty, QUERY_TIMEOUT)?
    };

    Ok(resolve_from(
        parse::terminal_name(&reply),
        parse::terminal_version(&reply),
        term_program.as_deref(),
        term.as_deref(),
    ))
}

/// The decision itself, separated from the I/O that gathers its inputs so it can be
/// tested against terminals that are not installed on the machine running the tests.
pub fn resolve_from(
    xtversion_name: Option<&str>,
    xtversion_version: Option<&str>,
    term_program: Option<&str>,
    term: Option<&str>,
) -> Resolution {
    let db = database();
    let version = xtversion_version.map(str::to_owned);

    if let Some(name) = xtversion_name {
        if let Some(terminal) = env::by_xtversion(db, name) {
            return Resolution::Known {
                terminal,
                evidence: Evidence::XtVersion,
                version,
            };
        }
        // The environment still gets a say: a terminal we do not recognise by name may
        // yet be one the table knows, and a match there is firmer than the guess below.
        if let Some((terminal, evidence)) = env::by_env(db, term_program, term) {
            return Resolution::Known {
                terminal,
                evidence,
                version,
            };
        }
        return Resolution::UnknownButModern {
            name: name.to_owned(),
            version,
        };
    }

    match env::by_env(db, term_program, term) {
        // A terminal that answers no XTVERSION reports no version either; the table
        // knows what it can do, not which release is running.
        Some((terminal, evidence)) => Resolution::Known {
            terminal,
            evidence,
            version: None,
        },
        None => Resolution::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(resolution: &Resolution) -> (&str, Evidence) {
        match resolution {
            Resolution::Known {
                terminal, evidence, ..
            } => (&terminal.id, *evidence),
            other => panic!("expected a known terminal, got {other:?}"),
        }
    }

    #[test]
    fn prefers_the_terminals_own_answer() {
        // kitty run from a shell that inherited iTerm2's environment: the query wins,
        // because the variables describe whatever launched the terminal.
        let resolution = resolve_from(
            Some("kitty"),
            None,
            Some("iTerm.app"),
            Some("xterm-256color"),
        );
        assert_eq!(known(&resolution), ("kitty", Evidence::XtVersion));
    }

    #[test]
    fn falls_back_to_the_environment_when_the_terminal_stays_silent() {
        let resolution = resolve_from(None, None, Some("Apple_Terminal"), Some("xterm-256color"));
        assert_eq!(
            known(&resolution),
            ("apple-terminal", Evidence::TermProgram)
        );
    }

    /// A name we do not know does not discard an environment we do.
    #[test]
    fn an_unknown_name_still_lets_the_environment_decide() {
        let resolution = resolve_from(Some("tmux"), None, None, Some("xterm-kitty"));
        assert_eq!(known(&resolution), ("kitty", Evidence::Term));
    }

    #[test]
    fn reports_an_unlisted_terminal_as_a_guess() {
        let resolution = resolve_from(Some("Nonesuch"), None, Some("Nonesuch"), Some("nonesuch"));
        assert_eq!(
            resolution,
            Resolution::UnknownButModern {
                name: "Nonesuch".to_owned(),
                version: None,
            }
        );
    }

    /// The version travels with the name so that probe can record which release it
    /// measured, whether or not the terminal is one the table already knows.
    #[test]
    fn keeps_the_version_the_terminal_reported() {
        let listed = resolve_from(Some("ghostty"), Some("1.4.0"), None, None);
        assert!(
            matches!(listed, Resolution::Known { version: Some(ref v), .. } if v == "1.4.0"),
            "{listed:?}"
        );

        let unlisted = resolve_from(Some("Nonesuch"), Some("20260101-abc"), None, None);
        assert_eq!(
            unlisted,
            Resolution::UnknownButModern {
                name: "Nonesuch".to_owned(),
                version: Some("20260101-abc".to_owned()),
            }
        );
    }

    #[test]
    fn reports_a_terminal_that_says_nothing_at_all_as_unknown() {
        assert_eq!(resolve_from(None, None, None, None), Resolution::Unknown);
        assert_eq!(
            resolve_from(None, None, Some("Nonesuch"), Some("nonesuch")),
            Resolution::Unknown
        );
    }
}
