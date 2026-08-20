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
    /// A name for this notification. Sending under a name already on screen updates that
    /// notification instead of adding one beside it.
    pub id: Option<String>,
}

impl Notification {
    pub fn new(body: impl Into<String>) -> Self {
        Self {
            title: None,
            body: body.into(),
            id: None,
        }
    }

    pub fn titled(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: Some(title.into()),
            id: None,
            body: body.into(),
        }
    }

    /// Names this notification, so that sending it again under the same name replaces
    /// what is on screen rather than stacking another notification on top.
    ///
    /// Any string will do — a path, a job name, whatever the caller already uses to mean
    /// "the same piece of work". It is encoded before it goes out.
    pub fn named(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
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
        /// True when the notification was named but no dialect the terminal accepts can
        /// carry a name. The notification was still shown; it simply will not replace
        /// anything, and nothing will replace it.
        dropped_id: bool,
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
    to(&detect::resolve()?, notification)
}

/// Shows a notification on a terminal already identified.
///
/// The same as [`notify`], for a caller that has a [`Resolution`] in hand and should not
/// pay for another round trip to the terminal to get one.
pub fn to(resolution: &Resolution, notification: &Notification) -> io::Result<Delivery> {
    let Some(accepts) = accepted(resolution) else {
        return Ok(Delivery::Nothing);
    };
    let Some(mut tty) = detect::query::open_tty()? else {
        return Ok(Delivery::Nothing);
    };

    let plan = plan(accepts, notification);
    let bytes = match &plan {
        Some(plan) => render::bytes(
            plan.sequence,
            plan.title.as_deref(),
            &plan.body,
            plan.id.as_deref(),
        ),
        None => b"\x07".to_vec(),
    };

    tty.write_all(&bytes)?;
    tty.flush()?;
    Ok(delivery_of(notification, plan.as_ref()))
}

/// What [`notify`] would do with this notification, without doing it.
///
/// Answers the question `doctor` asks four times over — what happens to a notification
/// with a title, with a name, with both, with neither — without putting four
/// notifications on the screen to find out.
pub fn preview(resolution: &Resolution, notification: &Notification) -> Delivery {
    let Some(accepts) = accepted(resolution) else {
        return Delivery::Nothing;
    };
    delivery_of(notification, plan(accepts, notification).as_ref())
}

/// The dialects available on this terminal, or `None` when there is no terminal.
fn accepted(resolution: &Resolution) -> Option<&[Sequence]> {
    match resolution {
        Resolution::Known { terminal, .. } => Some(&terminal.accepts),
        Resolution::UnknownButModern { .. } => Some(&GUESS),
        Resolution::Unknown => Some(&[]),
        Resolution::NoTty => None,
    }
}

/// What became of the notification, by comparing what was asked for against what the
/// chosen dialect could take.
fn delivery_of(notification: &Notification, plan: Option<&Plan>) -> Delivery {
    match plan {
        Some(plan) => Delivery::Sent {
            sequence: plan.sequence,
            folded_title: notification.title.is_some() && plan.title.is_none(),
            dropped_id: notification.id.is_some() && plan.id.is_none(),
        },
        None => Delivery::Bell,
    }
}

/// A notification reduced to what one dialect can carry.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Plan {
    sequence: Sequence,
    title: Option<String>,
    body: String,
    id: Option<String>,
}

/// Picks the dialect and reshapes the notification to fit it, or returns `None` when the
/// terminal accepts no dialect at all.
///
/// Kept separate from the I/O so the choice can be tested against terminals that are not
/// installed here.
fn plan(accepts: &[Sequence], notification: &Notification) -> Option<Plan> {
    let title = notification
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty());
    let id = notification.id.as_deref().filter(|id| !id.is_empty());

    let sequence = choose(accepts, title.is_some(), id.is_some())?;
    let capability = database().capability(sequence);

    let id = id.filter(|_| capability.id).map(str::to_owned);
    match title {
        Some(title) if !capability.title => Some(Plan {
            sequence,
            title: None,
            // The dialect has one field, so the title joins the body rather than
            // vanishing.
            body: format!("{title}: {}", notification.body),
            id,
        }),
        title => Some(Plan {
            sequence,
            title: title.map(str::to_owned),
            body: notification.body.clone(),
            id,
        }),
    }
}

