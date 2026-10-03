# Architecture

## Source layout

- `src/main.rs` — CLI entrypoint: parses `--parse-only` / `--max-depth` flags and dispatches to the matching REPL entry point via `IsTerminal`.
- `src/lib.rs` — facade: public API (`Session`, `evaluate_input`, `run_repl` / `run_interactive` / `run_parse_repl` / `run_parse_interactive`, `parse_input` / `parse_input_with_depth`, `DEFAULT_DISPLAY_DEPTH`, `Evaluation` with text and raw-byte REPL value output, plus the `value` re-exports), the `Stmt` driver (`apply_stmt`, `apply_decl`, `apply_assign`, the real / real-array assignment helpers, `evaluate_reg_range`), and `RegRange` / `RegValue` / `RegStorage` (the `Vector` / `Array` / `Real` / `RealArray` sum type covering scalar, vector-array, real, and real-array reg storage) session storage.
- `src/value.rs` — `Value` (the `Integer` / `Real` result wrapper), `LogicBit`, `Base`, `DisplayStyle`, `IntegerValue` (incl. width/sign/base/extension logic), bit ↔ bigint helpers, real formatting (`format_real`), and the 4-value bitwise truth tables.
- `src/lexer.rs` — `Token`, `tokenize`, literal text readers.
- `src/parser.rs` — `Stmt` (including the unified `ReplEcho(Vec<SystemArg>)` top-level expression-list form) / `Expr` / `LValue` / `SelectKind` / `UnaryOp` / `BinaryOp` AST, `DeclKind` / `DeclName`, the system-call enums (`SystemTask`, `MathFunctionKind`, `RealConversionKind`), `parse_statements` / `parse_expression`, `Parser` + iterative Pratt `Pending` frames (including select operands), decl/assign/lvalue helpers, `parse_integer` / `parse_real` and literal-text parsing helpers, plus the AST truncation helpers used by `--parse-only`.
- `src/eval.rs` — `ExprMeta`, `Annotated` / `AnnotatedKind` (parallel-tree cache of result-type meta and real/integer dispatch flag, built once by `annotate()` and consumed by `validate_annotated` and the evaluators), the public entrypoints (`evaluate_expr`, `semantic_check`, `evaluate_assignment_rhs`, `evaluate_constant_expr`), an iterative work-stack evaluator (`run_eval_loop` shared by `evaluate_annotated` for the integer pipeline and `evaluate_annotated_as_real` for the real pipeline, with `visit_eval` / `combine_eval` and `visit_real_eval` / `combine_real_eval` dispatchers), every per-operator evaluator threaded with `&Session`, width/sign propagation (`binary_result_meta`, `combine_binary_meta`), the lvalue assignment driver (`evaluate_lvalue_assignment`, `lvalue_meta`), the select family (`AnnotatedSelect`, `ResolvedSelectKind`, `annotate_select`, `evaluate_resolved_select`, and `apply_resolved_select`), width-bounded integer power evaluation, and reduction folds.
- `src/system_call.rs` — `SystemCallKind` / `SystemFunction` classification (`classify_system_call`), task execution (`execute_task` for `$finish` / `$stop` / `$display` / `$write`), the shared `$display` / `$write` format-control walker, and `format_repl_echo_args` for canonical unformatted lists plus leading-string format mode.
- `src/highlight.rs` — lenient span-aware tokenizer (`highlight_spans`, `TokenClass`) feeding the rustyline line highlighter; mirrors the lexer's boundary rules but never errors so partial input doesn't flash red mid-keystroke.
- `src/color.rs` — ANSI color helpers and the rustyline `PromptHelper` (prompt coloring, token coloring, `NO_COLOR` / terminal gating).
- `src/tests/` — unit tests, declared via `#[cfg(test)] mod tests;` in `lib.rs`. `mod.rs` lists the submodules; each `tests/<area>.rs` (literals, strings, parser_ast, arithmetic, system_tasks, display, repl, repl_echo, relational, logical, bitwise, shift, conditional, concat, casts, real, real_functions, variables, selects, lvalue, arrays, integer_real_decls, deep_nesting, limits) is self-contained with its own `use` imports, grouped by operator / feature area.

