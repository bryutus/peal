//! Measuring a terminal that the table does not describe, or does not describe rightly.
//!
//! The inverse of [`crate::doctor`]. doctor reads the table and explains it; probe
//! ignores the table and produces one, by sending each dialect in turn and asking the
//! only instrument that can tell whether a notification appeared — the person sitting
//! in front of it.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};

use crate::detect::{self, Resolution};
use crate::notify::render;
use crate::{Sequence, database};

/// What one run of probe found out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measurement {
    /// The name the terminal gave for itself, if it answers XTVERSION at all.
    pub xtversion: Option<String>,
    pub version: Option<String>,
    pub term_program: Option<String>,
    pub term: Option<String>,
    /// Every dialect that raised a notification, richest first.
    pub accepts: Vec<Sequence>,
    /// The operating system it was measured on, which the table does not record because
    /// every entry in it so far came from the same one.
    pub os: &'static str,
    /// Whether tmux stood between peal and the terminal.
    ///
    /// The dialects are wrapped to reach past it, so the answers should be the same
    /// either way — but "should be" is what a measurement is for, and a reader comparing
    /// two entries that disagree will want to know this about them.
    pub through_tmux: bool,
}

impl Measurement {
    /// The id this terminal should go by, derived the way the existing entries were.
    pub fn id(&self) -> String {
        let source = self
            .xtversion
            .as_deref()
            .or(self.term_program.as_deref())
            .or(self.term.as_deref())
            .unwrap_or("unknown");
        source
            .to_lowercase()
            .replace('_', "-")
            .trim_end_matches(".app")
            .to_owned()
    }
}

/// The entry to paste into `data/terminals.toml`, laid out the way the file already
/// looks so it can go straight in.
pub fn entry(measurement: &Measurement) -> String {
    let quoted = |value: Option<&String>| match value {
        Some(value) => format!("[\"{value}\"]"),
        None => "[]".to_owned(),
    };
    let accepts: Vec<String> = measurement
        .accepts
        .iter()
        .map(|sequence| format!("\"{}\"", sequence.key()))
        .collect();

    let mut out = match measurement.through_tmux {
        true => format!("# Measured on {}, through tmux\n", measurement.os),
        false => format!("# Measured on {}\n", measurement.os),
    };
    out.push_str("[[terminals]]\n");
    let _ = writeln!(out, "id             = \"{}\"", measurement.id());
    let _ = writeln!(
        out,
        "xtversion      = \"{}\"",
        measurement.xtversion.as_deref().unwrap_or("")
    );
    let _ = writeln!(
        out,
        "term_program   = {}",
        quoted(measurement.term_program.as_ref())
    );
    let _ = writeln!(
        out,
        "term           = {}",
        quoted(measurement.term.as_ref())
    );
    let _ = writeln!(out, "accepts        = [{}]", accepts.join(", "));
    out.push_str("verified       = true\n");
    if let Some(version) = &measurement.version {
        let _ = writeln!(out, "tested_version = \"{version}\"");
    }
    out
}

/// How the measurement stands against what the table already claims.
pub fn against_the_table(measurement: &Measurement) -> String {
    let id = measurement.id();
    let Some(known) = database().terminal(&id) else {
        return "  This terminal is not in the table yet. The entry below is what it should say.\n"
            .to_owned();
    };

    if known.accepts == measurement.accepts {
        return format!(
            "  Already in the table as \"{id}\", and this run agrees with it. Nothing to do,\n  \
             unless the version differs from the one recorded there.\n"
        );
    }
    format!(
        "  Already in the table as \"{id}\", and this run disagrees with it.\n\n    \
         the table says   {}\n    this run found   {}\n\n  \
         Either the terminal changed between versions or one of the two measurements is\n  \
         wrong. Worth saying which version you are on when you report it.\n",
        list(&known.accepts),
        list(&measurement.accepts),
    )
}

