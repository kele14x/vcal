# Architecture

## Source layout

- `src/main.rs` — CLI entrypoint.
- `src/lib.rs` — facade: public API (`Session`, `evaluate_input`, `run_repl`, `run_interactive`, `Evaluation`, plus the `value` re-exports), the `Stmt` driver (`apply_stmt`, `evaluate_reg_range`), `RegRange` / `RegValue` session storage, and module declarations.
- `src/value.rs` — `LogicBit`, `Base`, `IntegerValue` (incl. width/sign/base/extension logic), bit ↔ bigint helpers, 4-value truth tables.
- `src/lexer.rs` — `Token`, `tokenize`, literal text readers.
- `src/parser.rs` — `Stmt` / `Expr` / `UnaryOp` / `BinaryOp` AST, `parse_statements`, `Parser` + precedence-climbing levels, decl/assign helpers, `parse_integer` and literal-text parsing helpers.
- `src/eval.rs` — `ExprMeta`, `evaluate_expr` (and `evaluate_assignment_rhs` / `evaluate_constant_expr` entrypoints used by the `Stmt` driver), every per-operator evaluator threaded with `&Session`, width/sign propagation (`infer_expr_meta`, `combine_binary_meta`), `evaluate_expr_as_math_bigint`, `evaluate_power`, reduction folds.
- `src/tests.rs` — unit tests, declared via `#[cfg(test)] mod tests;` in `lib.rs`.

## REPL entry points

There are two REPL entry points, both kept in working order:

- `vcal::run_interactive` — rustyline-backed, TTY only.
- `vcal::run_repl(BufRead, Write)` — piped / test input.

`src/main.rs` dispatches between them via `IsTerminal`.
