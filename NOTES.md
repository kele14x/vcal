# Implementation Notes

## General evaluation model

### Core evaluation and literal extension

- vcal uses propagated-context unification per LRM §5.5.2, so context-determined expressions push width and signedness down to leaf primaries before evaluation.
- Relational results remain self-determined 1-bit unsigned values, with x/z propagation handled after operand unification.
- Unsized-literal extension is implemented with an `unsized_literal: bool` flag on `IntegerValue`, set only by the unsized parser branches (`parse_unsized_decimal` and the `width_hint.is_none()` arms of `parse_based_decimal` / `parse_based_radix`) and cleared on computed values and `resized_to_context` output.
- `resized_to_context` splits into two paths:
  - unsized + wider context -> `extend_unsized_to`, which fills from the literal's own signedness unless the MSB is x/z
  - sized or equal-width -> the normal §5.5.4 path
- This matches iverilog in cases such as `'bx | 64'b0` -> `64'bx...x` and `'shFFFFFFFF | 64'b0` -> `64'hFFFF_FFFF_FFFF_FFFF`, while sized operands still follow propagated-context extension, e.g. `32'sbx | 34'b0` -> `34'b00xx...x`.

### Operator precedence and associativity

- All operators associate left-to-right except the conditional operator, which associates right-to-left.
- `**` is still left-associative, so `3 ** 3 ** 3 = (3 ** 3) ** 3 = 19683`. This differs from Python, where `3 ** 3 ** 3 = 7625597484987`.
- Expression evaluation short-circuits where applicable.

### Width rules

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

### Leaf-extension rules

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

### Signedness rules

- Mainly derived from LRM §5.5.
- Unlike width, signedness depends only on the operands.
- Simple decimal numbers are signed.
- Some operators are self-determined in signedness.
- Example: `<` always yields an unsigned result, and also always yields 1 bit.

## Operator-specific notes

### Arithmetic operators

- There are six binary arithmetic operators: `+`, `-`, `*`, `/`, `%`, and `**`; and two unary arithmetic operators: unary `+` and unary `-`.
- Arithmetic operators are context-determined in width, including the `unary +` and `unary -` operators.
- Natural width:
  - binary arithmetic = `max(L(lhs), L(rhs))`
  - unary `+` / unary `-` = `L(operand)`
- Effective evaluation width under propagated outer context:
  - binary arithmetic = `max(L(lhs), L(rhs), L(context))`
  - unary `+` / unary `-` = `max(L(operand), L(context))`
- Width/context resize happens before unary `-` is evaluated, so `-4'sb0001` is not always interchangeable with the already-resized bit pattern `4'sb1111`.
- Resizing follows the propagated context signedness, so even a signed operand may be zero-extended in an unsigned context.
  - Example: `4'sb1000 + 8'b0` -> `8'b00001000`, because the propagated context is unsigned and `4'sb1000` is extended with zeros before evaluation.

### Relational operators

- Relational operators share their operand-unification path with equality operators.
- The relational expression itself has a self-determined 1-bit result width, but before comparison it first decides a single propagated operand context for both operands:
  - operand-unification width = `max(L(lhs), L(rhs))`
  - signedness = signed iff both operands are signed; otherwise unsigned
- That propagated context is pushed down through context-determined sub-expressions such as unary `-` and binary `+` / `*` until it reaches a leaf primary.
- Extension at the leaf is decided by the propagated signedness, not the leaf's own signedness:
  - propagated unsigned -> zero-extend the narrower side
  - propagated signed -> sign-extend the narrower side
- Once both operands are unified, comparison is performed as integers in that unified type.
- The result is always 1-bit and independent of the surrounding expression, per LRM §5.5.2.
- Examples:
  - `4'sb1111 < 8'd255` -> `1'b1`. Under unsigned propagated context, `4'sb1111` zero-extends to `8'b00001111` (= 15), not `8'b11111111`.
  - `-4'sb1000 < 8'd9` -> `1'b0`. Propagation reaches the leaf `4'sb1000`, which zero-extends to `0000_1000` (= 8); unary negation at 8-bit unsigned wraps to `1111_1000` (= 248), so `248 < 9` is false.

### Equality operators

- Equality expressions also have a self-determined 1-bit result width.
- Operand unification is otherwise identical to the relational operators:
  - operand-unification width = `max(L(lhs), L(rhs))`
  - signedness = signed iff both operands are signed; otherwise unsigned
