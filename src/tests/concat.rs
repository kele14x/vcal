use crate::lexer::{Token, tokenize};
use crate::{Session, evaluate_input};

// -------- Concatenation / replication (LRM 5.1.14) --------

#[test]
fn concatenation_joins_operands_msb_first() {
    // Leftmost operand occupies the high bits (LRM §5.1.14).
    let result = evaluate_input("{2'b10, 2'b01}").expect("eval");
    assert_eq!(result.output, "4'b1001");
}

#[test]
fn concatenation_supports_more_than_two_operands() {
    let result = evaluate_input("{1'b1, 2'b00, 1'b1}").expect("eval");
    assert_eq!(result.output, "4'b1001");
}

#[test]
fn concatenation_inherits_leftmost_operand_base() {
    // Same leftmost-wins rule as arithmetic/bitwise/shift (vcal display
    // convention; LRM doesn't prescribe one). The bit pattern (`8'h12`) is
    // per LRM §5.1.14; the base choice is ours.
    let hex_first = evaluate_input("{4'h1, 4'b10}").expect("eval");
    let bin_first = evaluate_input("{4'b10, 4'h1}").expect("eval");
    assert_eq!(hex_first.output, "8'h12");
    assert_eq!(bin_first.output, "8'b00100001");
}

#[test]
fn concatenation_preserves_x_and_z_bits() {
    // Concatenation never reduces unknown bits — each position is copied
    // through.
    let result = evaluate_input("{2'bxz, 2'b01}").expect("eval");
    assert_eq!(result.output, "4'bxz01");
}

#[test]
fn concatenation_result_is_unsigned_even_when_operands_signed() {
    // LRM 5.5.1 last paragraph + 5.1.14: result is unsigned regardless
    // of operand signedness.
    let result = evaluate_input("{4'sb1000, 4'sb0001}").expect("eval");
    assert_eq!(result.output, "8'b10000001");
}

#[test]
fn single_element_concatenation_is_identity_on_bits() {
    // `{x}` is legal LRM syntax (a single-operand concatenation) and
    // produces the operand's bit pattern re-flagged as unsigned.
    let result = evaluate_input("{4'b1010}").expect("eval");
    assert_eq!(result.output, "4'b1010");
}

#[test]
fn concatenation_widens_through_outer_arithmetic_context() {
    // The joined value (4 bits) zero-extends to the outer context width
    // (8 bits) before the addition runs — concatenation is unsigned, so
    // §5.5.4 zero-fills regardless of operand signedness.
    let result = evaluate_input("{4'b1010} + 8'd0").expect("eval");
    assert_eq!(result.output, "8'b00001010");
}

#[test]
fn replication_repeats_inner_concatenation() {
    // {N{...}} — N copies of the inner concatenation joined back to back.
    let single = evaluate_input("{4{1'b1}}").expect("eval");
    let multi = evaluate_input("{2{2'b01, 2'b10}}").expect("eval");
    assert_eq!(single.output, "4'b1111");
    assert_eq!(multi.output, "8'b01100110");
}

#[test]
fn replication_count_can_be_a_constant_expression() {
    // The count is any constant expression (LRM 5.1.14). vcal has no
    // variables, so any well-formed expression qualifies.
    let result = evaluate_input("{(1+3){1'b1}}").expect("eval");
    assert_eq!(result.output, "4'b1111");
}

#[test]
fn replication_can_nest_when_inner_is_braced() {
    // `{2{ {2{1'b1}} }}` — outer rep of an inner replication. Note the
    // inner `{2{1'b1}}` must itself be a brace primary; `{2{2{1'b1}}}`
    // is a syntax error since `2{1'b1}` is not a standalone primary
    // (LRM §5.1.14 grammar).
    let result = evaluate_input("{2{ {2{1'b1}} }}").expect("eval");
    assert_eq!(result.output, "4'b1111");
}

#[test]
fn concatenation_rejects_bare_unsized_literal_operand() {
    // LRM 5.1.14: "Unsized constant numbers shall not be allowed in
    // concatenations."
    let err = evaluate_input("{1, 4'd2}").expect_err("indefinite");
    assert_eq!(
        err,
        "Semantic error: concatenation operand has indefinite width"
    );
}

