use crate::evaluate_input;
use crate::lexer::{Token, tokenize};
use crate::parser::{BinaryOp, Expr, parse_expression};

// ---------- Shift operators (<< >> <<< >>>) ----------
//
// LRM 1364-2005 §5.1.12: the LHS is context-determined; the RHS is
// self-determined and "always treated as an unsigned number ... has no
// effect on the signedness of the result". `<<` and `<<<` zero-fill
// vacated positions; `>>` always zero-fills; `>>>` fills with the LHS
// sign bit when the result type is signed and zero-fills otherwise. If
// the RHS contains x or z, the entire result is unknown.
//
// The LRM single-bit truth tables only cover 1-bit operands (where
// multi-bit shift dynamics collapse), so the multi-bit cases here exercise
// the LRM §5.1.12 rules directly.

#[test]
fn tokenizes_shift_operators_as_single_tokens() {
    // Greedy lex: `<<<`/`>>>` win over `<<`/`>>`, which win over the
    // single-character `<`/`>` (and over `<=`/`>=` which still need the
    // `=`-specific path). A regression where `<<<` collapsed to two
    // tokens would silently become `<<` followed by `<`.
    let shl = tokenize("4'd1 << 1").expect("<< should tokenize");
    let shr = tokenize("4'd1 >> 1").expect(">> should tokenize");
    let ashl = tokenize("4'd1 <<< 1").expect("<<< should tokenize");
    let ashr = tokenize("4'd1 >>> 1").expect(">>> should tokenize");

    assert_eq!(shl[1], Token::LogicalShiftLeft);
    assert_eq!(shr[1], Token::LogicalShiftRight);
    assert_eq!(ashl[1], Token::ArithmeticShiftLeft);
    assert_eq!(ashr[1], Token::ArithmeticShiftRight);
}

#[test]
fn shift_lex_does_not_swallow_relational_or_le_ge() {
    // Adding `<<`/`<<<` paths must not regress `<=`, `>=`, or bare `<`/`>`.
    let le = tokenize("4'd1 <= 4'd2").expect("<=");
    let ge = tokenize("4'd1 >= 4'd2").expect(">=");
    let lt = tokenize("4'd1 < 4'd2").expect("<");
    let gt = tokenize("4'd1 > 4'd2").expect(">");

    assert_eq!(le[1], Token::LessEqual);
    assert_eq!(ge[1], Token::GreaterEqual);
    assert_eq!(lt[1], Token::Less);
    assert_eq!(gt[1], Token::Greater);
}

#[test]
fn evaluates_basic_logical_shift_left() {
    let shifted = evaluate_input("4'b0001 << 1").expect("<< 1");
    let by_two = evaluate_input("4'b0001 << 4'd2").expect("<< 2");
    // Top bit shifts out at the 4-bit self-determined width.
    let overflow = evaluate_input("4'b1000 << 1").expect("<< 1 overflow");
    let by_zero = evaluate_input("4'b0101 << 0").expect("<< 0 noop");

    assert_eq!(shifted.output, "4'b0010");
    assert_eq!(by_two.output, "4'b0100");
    assert_eq!(overflow.output, "4'b0000");
    assert_eq!(by_zero.output, "4'b0101");
}

#[test]
fn evaluates_basic_logical_shift_right() {
    let shifted = evaluate_input("4'b1000 >> 1").expect(">> 1");
    let by_two = evaluate_input("4'b1100 >> 4'd2").expect(">> 2");
    let by_zero = evaluate_input("4'b0101 >> 0").expect(">> 0 noop");
    // Logical right shift always zero-fills, even when the LHS is signed.
    let signed_zero_fill = evaluate_input("4'sb1000 >> 1").expect("signed >> 1");

    assert_eq!(shifted.output, "4'b0100");
    assert_eq!(by_two.output, "4'b0011");
    assert_eq!(by_zero.output, "4'b0101");
    assert_eq!(signed_zero_fill.output, "4'sb0100");
}

#[test]
fn arithmetic_left_shift_matches_logical_left_shift() {
    // LRM 5.1.12: `<<<` is exactly `<<` — both zero-fill the LSBs.
    let logical = evaluate_input("4'b0011 << 1").expect("<<");
    let arithmetic = evaluate_input("4'b0011 <<< 1").expect("<<<");
    let signed_logical = evaluate_input("4'sb1010 << 1").expect("signed <<");
    let signed_arith = evaluate_input("4'sb1010 <<< 1").expect("signed <<<");

    assert_eq!(logical.output, arithmetic.output);
    assert_eq!(signed_logical.output, signed_arith.output);
}

