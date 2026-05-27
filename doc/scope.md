# Scope

This is the **mutable working state**: what works today, what's actively in scope, and what's on the backlog. Agents update this file first when scope changes or a task completes — collapse completed features to one-line summaries; git history is the granular record.

For the long-term LRM-coverage target, see [lrm-coverage.md](lrm-coverage.md).

## What works

- REPL shell
- Integer literals (all LRM forms)
- Real literals and real arithmetic (LRM 3.5.2 / 5.1.5 / Tables 5-2, 5-3); mixed-type promotion (LRM 5.1.7)
- `$finish` / `$stop`
- `$signed()` / `$unsigned()` sign-cast system functions (LRM 5.5)
- Real-conversion system functions: `$rtoi`, `$itor`, `$realtobits`, `$bitstoreal` (LRM 17.7.1 / §3.5.3)
- Math system functions: `$clog2` plus 21 real-math functions (`$ln`/`$log10`/`$exp`/`$sqrt`/`$floor`/`$ceil`, the trig/hyperbolic family, `$pow`/`$atan2`/`$hypot`) per LRM 17.11; real-math wraps libm via Rust's `f64::*` to match the C standard library
- All operators between integers (see [operators.md](operators.md))
- Two-pass context (width, signedness) propagation
- Leftmost-base propagation
- `reg` / `integer` / `real` declarations + blocking assignment with full LRM A.8.5 `variable_lvalue`: bare name, bit-select, part-select, indexed part-selects, and arbitrarily nested concatenations of any of those on the LHS (see [variables.md](variables.md)). `integer` is fixed signed 32-bit decimal; `real` is IEEE 754 binary64 with 0.0 default and no width / sign / base; both reject `signed`, packed `[range]`, and unpacked `[unpacked_range]` at parse time. `time` is out of scope.
- RHS bit-select and part-select on declared *vector* regs (see [variables.md](variables.md))
- 1-D unpacked arrays (LRM 4.9 / A.2.2.1) on `reg`, `integer`, and `real`: `reg [3:0] a [0:15]` declares 16 vector elements; `integer a [0:3]` declares 4 signed-32-bit integer elements; `real r [0:3]` declares 4 f64 elements. `a[i]` selects an element (read or write); `a[i][m:l]` and the bit/indexed forms select within the chosen element for `reg`/`integer` arrays (real-array elements have no bits to slice, so only `r[i]` is legal). Vector-array-element leaves are valid inside an LHS concat; real-array elements are not. Multi-dim arrays, packed-array-of-array forms, and array initializers are out of scope (see [variables.md](variables.md) → "Unpacked arrays").
- `rustyline` history

## Active scope

- Single-line REPL input only
- Integer and real literals, parentheses
- Identifiers (simple_identifier per LRM 3.7.1) as primaries, with the four RHS select forms on declared *vector* regs only. Per LRM 5.2.1 a scalar reg (declared with no range) rejects all four forms — `reg [0:0] a` is the 1-bit-vector escape hatch. vcal evaluates select operands at runtime against the current session rather than at elaboration. The same four select forms plus nested concatenations are accepted on the LHS of a blocking assignment per LRM A.8.5 `variable_lvalue`. `reg`, `integer`, and `real` are the declared variable types (no `time`); a single unpacked dimension is accepted on all three (`reg [3:0] a [0:15]`, `integer a [0:3]`, `real r [0:3]`), but multi-dim arrays, packed-array-of-array forms, and array initializers remain out of scope. Real-array elements are f64-typed, so only `r[i]` is legal (no inner select, no part-select, not legal in concat lvalues); reads/writes flow through the real pipeline and OOB / x-z indexes drop the write and read as `0.0`. Per-name init via `name = constant_expression` is supported for vector / integer / real decls; array decls cannot carry an init expression.
- Top-level `Stmt` layer above `Expr`: declaration, blocking assignment, expression, and the hoisted `$finish` / `$stop` task forms; a `Session` owns the variable map (`RegValue`, not just bare `IntegerValue`) and is threaded through every evaluator entry.
- All operators between integers; arithmetic / relational / equality / logical / `?:` between reals (Table 5-2):
  - Arithmetic ops (`+ - * / % **`, unary `+`, unary `-`)
  - Relational ops (`<`, `>`, `<=`, `>=`)
  - Equality ops (`==`, `!=`, `===`, `!==`)
  - Logical ops (`!`, `&&`, `||`)
  - Bitwise ops (`~`, `&`, `|`, `^`, `~^` / `^~`)
  - Reduction unaries (`& ~& | ~| ^ ~^/^~`)
  - Shift operators `<< >> <<< >>>`
  - Conditional operator `?:` (the only ternary)
  - Concatenation `{a, b, ...}` and replication `{N{...}}`
