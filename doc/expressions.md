# Expression evaluation model

The general rules for how vcal evaluates expressions. Per-operator details live in [operators.md](operators.md); variable / select details live in [variables.md](variables.md); intentional divergences from the LRM are flagged in [non-standard.md](non-standard.md).

## Core evaluation and literal extension

- vcal uses propagated-context unification per LRM §5.5.2, so context-determined expressions push width and signedness down to leaf primaries before evaluation.
- Relational results remain self-determined 1-bit unsigned values, with x/z propagation handled after operand unification.
- Unsized-literal extension is implemented with an `unsized_literal: bool` flag on `IntegerValue`, set only by the unsized parser branches (`parse_unsized_decimal` and the `width_hint.is_none()` arms of `parse_based_decimal` / `parse_based_radix`) and cleared on computed values and `resized_to_context` output.
- `resized_to_context` splits into two paths:
  - unsized + wider context -> `extend_unsized_to`, which fills from the literal's own signedness unless the MSB is x/z
  - sized or equal-width -> the normal §5.5.4 path
- This matches iverilog in cases such as `'bx | 64'b0` -> `64'bx...x` and `'shFFFFFFFF | 64'b0` -> `64'hFFFF_FFFF_FFFF_FFFF`, while sized operands still follow propagated-context extension, e.g. `32'sbx | 34'b0` -> `34'b00xx...x`.

## Operator precedence and associativity

- All operators associate left-to-right except the conditional operator, which associates right-to-left.
- `**` is still left-associative, so `3 ** 3 ** 3 = (3 ** 3) ** 3 = 19683`. This differs from Python, where `3 ** 3 ** 3 = 7625597484987`.
- Expression evaluation short-circuits where applicable.

## Width rules

- Mainly derived from LRM §5.4.
- There are two kinds of expression bit-length rules:
  - self-determined expressions: width is determined only by the expression itself
  - context-determined expressions: width is determined by both the expression and the context it appears in
- vcal models this with two related widths for context-determined expressions:
  - natural width: the width inferred bottom-up from the expression's own operands, before any outer context is applied
  - effective evaluation width: the width actually used when evaluating the expression after propagated outer context is applied
- If an expression is context-determined, its effective evaluation width is `max(L(expr), L(context))`, where `L(expr)` is the expression's natural width. If there is no outer propagated context, the effective width is just the natural width.
- Common natural-width rules in vcal:
  - binary arithmetic and binary bitwise: `L(expr) = max(L(lhs), L(rhs))`
  - unary `+`, unary `-`, and unary `~`: `L(expr) = L(operand)`
  - shifts: `L(expr) = L(lhs)`; the RHS stays self-determined and does not contribute to result width
  - conditional `?:`: `L(expr) = max(L(then), L(else))`
- Example: `<` always returns a 1-bit unsigned result, so `a < b` is self-determined. By contrast, the RHS of an assignment is context-determined by both itself and the LHS width.
- vcal evaluates expressions in two passes:
  - first pass: bottom-up inference of self-determined width and signedness for each AST node
  - second pass: top-down context-determined evaluation so parent expressions can widen child arithmetic before truncation
- The second pass is required for cases like `(a + b) + 0`, where the outer expression widens the inner arithmetic before overflow is applied.

## Leaf-extension rules

- Leaf-extension splits by whether the leaf is a sized literal or an unsized literal.
- Sized leaf: extension follows §5.5.4 together with the propagated-type rule from §5.5.2.
  - signed propagated context -> sign-extend, propagating x/z if the MSB is x/z
  - unsigned propagated context -> zero-extend regardless of the operand's own signedness or MSB
- Unsized leaf: extension follows LRM Table 5-22 footnote a, independent of propagated signedness.
  - if the literal MSB is x or z, fill with that MSB
  - otherwise, fill from the literal's own declared signedness: sign-extend if signed, zero-extend if unsigned
  - the literal also keeps its self-determined >=32-bit width if the context is narrower
- Footnote a diverges from §5.5.4 in two iverilog-confirmed cases, and vcal follows iverilog / footnote a for unsized leaves:
  - `'bx | 64'b0` -> `64'bxxxx...x` because footnote a x-extends the MSB; §5.5.4 would zero-fill the upper 32 bits
  - `'shFFFFFFFF | 64'b0` -> `64'hFFFF_FFFF_FFFF_FFFF` because footnote a sign-extends per the literal's own `'sh`; §5.5.4 would zero-extend under unsigned propagated context
- Sized operands still follow propagated-context rules, so `32'sbx | 34'b0` becomes `34'b00xx...x`.
- Inside context-determined sub-expressions, outer width still propagates all the way to leaf literals, so `('bx | 4'b0) | 64'b0` also becomes `64'bxxxx...x`.
- If literal digits occupy fewer bits than the literal width, or fewer than 32 bits for an unsized literal, the value is automatically left-padded.
  - ordinary unsigned digits pad with `0`
  - `x` digits pad with `x`
  - `z` / `?` digits pad with `z`
- This digit-padding rule is not sign extension.
- An unsized constant remains unsized after parsing; its default >=32-bit form is only an intermediate. When it becomes an operand of an expression wider than 32 bits, leaf extension still follows footnote a rather than §5.5.4. Sized literals continue to follow §5.5.4.

## Signedness rules

- Mainly derived from LRM §5.5.
- Unlike width, signedness depends only on the operands.
- Simple decimal numbers are signed.
- Some operators are self-determined in signedness.
- Example: `<` always yields an unsigned result, and also always yields 1 bit.

## Base rules

The integer implementation holds at least four fields for the features specified in LRM:

- Width
- Signed
- Bits (value)

And one additional field for proper display in console:

- Base

The base of an arithmetic result is inferred from its operands so the output keeps the form the user typed when possible. The LRM does not specify this — it is a vcal display convention.

- A literal carries the base it was declared with. Unsized decimal literals (e.g. `42`) are decimal.
- A unary operator (`+`, `-`) preserves the operand's base. So `-4'b1` is `4'b1111`.
- A binary operator (`+`, `-`, `*`, `/`, `%`, `**`) takes the **leftmost** operand's base. So `4'b0111 + 4'b1001` is `4'b0000`, `8'h0a + 8'b1` is `8'h0b`, and `8'b00001010 + 8'h05` is `8'b00001111`.
  - The leftmost-wins rule mirrors the left-to-right evaluation order of the supported operators. There is no automatic base "promotion" between bases.
- Operators with non-obvious base rules (concatenation = leftmost leaf; conditional = then-branch; shift = LHS) are noted in [operators.md](operators.md).
