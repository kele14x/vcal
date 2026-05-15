# AGENTS.md

## Current State

- Phases 1 and 2 complete; Phase 3 (relational operators) is the active phase.
- Phase 1 — REPL shell, integer literals (all LRM forms), `$finish`/`$stop`. Done.
- Phase 2 — arithmetic ops (`+ - * / % **`, unary), two-pass width handling, leftmost-base propagation, rustyline history. Done.
- Phase 3 — relational operators (`<`, `>`, `<=`, `>=`). In progress; checklist below.

## Current Scope

- Single-line REPL input only
- Integer literals and parentheses
- Integer arithmetic operators (`+`, `-`, `*`, `/`, `%`, `**`, plus unary `+` / `-`)
- No variables, declarations, strings, real numbers, concatenation
- No equality (`==`/`!=`/`===`/`!==`), no logical (`&&`/`||`/`!`), no bitwise, no shifts, no variables, no strings, no real numbers

## Active Phase: Phase 3 target

REPL UX:

- (no REPL UX changes scoped for Phase 3)

LRM features:

- [ ] Relational operators from IEEE 1364-2005 section 5.1.7:
  - [ ] `<`
  - [ ] `>`
  - [ ] `<=`
  - [ ] `>=`
- [ ] Tokenize multi-character `<=` and `>=` correctly
- [ ] Support relational operator precedence and left-associativity per LRM Table 22 (below additive, above the future equality level)
- [ ] Size both operands to the larger of their two widths, treating the comparison as unsigned if either operand is unsigned
- [ ] Isolate relational operands from outer context width (a wider parent context cannot widen the operand evaluation, unlike arithmetic)
- [ ] Produce a 1-bit unsigned result that zero-extends when used in a wider parent context
- [ ] Propagate `x`/`z` through relational operators as a `1'bx` result
- [ ] Implement signed vs unsigned comparison per LRM 5.1.7
- [ ] Add unit tests for precedence, associativity, signed/unsigned mixing, x/z propagation, result widening, and `<=` parsing as relational

Non-LRM features:

- [ ] Render the 1-bit relational result always in binary (`1'b1` / `1'b0` / `1'bx`); the leftmost-wins base rule does not apply

Implementation notes:

- The result is intrinsically 1-bit unsigned; outer context can only zero-extend the result, never widen the comparison itself.
- Operand widening is bounded by the larger of the two operand widths — internal arithmetic inside an operand may still widen up to that bound, but no further.
- `<=` is relational only in Phase 3 — once variables and procedural blocks land, the parser will need to disambiguate against non-blocking assignment by context. This is a forward-compatibility concern, not a Phase 3 task.
- The exponent of `**` remains self-determined; relational subexpressions used as `**` exponent are still self-determined per LRM.

## Backlog

See README's "Supported Matrix" for the final target. Phase scoping for everything beyond Phase 3 is TBD — confirm with the user before starting work outside the active phase.

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
