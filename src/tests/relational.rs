use crate::evaluate_input;
use crate::lexer::{Token, tokenize};
use crate::parser::{BinaryOp, Expr, parse_expression};

#[test]
fn tokenizes_le_and_ge_as_single_tokens() {
    let le = tokenize("4 <= 5").expect("<= should tokenize");
    let ge = tokenize("4 >= 5").expect(">= should tokenize");
    let lt = tokenize("4 < 5").expect("< should tokenize");
    let gt = tokenize("4 > 5").expect("> should tokenize");

    assert_eq!(le.len(), 3);
    assert_eq!(le[1], Token::LessEqual);
    assert_eq!(ge.len(), 3);
    assert_eq!(ge[1], Token::GreaterEqual);
    assert_eq!(lt[1], Token::Less);
    assert_eq!(gt[1], Token::Greater);
}

#[test]
fn relational_binds_looser_than_additive() {
    // 1 + 2 < 4 parses as (1 + 2) < 4 → 3 < 4 → true
    let expr = parse_expression("1 + 2 < 4").expect("parse");
    match &expr {
        Expr::Binary {
            op: BinaryOp::LessThan,
            lhs,
            ..
        } => assert!(matches!(
            lhs.as_ref(),
            Expr::Binary {
                op: BinaryOp::Add,
                ..
            }
        )),
        other => panic!("expected top-level <, got {other:?}"),
    }

    let result = evaluate_input("1 + 2 < 4").expect("precedence");
    assert_eq!(result.output, "1'b1");
}

#[test]
fn relational_is_left_associative() {
    // 4 < 5 < 1 parses as (4 < 5) < 1 → 1 < 1 → false
    let expr = parse_expression("4 < 5 < 1").expect("parse");
    match &expr {
        Expr::Binary {
            op: BinaryOp::LessThan,
            lhs,
            ..
        } => assert!(matches!(
            lhs.as_ref(),
            Expr::Binary {
                op: BinaryOp::LessThan,
                ..
            }
        )),
        other => panic!("expected top-level <, got {other:?}"),
    }

    let result = evaluate_input("4 < 5 < 1").expect("assoc");
    assert_eq!(result.output, "1'b0");
}

#[test]
fn evaluates_basic_unsigned_relational_operators() {
    let lt = evaluate_input("4'd3 < 4'd5").expect("lt");
    let gt = evaluate_input("4'd5 > 4'd3").expect("gt");
    let le_eq = evaluate_input("4'd3 <= 4'd3").expect("le eq");
    let ge_eq = evaluate_input("4'd3 >= 4'd3").expect("ge eq");
    let le_false = evaluate_input("4'd4 <= 4'd3").expect("le false");
    let ge_false = evaluate_input("4'd2 >= 4'd3").expect("ge false");

    assert_eq!(lt.output, "1'b1");
    assert_eq!(gt.output, "1'b1");
    assert_eq!(le_eq.output, "1'b1");
    assert_eq!(ge_eq.output, "1'b1");
    assert_eq!(le_false.output, "1'b0");
    assert_eq!(ge_false.output, "1'b0");
}

#[test]
fn signed_relational_uses_real_world_signed_comparison() {
    let three_lt_five = evaluate_input("4'sd3 < 4'sd5").expect("signed lt");
    let neg_lt = evaluate_input("-4'sd1 < 4'sd2").expect("signed neg lt");
    let neg_gt_neg = evaluate_input("-4'sd1 > -4'sd2").expect("signed neg/neg");

    assert_eq!(three_lt_five.output, "1'b1");
    assert_eq!(neg_lt.output, "1'b1");
    assert_eq!(neg_gt_neg.output, "1'b1");
}

#[test]
fn mixed_signedness_uses_unsigned_comparison() {
    // -4'sd1 has bits 1111 → reinterpreted as unsigned 15; 15 > 0
    let neg_one_gt_zero = evaluate_input("-4'sd1 > 4'd0").expect("neg vs unsigned");
    // -4'sd1 propagates 8-bit unsigned context to the inner 4'sd1, which
    // zero-extends to 0000_0001; negation at 8-bit unsigned yields 255.
    let neg_one_gt_zero_widened = evaluate_input("-4'sd1 > 8'd0").expect("widened");
    // 4'sd2 zero-extends (unsigned context) to 0000_0010 = 2; 2 > 5 false.
    let two_not_gt_five = evaluate_input("4'sd2 > 8'd5").expect("not gt");

    assert_eq!(neg_one_gt_zero.output, "1'b1");
    assert_eq!(neg_one_gt_zero_widened.output, "1'b1");
    assert_eq!(two_not_gt_five.output, "1'b0");
}