#[test]
fn concatenation_rejects_arithmetic_with_unsized_operand() {
    // The indefinite-width flag propagates through context-determined
    // arithmetic: `4'd1 + 1` is indefinite because the `1` is unsized.
    let err = evaluate_input("{4'd1 + 1, 4'd2}").expect_err("indefinite");
    assert_eq!(
        err,
        "Semantic error: concatenation operand has indefinite width"
    );
}

#[test]
fn concatenation_accepts_arithmetic_when_all_operands_sized() {
    // `4'd1 + 4'd1` is sized (both operands sized → result is 4-bit), so
    // the operand has a definite width and concatenation succeeds.
    // Expect `00100010` = 8'd34.
    let result = evaluate_input("{4'd1 + 4'd1, 4'd2}").expect("eval");
    assert_eq!(result.output, "8'd34");
}

#[test]
fn concatenation_accepts_one_bit_results_with_unsized_subexpressions() {
    // Relational/equality/logical/reduction always produce 1-bit results
    // — they have a definite width even when their operands are unsized.
    // Expect `{1==2, 4'd2}` → `00010` = 5 bits.
    let result = evaluate_input("{1==2, 4'd2}").expect("eval");
    assert_eq!(result.output, "5'b00010");
}

#[test]
fn concatenation_rejects_shift_with_unsized_lhs() {
    // Shifts take their result width from the LHS only (LRM 5.1.12), so
    // an unsized LHS makes the whole expression indefinite.
    let err = evaluate_input("{1 << 1, 4'd2}").expect_err("indefinite");
    assert_eq!(
        err,
        "Semantic error: concatenation operand has indefinite width"
    );
}

#[test]
fn concatenation_rejects_conditional_with_unsized_branch() {
    // Conditional width is max(then, else) (LRM 5.1.13), so an unsized
    // branch makes the whole conditional indefinite.
    let err = evaluate_input("{1'b1 ? 1 : 4'd2, 4'd2}").expect_err("indefinite");
    assert_eq!(
        err,
        "Semantic error: concatenation operand has indefinite width"
    );
}

#[test]
fn concatenation_rejects_power_with_unsized_lhs() {
    // `**` takes its result width from the LHS only (LRM 5.1.5, same
    // shape as shifts), so an unsized LHS makes the whole expression
    // indefinite even when the RHS is sized.
    let err = evaluate_input("{2 ** 4'd3, 4'd2}").expect_err("indefinite");
    assert_eq!(
        err,
        "Semantic error: concatenation operand has indefinite width"
    );
}

#[test]
fn concatenation_accepts_power_with_sized_lhs() {
    // A sized LHS pins the operand's width even when the RHS is
    // unsized — `**` is LHS-determined, so `4'd2 ** 3` is 4 bits wide.
    // 2 ** 3 = 8 → 4'd8 = 0b1000; concatenated with 4'd2 = 0b0010 gives
    // 8'b10000010 = 8'd130.
    let result = evaluate_input("{4'd2 ** 3, 4'd2}").expect("eval");
    assert_eq!(result.output, "8'd130");
}

#[test]
fn top_level_replication_rejects_zero_count() {
    // LRM 5.1.14 only permits zero replication when it sits inside a
    // concatenation with at least one positive-size operand; a top-level
    // `{0{...}}` (no enclosing concat) is rejected.
    let err = evaluate_input("{0{1'b1}}").expect_err("zero count");
    assert_eq!(
        err,
        "Semantic error: replication count must be positive in this context"
    );
}

#[test]
fn zero_replication_inside_concatenation_contributes_no_bits() {
    // LRM 5.1.14: a replication may have a zero count when it is one of
    // the operands of a concatenation whose other operands sum to a
    // positive width. The zero-rep simply contributes nothing — e.g.
    // `{{0{1'b1}}, 1'b1}` → `1`.
    let prefix = evaluate_input("{ {0{1'b1}}, 1'b1 }").expect("zero rep prefix");
    let suffix = evaluate_input("{ 4'b1010, {0{1'b1}} }").expect("zero rep suffix");
    let middle = evaluate_input("{ 1'b1, {0{1'b1}}, 1'b0 }").expect("zero rep middle");
    let multiple = evaluate_input("{ {0{1'b1}}, {0{1'b1}}, 1'b1 }").expect("zero rep many");

    assert_eq!(prefix.output, "1'b1");
    assert_eq!(suffix.output, "4'b1010");
    assert_eq!(middle.output, "2'b10");
    assert_eq!(multiple.output, "1'b1");
}