- After unification:
  - `==` / `!=` follow LRM §5.1.8. The result is `1'bx` only when the relation is ambiguous.
  - any definite `0`/`1` mismatch makes the operands definitely unequal regardless of x/z elsewhere, so `==` returns `0` and `!=` returns `1`
  - otherwise, if any bit position contains x or z, the result is `1'bx`
  - otherwise, all bits match and the operands are equal
  - `===` / `!==` compare bit-for-bit, with x matching only x and z matching only z, so the result is always definite `0` or `1`
- Width extension for `===` / `!==` follows §5.5.4. The special x/z fill rule applies only when the propagated context is signed, meaning both operands are signed. Under unsigned propagated context, the narrower side is always zero-extended even if its MSB is x or z.
- Examples:
  - `4'sbx000 === 8'sbxxxxx000` -> `1'b1` because both operands are signed, so x-fill applies
  - `4'sbx000 === 8'b0000x000` -> `1'b1` because mixed signedness makes the propagated context unsigned, so zero-fill applies
  - `4'sbx000 === 8'bxxxxx000` -> `1'b0` because the RHS upper `xxxx` does not match the zero-filled upper bits of the LHS

### Logical operators

- Logical operands are reduced to a 1-bit logical value before applying the operator.
- Reduction rule:
  - any `1` -> `1`
  - all `0` -> `0`
  - otherwise, if there is x/z and no definite `1`, the result is `x`
- `!`, `&&`, and `||` follow the LRM §5.1.9 truth tables.
- Binary logical operators return a self-determined 1-bit unsigned result, which may then widen in an outer arithmetic context just like relational and equality operators.

### Bitwise operators

- Binary bitwise operators have:
  - natural width = `max(L(lhs), L(rhs))`
  - effective evaluation width under propagated outer context = `max(L(lhs), L(rhs), L(context))`
- Unary `~` has:
  - natural width = `L(operand)`
  - effective evaluation width under propagated outer context = `max(L(operand), L(context))`
- If there is no propagated outer context, effective width equals natural width.
- Like arithmetic, a wider parent context widens the per-bit operation before truncation.
- Signedness = signed iff both operands are signed for binary forms; unary `~` preserves operand signedness.
- Base inheritance follows existing rules:
  - leftmost-wins for binary bitwise operators
  - operand-preserving for unary `~`
- `~^` and `^~` always denote the same operator.
- There is no all-x short-circuit; per-bit truth tables are applied position by position.
- Implementation notes:
  - The parser precedence band is `&` > `^/~^/^~` > `|`, between `==` and `&&`.
  - Bare `&` and `|` now lex as bitwise operators instead of being rejected.
  - `~^` and `^~` collapse to one token.

### Reduction operators

- `&`, `|`, `^`, `~^`, and `^~` are also binary bitwise operators, so they are disambiguated by parse position: unary when no left-hand operand exists, binary otherwise.
- `~&` and `~|` exist only as unary reduction operators; using them in a binary position is a syntax error.
- Reduction folds use the same 4-state truth tables as the corresponding binary bitwise operators.
- Identity elements:
  - `1` for AND-fold
  - `0` for OR-fold and XOR-fold
- Negated reductions invert the folded result.
- All reduction operators produce a self-determined 1-bit unsigned result.
- Implementation notes:
  - `~&` and `~|` were added as dedicated tokens.
  - Binary `&` / `|` / `^` / `~^` / `^~` tokens are reused at unary position via parse-position disambiguation, without token rewriting.
  - A single `reduce_bits` helper performs the fold; negated reductions invert the folded result.

### Shift operators

- There are four shift operators: `<<`, `<<<`, `>>`, and `>>>`.
- Per LRM §5.1.12, the LHS is context-determined like arithmetic, while the RHS is self-determined and always treated as an unsigned number.
- Concretely:
  - natural width = `L(lhs)`
  - effective evaluation width = `max(L(lhs), L(context))`
  - the LHS widens to that result width before shifting, so an outer arithmetic context can preserve bits that a self-determined shift would have dropped
  - signedness comes from the LHS together with the propagated outer context per §5.5.1; the RHS never contributes to signedness
  - the RHS is interpreted as an unsigned bit pattern regardless of its declared signedness
  - a nominally negative shift count like `-4'sd1` therefore becomes a very large unsigned value
  - shift count is clamped to the result width, so any count >= width yields the saturated all-fill result
  - any x/z bit in the RHS makes the entire result all-x
- Vacated bits are filled as follows:
  - `<<`, `<<<`, and `>>` always zero-fill
  - `>>>` fills with the post-extension LHS MSB only when the propagated result type is signed; otherwise it zero-fills