#[test]
fn mixed_signedness_zero_extends_signed_primary_per_lrm_5_5_2() {
    // LRM §5.1.7 + §5.5.2: when one operand is unsigned the propagated
    // type is unsigned, so the narrower signed primary is ZERO-extended,
    // not sign-extended-then-reinterpreted. The buggy "extend with own
    // signedness, then reinterpret as unsigned" model would flip these
    // answers.
    //
    //   4'sb1111 < 8'd255  →  zero-ext 1111 → 0000_1111 = 15;  15 < 255 → 1
    //   4'sb1000 > 8'd7    →  zero-ext 1000 → 0000_1000 = 8;    8 >  7  → 1
    //   4'sb1000 < 8'd9    →  zero-ext 1000 → 0000_1000 = 8;    8 <  9  → 1
    //   4'sb1111 < 8'd16   →  zero-ext 1111 → 0000_1111 = 15;  15 < 16  → 1
    let lt_big = evaluate_input("4'sb1111 < 8'd255").expect("lt_big");
    let gt_small = evaluate_input("4'sb1000 > 8'd7").expect("gt_small");
    let lt_small = evaluate_input("4'sb1000 < 8'd9").expect("lt_small");
    let lt_sixteen = evaluate_input("4'sb1111 < 8'd16").expect("lt_sixteen");

    assert_eq!(lt_big.output, "1'b1");
    assert_eq!(gt_small.output, "1'b1");
    assert_eq!(lt_small.output, "1'b1");
    assert_eq!(lt_sixteen.output, "1'b1");
}

#[test]
fn unary_minus_propagates_unsigned_context_through_to_primary() {
    // LRM §5.5.2: propagation passes through context-determined unary `-`
    // down to the leaf primary. For `-4'sb1000 < 8'd9` the inner 4'sb1000
    // zero-extends to 0000_1000 = 8 (unsigned context); negation at 8-bit
    // unsigned wraps to 256-8 = 248; 248 < 9 → 0.
    //
    // The "evaluate -4'sb1000 self-determined first then sign-extend"
    // model would give 8 < 9 → 1, which §5.5.2 rules out.
    let lt = evaluate_input("-4'sb1000 < 8'd9").expect("unary lt");
    let gt = evaluate_input("-4'sb1000 > 8'd9").expect("unary gt");
    let lt_close = evaluate_input("-4'sb1000 < 8'd249").expect("248 < 249");

    assert_eq!(lt.output, "1'b0");
    assert_eq!(gt.output, "1'b1");
    assert_eq!(lt_close.output, "1'b1");
}

#[test]
fn mixed_signedness_relational_neg_one_widened_per_lrm_5_5_2() {
    // Both models agree here (the buggy "sign-extend then reinterpret"
    // path happens to coincide with the LRM-correct "propagate down,
    // negate at wider width" path because 4'sd1 has MSB=0). Kept as a
    // regression guard against future propagation changes.
    let gt = evaluate_input("-4'sd1 > 8'd16").expect("> case");
    let lt = evaluate_input("-4'sd1 < 8'd16").expect("< case");

    assert_eq!(gt.output, "1'b1");
    assert_eq!(lt.output, "1'b0");
}

#[test]
fn unsigned_relational_zero_extends_smaller_operand() {
    // 4'd1 zero-extends to 8 bits = 8'd1; 16 > 1 → true
    let result = evaluate_input("8'd16 > 4'd1").expect("widen unsigned");
    assert_eq!(result.output, "1'b1");
}

