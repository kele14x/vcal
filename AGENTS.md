# AGENTS.md

## Current State

- This repo is now a single Rust binary crate named `vcal`.
- Phase 1 is complete.
- Phase 2 is complete: arithmetic evaluator (IEEE 1364-2005 section 5.1.5), prompt spacing, and `rustyline`-backed in-memory arrow-key history. See `TODO.md` for the per-task checklist.
- The next phase is not yet scoped. Confirm scope with the user before introducing anything from `TODO.md` "Things to implement later" (variables, declarations, real numbers, non-arithmetic operators, strings, vectors, additional system tasks).

## Commands

- Run tests with `cargo test`.
- Run the CLI with `cargo run`.
- Build the binary with `cargo build`.

## Structure

- `src/main.rs` is the CLI entrypoint.
- `src/lib.rs` holds the REPL and evaluation logic so behavior can be unit tested without spawning the binary.

## Guidance

- Do not infer support from the top-level support matrix in `README.md`; many checked items are long-term targets and are intentionally out of scope for the current phase. Confirm with the user before expanding scope.
- Implement expressions with a parsed AST, not ad hoc string splitting.
- Keep expression width/sign handling as a separate analysis step from numeric evaluation.
- Prefer a two-pass design for expressions:
  - bottom-up self-determined width/sign inference for each AST node
  - top-down context-determined evaluation so parent width can widen child arithmetic
- For arithmetic operators, if any operand contains `x` or `z`, the result becomes all `x` bits at the effective result width.
- Result base follows the vcal leftmost-wins rule (see README "Base rules"): unary preserves the operand's base; binary takes the LHS base. Decimal is the default for unsized literals.
- Operator precedence and associativity follow IEEE 1364-2005 Table 22 — notably, unary binds tighter than `**`, and `**` is left-associative.
- Preserve the canonical output contract: prompts are `In[n]: ` and `Out[n]: ` (trailing space); print `Out[n]: ` with an empty value for system tasks like `$finish`/`$stop`.
- Interactive entry point is `vcal::run_interactive` (rustyline, TTY only); piped/test input uses the generic `vcal::run_repl(BufRead, Write)`. `src/main.rs` dispatches via `IsTerminal`. Keep both paths working.
