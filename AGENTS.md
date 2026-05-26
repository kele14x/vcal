# AGENTS.md

## Current State

What works:

- REPL shell
- Integer literals (all LRM forms)
- Real literals and real arithmetic (LRM 3.5.2 / 5.1.5 / Tables 5-2, 5-3); mixed-type promotion (LRM 5.1.7)
- `$finish`/`$stop`
- `$signed()` / `$unsigned()` sign-cast system functions (LRM 5.5)
- Real-conversion system functions: `$rtoi`, `$itor`, `$realtobits`, `$bitstoreal` (LRM 17.7.1 / §3.5.3)
- Math system functions: `$clog2` plus 21 real-math functions (`$ln`/`$log10`/`$exp`/`$sqrt`/`$floor`/`$ceil`, the trig/hyperbolic family, `$pow`/`$atan2`/`$hypot`) per LRM 17.11; real-math wraps libm via Rust's `f64::*` to match the C standard library
- All operators between integers
- Two-pass context (width, signedness) propagation
- Leftmost-base propagation
- `reg [signed] [range] name [= constant_expression] { , name [= constant_expression] }` declarations and blocking assignment `name = expression` (LRM A.2.1.3 / A.2.3 / A.6.2); persistent `Session` carries reg state across REPL turns, each reg preserves declared `msb`/`lsb` metadata for future bit/part-select work, and per-name init values flow through the same RHS path as `=` (real→integer conversion, NaN/±∞ → x). Each init expression is evaluated against the session *before* the new binding replaces the old one, so a redecl with `= name` carries the prior value forward (e.g. `reg [1:0] a = 2'b11; reg a = a` → `1'b1`). The whole decl statement commits all-or-nothing — if any init errors, the live session is left untouched, even for earlier names in the same list. The unpacked `{ dimension }` array form is intentionally out of scope.
- RHS bit-select `r[expr]`, constant part-select `r[m:l]`, and indexed part-selects `r[b +: w]` / `r[b -: w]` on declared regs (LRM 4.2.1 / 5.2.1 / 5.2.2); results are always unsigned (LRM 4.7), width is determined by the select form (1 / `|m-l|+1` / `w`), the reg's stored base is inherited, source→internal index mapping is `internal = |src - lsb_decl|` so forward/reversed/negative-endpoint decls all work, constant part-select direction must strictly match the reg's declared direction, out-of-range bits are filled with `x` per position (in-range bits keep their value, e.g. `reg [3:0] a = 4'b0101; a[4:3]` → `2'bx0`). Literal selects (`4'b1111[0]`) and LHS selects are out of scope.
- `rustyline` history

## Active Scope

- Single-line REPL input only
- Integer and real literals, parentheses
- Identifiers (simple_identifier per LRM 3.7.1) as primaries, with the four RHS select forms on declared regs: bit-select `r[expr]`, constant part-select `r[m:l]`, indexed part-selects `r[b +: w]` / `r[b -: w]`. `reg` is the only declared variable type so far (no `integer`/`real`/`time`, no LHS bit/part selects, no unpacked-array `{ dimension }` form). Per-name init via `name = constant_expression` is supported.
- Top-level `Stmt` layer above `Expr`: declaration, blocking assignment, expression, and the hoisted `$finish`/`$stop` task forms; a `Session` owns the variable map (`RegValue`, not just bare `IntegerValue`) and is threaded through every evaluator entry
- All operators between integers; arithmetic / relational / equality / logical / `?:` between reals (Table 5-2)
  - Arithmetic ops (`+ - * / % **`, unary +, unary -)
  - Relational ops (`<`, `>`, `<=`, `>=`)
  - Equality ops (`==`, `!=`, `===`, `!==`)
  - Logical ops (`!`, `&&`, `||`)
  - Bitwise ops (`~`, `&`, `|`, `^`, `~^`/`^~`)
  - Reduction unaries (`unary & ~& | ~| ^ ~^/^~`)
  - Shift operators `<< >> <<< >>>`
  - Conditional operator `?:` (the only ternary)
  - Concatenation `{a, b, ...}` and replication `{N{...}}`