#[test]
fn arithmetic_right_shift_sign_fills_when_signed() {
    // Signed self-determined: vacated MSB takes the LHS sign bit.
    //   4'sb1000 = -8;  >>> 1 → 4'sb1100 = -4
    //   4'sb1110 = -2;  >>> 1 → 4'sb1111 = -1
    //   4'sb1000 >>> 4'd3 → all four MSBs vacated, all filled with 1
    let neg_eight = evaluate_input("4'sb1000 >>> 1").expect("signed >>> 1");
    let neg_two = evaluate_input("4'sb1110 >>> 1").expect("signed >>> 1");
    let saturated = evaluate_input("4'sb1000 >>> 4'd3").expect("signed >>> 3");

    assert_eq!(neg_eight.output, "4'sb1100");
    assert_eq!(neg_two.output, "4'sb1111");
    assert_eq!(saturated.output, "4'sb1111");
}

#[test]
fn arithmetic_right_shift_zero_fills_when_unsigned() {
    // Unsigned LHS (self-determined unsigned context) → `>>>` is just `>>`.
    let unsigned = evaluate_input("4'b1000 >>> 1").expect("unsigned >>> 1");
    let unsigned_full = evaluate_input("4'b1111 >>> 4'd3").expect("unsigned >>> 3");

    assert_eq!(unsigned.output, "4'b0100");
    assert_eq!(unsigned_full.output, "4'b0001");
}

#[test]
fn arithmetic_right_shift_propagates_x_or_z_when_msb_is_unknown() {
    // The fill bit IS the LHS MSB (LRM 5.1.12). When the MSB is x, the
    // vacated positions become x; when it is z, the same z value is used.
    let x_fill = evaluate_input("4'sbx000 >>> 1").expect("x msb");
    let z_fill = evaluate_input("4'sbz000 >>> 1").expect("z msb");

    assert_eq!(x_fill.output, "4'sbxx00");
    assert_eq!(z_fill.output, "4'sbzz00");
}

#[test]
fn shift_with_unknown_rhs_returns_all_x() {
    // LRM 5.1.12: "If the right operand has an x or z value, then the
    // result shall be unknown." This dominates the LHS bit pattern — even
    // a fully-known LHS yields all-x.
    let x_rhs_left = evaluate_input("4'd5 << 4'bx").expect("x rhs <<");
    let z_rhs_right = evaluate_input("4'd5 >> 4'bz").expect("z rhs >>");
    let x_rhs_arith = evaluate_input("4'sb1000 >>> 4'bx").expect("x rhs >>>");
    // Even one x bit in the RHS poisons the entire result.
    let partial_x = evaluate_input("4'd5 << 4'b00x0").expect("partial x rhs");
    // Result inherits LHS base for rendering (decimal here → 4'dx).
    assert_eq!(x_rhs_left.output, "4'dx");
    assert_eq!(z_rhs_right.output, "4'dx");
    assert_eq!(x_rhs_arith.output, "4'sbxxxx");
    assert_eq!(partial_x.output, "4'dx");
}

#[test]
fn shift_preserves_lhs_bit_values_including_x_and_z() {
    // The shift moves bits into new positions without altering them; only
    // the vacated edge takes the fill value. So an x/z in the middle of
    // the LHS just slides one position over.
    let x_in_middle = evaluate_input("4'b01x0 << 1").expect("x in middle <<");
    let z_in_middle = evaluate_input("4'b1z00 >> 1").expect("z in middle >>");
    // Left shift by 1 of 4'sb1xx0: the MSB 1 shifts out (lost); xx slides
    // left to bits 3,2; the original LSB 0 slides to bit 1; bit 0 is the
    // zero-filled vacated LSB. → 4'sbxx00.
    let signed_xx = evaluate_input("4'sb1xx0 << 1").expect("signed x's");

    assert_eq!(x_in_middle.output, "4'b1x00");
    assert_eq!(z_in_middle.output, "4'b01z0");
    assert_eq!(signed_xx.output, "4'sbxx00");
}

