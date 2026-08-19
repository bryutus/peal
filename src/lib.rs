//! Which escape sequence to send a terminal so it raises a desktop notification.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::Deserialize;

/// The capability table, embedded at compile time so the binary stays self-contained.
const TERMINALS_TOML: &str = include_str!("../data/terminals.toml");

/// A notification dialect.
///
/// The set is closed in code rather than open in data: every dialect needs its own
/// byte-assembly routine, so a new entry in the TOML alone could never work. Adding
/// one here makes the compiler point at every place that has to handle it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sequence {
    Osc9,
    Osc777,
    Osc99,
}

impl Sequence {
    /// Every dialect, for exhaustiveness checks in tests.
    pub const ALL: [Sequence; 3] = [Sequence::Osc9, Sequence::Osc777, Sequence::Osc99];
}

/// What a dialect can express. A property of the sequence itself, not of any terminal.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    /// Human-readable wire format, for diagnostics.
    pub form: String,
    pub title: bool,
    pub body: bool,
    /// Whether a notification can be replaced or deduplicated.
    pub id: bool,
}

/// One terminal and the dialects it accepts.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Terminal {
    pub id: String,
    /// Name reported by XTVERSION, or empty when the terminal does not answer it.
    pub xtversion: String,
    pub term_program: Vec<String>,
    pub term: Vec<String>,
    /// Ordered, richest first. Resolution takes the first entry that satisfies the request.
    pub accepts: Vec<Sequence>,
    /// True when the entry was confirmed by measurement rather than transcribed from docs.
    pub verified: bool,
    #[serde(default)]
    pub tested_version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Database {
    pub sequences: BTreeMap<Sequence, Capability>,
    pub terminals: Vec<Terminal>,
}

impl Database {
    /// What the given dialect can express.
    ///
    /// Panics if the table has no entry, which the tests rule out: the data ships inside
    /// the binary, so a gap here is a bug in this crate rather than anything a caller
    /// could recover from.
    pub fn capability(&self, sequence: Sequence) -> &Capability {
        self.sequences
            .get(&sequence)
            .unwrap_or_else(|| panic!("no capability recorded for {sequence:?}"))
    }

    pub fn terminal(&self, id: &str) -> Option<&Terminal> {
        self.terminals.iter().find(|t| t.id == id)
    }
}

static DATABASE: LazyLock<Database> = LazyLock::new(|| {
    toml::from_str(TERMINALS_TOML).expect("embedded data/terminals.toml is malformed")
});

/// The embedded capability table.
pub fn database() -> &'static Database {
    &DATABASE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_data_parses() {
        let db = database();
        assert!(!db.terminals.is_empty());
        assert!(!db.sequences.is_empty());
    }

    /// Guards the split between the closed enum and the open data file: adding a variant
    /// without describing it in the TOML, or vice versa, fails here.
    #[test]
    fn every_dialect_has_a_capability() {
        let db = database();
        for sequence in Sequence::ALL {
            assert!(
                db.sequences.contains_key(&sequence),
                "data/terminals.toml describes no capability for {sequence:?}"
            );
        }
        assert_eq!(
            db.sequences.len(),
            Sequence::ALL.len(),
            "data/terminals.toml describes a dialect the enum does not list"
        );
    }

    #[test]
    fn accepted_dialects_are_described() {
        let db = database();
        for terminal in &db.terminals {
            for sequence in &terminal.accepts {
                assert!(
                    db.sequences.contains_key(sequence),
                    "{}: accepts {sequence:?} with no capability recorded",
                    terminal.id
                );
            }
        }
    }

    #[test]
    fn terminal_ids_are_unique() {
        let db = database();
        let mut seen = std::collections::BTreeSet::new();
        for terminal in &db.terminals {
            assert!(
                seen.insert(&terminal.id),
                "duplicate terminal id: {}",
                terminal.id
            );
        }
    }

    /// A terminal that answers neither XTVERSION nor an environment variable could never
    /// be identified, so an entry like that is a mistake in the data.
    #[test]
    fn every_terminal_is_identifiable() {
        let db = database();
        for terminal in &db.terminals {
            assert!(
                !terminal.xtversion.is_empty()
                    || !terminal.term_program.is_empty()
                    || !terminal.term.is_empty(),
                "{}: no way to identify this terminal",
                terminal.id
            );
        }
    }

    #[test]
    fn osc9_carries_no_title() {
        let db = database();
        assert!(!db.capability(Sequence::Osc9).title);
        assert!(db.capability(Sequence::Osc777).title);
    }
}
