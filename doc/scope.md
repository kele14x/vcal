# Scope

This is the **mutable working state**: what works today, what's still planned, and known issues. Agents update this file first when scope changes or a task completes — collapse completed work to one-line summaries; git history is the granular record.

For the long-term LRM-coverage target, see [lrm-coverage.md](lrm-coverage.md).

Priority key: **P0** critical or blocking; **P1** important; **P2** non-blocking and deferrable (may remain unimplemented). No P0 or P1 items are currently documented.

## What works

One line per capability. The linked stable doc holds the detail; do not restate it here.

- **REPL** — prompt, session, exit behavior, interactive editing (Enter submits; Ctrl-J and Option/Alt-Enter insert newlines, Shift-Enter also bound on Windows), output highlighting with `NO_COLOR` support, and the output byte policy including lossy UTF-8 on a Windows console: [repl.md](repl.md)
- **Top-level input** — every expression entry is a `display_expression` argument list: canonical echoes, null comma slots, a string-style first argument driving the format-control engine, and a trailing `;` suppressing the echo: [non-standard.md](non-standard.md#top-level-input)
- **Literals and context** — all LRM integer and real literal forms, leaf extension, two-pass width / signedness propagation, leftmost-base propagation, and real / integer promotion: [expressions.md](expressions.md)
- **String literals** — packed unsigned 8-bit vectors with escaped canonical display and string-only concatenation / replication: [expressions.md](expressions.md#string-literals)
- **Operators** — every operator between integers, plus real arithmetic; `**` takes a self-determined exponent and uses width-bounded power evaluation: [operators.md](operators.md)
- **Variables** — `reg` / `integer` / `real` declarations, `reg` display-base resolution, blocking assignment over the full LRM A.8.5 `variable_lvalue`, bit-/part-/indexed-part-selects, and 1-D unpacked arrays with their element and payload caps: [variables.md](variables.md)
- **Selects on concatenation results** — SystemVerilog-style read-only selects over `{...}` and `{n{...}}`: [non-standard.md](non-standard.md#systemverilog-extension-selects-on-concatenation-results)
- **System tasks** — `$finish`, `$stop`, `$display` / `$write` and the `b`/`o`/`h` suffixed variants over a basic formatting subset, with null arguments and static format-control validation: [non-standard.md](non-standard.md#display-tasks)
- **System functions** — sign casts, real conversions, `$clog2` and the real-math family: [lrm-coverage.md](lrm-coverage.md); vcal-specific display-base casts `$bin` / `$oct` / `$dec` / `$hex`: [non-standard.md](non-standard.md#display-base-cast-functions)
- **Pipeline** — annotate / validate / evaluate over heap work stacks with per-evaluation select caching and no fixed select nesting limit, plus `Syntax error:` / `Semantic error:` diagnostics that preserve the side effects of already-completed statements: [architecture.md](architecture.md), [repl.md](repl.md)

## Active scope

No implementation task is currently active.

Deferred follow-ups:

- Automatic continuation and Cmd-Enter. Ctrl-M retains its existing submit behavior; backslash-Enter continuation is excluded.
- Reserved-word rejection is deprioritized to may-never-implement (see Known issues). Prefer the other P2 items when picking up backlog work.

## Validation status

`cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, the full Rust test suite, the release recursion stress cases, and the Unix PTY checks in `scripts/test_multiline.py` all pass on macOS. Coverage includes deep parse-only pastes (~100,000 levels, ~500 KB) with and without color, truncated AST rendering, canonical versus formatted output, raw non-UTF-8 bytes, `NO_COLOR`, malformed-input recovery, and both interactive modes. Native Windows remains unverified for terminal rendering, Shift-Enter, and physical key mappings.

This is bounded evidence, not a guarantee that no P0 exists. Raising the parse-only AST display depth retains the documented recursive-rendering risk.

## Known issues

- **[P2, deprioritized] LRM-reserved keywords are usable as ordinary identifiers.** `reg if`, `reg while`, `reg module`, `reg always`, `reg case`, `reg endmodule`, `reg begin`, `reg input`, `reg output`, `reg wire` and most of LRM 3.6.4 succeed silently, and `if = 5; if` then evaluates. Only the words the parser actively consumes — `reg`, `signed`, `integer`, `real` — are reserved. Verilog-compatibility cleanup rather than a calculator blocker: lowest priority in this list, may never be implemented, and not a regression to fix opportunistically. Fix if taken up: reject a fixed reserved-word list in identifier positions while preserving the supported declaration syntax; unsupported keywords may fail immediately with a syntax error, and recognizing their statement grammar is out of scope.

- **[P2] Nested replication counts incur cubic work.** Replication annotation validates and evaluates the whole count subtree (`src/eval.rs:548`, `:564`) while validation separately evaluates descendant counts (`:1298`, `:3984`). Generator: `"{"*n + "1'b1" + "{1'b1}}"*n`. Every intermediate result stays one bit, but release-build medians at depths 80 / 160 / 320 were 0.018 / 0.109 / 0.812 s, and depth 640 (~5 KB of input) exceeded 3 s while parse-only took ~4 ms. Fix: retain validated count results within each evaluation rather than re-walking their subtrees, with count-position nesting regressions alongside the existing replication-body stress cases.

- **[P2] Comments between a literal's base and digits are rejected.** `8/*comment*/'d5` succeeds, but `8'd/*comment*/5` and `'h /*comment*/ ff` fail with `Syntax error: missing digits in integer literal`, because `read_base_marker_and_digits` (`src/lexer.rs:543`) treats `/` as a delimiter before reading any value digits. This contradicts the documented comment-separator rule in [repl.md](repl.md); Verilator accepts these forms, Icarus also rejects them. Fix: skip comments between the base token and the first value digit without joining digit runs separated by comments.

- **[P2] Assignments copy unrelated session data.** Declarations, successful integer/vector and real assignments, and valid real-array element writes deep-clone the entire variable map for atomic updates, including unrelated arrays. A release-build probe of 200 scalar assignments measured 0.0062 s without an array versus 0.282 s with `reg [255:0] bulk [0:65535]`. Performance only, not correctness. Fix: stage only affected bindings / elements while preserving statement atomicity and resolving RHS values and LHS indices against the original session.

- **[P2] Malformed real literals produce misleading diagnostics.** `1._0` and `9.` surface as `invalid decimal digits: 1.0` after the underscore-strip / digit-strip step, because the lexer's `real_after_dot` lookahead requires `.` followed by a digit and otherwise falls through to the integer path. Correct in spirit, misleading in wording. Fix: recognize "digit-run + `.`" as a real-literal commitment and emit a real-specific error.

- **[P2] Native Shift-Enter newline support is missing on Linux/macOS** (confirmed in kitty on macOS). Rustyline's Unix backend does not enable and decode enhanced keyboard reporting, so the editor cannot reliably distinguish Shift-Enter from Enter; Ctrl-J and Option/Alt-Enter provide explicit newlines and Shift-Enter is bound only on Windows. Deferred: extend or replace the editor backend with enhanced keyboard reporting while retaining editing, history, highlighting, and bracketed paste.

- **[P2] The stage-prefix invariant is enforced by convention, not structure.** `evaluate_expr` (`src/eval.rs:811`, `:813`) and `evaluate_assignment_rhs` (`:848`, `:859`) prefix the annotate and validate steps with `Semantic error:` but return the evaluate step's `Err` verbatim, so an error raised there reaches the REPL unprefixed. Candidate sites: the replication-count checks (`src/eval.rs:3326`, `:3330`, `:3337`), the real-as-integer leaf rejection (`:1650`), and `require_vector` (`src/lib.rs:109`, `:111`). Latent rather than observable — no input found so far reaches them, because the validator rejects the same cases first. Fix: replace the internal `Result<_, String>` with a typed `StageError` (`Syntax` / `Semantic`, later `Runtime`) rendered exactly once at the sole convergence point (`src/lib.rs:374`, `:384`), making classification a compile-time obligation at the raise site and retiring the deliberately unprefixed helpers (`evaluate_constant_expr` `:874`, `evaluate_subexpr_as_integer` `:887`, `replication_count_value` `:3984`). Cost: ~111 signatures across `eval.rs` (51), `parser.rs` (23), `lib.rs` (17), `lexer.rs` (11), `system_call.rs` (8), `value.rs` (1), plus deleting the 15 hand-written prefix sites; the public contract (`EvalError.message` stays a `String`) is unchanged. Must be migrated wholesale — a partial migration keeps two conventions alive with `From`-style conversions at the seams, worse than either endpoint. **Deferred until a genuine second error category lands** (file I/O, `$fopen`, anything whose success is only known at execution time): until then the enum defends a property with zero observed violations. The lighter alternative — defaulting unprefixed messages to `Semantic error:` behind a `debug_assert!` — was rejected: it guesses wrong once a runtime category exists, and substitutes runtime self-audit for what the tests should guarantee.
