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
- `rustyline` history

## Active Scope

- Single-line REPL input only
- Integer and real literals, parentheses
- No variables, declarations, strings
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
- `src/lib.rs` is the facade: public API (`evaluate_input`, `run_repl`, `run_interactive`, `Evaluation`, plus the `value` re-exports), the driver (`parse_line`, `parse_system_task`), and module declarations.
- `src/value.rs` — `LogicBit`, `Base`, `IntegerValue` (incl. width/sign/base/extension logic), bit ↔ bigint helpers, 4-value truth tables.
- `src/lexer.rs` — `Token`, `tokenize`, literal text readers.
- `src/parser.rs` — `Expr`/`UnaryOp`/`BinaryOp` AST, `Parser` + precedence-climbing levels, `parse_integer` and literal-text parsing helpers.
- `src/eval.rs` — `ExprMeta`, `evaluate_expr` and every per-operator evaluator, width/sign propagation (`infer_expr_meta`, `combine_binary_meta`), `evaluate_expr_as_math_bigint`, `evaluate_power`, reduction folds.
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