#[test]
fn relational_propagates_unknown_bits_as_one_bit_x() {
    let with_x_lhs = evaluate_input("4'bx < 4'd1").expect("x lhs");
    let with_z_rhs = evaluate_input("4'd0 < 4'bz").expect("z rhs");
    let with_partial_x = evaluate_input("4'b01x0 > 4'd1").expect("partial x");

    assert_eq!(with_x_lhs.output, "1'bx");
    assert_eq!(with_z_rhs.output, "1'bx");
    assert_eq!(with_partial_x.output, "1'bx");
}

#[test]
fn relational_result_widens_to_outer_arithmetic_context() {
    // (4'd3 < 4'd5) → 1'b1; outer + widens result to 4 bits.
    // Leftmost-base wins: relational's Binary; outer + is unsigned (mixed).
    let result = evaluate_input("(4'd3 < 4'd5) + 4'd0").expect("widened");
    assert_eq!(result.output, "4'b0001");
}

#[test]
fn relational_result_renders_in_binary_regardless_of_operand_base() {
    // Both operands hex but the 1-bit relational result is binary.
    let hex_compare = evaluate_input("8'h0a < 8'h0f").expect("hex compare");
    assert_eq!(hex_compare.output, "1'b1");
}

// ---------- Equality operators (==, !=, ===, !==) ----------
//
// Expected values follow the LRM 1364-2005 §5.1.8 + §5.5.2 rules:
//   * Operand unification follows the same shared-context model as
//     relational ops (max width; signed iff both signed; extension at the
//     leaf primary uses the propagated signedness).
//   * `==`/`!=` return 1'bx only when the relation is *ambiguous* — a
//     definite bit mismatch (0 vs 1) makes operands unequal regardless
//     of x/z elsewhere.
//   * `===`/`!==` compare bit-for-bit including x and z; result is always
//     a known 0 or 1, never x.

#[test]
fn evaluates_basic_equality_operators() {
    let eq_true = evaluate_input("4'd3 == 4'd3").expect("eq true");
    let eq_false = evaluate_input("4'd3 == 4'd5").expect("eq false");
    let ne_true = evaluate_input("4'd3 != 4'd5").expect("ne true");
    let ne_false = evaluate_input("4'd3 != 4'd3").expect("ne false");
    let case_eq = evaluate_input("4'd3 === 4'd3").expect("case eq");
    let case_ne = evaluate_input("4'd3 !== 4'd5").expect("case ne");

    assert_eq!(eq_true.output, "1'b1");
    assert_eq!(eq_false.output, "1'b0");
    assert_eq!(ne_true.output, "1'b1");
    assert_eq!(ne_false.output, "1'b0");
    assert_eq!(case_eq.output, "1'b1");
    assert_eq!(case_ne.output, "1'b1");
}

#[test]
fn equality_zero_extends_signed_primary_in_mixed_context() {
    // 4'sb1111 zero-extends (unsigned context) to 0000_1111 = 15;
    // RHS 8'hFF = 255; not equal → == 0, != 1.
    let eq = evaluate_input("4'sb1111 == 8'hFF").expect("eq");
    let ne = evaluate_input("4'sb1111 != 8'hFF").expect("ne");
    // 4'sb1000 zero-extends to 0000_1000 = 8; RHS 8'hF8 = 248 → not equal
    let eq8 = evaluate_input("4'sb1000 == 8'hF8").expect("eq8");
    // Positive signed primary: 4'sb0001 zero-extends to 1; equals 8'd1.
    let eq_pos = evaluate_input("4'sb0001 == 8'd1").expect("eq_pos");

    assert_eq!(eq.output, "1'b0");
    assert_eq!(ne.output, "1'b1");
    assert_eq!(eq8.output, "1'b0");
    assert_eq!(eq_pos.output, "1'b1");
}

