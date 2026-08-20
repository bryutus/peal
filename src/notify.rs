//! Raising a desktop notification through whatever dialect the terminal understands.

pub mod render;

use std::io::{self, Write};

use crate::detect::{self, Resolution};
use crate::{Sequence, database};

/// What to show, before it has been reduced to what the terminal can actually carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub title: Option<String>,
    pub body: String,
}

impl Notification {
    pub fn new(body: impl Into<String>) -> Self {
        Self {
            title: None,
            body: body.into(),
        }
    }

    pub fn titled(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: Some(title.into()),
            body: body.into(),
        }
    }
}

/// What was actually put on the terminal. Reported rather than discarded because the
/// terminal never answers back: this is the only account of what happened a caller can
/// get, and `doctor` exists to show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    Sent {
        sequence: Sequence,
        /// True when the dialect could not carry a title and it was folded into the body.
        folded_title: bool,
    },
    /// The terminal understands no notification dialect, so it got the bell instead.
    Bell,
    /// No controlling terminal. Nothing was sent, and whether that is a problem is the
    /// caller's to decide.
    Nothing,
}

/// The guess for a terminal that names itself but is absent from the table. Every
/// OSC-capable terminal measured so far accepts OSC 9, which is why it is the guess and
/// also why it is only a guess.
const GUESS: [Sequence; 1] = [Sequence::Osc9];

/// Shows a notification on the terminal running this process.
///
/// Delivery cannot be confirmed. The escape sequence is fire-and-forget: a terminal does
/// not report back, and on macOS the system silently drops notifications from an
/// application that has not been permitted to post them.
pub fn notify(notification: &Notification) -> io::Result<Delivery> {
    let resolution = detect::resolve()?;
    let accepts: &[Sequence] = match &resolution {
        Resolution::Known { terminal, .. } => &terminal.accepts,
        Resolution::UnknownButModern { .. } => &GUESS,
        Resolution::Unknown => &[],
        Resolution::NoTty => return Ok(Delivery::Nothing),
    };

    let Some(mut tty) = detect::query::open_tty()? else {
        return Ok(Delivery::Nothing);
    };

    let (bytes, delivery) = match plan(accepts, notification) {
        Some((sequence, title, body)) => (
            render::bytes(sequence, title.as_deref(), &body),
            Delivery::Sent {
                sequence,
                folded_title: notification.title.is_some() && title.is_none(),
            },
        ),
        None => (b"\x07".to_vec(), Delivery::Bell),
    };

    tty.write_all(&bytes)?;
    tty.flush()?;
    Ok(delivery)
}

/// Picks the dialect and shapes the text to fit it, or returns `None` when the terminal
/// accepts no dialect at all.
///
/// Kept separate from the I/O so the choice can be tested against terminals that are not
/// installed here.
fn plan(
    accepts: &[Sequence],
    notification: &Notification,
) -> Option<(Sequence, Option<String>, String)> {
    let sequence = choose(accepts, notification.title.is_some())?;

    let title = notification
        .title
        .as_deref()
        .filter(|t| !t.trim().is_empty());
    let Some(title) = title else {
        return Some((sequence, None, notification.body.clone()));
    };

    if database().capability(sequence).title {
        return Some((sequence, Some(title.to_owned()), notification.body.clone()));
    }
    // The dialect has one field, so the title joins the body rather than vanishing.
    Some((sequence, None, format!("{title}: {}", notification.body)))
}

/// The least expressive accepted dialect that still carries everything asked for.
///
/// A dialect with fields to spare has to send them empty, and an empty field is not the
/// same as no field: Ghostty wants all three of an OSC 777 and raises nothing at all
/// when the body is left off, so an untitled OSC 777 would have to carry a blank body.
/// Asking for one field when one field is what is needed avoids the question.
fn choose(accepts: &[Sequence], wants_title: bool) -> Option<Sequence> {
    let fits = accepts
        .iter()
        .rev()
        .find(|sequence| !wants_title || database().capability(**sequence).title);
    // With nothing on offer able to carry a title, the title is folded into the body of
    // the richest dialect there is.
    fits.or(accepts.first()).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accepts(id: &str) -> &'static [Sequence] {
        &database()
            .terminal(id)
            .expect("terminal in the table")
            .accepts
    }

    /// kitty accepts all three dialects. A title needs two fields and no more, so it
    /// gets OSC 777 rather than OSC 99, whose extra field is the id.
    #[test]
    fn kitty_gets_the_simplest_dialect_that_carries_a_title() {
        let plan = plan(accepts("kitty"), &Notification::titled("peal", "done")).unwrap();
        assert_eq!(
            plan,
            (Sequence::Osc777, Some("peal".to_owned()), "done".to_owned())
        );
    }

    #[test]
    fn ghostty_gets_the_one_dialect_of_its_two_that_carries_a_title() {
        let plan = plan(accepts("ghostty"), &Notification::titled("peal", "done")).unwrap();
        assert_eq!(plan.0, Sequence::Osc777);
    }

    /// Without a title, one field is enough, and a dialect with spare fields would only
    /// give the terminal something to fill in.
    #[test]
    fn an_untitled_notification_drops_to_the_single_field_dialect() {
        assert_eq!(choose(accepts("kitty"), false), Some(Sequence::Osc9));
        assert_eq!(choose(accepts("ghostty"), false), Some(Sequence::Osc9));
    }

    /// iTerm2 accepts only OSC 9, which has a single field, so the title has to travel
    /// inside the body or not at all.
    #[test]
    fn iterm2_gets_the_title_folded_into_the_body() {
        let plan = plan(accepts("iterm2"), &Notification::titled("peal", "done")).unwrap();
        assert_eq!(plan, (Sequence::Osc9, None, "peal: done".to_owned()));
    }

    #[test]
    fn a_notification_without_a_title_folds_nothing() {
        let plan = plan(accepts("iterm2"), &Notification::new("done")).unwrap();
        assert_eq!(plan, (Sequence::Osc9, None, "done".to_owned()));
    }

    /// Terminal.app raises no notification by any sequence, so there is nothing to plan.
    #[test]
    fn apple_terminal_gets_no_dialect() {
        assert!(plan(accepts("apple-terminal"), &Notification::new("done")).is_none());
        assert!(plan(&[], &Notification::new("done")).is_none());
    }

    /// An unlisted terminal is guessed at with the one dialect they all accepted.
    #[test]
    fn the_guess_is_the_widest_dialect() {
        let plan = plan(&GUESS, &Notification::titled("peal", "done")).unwrap();
        assert_eq!(plan, (Sequence::Osc9, None, "peal: done".to_owned()));
    }
}
