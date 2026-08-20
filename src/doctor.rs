//! Explaining what peal does on this terminal, and what to look at when it appears to
//! do nothing.
//!
//! The report exists because the escape sequence is fire-and-forget. Nothing comes back
//! to say a notification was refused, so the difference between "peal sent the wrong
//! bytes" and "the operating system dropped them" cannot be observed from inside the
//! program. All peal can do is say exactly what it sent and what it concluded, and let
//! the reader compare that against what they saw.

use std::fmt::Write as _;

use crate::detect::{Evidence, Resolution};
use crate::notify::{Delivery, Notification};
use crate::{Sequence, database, notify};

/// The report, as text ready to print.
pub fn report(resolution: &Resolution, inside_tmux: bool, test: Option<&Delivery>) -> String {
    let mut out = String::new();
    terminal_section(&mut out, resolution, inside_tmux);
    // With no terminal there is nothing further to say that the section above has not
    // said, and four identical lines saying so would only bury it.
    if resolution != &Resolution::NoTty {
        dialects_section(&mut out, resolution);
        if let Some(delivery) = test {
            test_section(&mut out, delivery);
        }
    }
    out
}

/// Runs the report against the terminal this process is attached to, sending one
/// notification so the reader has something to compare the words against.
pub fn run() -> std::io::Result<String> {
    let resolution = crate::detect::resolve()?;
    let inside_tmux = std::env::var_os("TMUX").is_some();
    let test = notify::to(
        &resolution,
        &Notification::titled("peal", "this is a test notification"),
    )?;
    Ok(report(&resolution, inside_tmux, Some(&test)))
}

fn terminal_section(out: &mut String, resolution: &Resolution, inside_tmux: bool) {
    out.push_str("Terminal\n");
    match resolution {
        Resolution::Known {
            terminal,
            evidence,
            version,
        } => {
            let _ = writeln!(out, "  {} — {}", terminal.id, describe(*evidence));
            match (&terminal.tested_version, terminal.verified) {
                (Some(tested), true) => {
                    let _ = writeln!(
                        out,
                        "  Measured on version {tested}, so what follows was observed rather than assumed."
                    );
                    // A terminal can gain or lose a dialect between releases, so a
                    // reader on a different one should know the table may be behind.
                    if let Some(running) = version.as_deref().filter(|v| *v != tested) {
                        let _ = writeln!(
                            out,
                            "  You are running {running}. Anything below may have changed since."
                        );
                    }
                }
                (None, true) => {
                    out.push_str(
                        "  Measured directly, so what follows was observed rather than assumed.\n",
                    );
                }
                _ => out.push_str(
                    "  Taken from documentation rather than measured. It may be wrong.\n",
                ),
            }
        }
        Resolution::UnknownButModern { name, version } => {
            match version {
                Some(version) => {
                    let _ = writeln!(
                        out,
                        "  {name} {version} — named itself, but peal has no entry for it."
                    );
                }
                None => {
                    let _ = writeln!(
                        out,
                        "  {name} — named itself, but peal has no entry for it."
                    );
                }
            }
            out.push_str(
                "  Answering XTVERSION at all suggests it will take an OSC 9, since every\n\
                 \x20 terminal measured so far did. That is an extrapolation from four terminals\n\
                 \x20 on one platform, not a fact about this one.\n",
            );
        }
        Resolution::Unknown => {
            out.push_str("  Unidentified. It answered no query and set no variable peal knows.\n");
            out.push_str(
                "  Only the bell is safe here: an escape sequence a terminal does not\n\
                 \x20 understand may be printed to the screen rather than ignored.\n",
            );
        }
        Resolution::NoTty => {
            out.push_str("  None. This process has no controlling terminal.\n");
            out.push_str(
                "  Output is going to a pipe, a file, or a job with no terminal at all, so\n\
                 \x20 there is nothing to notify. peal sends nothing and reports it.\n",
            );
        }
    }
    if inside_tmux {
        out.push_str(
            "\n  Running under tmux. tmux answers XTVERSION on its own behalf, which would\n\
             \x20 name the multiplexer rather than the terminal drawing it, so peal did not\n\
             \x20 ask and used the environment instead. Whatever the outer terminal can do\n\
             \x20 beyond what this says is not visible from here.\n",
        );
    }
}

fn describe(evidence: Evidence) -> &'static str {
    match evidence {
        Evidence::XtVersion => "it named itself when asked",
        Evidence::TermProgram => "recognised from TERM_PROGRAM, since it answers no query",
        Evidence::Term => {
            "recognised from TERM, since it answers no query and sets no TERM_PROGRAM"
        }
    }
}

