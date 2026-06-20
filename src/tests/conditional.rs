use crate::parser::{BinaryOp, Expr, parse_expression};
use crate::{Session, evaluate_input};

#[test]
fn conditional_selects_then_when_cond_true() {
    // LRM 5.1.13: a definite-true cond returns expression2, in the
    // unified width of then/else (4 bits here).
    let result = evaluate_input("1 ? 4'd5 : 4'd9").expect("eval");
    assert_eq!(result.output, "4'd5");
}

#[test]
fn conditional_selects_else_when_cond_false() {
    let result = evaluate_input("1'b0 ? 4'd5 : 4'd9").expect("eval");
    assert_eq!(result.output, "4'd9");
}

#[test]
fn conditional_reduces_wide_cond_to_logical() {
    // LRM 5.1.13: cond is self-determined and reduced to a 1-bit
    // logical (any 1 → true, all 0 → false).
    let any_one = evaluate_input("4'b1000 ? 4'd5 : 4'd9").expect("any 1");
    let all_zero = evaluate_input("4'b0000 ? 4'd5 : 4'd9").expect("all 0");

    assert_eq!(any_one.output, "4'd5");
    assert_eq!(all_zero.output, "4'd9");
}

#[test]
fn conditional_ambiguous_cond_merges_when_branches_agree() {
    // LRM 5.1.13: when cond is x, evaluate both branches and merge per
    // bit. Identical branches collapse to the shared value.
    let result = evaluate_input("1'bx ? 4'b1100 : 4'b1100").expect("eval");
    assert_eq!(result.output, "4'b1100");
}

#[test]
fn conditional_ambiguous_cond_merges_per_bit_with_disagreement() {
    // 1'bx ? 4'b1100 : 4'b1010 → bits are (1,1)=1, (1,0)=x, (0,1)=x,
    // (0,0)=0. With LSB-first storage rendered MSB-first as `1xx0`.
    let result = evaluate_input("1'bx ? 4'b1100 : 4'b1010").expect("eval");
    assert_eq!(result.output, "4'b1xx0");
}

#[test]
fn conditional_ambiguous_cond_handles_xz_bits() {
    // x agrees with x (stays x); z agrees with z (stays z, since the
    // merge keeps the shared bit verbatim); x vs z disagrees → x.
    let xx = evaluate_input("1'bx ? 1'bx : 1'bx").expect("xx");
    let zz = evaluate_input("1'bx ? 1'bz : 1'bz").expect("zz");
    let xz = evaluate_input("1'bx ? 1'bx : 1'bz").expect("xz");

    assert_eq!(xx.output, "1'bx");
    assert_eq!(zz.output, "1'bz");
    assert_eq!(xz.output, "1'bx");
}

#[test]
fn conditional_unifies_then_else_widths() {
    // Result width = max(L(then), L(else)). Selecting the narrower
    // branch zero-extends to the unified width.
    let result = evaluate_input("1 ? 4'd5 : 8'd1").expect("eval");
    assert_eq!(result.output, "8'd5");
}

#[test]
fn conditional_signedness_propagates_per_5_5_1() {
    // LRM 5.5.1: any unsigned operand → unsigned result. Pairing a
    // signed and an unsigned branch yields an unsigned conditional.
    let mixed = evaluate_input("1 ? 4'sd1 : 4'd1").expect("mixed");
    let both_signed = evaluate_input("1 ? 4'sd1 : 4'sd1").expect("both signed");

    assert_eq!(mixed.output, "4'd1");
    assert_eq!(both_signed.output, "4'sd1");
}

#[test]
fn untaken_conditional_branch_still_rejects_real_vector_bit_select_index() {
    let mut session = Session::new();
    session.eval("reg [3:0] r").expect("decl");
    let err = session
        .eval("1 ? 4'd1 : r[1.0]")
        .expect_err("real vector select index should fail during meta inference");
    assert!(err.contains("bit-select index cannot be real"));
}

#[test]
fn untaken_conditional_branch_still_rejects_real_array_element_index() {
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    let err = session
        .eval("1 ? 4'd1 : a[1.0]")
        .expect_err("real array element index should fail during meta inference");
    assert!(err.contains("array element index cannot be real"));
}

