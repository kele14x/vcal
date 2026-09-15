# Scope

This is the **mutable working state**: what works today, what's still planned, and known issues. Agents update this file first when scope changes or a task completes — collapse completed features to one-line summaries; git history is the granular record.

For the long-term LRM-coverage target, see [lrm-coverage.md](lrm-coverage.md).

## What works

- REPL shell with `rustyline` history
- Output byte policy: `$display` / `$write` and formatted `display_expression` output is a raw byte stream everywhere (pipes, files, Unix terminals stay byte-exact); a Windows console degrades non-UTF-8 bytes to lossy UTF-8 instead of erroring (see [repl.md](repl.md))
- Unified non-empty `display_expression` echo lists (see [non-standard.md](non-standard.md)): every top-level expression input is a display-expression argument list, so `a` retains canonical calculator output, `a, b` echoes both values canonically, null comma slots emit one space each like `$display`, and a leading string-style expression such as `"a=%d", a` uses the `$display` format-control engine while rendering arguments left unconsumed by format controls canonically; trailing `;` suppresses the whole echo
- Integer and real literals, all LRM forms (LRM 3.5.2)
- String literals as packed unsigned 8-bit vectors (LRM 3.6 / A.8.8), with friendly escaped display for bare strings and string-only concatenation / replication; the formatter shared by `$display` / `$write` and formatted `display_expression` supports basic format strings, null arguments, and raw-byte output; internal zero-width numeric display renders with one `0` digit
- All operators between integers (see [operators.md](operators.md))
- Integer `**` evaluates its exponent as a self-determined expression and uses width-bounded modular exponentiation, so nested or very large exponents wrap correctly without unbounded intermediates
- Real arithmetic and mixed integer/real promotion (LRM 5.1.5 / 5.1.7 / Tables 5-2, 5-3)
- Two-pass context (width, signedness) propagation; leftmost-base propagation; `reg` display base starts as a weak binary fallback and resolves from the first whole-reg integer init/assignment
- `reg` / `integer` / `real` declarations and blocking assignment with the full LRM A.8.5 `variable_lvalue` — bare name, bit/part/indexed-part selects, and arbitrarily nested concatenations on the LHS (see [variables.md](variables.md))
- 1-D unpacked arrays on `reg` / `integer` / `real` (LRM 4.9 / A.2.2.1); arrays are capped at 65,536 elements, and vector-array payload is additionally capped at the same 16,777,216-bit limit as scalar vectors
- Static-semantic validation as a top-level pre-pass over every expression entry and every display argument list — errors prefixed `Syntax error:` (lex/parse) or `Semantic error:` (static semantics), the latter including display format-control rejections
- Error diagnostics preserve side effects: `$display` / `$write` bytes from statements that completed earlier in the same input are emitted ahead of the diagnostic, while the failing statement and every later one do not run and no value is echoed (see [repl.md](repl.md))
- System tasks: `$finish`, `$stop` (LRM 17.4), `$display` / `$write` and the `b`/`o`/`h` suffixed variants `$displayb` / `$displayo` / `$displayh` / `$writeb` / `$writeo` / `$writeh` (LRM 17.1 display family, basic formatting subset including `%b`/`%o`/`%d`/`%h`/`%s`/`%c`/real controls). Suffixed variants default the unformatted-integer base to binary / octal / hex respectively; explicit format controls still override. All system tasks accept null arguments (empty comma slots); `$display`/`$write` family emit one space per null for a supported format control (the control is validated first, so an unsupported one still errors), `$finish`/`$stop` discard them. LRM display controls outside this subset are intentionally unsupported for now, including `%u`, `%z`, `%t`, `%m`, strength formats, and field-width / precision modifiers.
- System functions:
  - Sign casts (LRM 5.5): `$signed`, `$unsigned`
  - Real conversions (LRM 17.7.1 / §3.5.3): `$rtoi`, `$itor`, `$realtobits`, `$bitstoreal`
  - Math (LRM 17.11): `$clog2` plus 21 real-math functions (`$ln`/`$log10`/`$exp`/`$sqrt`/`$pow`/`$floor`/`$ceil`, the trig and hyperbolic family, `$atan2`/`$hypot`)
  - Display-base casts (vcal-specific): `$bin`, `$oct`, `$dec`, `$hex` — see [non-standard.md](non-standard.md)

