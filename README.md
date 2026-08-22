# peal

English | [日本語](README.ja.md)

[![CI](https://github.com/bryutus/peal/actions/workflows/ci.yml/badge.svg)](https://github.com/bryutus/peal/actions/workflows/ci.yml)

Raise a desktop notification from the terminal.

```console
$ peal --title peal "build finished"
```

Peal is implemented to address prolonged build processes, extensive test suites, and time-consuming deployment procedures.
By signaling the terminal to issue a notification upon task completion, it allows users to divert attention from the terminal until the execution concludes.

## The problem it solves

Terminals do not agree on how to be asked. There are three escape sequences in
circulation and no terminal accepts all of them alike:

- **OSC 9** has one field. Widest support, no title.
- **OSC 777** has a title and a body.
- **OSC 99** adds an id, so a notification can replace an earlier one.

Transmitting all three alternatives is not viable, as a terminal accepting multiple inputs triggers a separate notification for each.
Conversely, sending only the most comprehensive option is insufficient: certain terminals ignore specific dialects entirely, while others fail to produce any output for sequences containing unpopulated fields.
Consequently, peal identifies the terminal beforehand and selects the single dialect that encapsulates the requested data.

Nothing about this can be determined at runtime.
Terminals do not advertise which dialects they accept, and notifications operate on a fire-and-forget basis: no reply is returned to confirm receipt or report rejection by the operating system.
Peal therefore records what was actually observed, and [`peal probe`](#adding-a-terminal) serves to expand this record.

## Terminals

The actual observations are detailed in [`data/terminals.toml`](data/terminals.toml).
Each entry was confirmed through direct screen inspection rather than transcribed from documentation.

An unregistered terminal responding to XTVERSION receives an OSC 9 sequence, on the reasoning that every terminal measured so far accepted one.
That is an extrapolation, and `peal doctor` says so rather than presenting it as fact.
Completely unidentifiable terminals trigger a bell.

## Installing

```console
$ cargo install --git https://github.com/bryutus/peal
```

## Using it

```console
$ peal "build finished"
$ peal --title peal "build finished"
$ peal --name build "compiling"        # sending again under the same name replaces it
$ some-long-command && peal "done"
```

Nothing is printed when the notification is transmitted in its entirety.
Any data that the terminal cannot process is reported to stderr—such as a title folded into the body, a dropped name, or an alternate bell signal—as these represent unpredictable anomalies.

As a library:

```rust
use peal::notify::{self, Notification};

notify::notify(&Notification::titled("peal", "build finished"))?;
```

## When nothing appears

```console
$ peal doctor
```

It reports the identified terminal and the underlying evidence, specifies the resulting notification formats, and subsequently transmits a notification to serve as a baseline for comparison.

First, check that notifications are enabled for your terminal.
From inside the program, a terminal lacking permission to send notifications is indistinguishable from peal sending incorrect bytes; consequently, doctor asks whether the notification appeared rather than asserting that it did.

## Under tmux

peal works under tmux, with one setting:

```console
$ tmux set -g allow-passthrough on
```

tmux drops escape sequences it does not recognise instead of forwarding them, and that is every notification dialect there is.
peal wraps what it sends so that tmux hands it on to the terminal beyond — but the wrapper only works when that setting is on.
Without it, the bell is all that gets through, and `peal doctor` says so and names the setting.

## Adding a terminal

If your terminal is not in [`data/terminals.toml`](data/terminals.toml), this is the useful thing you can do:

```console
$ peal probe
```

It sends each dialect in turn and asks whether a notification appeared, then prints an entry for `data/terminals.toml`.
Open an issue with that output — or a pull request adding it — and say which terminal and version it came from.

Probing under tmux works, but the entry it prints is thinner: tmux overwrites TERM_PROGRAM and TERM with its own, so neither can be recorded, and the report says as much.
An entry measured outside tmux carries more.

Please check your notification permissions before reporting a terminal that appears to accept nothing.
A terminal that generates no notifications and one that lacks permission look identical from here, and the probe says so.

Any terminal, on any operating system, is worth adding.

## License

MIT or Apache-2.0, at your option.