#[test]
fn shift_clamps_oversized_count_to_lhs_width() {
    // LRM 5.1.12 doesn't bound the RHS, so we treat any count >= width
    // as an all-fill case. Useful both for huge constants and (next test)
    // for negative RHS values that bit-encode as huge unsigned numbers.
    let exactly_width = evaluate_input("4'd5 << 4'd4").expect("exactly width");
    let beyond_width = evaluate_input("4'b0101 << 4'd5").expect("beyond width");
    let beyond_width_right = evaluate_input("4'b1111 >> 4'd9").expect(">> beyond");
    let signed_beyond = evaluate_input("4'sb1000 >>> 4'd9").expect(">>> beyond");

    assert_eq!(exactly_width.output, "4'd0");
    assert_eq!(beyond_width.output, "4'b0000");
    assert_eq!(beyond_width_right.output, "4'b0000");
    // signed >>> with beyond-width count saturates to the sign bit.
    assert_eq!(signed_beyond.output, "4'sb1111");
}

#[test]
fn shift_treats_negative_rhs_as_large_unsigned() {
    // LRM 5.1.12: the RHS is "always treated as an unsigned number".
    // -1 has bits 1...1, which read unsigned is 2^N-1 — well past any
    // reasonable LHS width — so the shift saturates to all-fill.
    let neg_one_left = evaluate_input("4'd5 << -4'sd1").expect("<< -1");
    let neg_one_signed_arith = evaluate_input("4'sb1000 >>> -4'sd1").expect(">>> -1");

    assert_eq!(neg_one_left.output, "4'd0");
    assert_eq!(neg_one_signed_arith.output, "4'sb1111");
}

#[test]
fn shift_widens_lhs_through_outer_arithmetic_context() {
    // Self-determined the high bit of `4'd8 << 4'd1` shifts out → 4'd0.
    // Inside a 32-bit context the LHS first widens to 32 bits before the
    // shift, so the bit survives and the answer is 16, not 0. Same shape
    // as the existing `applies_width_rules_to_multiplicative_expressions`
    // test for arithmetic.
    let truncated = evaluate_input("4'd8 << 4'd1").expect("truncated");
    let widened = evaluate_input("(4'd8 << 4'd1) + 0").expect("widened");

    assert_eq!(truncated.output, "4'd0");
    assert_eq!(widened.output, "32'd16");
}

#[test]
fn arithmetic_right_shift_fill_follows_propagated_signedness() {
    // Same shift, three different propagated contexts. `>>>` flips
    // between sign-fill and zero-fill based on whether the result type
    // ends up signed (LRM §5.1.12).
    //
    //   Self-determined signed: signed → sign-fill → -4 → 4'sb1100.
    //   Mixed unsigned context (8'd0): unsigned → zero-fill, but the
    //     LHS first zero-extends to 8 bits = 8 → 8 >>> 1 = 4 → 8'b0...100.
    //   All-signed context (signed `0`): signed → sign-extend LHS to 32
    //     bits = -8 → -8 >>> 1 = -4 → 32'sb1...1100.
    let self_determined = evaluate_input("4'sb1000 >>> 1").expect("self");
    let mixed_unsigned = evaluate_input("(4'sb1000 >>> 1) + 8'd0").expect("mixed");
    let all_signed = evaluate_input("(4'sb1000 >>> 1) + 0").expect("all signed");

    assert_eq!(self_determined.output, "4'sb1100");
    // Result base inherits from the leftmost operand (Binary), and the
    // shared 8-bit unsigned context makes the outer result unsigned.
    assert_eq!(mixed_unsigned.output, "8'b00000100");
    assert_eq!(all_signed.output, "32'sb11111111111111111111111111111100");
}

#[test]
fn shift_inherits_leftmost_base_like_arithmetic() {
    // Result base is the LHS base, mirroring the existing leftmost-wins
    // rule for arithmetic and bitwise binaries.
    let hex = evaluate_input("8'h0a << 4'd1").expect("hex base");
    let binary = evaluate_input("8'b00001010 << 4'd1").expect("binary base");
    let decimal = evaluate_input("8'd10 << 4'd1").expect("decimal base");

    assert_eq!(hex.output, "8'h14");
    assert_eq!(binary.output, "8'b00010100");
    assert_eq!(decimal.output, "8'd20");
}