#[test]
fn equality_unary_minus_changes_extension_outcome_with_same_bits() {
    // Three cases that look almost identical but expose the LRM §5.5.2
    // propagation rule clearly. The bit pattern `1111` shows up in all
    // three, but the surrounding context decides whether it ends up as
    // 15 or as 255 in an 8-bit comparison.
    //
    //   -4'sh1 == 4'shF : both 4-bit signed, no extension; bits
    //                     `1111` == `1111`             → 1
    //   -4'sh1 == 8'hFF : mixed → 8-bit unsigned context. Propagation
    //                     passes through unary `-` to the primary
    //                     `4'sh1` = `0001`; zero-extends to
    //                     `0000_0001` = 1; negate at 8-bit unsigned
    //                     → `1111_1111` = 255. 255 == 255 → 1
    //   4'shF  == 8'hFF : same mixed context, but no unary `-`, so
    //                     the primary `4'shF` = `1111` zero-extends
    //                     directly to `0000_1111` = 15. 15 ≠ 255 → 0
    let neg_same_width = evaluate_input("-4'sh1 == 4'shF").expect("neg same width");
    let neg_widened = evaluate_input("-4'sh1 == 8'hFF").expect("neg widened");
    let no_neg_widened = evaluate_input("4'shF == 8'hFF").expect("no neg widened");

    assert_eq!(neg_same_width.output, "1'b1");
    assert_eq!(neg_widened.output, "1'b1");
    assert_eq!(no_neg_widened.output, "1'b0");
}

#[test]
fn equality_unary_minus_propagates_unsigned_context_to_primary() {
    // -4'sb1000 in 8-bit unsigned context: inner 4'sb1000 zero-extends to
    // 0000_1000 = 8; negate at 8-bit unsigned → 256-8 = 248. 248 == 248 → 1.
    let eq = evaluate_input("-4'sb1000 == 8'd248").expect("neg eq");
    // Same mechanism: -4'sd1 in 8-bit unsigned context becomes 255.
    let neg_one = evaluate_input("-4'sd1 == 8'hFF").expect("neg one eq");

    assert_eq!(eq.output, "1'b1");
    assert_eq!(neg_one.output, "1'b1");
}

#[test]
fn equality_both_signed_uses_sign_extension() {
    // Both signed → context signed → narrower side sign-extends.
    // -4'sd1 sign-extends to 8-bit -1 = 1111_1111; -8'sd1 same bits → equal.
    let neg_neg = evaluate_input("-4'sd1 == -8'sd1").expect("neg neg");
    // === on identical signed bit patterns → 1.
    let case_neg = evaluate_input("4'sb1111 === 4'sb1111").expect("case neg");

    assert_eq!(neg_neg.output, "1'b1");
    assert_eq!(case_neg.output, "1'b1");
}

#[test]
fn logical_equality_returns_x_only_when_ambiguous() {
    // All-x: nothing definite to mismatch on → ambiguous → x.
    let all_x = evaluate_input("4'bx == 4'd1").expect("all x");
    // RHS all-z: same reasoning → x.
    let all_z = evaluate_input("4'd0 == 4'bz").expect("all z");
    // Identical bit pattern with one x: also ambiguous → x.
    let same_x = evaluate_input("4'b01x0 == 4'b01x0").expect("same with x");
    // Definite mismatch elsewhere (bit[2]: 1 vs 0) makes operands
    // unequal regardless of the x bit → != is 1, not x.
    let definite_mismatch_eq = evaluate_input("4'b01x0 == 4'd1").expect("definite mismatch ==");
    let definite_mismatch_ne = evaluate_input("4'b01x0 != 4'd1").expect("definite mismatch !=");
    // No definite mismatch, only an x at bit[0] → ambiguous → x.
    let ambiguous_eq = evaluate_input("4'b101x == 4'b1010").expect("ambiguous ==");
    let ambiguous_ne = evaluate_input("4'b101x != 4'b1010").expect("ambiguous !=");

    assert_eq!(all_x.output, "1'bx");
    assert_eq!(all_z.output, "1'bx");
    assert_eq!(same_x.output, "1'bx");
    assert_eq!(definite_mismatch_eq.output, "1'b0");
    assert_eq!(definite_mismatch_ne.output, "1'b1");
    assert_eq!(ambiguous_eq.output, "1'bx");
    assert_eq!(ambiguous_ne.output, "1'bx");
}