/// The four shapes a notification can take, each labelled as the reader would describe
/// their own call.
fn requests() -> [(&'static str, Notification); 4] {
    [
        ("a plain notification", Notification::new(BODY)),
        ("with a title", Notification::titled(TITLE, BODY)),
        ("with a name", Notification::new(BODY).named(NAME)),
        ("with both", Notification::titled(TITLE, BODY).named(NAME)),
    ]
}

const TITLE: &str = "peal";
const BODY: &str = "this is a test notification";
const NAME: &str = "doctor";

fn dialects_section(out: &mut String, resolution: &Resolution) {
    out.push_str("\nWhat peal sends here\n");

    let previews = requests().map(|(label, notification)| {
        let delivery = notify::preview(resolution, &notification);
        (label, delivery)
    });

    // A terminal that treats all four shapes alike — one with no dialect at all — says
    // so once. Four identical lines would read as four different answers.
    if previews
        .iter()
        .all(|(_, delivery)| *delivery == previews[0].1)
    {
        describe_delivery(out, "any notification", &previews[0].1);
    } else {
        for (label, delivery) in &previews {
            describe_delivery(out, label, delivery);
        }
    }

    forms_section(out, &previews);
}

fn describe_delivery(out: &mut String, label: &str, delivery: &Delivery) {
    let _ = write!(out, "  {label:<22}");
    match delivery {
        Delivery::Sent {
            sequence,
            folded_title,
            dropped_id,
        } => {
            let losses = losses(*folded_title, *dropped_id);
            if losses.is_empty() {
                let _ = writeln!(out, "{sequence}");
            } else {
                // Kept on the one line: the same loss recurs across several rows, and
                // spelling it out under each would bury the dialects it happens to.
                let _ = writeln!(out, "{:<10}{}", sequence.to_string(), losses.join(", "));
            }
        }
        Delivery::Bell => {
            out.push_str("the bell — this terminal raises no notifications\n");
        }
        Delivery::Nothing => out.push_str("nothing, there being no terminal\n"),
    }
}

/// The wire format of each dialect the terminal actually uses.
///
/// Not needed to follow the report, but it is what to quote when a notification arrives
/// looking wrong and the terminal is the suspect.
fn forms_section(out: &mut String, previews: &[(&str, Delivery)]) {
    // In order of first use, and each one only once. A terminal can return to a dialect
    // it has already used — Ghostty alternates between two across the four shapes — so
    // this cannot just drop neighbouring repeats.
    let mut used: Vec<Sequence> = Vec::new();
    for (_, delivery) in previews {
        if let Delivery::Sent { sequence, .. } = delivery
            && !used.contains(sequence)
        {
            used.push(*sequence);
        }
    }
    if used.is_empty() {
        return;
    }

    out.push_str("\n  In full:\n");
    for sequence in used {
        let _ = writeln!(
            out,
            "    {:<10}{}",
            sequence.to_string(),
            database().capability(sequence).form
        );
    }
}

/// What the chosen dialect could not take, phrased short enough to sit beside its name.
fn losses(folded_title: bool, dropped_id: bool) -> Vec<&'static str> {
    let mut notes = Vec::new();
    if folded_title {
        notes.push("the title joins the body");
    }
    if dropped_id {
        notes.push("the name is dropped");
    }
    notes
}

