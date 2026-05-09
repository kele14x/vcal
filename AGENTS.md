# AGENTS.md

## Current State

- This repo is now a single Rust binary crate named `vcal`.
- Phase 1 scope is defined in `TODO.md`; implement against the `Implementation Plan` section, not the long-term support matrix.

## Commands

- Run tests with `cargo test`.
- Run the CLI with `cargo run`.
- Build the binary with `cargo build`.

## Structure

- `src/main.rs` is the CLI entrypoint.
- `src/lib.rs` holds the REPL and evaluation logic so behavior can be unit tested without spawning the binary.

## Guidance

- Keep Phase 1 limited to single-line REPL input, integer literals, and `$finish`/`$stop`.
- Preserve the canonical output contract from `README.md`: print values as Verilog literals via `Out[n]:`; print an empty `Out[n]:` line for system tasks with no return value.
- Do not infer support from the top-level support matrix; many checked items are long-term targets and are intentionally out of scope for the current phase.
