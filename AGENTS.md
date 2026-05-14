# AGENTS.md

## Current State

- This repo is now a single Rust binary crate named `vcal`.
- Phase 1 is complete.
- Phase 2 is the next active implementation target.
- `TODO.md` still records the completed Phase 1 checklist; use the Phase 2 notes in `README.md` for the next implementation steps until `TODO.md` is updated.

## Commands

- Run tests with `cargo test`.
- Run the CLI with `cargo run`.
- Build the binary with `cargo build`.

## Structure

- `src/main.rs` is the CLI entrypoint.
- `src/lib.rs` holds the REPL and evaluation logic so behavior can be unit tested without spawning the binary.

## Guidance

- Keep Phase 2 limited to single-line REPL input, integer literals, parentheses, and integer arithmetic operators from IEEE 1364-2005 section 5.1.5: unary `+`, unary `-`, `+`, `-`, `*`, `/`, `%`, `**`.
- Do not add variables, declarations, strings, real numbers, concatenation, or non-arithmetic operators as part of Phase 2.
- Implement arithmetic expressions with a parsed AST, not ad hoc string splitting.
- Keep expression width/sign handling as a separate analysis step from numeric evaluation.
- Prefer a two-pass design for expressions:
  - bottom-up self-determined width/sign inference for each AST node
  - top-down context-determined evaluation so parent width can widen child arithmetic
- For arithmetic operators, if any operand contains `x` or `z`, the result should become all `x` bits at the effective result width.
- Preserve the canonical output contract from `README.md`: print values as Verilog literals via `Out[n]:`; print an empty `Out[n]:` line for system tasks with no return value.
- Do not infer support from the top-level support matrix; many checked items are long-term targets and are intentionally out of scope for the current phase.
