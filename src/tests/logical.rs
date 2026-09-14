use crate::lexer::{Token, tokenize};
use crate::parser::{BinaryOp, Expr, UnaryOp, parse_expression};
use crate::{Session, evaluate_input};

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

// ---------- Mixed integer/real logical operands ----------
//
// LRM 5.1.9 reduces each operand to a single logical bit *before* the
// truth table applies. When one operand is real it is tempting to convert
// the other to real as well (LRM 3.5.3), but that conversion maps every
// x/z bit to 0.0 and would answer 0 where the truth table says x. Each
// operand therefore reduces in its own type: the real side via
// `logical_value_of_real`, the integer side via `logical_value`
// (reduction-OR, x/z preserved). Every expectation below matches Icarus
// Verilog.

#[test]
fn mixed_real_logical_keeps_unknown_integer_state() {
    let x_and_one = evaluate_input("1'bx && 1.0").expect("x && 1.0");
    let x_or_zero = evaluate_input("1'bx || 0.0").expect("x || 0.0");
    let z_and_one = evaluate_input("1'bz && 1.0").expect("z && 1.0");
    let z_or_zero = evaluate_input("1'bz || 0.0").expect("z || 0.0");
    let real_on_left = evaluate_input("1.0 && 1'bx").expect("1.0 && x");
    let zero_real_on_left = evaluate_input("0.0 && 1'bx").expect("0.0 && x");

    assert_eq!(x_and_one.output, "1'bx");
    assert_eq!(x_or_zero.output, "1'bx");
    assert_eq!(z_and_one.output, "1'bx");
    assert_eq!(z_or_zero.output, "1'bx");
    assert_eq!(real_on_left.output, "1'bx");
    assert_eq!(zero_real_on_left.output, "1'b0");
}

#[test]
fn mixed_real_logical_still_honors_dominant_definite_operand() {
    // A definite 0 dominates `&&` and a definite 1 dominates `||` even when
    // the other operand is unknown — the truth table's 0/1 rows beat x.
    let x_and_zero = evaluate_input("1'bx && 0.0").expect("x && 0.0");
    let x_or_one = evaluate_input("1'bx || 1.0").expect("x || 1.0");
    let definite_true = evaluate_input("1'b1 && 1.0").expect("1 && 1.0");
    let definite_false = evaluate_input("1'b0 || 1.0").expect("0 || 1.0");

    assert_eq!(x_and_zero.output, "1'b0");
    assert_eq!(x_or_one.output, "1'b1");
    assert_eq!(definite_true.output, "1'b1");
    assert_eq!(definite_false.output, "1'b1");
}

#[test]
fn mixed_real_logical_reduces_multibit_integer_by_or() {
    // The integer side reduces with OR across all its bits: any 1 bit makes
    // it definitely true, all-zero makes it definitely false, and x/z with no
    // 1 bit stays unknown.
    let zeros_and_x = evaluate_input("4'b00x0 && 1.0").expect("00x0 && 1.0");
    let zeros_and_z = evaluate_input("4'b00z0 && 1.0").expect("00z0 && 1.0");
    let has_one_bit = evaluate_input("4'b11x1 && 1.0").expect("11x1 && 1.0");
    let all_zero = evaluate_input("4'b0000 && 1.0").expect("0000 && 1.0");
    let x_with_one_or = evaluate_input("4'bx000 || 1.0").expect("x000 || 1.0");

    assert_eq!(zeros_and_x.output, "1'bx");
    assert_eq!(zeros_and_z.output, "1'bx");
    assert_eq!(has_one_bit.output, "1'b1");
    assert_eq!(all_zero.output, "1'b0");
    assert_eq!(x_with_one_or.output, "1'b1");
}

#[test]
fn mixed_real_logical_with_real_and_reg_variables() {
    let mut session = Session::new();
    session.eval("real zero = 0.0").expect("real decl");
    session.eval("real one = 1.5").expect("real decl");
    session.eval("reg b").expect("scalar reg decl");
    session.eval("reg [3:0] v").expect("vector reg decl");

    // `b` and `v` are uninitialized, so both read as unknown.
    assert_eq!(session.eval("b && zero").expect("x && 0.0").output, "1'b0");
    assert_eq!(session.eval("b || zero").expect("x || 0.0").output, "1'bx");
    assert_eq!(session.eval("zero && b").expect("0.0 && x").output, "1'b0");
    assert_eq!(session.eval("b && one").expect("x && 1.5").output, "1'bx");
    assert_eq!(
        session.eval("v && 1.0").expect("xxxx && 1.0").output,
        "1'bx"
    );
    assert_eq!(
        session.eval("v || 1.0").expect("xxxx || 1.0").output,
        "1'b1"
    );
}

#[test]
fn mixed_real_logical_result_feeds_outer_operators() {
    let or_dominates = evaluate_input("(1'bx && 1.0) || 1'b1").expect("nested ||");
    let and_stays_unknown = evaluate_input("(1'bx && 1.0) && 1'b1").expect("nested &&");
    let real_subtree = evaluate_input("1'bx && (1.0 + 0.0)").expect("real subtree");
    let in_concat = evaluate_input("{1'bx && 1.0}").expect("concat");
    let as_conditional = evaluate_input("(1'bx && 1.0) ? 1 : 2").expect("cond");

    assert_eq!(or_dominates.output, "1'b1");
    assert_eq!(and_stays_unknown.output, "1'bx");
    assert_eq!(real_subtree.output, "1'bx");
    assert_eq!(in_concat.output, "1'bx");
    assert_eq!(as_conditional.output, "32'sdx");
}

#[test]
fn real_relational_and_equality_still_convert_unknown_to_zero() {
    // Guard against over-correcting: only the *logical* operators reduce in
    // the operand's own type. `==`, `!=`, and the relational operators still
    // apply the LRM 3.5.3 real conversion, where x/z becomes 0.0 — matching
    // Icarus Verilog, which answers 0 for `1'bx == 1.0` and 1 for
    // `1'bx < 1.0`.
    let equal = evaluate_input("1'bx == 1.0").expect("x == 1.0");
    let not_equal = evaluate_input("1'bx != 1.0").expect("x != 1.0");
    let less_than = evaluate_input("1'bx < 1.0").expect("x < 1.0");
    let greater_equal = evaluate_input("1'bx >= 1.0").expect("x >= 1.0");

    assert_eq!(equal.output, "1'b0");
    assert_eq!(not_equal.output, "1'b1");
    assert_eq!(less_than.output, "1'b1");
    assert_eq!(greater_equal.output, "1'b0");
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
