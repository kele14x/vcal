# AGENTS.md

## Current State

What works:

- REPL shell
- Integer literals (all LRM forms)
- `$finish`/`$stop`
- All operators between integers
- Two-pass context (width, signedness) propagation
- Leftmost-base propagation
- `rustyline` history

## Active Scope

- Single-line REPL input only
- Integer literals and parentheses
- No variables, declarations, strings, real numbers
- All operators between integers
  - Arithmetic ops (`+ - * / % **`, unary +, unary -)
  - Relational ops (`<`, `>`, `<=`, `>=`)
  - Equality ops (`==`, `!=`, `===`, `!==`)
  - Logical ops (`!`, `&&`, `||`)
  - Bitwise ops (`~`, `&`, `|`, `^`, `~^`/`^~`)
  - Reduction unaries (`unary & ~& | ~| ^ ~^/^~`)
  - Shift operators `<< >> <<< >>>`
  - Conditional operator `?:` (the only ternary)
  - Concatenation `{a, b, ...}` and replication `{N{...}}`

## Backlog

See README's "Supported Matrix" for the final target. Phase scoping beyond concatenation (variables, multi-line input, real numbers, system functions, …) is TBD — confirm with the user before starting work outside the active scope.

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