fn list(sequences: &[Sequence]) -> String {
    if sequences.is_empty() {
        return "nothing".to_owned();
    }
    sequences
        .iter()
        .map(Sequence::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The warning for a run where nothing appeared at all.
///
/// Both explanations look identical from here: a terminal that raises no notifications
/// and a terminal that is not permitted to. The entry is still printed, since the first
/// is a real answer worth recording, but it cannot be trusted until the second is ruled
/// out.
pub fn nothing_appeared() -> String {
    let permission = if cfg!(target_os = "macos") {
        "System Settings > Notifications"
    } else {
        "the notification settings of your desktop environment"
    };
    format!(
        "  Nothing appeared for any dialect.\n\n  \
         That is a real answer for some terminals — Terminal.app raises no notifications\n  \
         by any sequence — but it is also exactly what a terminal looks like when it is\n  \
         not permitted to post notifications. Check {permission} and find\n  \
         this terminal before reporting the entry below; a newly installed terminal\n  \
         starts out denied.\n"
    )
}

/// TERM values that name a set of capabilities rather than a terminal.
///
/// Recording one as a way to identify a terminal would match every other terminal that
/// claims the same set, which is most of them. A probe leaves them out even though the
/// terminal really did set one — this is why the entries for iTerm2 and Terminal.app
/// have an empty `term` despite both setting `xterm-256color`.
const GENERIC_TERMS: [&str; 14] = [
    "ansi",
    "dumb",
    "linux",
    "screen",
    "screen-256color",
    "tmux",
    "tmux-256color",
    "vt100",
    "vt220",
    "xterm",
    "xterm-16color",
    "xterm-256color",
    "xterm-color",
    "xterm-new",
];

fn identifying_term(term: Option<String>) -> Option<String> {
    term.filter(|term| !GENERIC_TERMS.contains(&term.as_str()))
}

/// Dialects to try, most expressive first, which is also the order they belong in
/// `accepts`.
fn richest_first() -> Vec<Sequence> {
    let mut sequences = Sequence::ALL.to_vec();
    sequences.sort_by_key(|sequence| {
        let capability = database().capability(*sequence);
        let fields = usize::from(capability.title)
            + usize::from(capability.body)
            + usize::from(capability.id);
        std::cmp::Reverse(fields)
    });
    sequences
}

/// Runs the measurement, asking after each dialect whether anything appeared.
pub fn run() -> io::Result<String> {
    let resolution = detect::resolve()?;
    if resolution == Resolution::NoTty {
        return Err(io::Error::other(
            "probe needs a terminal to measure; this process has none",
        ));
    }
    let Some(mut tty) = detect::query::open_tty()? else {
        return Err(io::Error::other("probe needs a terminal to measure"));
    };
    let mut answers = BufReader::new(tty.try_clone()?);

    // The version comes from what the terminal said just now, never from the table: a
    // measurement records the release it was taken against, and the table's own figure
    // may be several releases behind what is running.
    let (xtversion, version) = match &resolution {
        Resolution::Known {
            terminal, version, ..
        } => (
            (!terminal.xtversion.is_empty()).then(|| terminal.xtversion.clone()),
            version.clone(),
        ),
        Resolution::UnknownButModern { name, version } => (Some(name.clone()), version.clone()),
        _ => (None, None),
    };

    let dialects = richest_first();
    writeln!(
        tty,
        "\nSending {} dialects, one at a time. Say whether a notification appeared.\n",
        dialects.len()
    )?;

    let mut accepts = Vec::new();
    for (index, sequence) in dialects.iter().enumerate() {
        let bytes = render::bytes(
            *sequence,
            Some("peal probe"),
            &format!(
                "{sequence} — this is dialect {} of {}",
                index + 1,
                dialects.len()
            ),
            None,
        );
        let bytes = match detect::inside_tmux() {
            true => detect::query::through_tmux(&bytes),
            false => bytes,
        };
        tty.write_all(&bytes)?;
        tty.flush()?;

        let question = format!(
            "  [{}/{}] Sent {sequence}. Did a notification appear?",
            index + 1,
            dialects.len()
        );
        if ask(&mut tty, &mut answers, &question)? {
            accepts.push(*sequence);
        }
    }

    // Under tmux both variables describe the multiplexer, not the terminal beyond it:
    // TERM_PROGRAM is overwritten with "tmux" and TERM with one of its own. Recording
    // either would make every terminal running under tmux look like this one.
    let (term_program, term) = match detect::inside_tmux() {
        true => (None, None),
        false => (
            nonempty(std::env::var("TERM_PROGRAM").ok()),
            identifying_term(nonempty(std::env::var("TERM").ok())),
        ),
    };

    let measurement = Measurement {
        xtversion,
        version,
        term_program,
        term,
        accepts,
        os: std::env::consts::OS,
        through_tmux: detect::inside_tmux(),
    };

    let mut out = String::from("\n");
    if measurement.accepts.is_empty() {
        out.push_str(&nothing_appeared());
        out.push('\n');
    }
    if let Some(note) = identification_note(&measurement) {
        out.push_str(&note);
        out.push('\n');
    }
    out.push_str(&against_the_table(&measurement));
    out.push('\n');
    out.push_str(&entry(&measurement));
    Ok(out)
}

fn nonempty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

/// What the entry had to leave out, and what that costs.
fn identification_note(measurement: &Measurement) -> Option<String> {
    let identifiable = measurement.xtversion.is_some()
        || measurement.term_program.is_some()
        || measurement.term.is_some();
    if !identifiable {
        return Some(
            "  This terminal cannot be identified. It gave no name of its own, and the\n\
             \x20 environment holds nothing that names it either, so peal has no way to\n\
             \x20 tell it from any other terminal. The entry below is a measurement, not\n\
             \x20 something that can be acted on yet.\n"
                .to_owned(),
        );
    }

    // Under tmux the environment describes tmux, so the name the terminal gave is the
    // only field left that identifies it. The dialects are still measured through to the
    // real terminal, which is what the entry is mostly for.
    if measurement.through_tmux {
        return Some(
            "  Measured through tmux, which overwrites TERM_PROGRAM and TERM with its\n\
             \x20 own. Neither names the terminal beyond it, so both are left out and the\n\
             \x20 name the terminal gave is all that identifies it here. An entry measured\n\
             \x20 outside tmux would carry more; the dialects below are the same either\n\
             \x20 way, the wrapper reaching the real terminal.\n"
                .to_owned(),
        );
    }

    let dropped = std::env::var("TERM")
        .ok()
        .filter(|term| !term.is_empty())
        .filter(|term| GENERIC_TERMS.contains(&term.as_str()))?;
    Some(format!(
        "  TERM is \"{dropped}\", which names a set of capabilities rather than this\n\
         \x20 terminal, so it is left out of the entry. Recording it would match every\n\
         \x20 other terminal that claims the same set.\n"
    ))
}

/// Asks until the answer is one of the two it can use. A mistyped answer here would
/// otherwise be recorded as a fact about the terminal.
fn ask(tty: &mut File, answers: &mut impl BufRead, question: &str) -> io::Result<bool> {
    loop {
        write!(tty, "{question} [y/n] ")?;
        tty.flush()?;

        let mut answer = String::new();
        if answers.read_line(&mut answer)? == 0 {
            return Err(io::Error::other("no answer given"));
        }
        match answer.trim().to_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => writeln!(tty, "  Please answer y or n.")?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured(accepts: Vec<Sequence>) -> Measurement {
        Measurement {
            xtversion: Some("ghostty".to_owned()),
            version: Some("1.3.1".to_owned()),
            term_program: Some("ghostty".to_owned()),
            term: Some("xterm-ghostty".to_owned()),
            accepts,
            os: "macos",
            through_tmux: false,
        }
    }

    /// The entry has to parse as part of the file it is going into, or pasting it in is
    /// the reader's problem to debug rather than probe's to get right.
    #[test]
    fn the_entry_parses_when_appended_to_the_table() {
        let text = entry(&measured(vec![Sequence::Osc777, Sequence::Osc9]));
        let appended = format!("{}\n{text}", include_str!("../data/terminals.toml"));
        let parsed: crate::Database = toml::from_str(&appended).expect("the entry should parse");

        let terminal = parsed.terminals.last().expect("the appended terminal");
        assert_eq!(terminal.id, "ghostty");
        assert_eq!(terminal.accepts, [Sequence::Osc777, Sequence::Osc9]);
        assert_eq!(terminal.tested_version.as_deref(), Some("1.3.1"));
        assert_eq!(terminal.term, ["xterm-ghostty"]);
        assert!(terminal.verified);
    }

    /// A terminal that answers no XTVERSION leaves the field empty rather than absent,
    /// which is how the table records Terminal.app.
    #[test]
    fn an_unnamed_terminal_still_parses() {
        let measurement = Measurement {
            xtversion: None,
            version: None,
            term_program: Some("Apple_Terminal".to_owned()),
            term: None,
            accepts: vec![],
            os: "macos",
            through_tmux: false,
        };
        let appended = format!(
            "{}\n{}",
            include_str!("../data/terminals.toml"),
            entry(&measurement)
        );
        let parsed: crate::Database = toml::from_str(&appended).expect("the entry should parse");

        let terminal = parsed.terminals.last().expect("the appended terminal");
        assert_eq!(terminal.id, "apple-terminal");
        assert!(terminal.xtversion.is_empty());
        assert!(terminal.accepts.is_empty());
        assert_eq!(terminal.tested_version, None);
    }

    #[test]
    fn records_which_system_it_was_measured_on() {
        assert!(entry(&measured(vec![])).starts_with("# Measured on macos\n"));
    }

    /// Two entries for one terminal that disagree are worth telling apart, and whether
    /// tmux was in the way is the first thing to check.
    #[test]
    fn records_that_tmux_was_in_the_way() {
        let through = Measurement {
            through_tmux: true,
            ..measured(vec![Sequence::Osc9])
        };
        assert!(entry(&through).starts_with("# Measured on macos, through tmux\n"));
    }

    /// The ids in the table were derived this way; a probe of one of those terminals has
    /// to arrive at the same string or the comparison below cannot find it.
    #[test]
    fn derives_the_ids_the_table_already_uses() {
        let apple = Measurement {
            xtversion: None,
            term_program: Some("Apple_Terminal".to_owned()),
            ..measured(vec![])
        };
        assert_eq!(apple.id(), "apple-terminal");
        assert_eq!(measured(vec![]).id(), "ghostty");
    }

    #[test]
    fn confirms_a_measurement_that_matches_the_table() {
        let report = against_the_table(&measured(vec![Sequence::Osc777, Sequence::Osc9]));
        assert!(report.contains("agrees with it"), "{report}");
    }

    /// A terminal that changed between versions is exactly what probe exists to catch,
    /// so the difference has to be shown rather than silently overwritten.
    #[test]
    fn shows_the_difference_when_it_does_not_match() {
        let report = against_the_table(&measured(vec![Sequence::Osc9]));
        assert!(report.contains("disagrees"), "{report}");
        assert!(report.contains("OSC 777, OSC 9"), "{report}");
        assert!(report.contains("OSC 9\n"), "{report}");
    }

    #[test]
    fn says_when_the_terminal_is_new_to_the_table() {
        let unknown = Measurement {
            xtversion: Some("Nonesuch".to_owned()),
            ..measured(vec![Sequence::Osc9])
        };
        let report = against_the_table(&unknown);
        assert!(report.contains("not in the table yet"), "{report}");
    }

    /// Silence is ambiguous, and the ambiguity is the whole content of the warning.
    #[test]
    fn warns_that_silence_may_be_a_permission_setting() {
        let warning = nothing_appeared();
        assert!(warning.contains("not permitted"), "{warning}");
        assert!(warning.contains("Terminal.app"), "{warning}");
    }

    /// The entries for iTerm2 and Terminal.app have an empty `term` for this reason: a
    /// probe that recorded xterm-256color would make every terminal claiming it look
    /// like the one being measured.
    #[test]
    fn leaves_a_generic_term_out_of_the_entry() {
        assert_eq!(identifying_term(Some("xterm-256color".to_owned())), None);
        assert_eq!(identifying_term(Some("screen".to_owned())), None);
        assert_eq!(
            identifying_term(Some("xterm-ghostty".to_owned())),
            Some("xterm-ghostty".to_owned())
        );
        // Unknown values stay in. A terminal-specific TERM peal has not seen is exactly
        // what a probe is for, and a human reads the entry before it is committed.
        assert_eq!(
            identifying_term(Some("nonesuch".to_owned())),
            Some("nonesuch".to_owned())
        );
    }

    /// tmux overwrites TERM_PROGRAM with its own name, so an entry recording it would
    /// match every terminal running under tmux rather than the one measured.
    #[test]
    fn says_the_environment_describes_tmux_rather_than_the_terminal() {
        let through = Measurement {
            term_program: None,
            term: None,
            through_tmux: true,
            ..measured(vec![Sequence::Osc9])
        };
        let note = identification_note(&through).expect("a note");
        assert!(note.contains("overwrites TERM_PROGRAM"), "{note}");

        let entry = entry(&through);
        assert!(entry.contains("term_program   = []"), "{entry}");
        assert!(entry.contains("term           = []"), "{entry}");
    }

    /// Every terminal in the table can be identified some way; an entry that cannot be
    /// would break that, so it has to be flagged rather than quietly filed.
    #[test]
    fn flags_an_entry_that_could_never_be_matched() {
        let anonymous = Measurement {
            xtversion: None,
            term_program: None,
            term: None,
            ..measured(vec![Sequence::Osc9])
        };
        let note = identification_note(&anonymous).expect("a note");
        assert!(note.contains("cannot be identified"), "{note}");

        // Even under tmux, where the environment is expected to be useless, a terminal
        // that gave no name of its own is the more pressing thing to say.
        let both = Measurement {
            through_tmux: true,
            ..anonymous
        };
        let note = identification_note(&both).expect("a note");
        assert!(note.contains("cannot be identified"), "{note}");
    }

    #[test]
    fn orders_dialects_richest_first() {
        assert_eq!(
            richest_first(),
            [Sequence::Osc99, Sequence::Osc777, Sequence::Osc9]
        );
    }
}
