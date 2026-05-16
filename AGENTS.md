# AGENTS.md

## Current State

- Phases 1–7 complete. No active phase; next-phase scope is TBD — confirm with user before starting new work.
- Phase 1 — REPL shell, integer literals (all LRM forms), `$finish`/`$stop`. Done.
- Phase 2 — arithmetic ops (`+ - * / % **`, unary), two-pass width handling, leftmost-base propagation, rustyline history. Done.
- Phase 3 — relational ops (`<`, `>`, `<=`, `>=`) with LRM 5.5.2 propagated-context unification (zero-extend at the leaf primary when context is unsigned, sign-extend when signed), 1-bit unsigned result, x/z propagation. Done.
- Phase 4 — equality ops (`==`, `!=`, `===`, `!==`) sharing the relational unification path; per-bit ambiguity for `==`/`!=` (a definite mismatch defeats x), bit-for-bit including x/z for `===`/`!==`; corrected `context_extension_bit` to zero-fill under unsigned propagated context. Done.
- Phase 5 — logical ops (`!`, `&&`, `||`) with self-determined operands reduced to a 1-bit logical value (any `1` → 1, all `0` → 0, any x/z without a `1` → x), LRM §5.1.9 truth tables, 1-bit unsigned binary result that widens through outer arithmetic context like relational/equality. Done.
- Phase 6a — per-bit bitwise ops (`~`, `&`, `|`, `^`, `~^`/`^~`). New parser band `&` > `^/~^/^~` > `|` between `==` and `&&`; LRM §5.1.10 truth tables zipped per position with no all-x short-circuit; context-determined width like arithmetic; bare `&`/`|` now lex as bitwise (the Phase 5 reject is replaced); `~^` and `^~` collapse to one token. Done.
- Phase 6b — reduction unaries (`unary & ~& | ~| ^ ~^/^~`). New `~&`/`~|` tokens; binary `&`/`|`/`^`/`~^`/`^~` tokens are reused at unary position via parse-position disambiguation (no token rewrite). Single `reduce_bits` helper folds operand bits via the binary 4-state truth tables from 6a; identity element is `1` for AND-fold and `0` for OR/XOR; negated forms invert the fold. Self-determined 1-bit unsigned result that widens through outer arithmetic context like `!`/`&&`/`||`/relational/equality. `~&`/`~|` are unary-only (no binary parse level consumes them, so `a ~& b` correctly errors). Done.
- Phase 7 — shift operators `<< >> <<< >>>`. New parse band `parse_shift` between additive and relational (LRM Table 5-4); greedy lex of `<<<`/`<<` and `>>>`/`>>` doesn't disturb `<=`/`>=`/`<`/`>`. LHS context-determined like arithmetic; RHS self-determined and read as unsigned bits — never widens the LHS or flips result signedness (LRM §5.1.12). Shift count clamped to the result width so `BigUint`-sized counts saturate to all-fill. RHS x/z poisons the whole result to all-x. `<<`/`<<<`/`>>` always zero-fill; `>>>` fills with the LHS sign bit only when the **propagated** context is signed (so an unsigned outer context turns `>>>` into a logical right shift, matching iverilog). Done.

## Current Scope

- Single-line REPL input only
- Integer literals and parentheses
- Integer arithmetic operators (`+`, `-`, `*`, `/`, `%`, `**`, plus unary `+` / `-`)
- Relational operators (`<`, `>`, `<=`, `>=`) — 1-bit unsigned binary result
- Equality operators (`==`, `!=`, `===`, `!==`) — 1-bit unsigned binary result
- Logical operators (`!`, `&&`, `||`) — 1-bit unsigned binary result with x/z reduction
- No variables, declarations, strings, real numbers, concatenation
- Bitwise (per-bit) operators (`~`, `&`, `|`, `^`, `~^`/`^~`)
- Reduction unaries (`unary & ~& | ~| ^ ~^/^~`)
- Shift operators (`<<`, `>>`, `<<<`, `>>>`) — LHS context-determined, RHS self-determined and treated as unsigned (LRM §5.1.12)

## Active Phase

No active phase. Confirm next-phase scope with the user before starting new work — the conditional operator (`?:`), concatenation/replication (`{} {{}}`), and variables/declarations are all in the long-term backlog.

## Backlog

See README's "Supported Matrix" for the final target. Phase scoping beyond bitwise (shifts, conditional, concatenation, variables, …) is TBD — confirm with the user before starting work outside the active phase.

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
