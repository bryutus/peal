//! Identifying a terminal from the capability table. Pure lookups, no I/O.

use crate::{Database, Terminal};

/// How a terminal was identified. Recorded because the three routes do not carry the
/// same weight: XTVERSION comes from the terminal itself, while the environment
/// variables are inherited and survive into whatever the terminal spawns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// The terminal named itself in reply to XTVERSION.
    XtVersion,
    /// The terminal named itself, but only because the question was passed through tmux
    /// to reach it. Worth telling apart: it means tmux's `allow-passthrough` is on, and
    /// so everything else peal sends will reach the terminal too.
    XtVersionThroughTmux,
    TermProgram,
    /// A variable whose presence names the terminal, carried here so the report can say
    /// which one it was. Weaker than a query and no stronger than `TERM_PROGRAM`: it is
    /// inherited by everything the terminal spawns.
    EnvMarker(&'static str),
    Term,
}

/// The terminal whose XTVERSION name matches, comparing case-insensitively because
/// the replies are not consistently cased (`iTerm2`, `ghostty`).
pub fn by_xtversion<'a>(db: &'a Database, name: &str) -> Option<&'a Terminal> {
    db.terminals
        .iter()
        .find(|t| !t.xtversion.is_empty() && t.xtversion.eq_ignore_ascii_case(name))
}

/// Every marker variable the table names, which is the set worth looking up in the
/// environment. They come from the data so that recording a new one needs no code.
pub fn marker_names(db: &'static Database) -> impl Iterator<Item = &'static str> {
    db.terminals
        .iter()
        .flat_map(|t| t.env.iter().map(String::as_str))
}

/// The terminal matching the environment, with the evidence that identified it.
///
/// Three channels in descending order of what they prove. `TERM_PROGRAM` names an
/// application. A marker variable names one too, but only terminals that set no
/// `TERM_PROGRAM` need it, so it never competes with one. `TERM` names a terminfo entry
/// that another terminal may deliberately claim, and comes last for that reason.
///
/// `present` holds the marker variables found set in the environment, by name.
pub fn by_env(
    db: &'static Database,
    term_program: Option<&str>,
    present: &[&str],
    term: Option<&str>,
) -> Option<(&'static Terminal, Evidence)> {
    if let Some(value) = term_program.filter(|v| !v.is_empty()) {
        let found = db
            .terminals
            .iter()
            .find(|t| t.term_program.iter().any(|c| c.eq_ignore_ascii_case(value)));
        if let Some(terminal) = found {
            return Some((terminal, Evidence::TermProgram));
        }
    }
    for terminal in &db.terminals {
        let found = terminal
            .env
            .iter()
            .find(|marker| present.iter().any(|p| p.eq_ignore_ascii_case(marker)));
        if let Some(marker) = found {
            return Some((terminal, Evidence::EnvMarker(marker)));
        }
    }

    let value = term.filter(|v| !v.is_empty())?;
    let terminal = db
        .terminals
        .iter()
        .find(|t| t.term.iter().any(|c| c.eq_ignore_ascii_case(value)))?;
    Some((terminal, Evidence::Term))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database;

    #[test]
    fn matches_an_xtversion_name_regardless_of_case() {
        let db = database();
        assert_eq!(by_xtversion(db, "iTerm2").map(|t| &*t.id), Some("iterm2"));
        assert_eq!(by_xtversion(db, "iterm2").map(|t| &*t.id), Some("iterm2"));
        assert_eq!(by_xtversion(db, "GHOSTTY").map(|t| &*t.id), Some("ghostty"));
    }

    /// The name is deliberately one no terminal has. Using a real unlisted terminal
    /// would make this test fail the day that terminal is measured and added.
    #[test]
    fn does_not_match_an_unknown_name() {
        assert!(by_xtversion(database(), "nonesuch").is_none());
    }

    /// Terminal.app answers no XTVERSION, so its entry records an empty name. An empty
    /// reply must not select it — that would make every silent terminal Terminal.app.
    #[test]
    fn an_empty_name_matches_nothing() {
        assert!(by_xtversion(database(), "").is_none());
    }

    #[test]
    fn identifies_apple_terminal_from_term_program() {
        let db = database();
        let (terminal, evidence) =
            by_env(db, Some("Apple_Terminal"), &[], Some("xterm-256color")).unwrap();
        assert_eq!(terminal.id, "apple-terminal");
        assert_eq!(evidence, Evidence::TermProgram);
    }

    /// kitty sets no TERM_PROGRAM at all, which is why TERM has to be consulted too.
    #[test]
    fn falls_back_to_term() {
        let db = database();
        let (terminal, evidence) = by_env(db, None, &[], Some("xterm-kitty")).unwrap();
        assert_eq!(terminal.id, "kitty");
        assert_eq!(evidence, Evidence::Term);
    }

    /// An unset variable reaches us as an empty string as readily as as `None`.
    #[test]
    fn ignores_empty_values() {
        let db = database();
        assert!(by_env(db, Some(""), &[], Some("")).is_none());
        assert!(by_env(db, None, &[], None).is_none());
        let (terminal, _) = by_env(db, Some(""), &[], Some("xterm-kitty")).unwrap();
        assert_eq!(terminal.id, "kitty");
    }

    #[test]
    fn does_not_identify_an_unknown_environment() {
        assert!(
            by_env(
                database(),
                Some("Nonesuch"),
                &["NONESUCH_SESSION"],
                Some("nonesuch")
            )
            .is_none()
        );
    }

    /// Windows Terminal sets neither variable the other two channels read, so the marker
    /// is the whole of its identification.
    #[test]
    fn identifies_a_terminal_by_a_marker_variable() {
        let db = database();
        let (terminal, evidence) =
            by_env(db, None, &["WT_SESSION"], Some("xterm-256color")).unwrap();
        assert_eq!(terminal.id, "windows-terminal");
        assert_eq!(evidence, Evidence::EnvMarker("WT_SESSION"));
    }

    /// The marker is a fallback for terminals that name themselves no other way, so a
    /// TERM_PROGRAM that the table knows still wins.
    #[test]
    fn a_named_application_outranks_a_marker() {
        let db = database();
        let (terminal, evidence) =
            by_env(db, Some("Apple_Terminal"), &["WT_SESSION"], None).unwrap();
        assert_eq!(terminal.id, "apple-terminal");
        assert_eq!(evidence, Evidence::TermProgram);
    }

    /// TERM is the weakest channel and a terminfo name can be claimed by anything, so a
    /// marker is consulted before it.
    #[test]
    fn a_marker_outranks_term() {
        let db = database();
        let (terminal, _) = by_env(db, None, &["WT_SESSION"], Some("xterm-kitty")).unwrap();
        assert_eq!(terminal.id, "windows-terminal");
    }

    #[test]
    fn marker_names_come_from_the_table() {
        assert!(marker_names(database()).any(|name| name == "WT_SESSION"));
    }
}