#[test]
fn case_equality_matches_x_and_z_literally() {
    // === requires bit-for-bit identity including x and z; result never x.
    let xxxx_eq = evaluate_input("4'bxxxx === 4'bxxxx").expect("xxxx eq");
    let mixed_eq = evaluate_input("4'bx101 === 4'bx101").expect("mixed eq");
    let mixed_ne_diff = evaluate_input("4'bx101 !== 4'bx100").expect("mixed ne diff");
    let xxxx_vs_zero = evaluate_input("4'bxxxx === 4'd0").expect("xxxx vs zero");
    let x_vs_one = evaluate_input("4'bx101 === 4'b1101").expect("x vs one");
    let zzzz_eq = evaluate_input("4'bzzzz === 4'bzzzz").expect("zzzz eq");
    let xz_pattern = evaluate_input("4'bxzxz === 4'bxzxz").expect("xz pattern");
    let same_ne = evaluate_input("4'bxxxx !== 4'bxxxx").expect("same !==");

    assert_eq!(xxxx_eq.output, "1'b1");
    assert_eq!(mixed_eq.output, "1'b1");
    assert_eq!(mixed_ne_diff.output, "1'b1");
    assert_eq!(xxxx_vs_zero.output, "1'b0");
    assert_eq!(x_vs_one.output, "1'b0");
    assert_eq!(zzzz_eq.output, "1'b1");
    assert_eq!(xz_pattern.output, "1'b1");
    assert_eq!(same_ne.output, "1'b0");
}

#[test]
fn case_equality_extends_unsigned_with_zero_not_x() {
    // LRM 5.5.4: x/z fill on extension applies only to SIGNED resize.
    // For mixed signedness (unsigned context) the narrower side
    // zero-extends regardless of MSB, so 4'bx101 becomes 0000_x101.
    let zero_filled = evaluate_input("4'bx101 === 8'b0000x101").expect("zero filled");
    let not_x_filled = evaluate_input("4'bx101 === 8'bxxxxx101").expect("not x filled");
    // Same for z.
    let z_zero_filled = evaluate_input("4'bz101 === 8'b0000z101").expect("z zero filled");
    let z_not_z_filled = evaluate_input("4'bz101 === 8'bzzzzz101").expect("z not z filled");

    assert_eq!(zero_filled.output, "1'b1");
    assert_eq!(not_x_filled.output, "1'b0");
    assert_eq!(z_zero_filled.output, "1'b1");
    assert_eq!(z_not_z_filled.output, "1'b0");
}

#[test]
fn case_equality_signed_extends_msb_with_x_or_z() {
    // LRM 5.5.4: when BOTH operands are signed (context signed), an x or
    // z MSB does propagate into the upper bits.
    let signed_x_fill = evaluate_input("4'sbx000 === 8'sbxxxxx000").expect("signed x fills");
    let signed_zero_fill_wrong =
        evaluate_input("4'sbx000 === 8'sb0000x000").expect("signed zero would be wrong");

    assert_eq!(signed_x_fill.output, "1'b1");
    assert_eq!(signed_zero_fill_wrong.output, "1'b0");
}

#[test]
fn equality_lower_precedence_than_relational() {
    // LRM 5.1.8: equality is lower precedence than relational. So
    // `4'd1 < 4'd2 == 4'd1` parses as `(4'd1 < 4'd2) == 4'd1`, which is
    // 1'b1 == 4'd1 → 1 == 1 → 1. The other grouping would yield 0.
    let result = evaluate_input("4'd1 < 4'd2 == 4'd1").expect("precedence");
    assert_eq!(result.output, "1'b1");
}

#[test]
fn equality_is_left_associative() {
    // 4'd1 == 4'd1 == 4'd1 → (1 == 1) == 4'd1 → 1'b1 == 4'd1 → 1 == 1 → 1
    let result = evaluate_input("4'd1 == 4'd1 == 4'd1").expect("assoc");
    let expr = parse_expression("4'd1 == 4'd1 == 4'd1").expect("parse assoc");
    match &expr {
        Expr::Binary {
            op: BinaryOp::Equal,
            lhs,
            ..
        } => assert!(matches!(
            lhs.as_ref(),
            Expr::Binary {
                op: BinaryOp::Equal,
                ..
            }
        )),
        other => panic!("expected top-level ==, got {other:?}"),
    }
    assert_eq!(result.output, "1'b1");
}

#[test]
fn equality_result_widens_to_outer_arithmetic_context() {
    // (4'd3 == 4'd3) → 1'b1; outer + widens result to 4 bits.
    let result = evaluate_input("(4'd3 == 4'd3) + 4'd0").expect("widened");
    assert_eq!(result.output, "4'b0001");
}