## Active scope

Planned but not yet implemented:

- **Multi-line edit.** The REPL accepts only single-line input today; the right TUI affordance for multi-line editing is still being explored.

## Known issues

- Declarations and assignments deep-clone the entire variable map for atomic updates, including unrelated arrays. A local release-build probe of 200 scalar assignments took about 0.003 s without an array versus 0.31 s with `reg [255:0] bulk [0:65535]` (including process startup and declarations). Consider staging only affected bindings / elements while preserving statement atomicity and original-session LHS index resolution.

- Malformed real literals like `1._0` or `9.` surface as `invalid decimal digits: 1.0` after the underscore-strip / digit-strip step, because the lexer's `real_after_dot` lookahead requires `.` followed by a digit and otherwise falls through to the integer path. The diagnostic is correct in spirit (the literal is not a valid real) but the message is misleading. A future pass should recognize "digit-run + `.`" as a real-literal commitment and emit a real-specific error.

- LRM-reserved keywords are usable as ordinary identifiers. `reg if`, `reg while`, `reg module`, `reg always`, `reg case`, `reg endmodule`, `reg begin`, `reg input`, `reg output`, `reg wire` (and many more from LRM 3.6.4) all succeed silently; `if = 5; if` then evaluates. Only the words the parser actively consumes — `reg`, `signed`, `integer`, `real` — are reserved. Harmless today because there's no control flow, but a snippet pasted from a module will bind names the user did not intend. Fix: lift the LRM 3.6.4 reserved-word list into the lexer and reject any of them in identifier position.

- The stage-prefix invariant is enforced by convention, not structure. `evaluate_expr` (`src/eval.rs:811`, `:813`) and `evaluate_assignment_rhs` (`src/eval.rs:848`, `:859`) prefix the annotate and validate steps with `Semantic error:` but return the evaluate step's `Err` verbatim, so any error raised there reaches the REPL with no prefix. Candidate sites inside that phase are the replication-count checks (`src/eval.rs:3326`, `:3330`, `:3337`), the real-as-integer leaf rejection (`src/eval.rs:1650`), and `require_vector` (`src/lib.rs:109`, `:111`). Latent rather than observable: no input found so far reaches them, because the validator rejects the same cases first. Contrast the *deliberately* unprefixed helpers, which document that callers add the prefix — `evaluate_constant_expr` (`:874`), `evaluate_subexpr_as_integer` (`:887`), `replication_count_value` (`:3984`). Fix: replace the internal `Result<_, String>` with a typed `StageError` (`Syntax` / `Semantic`, later `Runtime`) and render the prefix exactly once at the sole convergence point (`src/lib.rs:374`, `:384`). This makes classification a compile-time obligation on whoever raises the error — the person who still knows which stage they are in — instead of a guess made at the boundary, and it retires the "caller must prefix" helpers as a concept. Cost: ~111 signatures across `eval.rs` (51), `parser.rs` (23), `lib.rs` (17), `lexer.rs` (11), `system_call.rs` (8), `value.rs` (1), plus deleting the 15 hand-written prefix sites. Must be migrated wholesale — a partial migration keeps two conventions alive with `From`-style conversions at the seams, which is worse than either endpoint. The public contract is unchanged either way (`EvalError.message` stays a `String`). **Deferred until a genuine second error category lands** (file I/O, `$fopen`, anything whose success is only known at execution time): until then the enum defends a property with zero observed violations. A lighter alternative — defaulting any unprefixed message to `Semantic error:` at `src/lib.rs:384` behind a `debug_assert!` — was considered and rejected: it either guesses wrong once a runtime category exists, or it substitutes runtime self-audit for what the tests should guarantee.
