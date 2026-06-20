use crate::evaluate_input;
use crate::lexer::{Token, tokenize};
use crate::parser::{BinaryOp, Expr, UnaryOp, parse_expression};

// ---------- Logical operators (!, &&, ||) ----------
//
// Expected values follow the LRM 1364-2005 §5.1.9 Table 5-7 truth tables.
// Operands of !, &&, || are self-determined (LRM §5.4 Table 5-22) — each
// operand is reduced to a 1-bit logical value before the truth table
// applies, so width unification is irrelevant.

#[test]
fn tokenizes_logical_operators_as_single_tokens() {
    let and = tokenize("4'd1 && 4'd0").expect("&& should tokenize");
    let or = tokenize("4'd1 || 4'd0").expect("|| should tokenize");
    let bang = tokenize("!4'd0").expect("! should tokenize");

    assert_eq!(and[1], Token::LogicalAnd);
    assert_eq!(or[1], Token::LogicalOr);
    assert_eq!(bang[0], Token::Bang);
}

#[test]
fn evaluates_logical_not_truth_table() {
    let not_zero = evaluate_input("!1'b0").expect("!0");
    let not_one = evaluate_input("!1'b1").expect("!1");
    let not_x = evaluate_input("!1'bx").expect("!x");
    let not_z = evaluate_input("!1'bz").expect("!z");

    assert_eq!(not_zero.output, "1'b1");
    assert_eq!(not_one.output, "1'b0");
    assert_eq!(not_x.output, "1'bx");
    assert_eq!(not_z.output, "1'bx");
}

#[test]
fn logical_not_reduces_across_operand_width() {
    // Any 1 bit makes the operand definitely true; all-zero is false; an x
    // or z with no 1 bit is ambiguous → x. A 1 bit defeats x in the
    // reduction, so 4'b01x0 → false, not x.
    let not_five = evaluate_input("!4'd5").expect("!5");
    let not_zero8 = evaluate_input("!8'd0").expect("!8'd0");
    let not_x_only = evaluate_input("!4'b00x0").expect("!00x0");
    let not_one_with_x = evaluate_input("!4'b01x0").expect("!01x0");

    assert_eq!(not_five.output, "1'b0");
    assert_eq!(not_zero8.output, "1'b1");
    assert_eq!(not_x_only.output, "1'bx");
    assert_eq!(not_one_with_x.output, "1'b0");
}

#[test]
fn evaluates_logical_and_truth_table() {
    // Table 5-7 cases including the "0 dominates x" and "1 && 1 = 1" rows.
    let true_and_true = evaluate_input("4'd1 && 4'd1").expect("1&&1");
    let true_and_false = evaluate_input("4'd5 && 4'd0").expect("5&&0");
    let false_and_true = evaluate_input("4'd0 && 4'd5").expect("0&&5");
    let false_and_x = evaluate_input("4'd0 && 4'bx").expect("0&&x");
    let x_and_false = evaluate_input("4'bx && 4'd0").expect("x&&0");
    let x_and_true = evaluate_input("4'bx && 4'd1").expect("x&&1");
    let x_and_x = evaluate_input("4'bx && 4'bx").expect("x&&x");

    assert_eq!(true_and_true.output, "1'b1");
    assert_eq!(true_and_false.output, "1'b0");
    assert_eq!(false_and_true.output, "1'b0");
    assert_eq!(false_and_x.output, "1'b0");
    assert_eq!(x_and_false.output, "1'b0");
    assert_eq!(x_and_true.output, "1'bx");
    assert_eq!(x_and_x.output, "1'bx");
}

#[test]
fn evaluates_logical_or_truth_table() {
    let true_or_false = evaluate_input("4'd1 || 4'd0").expect("1||0");
    let false_or_false = evaluate_input("4'd0 || 4'd0").expect("0||0");
    let false_or_true = evaluate_input("4'd0 || 4'd5").expect("0||5");
    let true_or_x = evaluate_input("4'd1 || 4'bx").expect("1||x");
    let x_or_true = evaluate_input("4'bx || 4'd1").expect("x||1");
    let x_or_false = evaluate_input("4'bx || 4'd0").expect("x||0");
    let x_or_x = evaluate_input("4'bx || 4'bx").expect("x||x");

    assert_eq!(true_or_false.output, "1'b1");
    assert_eq!(false_or_false.output, "1'b0");
    assert_eq!(false_or_true.output, "1'b1");
    assert_eq!(true_or_x.output, "1'b1");
    assert_eq!(x_or_true.output, "1'b1");
    assert_eq!(x_or_false.output, "1'bx");
    assert_eq!(x_or_x.output, "1'bx");
}

#[test]
fn logical_result_renders_in_binary_regardless_of_operand_base() {
    // Operands hex but the 1-bit logical result is binary, like
    // relational/equality.
    let hex_and = evaluate_input("8'h0a && 8'h0f").expect("hex &&");
    let hex_or = evaluate_input("8'h00 || 8'h0f").expect("hex ||");
    let hex_not = evaluate_input("!8'h0a").expect("hex !");

    assert_eq!(hex_and.output, "1'b1");
    assert_eq!(hex_or.output, "1'b1");
    assert_eq!(hex_not.output, "1'b0");
}