#[test]
fn untaken_conditional_branch_still_rejects_real_array_inner_bit_select_index() {
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    let err = session
        .eval("1 ? 4'd1 : a[0][1.0]")
        .expect_err("real inner bit-select index should fail during meta inference");
    assert!(err.contains("bit-select index cannot be real"));
}

#[test]
fn untaken_conditional_branch_still_rejects_array_inner_part_direction_mismatch() {
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    let err = session
        .eval("1 ? 4'd1 : a[0][0:3]")
        .expect_err("inner direction mismatch should fail during meta inference");
    assert!(err.contains("part-select direction does not match"));
}

#[test]
fn untaken_real_conditional_branch_still_rejects_bitstoreal_wrong_width() {
    let err = evaluate_input("1 ? 1.0 : $bitstoreal(1'b0)")
        .expect_err("bad $bitstoreal width should fail during branch validation");
    assert!(err.contains("$bitstoreal argument must be 64 bits wide"));
}

#[test]
fn untaken_real_conditional_branch_still_rejects_modulus_on_real() {
    let err = evaluate_input("1 ? 1.0 : 1.0 % 1")
        .expect_err("real modulus should fail during branch validation");
    assert!(err.contains("operator % not allowed on real operand"));
}

#[test]
fn untaken_real_conditional_branch_still_rejects_sign_cast_on_real() {
    let err = evaluate_input("1 ? 1.0 : $signed(1.0)")
        .expect_err("sign cast on real should fail during branch validation");
    assert!(err.contains("$signed argument cannot be real"));
}

#[test]
fn untaken_real_conditional_branch_still_rejects_system_task() {
    let err = evaluate_input("1 ? 1.0 : ($finish)")
        .expect_err("system task should fail during branch validation");
    assert!(err.contains("system task"));
}

#[test]
fn hidden_error_in_conditional_condition_is_still_rejected() {
    let mut session = Session::new();
    session.eval("reg [3:0] r").expect("decl");
    let err = session
        .eval("(1 ? 1'b1 : r[1.0]) ? 1'b1 : 1'b0")
        .expect_err("invalid select in condition should fail before branch choice");
    assert!(err.contains("bit-select index cannot be real"));
}

#[test]
fn conditional_extends_per_5_5_2_not_per_5_1_13() {
    // LRM §5.1.13 last paragraph says the shorter branch is zero-filled
    // from the left, but §5.5.2 says signed-signed unifies under a
    // signed propagated context and the narrower side sign-extends.
    // The two rules disagree for `1 ? 4'shF : 8'sh0`. vcal follows
    // §5.5.2, consistent with the bitwise path:
    //   - both signed → sign-extend the narrower branch.
    //   - any unsigned → unsigned context → zero-extend.
    let both_signed = evaluate_input("1 ? 4'shF : 8'sh0").expect("both signed");
    let mixed_unsigned = evaluate_input("1 ? 4'shF : 8'h0").expect("mixed");

    assert_eq!(both_signed.output, "8'shff");
    assert_eq!(mixed_unsigned.output, "8'h0f");
}

#[test]
fn conditional_outer_arithmetic_context_widens_branches() {
    // Self-determined `1 ? 4'd8 : 4'd0` is 4'd8. Inside a 32-bit
    // context the branches first widen to 32 bits before selection,
    // matching the shape of the existing shift-widening test.
    let self_determined = evaluate_input("1 ? 4'd8 : 4'd0").expect("self");
    let widened = evaluate_input("(1 ? 4'd8 : 4'd0) + 0").expect("widened");

    assert_eq!(self_determined.output, "4'd8");
    assert_eq!(widened.output, "32'd8");
}

