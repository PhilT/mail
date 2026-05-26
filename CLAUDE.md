# mail/

Rust toolkit for working with the local Maildir at `/data/mail/namecheap/`.
A `[lib]` plus a binary per command (`mail-triage`, ... more to come), sharing
parsing and action helpers.

## Test First (TDD)

Write a failing test before writing the implementation. New behaviour lands
as: (1) red test describing the contract, (2) just enough code to turn it
green, (3) tidy.

**Public surface only.** Tests go in `tests/` and exercise the binary's CLI
or the library's public API. No reach-in to private functions — if it can't
be observed from outside the crate, it doesn't need its own test. Fixtures
live in `tests/fixtures/`.

## Dev shell

`nix-shell` in this directory picks up rustc / cargo / clippy / rustfmt /
rust-analyzer (see `shell.nix`).
