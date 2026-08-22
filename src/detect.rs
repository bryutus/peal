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
pub use query::Route;

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
    /// reasoning is only the terminals that have been measured, and this variant exists
    /// so callers and `doctor` can say so rather than presenting it as fact.
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

    let Some(mut tty) = query::open_tty()? else {
        return Ok(Resolution::NoTty);
    };

    // Asked plainly inside tmux, the question is answered by tmux itself, naming the
    // multiplexer rather than the terminal drawing it. Wrapped, it reaches past to the
    // real terminal — where tmux's `allow-passthrough` is on. Where it is off nothing
    // comes back, and the environment has the last word as it did before.
    let route = match inside_tmux() {
        true => Route::ThroughTmux,
        false => Route::Direct,
    };
    let reply = query::xtversion(&mut tty, QUERY_TIMEOUT, route)?;

    Ok(resolve_from(Signals {
        xtversion_name: parse::terminal_name(&reply),
        xtversion_version: parse::terminal_version(&reply),
        route,
        term_program: term_program.as_deref(),
        term: term.as_deref(),
    }))
}

/// Whether this process is running inside tmux.
///
/// Everything peal writes to the terminal has to be wrapped when it is, so this is asked
/// on the way out as well as here.
pub fn inside_tmux() -> bool {
    std::env::var_os("TMUX").is_some()
}

/// Everything the terminal and the environment said, before any of it is interpreted.
#[derive(Debug, Clone, Copy, Default)]
pub struct Signals<'a> {
    pub xtversion_name: Option<&'a str>,
    pub xtversion_version: Option<&'a str>,
    /// How the question reached the terminal, which decides what an answer proves.
    pub route: Route,
    pub term_program: Option<&'a str>,
    pub term: Option<&'a str>,
}

/// The decision itself, separated from the I/O that gathers its inputs so it can be
/// tested against terminals that are not installed on the machine running the tests.
pub fn resolve_from(signals: Signals<'_>) -> Resolution {
    let Signals {
        xtversion_name,
        xtversion_version,
        route,
        term_program,
        term,
    } = signals;

    let db = database();
    let version = xtversion_version.map(str::to_owned);
    let named_itself = match route {
        Route::Direct => Evidence::XtVersion,
        Route::ThroughTmux => Evidence::XtVersionThroughTmux,
    };

    if let Some(name) = xtversion_name {
        if let Some(terminal) = env::by_xtversion(db, name) {
            return Resolution::Known {
                terminal,
                evidence: named_itself,
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
        let resolution = resolve_from(Signals {
            xtversion_name: Some("kitty"),
            term_program: Some("iTerm.app"),
            term: Some("xterm-256color"),
            ..Signals::default()
        });
        assert_eq!(known(&resolution), ("kitty", Evidence::XtVersion));
    }

    #[test]
    fn falls_back_to_the_environment_when_the_terminal_stays_silent() {
        let resolution = resolve_from(Signals {
            term_program: Some("Apple_Terminal"),
            term: Some("xterm-256color"),
            ..Signals::default()
        });
        assert_eq!(
            known(&resolution),
            ("apple-terminal", Evidence::TermProgram)
        );
    }

    /// A name we do not know does not discard an environment we do.
    #[test]
    fn an_unknown_name_still_lets_the_environment_decide() {
        let resolution = resolve_from(Signals {
            xtversion_name: Some("tmux"),
            term: Some("xterm-kitty"),
            ..Signals::default()
        });
        assert_eq!(known(&resolution), ("kitty", Evidence::Term));
    }

    #[test]
    fn reports_an_unlisted_terminal_as_a_guess() {
        let resolution = resolve_from(Signals {
            xtversion_name: Some("Nonesuch"),
            term_program: Some("Nonesuch"),
            term: Some("nonesuch"),
            ..Signals::default()
        });
        assert_eq!(
            resolution,
            Resolution::UnknownButModern {
                name: "Nonesuch".to_owned(),
                version: None,
            }
        );
    }

    /// An answer that only arrived because it was wrapped is worth telling apart: it
    /// proves tmux will carry the notifications too.
    #[test]
    fn records_that_the_answer_came_through_tmux() {
        let resolution = resolve_from(Signals {
            xtversion_name: Some("kitty"),
            route: Route::ThroughTmux,
            ..Signals::default()
        });
        assert_eq!(
            known(&resolution),
            ("kitty", Evidence::XtVersionThroughTmux)
        );
    }

    /// tmux with allow-passthrough off: the wrapped question reaches nobody, so there is
    /// no answer, and kitty sets no TERM_PROGRAM to fall back on.
    #[test]
    fn falls_back_to_nothing_when_the_wrapper_reaches_nobody() {
        let resolution = resolve_from(Signals {
            route: Route::ThroughTmux,
            term: Some("screen-256color"),
            ..Signals::default()
        });
        assert_eq!(resolution, Resolution::Unknown);
    }

    /// The version travels with the name so that probe can record which release it
    /// measured, whether or not the terminal is one the table already knows.
    #[test]
    fn keeps_the_version_the_terminal_reported() {
        let listed = resolve_from(Signals {
            xtversion_name: Some("ghostty"),
            xtversion_version: Some("1.4.0"),
            ..Signals::default()
        });
        assert!(
            matches!(listed, Resolution::Known { version: Some(ref v), .. } if v == "1.4.0"),
            "{listed:?}"
        );

        let unlisted = resolve_from(Signals {
            xtversion_name: Some("Nonesuch"),
            xtversion_version: Some("20260101-abc"),
            ..Signals::default()
        });
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
        assert_eq!(
            resolve_from(Signals {
                ..Signals::default()
            }),
            Resolution::Unknown
        );
        assert_eq!(
            resolve_from(Signals {
                term_program: Some("Nonesuch"),
                term: Some("nonesuch"),
                ..Signals::default()
            }),
            Resolution::Unknown
        );
    }
}