#[test]
fn conditional_outer_unsigned_context_zero_fills_signed_branch() {
    // Mirror of the shift `>>>` propagation test. Same conditional, two
    // outer contexts:
    //   Self-determined: signed → 4'sb1000 sign-extends nowhere
    //     (already 4 bits).
    //   Mixed unsigned (8'd0): result type is unsigned → 4'sb1000
    //     zero-extends to 8'b00001000.
    //   All-signed (signed `0`): result type is signed → 4'sb1000
    //     sign-extends to 32'sb1...11000.
    let self_determined = evaluate_input("1 ? 4'sb1000 : 4'sb1000").expect("self");
    let mixed = evaluate_input("(1 ? 4'sb1000 : 4'sb1000) + 8'd0").expect("mixed");
    let all_signed = evaluate_input("(1 ? 4'sb1000 : 4'sb1000) + 0").expect("all signed");

    assert_eq!(self_determined.output, "4'sb1000");
    assert_eq!(mixed.output, "8'b00001000");
    assert_eq!(all_signed.output, "32'sb11111111111111111111111111111000");
}

#[test]
fn conditional_is_right_associative() {
    // `1'b0 ? 1 : 1'b1 ? 2 : 3` parses as `1'b0 ? 1 : (1'b1 ? 2 : 3)`.
    // Cond is false, so the else branch runs, picking 2.
    let expr = parse_expression("1'b0 ? 1 : 1'b1 ? 2 : 3").expect("parse");
    match &expr {
        Expr::Conditional { else_expr, .. } => {
            assert!(matches!(else_expr.as_ref(), Expr::Conditional { .. }));
        }
        other => panic!("expected top-level conditional, got {other:?}"),
    }
    let result = evaluate_input("1'b0 ? 1 : 1'b1 ? 2 : 3").expect("eval");
    // Unsized integer literals are 32-bit signed (LRM 3.5.1), so all
    // three branches are signed and the result keeps signedness.
    assert_eq!(result.output, "32'sd2");
}

#[test]
fn conditional_lower_precedence_than_logical_or() {
    // LRM Table 5-4: `?:` sits below `||`. `1 || 0 ? 1 : 2` parses as
    // `(1 || 0) ? 1 : 2`, picking the then branch.
    let expr = parse_expression("1 || 0 ? 1 : 2").expect("parse");
    match &expr {
        Expr::Conditional { cond, .. } => {
            assert!(matches!(
                cond.as_ref(),
                Expr::Binary {
                    op: BinaryOp::LogicalOr,
                    ..
                }
            ));
        }
        other => panic!("expected top-level conditional, got {other:?}"),
    }
    let result = evaluate_input("1 || 0 ? 1 : 2").expect("eval");
    assert_eq!(result.output, "32'sd1");
}

#[test]
fn conditional_lower_precedence_than_relational_and_arithmetic() {
    // `2 > 1 ? 5 : 6` parses as `(2 > 1) ? 5 : 6` and `1 + 1 ? 3 : 4`
    // parses as `(1 + 1) ? 3 : 4` — both lower-precedence operators
    // bind into the cond.
    let relational = evaluate_input("2 > 1 ? 5 : 6").expect("relational cond");
    let arithmetic = evaluate_input("1 + 1 ? 3 : 4").expect("arithmetic cond");

    assert_eq!(relational.output, "32'sd5");
    assert_eq!(arithmetic.output, "32'sd3");
}

#[test]
fn conditional_inherits_then_branch_base() {
    // Result base follows the then branch (the leftmost bit-pattern
    // operand after the cond), mirroring leftmost-wins for binaries.
    let hex_then = evaluate_input("1 ? 8'h0a : 8'd5").expect("hex then");
    let dec_then = evaluate_input("1 ? 8'd10 : 8'h05").expect("dec then");

    assert_eq!(hex_then.output, "8'h0a");
    assert_eq!(dec_then.output, "8'd10");
}

#[test]
fn conditional_missing_colon_is_parse_error() {
    // A `?` without `:` should not silently parse as something else.
    let err = evaluate_input("1 ? 2").expect_err("missing colon");
    assert_eq!(err, "Syntax error: expected `:` in conditional expression");
}

#[test]
fn conditional_chained_in_else_position() {
    // `0 ? 1 : 0 ? 2 : 3` is right-associative so it evaluates as
    // `0 ? 1 : (0 ? 2 : 3)` = `0 ? 1 : 3` = 3.
    let result = evaluate_input("1'b0 ? 4'd1 : 1'b0 ? 4'd2 : 4'd3").expect("eval");
    assert_eq!(result.output, "4'd3");
}