fn test_section(out: &mut String, delivery: &Delivery) {
    out.push_str("\nTest notification\n");
    match delivery {
        Delivery::Sent { sequence, .. } => {
            let _ = writeln!(out, "  Sent as {sequence}. Did it appear?\n");
            out.push_str(
                "  If it did not, the terminal is most likely not permitted to post\n\
                 \x20 notifications. ",
            );
            out.push_str(if cfg!(target_os = "macos") {
                "Open System Settings > Notifications and find the\n  terminal application; newly installed terminals default to off.\n"
            } else {
                "Check the notification settings of the desktop\n  environment for the terminal application.\n"
            });
            out.push_str(
                "\n  This has to ask rather than tell. The escape sequence is fire-and-forget:\n\
                 \x20 a terminal that dropped the notification reports exactly what one that\n\
                 \x20 raised it reports, which is nothing.\n",
            );
        }
        Delivery::Bell => {
            out.push_str("  Rang the bell, this terminal having no notification dialect.\n");
        }
        Delivery::Nothing => out.push_str("  Nothing was sent, there being no terminal.\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(id: &str, evidence: Evidence) -> Resolution {
        Resolution::Known {
            terminal: database().terminal(id).expect("terminal in the table"),
            evidence,
            version: None,
        }
    }

    #[test]
    fn names_the_terminal_and_how_it_was_found() {
        let report = report(&known("kitty", Evidence::XtVersion), false, None);
        assert!(report.contains("kitty"), "{report}");
        assert!(report.contains("named itself"), "{report}");
        assert!(report.contains("0.48.2"), "{report}");
    }

    /// The four requests must each get a line, or the reader cannot tell which case
    /// their own call falls into.
    #[test]
    fn covers_every_shape_of_notification() {
        let report = report(&known("kitty", Evidence::XtVersion), false, None);
        for (label, _) in requests() {
            assert!(report.contains(label), "missing {label}:\n{report}");
        }
    }

    /// What a terminal cannot do is the part worth reading, so it has to be spelled out
    /// rather than left as a missing dialect name.
    #[test]
    fn explains_what_iterm2_cannot_carry() {
        let report = report(&known("iterm2", Evidence::TermProgram), false, None);
        assert!(report.contains("the title joins the body"), "{report}");
        assert!(report.contains("the name is dropped"), "{report}");
    }

    /// The table names dialects; the wire format belongs underneath it, where it does
    /// not push the four answers off to the right.
    #[test]
    fn names_dialects_in_the_table_and_spells_them_out_below() {
        let report = report(&known("ghostty", Evidence::XtVersion), false, None);
        let (table, full) = report.split_once("In full:").expect("a forms section");
        assert!(table.contains("with a title          OSC 777"), "{table}");
        assert!(!table.contains("OSC 777 ; notify"), "{table}");
        assert!(
            full.contains("OSC 777 ; notify ; <title> ; <body> BEL"),
            "{full}"
        );
    }

    /// A dialect used by more than one of the four shapes is spelled out once — including
    /// when the shapes that use it are not adjacent, as on Ghostty.
    #[test]
    fn spells_out_each_dialect_once() {
        let iterm2 = report(&known("iterm2", Evidence::TermProgram), false, None);
        assert_eq!(iterm2.matches("OSC 9 ; <body> BEL").count(), 1, "{iterm2}");

        let ghostty = report(&known("ghostty", Evidence::XtVersion), false, None);
        let (_, full) = ghostty.split_once("In full:").expect("a forms section");
        assert_eq!(full.matches("OSC 9 ; <body> BEL").count(), 1, "{full}");
        assert_eq!(full.matches("OSC 777 ; notify").count(), 1, "{full}");
    }

    /// Terminal.app treats every shape alike, so saying so four times would read as four
    /// separate answers that happen to agree.
    #[test]
    fn tells_apple_terminal_once_that_it_gets_the_bell() {
        let report = report(&known("apple-terminal", Evidence::TermProgram), false, None);
        assert_eq!(
            report.matches("raises no notifications").count(),
            1,
            "{report}"
        );
        assert!(report.contains("any notification"), "{report}");
    }

    /// A guess has to read as a guess.
    #[test]
    fn marks_an_unlisted_terminal_as_extrapolation() {
        let resolution = Resolution::UnknownButModern {
            name: "Nonesuch".to_owned(),
            version: None,
        };
        let report = report(&resolution, false, None);
        assert!(report.contains("Nonesuch"), "{report}");
        assert!(report.contains("extrapolation"), "{report}");
    }

    /// The table records the version it was measured against. A reader on a newer one
    /// is reading something that may have gone stale.
    #[test]
    fn warns_when_the_running_version_is_not_the_measured_one() {
        let resolution = Resolution::Known {
            terminal: database().terminal("ghostty").expect("in the table"),
            evidence: Evidence::XtVersion,
            version: Some("1.4.0".to_owned()),
        };
        let newer = report(&resolution, false, None);
        assert!(newer.contains("Measured on version 1.3.1"), "{newer}");
        assert!(newer.contains("You are running 1.4.0"), "{newer}");

        let same = Resolution::Known {
            terminal: database().terminal("ghostty").expect("in the table"),
            evidence: Evidence::XtVersion,
            version: Some("1.3.1".to_owned()),
        };
        let matching = report(&same, false, None);
        assert!(!matching.contains("You are running"), "{matching}");
    }

    #[test]
    fn explains_why_tmux_limits_what_it_can_say() {
        let report = report(&known("kitty", Evidence::Term), true, None);
        assert!(report.contains("tmux"), "{report}");
    }

    /// Without a terminal the first section has said everything; the rest would only
    /// repeat "there is no terminal" in three more shapes.
    #[test]
    fn stops_after_saying_there_is_no_terminal() {
        let report = report(&Resolution::NoTty, false, Some(&Delivery::Nothing));
        assert!(report.contains("no controlling terminal"), "{report}");
        assert!(!report.contains("What peal sends here"), "{report}");
        assert!(!report.contains("Test notification"), "{report}");
    }

    /// The permission note is the single most useful line in the report, and it is only
    /// worth printing when something was actually sent for the reader to have missed.
    #[test]
    fn asks_whether_the_test_notification_appeared() {
        let delivery = Delivery::Sent {
            sequence: Sequence::Osc777,
            folded_title: false,
            dropped_id: false,
        };
        let report = report(
            &known("ghostty", Evidence::XtVersion),
            false,
            Some(&delivery),
        );
        assert!(report.contains("Did it appear?"), "{report}");
        assert!(report.contains("not permitted to post"), "{report}");
    }
}
