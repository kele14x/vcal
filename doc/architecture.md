# Architecture

## Source layout

- `src/main.rs` — CLI entrypoint.
- `src/lib.rs` — facade: public API (`Session`, `evaluate_input`, `run_repl`, `run_interactive`, `Evaluation`, plus the `value` re-exports), the `Stmt` driver (`apply_stmt`, `evaluate_reg_range`), `RegRange` / `RegValue` / `RegStorage` (the `Vector` vs `Array` sum type for the unpacked-dim reg storage) session storage, and module declarations.
- `src/value.rs` — `LogicBit`, `Base`, `IntegerValue` (incl. width/sign/base/extension logic), bit ↔ bigint helpers, 4-value truth tables.
- `src/lexer.rs` — `Token`, `tokenize`, literal text readers.
- `src/parser.rs` — `Stmt` / `Expr` / `UnaryOp` / `BinaryOp` AST, `parse_statements`, `Parser` + precedence-climbing levels, decl/assign helpers, `parse_integer` and literal-text parsing helpers.
- `src/eval.rs` — `ExprMeta`, `Annotated` / `AnnotatedKind` (parallel-tree cache of result-type meta and real/integer dispatch flag, built once by `annotate()` and consumed by `validate_annotated` and `evaluate_annotated`), `evaluate_expr` (and `evaluate_assignment_rhs` / `evaluate_constant_expr` entrypoints used by the `Stmt` driver), every per-operator evaluator threaded with `&Session`, width/sign propagation (`infer_expr_meta`, `combine_binary_meta`), `evaluate_expr_as_math_bigint`, `evaluate_power`, reduction folds.
- `src/tests.rs` — unit tests, declared via `#[cfg(test)] mod tests;` in `lib.rs`.

## REPL entry points

There are two REPL entry points, both kept in working order:

- `vcal::run_interactive` — rustyline-backed, TTY only.
- `vcal::run_repl(BufRead, Write)` — piped / test input.

`src/main.rs` dispatches between them via `IsTerminal`.

## Expression evaluation passes

Every public entry point in `eval.rs` (`evaluate_expr`, `semantic_check`, `evaluate_assignment_rhs`, `evaluate_constant_expr`) runs the same three-phase pipeline:

1. **Annotate** (`annotate`) — single bottom-up walk that produces an `Annotated` tree mirroring the `Expr`. Each node caches its result-type `ExprMeta` (or `None` for real-typed) and structural children. This replaces what used to be redundant per-call walks of `expression_is_real` and `infer_expr_meta` from inside the validator and evaluator at every Binary level.
2. **Validate** (`validate_annotated`) — top-down structural pass that reads `is_real()` / `meta()` from the precomputed annotation, surfacing operator-on-real, $bitstoreal-width, and other semantic errors.
3. **Evaluate** — `evaluate_annotated` for the integer pipeline, `evaluate_expr_as_real` for the real pipeline. The integer dispatch routes `Binary` through `evaluate_binary_annotated` (annotated children → O(1) meta lookups, no subtree re-walks); other shapes fall back to the legacy `evaluate_expr_in_context` for now since they aren't on the chain spine that motivated the refactor.

The legacy `validate_expr_structure` / `evaluate_expr_in_context` / `expression_is_real` / `infer_expr_meta` helpers remain in `eval.rs` and are still used by Select sub-expressions (the index/range expressions inside `SelectKind`, which aren't part of the annotated tree) and by the real evaluator.
