# peal

[![CI](https://github.com/bryutus/peal/actions/workflows/ci.yml/badge.svg)](https://github.com/bryutus/peal/actions/workflows/ci.yml)

Raise a desktop notification from the terminal.

```console
$ peal --title peal "build finished"
```

Long builds, slow test suites, deploys that take a coffee break — peal tells
the terminal to raise a notification when one finishes, so you can look away
until it does.

## The problem it solves

Terminals do not agree on how to be asked. There are three escape sequences in
circulation and no terminal accepts all of them alike:

- **OSC 9** has one field. Widest support, no title.
- **OSC 777** has a title and a body.
- **OSC 99** adds an id, so a notification can replace an earlier one.

Sending all three is not an option — kitty raises three separate
notifications. Sending the richest is not either: iTerm2 ignores OSC 777
entirely, and Ghostty raises nothing for an OSC 777 whose body field is
missing. So peal identifies the terminal first and picks the one dialect that
carries what you asked for.

Nothing about this can be looked up at runtime. Terminals do not advertise
which dialects they accept, and the notification is fire-and-forget: no reply
comes back to say it arrived, or that the operating system refused it. The only
way to know is to have watched a screen. So peal ships a table of what was
actually seen, and [`peal probe`](#adding-a-terminal) is how that table grows.

## Terminals

| Terminal | Dialects accepted | Measured on |
|---|---|---|
| kitty | OSC 99, OSC 777, OSC 9 | 0.48.2 |
| Ghostty | OSC 777, OSC 9 | 1.3.1 |
| iTerm2 | OSC 9 | 3.6.11 |
| Terminal.app | none — bell only | |
| WezTerm | none — bell only | 20240203-110809-5046fc22 |

All measured on macOS. Every row was confirmed by watching the screen, never
transcribed from documentation — WezTerm's documentation reads as though OSC
777 should work, and it did not.

A terminal that is not listed but answers XTVERSION is sent an OSC 9, on the
reasoning that every terminal measured so far accepted one. That is an
extrapolation, and `peal doctor` says so rather than presenting it as fact. A
terminal that cannot be identified at all gets the bell.

## Installing

```console
$ cargo install --git https://github.com/bryutus/peal
```

## Using it

```console
$ peal "build finished"
$ peal --title peal "build finished"
$ peal --name build "compiling"        # sending again under the same name replaces it
```

Nothing is printed when the notification goes out whole. Anything the terminal
could not carry is reported on stderr — a title folded into the body, a name
dropped, the bell rung instead — since that is what you could not have
predicted.

As a library:

```rust
use peal::notify::{self, Notification};

notify::notify(&Notification::titled("peal", "build finished"))?;
```

## When nothing appears

```console
$ peal doctor
```

It reports which terminal was identified and on what evidence, what each shape
of notification becomes here, and then sends one so there is something to
compare the words against.

The most common answer is the least interesting one: **on macOS a terminal
cannot post notifications until it is permitted to, and a freshly installed one
starts out denied.** All four terminals measured for the table above defaulted
to off. From inside the program this is indistinguishable from peal sending the
wrong bytes, which is why doctor asks whether the notification appeared instead
of telling you it did.

## Under tmux

peal works under tmux, with one setting:

```console
$ tmux set -g allow-passthrough on
```

tmux drops escape sequences it does not recognise instead of forwarding them,
and that is every notification dialect there is. peal wraps what it sends so
that tmux hands it on to the terminal beyond — but the wrapper only works when
that setting is on. Without it, the bell is all that gets through, and `peal
doctor` says so and names the setting.

## Adding a terminal

If your terminal is not in the table above, this is the useful thing you can
do:

```console
$ peal probe
```

It sends each dialect in turn and asks whether a notification appeared, then
prints an entry for `data/terminals.toml`. Open an issue with that output — or
a pull request adding it — and say which terminal and version it came from.

Please check your notification permissions before reporting a terminal that
accepted nothing. A terminal that raises no notifications and a terminal that
is not allowed to look exactly alike from here, and probe will say so.

Terminals that would be particularly useful: anything on Linux or Windows,
where nothing has been measured at all.

## License

MIT or Apache-2.0, at your option.