- Example: `4'sb1000 >>> 1` is `4'sb1100` self-determined, but `(4'sb1000 >>> 1) + 8'd0` becomes `8'b00000100` because the unsigned `8'd0` flips the propagated context to unsigned.
- Result base inherits from the LHS.
- Non-shifted LHS bits preserve their original values, including x/z, so `4'b01x0 << 1` is `4'b1x00`.
- Implementation notes:
  - `parse_shift` sits between additive and relational, matching LRM Table 5-4.
  - Greedy lexing of `<<<` / `<<` and `>>>` / `>>` preserves `<=` / `>=` / `<` / `>`.
  - Shift count is clamped to the result width so `BigUint`-sized counts saturate safely to the all-fill result.

### Conditional operator

- Verilog's only ternary operator is `expression1 ? expression2 : expression3`.
- It is right-associative, so `a ? b : c ? d : e` parses as `a ? b : (c ? d : e)`.
- `expression1` is self-determined and reduced to a 1-bit logical using the same rule as `&&`, `||`, and `!`.
- `expression2` and `expression3` are context-determined and both take the result width and signedness.
- Natural width = `max(L(expression2), L(expression3))`.
- Effective evaluation width = `max(L(expression2), L(expression3), L(context))`.
- Signedness combines per §5.5.1, but the propagated outer signedness still overrides leaf extension. In practice, a mixed unsigned outer context zero-fills both branches at their leaves rather than sign-filling them.
- If the condition is definite `0` or `1`, only the selected branch contributes bits.
- If the condition is x/z, both branches are evaluated and merged per bit:
  - agreeing bits stay as-is, including x/x and z/z
  - disagreeing bits become x
- Result base inherits from the then branch, i.e. `expression2`, the leftmost data-carrying branch.
- Width extension for the branches exposes the same §5.1.13 vs §5.5.2 inconsistency seen in the bitwise rules. §5.1.13 says the shorter operand should be lengthened with zero-fill, while §5.5.2 says sign-extend whenever the propagated type is signed. For `1 ? 4'shF : 8'sh0` the rules disagree. vcal follows §5.5.2 and matches iverilog:
  - `1 ? 4'shF : 8'sh0` -> `8'shff`
  - `1 ? 4'shF : 8'h0` -> `8'h0f`

### Concatenation and replication operators

- The brace forms from LRM §5.1.14 are primaries, so they sit alongside literals and grouped expressions rather than participating in infix precedence.
- Syntax:
  - concatenation: `{ expr {, expr} }`
  - replication: `{ count_expr { expr {, expr} } }`
- Inner operands are self-determined, with no outer context propagated into them, and their bits are joined MSB-first.
- The result is always unsigned, base inheritance is leftmost-wins, and any outer context only zero-extends the already joined result.
- Indefinite-width rejection uses `is_indefinite_width`, following LRM §5.1.14. Any expression whose self-determined width depends on an unsized literal is rejected inside concatenation. That indefinite-width flag propagates through context-determined operators (arithmetic, bitwise, power, shift LHS, conditional branches, unary `+` / `-` / `~`) and stops at definite 1-bit-result operators (relational, equality, logical, reduction) and at concatenation and replication themselves.
- Examples:
  - `{1, 4'd2}` -> error
  - `{4'd1 + 1, 4'd2}` -> error
  - `{1 << 1, 4'd2}` -> error
  - `{1'b1 ? 1 : 4'd2, 4'd2}` -> error
  - `{4'd1 + 4'd1, 4'd2}` -> OK
  - `{1 == 2, 4'd2}` -> OK
- Replication count rules:
  - must be a constant expression, which is always true in vcal today
  - must be non-negative and contain no x/z
  - is interpreted as a mathematical integer using its declared signedness, so `-1` is rejected as negative even if its bit pattern could be read as a large unsigned value
  - zero is allowed only when the replication appears inside a concatenation that has at least one other positive-size operand
- Examples of the zero-replication rule:
  - `{{0{1'b1}}, 1'b1}` -> `1'b1`
  - top-level `{0{1'b1}}` -> error
  - `{{0{1'b1}}}` -> error
  - `{{0{1'b1}}, {0{1'b1}}}` -> error
  - `{N{{0{1'b1}}}}` -> error if every replicated operand stays zero-sized
- The zero-permission also applies through `Grouped`, so `{({0{1'b1}}), 1'b1}` is treated the same as the unwrapped form.