/// The least expressive accepted dialect that still carries everything asked for.
///
/// A dialect with fields to spare has to send them empty, and an empty field is not the
/// same as no field: Ghostty wants all three of an OSC 777 and raises nothing at all
/// when the body is left off, so an untitled OSC 777 would have to carry a blank body.
/// Asking for one field when one field is what is needed avoids the question.
///
/// What the terminal cannot do is given up in order, id first. A name that goes nowhere
/// costs the notification nothing but the replacing; a title that goes nowhere would
/// cost it half its text, so the title is what the remaining choice is made on.
fn choose(accepts: &[Sequence], wants_title: bool, wants_id: bool) -> Option<Sequence> {
    let carries = |sequence: &Sequence, id: bool| {
        let capability = database().capability(*sequence);
        (!wants_title || capability.title) && (!id || capability.id)
    };

    let simplest = |id: bool| accepts.iter().rev().find(|s| carries(s, id));
    simplest(wants_id)
        .or_else(|| simplest(false))
        // With nothing on offer able to carry a title either, the title is folded into
        // the body of the richest dialect there is.
        .or(accepts.first())
        .copied()
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
        assert_eq!(plan.sequence, Sequence::Osc777);
        assert_eq!(plan.title.as_deref(), Some("peal"));
        assert_eq!(plan.body, "done");
        assert_eq!(plan.id, None);
    }

    #[test]
    fn ghostty_gets_the_one_dialect_of_its_two_that_carries_a_title() {
        let plan = plan(accepts("ghostty"), &Notification::titled("peal", "done")).unwrap();
        assert_eq!(plan.sequence, Sequence::Osc777);
    }

    /// Without a title, one field is enough, and a dialect with spare fields would only
    /// give the terminal something to fill in.
    #[test]
    fn an_untitled_notification_drops_to_the_single_field_dialect() {
        assert_eq!(choose(accepts("kitty"), false, false), Some(Sequence::Osc9));
        assert_eq!(
            choose(accepts("ghostty"), false, false),
            Some(Sequence::Osc9)
        );
    }

    /// iTerm2 accepts only OSC 9, which has a single field, so the title has to travel
    /// inside the body or not at all.
    #[test]
    fn iterm2_gets_the_title_folded_into_the_body() {
        let plan = plan(accepts("iterm2"), &Notification::titled("peal", "done")).unwrap();
        assert_eq!(plan.sequence, Sequence::Osc9);
        assert_eq!(plan.title, None);
        assert_eq!(plan.body, "peal: done");
    }

    #[test]
    fn a_notification_without_a_title_folds_nothing() {
        let plan = plan(accepts("iterm2"), &Notification::new("done")).unwrap();
        assert_eq!(plan.body, "done");
        assert_eq!(plan.title, None);
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
        assert_eq!(plan.sequence, Sequence::Osc9);
        assert_eq!(plan.body, "peal: done");
    }

    /// A name is the one thing OSC 99 has that the others do not, so asking for one is
    /// what makes it worth its extra field.
    #[test]
    fn a_named_notification_takes_the_only_dialect_that_carries_a_name() {
        let notification = Notification::titled("peal", "done").named("build");
        let plan = plan(accepts("kitty"), &notification).unwrap();
        assert_eq!(plan.sequence, Sequence::Osc99);
        assert_eq!(plan.title.as_deref(), Some("peal"));
        assert_eq!(plan.id.as_deref(), Some("build"));
    }

    #[test]
    fn a_named_notification_needs_no_title_to_reach_osc99() {
        let plan = plan(accepts("kitty"), &Notification::new("done").named("build")).unwrap();
        assert_eq!(plan.sequence, Sequence::Osc99);
        assert_eq!(plan.id.as_deref(), Some("build"));
    }

    /// Ghostty carries a title but no name. Keeping the title matters more than keeping
    /// the name, so the name is what goes.
    #[test]
    fn a_name_is_given_up_before_a_title_is() {
        let notification = Notification::titled("peal", "done").named("build");
        let plan = plan(accepts("ghostty"), &notification).unwrap();
        assert_eq!(plan.sequence, Sequence::Osc777);
        assert_eq!(plan.title.as_deref(), Some("peal"));
        assert_eq!(plan.id, None);
    }

    /// iTerm2 can carry neither, so the title folds into the body and the name is lost.
    #[test]
    fn iterm2_loses_the_name_and_folds_the_title() {
        let notification = Notification::titled("peal", "done").named("build");
        let plan = plan(accepts("iterm2"), &notification).unwrap();
        assert_eq!(plan.sequence, Sequence::Osc9);
        assert_eq!(plan.body, "peal: done");
        assert_eq!(plan.id, None);
    }

    /// An empty name is no name, the same as an empty title: it must not cost the
    /// notification a dialect it would otherwise have used.
    #[test]
    fn an_empty_name_is_no_name() {
        let plan = plan(accepts("kitty"), &Notification::new("done").named("")).unwrap();
        assert_eq!(plan.sequence, Sequence::Osc9);
        assert_eq!(plan.id, None);
    }
}
