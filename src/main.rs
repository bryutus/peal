//! The `peal` command.

use std::io::Write as _;
use std::process::ExitCode;

use peal::notify::{self, Delivery, Notification};
use peal::{doctor, probe};

const USAGE: &str = "\
peal — raise a desktop notification from the terminal

    peal [--title <title>] [--name <name>] <body>
    peal doctor
    peal probe

    --title <title>   A heading above the body. Terminals that cannot show one
                      separately are given \"title: body\" instead.
    --name <name>     Sending again under the same name replaces the notification
                      rather than adding another. Only kitty can do this so far.
    --                Everything after this is the body, even \"doctor\".
    -V, --version     The version of peal that is running.

    doctor            Report what peal will do on this terminal, and send one
                      notification to check it against.
    probe             Measure a terminal peal does not know, by sending every
                      dialect and asking which ones appeared. Prints an entry
                      for data/terminals.toml.
";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match Command::parse(&arguments) {
        Ok(Command::Help) => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Ok(Command::Version) => {
            println!("peal {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Ok(Command::Doctor) => match doctor::run() {
            Ok(report) => {
                print!("{report}");
                ExitCode::SUCCESS
            }
            Err(error) => fail(&error),
        },
        Ok(Command::Probe) => match probe::run() {
            Ok(report) => {
                print!("{report}");
                ExitCode::SUCCESS
            }
            Err(error) => fail(&error),
        },
        Ok(Command::Notify(notification)) => match notify::notify(&notification) {
            Ok(delivery) => {
                announce(&delivery);
                ExitCode::SUCCESS
            }
            Err(error) => fail(&error),
        },
        Err(problem) => {
            eprintln!("peal: {problem}");
            eprint!("\n{USAGE}");
            // Distinct from a failure to notify, so a script can tell a mistake in its
            // own arguments from a terminal that would not take the notification.
            ExitCode::from(2)
        }
    }
}

fn fail(error: &std::io::Error) -> ExitCode {
    eprintln!("peal: {error}");
    ExitCode::FAILURE
}

/// Says only what the caller could not have predicted. A notification that went out
/// whole needs no commentary; one that was reduced or never sent does.
fn announce(delivery: &Delivery) {
    let mut err = std::io::stderr();
    match delivery {
        Delivery::Sent {
            folded_title,
            dropped_id,
            ..
        } => {
            if *folded_title {
                let _ = writeln!(
                    err,
                    "peal: this terminal shows no separate title; it was folded into the body"
                );
            }
            if *dropped_id {
                let _ = writeln!(
                    err,
                    "peal: this terminal cannot replace a notification; the name was dropped"
                );
            }
        }
        Delivery::Bell => {
            let _ = writeln!(
                err,
                "peal: this terminal raises no notifications; rang the bell instead"
            );
        }
        Delivery::Nothing => {
            let _ = writeln!(err, "peal: no terminal attached; nothing was sent");
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Notify(Notification),
    Doctor,
    Probe,
    Help,
    Version,
}

impl Command {
    /// Hand-rolled because the whole grammar is one body and two options. A parser
    /// library would be most of the dependency tree of this program.
    fn parse(arguments: &[String]) -> Result<Self, String> {
        if arguments.is_empty() {
            return Ok(Command::Help);
        }
        if arguments.len() == 1 {
            match arguments[0].as_str() {
                "doctor" => return Ok(Command::Doctor),
                "probe" => return Ok(Command::Probe),
                "-h" | "--help" => return Ok(Command::Help),
                "-V" | "--version" => return Ok(Command::Version),
                _ => {}
            }
        }

        let mut title = None;
        let mut name = None;
        let mut body: Option<&str> = None;

        let mut rest = arguments.iter();
        while let Some(argument) = rest.next() {
            let mut take = |option: &str| {
                rest.next()
                    .map(String::as_str)
                    .ok_or_else(|| format!("{option} needs a value"))
            };
            match argument.as_str() {
                "--title" => title = Some(take("--title")?),
                "--name" => name = Some(take("--name")?),
                "-h" | "--help" => return Ok(Command::Help),
                "-V" | "--version" => return Ok(Command::Version),
                // Everything after `--` is the body, which is how a body of "doctor" or
                // one starting with a dash gets through.
                "--" => {
                    let remainder: Vec<&str> = rest.by_ref().map(String::as_str).collect();
                    match remainder.as_slice() {
                        [only] => body = Some(only),
                        [] => return Err("no body given after --".to_owned()),
                        _ => return Err("expected one body, got several".to_owned()),
                    }
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown option {other}"));
                }
                other if body.is_some() => {
                    return Err(format!("expected one body, also got {other:?}"));
                }
                other => body = Some(other),
            }
        }

        let body = body.ok_or_else(|| "no body given".to_owned())?;
        let mut notification = match title {
            Some(title) => Notification::titled(title, body),
            None => Notification::new(body),
        };
        if let Some(name) = name {
            notification = notification.named(name);
        }
        Ok(Command::Notify(notification))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Result<Command, String> {
        let arguments: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        Command::parse(&arguments)
    }

    #[test]
    fn a_lone_argument_is_the_body() {
        assert_eq!(
            parse("done"),
            Ok(Command::Notify(Notification::new("done")))
        );
    }

    #[test]
    fn reads_the_options() {
        assert_eq!(
            parse("--title peal --name build done"),
            Ok(Command::Notify(
                Notification::titled("peal", "done").named("build")
            ))
        );
    }

    /// Options after the body read the same as options before it.
    #[test]
    fn order_does_not_matter() {
        assert_eq!(parse("done --title peal"), parse("--title peal done"));
    }

    #[test]
    fn the_two_commands_are_commands_not_bodies() {
        assert_eq!(parse("doctor"), Ok(Command::Doctor));
        assert_eq!(parse("probe"), Ok(Command::Probe));
    }

    /// The escape hatch for the one body that would otherwise be swallowed.
    #[test]
    fn a_body_of_doctor_gets_through_after_a_double_dash() {
        assert_eq!(
            parse("-- doctor"),
            Ok(Command::Notify(Notification::new("doctor")))
        );
    }

    #[test]
    fn nothing_at_all_asks_for_help() {
        assert_eq!(parse(""), Ok(Command::Help));
        assert_eq!(parse("--help"), Ok(Command::Help));
    }

    /// Capital `-V`, as every other command-line program spells it; lowercase `-v` is
    /// left free for the verbosity flag it usually means.
    #[test]
    fn asks_which_version_is_running() {
        assert_eq!(parse("--version"), Ok(Command::Version));
        assert_eq!(parse("-V"), Ok(Command::Version));
        // A body of "--version" is still reachable, the same way "doctor" is.
        assert_eq!(
            parse("-- --version"),
            Ok(Command::Notify(Notification::new("--version")))
        );
    }

    #[test]
    fn refuses_what_it_cannot_read() {
        assert!(parse("--title").is_err());
        assert!(parse("--colour red done").is_err());
        assert!(parse("one two").is_err());
        assert!(parse("--title peal").is_err());
    }
}