#[test]
fn shift_rhs_is_self_determined_and_does_not_widen_lhs() {
    // RHS at LRM Table 5-22 is self-determined, so a wide RHS must NOT
    // pull the LHS up to its width. Without the self-determined rule the
    // 4-bit `4'd8` would widen to 32 bits and `<< 1` would yield 16
    // instead of the truncated 4'd0.
    let wide_rhs = evaluate_input("4'd8 << 32'd1").expect("wide rhs");
    assert_eq!(wide_rhs.output, "4'd0");
}

#[test]
fn shift_rhs_signedness_does_not_flip_result_signedness() {
    // LRM 5.1.12: the RHS "has no effect on the signedness of the
    // result". A signed RHS therefore keeps the LHS-driven signedness.
    let signed_lhs_signed_rhs = evaluate_input("4'sd2 << 4'sd1").expect("ss");
    let unsigned_lhs_signed_rhs = evaluate_input("4'd2 << 4'sd1").expect("us");

    assert_eq!(signed_lhs_signed_rhs.output, "4'sd4");
    assert_eq!(unsigned_lhs_signed_rhs.output, "4'd4");
}

#[test]
fn shift_precedence_below_additive_above_relational() {
    // LRM Table 5-4: `+`/`-` > `<<`/`>>` > `<`/`>`.
    //
    //   `4'd1 + 4'd2 << 4'd1` parses as `(4'd1 + 4'd2) << 4'd1`
    //                              = 3 << 1 = 6.
    //   `4'd2 << 4'd1 < 4'd5` parses as `(4'd2 << 4'd1) < 4'd5`
    //                              = 4 < 5 = 1.
    let add_then_shift_expr = parse_expression("4'd1 + 4'd2 << 4'd1").expect("parse");
    match &add_then_shift_expr {
        Expr::Binary {
            op: BinaryOp::LogicalShiftLeft,
            lhs,
            ..
        } => assert!(matches!(
            lhs.as_ref(),
            Expr::Binary {
                op: BinaryOp::Add,
                ..
            }
        )),
        other => panic!("expected top-level <<, got {other:?}"),
    }
    let add_then_shift = evaluate_input("4'd1 + 4'd2 << 4'd1").expect("eval");
    assert_eq!(add_then_shift.output, "4'd6");

    let shift_then_relational_expr = parse_expression("4'd2 << 4'd1 < 4'd5").expect("parse");
    match &shift_then_relational_expr {
        Expr::Binary {
            op: BinaryOp::LessThan,
            lhs,
            ..
        } => assert!(matches!(
            lhs.as_ref(),
            Expr::Binary {
                op: BinaryOp::LogicalShiftLeft,
                ..
            }
        )),
        other => panic!("expected top-level <, got {other:?}"),
    }
    let shift_then_relational = evaluate_input("4'd2 << 4'd1 < 4'd5").expect("eval");
    assert_eq!(shift_then_relational.output, "1'b1");
}

#[test]
fn shift_is_left_associative() {
    // `a << b << c` parses as `(a << b) << c`. Same shape check used for
    // the other binary levels.
    let expr = parse_expression("4'd1 << 4'd1 << 4'd1").expect("parse");
    match &expr {
        Expr::Binary {
            op: BinaryOp::LogicalShiftLeft,
            lhs,
            ..
        } => assert!(matches!(
            lhs.as_ref(),
            Expr::Binary {
                op: BinaryOp::LogicalShiftLeft,
                ..
            }
        )),
        other => panic!("expected top-level <<, got {other:?}"),
    }
    // (1 << 1) << 1 = 2 << 1 = 4 at 4-bit width.
    let result = evaluate_input("4'd1 << 4'd1 << 4'd1").expect("eval");
    assert_eq!(result.output, "4'd4");
}

#[test]
fn shift_at_primary_position_is_rejected() {
    // No shift operator is unary, so a leading shift token has no
    // operand to the left. parse_primary's catchall must turn this into
    // the standard "expected expression operand" error rather than
    // silently consuming the operator as something else.
    let lead_shl = evaluate_input("<< 4'd1").expect_err("leading <<");
    let lead_shr = evaluate_input(">> 4'd1").expect_err("leading >>");

    assert_eq!(lead_shl, "Syntax error: expected expression operand");
    assert_eq!(lead_shr, "Syntax error: expected expression operand");
}