#[test]
fn logical_not_binds_tighter_than_power() {
    // LRM Table 5-4: unary operators (including !) are higher precedence
    // than **. So `!4'd0 ** 4'd2` parses as `(!4'd0) ** 4'd2` → 1**2 → 1.
    let expr = parse_expression("!4'd0 ** 4'd2").expect("parse");
    match &expr {
        Expr::Binary {
            op: BinaryOp::Power,
            lhs,
            ..
        } => assert!(matches!(
            lhs.as_ref(),
            Expr::Unary {
                op: UnaryOp::LogicalNot,
                ..
            }
        )),
        other => panic!("expected top-level **, got {other:?}"),
    }
    let result = evaluate_input("!4'd0 ** 4'd2").expect("eval");
    assert_eq!(result.output, "1'b1");
}

#[test]
fn logical_and_lower_precedence_than_equality() {
    // `4'd0 == 4'd0 && 4'd1` parses as `(4'd0 == 4'd0) && 4'd1`.
    let expr = parse_expression("4'd0 == 4'd0 && 4'd1").expect("parse");
    match &expr {
        Expr::Binary {
            op: BinaryOp::LogicalAnd,
            lhs,
            ..
        } => assert!(matches!(
            lhs.as_ref(),
            Expr::Binary {
                op: BinaryOp::Equal,
                ..
            }
        )),
        other => panic!("expected top-level &&, got {other:?}"),
    }
    let result = evaluate_input("4'd0 == 4'd0 && 4'd1").expect("eval");
    assert_eq!(result.output, "1'b1");
}

#[test]
fn logical_or_lower_precedence_than_logical_and() {
    // `4'd1 || 4'd0 && 4'd0` parses as `4'd1 || (4'd0 && 4'd0)` → 1.
    let expr = parse_expression("4'd1 || 4'd0 && 4'd0").expect("parse");
    match &expr {
        Expr::Binary {
            op: BinaryOp::LogicalOr,
            rhs,
            ..
        } => assert!(matches!(
            rhs.as_ref(),
            Expr::Binary {
                op: BinaryOp::LogicalAnd,
                ..
            }
        )),
        other => panic!("expected top-level ||, got {other:?}"),
    }
    let result = evaluate_input("4'd1 || 4'd0 && 4'd0").expect("eval");
    assert_eq!(result.output, "1'b1");
}

#[test]
fn logical_not_chains_recursively() {
    // !! parses as `!(!x)` because `!` is right-associative through
    // the recursive parse_unary; it also lets us test that the inner
    // 1'b0 from `!4'd5` is correctly fed back into `!`.
    let result = evaluate_input("!!4'd5").expect("!!5");
    let zero = evaluate_input("!!4'd0").expect("!!0");

    assert_eq!(result.output, "1'b1");
    assert_eq!(zero.output, "1'b0");
}

#[test]
fn logical_and_is_left_associative() {
    // a && b && c parses as (a && b) && c; same shape check as the
    // existing equality_is_left_associative test.
    let expr = parse_expression("4'd1 && 4'd1 && 4'd1").expect("parse");
    match &expr {
        Expr::Binary {
            op: BinaryOp::LogicalAnd,
            lhs,
            ..
        } => assert!(matches!(
            lhs.as_ref(),
            Expr::Binary {
                op: BinaryOp::LogicalAnd,
                ..
            }
        )),
        other => panic!("expected top-level &&, got {other:?}"),
    }
    let result = evaluate_input("4'd1 && 4'd1 && 4'd0").expect("eval");
    assert_eq!(result.output, "1'b0");
}

#[test]
fn logical_or_is_left_associative() {
    let expr = parse_expression("4'd0 || 4'd0 || 4'd1").expect("parse");
    match &expr {
        Expr::Binary {
            op: BinaryOp::LogicalOr,
            lhs,
            ..
        } => assert!(matches!(
            lhs.as_ref(),
            Expr::Binary {
                op: BinaryOp::LogicalOr,
                ..
            }
        )),
        other => panic!("expected top-level ||, got {other:?}"),
    }
    let result = evaluate_input("4'd0 || 4'd0 || 4'd1").expect("eval");
    assert_eq!(result.output, "1'b1");
}

#[test]
fn logical_result_widens_to_outer_arithmetic_context() {
    // (4'd1 && 4'd1) → 1'b1; outer + widens to 4 bits and inherits the
    // leftmost operand's binary base (the && result's base).
    let result = evaluate_input("(4'd1 && 4'd1) + 4'd0").expect("widened &&");
    let or_widened = evaluate_input("(4'd0 || 4'd0) + 4'd0").expect("widened ||");
    let not_widened = evaluate_input("(!4'd0) + 4'd0").expect("widened !");

    assert_eq!(result.output, "4'b0001");
    assert_eq!(or_widened.output, "4'b0000");
    assert_eq!(not_widened.output, "4'b0001");
}
