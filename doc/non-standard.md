# Non-standard behavior

vcal-specific divergences from IEEE 1364-2005. Full operator rules live in [operators.md](operators.md); variable rules in [variables.md](variables.md).

## Top-level input

The LRM defines `statement ::= blocking_assignment ; | system_task_enable` inside a module, with declarations at the module-item level (Annex A.2). vcal has no module wrapper and no separate elaboration stage, so the REPL accepts a flat top-level stream `{ statement | system_task_enable | declaration | expression }`. `declaration` (`reg`, `integer`, `real`) is hoisted from module-item level so users can introduce a variable without a module. `expression` lets a bare expression like `a + 1` evaluate and display its value — the calculator-mode behavior, which has no LRM counterpart.

## Trailing semicolons

The Verilog LRM requires a trailing semicolon for each statement. This is annoying for a calculator app. vcal accepts an optional trailing semicolon. Users may use a trailing semicolon to explicitly end the input phase and force the app to evaluate the input (works together with multi-line edit).

## Integer constants

Unsized number (simple decimal number or a number without size) shall be at least 32 bits. vcal uses a number of bits longer than 32 if the value needs more bits instead of strictly truncating to 32 bits as the LRM permits.

## Arithmetic operators

The LRM specifies any unknown bits will cause the arithmetic operator to return all `x`. However in almost all implementations (`iverilog`, etc.), the `unary +` returns the bits the same, including `x` and `z`. For other arithmetic operators, if any operand has any `x` or `z` bit, then the entire result value shall be all `x`. vcal follows the implementation convention rather than the strict LRM rule.

## Bitwise operators

LRM 1364-2005 has an internal inconsistency about operand extension: §5.1.10 says "the shorter operand is zero-filled in the most significant bit positions", but §5.5.2 says a narrower operand is sign-extended whenever the propagated type is signed (which, by §5.5.1, happens when *all* operands are signed). For `4'shF | 8'sh0` the two rules disagree — §5.1.10 would give `8'sh0F`, §5.5.2 gives `8'shFF`. vcal follows §5.5.2 (sign-extend when both signed, zero-extend otherwise), matching iverilog, VCS, Xcelium, and the IEEE 1800 (SystemVerilog) clarification that drops the §5.1.10 sentence entirely. This is the same extension rule already used by relational/equality/arithmetic in vcal, so all operators stay consistent.

## Bit-select and part-select operands

The Verilog LRM requires constant-expression operands for `r[m:l]` and for the `width` half of `r[b +: w]` / `r[b -: w]`, because simulators and synthesizers resolve those shapes during elaboration. vcal has no separate elaboration stage: the REPL evaluates each input directly against the current `Session`. So vcal deliberately relaxes those forms to ordinary integer expressions evaluated at runtime. The resulting values still must be usable as a select shape: part-select endpoints and indexed widths must resolve to definite integers, and indexed widths must be positive.

## Real numbers

vcal stores real values as Rust `f64`, which is IEEE 754 binary64 — the same format LRM §3.5.2 references. A few corners the LRM leaves to the implementation are pinned down here:

- §5.1.5 says `0.0 ** ≤0` and `negative ** non-integral` are *unspecified* for real `**`. vcal returns whatever Rust's `f64::powf` produces:
  - `0.0 ** 0.0` → `1.0`
  - `0.0 ** -1.0` → `inf`
  - `(-2.0) ** 0.5` → `NaN`
  These come from IEEE 754 directly. iverilog and VCS may differ on the exact value, so don't rely on a specific corner result.
- `1'bx ? real_a : real_b` cannot reproduce the integer per-bit-merge rule (real has no per-bit identity). vcal returns the common branch value when both branches agree bit-for-bit on `f64::to_bits`, and `NaN` otherwise.
- Real values render in fixed-point for magnitudes in `[1e-4, 1e10)` and scientific notation outside that window — purely a display choice, not specified by the LRM.
- §17.8 doesn't address NaN / ±∞ in `$rtoi`. vcal returns 32 bits of `x` to surface "no defined integer image" rather than silently mapping to zero. Out-of-range finite values wrap mod 2³² (the same overflow rule the rest of the integer pipeline uses).
- §17.8 doesn't address NaN / ±∞ in `$itor` either. `$itor` on a real argument goes through implicit real→integer→real; the implicit real→int step has no integer image for NaN/±∞, so it yields `x` (matching the `$rtoi` rule above), and §3.5.3's int→real then maps every `x` bit to `0`. So `$itor(0.0/0.0)` and `$itor(±1.0/0.0)` all collapse to `0.0`, keeping `$itor` self-consistent with `$rtoi`.
- §17.8 doesn't carve out an x/z rule for `$bitstoreal`. vcal applies §3.5.3's "x/z → 0" rule to its 64-bit operand for consistency with the sibling integer-to-real conversions, so `$bitstoreal(64'bx)` decodes as `+0.0`.
- §17.11 doesn't address x/z bits in `$clog2`. vcal returns 32 bits of `x` whenever the operand contains any x or z bit, mirroring the `$rtoi` NaN/±∞ rule (surface "no defined image" rather than silently mapping to zero). Real arguments take the §3.5.3 round-half-away-from-zero path, so NaN/±∞ collapse to `32'sdx` the same way they do under `$rtoi`. Finite reals wrap mod 2³² before the unsigned interpretation, matching `$rtoi`'s 32-bit signed result domain. Per LRM the operand is "treated as an unsigned value" of its natural width, so `$clog2(64'hFFFF_FFFF_FFFF_FFFF)` is `32'sd64` and `$clog2(-1)` (32-bit signed) is `32'sd32`.

## Conditional operator

vcal deliberately diverges from LRM Table 5-21 on the ambiguous-cond merge. The strict table reduces *every* combination other than `(0,0)` and `(1,1)` to `x` — including `(x,x)` and `(z,z)`. iverilog (and most other simulators) instead use the value-preserving rule above, on the principle that if both branches put the same `x` (or `z`) at the same position regardless of cond, the result is necessarily that bit and reducing it to `x` would discard information. So `1'bx ? 4'b01xz : 4'b01xz` is `4'b01xz` here (and in iverilog), not the `4'b01xx` the LRM table prescribes. vcal follows iverilog as the practical reference.

## Display-base cast functions

vcal adds four non-standard system functions — `$bin`, `$oct`, `$dec`, `$hex` — that change only the display base of an integer expression. The argument is evaluated as a self-determined expression; the result has the same width, signedness, and bits, with `Base` overridden to the cast's target. Outer-context width still flows back through the cast per §5.5.2 (same shape as `$signed` / `$unsigned`). Real arguments are rejected — reals have no display base.

These exist so users do not need tricks like `1'b0 + 1` to render `1` in binary; `$bin(1)` does the job directly.

Because the argument is evaluated self-determined, the cast acts as a context barrier — outer-context width does *not* flow into the argument. So `$hex(4'hf + 4'hf) + 8'h0` is `8'h0e` (the inner `+` overflows at 4 bits, then extends), while the un-cast `(4'hf + 4'hf) + 8'h0` is `8'h1e` (the outer 8-bit context widens the inner `+` before computing). This matches the §5.5 self-determined-argument rule already used by `$signed` / `$unsigned` and is not specific to the display-base casts.