#[test]
fn zero_replication_through_grouped_is_treated_the_same() {
    // `({0{1'b1}})` is `Grouped(Replication{0, ...})`. vcal looks
    // through `Grouped` to find the underlying Replication node when
    // applying the zero-permission rule from LRM §5.1.14.
    let result = evaluate_input("{ ({0{1'b1}}), 1'b1 }").expect("grouped zero rep");
    assert_eq!(result.output, "1'b1");
}

#[test]
fn zero_replication_inside_nested_replication_inner_list() {
    // The zero-permission also applies to a replication's *inner*
    // concatenation list, since that list is itself a concatenation.
    // Expect `{2{ {0{1'b1}}, 1'b1 }}` → `11`.
    let result = evaluate_input("{2{ {0{1'b1}}, 1'b1 }}").expect("nested zero rep");
    assert_eq!(result.output, "2'b11");
}

#[test]
fn concatenation_of_only_zero_replication_is_rejected() {
    // `{ {0{1'b1}} }` — one operand, and it has zero size. No
    // positive-size sibling, so the surrounding concatenation has no
    // positive-size operand, which LRM 5.1.14 forbids.
    let solo = evaluate_input("{ {0{1'b1}} }").expect_err("solo zero rep in concat");
    let pair =
        evaluate_input("{ {0{1'b1}}, {0{1'b1}} }").expect_err("two zero reps no positive sibling");
    let nested = evaluate_input("{2{ {0{1'b1}} }}").expect_err("outer rep over zero-only inner");
    assert_eq!(
        solo,
        "Semantic error: concatenation must have at least one operand with positive size"
    );
    assert_eq!(
        pair,
        "Semantic error: concatenation must have at least one operand with positive size"
    );
    assert_eq!(
        nested,
        "Semantic error: concatenation must have at least one operand with positive size"
    );
}

#[test]
fn replication_rejects_negative_count() {
    // `-1` is signed-negative — read as a math integer, sign() = Minus.
    let err = evaluate_input("{-1{1'b1}}").expect_err("negative count");
    assert_eq!(
        err,
        "Semantic error: replication count must be non-negative"
    );
}

// The validator runs as a pre-pass on the whole expression tree, so a
// structural error inside a zero-count replication is caught even though
// the runtime would short-circuit before walking the inner items. These
// three positions used to surface differently:
//   leftmost  — caught by the leftmost-base inference path
//   rightmost — NOT caught (the bug fixed by the pre-pass)
//   middle    — NOT caught (same bug)
// All three now produce the same `Semantic error:` diagnostic.
#[test]
fn zero_replication_inner_items_validated_in_every_position() {
    let mut session = Session::new();
    session.eval("reg [3:0] r").expect("decl");

    let leftmost = session
        .eval("{ {0{r[1.0]}}, 1'b1 }")
        .expect_err("leftmost zero-rep with real index");
    let rightmost = session
        .eval("{ 1'b1, {0{r[1.0]}} }")
        .expect_err("rightmost zero-rep with real index");
    let middle = session
        .eval("{ 1'b1, {0{r[1.0]}}, 1'b0 }")
        .expect_err("middle zero-rep with real index");

    assert_eq!(leftmost, "Semantic error: bit-select index cannot be real");
    assert_eq!(rightmost, "Semantic error: bit-select index cannot be real");
    assert_eq!(middle, "Semantic error: bit-select index cannot be real");
}

#[test]
fn replication_rejects_unknown_count() {
    // A count with any x or z bit is rejected — per LRM 5.1.14 the count
    // must be "a constant expression that is non-negative, non-x, non-z".
    let err = evaluate_input("{1'bx{1'b1}}").expect_err("unknown count");
    assert_eq!(
        err,
        "Semantic error: replication count contains unknown bits"
    );
}

// Regression (P1): annotate() constant-evaluates a replication's count to
// derive the width meta. That evaluation must not run ahead of validation —
// an invalid count (real operand of an integer-only op, or a concat of real
// items) used to reach the evaluator's validated-input `unreachable!` and
// panic instead of surfacing a clean diagnostic. The count subtree is now
// validated before it is evaluated, in both the select-subexpression path
// and the top-level path.
#[test]
fn replication_count_with_real_binary_operand_is_a_clean_error() {
    // Top-level: the count `1.0 % 2` applies `%` to a real operand.
    let err = evaluate_input("{(1.0 % 2){1}}").expect_err("real % in count");
    assert_eq!(
        err,
        "Semantic error: operator % not allowed on real operand"
    );
}

