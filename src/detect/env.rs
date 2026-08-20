//! Identifying a terminal from the capability table. Pure lookups, no I/O.

use crate::{Database, Terminal};

/// How a terminal was identified. Recorded because the three routes do not carry the
/// same weight: XTVERSION comes from the terminal itself, while the environment
/// variables are inherited and survive into whatever the terminal spawns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// The terminal named itself in reply to XTVERSION.
    XtVersion,
    TermProgram,
    Term,
}

/// The terminal whose XTVERSION name matches, comparing case-insensitively because
/// the replies are not consistently cased (`iTerm2`, `ghostty`).
pub fn by_xtversion<'a>(db: &'a Database, name: &str) -> Option<&'a Terminal> {
    db.terminals
        .iter()
        .find(|t| !t.xtversion.is_empty() && t.xtversion.eq_ignore_ascii_case(name))
}

/// The terminal matching the environment, with the evidence that identified it.
///
/// `TERM_PROGRAM` is tried first: it names an application, while `TERM` names a
/// terminfo entry that another terminal may deliberately claim.
pub fn by_env<'a>(
    db: &'a Database,
    term_program: Option<&str>,
    term: Option<&str>,
) -> Option<(&'a Terminal, Evidence)> {
    if let Some(value) = term_program.filter(|v| !v.is_empty()) {
        let found = db
            .terminals
            .iter()
            .find(|t| t.term_program.iter().any(|c| c.eq_ignore_ascii_case(value)));
        if let Some(terminal) = found {
            return Some((terminal, Evidence::TermProgram));
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

    #[test]
    fn does_not_match_an_unknown_name() {
        assert!(by_xtversion(database(), "wezterm").is_none());
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
            by_env(db, Some("Apple_Terminal"), Some("xterm-256color")).unwrap();
        assert_eq!(terminal.id, "apple-terminal");
        assert_eq!(evidence, Evidence::TermProgram);
    }

    /// kitty sets no TERM_PROGRAM at all, which is why TERM has to be consulted too.
    #[test]
    fn falls_back_to_term() {
        let db = database();
        let (terminal, evidence) = by_env(db, None, Some("xterm-kitty")).unwrap();
        assert_eq!(terminal.id, "kitty");
        assert_eq!(evidence, Evidence::Term);
    }

    /// An unset variable reaches us as an empty string as readily as as `None`.
    #[test]
    fn ignores_empty_values() {
        let db = database();
        assert!(by_env(db, Some(""), Some("")).is_none());
        assert!(by_env(db, None, None).is_none());
        let (terminal, _) = by_env(db, Some(""), Some("xterm-kitty")).unwrap();
        assert_eq!(terminal.id, "kitty");
    }

    #[test]
    fn does_not_identify_an_unknown_environment() {
        assert!(by_env(database(), Some("WezTerm"), Some("wezterm")).is_none());
    }
}
