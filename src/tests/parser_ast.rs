use crate::evaluate_input;
use crate::parser::{BinaryOp, Expr, UnaryOp, parse_expression, parse_integer};

#[test]
fn parses_parenthesized_literal_expression() {
    let evaluation = evaluate_input("(42)").expect("parenthesized literal should parse");
    assert_eq!(evaluation.output, "32'sd42");
}

#[test]
fn parses_binary_operator_precedence_into_ast() {
    let expression = parse_expression("1 + 2 * 3").expect("expression should parse");

    assert_eq!(
        expression,
        Expr::Binary {
            op: BinaryOp::Add,
            lhs: Box::new(Expr::Literal(
                parse_integer("1").expect("literal should parse")
            )),
            rhs: Box::new(Expr::Binary {
                op: BinaryOp::Multiply,
                lhs: Box::new(Expr::Literal(
                    parse_integer("2").expect("literal should parse")
                )),
                rhs: Box::new(Expr::Literal(
                    parse_integer("3").expect("literal should parse")
                )),
            }),
        }
    );
}

#[test]
fn parses_unary_and_power_operators_into_ast() {
    let expression = parse_expression("-2 ** 3").expect("expression should parse");

    assert_eq!(
        expression,
        Expr::Binary {
            op: BinaryOp::Power,
            lhs: Box::new(Expr::Unary {
                op: UnaryOp::Minus,
                expr: Box::new(Expr::Literal(
                    parse_integer("2").expect("literal should parse")
                )),
            }),
            rhs: Box::new(Expr::Literal(
                parse_integer("3").expect("literal should parse")
            )),
        }
    );
}

#[test]
fn unary_minus_binds_tighter_than_power() {
    let even_exp = evaluate_input("-2 ** 2").expect("even exponent should evaluate");
    let odd_exp = evaluate_input("-2 ** 3").expect("odd exponent should evaluate");

    assert_eq!(even_exp.output, "32'sd4");
    assert_eq!(odd_exp.output, "-32'sd8");
}

#[test]
fn parses_power_operator_left_associatively() {
    let expression = parse_expression("3 ** 3 ** 3").expect("expression should parse");

    assert_eq!(
        expression,
        Expr::Binary {
            op: BinaryOp::Power,
            lhs: Box::new(Expr::Binary {
                op: BinaryOp::Power,
                lhs: Box::new(Expr::Literal(
                    parse_integer("3").expect("literal should parse")
                )),
                rhs: Box::new(Expr::Literal(
                    parse_integer("3").expect("literal should parse")
                )),
            }),
            rhs: Box::new(Expr::Literal(
                parse_integer("3").expect("literal should parse")
            )),
        }
    );
}

#[test]
fn evaluates_chained_power_left_to_right() {
    let evaluation = evaluate_input("3 ** 3 ** 3").expect("chained power should evaluate");
    assert_eq!(evaluation.output, "32'sd19683");
}

#[test]
fn rejects_missing_closing_parenthesis() {
    let error = parse_expression("(1 + 2").expect_err("expression should be rejected");
    assert_eq!(error, "missing closing parenthesis");
}