## REPL entry points

There are four REPL entry points, all kept in working order — a normal pair and a `--parse-only` pair that stop after the parser and print the AST instead of evaluating:

- `vcal::run_interactive` — rustyline-backed, TTY only.
- `vcal::run_repl(BufRead, Write)` — piped / test input.
- `vcal::run_parse_interactive(depth)` — rustyline-backed `--parse-only` REPL, TTY only.
- `vcal::run_parse_repl(BufRead, Write, depth)` — piped / test `--parse-only` REPL.

`src/main.rs` dispatches between them via `IsTerminal` (TTY vs piped) and the `--parse-only` flag.

## Expression parsing

`parse_expr_bp` uses a heap `Vec<Pending>` for grouping, operators, system calls, concatenation/replication, and select operands. Select brackets have no fixed nesting limit; their first operand, range endpoint or indexed width, and optional chained array-element select all reduce on this same stack. The separate parse-only display-depth limit bounds recursive debug rendering rather than expression parsing.

## Expression evaluation passes

Every public entry point in `eval.rs` (`evaluate_expr`, `semantic_check`, `evaluate_assignment_rhs`, `evaluate_constant_expr`) runs the same three-phase pipeline:

1. **Annotate** (`annotate`) — single bottom-up walk that produces an `Annotated` tree mirroring the `Expr`. Each node caches its result-type `ExprMeta` (or `None` for real-typed) and structural children. This replaces what used to be redundant per-call real-type / meta walks from inside the validator and evaluator at every Binary level.
2. **Validate** (`validate_annotated`) — top-down structural pass that reads `is_real()` / `meta()` from the precomputed annotation, surfacing operator-on-real, $bitstoreal-width, and other semantic errors.
3. **Evaluate** — `evaluate_annotated` for the integer pipeline, `evaluate_annotated_as_real` for the real pipeline. Both drive a single iterative work-stack (`run_eval_loop`) with two value stacks (integer / real) and `EvalTask::Visit` / `EvalTask::Combine` frames, so deep alternation between real and integer subtrees (`$rtoi`, `$itor`, `!real`, real-typed conditional branches, implicit §3.5.3 coercion) doesn't grow the Rust call stack. `visit_eval` / `combine_eval` handle the integer side (Binary routed through `visit_binary_eval` with O(1) meta lookups, no subtree re-walks); `visit_real_eval` / `combine_real_eval` handle the real side; leaf shapes fall back to `evaluate_leaf_expr_in_context`.

`SelectKind` index / range / base / width sub-expressions are children of `AnnotatedKind::Select`. The bottom-up `annotate_select` combiner validates each operand once before resolving part-select endpoints or indexed widths with the existing iterative evaluator. Those bound values and widths live in `ResolvedSelectKind`. The later validator treats the select as already checked, avoiding another traversal of its descendants. When an input contains several semantic errors, this bottom-up operand checking can change which diagnostic appears first; errors still carry the semantic-stage prefix.

Bit indices and indexed bases remain self-determined and are scheduled by `push_select_eval` on the shared evaluator work stack, including indices of real-array elements. Each `AnnotatedSelect` has a lazy `OnceCell<Value>` holding its natural result before outer width/sign extension. Enclosing select bounds reuse that result instead of evaluating descendant selects again. Annotations and their caches are local to one pipeline invocation against an unchanged `Session`; later statements and later inputs build fresh annotations. `Annotated` drops select operand trees through the same iterative child-draining path as other expression shapes.

Lvalue shape/index helpers still accept raw `Expr` operands, but each such operand goes through the annotated pipeline, so nested selects inside it use these same cached, iterative paths. The remaining raw-`Expr` helpers include `expression_is_real` (lvalue and declaration checks) and `is_indefinite_width` (the concatenation-operand check in `validate_annotated`).
