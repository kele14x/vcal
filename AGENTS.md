# AGENTS.md

## Current State

- Phases 1–5 complete. No active phase; next-phase scope is TBD.
- Phase 1 — REPL shell, integer literals (all LRM forms), `$finish`/`$stop`. Done.
- Phase 2 — arithmetic ops (`+ - * / % **`, unary), two-pass width handling, leftmost-base propagation, rustyline history. Done.
- Phase 3 — relational ops (`<`, `>`, `<=`, `>=`) with LRM 5.5.2 propagated-context unification (zero-extend at the leaf primary when context is unsigned, sign-extend when signed), 1-bit unsigned result, x/z propagation. Done.
- Phase 4 — equality ops (`==`, `!=`, `===`, `!==`) sharing the relational unification path; per-bit ambiguity for `==`/`!=` (a definite mismatch defeats x), bit-for-bit including x/z for `===`/`!==`; corrected `context_extension_bit` to zero-fill under unsigned propagated context. Done.
- Phase 5 — logical ops (`!`, `&&`, `||`) with self-determined operands reduced to a 1-bit logical value (any `1` → 1, all `0` → 0, any x/z without a `1` → x), LRM §5.1.9 truth tables, 1-bit unsigned binary result that widens through outer arithmetic context like relational/equality. Bare `&`/`|` rejected (bitwise out of scope). Done.

## Current Scope

- Single-line REPL input only
- Integer literals and parentheses
- Integer arithmetic operators (`+`, `-`, `*`, `/`, `%`, `**`, plus unary `+` / `-`)
- Relational operators (`<`, `>`, `<=`, `>=`) — 1-bit unsigned binary result
- Equality operators (`==`, `!=`, `===`, `!==`) — 1-bit unsigned binary result
- Logical operators (`!`, `&&`, `||`) — 1-bit unsigned binary result with x/z reduction
- No variables, declarations, strings, real numbers, concatenation
- No bitwise, no shifts

## Active Phase

No active phase. Next-phase scope is TBD — confirm with the user before starting new work. Likely candidates are bitwise (`& | ^ ~`) or shift operators (`<< >> <<< >>>`); see README's "Supported Matrix" for the full target.

## Backlog

See README's "Supported Matrix" for the final target. Phase scoping for everything beyond Phase 4 is TBD — confirm with the user before starting work outside the active phase.

## Commands

- Run tests with `cargo test`.
- Run the CLI with `cargo run`.
- Build the binary with `cargo build`.

## Structure

- `src/main.rs` is the CLI entrypoint.
- `src/lib.rs` holds the REPL and evaluation logic so behavior can be unit tested without spawning the binary.

## Guidance

- Do not infer scope from README's "Supported Matrix" — many checked boxes are long-term targets, not current scope. Confirm with the user before expanding.
- Prompt format is `In[n]: ` / `Out[n]: ` (trailing space). Print `Out[n]: ` with an empty value for system tasks like `$finish`/`$stop`.
- Two REPL entry points: `vcal::run_interactive` (rustyline, TTY only) and `vcal::run_repl(BufRead, Write)` (piped/test). `src/main.rs` dispatches via `IsTerminal`. Keep both paths working.
- Stable design rules (operator precedence, width handling, base propagation, x/z propagation) are documented in README's "Operator precedence", "Width rules", and "Base rules" sections — consult those rather than re-deriving from LRM.

## Meta-rules

- Add LRM edge-case tests as new operators land.
- Update AGENTS.md first when the active phase changes or a phase task is completed.
- Documentation boundary: README.md holds stable, human-facing content (final target/scope, user requirements, LRM clarifications, design rules — operator precedence, width handling, base propagation, x/z propagation). AGENTS.md holds mutable, agent-facing working state (current phase status, current scope, active checklist, these meta-rules). Quick test: if a fact will still hold after the next 3 phases ship, it belongs in README; otherwise here.
- Collapse completed phases to one-line summaries in AGENTS.md; git history is the granular record.