- System functions:
  - Sign casts (LRM 5.5): `$signed`, `$unsigned`
  - Display-base casts (vcal-specific): `$bin`, `$oct`, `$dec`, `$hex`
  - Real conversions (LRM 17.7.1): `$rtoi`, `$itor`, `$realtobits`, `$bitstoreal`
  - Math (LRM 17.11): `$clog2`; `$ln`, `$log10`, `$exp`, `$sqrt`, `$pow`, `$floor`, `$ceil`, `$sin`, `$cos`, `$tan`, `$asin`, `$acos`, `$atan`, `$atan2`, `$hypot`, `$sinh`, `$cosh`, `$tanh`, `$asinh`, `$acosh`, `$atanh`

## Backlog

See [lrm-coverage.md](lrm-coverage.md) for the final target. Phase scoping beyond real numbers (variables, multi-line input, …) is TBD — confirm with the user before starting work outside the active scope.

Specific forward-looking items lifted from the original requirements / gap notes:

- TUI / multi-line editor — the way of multi-line edit is not clear yet.
- `time` variable declarations; multi-dim packed forms and the multi-dim unpacked `{ dimension }` form (the single unpacked dimension is supported).
- Comments (LRM 3.3), attributes (LRM 3.8), escaped identifiers (LRM 3.7.1).
- Display tasks: `$display` / `$displayb` / `$displayo` / `$displayh` (LRM 17.1).
- Probabilistic distribution functions (`$random`, `$dist_*`) per LRM 17.9.

## Known issues

- Malformed real literals like `1._0` or `9.` surface as `invalid decimal digits: 1.0` after the underscore-strip / digit-strip step, because the lexer's `real_after_dot` lookahead requires `.` followed by a digit and otherwise falls through to the integer path. The diagnostic is correct in spirit (the literal is not a valid real) but the message is misleading. A future pass should recognize "digit-run + `.`" as a real-literal commitment and emit a real-specific error.

## Recent updates

- `integer` and `real` now accept a single unpacked dimension (LRM A.2.2.1 `variable_type` / `real_type`): `integer a [0:3]` is a 4-element array of signed-32-bit integers, `real r [0:3]` is a 4-element array of f64 slots. Vector-integer arrays reuse the existing `reg`-array storage with the fixed integer element template; real arrays use a new `RegStorage::RealArray` and a separate write path (`apply_real_array_element_assign`) since concats / inner selects don't apply to f64 elements. Multi-dim arrays and array-with-init are still rejected.

- `integer` and `real` keywords (LRM 4.8) are accepted alongside `reg`. `integer` is a fixed signed 32-bit decimal vector (`32'sdx` default), so bit/part-selects against it work the same as on a hand-rolled `reg signed [31:0] i`. `real` is an IEEE 754 binary64 slot with a 0.0 default and no width / sign / base — assignments dispatch through a real-LHS pipeline that promotes integer RHS via §5.1.7 / §3.5.3. Both kinds reject `signed` and packed `[range]` at parse time; the parser also rejects each keyword as a variable name. `time` remains out of scope.

- Static-semantic validation is now a top-level pre-pass: every expression evaluation entry point (`evaluate_expr`, `evaluate_assignment_rhs`, `evaluate_constant_expr`, the LHS / RHS of `evaluate_lvalue_assignment`, and `evaluate_reg_range`) walks the whole tree before any evaluation runs. Errors are surfaced eagerly but stop at the first failure (no multi-error collection). This closes the previously-hidden cases where structural errors sat inside zero-count replications (`{1'b1, {0{r[1.0]}}}`), untaken `?:` branches, or other positions the evaluator would otherwise skip. Errors now carry distinguishing prefixes — `"Syntax error: "` for parser/lexer rejections and `"Semantic error: "` for validator rejections — so the rejection stage is visible from the message alone; genuine runtime conditions (e.g. unknown-bit part-select endpoints) remain unprefixed.