#[test]
fn replication_count_concat_of_real_is_a_clean_error() {
    // Top-level: the count `({1.0})` is a concatenation whose operand is
    // real — concatenation requires definite bit widths.
    let err = evaluate_input("{({1.0}){1}}").expect_err("real concat in count");
    assert_eq!(err, "Semantic error: concatenation operand cannot be real");
}

#[test]
fn invalid_replication_count_in_select_index_is_a_clean_error() {
    // The select-subexpression position routes through the same eager
    // count evaluation via validate_subexpr_structure; it must produce the
    // same diagnostics, not panic.
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("decl");

    let real_op = session
        .eval("a[{(1.0 % 2){1}}]")
        .expect_err("real % in select-index count");
    assert_eq!(
        real_op,
        "Semantic error: operator % not allowed on real operand"
    );

    let real_concat = session
        .eval("a[{({1.0}){1}}]")
        .expect_err("real concat in select-index count");
    assert_eq!(
        real_concat,
        "Semantic error: concatenation operand cannot be real"
    );
}

#[test]
fn real_typed_count_with_structural_error_keeps_specific_diagnostic() {
    // A real-typed count skips the eager constant-evaluation, but its
    // subtree is still validated — the specific structural diagnostic
    // must surface ahead of the generic "replication count cannot be
    // real". Covers the four count shapes that are real-typed *because*
    // of the error they contain.
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("decl");

    let task = session.eval("a[{($finish){1}}]").expect_err("task count");
    assert_eq!(
        task,
        "Semantic error: $finish() is a system task, it cannot be called as a function."
    );

    let op_on_real = session.eval("a[{(~1.0){1}}]").expect_err("~ on real count");
    assert_eq!(
        op_on_real,
        "Semantic error: operator ~ not allowed on real operand"
    );

    let signed_real = session
        .eval("a[{($signed(1.0)){1}}]")
        .expect_err("$signed(real) count");
    assert_eq!(
        signed_real,
        "Semantic error: $signed argument cannot be real"
    );

    let bitstoreal = session
        .eval("a[{($bitstoreal(1)){1}}]")
        .expect_err("$bitstoreal width count");
    assert_eq!(
        bitstoreal,
        "Semantic error: $bitstoreal argument must be 64 bits wide, got 32"
    );

    // A plain real count (no structural error) still gets the generic
    // diagnostic.
    let plain_real = session.eval("a[{(1.0){1}}]").expect_err("plain real count");
    assert_eq!(
        plain_real,
        "Semantic error: replication count cannot be real"
    );
}

#[test]
fn empty_braces_is_a_parse_error() {
    // `{}` — no expressions inside; LRM grammar requires at least one.
    let err = evaluate_input("{}").expect_err("empty");
    assert_eq!(err, "Syntax error: expected expression operand");
}

#[test]
fn unclosed_concatenation_is_a_parse_error() {
    let err = evaluate_input("{4'd1, 4'd2").expect_err("unclosed");
    assert_eq!(err, "Syntax error: missing closing brace in concatenation");
}

#[test]
fn tokenizes_braces_and_comma_as_separate_tokens() {
    // Braces and comma must split adjacent literals — `1,2'b10` should
    // tokenize as `1`, `,`, `2'b10`, not be swallowed into a single
    // integer literal.
    let tokens = tokenize("{1'd1,2'b10}").expect("tokens");
    assert_eq!(
        tokens,
        vec![
            Token::LBrace,
            Token::IntegerLiteral("1'd1".to_string()),
            Token::Comma,
            Token::IntegerLiteral("2'b10".to_string()),
            Token::RBrace,
        ]
    );
}

#[test]
fn replication_widens_through_outer_arithmetic_context() {
    // Same self-determined-then-extend shape as plain concatenation: the
    // 4-bit replication result zero-extends to the 8-bit outer context.
    let result = evaluate_input("{4{1'b1}} + 8'd0").expect("eval");
    assert_eq!(result.output, "8'b00001111");
}