- System functions:
  - Sign casts (LRM 5.5): `$signed`, `$unsigned`
  - Display-base casts (vcal-specific): `$bin`, `$oct`, `$dec`, `$hex`
  - Real conversions (LRM 17.7.1): `$rtoi`, `$itor`, `$realtobits`, `$bitstoreal`
  - Math (LRM 17.11): `$clog2`; `$ln`, `$log10`, `$exp`, `$sqrt`, `$pow`, `$floor`, `$ceil`, `$sin`, `$cos`, `$tan`, `$asin`, `$acos`, `$atan`, `$atan2`, `$hypot`, `$sinh`, `$cosh`, `$tanh`, `$asinh`, `$acosh`, `$atanh`

## Backlog

See README's "Supported Matrix" for the final target. Phase scoping beyond real numbers (variables, multi-line input, …) is TBD — confirm with the user before starting work outside the active scope.

## Commands

- Run tests with `cargo test`.
- Run the CLI with `cargo run`.
- Build the binary with `cargo build`.

## Structure

- `src/main.rs` is the CLI entrypoint.
- `src/lib.rs` is the facade: public API (`Session`, `evaluate_input`, `run_repl`, `run_interactive`, `Evaluation`, plus the `value` re-exports), the `Stmt` driver (`apply_stmt`, `evaluate_reg_range`), `RegRange`/`RegValue` session storage, and module declarations.
- `src/value.rs` — `LogicBit`, `Base`, `IntegerValue` (incl. width/sign/base/extension logic), bit ↔ bigint helpers, 4-value truth tables.
- `src/lexer.rs` — `Token`, `tokenize`, literal text readers.
- `src/parser.rs` — `Stmt`/`Expr`/`UnaryOp`/`BinaryOp` AST, `parse_statements`, `Parser` + precedence-climbing levels, decl/assign helpers, `parse_integer` and literal-text parsing helpers.
- `src/eval.rs` — `ExprMeta`, `evaluate_expr` (and `evaluate_assignment_rhs` / `evaluate_constant_expr` entrypoints used by the `Stmt` driver), every per-operator evaluator threaded with `&Session`, width/sign propagation (`infer_expr_meta`, `combine_binary_meta`), `evaluate_expr_as_math_bigint`, `evaluate_power`, reduction folds.
- `src/tests.rs` — unit tests, declared via `#[cfg(test)] mod tests;` in `lib.rs`.

## Guidance

- Do not infer scope from README's "Supported Matrix" — many checked boxes are long-term targets, not current scope. Confirm with the user before expanding.
- Two REPL entry points: `vcal::run_interactive` (rustyline, TTY only) and `vcal::run_repl(BufRead, Write)` (piped/test). `src/main.rs` dispatches via `IsTerminal`. Keep both paths working.
- Most of the design rules should be deriving from LRM. However some rules are minor modified because the LRM is ambiguity or self-contradictory。 They are documented in the "Detailed Implementation" section in the README.md — consult those before reading the LRM.
- None LRM features like the REPL are documented in README.md

## Meta-rules

- Add LRM edge-case tests as new operators land.
- Update AGENTS.md first when the active scope changes or a task is completed.
- Documentation boundary:
  - README.md holds stable, human-facing content (final target/scope, user requirements, LRM clarifications, design rules — operator precedence, width handling, base propagation, x/z propagation). Do not edit it without info user.
  - AGENTS.md holds mutable, agent-facing working state (current status, current scope, active checklist, these meta-rules).
  - Quick test: if a fact will still hold after the new feature ship, it belongs in README; otherwise here.
- Collapse completed feature to one-line summaries in AGENTS.md; git history is the granular record.
