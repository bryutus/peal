# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`peal` raises a desktop notification by writing an OSC escape sequence to the terminal.
Three dialects exist (OSC 9, OSC 777, OSC 99) and no terminal accepts all of them alike, so peal identifies the terminal first and sends exactly one.
`README.md` has the reasoning.

## Invariants

These span several files and are easy to break without noticing.

- **Deciding is pure; only `detect/query.rs` touches the tty.** `resolve_from(Signals)`, `plan()`, `choose()` and `render::bytes()` take their inputs as values so the tests can cover terminals that are not installed on the machine running them. Keep it that way.
- **`data/terminals.toml` is a record of measurement, not documentation.** It is `include_str!`'d into the binary. `Sequence` is a closed enum — each variant needs its own byte-assembly arm, so adding a dialect to the data alone can never work, and `src/lib.rs` has tests that fail when the two drift apart.
- **`choose()` picks the *least* expressive accepted dialect that still carries what was asked for.** A richer dialect would have to send its spare fields empty, and an empty field is not the same as no field — Ghostty raises nothing at all for an OSC 777 with no body.
- **`Resolution` has four variants on purpose.** A measured terminal, one that named itself but is absent from the table, one that would not identify itself, and no tty. The second is an extrapolation and must be reported as a guess, never as fact.

## Conventions

- Notification text is assumed hostile — it is usually someone else's build output. `render::sanitize` strips control characters, and no route to the wire may skip it.
- Adding a dependency needs a real reason. Argument parsing is hand-rolled because a parser crate would have been most of this program's dependency tree.
- `Cargo.toml`'s `include` patterns must stay anchored (`/src/**/*`); unanchored ones match at any depth and pulled a stray `README.md` into the package.
- CI runs `cargo clippy --all-targets -- -D warnings` on both Linux and macOS — both, because the tty handling calls libc directly and termios types differ between them.
- `peal doctor` and `peal probe` write to the real terminal and `probe` waits on answers from the person running it, so neither is something to run casually while testing.
