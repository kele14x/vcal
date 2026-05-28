# Scope

This is the **mutable working state**: what works today, what's still planned, and known issues. Agents update this file first when scope changes or a task completes — collapse completed features to one-line summaries; git history is the granular record.

For the long-term LRM-coverage target, see [lrm-coverage.md](lrm-coverage.md).

## What works

- REPL shell with `rustyline` history
- Integer and real literals, all LRM forms (LRM 3.5.2)
- All operators between integers (see [operators.md](operators.md))
- Real arithmetic and mixed integer/real promotion (LRM 5.1.5 / 5.1.7 / Tables 5-2, 5-3)
- Two-pass context (width, signedness) propagation; leftmost-base propagation
- `reg` / `integer` / `real` declarations and blocking assignment with the full LRM A.8.5 `variable_lvalue` — bare name, bit/part/indexed-part selects, and arbitrarily nested concatenations on the LHS (see [variables.md](variables.md))
- 1-D unpacked arrays on `reg` / `integer` / `real` (LRM 4.9 / A.2.2.1)
- Static-semantic validation as a top-level pre-pass over every expression entry — errors prefixed `Syntax error:` (lex/parse) or `Semantic error:` (validator)
- System tasks: `$finish`, `$stop` (LRM 17.4)
- System functions:
  - Sign casts (LRM 5.5): `$signed`, `$unsigned`
  - Real conversions (LRM 17.7.1 / §3.5.3): `$rtoi`, `$itor`, `$realtobits`, `$bitstoreal`
  - Math (LRM 17.11): `$clog2` plus 21 real-math functions (`$ln`/`$log10`/`$exp`/`$sqrt`/`$pow`/`$floor`/`$ceil`, the trig and hyperbolic family, `$atan2`/`$hypot`)
  - Display-base casts (vcal-specific): `$bin`, `$oct`, `$dec`, `$hex` — see [non-standard.md](non-standard.md)

## Active scope

Planned but not yet implemented:

- **Multi-line edit.** The REPL accepts only single-line input today; the right TUI affordance for multi-line editing is still being explored.
- **String literals (LRM 3.6 / A.8.8).** No `"..."` token in the lexer and no `String` AST variant. The use case is tied to the display task family (`$display` etc., LRM 17.1) — both will likely land together.

## Known issues

- Malformed real literals like `1._0` or `9.` surface as `invalid decimal digits: 1.0` after the underscore-strip / digit-strip step, because the lexer's `real_after_dot` lookahead requires `.` followed by a digit and otherwise falls through to the integer path. The diagnostic is correct in spirit (the literal is not a valid real) but the message is misleading. A future pass should recognize "digit-run + `.`" as a real-literal commitment and emit a real-specific error.
