# AGENTS.md

## Current State

- Phases 1–10 complete. No active phase; next-phase scope is TBD — confirm with user before starting new work.
- Phase 1 — REPL shell, integer literals (all LRM forms), `$finish`/`$stop`. Done.
- Phase 2 — arithmetic ops (`+ - * / % **`, unary), two-pass width handling, leftmost-base propagation, rustyline history. Done.
- Phase 3 — relational ops (`<`, `>`, `<=`, `>=`) with LRM 5.5.2 propagated-context unification (zero-extend at the leaf primary when context is unsigned, sign-extend when signed), 1-bit unsigned result, x/z propagation. Done.
- Phase 4 — equality ops (`==`, `!=`, `===`, `!==`) sharing the relational unification path; per-bit ambiguity for `==`/`!=` (a definite mismatch defeats x), bit-for-bit including x/z for `===`/`!==`; corrected `context_extension_bit` to zero-fill under unsigned propagated context. Done.
- Phase 5 — logical ops (`!`, `&&`, `||`) with self-determined operands reduced to a 1-bit logical value (any `1` → 1, all `0` → 0, any x/z without a `1` → x), LRM §5.1.9 truth tables, 1-bit unsigned binary result that widens through outer arithmetic context like relational/equality. Done.
- Phase 6a — per-bit bitwise ops (`~`, `&`, `|`, `^`, `~^`/`^~`). New parser band `&` > `^/~^/^~` > `|` between `==` and `&&`; LRM §5.1.10 truth tables zipped per position with no all-x short-circuit; context-determined width like arithmetic; bare `&`/`|` now lex as bitwise (the Phase 5 reject is replaced); `~^` and `^~` collapse to one token. Done.
- Phase 6b — reduction unaries (`unary & ~& | ~| ^ ~^/^~`). New `~&`/`~|` tokens; binary `&`/`|`/`^`/`~^`/`^~` tokens are reused at unary position via parse-position disambiguation (no token rewrite). Single `reduce_bits` helper folds operand bits via the binary 4-state truth tables from 6a; identity element is `1` for AND-fold and `0` for OR/XOR; negated forms invert the fold. Self-determined 1-bit unsigned result that widens through outer arithmetic context like `!`/`&&`/`||`/relational/equality. `~&`/`~|` are unary-only (no binary parse level consumes them, so `a ~& b` correctly errors). Done.
- Phase 7 — shift operators `<< >> <<< >>>`. New parse band `parse_shift` between additive and relational (LRM Table 5-4); greedy lex of `<<<`/`<<` and `>>>`/`>>` doesn't disturb `<=`/`>=`/`<`/`>`. LHS context-determined like arithmetic; RHS self-determined and read as unsigned bits — never widens the LHS or flips result signedness (LRM §5.1.12). Shift count clamped to the result width so `BigUint`-sized counts saturate to all-fill. RHS x/z poisons the whole result to all-x. `<<`/`<<<`/`>>` always zero-fill; `>>>` fills with the LHS sign bit only when the **propagated** context is signed (so an unsigned outer context turns `>>>` into a logical right shift, matching iverilog). Done.
- Phase 8 — conditional operator `?:` (the only ternary, LRM §5.1.13). New `Expr::Conditional` AST variant and `parse_conditional` slot between `parse_expression` and `parse_logical_or`; right-associative via recursive descent on the else branch. Cond is self-determined and reduced to a 1-bit logical (reuses `logical_value`). then/else are context-determined, unify width = `max(L(then), L(else), L(context))` and signedness per §5.5.1, with the same propagated-context override as the shift path so a mixed unsigned outer context zero-fills both branches. When cond is x/z, both branches evaluate and merge per bit (agreeing bits stay, disagreeing → x). Result base inherits from the then branch. Side effect: `is_expression_delimiter` now lists `:`, and the based-literal post-apostrophe loop terminates on whitespace once `saw_digit` is true so `1'b0 ? 4'd5 : 4'd9` no longer swallows the `?` and `4'd5` as digits (`?` stays a valid z alias inside contiguous literal text). Done.
- Phase 9 — unsized constant leaf-extension per LRM Table 5-22 footnote a. New `unsized_literal: bool` on `IntegerValue`, set true only by the unsized parser branches (`parse_unsized_decimal` and the `width_hint.is_none()` arms of `parse_based_decimal`/`parse_based_radix`); all computed values and `resized_to_context` output set it false. `resized_to_context` branches: unsized + wider context → new `extend_unsized_to` (MSB-fill if x/z, else fill per the literal's own signedness — independent of propagated context); sized or equal-width → existing §5.5.4 path. Fixes iverilog divergence on unsized x/z leaves in wider unsigned contexts (`'bx | 64'b0` → `64'bx…x`) and on unsized signed leaves with MSB=1 in mixed-signedness contexts (`'shFFFFFFFF | 64'b0` → `64'hFFFF_FFFF_FFFF_FFFF`). Outer context still propagates to leaves through context-determined sub-expressions, so `('bx | 4'b0) | 64'b0` is also all-x. Sized operands keep §5.5.4 (e.g. `32'sbx | 34'b0` = `34'b00xx…x`). Done.
- Phase 10 — concatenation `{a, b, ...}` and replication `{N{...}}` (LRM 5.1.14). New `LBrace`/`RBrace`/`Comma` tokens (added to `is_expression_delimiter` so adjacent literals split correctly); new `Expr::Concatenation { items }` and `Expr::Replication { count, items }` AST variants. `parse_brace_primary` disambiguates the two forms by what follows the first inner expression (`{` → replication, else `,`/`}` → plain concatenation). Operands are self-determined (no context propagated down) and joined MSB-first (leftmost item to high bits); result is always unsigned, base inherits leftmost-wins, and outer context only zero-extends the joined result. Indefinite-width rejection (LRM 5.1.14: "unsized constant numbers shall not be allowed in concatenations") via `is_indefinite_width` — the unsized flag propagates through context-determined operators (arithmetic/bitwise/power/shift LHS/conditional/unary +/-/~) but stops at every 1-bit-result operator (relational/equality/logical/reduction) and at concatenation/replication themselves. Replication count is non-x/z, non-negative; zero is permitted only inside an enclosing concatenation that has at least one positive-size operand (LRM 5.1.14 footnote: "Such a replication shall appear only within a concatenation in which at least one of the operands of the concatenation has a positive size."). `evaluate_replication_count_allow_zero` is the lenient helper used by `infer_expr_meta` and by `evaluate_concatenation_item_bits` (which looks through `Grouped` to find a Replication child); the strict `evaluate_replication_count` rejects zero at top-level Replication. After joining bits, `collect_concatenation_bits` errors if the result is empty — that's the case where every operand collapsed to zero width. Done.

## Current Scope

- Single-line REPL input only
- Integer literals and parentheses
- Integer arithmetic operators (`+`, `-`, `*`, `/`, `%`, `**`, plus unary `+` / `-`)
- Relational operators (`<`, `>`, `<=`, `>=`) — 1-bit unsigned binary result
- Equality operators (`==`, `!=`, `===`, `!==`) — 1-bit unsigned binary result
- Logical operators (`!`, `&&`, `||`) — 1-bit unsigned binary result with x/z reduction
- No variables, declarations, strings, real numbers
- Bitwise (per-bit) operators (`~`, `&`, `|`, `^`, `~^`/`^~`)
- Reduction unaries (`unary & ~& | ~| ^ ~^/^~`)
- Shift operators (`<<`, `>>`, `<<<`, `>>>`) — LHS context-determined, RHS self-determined and treated as unsigned (LRM §5.1.12)
- Conditional operator (`?:`) — right-associative ternary; cond self-determined and reduced to 1-bit logical, then/else context-determined; ambiguous-cond merge is per-bit
- Concatenation (`{a, b, ...}`) and replication (`{N{...}}`) — operands self-determined and must have definite width (no unsized literals); result always unsigned

## Active Phase

No active phase. Confirm next-phase scope with the user before starting new work — variables/declarations are the remaining long-term backlog item.

## Backlog

See README's "Supported Matrix" for the final target. Phase scoping beyond concatenation (variables, multi-line input, real numbers, system functions, …) is TBD — confirm with the user before starting work outside the active phase.

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
