# Architecture

## Source layout

- `src/main.rs` — CLI entrypoint: parses `--parse-only` / `--max-depth` flags and dispatches to the matching REPL entry point via `IsTerminal`.
- `src/lib.rs` — facade: public API (`Session`, `evaluate_input`, `run_repl` / `run_interactive` / `run_parse_repl` / `run_parse_interactive`, `parse_input` / `parse_input_with_depth`, `DEFAULT_DISPLAY_DEPTH`, `Evaluation`, plus the `value` re-exports), the `Stmt` driver (`apply_stmt`, `apply_decl`, `apply_assign`, the real / real-array assignment helpers, `evaluate_reg_range`), and `RegRange` / `RegValue` / `RegStorage` (the `Vector` / `Array` / `Real` / `RealArray` sum type covering scalar, vector-array, real, and real-array reg storage) session storage.
- `src/value.rs` — `Value` (the `Integer` / `Real` result wrapper), `LogicBit`, `Base`, `DisplayStyle`, `IntegerValue` (incl. width/sign/base/extension logic), bit ↔ bigint helpers, real formatting (`format_real`), and the 4-value bitwise truth tables.
- `src/lexer.rs` — `Token`, `tokenize`, literal text readers.
- `src/parser.rs` — `Stmt` / `Expr` / `LValue` / `SelectKind` / `UnaryOp` / `BinaryOp` AST, `DeclKind` / `DeclName`, the system-call enums (`SystemTask`, `MathFunctionKind`, `RealConversionKind`), `parse_statements` / `parse_expression`, `Parser` + precedence-climbing levels, decl/assign/lvalue helpers, `parse_integer` / `parse_real` and literal-text parsing helpers, plus the AST truncation helpers used by `--parse-only`.
- `src/eval.rs` — `ExprMeta`, `Annotated` / `AnnotatedKind` (parallel-tree cache of result-type meta and real/integer dispatch flag, built once by `annotate()` and consumed by `validate_annotated` and the evaluators), the public entrypoints (`evaluate_expr`, `semantic_check`, `evaluate_assignment_rhs`, `evaluate_constant_expr`), an iterative work-stack evaluator (`run_eval_loop` shared by `evaluate_annotated` for the integer pipeline and `evaluate_annotated_as_real` for the real pipeline, with `visit_eval` / `combine_eval` and `visit_real_eval` / `combine_real_eval` dispatchers), every per-operator evaluator threaded with `&Session`, width/sign propagation (`infer_expr_meta`, `combine_binary_meta`), the lvalue assignment driver (`evaluate_lvalue_assignment`, `lvalue_meta`), the select family (`evaluate_select` and per-form helpers), `evaluate_expr_as_math_bigint`, `evaluate_power`, and reduction folds.
- `src/system_call.rs` — `SystemCallKind` / `SystemFunction` classification (`classify_system_call`), task execution (`execute_task` for `$finish` / `$stop` / `$display` / `$write`), and the `$display` / `$write` format-control walker (`format_display_args` and the `%`-specifier handlers).
- `src/highlight.rs` — lenient span-aware tokenizer (`highlight_spans`, `TokenClass`) feeding the rustyline line highlighter; mirrors the lexer's boundary rules but never errors so partial input doesn't flash red mid-keystroke.
- `src/color.rs` — ANSI color helpers and the rustyline `PromptHelper` (prompt coloring, token coloring, `NO_COLOR` / terminal gating).
- `src/tests.rs` — unit tests, declared via `#[cfg(test)] mod tests;` in `lib.rs`.

## REPL entry points

There are four REPL entry points, all kept in working order — a normal pair and a `--parse-only` pair that stop after the parser and print the AST instead of evaluating:

- `vcal::run_interactive` — rustyline-backed, TTY only.
- `vcal::run_repl(BufRead, Write)` — piped / test input.
- `vcal::run_parse_interactive(depth)` — rustyline-backed `--parse-only` REPL, TTY only.
- `vcal::run_parse_repl(BufRead, Write, depth)` — piped / test `--parse-only` REPL.

`src/main.rs` dispatches between them via `IsTerminal` (TTY vs piped) and the `--parse-only` flag.

## Expression evaluation passes

Every public entry point in `eval.rs` (`evaluate_expr`, `semantic_check`, `evaluate_assignment_rhs`, `evaluate_constant_expr`) runs the same three-phase pipeline:

1. **Annotate** (`annotate`) — single bottom-up walk that produces an `Annotated` tree mirroring the `Expr`. Each node caches its result-type `ExprMeta` (or `None` for real-typed) and structural children. This replaces what used to be redundant per-call walks of `expression_is_real` and `infer_expr_meta` from inside the validator and evaluator at every Binary level.
2. **Validate** (`validate_annotated`) — top-down structural pass that reads `is_real()` / `meta()` from the precomputed annotation, surfacing operator-on-real, $bitstoreal-width, and other semantic errors.
3. **Evaluate** — `evaluate_annotated` for the integer pipeline, `evaluate_annotated_as_real` for the real pipeline. Both drive a single iterative work-stack (`run_eval_loop`) with two value stacks (integer / real) and `EvalTask::Visit` / `EvalTask::Combine` frames, so deep alternation between real and integer subtrees (`$rtoi`, `$itor`, `!real`, real-typed conditional branches, implicit §3.5.3 coercion) doesn't grow the Rust call stack. `visit_eval` / `combine_eval` handle the integer side (Binary routed through `visit_binary_eval` with O(1) meta lookups, no subtree re-walks); `visit_real_eval` / `combine_real_eval` handle the real side; leaf shapes fall back to `evaluate_leaf_expr_in_context`.

The legacy `validate_expr_structure` / `evaluate_leaf_expr_in_context` / `expression_is_real` / `infer_expr_meta` helpers remain in `eval.rs`. `validate_expr_structure` is still used by `validate_select_kind_structure` for the index / range / base / width sub-expressions inside `SelectKind` (which aren't part of the annotated tree); `expression_is_real` and `infer_expr_meta` feed the validator's real-operand checks, the select / lvalue meta walkers, and the integer-vs-real dispatch in the public entry points.
