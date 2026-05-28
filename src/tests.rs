use std::io::Cursor;

use num_bigint::BigInt;

use crate::lexer::{Token, tokenize};
use crate::parser::{BinaryOp, Expr, UnaryOp, parse_expression, parse_integer};
use crate::{Session, evaluate_input, run_repl};

#[test]
fn evaluates_unsized_decimal() {
    let evaluation = evaluate_input("42").expect("decimal literal should parse");
    assert_eq!(evaluation.output, "32'sd42");
    assert!(!evaluation.should_exit);
}

#[test]
fn evaluates_unsized_hex_with_32_bit_width() {
    let evaluation = evaluate_input("'hFF").expect("hex literal should parse");
    assert_eq!(evaluation.output, "32'h000000ff");
}

#[test]
fn evaluates_sized_signed_decimal() {
    let evaluation = evaluate_input("8'Sd255;").expect("signed decimal should parse");
    assert_eq!(evaluation.output, "-8'sd1");
}

#[test]
fn formats_signed_decimal_and_non_decimal_outputs_differently() {
    let simple_decimal = evaluate_input("1").expect("simple decimal should parse");
    let simple_negative = evaluate_input("-1").expect("simple negative should evaluate");
    let signed_positive = evaluate_input("4'sd1").expect("signed decimal should parse");
    let signed_negative =
        evaluate_input("-4'sd1").expect("signed decimal negation should evaluate");
    let signed_hex = evaluate_input("4'shF").expect("signed hex should parse");

    assert_eq!(simple_decimal.output, "32'sd1");
    assert_eq!(simple_negative.output, "-32'sd1");
    assert_eq!(signed_positive.output, "4'sd1");
    assert_eq!(signed_negative.output, "-4'sd1");
    assert_eq!(signed_hex.output, "4'shf");
}

#[test]
fn accepts_spaces_inside_based_integer_literals_in_expressions() {
    let literal = evaluate_input("8 'd 6").expect("spaced based literal should parse");
    let unary = evaluate_input("- 8 'd 6").expect("spaced unary minus literal should parse");
    let expr =
        evaluate_input("8 'd 6 + 1").expect("spaced based literal expression should parse");

    assert_eq!(literal.output, "8'd6");
    assert_eq!(unary.output, "8'd250");
    assert_eq!(expr.output, "32'd7");
}

#[test]
fn rejects_spaces_inside_base_token() {
    let missing_base =
        evaluate_input("8 ' d 6").expect_err("space after apostrophe should be rejected");
    let split_signed = evaluate_input("8 ' s d 6")
        .expect_err("spaces inside signed base token should be rejected");
    let split_signed_base =
        evaluate_input("8 's d 6").expect_err("space between s and base should be rejected");

    assert_eq!(missing_base, "Syntax error: missing base after apostrophe");
    assert_eq!(split_signed, "Syntax error: missing base after apostrophe");
    assert_eq!(split_signed_base, "Syntax error: missing base after signed marker");
}

#[test]
fn accepts_apostrophe_led_based_literals_with_spaced_digits() {
    let hex = evaluate_input("'h 837FF").expect("apostrophe-led hex literal should parse");
    let signed_hex =
        evaluate_input("'sh f").expect("apostrophe-led signed hex literal should parse");

    assert_eq!(hex.output, "32'h000837ff");
    assert_eq!(signed_hex.output, "32'sh0000000f");
}

#[test]
fn accepts_underscores_in_size_and_digits() {
    let decimal = evaluate_input("1_6'd1_0").expect("underscored decimal should parse");
    let hex = evaluate_input("'hff_ff").expect("underscored hex should parse");

    assert_eq!(decimal.output, "16'd10");
    assert_eq!(hex.output, "32'h0000ffff");
}

#[test]
fn evaluates_based_literal_with_unknown_digits() {
    let evaluation = evaluate_input("4'b10x?").expect("binary literal should parse");
    assert_eq!(evaluation.output, "4'b10xz");
}

#[test]
fn extends_sized_literals_from_their_leftmost_digit_kind() {
    let zero_extended = evaluate_input("4'b1").expect("binary literal should parse");
    let x_extended = evaluate_input("4'bx").expect("x literal should parse");
    let z_extended = evaluate_input("4'b?").expect("z literal should parse");
    let hex_extended = evaluate_input("8'hf").expect("hex literal should parse");

    assert_eq!(zero_extended.output, "4'b0001");
    assert_eq!(x_extended.output, "4'bxxxx");
    assert_eq!(z_extended.output, "4'bzzzz");
    assert_eq!(hex_extended.output, "8'h0f");
}

#[test]
fn keeps_unsized_literals_wider_than_32_bits_when_needed() {
    let evaluation =
        evaluate_input("4294967296").expect("wide unsized decimal literal should parse");
    assert_eq!(evaluation.output, "34'sd4294967296");
}

// LRM Table 5-22 footnote a: an unsized x/z-leading constant in an
// expression wider than 32 bits extends by the MSB regardless of the
// propagated context signedness. Sized x/z operands still follow §5.5.4
// (zero-fill in unsigned propagated context).
#[test]
fn unsized_x_literal_msb_extends_in_wider_unsigned_context() {
    let bitwise = evaluate_input("'bx | 64'b0").expect("expression should evaluate");
    assert_eq!(
        bitwise.output,
        "64'bxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
    );

    let case_eq = evaluate_input("'bx === 64'bx").expect("expression should evaluate");
    assert_eq!(case_eq.output, "1'b1");
}

#[test]
fn unsized_x_literal_msb_extends_regardless_of_mixed_signedness() {
    let unsigned_unsized_signed_sized =
        evaluate_input("'bx | 64'sb0").expect("expression should evaluate");
    assert_eq!(
        unsigned_unsized_signed_sized.output,
        "64'bxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
    );

    let signed_unsized_unsigned_sized =
        evaluate_input("'sbx | 64'b0").expect("expression should evaluate");
    assert_eq!(
        signed_unsized_unsigned_sized.output,
        "64'bxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
    );
}

#[test]
fn unsized_signed_literal_sign_extends_per_own_signedness_in_unsigned_context() {
    // 'shFFFFFFFF is signed with MSB=1 at the 32-bit default. Per footnote
    // a's "Otherwise" branch, the own (signed) signedness drives a sign-
    // extend even though the propagated context is unsigned. §5.5.4 would
    // instead zero-extend and yield 64'h00000000FFFFFFFF.
    let evaluation =
        evaluate_input("'shFFFFFFFF | 64'b0").expect("expression should evaluate");
    assert_eq!(evaluation.output, "64'hffffffffffffffff");
}

#[test]
fn outer_context_propagates_to_unsized_leaf_through_inner_expression() {
    let nested = evaluate_input("('bx | 4'b0) | 64'b0").expect("expression should evaluate");
    assert_eq!(
        nested.output,
        "64'bxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
    );

    // Without the outer 64-bit context the inner expression stays at its
    // own self-determined max(32, 4) = 32 bits.
    let alone = evaluate_input("('bx | 4'b0)").expect("expression should evaluate");
    assert_eq!(alone.output, "32'bxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");
}

#[test]
fn sized_operands_still_follow_propagated_context_extension() {
    // 32'sbx is sized signed with MSB=x. Mixed with 34'b0 (unsigned) the
    // propagated context is unsigned, so §5.5.4 zero-fills the two extra
    // MSB positions even though the operand's MSB is x.
    let mixed = evaluate_input("32'sbx | 34'b0").expect("expression should evaluate");
    assert_eq!(mixed.output, "34'b00xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");

    // Both signed → propagated signed → MSB-fill carries x up.
    let both_signed = evaluate_input("32'sbx | 34'sb0").expect("expression should evaluate");
    assert_eq!(
        both_signed.output,
        "34'sbxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
    );
}

#[test]
fn unsized_value_literals_unchanged_in_wider_context() {
    // Sanity: value (non-x/z) literals should produce the same bits
    // whether we eager-size at 32 + §5.5.4 extend, or footnote-a extend.
    let unsigned_hex =
        evaluate_input("'h7FFFFFFF | 64'b0").expect("expression should evaluate");
    assert_eq!(unsigned_hex.output, "64'h000000007fffffff");

    let signed_decimal = evaluate_input("42 + 64'sb0").expect("expression should evaluate");
    assert_eq!(signed_decimal.output, "64'sd42");
}

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

#[test]
fn evaluates_unary_and_binary_additive_operators() {
    let unary_plus = evaluate_input("+5").expect("unary plus should evaluate");
    let unary_minus = evaluate_input("-5").expect("unary minus should evaluate");
    let addition = evaluate_input("4'd15 + 4'd1").expect("addition should evaluate");
    let subtraction = evaluate_input("4'd0 - 4'd1").expect("subtraction should evaluate");

    assert_eq!(unary_plus.output, "32'sd5");
    assert_eq!(unary_minus.output, "-32'sd5");
    assert_eq!(addition.output, "4'd0");
    assert_eq!(subtraction.output, "4'd15");
}

#[test]
fn unary_plus_preserves_operand_bits_including_unknowns() {
    let binary = evaluate_input("+4'b01xz").expect("unary plus should preserve bits");
    let decimal = evaluate_input("+1").expect("unary plus on simple decimal should evaluate");

    assert_eq!(binary.output, "4'b01xz");
    assert_eq!(decimal.output, "32'sd1");
}

#[test]
fn widens_nested_addition_from_parent_context() {
    let evaluation = evaluate_input("4'd15 + 4'd1 + 0").expect("addition should evaluate");
    assert_eq!(evaluation.output, "32'd16");
}

#[test]
fn returns_all_x_when_additive_operand_contains_unknown_bits() {
    let addition = evaluate_input("4'bx + 1").expect("x addition should evaluate");
    let unary = evaluate_input("-4'bz").expect("z unary minus should evaluate");

    // Result base inherits from leftmost operand (binary), so the all-x result
    // is rendered in binary, one digit per bit.
    assert_eq!(addition.output, "32'bxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");
    assert_eq!(unary.output, "4'bxxxx");
}

#[test]
fn evaluates_multiplicative_operators() {
    let multiply = evaluate_input("4'd3 * 4'd5").expect("multiply should evaluate");
    let divide = evaluate_input("8'd21 / 8'd4").expect("divide should evaluate");
    let modulus = evaluate_input("8'd21 % 8'd4").expect("modulus should evaluate");

    assert_eq!(multiply.output, "4'd15");
    assert_eq!(divide.output, "8'd5");
    assert_eq!(modulus.output, "8'd1");
}

#[test]
fn applies_width_rules_to_multiplicative_expressions() {
    let truncated = evaluate_input("4'd8 * 4'd4").expect("multiply should evaluate");
    let widened =
        evaluate_input("4'd8 * 4'd4 + 0").expect("context-widened multiply should evaluate");

    assert_eq!(truncated.output, "4'd0");
    assert_eq!(widened.output, "32'd32");
}

#[test]
fn returns_all_x_for_multiplicative_unknowns_and_zero_division() {
    let unknown = evaluate_input("4'bx * 2").expect("unknown multiply should evaluate");
    let divide_by_zero =
        evaluate_input("8'd3 / 8'd0").expect("divide by zero should evaluate to x");
    let modulus_by_zero =
        evaluate_input("8'd3 % 8'd0").expect("modulus by zero should evaluate to x");

    assert_eq!(unknown.output, "32'bxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");
    assert_eq!(divide_by_zero.output, "8'dx");
    assert_eq!(modulus_by_zero.output, "8'dx");
}

#[test]
fn evaluates_power_operator() {
    let square = evaluate_input("4'd3 ** 2").expect("power should evaluate");
    let zero_exp = evaluate_input("4'd2 ** 0").expect("zero exponent should evaluate");
    let negative_exp = evaluate_input("4'd2 ** -1").expect("negative exponent should evaluate");

    assert_eq!(square.output, "4'd9");
    assert_eq!(zero_exp.output, "4'd1");
    assert_eq!(negative_exp.output, "4'd0");
}

#[test]
fn applies_lhs_width_rule_to_power_operator() {
    let self_determined = evaluate_input("4'd3 ** 4'd3").expect("power should evaluate");
    let context_widened =
        evaluate_input("4'd3 ** 4'd3 + 0").expect("power should widen in context");

    assert_eq!(self_determined.output, "4'd11");
    assert_eq!(context_widened.output, "32'd27");
}

#[test]
fn returns_all_x_for_power_unknowns_and_undefined_zero_negative_exponent() {
    let unknown = evaluate_input("4'bx ** 2").expect("unknown power should evaluate");
    let undefined = evaluate_input("0 ** -1").expect("undefined integer power should yield x");

    assert_eq!(unknown.output, "4'bxxxx");
    assert_eq!(undefined.output, "32'sdx");
}

#[test]
fn zero_extends_signed_operands_in_mixed_unsigned_expressions() {
    let addition = evaluate_input("4'sd15 + 4'd1").expect("mixed add should evaluate");
    let division = evaluate_input("4'sd8 / 4'd2").expect("mixed divide should evaluate");

    assert_eq!(addition.output, "4'd0");
    assert_eq!(division.output, "4'd4");
}

#[test]
fn preserves_signed_results_when_all_operands_are_signed() {
    let addition = evaluate_input("4'sd15 + 4'sd1").expect("signed add should evaluate");
    let division = evaluate_input("4'sd8 / 4'sd2").expect("signed divide should evaluate");
    let modulus = evaluate_input("4'sd15 % 4'sd2").expect("signed modulus should evaluate");

    assert_eq!(addition.output, "4'sd0");
    assert_eq!(division.output, "-4'sd4");
    assert_eq!(modulus.output, "-4'sd1");
}

#[test]
fn handles_signed_negative_values_in_arithmetic() {
    let addition = evaluate_input("-4'sd1 + 4'sd1").expect("signed add should evaluate");
    let division = evaluate_input("-4'sd8 / 4'sd2").expect("signed divide should evaluate");
    let modulus = evaluate_input("-4'sd8 % 4'sd3").expect("signed modulus should evaluate");

    assert_eq!(addition.output, "4'sd0");
    assert_eq!(division.output, "-4'sd4");
    assert_eq!(modulus.output, "-4'sd2");
}

#[test]
fn widens_signed_subexpressions_before_truncation() {
    let evaluation =
        evaluate_input("(-4'sd1 + -4'sd1) + 0").expect("signed expression should evaluate");
    assert_eq!(evaluation.output, "-32'sd2");
}

#[test]
fn evaluates_negative_base_power_cases_from_lrm_examples() {
    let odd = evaluate_input("(-4'sd1) ** 3").expect("odd negative-base power should evaluate");
    let even =
        evaluate_input("(-4'sd1) ** 2").expect("even negative-base power should evaluate");
    let reciprocal =
        evaluate_input("(-4'sd1) ** -3").expect("negative exponent should evaluate");

    assert_eq!(odd.output, "-4'sd1");
    assert_eq!(even.output, "4'sd1");
    assert_eq!(reciprocal.output, "-4'sd1");
}

#[test]
fn accepts_finish_and_stop_with_optional_parens() {
    let finish = evaluate_input("$finish()").expect("$finish() should parse");
    let stop = evaluate_input("$stop();").expect("$stop() should parse");

    assert_eq!(finish.output, "");
    assert!(finish.should_exit);
    assert_eq!(stop.output, "");
    assert!(stop.should_exit);
}

// LRM 17.4 allows `$finish[(n)]` where n is a verbosity level. vcal prints
// no exit diagnostic so the argument is meaningless — accept and discard.
#[test]
fn accepts_finish_with_single_integer_argument() {
    for input in ["$finish(0)", "$finish(1)", "$finish(2)"] {
        let result = evaluate_input(input).unwrap_or_else(|err| panic!("{input}: {err}"));
        assert!(result.should_exit, "{input}");
    }
}

// vcal is intentionally lenient about the LRM 0-or-1 arity rule: enforcing
// it would teach users a restriction vcal itself does not act on, since the
// argument list is never inspected.
#[test]
fn accepts_finish_and_stop_with_extra_arguments() {
    let finish = evaluate_input("$finish(0, 1, 2)").expect("multi-arg finish");
    assert!(finish.should_exit);
    let stop = evaluate_input("$stop(1, 2, 3)").expect("multi-arg stop");
    assert!(stop.should_exit);
}

// Argument is parsed for syntactic validity but never evaluated, so a
// would-be runtime error inside the argument still exits cleanly.
#[test]
fn finish_argument_is_not_evaluated() {
    let result = evaluate_input("$finish(1/0)").expect("expression arg should parse and discard");
    assert!(result.should_exit);
}

// `($finish)` collapses to a bare SystemTask after the top-level Grouped
// unwrap, so parenthesisation does not change exit behavior.
#[test]
fn grouped_system_task_still_exits() {
    let result = evaluate_input("($finish)").expect("grouped task should exit");
    assert!(result.should_exit);
    let nested = evaluate_input("(($stop()))").expect("nested-grouped task should exit");
    assert!(nested.should_exit);
}

// Identifier match is exact: `$finisher` is a valid system_function_identifier
// per LRM A.9.3 but is not in vcal's supported set, so it surfaces the
// generic "unsupported" message rather than the task-in-expression one.
#[test]
fn task_like_identifier_with_trailing_chars_is_unknown_function() {
    let error = evaluate_input("$finisher").expect_err("$finisher is not supported");
    assert!(
        error.contains("unsupported system function: $finisher"),
        "got: {error}"
    );
    let error = evaluate_input("$stop_clock").expect_err("$stop_clock is not supported");
    assert!(
        error.contains("unsupported system function: $stop_clock"),
        "got: {error}"
    );
}

// When `$finish` / `$stop` appears inside an expression — at any position,
// with or without parens, with or without arguments — the evaluator
// surfaces the task-in-expression diagnostic. The message intentionally
// uses the `$name()` function-call form to convey what the user is doing
// wrong, regardless of how they wrote the task.
#[test]
fn system_task_in_expression_is_rejected() {
    for input in [
        "1 + $finish",
        "$finish + 1",
        "$finish() + 1",
        "1 + $finish(0)",
        "1 + $stop",
        "$stop() ? 1 : 2",
        "-$finish",
        "{$finish, 4'b0}",
    ] {
        let error = evaluate_input(input).expect_err(input);
        assert!(
            error.contains("is a system task")
                && error.contains("cannot be called as a function"),
            "{input}: got {error}"
        );
    }
}

// Syntactic malformation inside the argument list still surfaces a parse
// error — leniency is about value/arity, not malformed syntax.
#[test]
fn system_task_with_malformed_argument_is_parse_error() {
    let error = evaluate_input("$finish(1 +)").expect_err("trailing + should be a parse error");
    assert!(!error.is_empty());
    let error = evaluate_input("$finish(1,)").expect_err("trailing comma should be a parse error");
    assert!(!error.is_empty());
    let error = evaluate_input("$finish(").expect_err("unclosed paren should be a parse error");
    assert!(!error.is_empty());
}

#[test]
fn runs_repl_until_exit_command() {
    let mut input = Cursor::new("42\n$finish\nignored\n");
    let mut output = Vec::new();

    run_repl(&mut input, &mut output).expect("REPL should run");

    let output = String::from_utf8(output).expect("output should be valid UTF-8");
    assert_eq!(output, "In[0]: Out[0]: 32'sd42\nIn[1]: Out[1]: \n");
}

#[test]
fn repl_emits_error_lines_and_continues_to_next_prompt() {
    // On evaluation failure the REPL prints an empty `Out[N]: ` followed
    // by the message on its own line (the message already carries a
    // stage prefix like `Syntax error:` / `Semantic error:` when one
    // applies), then advances the index and prompts for the next input
    // — it does not abort or skip the index. Sequence: bad input →
    // error, then valid input → result, then exit.
    let mut input = Cursor::new("1 +\n42\n$finish\n");
    let mut output = Vec::new();

    run_repl(&mut input, &mut output).expect("REPL should run");

    let output = String::from_utf8(output).expect("output should be valid UTF-8");
    assert_eq!(
        output,
        "In[0]: Out[0]: \n\
         Syntax error: unexpected end of expression\n\
         In[1]: Out[1]: 32'sd42\n\
         In[2]: Out[2]: \n",
    );
}

// Stage-prefix sanity: the `Syntax error:` / `Semantic error:` prefixes
// tell the user which phase rejected their input. Parser/lexer errors get
// the syntax prefix; validator errors get the semantic prefix. Genuine
// runtime conditions (division by zero, out-of-range bit-select, etc.)
// are absorbed into x bits per LRM and never surface as Err, so there is
// no third "no-prefix" case worth pinning down with a test.
#[test]
fn syntax_error_prefix_distinguishes_stage() {
    let err = evaluate_input("1 +").expect_err("trailing operator");
    assert!(
        err.starts_with("Syntax error:"),
        "parser errors should carry the Syntax error prefix, got: {err}"
    );
}

#[test]
fn semantic_error_prefix_distinguishes_stage() {
    // `$bitstoreal(1'b0)` parses cleanly; the rejection is the static
    // "argument must be 64 bits wide" check that lives in the validator.
    let err = evaluate_input("$bitstoreal(1'b0)").expect_err("wrong-width $bitstoreal");
    assert!(
        err.starts_with("Semantic error:"),
        "validator errors should carry the Semantic error prefix, got: {err}"
    );
}

#[test]
fn strips_multiple_trailing_semicolons() {
    // `strip_statement_terminators` loops, removing trailing `;` until
    // none remain. `1 + 1;;` strips to `1 + 1`.
    let result = evaluate_input("1 + 1;;").expect("eval");
    assert_eq!(result.output, "32'sd2");
}

#[test]
fn strips_trailing_semicolons_with_intervening_whitespace() {
    // Whitespace between (and after) the `;` separators is folded by the
    // trim_end() inside the loop, so `1 + 1 ; ; ;` and `1 + 1;` evaluate
    // to the same thing.
    let result = evaluate_input("1 + 1 ; ; ;").expect("eval");
    assert_eq!(result.output, "32'sd2");
}

#[test]
fn only_semicolons_produces_empty_output() {
    let result = evaluate_input(";;;").expect("eval");
    assert_eq!(result.output, "");
}

#[test]
fn empty_input_produces_empty_output() {
    let result = evaluate_input("").expect("eval");
    assert_eq!(result.output, "");
}

#[test]
fn whitespace_only_input_produces_empty_output() {
    let result = evaluate_input("   \t  ").expect("eval");
    assert_eq!(result.output, "");
}

#[test]
fn binary_arithmetic_preserves_shared_operand_base() {
    let binary_add = evaluate_input("4'b0111 + 4'b1001").expect("binary add should evaluate");
    let hex_add = evaluate_input("8'h0a + 8'h05").expect("hex add should evaluate");
    let hex_mul = evaluate_input("8'h0a * 8'h02").expect("hex multiply should evaluate");
    let hex_power = evaluate_input("4'h2 ** 2").expect("hex power should evaluate");

    assert_eq!(binary_add.output, "4'b0000");
    assert_eq!(hex_add.output, "8'h0f");
    assert_eq!(hex_mul.output, "8'h14");
    assert_eq!(hex_power.output, "4'h4");
}

#[test]
fn binary_arithmetic_takes_leftmost_base_when_operands_differ() {
    let hex_then_binary = evaluate_input("8'h0a + 8'b1").expect("hex+binary should evaluate");
    let binary_then_hex =
        evaluate_input("8'b00001010 + 8'h05").expect("binary+hex should evaluate");

    assert_eq!(hex_then_binary.output, "8'h0b");
    assert_eq!(binary_then_hex.output, "8'b00001111");
}

#[test]
fn unary_minus_preserves_operand_base() {
    let binary = evaluate_input("-4'b1").expect("binary unary minus should evaluate");
    let hex = evaluate_input("-8'h01").expect("hex unary minus should evaluate");

    assert_eq!(binary.output, "4'b1111");
    assert_eq!(hex.output, "8'hff");
}

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
    match expr {
        Expr::Binary {
            op: BinaryOp::LessThan,
            lhs,
            ..
        } => assert!(matches!(*lhs, Expr::Binary { op: BinaryOp::Add, .. })),
        other => panic!("expected top-level <, got {other:?}"),
    }

    let result = evaluate_input("1 + 2 < 4").expect("precedence");
    assert_eq!(result.output, "1'b1");
}

#[test]
fn relational_is_left_associative() {
    // 4 < 5 < 1 parses as (4 < 5) < 1 → 1 < 1 → false
    let expr = parse_expression("4 < 5 < 1").expect("parse");
    match expr {
        Expr::Binary {
            op: BinaryOp::LessThan,
            lhs,
            ..
        } => assert!(matches!(
            *lhs,
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
    let signed_x_fill =
        evaluate_input("4'sbx000 === 8'sbxxxxx000").expect("signed x fills");
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
    match expr {
        Expr::Binary {
            op: BinaryOp::Equal,
            lhs,
            ..
        } => assert!(matches!(*lhs, Expr::Binary { op: BinaryOp::Equal, .. })),
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
    match expr {
        Expr::Binary {
            op: BinaryOp::Power,
            lhs,
            ..
        } => assert!(matches!(
            *lhs,
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
    match expr {
        Expr::Binary {
            op: BinaryOp::LogicalAnd,
            lhs,
            ..
        } => assert!(matches!(*lhs, Expr::Binary { op: BinaryOp::Equal, .. })),
        other => panic!("expected top-level &&, got {other:?}"),
    }
    let result = evaluate_input("4'd0 == 4'd0 && 4'd1").expect("eval");
    assert_eq!(result.output, "1'b1");
}

#[test]
fn logical_or_lower_precedence_than_logical_and() {
    // `4'd1 || 4'd0 && 4'd0` parses as `4'd1 || (4'd0 && 4'd0)` → 1.
    let expr = parse_expression("4'd1 || 4'd0 && 4'd0").expect("parse");
    match expr {
        Expr::Binary {
            op: BinaryOp::LogicalOr,
            rhs,
            ..
        } => assert!(matches!(
            *rhs,
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
    match expr {
        Expr::Binary {
            op: BinaryOp::LogicalAnd,
            lhs,
            ..
        } => assert!(matches!(
            *lhs,
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
    match expr {
        Expr::Binary {
            op: BinaryOp::LogicalOr,
            lhs,
            ..
        } => assert!(matches!(
            *lhs,
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

// ---------- Bitwise operators (~, &, |, ^, ~^/^~) ----------
//
// Expected values follow the LRM 1364-2005 §5.1.10 Tables 5-9..5-12 truth
// tables. Per LRM Table 5-22, per-bit `~` and the binary forms are
// context-determined like arithmetic: width = max(L(lhs), L(rhs)), signed
// iff both operands signed.

#[test]
fn tokenizes_bitwise_single_char_operators() {
    // Bare & and | are no longer rejected: they tokenize as their
    // bitwise forms when not followed by a second &/|.
    let amp = tokenize("4'd1 & 4'd0").expect("& should tokenize");
    let pipe = tokenize("4'd1 | 4'd0").expect("| should tokenize");
    let xor = tokenize("4'd1 ^ 4'd0").expect("^ should tokenize");
    let tilde = tokenize("~4'd0").expect("~ should tokenize");

    assert_eq!(amp[1], Token::BitwiseAnd);
    assert_eq!(pipe[1], Token::BitwiseOr);
    assert_eq!(xor[1], Token::BitwiseXor);
    assert_eq!(tilde[0], Token::Tilde);
}

#[test]
fn tokenizes_xnor_with_either_spelling() {
    // LRM 5.1.10: `^~` and `~^` denote the same operator; both lex to a
    // single BitwiseXnor token so downstream code does not branch on
    // spelling.
    let tilde_caret = tokenize("4'd1 ~^ 4'd0").expect("~^ should tokenize");
    let caret_tilde = tokenize("4'd1 ^~ 4'd0").expect("^~ should tokenize");

    assert_eq!(tilde_caret[1], Token::BitwiseXnor);
    assert_eq!(caret_tilde[1], Token::BitwiseXnor);
}

#[test]
fn double_amp_and_pipe_still_lex_as_logical() {
    // Greedy two-char matching must win over the bare bitwise tokens;
    // otherwise && would silently become two & tokens.
    let and = tokenize("4'd1 && 4'd0").expect("&& should tokenize");
    let or = tokenize("4'd1 || 4'd0").expect("|| should tokenize");

    assert_eq!(and[1], Token::LogicalAnd);
    assert_eq!(or[1], Token::LogicalOr);
}

#[test]
fn evaluates_bitwise_not_truth_table() {
    let zero = evaluate_input("~1'b0").expect("~0");
    let one = evaluate_input("~1'b1").expect("~1");
    let x = evaluate_input("~1'bx").expect("~x");
    let z = evaluate_input("~1'bz").expect("~z");

    assert_eq!(zero.output, "1'b1");
    assert_eq!(one.output, "1'b0");
    assert_eq!(x.output, "1'bx");
    assert_eq!(z.output, "1'bx");
}

#[test]
fn bitwise_not_flips_each_bit_independently() {
    // Per-bit operation: x and z fold to x; other bits flip. Crucially
    // there is no all-x short-circuit (unlike arithmetic), so known and
    // unknown bits coexist in the result.
    let mixed = evaluate_input("~4'b01xz").expect("~01xz");
    let all_zeros = evaluate_input("~4'b0000").expect("~0000");

    assert_eq!(mixed.output, "4'b10xx");
    assert_eq!(all_zeros.output, "4'b1111");
}

#[test]
fn bitwise_not_preserves_operand_base() {
    let binary = evaluate_input("~4'b0001").expect("binary ~");
    let hex = evaluate_input("~8'h0a").expect("hex ~");

    assert_eq!(binary.output, "4'b1110");
    assert_eq!(hex.output, "8'hf5");
}

#[test]
fn bitwise_not_chains() {
    // parse_unary recurses, so ~~ parses as ~(~x).
    let result = evaluate_input("~~4'b0101").expect("~~0101");
    assert_eq!(result.output, "4'b0101");
}

#[test]
fn bitwise_not_widens_through_outer_arithmetic_context() {
    // Self-determined: ~4'b0001 = 4'b1110. With outer + 0 (32-bit signed
    // 0 makes the shared context 32-bit unsigned), the operand widens to
    // 32 bits BEFORE the negation runs, so we get 32 ones except the
    // LSB. Leftmost-base wins, so result is binary.
    let widened = evaluate_input("~4'b0001 + 0").expect("widened ~");
    assert_eq!(widened.output, "32'b11111111111111111111111111111110");
}

#[test]
fn bitwise_not_binds_tighter_than_power() {
    // LRM Table 5-4: unary ~ is tighter than **, so `~4'd1 ** 2` parses
    // as `(~4'd1) ** 2`. ~4'd1 self-determined at 4-bit unsigned is
    // 4'b1110 = 14; 14**2 = 196; 196 mod 16 = 4. Result base inherits
    // from the lhs (decimal), so 4'd4.
    let expr = parse_expression("~4'd1 ** 2").expect("parse");
    match expr {
        Expr::Binary {
            op: BinaryOp::Power,
            lhs,
            ..
        } => assert!(matches!(
            *lhs,
            Expr::Unary {
                op: UnaryOp::BitwiseNot,
                ..
            }
        )),
        other => panic!("expected top-level **, got {other:?}"),
    }
    let result = evaluate_input("~4'd1 ** 2").expect("eval");
    assert_eq!(result.output, "4'd4");
}

#[test]
fn evaluates_bitwise_and_truth_table() {
    // LRM Table 5-9: 0 dominates AND, x/z elsewhere → x, only 1&1 yields 1.
    let zero_zero = evaluate_input("1'b0 & 1'b0").expect("0&0");
    let one_one = evaluate_input("1'b1 & 1'b1").expect("1&1");
    let zero_x = evaluate_input("1'b0 & 1'bx").expect("0&x");
    let one_x = evaluate_input("1'b1 & 1'bx").expect("1&x");
    let one_z = evaluate_input("1'b1 & 1'bz").expect("1&z");
    let x_z = evaluate_input("1'bx & 1'bz").expect("x&z");

    assert_eq!(zero_zero.output, "1'b0");
    assert_eq!(one_one.output, "1'b1");
    assert_eq!(zero_x.output, "1'b0");
    assert_eq!(one_x.output, "1'bx");
    assert_eq!(one_z.output, "1'bx");
    assert_eq!(x_z.output, "1'bx");
}

#[test]
fn evaluates_bitwise_or_truth_table() {
    // Symmetric to AND with 1 dominating.
    let zero_zero = evaluate_input("1'b0 | 1'b0").expect("0|0");
    let one_zero = evaluate_input("1'b1 | 1'b0").expect("1|0");
    let one_x = evaluate_input("1'b1 | 1'bx").expect("1|x");
    let zero_x = evaluate_input("1'b0 | 1'bx").expect("0|x");
    let zero_z = evaluate_input("1'b0 | 1'bz").expect("0|z");
    let x_z = evaluate_input("1'bx | 1'bz").expect("x|z");

    assert_eq!(zero_zero.output, "1'b0");
    assert_eq!(one_zero.output, "1'b1");
    assert_eq!(one_x.output, "1'b1");
    assert_eq!(zero_x.output, "1'bx");
    assert_eq!(zero_z.output, "1'bx");
    assert_eq!(x_z.output, "1'bx");
}

#[test]
fn evaluates_bitwise_xor_truth_table() {
    // XOR has no dominator: any x/z anywhere → x. Otherwise standard XOR.
    let zero_one = evaluate_input("1'b0 ^ 1'b1").expect("0^1");
    let one_one = evaluate_input("1'b1 ^ 1'b1").expect("1^1");
    let zero_zero = evaluate_input("1'b0 ^ 1'b0").expect("0^0");
    let one_x = evaluate_input("1'b1 ^ 1'bx").expect("1^x");
    let zero_z = evaluate_input("1'b0 ^ 1'bz").expect("0^z");

    assert_eq!(zero_one.output, "1'b1");
    assert_eq!(one_one.output, "1'b0");
    assert_eq!(zero_zero.output, "1'b0");
    assert_eq!(one_x.output, "1'bx");
    assert_eq!(zero_z.output, "1'bx");
}

#[test]
fn evaluates_bitwise_xnor_truth_table_with_either_spelling() {
    // ^~ and ~^ are the same operator (NOT-of-XOR semantics).
    let tilde_caret_eq = evaluate_input("1'b0 ~^ 1'b0").expect("0~^0");
    let caret_tilde_eq = evaluate_input("1'b0 ^~ 1'b0").expect("0^~0");
    let one_one = evaluate_input("1'b1 ^~ 1'b1").expect("1^~1");
    let mixed = evaluate_input("1'b1 ~^ 1'b0").expect("1~^0");
    let one_x = evaluate_input("1'b1 ~^ 1'bx").expect("1~^x");

    assert_eq!(tilde_caret_eq.output, "1'b1");
    assert_eq!(caret_tilde_eq.output, "1'b1");
    assert_eq!(one_one.output, "1'b1");
    assert_eq!(mixed.output, "1'b0");
    assert_eq!(one_x.output, "1'bx");
}

#[test]
fn bitwise_binary_zips_known_and_unknown_bits_per_position() {
    // The arithmetic all-x short-circuit does NOT apply: bitwise ops mix
    // known and unknown bits per position. Worked examples (bit 0 = LSB):
    //
    //   4'b1100 & 4'b10x1 → bits: 0&1=0, 0&x=0 (0 dominates), 1&0=0, 1&1=1 → 4'b1000
    //   4'b1100 | 4'b00x1 → bits: 0|1=1, 0|x=x, 1|0=1, 1|0=1            → 4'b11x1
    //   4'b1100 ^ 4'b00x1 → bits: 0^1=1, 0^x=x, 1^0=1, 1^0=1            → 4'b11x1
    let and = evaluate_input("4'b1100 & 4'b10x1").expect("mixed &");
    let or = evaluate_input("4'b1100 | 4'b00x1").expect("mixed |");
    let xor = evaluate_input("4'b1100 ^ 4'b00x1").expect("mixed ^");

    assert_eq!(and.output, "4'b1000");
    assert_eq!(or.output, "4'b11x1");
    assert_eq!(xor.output, "4'b11x1");
}

#[test]
fn bitwise_binary_uses_max_width_of_operands() {
    // Same width: trivially preserved.
    let same = evaluate_input("4'b1100 & 4'b1010").expect("4&4");
    // Mixed width (both unsigned → unsigned context): narrower operand
    // zero-extends to the wider width before zipping.
    //   8'hff = 8'b11111111; 4'b1010 zero-extends to 8'b00001010;
    //   AND → 8'b00001010.
    let mixed = evaluate_input("8'hff & 4'b1010").expect("8&4");

    assert_eq!(same.output, "4'b1000");
    assert_eq!(mixed.output, "8'h0a");
}

#[test]
fn bitwise_binary_signed_only_when_both_signed() {
    // Both signed → context signed → narrower side sign-extends.
    //   4'sb1111 sign-extends to 8'sb11111111;
    //   & 8'sb01010101 → 8'sb01010101.
    let both_signed = evaluate_input("4'sb1111 & 8'sb01010101").expect("both signed");
    // Mixed → context unsigned → narrower zero-extends.
    //   4'sb1111 zero-extends to 8'b00001111;
    //   & 8'b01010101 → 8'b00000101.
    let mixed = evaluate_input("4'sb1111 & 8'b01010101").expect("mixed");

    assert_eq!(both_signed.output, "8'sb01010101");
    assert_eq!(mixed.output, "8'b00000101");
}

#[test]
fn bitwise_extends_per_5_5_2_not_per_5_1_10() {
    // LRM §5.1.10 says the shorter operand "shall be zero-filled in the
    // most significant bit positions", but §5.5.2 says signed-signed
    // operands unify under a signed propagated context and the narrower
    // side sign-extends. The two rules disagree for `4'shF | 8'sh0`.
    // vcal follows §5.5.2 (the later SystemVerilog clarification drops the
    // §5.1.10 sentence):
    //   - both signed → sign-extend the narrower operand.
    //   - any unsigned → unsigned context → zero-extend.
    let both_signed = evaluate_input("4'shF | 8'sh0").expect("both signed");
    let mixed_unsigned = evaluate_input("4'shF | 8'h0").expect("mixed");

    assert_eq!(both_signed.output, "8'shff");
    assert_eq!(mixed_unsigned.output, "8'h0f");
}

#[test]
fn bitwise_binary_widens_through_outer_arithmetic_context() {
    // Without context-widening these would be 4-bit. With outer + 0
    // (32-bit signed 0 produces 32-bit unsigned shared context), the
    // bitwise operands widen to 32 bits BEFORE zipping. Leftmost-base
    // for the outer + is the bitwise op's binary base.
    let widened_and = evaluate_input("(4'b1100 & 4'b1010) + 0").expect("widened &");
    let widened_or = evaluate_input("(4'b0100 | 4'b1010) + 0").expect("widened |");
    let widened_xor = evaluate_input("(4'b0110 ^ 4'b1010) + 0").expect("widened ^");

    assert_eq!(widened_and.output, "32'b00000000000000000000000000001000");
    assert_eq!(widened_or.output, "32'b00000000000000000000000000001110");
    assert_eq!(widened_xor.output, "32'b00000000000000000000000000001100");
}

#[test]
fn bitwise_band_precedence_below_equality() {
    // `4'd1 == 4'd1 & 4'd1` parses as `(4'd1 == 4'd1) & 4'd1`. The 1-bit
    // 1'b1 zero-extends to 4'b0001 under the unified 4-bit context, then
    // & 4'b0001 → 4'b0001.
    let expr = parse_expression("4'd1 == 4'd1 & 4'd1").expect("parse");
    match expr {
        Expr::Binary {
            op: BinaryOp::BitwiseAnd,
            lhs,
            ..
        } => assert!(matches!(*lhs, Expr::Binary { op: BinaryOp::Equal, .. })),
        other => panic!("expected top-level &, got {other:?}"),
    }
    let result = evaluate_input("4'd1 == 4'd1 & 4'd1").expect("eval");
    assert_eq!(result.output, "4'b0001");
}

#[test]
fn bitwise_band_precedence_above_logical_and() {
    // `4'd1 & 4'd1 && 4'd0` parses as `(4'd1 & 4'd1) && 4'd0`.
    let expr = parse_expression("4'd1 & 4'd1 && 4'd0").expect("parse");
    match expr {
        Expr::Binary {
            op: BinaryOp::LogicalAnd,
            lhs,
            ..
        } => assert!(matches!(
            *lhs,
            Expr::Binary {
                op: BinaryOp::BitwiseAnd,
                ..
            }
        )),
        other => panic!("expected top-level &&, got {other:?}"),
    }
    let result = evaluate_input("4'd1 & 4'd1 && 4'd0").expect("eval");
    assert_eq!(result.output, "1'b0");
}

#[test]
fn bitwise_internal_precedence_and_tightest_or_loosest() {
    // & > ^ > | per LRM Table 5-4.
    //
    //   4'b0110 ^ 4'b0011 & 4'b1100  →  4'b0110 ^ (4'b0011 & 4'b1100)
    //                                = 4'b0110 ^ 4'b0000 = 4'b0110
    //   4'b1000 | 4'b0001 ^ 4'b1010  →  4'b1000 | (4'b0001 ^ 4'b1010)
    //                                = 4'b1000 | 4'b1011 = 4'b1011
    let and_under_xor = evaluate_input("4'b0110 ^ 4'b0011 & 4'b1100").expect("eval");
    let xor_under_or = evaluate_input("4'b1000 | 4'b0001 ^ 4'b1010").expect("eval");

    assert_eq!(and_under_xor.output, "4'b0110");
    assert_eq!(xor_under_or.output, "4'b1011");
}

#[test]
fn bitwise_binary_is_left_associative() {
    // Same shape check used elsewhere: a OP b OP c parses as (a OP b) OP c.
    let expr = parse_expression("4'd1 & 4'd2 & 4'd3").expect("parse");
    match expr {
        Expr::Binary {
            op: BinaryOp::BitwiseAnd,
            lhs,
            ..
        } => assert!(matches!(
            *lhs,
            Expr::Binary {
                op: BinaryOp::BitwiseAnd,
                ..
            }
        )),
        other => panic!("expected top-level &, got {other:?}"),
    }

    // Cross-check XOR chain: (1^2)^3 = 3^3 = 0. Leftmost-base is
    // decimal (all operands `'d`), so the result renders as `4'd0`.
    let xor_chain = evaluate_input("4'd1 ^ 4'd2 ^ 4'd3").expect("eval");
    assert_eq!(xor_chain.output, "4'd0");
}

#[test]
fn bitwise_binary_inherits_leftmost_base() {
    // Same leftmost-wins rule as arithmetic.
    let hex_then_binary = evaluate_input("8'h0a & 8'b00001111").expect("hex&binary");
    let binary_then_hex = evaluate_input("8'b00001111 & 8'h0a").expect("binary&hex");

    assert_eq!(hex_then_binary.output, "8'h0a");
    assert_eq!(binary_then_hex.output, "8'b00001010");
}

// ---------- Reduction unary operators (& ~& | ~| ^ ~^/^~) ----------
//
// Expected values follow the LRM 1364-2005 §5.1.11 fold rules. Per LRM
// Table 5-22, reduction operands are self-determined and the result is
// always 1-bit unsigned that widens through outer arithmetic context like
// `!`, `&&`, `||`, relational, equality.

#[test]
fn tokenizes_nand_and_nor_as_single_tokens() {
    // ~& and ~| are unary-only operators (LRM A.8.6: binary_operator does
    // not list them). They must lex greedily as one token so the parser
    // can claim them at unary position without re-splitting.
    let nand = tokenize("~&4'b1111").expect("~& should tokenize");
    let nor = tokenize("~|4'b0000").expect("~| should tokenize");

    assert_eq!(nand[0], Token::BitwiseNand);
    assert_eq!(nor[0], Token::BitwiseNor);
}

#[test]
fn bare_tilde_unaffected_by_reduction_lexing() {
    // After the ~&/~|/~^ greedy paths, a bare ~ followed by anything else
    // (whitespace, digit-start, paren) must still produce a Tilde token.
    let spaced = tokenize("~ 4'd1").expect("~ + space");
    let parened = tokenize("~(4'd1)").expect("~(...)");

    assert_eq!(spaced[0], Token::Tilde);
    assert_eq!(parened[0], Token::Tilde);
}

#[test]
fn reduction_nand_nor_rejected_as_binary() {
    // No parse_bitwise_* level consumes BitwiseNand/BitwiseNor, so
    // `a ~& b` cleanly fails after the lhs is reduced to a primary.
    let nand = evaluate_input("4'd1 ~& 4'd1").expect_err("binary ~& rejected");
    let nor = evaluate_input("4'd0 ~| 4'd0").expect_err("binary ~| rejected");

    assert_eq!(nand, "Syntax error: unexpected token after end of statement");
    assert_eq!(nor, "Syntax error: unexpected token after end of statement");
}

#[test]
fn evaluates_reduction_and_single_bit_truth_table() {
    // Single-bit reduction degenerates to identity for known values and
    // x for x/z (LRM §5.1.11).
    let zero = evaluate_input("&1'b0").expect("&0");
    let one = evaluate_input("&1'b1").expect("&1");
    let x = evaluate_input("&1'bx").expect("&x");
    let z = evaluate_input("&1'bz").expect("&z");

    assert_eq!(zero.output, "1'b0");
    assert_eq!(one.output, "1'b1");
    assert_eq!(x.output, "1'bx");
    assert_eq!(z.output, "1'bx");
}

#[test]
fn evaluates_reduction_or_single_bit_truth_table() {
    let zero = evaluate_input("|1'b0").expect("|0");
    let one = evaluate_input("|1'b1").expect("|1");
    let x = evaluate_input("|1'bx").expect("|x");
    let z = evaluate_input("|1'bz").expect("|z");

    assert_eq!(zero.output, "1'b0");
    assert_eq!(one.output, "1'b1");
    assert_eq!(x.output, "1'bx");
    assert_eq!(z.output, "1'bx");
}

#[test]
fn evaluates_reduction_xor_single_bit_truth_table() {
    let zero = evaluate_input("^1'b0").expect("^0");
    let one = evaluate_input("^1'b1").expect("^1");
    let x = evaluate_input("^1'bx").expect("^x");
    let z = evaluate_input("^1'bz").expect("^z");

    assert_eq!(zero.output, "1'b0");
    assert_eq!(one.output, "1'b1");
    assert_eq!(x.output, "1'bx");
    assert_eq!(z.output, "1'bx");
}

#[test]
fn evaluates_negated_reduction_single_bit_truth_tables() {
    // The negated forms are NOT-of-positive (per LRM 5.1.11 last
    // sentence). Single-bit cases: known → flipped, x/z → x.
    let nand_one = evaluate_input("~&1'b1").expect("~&1");
    let nand_zero = evaluate_input("~&1'b0").expect("~&0");
    let nand_x = evaluate_input("~&1'bx").expect("~&x");
    let nor_one = evaluate_input("~|1'b1").expect("~|1");
    let nor_zero = evaluate_input("~|1'b0").expect("~|0");
    let nor_z = evaluate_input("~|1'bz").expect("~|z");
    let xnor_one = evaluate_input("~^1'b1").expect("~^1");
    let xnor_zero = evaluate_input("~^1'b0").expect("~^0");
    let xnor_x = evaluate_input("~^1'bx").expect("~^x");

    assert_eq!(nand_one.output, "1'b0");
    assert_eq!(nand_zero.output, "1'b1");
    assert_eq!(nand_x.output, "1'bx");
    assert_eq!(nor_one.output, "1'b0");
    assert_eq!(nor_zero.output, "1'b1");
    assert_eq!(nor_z.output, "1'bx");
    assert_eq!(xnor_one.output, "1'b0");
    assert_eq!(xnor_zero.output, "1'b1");
    assert_eq!(xnor_x.output, "1'bx");
}

#[test]
fn xnor_reduction_accepts_either_spelling() {
    // ^~ and ~^ are the same operator at unary position too.
    let tilde_caret = evaluate_input("~^4'b1100").expect("~^");
    let caret_tilde = evaluate_input("^~4'b1100").expect("^~");

    // 4'b1100 has two 1s → XOR parity = 0 → XNOR = 1.
    assert_eq!(tilde_caret.output, "1'b1");
    assert_eq!(caret_tilde.output, "1'b1");
}

#[test]
fn reduction_and_folds_multi_bit_operand() {
    // 0 dominates AND-reduction even against x/z (because
    // bitwise_and_bits(0, x) = 0). Otherwise: any x/z → x; all-1 → 1.
    let all_ones = evaluate_input("&4'b1111").expect("&1111");
    let has_zero = evaluate_input("&4'b1101").expect("&1101");
    let zero_dominates_over_x = evaluate_input("&4'b110x").expect("&110x");
    let unknown_no_zero = evaluate_input("&4'b111x").expect("&111x");
    let unknown_no_zero_z = evaluate_input("&4'b111z").expect("&111z");
    let unknown_mixed = evaluate_input("&4'b1x1z").expect("&1x1z");

    assert_eq!(all_ones.output, "1'b1");
    assert_eq!(has_zero.output, "1'b0");
    assert_eq!(zero_dominates_over_x.output, "1'b0");
    assert_eq!(unknown_no_zero.output, "1'bx");
    assert_eq!(unknown_no_zero_z.output, "1'bx");
    assert_eq!(unknown_mixed.output, "1'bx");
}

#[test]
fn reduction_or_folds_multi_bit_operand() {
    // Symmetric: 1 dominates OR-reduction. all-0 → 0; any 1 → 1;
    // otherwise any x/z → x.
    let all_zeros = evaluate_input("|4'b0000").expect("|0000");
    let has_one = evaluate_input("|4'b0010").expect("|0010");
    let one_dominates_over_x = evaluate_input("|4'b001x").expect("|001x");
    let unknown_no_one = evaluate_input("|4'b000x").expect("|000x");
    let unknown_no_one_z = evaluate_input("|4'b000z").expect("|000z");

    assert_eq!(all_zeros.output, "1'b0");
    assert_eq!(has_one.output, "1'b1");
    assert_eq!(one_dominates_over_x.output, "1'b1");
    assert_eq!(unknown_no_one.output, "1'bx");
    assert_eq!(unknown_no_one_z.output, "1'bx");
}

#[test]
fn reduction_xor_folds_to_parity_and_x_on_unknowns() {
    // XOR has no dominator: any x/z anywhere → x. Otherwise standard
    // odd-parity.
    let even_parity = evaluate_input("^4'b1111").expect("^1111");
    let odd_parity = evaluate_input("^4'b1110").expect("^1110");
    let zero = evaluate_input("^4'b0000").expect("^0000");
    let unknown = evaluate_input("^4'b111x").expect("^111x");
    let unknown_with_zero = evaluate_input("^4'b110x").expect("^110x");
    let unknown_z = evaluate_input("^4'b00z0").expect("^00z0");

    assert_eq!(even_parity.output, "1'b0");
    assert_eq!(odd_parity.output, "1'b1");
    assert_eq!(zero.output, "1'b0");
    assert_eq!(unknown.output, "1'bx");
    assert_eq!(unknown_with_zero.output, "1'bx");
    assert_eq!(unknown_z.output, "1'bx");
}

#[test]
fn negated_reductions_fold_then_invert() {
    // Spot-check that NAND/NOR/XNOR are exactly NOT-of-positive across
    // multi-bit operands too.
    let nand_all_ones = evaluate_input("~&4'b1111").expect("~&1111");
    let nand_has_zero = evaluate_input("~&4'b1101").expect("~&1101");
    let nand_unknown = evaluate_input("~&4'b111x").expect("~&111x");
    let nor_all_zeros = evaluate_input("~|4'b0000").expect("~|0000");
    let nor_has_one = evaluate_input("~|4'b0010").expect("~|0010");
    let xnor_even = evaluate_input("~^4'b1111").expect("~^1111");
    let xnor_odd = evaluate_input("~^4'b1110").expect("~^1110");
    let xnor_unknown = evaluate_input("~^4'b111x").expect("~^111x");

    assert_eq!(nand_all_ones.output, "1'b0");
    assert_eq!(nand_has_zero.output, "1'b1");
    assert_eq!(nand_unknown.output, "1'bx");
    assert_eq!(nor_all_zeros.output, "1'b1");
    assert_eq!(nor_has_one.output, "1'b0");
    assert_eq!(xnor_even.output, "1'b1");
    assert_eq!(xnor_odd.output, "1'b0");
    assert_eq!(xnor_unknown.output, "1'bx");
}

#[test]
fn reduction_result_renders_in_binary_regardless_of_operand_base() {
    // Operand bases vary but the 1-bit reduction result is always
    // binary, like `!`/relational/equality.
    let hex_and = evaluate_input("&8'hff").expect("&hex");
    let hex_xor = evaluate_input("^8'h05").expect("^hex");
    let dec_or = evaluate_input("|4'd0").expect("|dec");

    assert_eq!(hex_and.output, "1'b1");
    assert_eq!(hex_xor.output, "1'b0");
    assert_eq!(dec_or.output, "1'b0");
}

#[test]
fn reduction_widens_through_outer_arithmetic_context() {
    // (&4'b1111) → 1'b1; outer + widens to the parent's 32-bit decimal
    // context. The reduction result's binary base wins the leftmost-base
    // rule, so the parent + renders in binary.
    let widened = evaluate_input("(&4'b1111) + 0").expect("widened &");
    let widened_xnor = evaluate_input("(~^4'b1110) + 0").expect("widened ~^");

    assert_eq!(widened.output, "32'b00000000000000000000000000000001");
    assert_eq!(widened_xnor.output, "32'b00000000000000000000000000000000");
}

#[test]
fn reduction_position_disambiguates_unary_from_binary_and() {
    // `&4'b1111` — pure unary reduction (1).
    // `4'd1 & &4'b1111` — binary AND with rhs = unary reduction (1 & 1 = 1).
    // `4'd1 & 4'd2` — pure binary AND (0).
    let pure_unary = evaluate_input("&4'b1111").expect("pure unary");
    let mixed = evaluate_input("4'd1 & &4'b1111").expect("binary + unary");
    let pure_binary = evaluate_input("4'd1 & 4'd2").expect("pure binary");

    assert_eq!(pure_unary.output, "1'b1");
    // 4'd1 (4 bits) & (reduced 1'b1, zero-extended to 4 bits = 4'b0001) = 4'b0001.
    // Leftmost-base is decimal (4'd1).
    assert_eq!(mixed.output, "4'd1");
    assert_eq!(pure_binary.output, "4'd0");
}

#[test]
fn reduction_chains_through_recursive_parse_unary() {
    // !!, ~~, and reduction stacks all flow through parse_unary
    // recursively. `&|4'b0110` parses as `&(|4'b0110)` → &(1'b1) → 1.
    // `~~&4'b1111` parses as `~(~(&4'b1111))` → ~(~1) → ~0 → 1.
    let nested_reductions = evaluate_input("&|4'b0110").expect("&|0110");
    let not_chains_into_reduction = evaluate_input("~~&4'b1111").expect("~~&1111");

    assert_eq!(nested_reductions.output, "1'b1");
    assert_eq!(not_chains_into_reduction.output, "1'b1");
}

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
    assert_eq!(
        all_signed.output,
        "32'sb11111111111111111111111111111100"
    );
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
    match add_then_shift_expr {
        Expr::Binary {
            op: BinaryOp::LogicalShiftLeft,
            lhs,
            ..
        } => assert!(matches!(*lhs, Expr::Binary { op: BinaryOp::Add, .. })),
        other => panic!("expected top-level <<, got {other:?}"),
    }
    let add_then_shift = evaluate_input("4'd1 + 4'd2 << 4'd1").expect("eval");
    assert_eq!(add_then_shift.output, "4'd6");

    let shift_then_relational_expr =
        parse_expression("4'd2 << 4'd1 < 4'd5").expect("parse");
    match shift_then_relational_expr {
        Expr::Binary {
            op: BinaryOp::LessThan,
            lhs,
            ..
        } => assert!(matches!(
            *lhs,
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
    match expr {
        Expr::Binary {
            op: BinaryOp::LogicalShiftLeft,
            lhs,
            ..
        } => assert!(matches!(
            *lhs,
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

#[test]
fn reduction_binds_tighter_than_power() {
    // LRM Table 5-4: unary reductions are at the unary level, tighter
    // than **. So `&4'b1111 ** 2` parses as `(&4'b1111) ** 2` = 1**2 = 1.
    let expr = parse_expression("&4'b1111 ** 2").expect("parse");
    match expr {
        Expr::Binary {
            op: BinaryOp::Power,
            lhs,
            ..
        } => assert!(matches!(
            *lhs,
            Expr::Unary {
                op: UnaryOp::ReductionAnd,
                ..
            }
        )),
        other => panic!("expected top-level **, got {other:?}"),
    }
    let result = evaluate_input("&4'b1111 ** 2").expect("eval");
    assert_eq!(result.output, "1'b1");
}

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
    assert_eq!(
        all_signed.output,
        "32'sb11111111111111111111111111111000"
    );
}

#[test]
fn conditional_is_right_associative() {
    // `1'b0 ? 1 : 1'b1 ? 2 : 3` parses as `1'b0 ? 1 : (1'b1 ? 2 : 3)`.
    // Cond is false, so the else branch runs, picking 2.
    let expr = parse_expression("1'b0 ? 1 : 1'b1 ? 2 : 3").expect("parse");
    match expr {
        Expr::Conditional { else_expr, .. } => {
            assert!(matches!(*else_expr, Expr::Conditional { .. }));
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
    match expr {
        Expr::Conditional { cond, .. } => {
            assert!(matches!(
                *cond,
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
    assert_eq!(err, "Semantic error: concatenation operand has indefinite width");
}

#[test]
fn concatenation_rejects_arithmetic_with_unsized_operand() {
    // The indefinite-width flag propagates through context-determined
    // arithmetic: `4'd1 + 1` is indefinite because the `1` is unsized.
    let err = evaluate_input("{4'd1 + 1, 4'd2}").expect_err("indefinite");
    assert_eq!(err, "Semantic error: concatenation operand has indefinite width");
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
    assert_eq!(err, "Semantic error: concatenation operand has indefinite width");
}

#[test]
fn concatenation_rejects_conditional_with_unsized_branch() {
    // Conditional width is max(then, else) (LRM 5.1.13), so an unsized
    // branch makes the whole conditional indefinite.
    let err = evaluate_input("{1'b1 ? 1 : 4'd2, 4'd2}").expect_err("indefinite");
    assert_eq!(err, "Semantic error: concatenation operand has indefinite width");
}

#[test]
fn concatenation_rejects_power_with_unsized_lhs() {
    // `**` takes its result width from the LHS only (LRM 5.1.5, same
    // shape as shifts), so an unsized LHS makes the whole expression
    // indefinite even when the RHS is sized.
    let err = evaluate_input("{2 ** 4'd3, 4'd2}").expect_err("indefinite");
    assert_eq!(err, "Semantic error: concatenation operand has indefinite width");
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
    assert_eq!(err, "Semantic error: replication count must be positive in this context");
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
    let solo =
        evaluate_input("{ {0{1'b1}} }").expect_err("solo zero rep in concat");
    let pair = evaluate_input("{ {0{1'b1}}, {0{1'b1}} }")
        .expect_err("two zero reps no positive sibling");
    let nested =
        evaluate_input("{2{ {0{1'b1}} }}").expect_err("outer rep over zero-only inner");
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
    assert_eq!(err, "Semantic error: replication count must be non-negative");
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
    assert_eq!(err, "Semantic error: replication count contains unknown bits");
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

// LRM §5.5 examples: $signed/$unsigned preserve size and bit pattern; only
// the type label changes. `$signed(4'b1100)` flips the unsigned 12 to the
// signed -4; `$unsigned(-4'sd4)` flips the signed -4 to the unsigned 12.
#[test]
fn signed_unsigned_match_lrm_examples() {
    let signed_from_binary =
        evaluate_input("$signed(4'b1100)").expect("$signed should evaluate");
    let unsigned_from_negative_sized =
        evaluate_input("$unsigned(-4'sd4)").expect("$unsigned should evaluate");
    let unsigned_from_unsized_negative =
        evaluate_input("$unsigned(-4)").expect("$unsigned of unsized negative should evaluate");

    assert_eq!(signed_from_binary.output, "4'sb1100");
    assert_eq!(unsigned_from_negative_sized.output, "4'd12");
    // `-4` is 32-bit signed; $unsigned reinterprets the bits, giving 2^32 - 4.
    assert_eq!(unsigned_from_unsized_negative.output, "32'd4294967292");
}

#[test]
fn sign_casts_round_trip() {
    let signed_then_unsigned =
        evaluate_input("$signed($unsigned(-4'sd4))").expect("nested cast should evaluate");
    assert_eq!(signed_then_unsigned.output, "-4'sd4");

    let unsigned_then_signed =
        evaluate_input("$unsigned($signed(4'b1100))").expect("nested cast should evaluate");
    assert_eq!(unsigned_then_signed.output, "4'b1100");
}

#[test]
fn sign_cast_extends_per_propagated_outer_type() {
    // LRM 5.5.2: each operand extends per the *propagated* type, not its
    // own. The cast's signedness contributes to the 5.5.1 "all signed?"
    // check (which sets the propagated type), but the cast result itself
    // still follows the propagated rule at the leaf.
    //
    // Unsigned propagated context → zero-extend even though the cast says
    // "signed": `$signed(4'b1111)` becomes 8'b00001111, not 8'b11111111.
    let unsigned_outer =
        evaluate_input("$signed(4'b1111) + 8'b0").expect("unsigned outer should evaluate");
    assert_eq!(unsigned_outer.output, "8'b00001111");

    // Signed propagated context (both operands signed) → sign-extend; the
    // 4-bit signed -1 becomes 8'sb11111111 = -1. Display follows the
    // leftmost-base rule: the binary cast wins over the decimal `8'sd0`.
    let signed_outer =
        evaluate_input("$signed(4'b1111) + 8'sd0").expect("signed outer should evaluate");
    assert_eq!(signed_outer.output, "8'sb11111111");

    // Mirror case: $unsigned in a signed outer context still zero-extends
    // because the cast forces the propagated type to unsigned per §5.5.1.
    let unsigned_cast_in_signed_outer = evaluate_input("$unsigned(-4'sd4) + 8'sd0")
        .expect("$unsigned in signed outer should evaluate");
    assert_eq!(unsigned_cast_in_signed_outer.output, "8'd12");
}

#[test]
fn sign_cast_preserves_argument_base() {
    let hex = evaluate_input("$signed(4'hf)").expect("hex cast should evaluate");
    let decimal = evaluate_input("$unsigned(4'sd1)").expect("decimal cast should evaluate");

    assert_eq!(hex.output, "4'shf");
    assert_eq!(decimal.output, "4'd1");
}

#[test]
fn sign_cast_propagates_unknown_bits() {
    // Bit pattern carries through unchanged; x/z bits remain x/z in the
    // result regardless of the cast's signedness.
    let signed_x = evaluate_input("$signed(4'b10x1)").expect("eval");
    let unsigned_z = evaluate_input("$unsigned(4'b1z01)").expect("eval");

    assert_eq!(signed_x.output, "4'sb10x1");
    assert_eq!(unsigned_z.output, "4'b1z01");
}

#[test]
fn rejects_unknown_system_function() {
    let err = evaluate_input("$bogus(1)").expect_err("unknown $-function should error");
    assert_eq!(err, "Syntax error: unsupported system function: $bogus");
}

#[test]
fn rejects_sign_cast_missing_parenthesis() {
    let missing_open = evaluate_input("$signed 1").expect_err("missing `(` should error");
    let missing_close = evaluate_input("$signed(1").expect_err("missing `)` should error");

    assert_eq!(missing_open, "Syntax error: expected `(` after $signed");
    assert_eq!(missing_close, "Syntax error: expected `)` after $signed argument");
}

// vcal-specific display-base casts: `$bin` / `$oct` / `$dec` / `$hex` change
// only the `Base` field — width, signedness, and bits pass through unchanged.

#[test]
fn base_casts_change_display_base_in_each_direction() {
    assert_eq!(evaluate_input("$bin(4'hf)").expect("bin").output, "4'b1111");
    assert_eq!(
        evaluate_input("$hex(4'b1010)").expect("hex").output,
        "4'ha"
    );
    assert_eq!(
        evaluate_input("$oct(8'b11110000)").expect("oct").output,
        "8'o360"
    );
    assert_eq!(
        evaluate_input("$dec(4'b1010)").expect("dec").output,
        "4'd10"
    );
}

#[test]
fn base_cast_preserves_width_and_signedness() {
    // Signed input survives the cast: the binary form keeps the `s` flag.
    let signed_bin = evaluate_input("$bin(-4'sd1)").expect("signed bin");
    assert_eq!(signed_bin.output, "4'sb1111");

    // Negative signed-decimal rendering still applies after the cast — the
    // cast preserves signedness, so the dedicated signed-decimal branch fires.
    let signed_dec = evaluate_input("$dec(4'sb1111)").expect("signed dec");
    assert_eq!(signed_dec.output, "-4'sd1");

    // Unsigned input stays unsigned.
    let unsigned_hex = evaluate_input("$hex(4'b1010)").expect("unsigned hex");
    assert_eq!(unsigned_hex.output, "4'ha");
}

#[test]
fn base_cast_propagates_outer_context_width() {
    // `8'h1` is 8 bits; `+ 16'b0` widens the propagated context to 16 bits.
    // The cast result extends per the outer context (zero-extend, since the
    // outer is unsigned), and the leftmost-base rule keeps binary display.
    let widened = evaluate_input("$bin(8'h1) + 16'b0").expect("widened");
    assert_eq!(widened.output, "16'b0000000000000001");
}

#[test]
fn base_casts_chain_and_are_idempotent() {
    assert_eq!(
        evaluate_input("$hex($bin(4'b1111))").expect("chain").output,
        "4'hf"
    );
    assert_eq!(
        evaluate_input("$bin($hex(4'd5))").expect("chain").output,
        "4'b0101"
    );
}

#[test]
fn base_cast_passes_unknown_bits_through() {
    let bin_x = evaluate_input("$bin(4'hX)").expect("x bits");
    let hex_z = evaluate_input("$hex(4'b10z1)").expect("z bits");

    assert_eq!(bin_x.output, "4'bxxxx");
    assert_eq!(hex_z.output, "4'hz");
}

#[test]
fn base_cast_locks_in_width_so_concatenation_accepts_unsized_arg() {
    // `1` is an unsized literal — bare `{1, 4'b0}` would be rejected as
    // indefinite width. Wrapping it in `$bin` locks in the 32-bit width and
    // makes the concatenation legal.
    let concat = evaluate_input("{$bin(1), 4'b0}").expect("concat");
    assert_eq!(
        concat.output,
        "36'b000000000000000000000000000000010000"
    );
}

#[test]
fn rejects_base_cast_on_real() {
    let err = evaluate_input("$bin(1.5)").expect_err("real arg");
    assert_eq!(err, "Semantic error: $bin argument cannot be real");

    let err = evaluate_input("$hex(2.0)").expect_err("real arg");
    assert_eq!(err, "Semantic error: $hex argument cannot be real");
}

#[test]
fn rejects_base_cast_missing_parenthesis() {
    let missing_open = evaluate_input("$bin 1").expect_err("missing `(` should error");
    let missing_close = evaluate_input("$bin(1").expect_err("missing `)` should error");

    assert_eq!(missing_open, "Syntax error: expected `(` after $bin");
    assert_eq!(missing_close, "Syntax error: expected `)` after $bin argument");
}

// LRM §3.5.2 examples — accepted real-number forms.
#[test]
fn parses_real_decimal_forms() {
    assert_eq!(evaluate_input("1.0").expect("1.0").output, "1.0");
    assert_eq!(evaluate_input("1.2").expect("1.2").output, "1.2");
    assert_eq!(evaluate_input("0.1").expect("0.1").output, "0.1");
    assert_eq!(
        evaluate_input("2394.26331").expect("2394.26331").output,
        "2394.26331"
    );
}

#[test]
fn parses_real_scientific_forms() {
    assert_eq!(evaluate_input("1.2E12").expect("1.2E12").output, "1.2e+12");
    assert_eq!(evaluate_input("1.30e-2").expect("1.30e-2").output, "0.013");
    assert_eq!(evaluate_input("0.1e-0").expect("0.1e-0").output, "0.1");
    assert_eq!(evaluate_input("23E10").expect("23E10").output, "2.3e+11");
    assert_eq!(evaluate_input("29E-2").expect("29E-2").output, "0.29");
    assert_eq!(
        evaluate_input("236.123_763_e-12")
            .expect("underscored real")
            .output,
        "2.36123763e-10"
    );
}

// LRM §3.5.2 invalid forms — must have a digit on each side of `.`.
#[test]
fn rejects_invalid_real_forms() {
    evaluate_input(".12").expect_err("missing leading digit");
    evaluate_input("9.").expect_err("missing trailing digit before EOF");
    evaluate_input("4.E3").expect_err("missing trailing digit before exponent");
    evaluate_input(".2e-7").expect_err("missing leading digit before fraction");
}

// LRM §5.1.5 / Table 5-2: `+`, `-`, `*`, `/`, `**` legal on reals.
#[test]
fn evaluates_real_arithmetic() {
    assert_eq!(evaluate_input("1.0 + 2.0").expect("add").output, "3.0");
    assert_eq!(evaluate_input("1.0 + 2").expect("mixed add").output, "3.0");
    assert_eq!(
        evaluate_input("3.0 - 1.5").expect("subtract").output,
        "1.5"
    );
    assert_eq!(
        evaluate_input("2.0 * 3.0").expect("multiply").output,
        "6.0"
    );
    assert_eq!(
        evaluate_input("5.0 / 2.0").expect("real divide").output,
        "2.5"
    );
    assert_eq!(evaluate_input("-1.5").expect("unary minus").output, "-1.5");
    assert_eq!(evaluate_input("+1.5").expect("unary plus").output, "1.5");
}

// LRM §5.1.5 / Table 5-8: real square root, integer-divided exponent.
#[test]
fn evaluates_real_power() {
    assert_eq!(
        evaluate_input("2.0 ** -1").expect("real reciprocal").output,
        "0.5"
    );
    assert_eq!(
        evaluate_input("9.0 ** (1/2)")
            .expect("integer-divided exponent")
            .output,
        "1.0"
    );
    assert_eq!(
        evaluate_input("9 ** 0.5").expect("real square root").output,
        "3.0"
    );
    assert_eq!(
        evaluate_input("-3.0 ** 2.0")
            .expect("negative base, integral exponent")
            .output,
        "9.0"
    );
}

// LRM §5.1.5: `0.0 ** ≤0` and `negative ** non-integral` are unspecified.
// vcal inherits whatever f64::powf returns (NaN / ±∞ / 1.0).
#[test]
fn unspecified_real_power_corners_use_ieee_754() {
    assert_eq!(
        evaluate_input("0.0 ** 0.0").expect("0**0").output,
        "1.0"
    );
    assert_eq!(
        evaluate_input("0.0 ** -1.0").expect("0**neg").output,
        "inf"
    );
    assert_eq!(
        evaluate_input("(-2.0) ** 0.5")
            .expect("neg ** non-integral")
            .output,
        "NaN"
    );
}

// LRM §5.1.7 / §5.1.8: relational/equality with a real operand promotes
// the integer side to real, comparison runs in f64, result is 1-bit.
#[test]
fn evaluates_real_relational_and_equality() {
    assert_eq!(
        evaluate_input("1.5 < 2").expect("real < int").output,
        "1'b1"
    );
    assert_eq!(
        evaluate_input("2.0 > 1.5").expect("real > real").output,
        "1'b1"
    );
    assert_eq!(
        evaluate_input("1.0 == 1.0").expect("real ==").output,
        "1'b1"
    );
    assert_eq!(
        evaluate_input("1.0 != 2.0").expect("real !=").output,
        "1'b1"
    );
    assert_eq!(
        evaluate_input("1.0 == 1").expect("mixed ==").output,
        "1'b1"
    );
}

// LRM §5.1.9: !, &&, || on reals reduce via the 0=false / non-zero=true
// rule; result is always 1-bit integer.
#[test]
fn evaluates_real_logical() {
    assert_eq!(evaluate_input("!1.0").expect("!1.0").output, "1'b0");
    assert_eq!(evaluate_input("!0.0").expect("!0.0").output, "1'b1");
    assert_eq!(
        evaluate_input("1.0 && 2.0").expect("&& real").output,
        "1'b1"
    );
    assert_eq!(
        evaluate_input("0.0 || 1.0").expect("|| real").output,
        "1'b1"
    );
    assert_eq!(
        evaluate_input("1.0 && 0").expect("mixed &&").output,
        "1'b0"
    );
}

// LRM §5.1.13: ?: branches promote to real if either branch is real;
// result type is real even when only one branch is.
#[test]
fn conditional_promotes_branches_to_real() {
    assert_eq!(
        evaluate_input("1 ? 1.0 : 2").expect("then real").output,
        "1.0"
    );
    assert_eq!(
        evaluate_input("0 ? 1 : 2.0").expect("else real").output,
        "2.0"
    );
    assert_eq!(
        evaluate_input("1.0 ? 1 : 2").expect("real cond, int branches").output,
        "32'sd1"
    );
}

// LRM Table 5-3: every operator listed there must reject a real operand.
#[test]
fn rejects_modulus_on_real() {
    let err = evaluate_input("1.0 % 2.0").expect_err("modulus on real");
    assert_eq!(err, "Semantic error: operator % not allowed on real operand");
}

#[test]
fn rejects_case_equality_on_real() {
    assert_eq!(
        evaluate_input("1.0 === 1.0").expect_err("=== on real"),
        "Semantic error: operator === not allowed on real operand"
    );
    assert_eq!(
        evaluate_input("1.0 !== 1.0").expect_err("!== on real"),
        "Semantic error: operator !== not allowed on real operand"
    );
}

#[test]
fn rejects_bitwise_on_real() {
    assert_eq!(
        evaluate_input("1.0 & 2.0").expect_err("& on real"),
        "Semantic error: operator & not allowed on real operand"
    );
    assert_eq!(
        evaluate_input("1.0 | 2.0").expect_err("| on real"),
        "Semantic error: operator | not allowed on real operand"
    );
    assert_eq!(
        evaluate_input("~1.0").expect_err("~ on real"),
        "Semantic error: operator ~ not allowed on real operand"
    );
}

#[test]
fn rejects_reductions_on_real() {
    assert_eq!(
        evaluate_input("&1.0").expect_err("& reduction on real"),
        "Semantic error: operator & not allowed on real operand"
    );
    assert_eq!(
        evaluate_input("~|1.0").expect_err("~| reduction on real"),
        "Semantic error: operator ~| not allowed on real operand"
    );
}

#[test]
fn rejects_shifts_on_real() {
    assert_eq!(
        evaluate_input("1.0 << 1").expect_err("<< on real"),
        "Semantic error: operator << not allowed on real operand"
    );
    assert_eq!(
        evaluate_input("1.0 >>> 1").expect_err(">>> on real"),
        "Semantic error: operator >>> not allowed on real operand"
    );
}

#[test]
fn rejects_concatenation_with_real() {
    assert_eq!(
        evaluate_input("{1.0, 2.0}").expect_err("concat with real"),
        "Semantic error: concatenation operand cannot be real"
    );
    assert_eq!(
        evaluate_input("{2{1.0}}").expect_err("replication with real"),
        "Semantic error: replication operand cannot be real"
    );
}

#[test]
fn rejects_sign_cast_on_real() {
    assert_eq!(
        evaluate_input("$signed(1.0)").expect_err("$signed real"),
        "Semantic error: $signed argument cannot be real"
    );
    assert_eq!(
        evaluate_input("$unsigned(2.0)").expect_err("$unsigned real"),
        "Semantic error: $unsigned argument cannot be real"
    );
}

// Real arithmetic propagates through chained operators — once any
// sub-expression is real, the result type stays real until a
// 1-bit-result operator (relational/equality/logical) collapses it.
#[test]
fn real_propagates_through_chained_arithmetic() {
    assert_eq!(
        evaluate_input("1.0 + 2 * 3").expect("real propagates").output,
        "7.0"
    );
    assert_eq!(
        evaluate_input("(1 + 2) * 1.0")
            .expect("integer subexpr lifted to real")
            .output,
        "3.0"
    );
    assert_eq!(
        evaluate_input("1.0 + (1 < 2)")
            .expect("relational result lifted to real")
            .output,
        "2.0"
    );
}

// LRM A.8.7 `unsigned_number ::= decimal_digit { _ | decimal_digit }`: the
// exponent's digit run must START with a decimal digit. Regression test —
// before the consume_exponent fix, `5.0e_3` was silently accepted as
// `5000.0` because the dot-path didn't enforce digit-leading on the
// exponent.
#[test]
fn rejects_underscore_leading_exponent() {
    assert_eq!(
        evaluate_input("5.0e_3").expect_err("underscore-leading exponent"),
        "Syntax error: missing exponent digits in real literal"
    );
    assert_eq!(
        evaluate_input("5.0e+_3").expect_err("underscore-leading after sign"),
        "Syntax error: missing exponent digits in real literal"
    );
    assert_eq!(
        evaluate_input("1e_3").expect_err("bare-exponent underscore-leading"),
        "Syntax error: missing exponent digits in real literal"
    );
    assert_eq!(
        evaluate_input("1e").expect_err("bare exponent without digits"),
        "Syntax error: missing exponent digits in real literal"
    );
}

// LRM §3.5.3: "Individual bits that are x or z in the net or the
// variable shall be treated as zero upon conversion." So promoting an
// integer with x/z bits to real for mixed-type arithmetic substitutes 0
// for each unknown bit and keeps the rest.
#[test]
fn xz_integer_promotes_to_zero_bits_in_real_context() {
    assert_eq!(
        evaluate_input("1'bx + 1.0").expect("x + real").output,
        "1.0"
    );
    assert_eq!(
        evaluate_input("1'bz * 2.0").expect("z * real").output,
        "0.0"
    );
    // 4'b01x0 with x→0 is 4'b0100 = 4, so 4 + 1.0 = 5.0.
    assert_eq!(
        evaluate_input("4'b01x0 + 1.0")
            .expect("partial-x + real")
            .output,
        "5.0"
    );
}

// IEEE 754: every comparison involving NaN is "unordered" — `==` is false,
// `!=` is true, all four ordered comparisons (<, <=, >, >=) are false.
// vcal inherits these semantics directly from f64 ops.
#[test]
fn nan_comparisons_follow_ieee_754() {
    let nan = "(0.0 / 0.0)";
    assert_eq!(
        evaluate_input(&format!("{nan} == {nan}"))
            .expect("NaN == NaN")
            .output,
        "1'b0"
    );
    assert_eq!(
        evaluate_input(&format!("{nan} != {nan}"))
            .expect("NaN != NaN")
            .output,
        "1'b1"
    );
    assert_eq!(
        evaluate_input(&format!("{nan} < 1.0"))
            .expect("NaN < 1")
            .output,
        "1'b0"
    );
    assert_eq!(
        evaluate_input(&format!("{nan} >= 1.0"))
            .expect("NaN >= 1")
            .output,
        "1'b0"
    );
    // NaN reduces to logical x via README's "real NaN → x" rule, so
    // NaN || 1 collapses to 1 and NaN && 0 collapses to 0 (an x/1 → 1
    // and x/0 → 0 by the §5.1.9 truth table).
    assert_eq!(
        evaluate_input(&format!("{nan} || 1.0"))
            .expect("NaN || 1")
            .output,
        "1'b1"
    );
    assert_eq!(
        evaluate_input(&format!("{nan} && 0.0"))
            .expect("NaN && 0")
            .output,
        "1'b0"
    );
}

// LRM Table 5-3: replication count is integer-only. Symmetric coverage to
// the existing `{2{1.0}}` (real operand) test.
#[test]
fn rejects_real_replication_count() {
    assert_eq!(
        evaluate_input("{2.0{1'b1}}").expect_err("real replication count"),
        "Semantic error: replication count cannot be real"
    );
    assert_eq!(
        evaluate_input("{(1.5 + 0.5){1'b1}}")
            .expect_err("real-typed expression as count"),
        "Semantic error: replication count cannot be real"
    );
}

// LRM §5.1.7 / §3.5.3: an integer side of a mixed-type expression is
// converted to real per its declared signedness. `4'sb1111` is signed -1,
// so it must become -1.0 (not 15.0) when promoted.
#[test]
fn signed_integer_promotes_to_real_per_declared_signedness() {
    assert_eq!(
        evaluate_input("4'sb1111 + 1.0")
            .expect("signed -1 + 1.0")
            .output,
        "0.0"
    );
    assert_eq!(
        evaluate_input("4'b1111 + 1.0")
            .expect("unsigned 15 + 1.0")
            .output,
        "16.0"
    );
    assert_eq!(
        evaluate_input("4'sb1111 == -1.0")
            .expect("signed -1 == -1.0")
            .output,
        "1'b1"
    );
    assert_eq!(
        evaluate_input("4'b1111 == -1.0")
            .expect("unsigned 15 == -1.0")
            .output,
        "1'b0"
    );
    assert_eq!(
        evaluate_input("4'sb1111 < 0.0")
            .expect("signed -1 < 0.0")
            .output,
        "1'b1"
    );
}

// README "Real numbers": display switches to scientific outside
// [1e-4, 1e10). The endpoints are load-bearing — 1e-4 stays fixed-point,
// 1e10 switches to scientific, and just-below-1e10 stays fixed-point.
#[test]
fn real_format_window_boundaries() {
    // 1e-4 is the inclusive lower bound — fixed-point.
    assert_eq!(evaluate_input("1.0e-4").expect("1e-4").output, "0.0001");
    // 1e10 is the exclusive upper bound — scientific.
    assert_eq!(evaluate_input("1.0e10").expect("1e10").output, "1.0e+10");
    // Just below 1e10 — still fixed-point.
    assert_eq!(
        evaluate_input("9999999999.0")
            .expect("just below 1e10")
            .output,
        "9999999999.0"
    );
    // Just below 1e-4 — scientific.
    assert_eq!(
        evaluate_input("9.999e-5").expect("just below 1e-4").output,
        "9.999e-5"
    );
}

// LRM A.8.7: every digit-run grammar (unsigned_number,
// non_zero_unsigned_number, *_value) starts with a digit (or x/z for the
// based forms), never with an underscore. Before the fix, strip_underscores
// ran before validation, so `8'b_101` silently parsed as binary 101 and
// `'d_x` as a decimal x-literal. (The unsized-decimal and size cases —
// `_1`, `_8'd5` — now lex as identifiers under LRM 3.7.1 and surface
// through the identifier / parser paths instead.)
#[test]
fn rejects_leading_underscore_in_number_literals() {
    assert_eq!(
        evaluate_input("8'b_101").expect_err("binary value"),
        "Syntax error: number cannot start with underscore: _101"
    );
    assert_eq!(
        evaluate_input("8'h_a").expect_err("hex value"),
        "Syntax error: number cannot start with underscore: _a"
    );
    assert_eq!(
        evaluate_input("8'd_5").expect_err("decimal value"),
        "Syntax error: number cannot start with underscore: _5"
    );
    assert_eq!(
        evaluate_input("'d_x").expect_err("decimal x form"),
        "Syntax error: number cannot start with underscore: _x"
    );
}

// Underscores remain legal as separators inside a digit run — only the
// leading position is forbidden. Guard against the regression-test fix
// over-rejecting valid forms.
#[test]
fn accepts_underscore_separators_inside_digit_run() {
    assert_eq!(
        evaluate_input("1_2").expect("decimal separator").output,
        "32'sd12"
    );
    assert_eq!(
        evaluate_input("1_000_000")
            .expect("multiple separators")
            .output,
        "32'sd1000000"
    );
    assert_eq!(
        evaluate_input("8'b1010_0101")
            .expect("binary separator")
            .output,
        "8'b10100101"
    );
    assert_eq!(
        evaluate_input("16'hdead_beef")
            .expect("hex separator")
            .output,
        "16'hbeef"
    );
}

// LRM 17.8: $rtoi truncates toward zero (NOT round). The example values
// 123.45 → 123 and -22.7 → -22 come straight from the LRM clause. Result is
// 32-bit signed (Verilog's `integer` type), displayed in decimal.
#[test]
fn rtoi_truncates_toward_zero() {
    assert_eq!(
        evaluate_input("$rtoi(123.45)").expect("$rtoi positive").output,
        "32'sd123"
    );
    assert_eq!(
        evaluate_input("$rtoi(-22.7)").expect("$rtoi negative").output,
        "-32'sd22"
    );
    // Truncation, not rounding: 0.9 → 0, -0.9 → 0.
    assert_eq!(evaluate_input("$rtoi(0.9)").expect("0.9").output, "32'sd0");
    assert_eq!(
        evaluate_input("$rtoi(-0.9)").expect("-0.9").output,
        "32'sd0"
    );
    assert_eq!(
        evaluate_input("$rtoi(1.999)").expect("1.999").output,
        "32'sd1"
    );
    assert_eq!(
        evaluate_input("$rtoi(-1.999)").expect("-1.999").output,
        "-32'sd1"
    );
}

// LRM is silent on NaN / ±∞ in $rtoi. vcal returns 32 bits of x to surface
// "no defined integer image" rather than silently mapping to zero.
#[test]
fn rtoi_nan_and_infinity_yield_x() {
    assert_eq!(
        evaluate_input("$rtoi(0.0 / 0.0)").expect("$rtoi NaN").output,
        "32'sdx"
    );
    assert_eq!(
        evaluate_input("$rtoi(1.0 / 0.0)")
            .expect("$rtoi +inf")
            .output,
        "32'sdx"
    );
    assert_eq!(
        evaluate_input("$rtoi(-1.0 / 0.0)")
            .expect("$rtoi -inf")
            .output,
        "32'sdx"
    );
}

// LRM §3.5.3 carries through $rtoi: an integer operand auto-promotes to
// real (x/z bits → 0), then truncation gives the same magnitude. Lets users
// write `$rtoi(integer_expr)` without an extra $itor wrapper.
#[test]
fn rtoi_accepts_integer_operand() {
    assert_eq!(
        evaluate_input("$rtoi(1.0 + 1)")
            .expect("$rtoi mixed promotes to real then truncates")
            .output,
        "32'sd2"
    );
    // Pure integer argument: truncates to itself, just retyped to 32-bit
    // signed integer.
    assert_eq!(
        evaluate_input("$rtoi(4'b01x0 + 1.0)")
            .expect("$rtoi with x→0 promotion")
            .output,
        "32'sd5"
    );
}

// LRM 17.8: $itor converts integer to real. §3.5.3 specifies the x/z → 0
// rule for the underlying integer-to-real conversion, so $itor surfaces
// known bits and ignores unknowns.
#[test]
fn itor_converts_integer_to_real() {
    assert_eq!(
        evaluate_input("$itor(5)").expect("$itor positive").output,
        "5.0"
    );
    assert_eq!(
        evaluate_input("$itor(-5)").expect("$itor negative").output,
        "-5.0"
    );
    assert_eq!(evaluate_input("$itor(0)").expect("$itor zero").output, "0.0");
    // x/z bits become 0 per §3.5.3, so 4'b01x0 → 0100 → 4 → 4.0.
    assert_eq!(
        evaluate_input("$itor(4'b01x0)")
            .expect("$itor with x bits")
            .output,
        "4.0"
    );
    assert_eq!(
        evaluate_input("$itor(1'bx)")
            .expect("$itor pure x → 0")
            .output,
        "0.0"
    );
}

// LRM 17.8 types the $itor argument as `int_val`. Simulators diverge on
// real input (iverilog rounds via §3.5.3 to 1.0, vcs/xsim pass 1.1
// through), and the result type is already real — a real argument is
// either non-portable or pointless, so the validator rejects it.
#[test]
fn itor_rejects_real_argument() {
    assert_eq!(
        evaluate_input("$itor(1.1)").expect_err("$itor real literal"),
        "Semantic error: $itor argument cannot be real"
    );
    assert_eq!(
        evaluate_input("$itor(-2.6)").expect_err("$itor negative real"),
        "Semantic error: $itor argument cannot be real"
    );
    // Real-result expressions (NaN/±∞ from real arithmetic) take the same
    // path — there's nothing real-specific about the rejection.
    assert_eq!(
        evaluate_input("$itor(0.0 / 0.0)").expect_err("$itor NaN"),
        "Semantic error: $itor argument cannot be real"
    );
}

// $itor on an integer-typed operand must surface ±∞ when the magnitude
// exceeds f64 range — that's what `BigInt::to_f64` produces. The earlier
// implementation routed every $itor through real→int→real, which
// collapsed those infinities back to 0.0 (since ±∞ has no integer image)
// and silently discarded the value.
#[test]
fn itor_oversized_integer_saturates_to_infinity() {
    let huge_pos = format!("$itor(1{})", "0".repeat(309));
    assert_eq!(
        evaluate_input(&huge_pos)
            .expect("$itor 10**309")
            .output,
        "inf"
    );
    let huge_neg = format!("$itor(-1{})", "0".repeat(309));
    assert_eq!(
        evaluate_input(&huge_neg)
            .expect("$itor -10**309")
            .output,
        "-inf"
    );
    // 10**308 is still within f64 range — the boundary stays representable.
    let in_range = format!("$itor(1{})", "0".repeat(308));
    assert_eq!(
        evaluate_input(&in_range)
            .expect("$itor 10**308")
            .output,
        "1.0e+308"
    );
}

// LRM 17.8: $realtobits exposes the IEEE 754 binary64 bit pattern as a
// 64-bit unsigned vector. The reference values come from the standard
// encodings — 1.0 = 0x3FF0...0, +0.0 = all zeros, -1.0 = 0xBFF0...0.
#[test]
fn realtobits_returns_ieee754_pattern() {
    assert_eq!(
        evaluate_input("$realtobits(1.0)").expect("1.0").output,
        "64'h3ff0000000000000"
    );
    assert_eq!(
        evaluate_input("$realtobits(0.0)").expect("0.0").output,
        "64'h0000000000000000"
    );
    assert_eq!(
        evaluate_input("$realtobits(-1.0)").expect("-1.0").output,
        "64'hbff0000000000000"
    );
    assert_eq!(
        evaluate_input("$realtobits(2.0)").expect("2.0").output,
        "64'h4000000000000000"
    );
}

// $realtobits accepts an integer operand, promoting via §3.5.3 first.
// Useful so users can inspect the bit pattern of an integer-derived real
// without an explicit $itor.
#[test]
fn realtobits_accepts_integer_operand() {
    assert_eq!(
        evaluate_input("$realtobits(1 + 0.0)")
            .expect("integer auto-promotes")
            .output,
        "64'h3ff0000000000000"
    );
}

// LRM 17.8: $bitstoreal is the inverse of $realtobits — same 64-bit
// pattern, decoded as IEEE 754 binary64.
#[test]
fn bitstoreal_decodes_ieee754_pattern() {
    assert_eq!(
        evaluate_input("$bitstoreal(64'h3ff0000000000000)")
            .expect("1.0 pattern")
            .output,
        "1.0"
    );
    assert_eq!(
        evaluate_input("$bitstoreal(64'h0000000000000000)")
            .expect("zero pattern")
            .output,
        "0.0"
    );
    assert_eq!(
        evaluate_input("$bitstoreal(64'hbff0000000000000)")
            .expect("-1.0 pattern")
            .output,
        "-1.0"
    );
}

// $realtobits → $bitstoreal round-trips every finite real (and ±0.0)
// because IEEE 754 binary64 encoding is one-to-one on those values.
#[test]
fn realtobits_bitstoreal_round_trip() {
    assert_eq!(
        evaluate_input("$bitstoreal($realtobits(3.14))")
            .expect("round-trip pi")
            .output,
        "3.14"
    );
    assert_eq!(
        evaluate_input("$bitstoreal($realtobits(-2.5))")
            .expect("round-trip -2.5")
            .output,
        "-2.5"
    );
}

// Non-finite IEEE 754 patterns decode to the f64 special values. The
// textual rendering collapses every NaN to "NaN" and every infinity to
// "inf"/"-inf" (per format_real in value.rs), so multiple distinct NaN
// payloads share the same display — payload preservation is checked by
// the round-trip test below.
#[test]
fn bitstoreal_decodes_non_finite_patterns() {
    assert_eq!(
        evaluate_input("$bitstoreal(64'hFFFFFFFFFFFFFFFF)")
            .expect("all-1s NaN")
            .output,
        "NaN"
    );
    assert_eq!(
        evaluate_input("$bitstoreal(64'h7FF8000000000000)")
            .expect("quiet NaN")
            .output,
        "NaN"
    );
    assert_eq!(
        evaluate_input("$bitstoreal(64'h7FF0000000000001)")
            .expect("signaling NaN")
            .output,
        "NaN"
    );
    assert_eq!(
        evaluate_input("$bitstoreal(64'h7FF0000000000000)")
            .expect("+inf")
            .output,
        "inf"
    );
    assert_eq!(
        evaluate_input("$bitstoreal(64'hFFF0000000000000)")
            .expect("-inf")
            .output,
        "-inf"
    );
}

// Round-trip preserves the full 64-bit payload even for NaN, matching
// iverilog. The f64 carrying the value flows through evaluate_expr_as_real
// as a pass-through (no arithmetic, no FPU register touch), so the
// from_bits/to_bits pair stays a transparent transmute. Locks in the
// contract that bits_value_to_real may not canonicalize NaN.
#[test]
fn realtobits_bitstoreal_round_trip_preserves_nan_payload() {
    assert_eq!(
        evaluate_input("$realtobits($bitstoreal(64'hFFFFFFFFFFFFFFFF))")
            .expect("all-1s NaN round-trip")
            .output,
        "64'hffffffffffffffff"
    );
    assert_eq!(
        evaluate_input("$realtobits($bitstoreal(64'h7FF8000000000000))")
            .expect("quiet NaN round-trip")
            .output,
        "64'h7ff8000000000000"
    );
    assert_eq!(
        evaluate_input("$realtobits($bitstoreal(64'h7FF0000000000001))")
            .expect("signaling NaN round-trip")
            .output,
        "64'h7ff0000000000001"
    );
    assert_eq!(
        evaluate_input("$realtobits($bitstoreal(64'h7FF0000000000000))")
            .expect("+inf round-trip")
            .output,
        "64'h7ff0000000000000"
    );
    assert_eq!(
        evaluate_input("$realtobits($bitstoreal(64'hFFF0000000000000))")
            .expect("-inf round-trip")
            .output,
        "64'hfff0000000000000"
    );
}

// $bitstoreal on a real argument has no defined bit-cast meaning (the
// argument is already a real, not a 64-bit pattern). Reject explicitly.
#[test]
fn bitstoreal_rejects_real_argument() {
    assert_eq!(
        evaluate_input("$bitstoreal(1.0)").expect_err("$bitstoreal real"),
        "Semantic error: $bitstoreal argument cannot be real"
    );
}

// LRM 17.8: $bitstoreal expects a 64-bit pattern. Anything narrower
// would silently zero-extend and anything wider would silently truncate;
// both are likely user mistakes, so we reject them up front. The width
// check uses the argument's self-determined width, so a 32-bit unsized
// literal or a width-mixing expression is rejected even if its bit
// pattern would otherwise round-trip.
#[test]
fn bitstoreal_rejects_non_64_bit_argument() {
    assert_eq!(
        evaluate_input("$bitstoreal(1)").expect_err("32-bit unsized"),
        "Semantic error: $bitstoreal argument must be 64 bits wide, got 32"
    );
    assert_eq!(
        evaluate_input("$bitstoreal(1'b0)").expect_err("1-bit"),
        "Semantic error: $bitstoreal argument must be 64 bits wide, got 1"
    );
    assert_eq!(
        evaluate_input("$bitstoreal(63'h0)").expect_err("63-bit"),
        "Semantic error: $bitstoreal argument must be 64 bits wide, got 63"
    );
    assert_eq!(
        evaluate_input("$bitstoreal(65'h0)").expect_err("65-bit"),
        "Semantic error: $bitstoreal argument must be 64 bits wide, got 65"
    );
}

// Width is taken from the argument's self-determined meta, so an
// expression that arithmetically produces a 64-bit value still needs
// each operand to drive the unified width to 64. `64'h0 + 64'h0` is
// 64-bit, so it's accepted; `64'h0 + 32'h0` would unify to 64 too
// (max), so it's also accepted.
#[test]
fn bitstoreal_accepts_64_bit_expressions() {
    assert_eq!(
        evaluate_input("$bitstoreal(64'h3ff0000000000000 | 64'h0)")
            .expect("bitwise expression sized 64")
            .output,
        "1.0"
    );
    assert_eq!(
        evaluate_input("$bitstoreal($signed(64'h3ff0000000000000))")
            .expect("signed cast preserves 64-bit width")
            .output,
        "1.0"
    );
}

// LRM §3.5.3 carries through $bitstoreal: x/z bits in the operand convert
// to 0 in the bit pattern, so e.g. an all-x 64-bit operand decodes as the
// all-zeros pattern, which IEEE 754 reads as +0.0.
#[test]
fn bitstoreal_treats_xz_as_zero_bits() {
    assert_eq!(
        evaluate_input("$bitstoreal(64'bx)").expect("all-x → +0.0").output,
        "0.0"
    );
}

// Outer-context widening: $rtoi's 32-bit signed result and $realtobits's
// 64-bit unsigned result extend per propagated context type at the leaf,
// matching the §5.5.2 rule already used by $signed/$unsigned.
#[test]
fn real_conversions_widen_per_outer_context() {
    // Signed outer context → sign-extend the 32-bit signed -1 to 64 bits.
    assert_eq!(
        evaluate_input("$rtoi(-1.0) + 64'sd0")
            .expect("$rtoi widens")
            .output,
        "-64'sd1"
    );
    // Unsigned outer context wider than 64 bits → zero-extend the bit
    // pattern (the high bits stay 0).
    assert_eq!(
        evaluate_input("$realtobits(1.0) + 65'h0")
            .expect("$realtobits widens")
            .output,
        "65'h03ff0000000000000"
    );
}

// Parentheses are required after each new $-function, mirroring
// $signed/$unsigned diagnostics.
#[test]
fn rejects_real_conversion_missing_parenthesis() {
    assert_eq!(
        evaluate_input("$rtoi 1.0").expect_err("missing `(`"),
        "Syntax error: expected `(` after $rtoi"
    );
    assert_eq!(
        evaluate_input("$itor(1").expect_err("missing `)`"),
        "Syntax error: expected `)` after $itor argument"
    );
}

// LRM 17.11: $clog2 returns the ceiling of log base 2 of an unsigned
// argument; $clog2(0) is defined to be 0.
#[test]
fn clog2_returns_ceiling_log2() {
    assert_eq!(
        evaluate_input("$clog2(0)").expect("$clog2(0)").output,
        "32'sd0"
    );
    assert_eq!(
        evaluate_input("$clog2(1)").expect("$clog2(1)").output,
        "32'sd0"
    );
    assert_eq!(
        evaluate_input("$clog2(2)").expect("$clog2(2)").output,
        "32'sd1"
    );
    assert_eq!(
        evaluate_input("$clog2(3)").expect("$clog2(3)").output,
        "32'sd2"
    );
    assert_eq!(
        evaluate_input("$clog2(4)").expect("$clog2(4)").output,
        "32'sd2"
    );
    assert_eq!(
        evaluate_input("$clog2(5)").expect("$clog2(5)").output,
        "32'sd3"
    );
    assert_eq!(
        evaluate_input("$clog2(8)").expect("$clog2(8)").output,
        "32'sd3"
    );
}

// LRM 17.11: "the argument shall be treated as an unsigned value", so the
// operand's natural width drives the result — a 64-bit all-ones is 2^64-1
// and clog2 = 64, not 32.
#[test]
fn clog2_uses_argument_natural_width_unsigned() {
    assert_eq!(
        evaluate_input("$clog2(64'hFFFFFFFFFFFFFFFF)")
            .expect("$clog2 wide")
            .output,
        "32'sd64"
    );
    // -1 is 32'shFFFF_FFFF — as unsigned, 2^32-1. clog2 = 32.
    assert_eq!(
        evaluate_input("$clog2(-1)").expect("$clog2(-1)").output,
        "32'sd32"
    );
    // 8'sb1000_0000 is signed -128 — as unsigned 8-bit, 128. clog2 = 7.
    assert_eq!(
        evaluate_input("$clog2(8'sb10000000)")
            .expect("$clog2 signed-msb")
            .output,
        "32'sd7"
    );
}

// LRM is silent on x/z bits in $clog2. vcal surfaces 32 bits of x to mark
// "no defined image", mirroring the $rtoi NaN/±∞ rule.
#[test]
fn clog2_xz_bits_collapse_to_x_result() {
    assert_eq!(
        evaluate_input("$clog2(4'b01x0)").expect("$clog2 x").output,
        "32'sdx"
    );
    assert_eq!(
        evaluate_input("$clog2(4'b01z0)").expect("$clog2 z").output,
        "32'sdx"
    );
    assert_eq!(
        evaluate_input("$clog2(1'bx)").expect("$clog2 pure x").output,
        "32'sdx"
    );
}

// LRM 17.11.1: $clog2's argument "can be an integer or an arbitrary sized
// vector value" — real is not listed. The validator rejects it for the
// same reason $itor does: an implicit real→integer round would silently
// pick one vendor's interpretation.
#[test]
fn clog2_rejects_real_argument() {
    assert_eq!(
        evaluate_input("$clog2(8.0)").expect_err("$clog2 real literal"),
        "Semantic error: $clog2 argument cannot be real"
    );
    assert_eq!(
        evaluate_input("$clog2(-2.5)").expect_err("$clog2 negative real"),
        "Semantic error: $clog2 argument cannot be real"
    );
    assert_eq!(
        evaluate_input("$clog2(0.0 / 0.0)").expect_err("$clog2 NaN"),
        "Semantic error: $clog2 argument cannot be real"
    );
}

// $clog2's 32-bit signed result widens under outer arithmetic context
// the same way $rtoi does.
#[test]
fn clog2_widens_in_outer_context() {
    assert_eq!(
        evaluate_input("$clog2(8) + 64'sd0")
            .expect("$clog2 widens")
            .output,
        "64'sd3"
    );
}

// LRM 17.11: real-typed math functions follow the C standard library;
// Rust's f64::* methods wrap libm so semantics match by construction.
#[test]
fn real_math_basic_results() {
    assert_eq!(
        evaluate_input("$sqrt(4.0)").expect("$sqrt").output,
        "2.0"
    );
    assert_eq!(
        evaluate_input("$ln(1.0)").expect("$ln(1)").output,
        "0.0"
    );
    assert_eq!(
        evaluate_input("$log10(100.0)").expect("$log10").output,
        "2.0"
    );
    assert_eq!(
        evaluate_input("$exp(0.0)").expect("$exp(0)").output,
        "1.0"
    );
    assert_eq!(
        evaluate_input("$floor(2.7)").expect("$floor").output,
        "2.0"
    );
    assert_eq!(
        evaluate_input("$ceil(2.3)").expect("$ceil").output,
        "3.0"
    );
    assert_eq!(
        evaluate_input("$sin(0.0)").expect("$sin(0)").output,
        "0.0"
    );
    assert_eq!(
        evaluate_input("$cos(0.0)").expect("$cos(0)").output,
        "1.0"
    );
    assert_eq!(
        evaluate_input("$tan(0.0)").expect("$tan(0)").output,
        "0.0"
    );
    assert_eq!(
        evaluate_input("$asin(0.0)").expect("$asin(0)").output,
        "0.0"
    );
    assert_eq!(
        evaluate_input("$atan(0.0)").expect("$atan(0)").output,
        "0.0"
    );
    assert_eq!(
        evaluate_input("$sinh(0.0)").expect("$sinh(0)").output,
        "0.0"
    );
    assert_eq!(
        evaluate_input("$cosh(0.0)").expect("$cosh(0)").output,
        "1.0"
    );
    assert_eq!(
        evaluate_input("$tanh(0.0)").expect("$tanh(0)").output,
        "0.0"
    );
    assert_eq!(
        evaluate_input("$asinh(0.0)").expect("$asinh(0)").output,
        "0.0"
    );
    assert_eq!(
        evaluate_input("$acosh(1.0)").expect("$acosh(1)").output,
        "0.0"
    );
    assert_eq!(
        evaluate_input("$atanh(0.0)").expect("$atanh(0)").output,
        "0.0"
    );
}

// Integer arguments auto-promote to real via §3.5.3 (x/z → 0), so users
// can write `$sqrt(4)` without an explicit `$itor` wrapper.
#[test]
fn real_math_accepts_integer_argument() {
    assert_eq!(
        evaluate_input("$sqrt(4)").expect("$sqrt int").output,
        "2.0"
    );
    // 4'b01x0 → x/z→0 → 0100 → 4 → sqrt = 2.0
    assert_eq!(
        evaluate_input("$sqrt(4'b01x0)")
            .expect("$sqrt with x bits")
            .output,
        "2.0"
    );
    assert_eq!(
        evaluate_input("$exp(0)").expect("$exp int").output,
        "1.0"
    );
}

// 2-arg math functions: $pow, $atan2, $hypot.
#[test]
fn real_math_two_argument_functions() {
    assert_eq!(
        evaluate_input("$pow(2.0, 10.0)").expect("$pow").output,
        "1024.0"
    );
    assert_eq!(
        evaluate_input("$atan2(0.0, 1.0)").expect("$atan2").output,
        "0.0"
    );
    assert_eq!(
        evaluate_input("$hypot(3.0, 4.0)").expect("$hypot").output,
        "5.0"
    );
}

// $pow shares f64::powf with the `**` operator on reals — same corners
// (0**0=1.0, neg**non-integral=NaN, 0**neg=+inf) the README pins down.
#[test]
fn pow_matches_real_power_operator_corners() {
    assert_eq!(
        evaluate_input("$pow(0.0, 0.0)").expect("0**0").output,
        "1.0"
    );
    assert_eq!(
        evaluate_input("$pow(0.0, -1.0)").expect("0**neg").output,
        "inf"
    );
    assert_eq!(
        evaluate_input("$pow(-2.0, 0.5)")
            .expect("neg ** non-integral")
            .output,
        "NaN"
    );
}

// IEEE 754 propagation for NaN/±∞ flows through f64 directly.
#[test]
fn real_math_nan_and_infinity_propagate() {
    assert_eq!(
        evaluate_input("$sqrt(-1.0)").expect("$sqrt neg").output,
        "NaN"
    );
    assert_eq!(
        evaluate_input("$ln(0.0)").expect("$ln(0)").output,
        "-inf"
    );
    assert_eq!(
        evaluate_input("$ln(-1.0)").expect("$ln neg").output,
        "NaN"
    );
    assert_eq!(
        evaluate_input("$acos(2.0)")
            .expect("$acos out of range")
            .output,
        "NaN"
    );
}

// Parser: missing parens, wrong arity.
#[test]
fn math_function_parser_errors() {
    assert_eq!(
        evaluate_input("$sqrt 4.0").expect_err("missing `(`"),
        "Syntax error: expected `(` after $sqrt"
    );
    assert_eq!(
        evaluate_input("$pow(2.0").expect_err("missing `)`"),
        "Syntax error: expected `)` after $pow argument"
    );
    assert_eq!(
        evaluate_input("$pow(1.0)").expect_err("$pow 1 arg"),
        "Syntax error: $pow expects 2 arguments, got 1"
    );
    assert_eq!(
        evaluate_input("$sqrt(1.0, 2.0)").expect_err("$sqrt 2 args"),
        "Syntax error: $sqrt expects 1 argument, got 2"
    );
    assert_eq!(
        evaluate_input("$clog2(1, 2)").expect_err("$clog2 2 args"),
        "Syntax error: $clog2 expects 1 argument, got 2"
    );
}

// `reg` declarations and blocking assignment — the smallest end-to-end
// variable type. These tests cover decl forms (with / without range, signed,
// reversed range, multi-name), default x-initialization, width/sign behavior
// of blocking assignment, the binary display base for regs, error surfaces
// for undeclared / redeclared identifiers and real RHS, and Session state
// persistence across multiple `eval` calls.

#[test]
fn reg_decl_without_range_is_one_bit_unsigned() {
    let mut session = Session::new();
    assert!(session.eval("reg a").expect("decl").output.is_empty());
    assert_eq!(session.eval("a").expect("read").output, "1'bx");
}

#[test]
fn reg_decl_with_range_initializes_to_x() {
    let mut session = Session::new();
    assert!(session.eval("reg [7:0] a").expect("decl").output.is_empty());
    assert_eq!(session.eval("a").expect("read").output, "8'bxxxxxxxx");
}

#[test]
fn reg_signed_decl_renders_with_signed_marker() {
    let mut session = Session::new();
    assert!(
        session
            .eval("reg signed [7:0] a")
            .expect("decl")
            .output
            .is_empty()
    );
    assert_eq!(session.eval("a").expect("read").output, "8'sbxxxxxxxx");
}

#[test]
fn reg_decl_with_multiple_names_in_one_statement() {
    let mut session = Session::new();
    assert!(
        session
            .eval("reg [3:0] a, b, c")
            .expect("decl")
            .output
            .is_empty()
    );
    assert_eq!(session.eval("a").expect("read a").output, "4'bxxxx");
    assert_eq!(session.eval("b").expect("read b").output, "4'bxxxx");
    assert_eq!(session.eval("c").expect("read c").output, "4'bxxxx");
}

#[test]
fn reg_decl_with_reversed_range_yields_same_width() {
    // LRM 4.8: a reversed `[lsb:msb]` is tolerated; width is |msb - lsb| + 1.
    let mut session = Session::new();
    session.eval("reg [0:7] a").expect("decl");
    assert_eq!(session.eval("a").expect("read").output, "8'bxxxxxxxx");
}

#[test]
fn reversed_and_forward_reg_ranges_behave_the_same_in_expressions() {
    let mut session = Session::new();
    session.eval("reg [3:0] a").expect("forward decl");
    session.eval("reg [0:3] b").expect("reversed decl");
    session.eval("a = 4'b1010").expect("assign a");
    session.eval("b = 4'b1010").expect("assign b");

    assert_eq!(session.eval("a + 1").expect("a + 1").output, "32'b00000000000000000000000000001011");
    assert_eq!(session.eval("b + 1").expect("b + 1").output, "32'b00000000000000000000000000001011");
    assert_eq!(session.eval("a == b").expect("a == b").output, "1'b1");
    assert_eq!(session.eval("{a,b}").expect("concat").output, "8'b10101010");
}

#[test]
fn forward_and_reversed_reg_ranges_preserve_declared_endpoints() {
    let mut session = Session::new();
    session.eval("reg [3:0] a").expect("forward decl");
    session.eval("reg [0:3] b").expect("reversed decl");

    assert_eq!(
        session.lookup_reg_range("a").expect("range for a"),
        (&BigInt::from(3u8), &BigInt::from(0u8))
    );
    assert_eq!(
        session.lookup_reg_range("b").expect("range for b"),
        (&BigInt::from(0u8), &BigInt::from(3u8))
    );
}

#[test]
fn scalar_and_explicit_one_bit_reg_ranges_remain_distinct() {
    let mut session = Session::new();
    session.eval("reg a").expect("scalar decl");
    session.eval("reg [0:0] b").expect("explicit one-bit decl");

    assert_eq!(session.lookup_reg_range("a"), None);
    assert_eq!(
        session.lookup_reg_range("b").expect("range for b"),
        (&BigInt::from(0u8), &BigInt::from(0u8))
    );

    assert_eq!(session.eval("a = 1'b1").expect("assign a").output, "1'b1");
    assert_eq!(session.eval("b = 1'b1").expect("assign b").output, "1'b1");
    assert_eq!(session.eval("a == b").expect("a == b").output, "1'b1");
}

#[test]
fn assignment_preserves_declared_reg_range_metadata() {
    let mut session = Session::new();
    session.eval("reg [0:3] a").expect("decl");
    session.eval("a = 4'b1010").expect("assign");

    assert_eq!(
        session.lookup_reg_range("a").expect("range for a"),
        (&BigInt::from(0u8), &BigInt::from(3u8))
    );
}

#[test]
fn redeclaration_replaces_declared_reg_range_metadata() {
    let mut session = Session::new();
    session.eval("reg [3:0] a").expect("first decl");
    session.eval("reg [0:3] a").expect("redecl");

    assert_eq!(
        session.lookup_reg_range("a").expect("range for a"),
        (&BigInt::from(0u8), &BigInt::from(3u8))
    );
}

#[test]
fn negative_reg_ranges_preserve_declared_endpoints() {
    let mut session = Session::new();
    session.eval("reg [-1:0] a").expect("negative decl");
    session.eval("reg [1:-2] b").expect("mixed-sign decl");

    assert_eq!(
        session.lookup_reg_range("a").expect("range for a"),
        (&BigInt::from(-1), &BigInt::from(0u8))
    );
    assert_eq!(
        session.lookup_reg_range("b").expect("range for b"),
        (&BigInt::from(1u8), &BigInt::from(-2))
    );
}

#[test]
fn constant_expression_reg_ranges_store_evaluated_endpoints() {
    let mut session = Session::new();
    session.eval("reg [3+1:0] a").expect("decl");

    assert_eq!(
        session.lookup_reg_range("a").expect("range for a"),
        (&BigInt::from(4u8), &BigInt::from(0u8))
    );
}

#[test]
fn reg_decl_with_constant_expression_range() {
    let mut session = Session::new();
    session.eval("reg [3+1:0] a").expect("decl");
    assert_eq!(session.eval("a").expect("read").output, "5'bxxxxx");
}

#[test]
fn reg_decl_produces_empty_out_line() {
    // Mirrors the `$finish`/`$stop` empty-Out convention for non-value
    // statements.
    let evaluation = evaluate_input("reg [7:0] a").expect("decl");
    assert_eq!(evaluation.output, "");
    assert!(!evaluation.should_exit);
}

#[test]
fn assignment_truncates_wider_rhs_to_reg_width() {
    let mut session = Session::new();
    session.eval("reg [3:0] a").expect("decl");
    assert_eq!(session.eval("a = 8'hff").expect("assign").output, "4'b1111");
}

#[test]
fn assignment_sign_extends_narrower_rhs_into_signed_reg() {
    let mut session = Session::new();
    session.eval("reg signed [7:0] a").expect("decl");
    // Binary base is the reg's default display, so the sign-extended bits
    // print in their per-position form rather than as signed decimal.
    assert_eq!(
        session.eval("a = 4'shf").expect("assign").output,
        "8'sb11111111"
    );
}

#[test]
fn assignment_preserves_x_and_z_bits() {
    let mut session = Session::new();
    session.eval("reg [3:0] a").expect("decl");
    assert_eq!(
        session.eval("a = 4'b10xz").expect("assign").output,
        "4'b10xz"
    );
}

#[test]
fn reg_value_participates_in_later_expression_with_its_own_base() {
    // After storing 4'h0a into an 8-bit binary-base reg, `a + 4'b1` is
    // evaluated with `a`'s metadata propagated (binary base wins from the
    // leftmost operand, width = 8).
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("decl");
    session.eval("a = 4'h0a").expect("assign");
    assert_eq!(
        session.eval("a + 4'b1").expect("expr").output,
        "8'b00001011"
    );
}

#[test]
fn assignment_of_real_value_implicitly_converts_per_lrm_3_5_3() {
    // LRM §3.5.3: implicit real→integer conversion rounds to nearest
    // with ties away from zero (distinct from `$rtoi`'s truncation). So
    // `1.5` rounds to 2, not truncates to 1.
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("decl");
    assert_eq!(
        session.eval("a = 1.5").expect("real RHS rounds").output,
        "8'b00000010"
    );
    assert_eq!(
        session.eval("a = -2.5").expect("ties away from zero").output,
        "8'b11111101"
    );
    assert_eq!(
        session.eval("a = 3.4").expect("rounds toward 3").output,
        "8'b00000011"
    );
}

#[test]
fn assignment_of_nan_or_infinity_real_fills_lvalue_with_x_bits() {
    // NaN / ±∞ have no integer image (`$rtoi` returns 32 bits of x for
    // these). For an assignment lvalue we surface that "no defined
    // integer" by filling the reg's declared width with x.
    let mut session = Session::new();
    session.eval("reg [3:0] a").expect("decl");
    assert_eq!(
        session.eval("a = 0.0/0.0").expect("NaN").output,
        "4'bxxxx"
    );
    assert_eq!(
        session.eval("a = 1.0/0.0").expect("+inf").output,
        "4'bxxxx"
    );
}

#[test]
fn reading_undeclared_identifier_is_an_error() {
    let mut session = Session::new();
    let err = session
        .eval("b + 1")
        .expect_err("undeclared identifier should be rejected");
    assert_eq!(err, "Semantic error: undeclared identifier: b");
}

#[test]
fn assigning_to_undeclared_identifier_is_an_error() {
    let mut session = Session::new();
    let err = session
        .eval("b = 1")
        .expect_err("assignment to undeclared should be rejected");
    assert_eq!(err, "Semantic error: undeclared identifier: b");
}

#[test]
fn redeclaration_replaces_the_previous_binding() {
    // The REPL is single-scope and a redecl is the user's way of resetting
    // a reg's metadata. The new decl wipes width / signed / base / value;
    // the new reg starts at all-x just like a fresh one.
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("first decl");
    session.eval("a = 8'h2a").expect("populate");
    assert_eq!(session.eval("a").expect("read").output, "8'b00101010");
    session.eval("reg [3:0] a").expect("redecl narrower");
    assert_eq!(
        session.eval("a").expect("read after redecl").output,
        "4'bxxxx"
    );
}

#[test]
fn reg_decl_accepts_negative_range_endpoint() {
    let mut session = Session::new();
    session.eval("reg [-1:0] a").expect("negative endpoint should be accepted");
    assert_eq!(session.eval("a").expect("read").output, "2'bxx");
}

#[test]
fn reg_decl_accepts_mixed_sign_range_endpoint() {
    let mut session = Session::new();
    session
        .eval("reg [1:-2] a")
        .expect("mixed-sign endpoints should be accepted");
    assert_eq!(session.eval("a").expect("read").output, "4'bxxxx");
}

#[test]
fn reg_decl_rejects_x_range_endpoint() {
    let err = evaluate_input("reg ['bx:0] a").expect_err("x range should be rejected");
    assert!(
        err.contains("range") && err.contains("unknown"),
        "error should mention range and unknown bits, got: {err}"
    );
}

#[test]
fn reg_decl_rejects_range_width_that_overflows_usize() {
    let input = format!("reg [{}:0] a", usize::MAX);
    let err = evaluate_input(&input).expect_err("overflowing width should be rejected");
    assert_eq!(err, "Semantic error: reg range width too large");
}

#[test]
fn session_state_persists_across_eval_calls() {
    // The plan's "declare in one call, assign in another, read in a third"
    // scenario: each step is a separate `eval` so the session state is the
    // only thing carrying `a` between them.
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("decl");
    assert_eq!(
        session.eval("a = 4'hF + 4'hF").expect("assign").output,
        "8'b00011110"
    );
    assert_eq!(session.eval("a").expect("read").output, "8'b00011110");
}

#[test]
fn reg_decl_init_value_populates_bits() {
    let mut session = Session::new();
    session.eval("reg [7:0] a = 8'h2a").expect("decl with init");
    assert_eq!(session.eval("a").expect("read").output, "8'b00101010");
}

#[test]
fn reg_decl_init_truncates_wider_literal_to_reg_width() {
    // The init RHS goes through the same width context as a blocking
    // assignment, so an 8-bit literal narrowed to a 4-bit reg keeps the
    // low 4 bits.
    let mut session = Session::new();
    session.eval("reg [3:0] a = 8'hff").expect("decl with init");
    assert_eq!(session.eval("a").expect("read").output, "4'b1111");
    let mut session = Session::new();
    session.eval("reg [3:0] a = 8'h1f").expect("decl with init");
    assert_eq!(session.eval("a").expect("read").output, "4'b1111");
}

#[test]
fn reg_decl_init_sign_extends_into_signed_reg() {
    // `-1` is unsized signed 32-bit; flowing through a signed 4-bit reg
    // sign-extends down to all-ones — same path as `a = -1`.
    let mut session = Session::new();
    session.eval("reg signed [3:0] s = -1").expect("decl with signed init");
    assert_eq!(session.eval("s").expect("read").output, "4'sb1111");
}

#[test]
fn reg_decl_init_real_value_implicitly_converts_per_lrm_3_5_3() {
    // Real init triggers the same implicit real→integer conversion as a
    // blocking assignment: round half away from zero.
    let mut session = Session::new();
    session.eval("reg [7:0] a = 1.5").expect("real init rounds");
    assert_eq!(session.eval("a").expect("read").output, "8'b00000010");
    let mut session = Session::new();
    session.eval("reg signed [7:0] a = -2.5").expect("ties away from zero");
    assert_eq!(session.eval("a").expect("read").output, "8'sb11111101");
    let mut session = Session::new();
    session.eval("reg [7:0] a = 3.4").expect("rounds toward 3");
    assert_eq!(session.eval("a").expect("read").output, "8'b00000011");
}

#[test]
fn reg_decl_init_nan_or_infinity_fills_with_x_bits() {
    let mut session = Session::new();
    session.eval("reg [3:0] a = 0.0/0.0").expect("NaN");
    assert_eq!(session.eval("a").expect("read").output, "4'bxxxx");
    let mut session = Session::new();
    session.eval("reg [3:0] a = 1.0/0.0").expect("+inf");
    assert_eq!(session.eval("a").expect("read").output, "4'bxxxx");
}

#[test]
fn reg_decl_partial_init_list_leaves_uninitialized_names_x() {
    // `reg a, b = 5, c` declares three 1-bit regs; only `b` carries an
    // init expression, so `a` and `c` retain the default x bits.
    let mut session = Session::new();
    session.eval("reg a, b = 5, c").expect("partial init list");
    assert_eq!(session.eval("a").expect("read a").output, "1'bx");
    assert_eq!(session.eval("b").expect("read b").output, "1'b1");
    assert_eq!(session.eval("c").expect("read c").output, "1'bx");
}

#[test]
fn reg_decl_init_sees_earlier_name_in_same_decl() {
    // LRM A.2.3 lists variable_types in textual order, so the natural
    // semantics — and the most useful for a calculator — is for `b`'s
    // init to see `a`'s freshly-applied init value.
    let mut session = Session::new();
    session
        .eval("reg [3:0] a = 1, b = a + 1")
        .expect("sequential init");
    assert_eq!(session.eval("a").expect("read a").output, "4'b0001");
    assert_eq!(session.eval("b").expect("read b").output, "4'b0010");
}

#[test]
fn reg_decl_self_referencing_init_reads_prior_binding() {
    // The init expression is evaluated against the session as-is — i.e.
    // before the new binding replaces the old one — so a self-reference
    // pulls the prior value through the init RHS. Same-width redecl
    // with `= a` is therefore an idiomatic "carry the old value forward".
    let mut session = Session::new();
    session.eval("reg [3:0] a = 7").expect("first decl");
    session
        .eval("reg [3:0] a = a")
        .expect("redecl with self-init");
    assert_eq!(session.eval("a").expect("read").output, "4'b0111");
}

#[test]
fn reg_decl_self_referencing_init_narrows_prior_binding() {
    // Narrowing redecl with `= a` carries the prior bits through the
    // assignment-RHS width context, dropping high bits. With prior
    // `reg [1:0] a = 2'b11` (=3) and a new 1-bit `reg a = a`, the low
    // bit survives.
    let mut session = Session::new();
    session.eval("reg [1:0] a = 2'b11").expect("first decl");
    session.eval("reg a = a").expect("redecl narrower with self-init");
    assert_eq!(session.eval("a").expect("read").output, "1'b1");
}

#[test]
fn reg_decl_self_referencing_init_without_prior_binding_errors() {
    // No prior binding means the identifier in the init RHS is genuinely
    // undeclared at evaluation time — surface the same error path as a
    // normal expression.
    let err = evaluate_input("reg a = a")
        .expect_err("self-init without prior binding errors");
    assert_eq!(err, "Semantic error: undeclared identifier: a");
}

#[test]
fn reg_decl_init_can_reference_previously_declared_reg() {
    let mut session = Session::new();
    session.eval("reg [3:0] a = 5").expect("first decl");
    session.eval("reg [7:0] b = a + 1").expect("init from prior reg");
    assert_eq!(session.eval("b").expect("read").output, "8'b00000110");
}

#[test]
fn reg_decl_init_preserves_declared_reg_range_metadata() {
    // The init applies after the RegValue is inserted, so the range
    // metadata stored at decl time is still present afterwards.
    let mut session = Session::new();
    session.eval("reg [0:3] a = 4'b1010").expect("decl with init");
    assert_eq!(
        session.lookup_reg_range("a").expect("range for a"),
        (&BigInt::from(0u8), &BigInt::from(3u8))
    );
    assert_eq!(session.eval("a").expect("read").output, "4'b1010");
}

#[test]
fn reg_decl_init_propagates_rhs_evaluation_error() {
    // A bare init expression has access to the surrounding session, so
    // referencing an undeclared identifier surfaces the usual error
    // rather than silently leaving the new reg at x.
    let err = evaluate_input("reg [3:0] a = nope")
        .expect_err("undeclared identifier in init");
    assert_eq!(err, "Semantic error: undeclared identifier: nope");
}

#[test]
fn reg_decl_failed_init_in_multi_name_decl_leaves_no_partial_state() {
    // The decl is committed all-or-nothing: a later init's failure means
    // none of the earlier names land in the session, so the user does not
    // see `a` silently bound when the line ended in an error.
    let mut session = Session::new();
    let err = session
        .eval("reg [3:0] a = 1, b = nope")
        .expect_err("undeclared identifier in init");
    assert_eq!(err, "Semantic error: undeclared identifier: nope");
    assert!(session.lookup("a").is_none(), "a should not be bound");
    assert!(session.lookup("b").is_none(), "b should not be bound");
}

#[test]
fn reg_decl_failed_init_preserves_prior_binding_for_redeclared_name() {
    // Stronger version of the rollback: when `a` already has a binding,
    // a failed redecl that names `a` must leave the prior `a` exactly as
    // it was — staged inserts never reach the live session on error.
    let mut session = Session::new();
    session.eval("reg [3:0] a = 7").expect("prior decl");
    let err = session
        .eval("reg [3:0] a = 1, b = nope")
        .expect_err("undeclared identifier in init");
    assert_eq!(err, "Semantic error: undeclared identifier: nope");
    assert_eq!(session.eval("a").expect("read a").output, "4'b0111");
    assert!(session.lookup("b").is_none(), "b should not be bound");
}

#[test]
fn reg_decl_rejects_duplicate_names_even_with_init() {
    let err = evaluate_input("reg [3:0] a = 1, a = 2")
        .expect_err("duplicate names rejected");
    assert!(
        err.contains("duplicate name"),
        "error should mention duplicate name, got: {err}"
    );
}

#[test]
fn bit_select_returns_each_bit_from_forward_decl() {
    let mut session = Session::new();
    session
        .eval("reg [7:0] r = 8'b10110010")
        .expect("decl with init");
    assert_eq!(session.eval("r[0]").expect("bit 0").output, "1'b0");
    assert_eq!(session.eval("r[1]").expect("bit 1").output, "1'b1");
    assert_eq!(session.eval("r[7]").expect("bit 7").output, "1'b1");
}

#[test]
fn bit_select_maps_source_index_to_internal_on_reversed_decl() {
    // `reg [0:7]` puts source index 0 at the MSB end; the formula
    // `internal = |src - lsb_decl|` flips it back to the right bit.
    let mut session = Session::new();
    session
        .eval("reg [0:7] r = 8'b10110010")
        .expect("decl with reversed range");
    // 8'b10110010 has bits[7]=1, bits[0]=0 LSB-first; with lsb_decl=7,
    // src=0 → internal=7 (MSB), src=7 → internal=0 (LSB).
    assert_eq!(session.eval("r[0]").expect("MSB").output, "1'b1");
    assert_eq!(session.eval("r[7]").expect("LSB").output, "1'b0");
}

#[test]
fn constant_part_select_on_forward_decl_returns_unsigned_slice() {
    // Part-select results are always unsigned per LRM 4.7, and the
    // declared reg base (Binary, set by the decl path) flows through.
    let mut session = Session::new();
    session
        .eval("reg [7:0] r = 8'hAB")
        .expect("decl with init");
    assert_eq!(session.eval("r[3:0]").expect("low nibble").output, "4'b1011");
    assert_eq!(session.eval("r[7:4]").expect("high nibble").output, "4'b1010");
}

#[test]
fn constant_part_select_on_reversed_decl_requires_msb_le_lsb() {
    // For `[0:7]`, smaller source index is more significant, so the
    // legal direction is `[smaller:larger]`.
    let mut session = Session::new();
    session
        .eval("reg [0:7] r = 8'b10110010")
        .expect("decl with reversed range");
    assert_eq!(session.eval("r[2:5]").expect("legal").output, "4'b1100");
    let err = session
        .eval("r[5:2]")
        .expect_err("forward direction on reversed decl errors");
    assert!(
        err.contains("direction does not match"),
        "error should mention direction mismatch, got: {err}"
    );
}

#[test]
fn constant_part_select_wrong_direction_on_forward_decl_errors() {
    let mut session = Session::new();
    session
        .eval("reg [7:0] r = 8'hAB")
        .expect("decl with init");
    let err = session
        .eval("r[2:5]")
        .expect_err("reversed direction on forward decl errors");
    assert!(
        err.contains("direction does not match"),
        "error should mention direction mismatch, got: {err}"
    );
}

#[test]
fn indexed_part_select_up_walks_from_base_upward() {
    let mut session = Session::new();
    session
        .eval("reg [7:0] r = 8'b10101010")
        .expect("decl with init");
    // [base +: width] selects bits base..base+width-1; for forward decl
    // the larger source index is more significant.
    assert_eq!(session.eval("r[2 +: 4]").expect("up").output, "4'b1010");
}

#[test]
fn indexed_part_select_down_walks_from_base_downward() {
    let mut session = Session::new();
    session
        .eval("reg [7:0] r = 8'b10101010")
        .expect("decl with init");
    // [base -: width] selects bits base-width+1..base, same bit range
    // as the `2 +: 4` case above.
    assert_eq!(session.eval("r[5 -: 4]").expect("down").output, "4'b1010");
}

#[test]
fn out_of_range_bit_select_yields_x() {
    // LRM 4.2.1: bit-select with index outside the declared range
    // returns x.
    let mut session = Session::new();
    session
        .eval("reg [7:0] r = 8'hAB")
        .expect("decl with init");
    assert_eq!(session.eval("r[8]").expect("above range").output, "1'bx");
    assert_eq!(
        session.eval("r[-1]").expect("below range").output,
        "1'bx"
    );
}

#[test]
fn out_of_range_part_select_fills_only_the_out_of_range_bits_with_x() {
    // LRM 4.2.1's "out-of-range → x" rule applies per position, so the
    // in-range bits keep their value and only the off-the-end positions
    // become x. 8'hAB = 8'b10101011, so bits 6 and 7 are `1` and `0`,
    // and bits 8 / 9 are out of range.
    let mut session = Session::new();
    session
        .eval("reg [7:0] r = 8'hAB")
        .expect("decl with init");
    assert_eq!(
        session.eval("r[9:6]").expect("constant partial overlap").output,
        "4'bxx10"
    );
    assert_eq!(
        session.eval("r[6 +: 4]").expect("indexed partial overlap").output,
        "4'bxx10"
    );
    // The example from the bug report, exact wording: `reg [3:0] a =
    // 4'b0101; a[4:3]` → `2'bx0` (bit 4 oob → x; bit 3 in range → 0).
    let mut session = Session::new();
    session.eval("reg [3:0] a = 4'b0101").expect("decl with init");
    assert_eq!(session.eval("a[4:3]").expect("partial overlap").output, "2'bx0");
}

#[test]
fn xz_in_bit_select_index_yields_x() {
    let mut session = Session::new();
    session
        .eval("reg [7:0] r = 8'hAB")
        .expect("decl with init");
    session
        .eval("reg [3:0] i = 4'bxx10")
        .expect("decl with x bits");
    // i has x bits anywhere → bit-select index unknown → result 1'bx.
    assert_eq!(session.eval("r[i]").expect("x in index").output, "1'bx");
}

#[test]
fn xz_in_indexed_part_select_base_yields_all_x() {
    let mut session = Session::new();
    session
        .eval("reg [7:0] r = 8'hAB")
        .expect("decl with init");
    session
        .eval("reg [3:0] i = 4'bxx10")
        .expect("decl with x bits");
    assert_eq!(
        session.eval("r[i +: 4]").expect("x in base").output,
        "4'bxxxx"
    );
}

#[test]
fn xz_in_constant_part_select_endpoint_errors() {
    let mut session = Session::new();
    session
        .eval("reg [7:0] r = 8'hAB")
        .expect("decl with init");
    let err = session
        .eval("r[4'bxxxx:0]")
        .expect_err("x in constant endpoint errors");
    assert!(
        err.contains("part-select msb contains unknown bits"),
        "error should mention unknown bits, got: {err}"
    );
}

#[test]
fn xz_or_nonpositive_indexed_width_errors() {
    let mut session = Session::new();
    session
        .eval("reg [7:0] r = 8'hAB")
        .expect("decl with init");
    let err_xz = session
        .eval("r[0 +: 4'bxxxx]")
        .expect_err("x in width errors");
    assert!(
        err_xz.contains("indexed part-select width contains unknown bits"),
        "error should mention unknown bits, got: {err_xz}"
    );
    let err_zero = session
        .eval("r[0 +: 0]")
        .expect_err("zero width errors");
    assert!(
        err_zero.contains("indexed part-select width must be positive"),
        "error should mention positive, got: {err_zero}"
    );
    let err_neg = session
        .eval("r[0 +: -1]")
        .expect_err("negative width errors");
    assert!(
        err_neg.contains("indexed part-select width must be positive"),
        "error should mention positive, got: {err_neg}"
    );
}

#[test]
fn select_from_signed_reg_is_unsigned() {
    // LRM 4.7: a part-select is always unsigned, even on a signed reg.
    // -8'sd1 stores all-ones; the 8-bit select reads back all-ones as
    // an unsigned 8-bit value (rendered in the reg's binary base).
    let mut session = Session::new();
    session
        .eval("reg signed [7:0] s = -8'sd1")
        .expect("signed decl");
    assert_eq!(session.eval("s[7:0]").expect("full select").output, "8'b11111111");
}

#[test]
fn literal_cannot_be_followed_by_bit_select() {
    // `Expr::Select` only forms from the `Token::Identifier` branch of
    // `parse_primary`, so `4'b1111[0]` leaves the `[0]` to dangle and
    // surfaces as a statement-boundary parse error.
    let err = evaluate_input("4'b1111[0]").expect_err("literal select rejected");
    assert!(
        err.contains("unexpected token"),
        "error should mention unexpected token, got: {err}"
    );
}

#[test]
fn select_result_widens_in_outer_context() {
    // The select itself is self-determined unsigned, but the outer
    // context (`+ 16'b0`) widens it to 16 bits with zero extension.
    let mut session = Session::new();
    session
        .eval("reg [7:0] r = 8'hAB")
        .expect("decl with init");
    assert_eq!(
        session.eval("r[3:0] + 16'b0").expect("widened").output,
        "16'b0000000000001011"
    );
}

#[test]
fn bit_select_on_negative_endpoint_reg() {
    // `reg [-1:2]` is a reversed decl (msb < lsb) with width 4; the
    // source-index mapping handles negative endpoints the same way as
    // any other reversed decl.
    let mut session = Session::new();
    session
        .eval("reg [-1:2] r = 4'b1011")
        .expect("negative-endpoint decl");
    assert_eq!(session.eval("r[-1]").expect("MSB end").output, "1'b1");
    assert_eq!(session.eval("r[2]").expect("LSB end").output, "1'b1");
    assert_eq!(session.eval("r[0]").expect("middle").output, "1'b0");
    assert_eq!(session.eval("r[1]").expect("middle").output, "1'b1");
}

#[test]
fn select_on_scalar_reg_is_illegal_per_lrm_5_2_1() {
    // LRM 5.2.1: "A bit-select or part-select of a scalar ... shall be
    // illegal." A reg declared with no range is a scalar even when its
    // width happens to be 1; all four select forms must reject it.
    let mut session = Session::new();
    session.eval("reg a").expect("scalar decl");
    for form in ["a[0]", "a[0:0]", "a[0 +: 1]", "a[0 -: 1]"] {
        let err = session
            .eval(form)
            .expect_err(&format!("{form} on scalar reg should error"));
        assert!(
            err.contains("scalar reg"),
            "error should mention scalar reg, got: {err}"
        );
    }
}

#[test]
fn one_bit_vector_reg_still_allows_selects() {
    // `reg [0:0] a` is a 1-bit *vector*, not a scalar, so the same
    // selects that error on `reg a` succeed here.
    let mut session = Session::new();
    session.eval("reg [0:0] a = 1'b1").expect("vector decl");
    assert_eq!(session.eval("a[0]").expect("bit").output, "1'b1");
    assert_eq!(session.eval("a[0:0]").expect("part const").output, "1'b1");
    assert_eq!(session.eval("a[0 +: 1]").expect("up").output, "1'b1");
    assert_eq!(session.eval("a[0 -: 1]").expect("down").output, "1'b1");
}

#[test]
fn indexed_part_select_requires_adjacent_colon() {
    // `+:` is lexed greedily and adjacency-only; a space between the
    // `+` and `:` breaks the token boundary and the bracket contents
    // no longer match any select form, so it fails at parse.
    let mut session = Session::new();
    session
        .eval("reg [7:0] r = 8'hAB")
        .expect("decl with init");
    assert_eq!(
        session.eval("r[2 +: 4]").expect("adjacent ok").output,
        "4'b1010"
    );
    let err = session
        .eval("r[2 + : 4]")
        .expect_err("space-separated rejected");
    assert!(
        !err.is_empty(),
        "space-separated `+ :` should not parse as indexed select"
    );
}

// ===========================================================================
// LRM A.8.5 variable_lvalue: bit/part-select and concatenation on the LHS.
// ===========================================================================

#[test]
fn bare_name_lhs_unchanged() {
    // Regression guard: extending Stmt::Assign to a full LValue must not
    // perturb the original bare-name behavior.
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("decl");
    assert_eq!(
        session.eval("a = 8'hAB").expect("bare assign").output,
        "8'b10101011"
    );
    assert_eq!(session.eval("a").expect("read").output, "8'b10101011");
}

#[test]
fn bit_select_lhs_writes_single_bit() {
    let mut session = Session::new();
    session.eval("reg [7:0] r = 8'h00").expect("decl");
    assert_eq!(
        session.eval("r[2] = 1'b1").expect("bit-select assign").output,
        "1'b1"
    );
    assert_eq!(session.eval("r").expect("read r").output, "8'b00000100");
}

#[test]
fn part_const_lhs_writes_slice() {
    let mut session = Session::new();
    session.eval("reg [7:0] r = 8'h00").expect("decl");
    assert_eq!(
        session
            .eval("r[5:2] = 4'hF")
            .expect("part-const assign")
            .output,
        "4'b1111"
    );
    assert_eq!(session.eval("r").expect("read r").output, "8'b00111100");
}

#[test]
fn part_indexed_up_lhs_writes_slice() {
    // `r[2 +: 4]` covers source indices 2..5 (LSB-first); for forward
    // range [7:0] that maps to internal indices [2,3,4,5].
    let mut session = Session::new();
    session.eval("reg [7:0] r = 8'h00").expect("decl");
    assert_eq!(
        session
            .eval("r[2 +: 4] = 4'b1010")
            .expect("indexed-up assign")
            .output,
        "4'b1010"
    );
    assert_eq!(session.eval("r").expect("read r").output, "8'b00101000");
}

#[test]
fn part_indexed_down_lhs_writes_slice() {
    // `r[5 -: 4]` covers source indices 2..5 — bit-for-bit equivalent to
    // `r[2 +: 4]` on a forward range — so the result matches the up form.
    let mut session = Session::new();
    session.eval("reg [7:0] r = 8'h00").expect("decl");
    assert_eq!(
        session
            .eval("r[5 -: 4] = 4'b1010")
            .expect("indexed-down assign")
            .output,
        "4'b1010"
    );
    assert_eq!(session.eval("r").expect("read r").output, "8'b00101000");
}

#[test]
fn concat_lhs_distributes_bits() {
    // Leaves are flattened left-to-right; the RHS bit stream feeds from
    // LSB end (rightmost leaf) to MSB end (leftmost leaf).
    let mut session = Session::new();
    session.eval("reg [3:0] a = 4'h0").expect("decl a");
    session.eval("reg [3:0] b = 4'h0").expect("decl b");
    assert_eq!(
        session
            .eval("{a, b} = 8'hAB")
            .expect("concat assign")
            .output,
        "8'b10101011"
    );
    assert_eq!(session.eval("a").expect("read a").output, "4'b1010");
    assert_eq!(session.eval("b").expect("read b").output, "4'b1011");
}

#[test]
fn nested_concat_lhs() {
    // `{x, {y, z}}` flattens to [x, y, z]; the inner concat is
    // structural, not a new scope.
    let mut session = Session::new();
    session.eval("reg [1:0] x = 2'b00").expect("decl x");
    session.eval("reg [1:0] y = 2'b00").expect("decl y");
    session.eval("reg [1:0] z = 2'b00").expect("decl z");
    assert_eq!(
        session
            .eval("{x, {y, z}} = 6'b110010")
            .expect("nested concat assign")
            .output,
        "6'b110010"
    );
    assert_eq!(session.eval("x").expect("read x").output, "2'b11");
    assert_eq!(session.eval("y").expect("read y").output, "2'b00");
    assert_eq!(session.eval("z").expect("read z").output, "2'b10");
}

#[test]
fn concat_lhs_with_selects() {
    // Mixed concat: each leaf computes its own internal-index sequence,
    // then they're stitched into a single bit stream.
    let mut session = Session::new();
    session.eval("reg [7:0] a = 8'h00").expect("decl a");
    session.eval("reg [7:0] b = 8'h00").expect("decl b");
    assert_eq!(
        session
            .eval("{a[3:0], b[7:4]} = 8'hAB")
            .expect("concat-of-selects assign")
            .output,
        "8'b10101011"
    );
    // a[3:0] receives the MSB-side nibble 0xA.
    assert_eq!(session.eval("a").expect("read a").output, "8'b00001010");
    // b[7:4] receives the LSB-side nibble 0xB.
    assert_eq!(session.eval("b").expect("read b").output, "8'b10110000");
}

#[test]
fn lhs_part_const_endpoints_runtime_eval() {
    // vcal evaluates "constant" endpoints against the live session so a
    // declared reg can supply the endpoint; same relaxation we already
    // grant the RHS select forms.
    let mut session = Session::new();
    session.eval("reg [3:0] hi = 5").expect("decl hi");
    session.eval("reg [7:0] r = 8'h00").expect("decl r");
    assert_eq!(
        session
            .eval("r[hi:2] = 4'hF")
            .expect("runtime endpoint")
            .output,
        "4'b1111"
    );
    assert_eq!(session.eval("r").expect("read r").output, "8'b00111100");
}

#[test]
fn lhs_select_on_negative_endpoint_reg() {
    // Reversed reg with a negative endpoint: same source-index → internal
    // mapping the RHS select forms use, just applied on the write side.
    let mut session = Session::new();
    session.eval("reg [-1:2] r = 4'b0000").expect("decl");
    assert_eq!(
        session.eval("r[-1] = 1'b1").expect("write MSB").output,
        "1'b1"
    );
    assert_eq!(session.eval("r").expect("read r").output, "4'b1000");
}

#[test]
fn lhs_bit_select_out_of_range_silently_drops() {
    // LRM 4.2.1: an out-of-range bit-select is "no assignment performed";
    // the reg keeps its prior bits and no error is raised.
    let mut session = Session::new();
    session.eval("reg [3:0] r = 4'b1010").expect("decl");
    assert_eq!(
        session.eval("r[7] = 1'b1").expect("oob bit-select").output,
        "1'b1"
    );
    assert_eq!(session.eval("r").expect("read r").output, "4'b1010");
}

#[test]
fn lhs_part_select_partial_overlap_drops_off_end() {
    // `r[5:2] = 4'b1111` on a 4-bit reg writes the in-range positions
    // (bits 2,3) and silently drops the out-of-range positions (4,5).
    let mut session = Session::new();
    session.eval("reg [3:0] r = 4'h0").expect("decl");
    assert_eq!(
        session
            .eval("r[5:2] = 4'b1111")
            .expect("partial overlap")
            .output,
        "4'b1111"
    );
    assert_eq!(session.eval("r").expect("read r").output, "4'b1100");
}

#[test]
fn lhs_bit_select_xz_index_silently_drops() {
    // LRM 4.2.1 again: an x/z index on the LHS performs no assignment;
    // the reg's prior bits are preserved.
    let mut session = Session::new();
    session.eval("reg [1:0] idx").expect("uninit idx is all x");
    session.eval("reg [3:0] r = 4'b1010").expect("decl r");
    assert_eq!(
        session
            .eval("r[idx] = 1'b1")
            .expect("x-index bit-select")
            .output,
        "1'b1"
    );
    assert_eq!(session.eval("r").expect("read r").output, "4'b1010");
}

#[test]
fn lhs_concat_duplicate_bit_picks_msb_side_leaf() {
    // IEEE 1364-2005 doesn't say what happens when an lvalue concat
    // names the same target bit twice — the result is implementation-
    // defined. vcal walks leaves right-to-left so the MSB-side leaf
    // writes last and wins. With `{a[0], a[0]} = 2'b10`, the MSB-side
    // a[0] receives the RHS MSB (1), the LSB-side a[0] receives the RHS
    // LSB (0) first, then the MSB-side write overwrites — net a[0] = 1.
    let mut session = Session::new();
    session.eval("reg [3:0] a = 4'h0").expect("decl");
    assert_eq!(
        session
            .eval("{a[0], a[0]} = 2'b10")
            .expect("duplicate-bit lvalue is not an error")
            .output,
        "2'b10"
    );
    assert_eq!(session.eval("a").expect("read a").output, "4'b0001");
}

#[test]
fn lhs_scalar_reg_with_select_rejected() {
    let mut session = Session::new();
    session.eval("reg s").expect("scalar decl");
    let err = session
        .eval("s[0] = 1'b1")
        .expect_err("select on scalar rejected");
    assert!(
        err.contains("scalar reg"),
        "want scalar-reg error, got: {err}"
    );
    assert_eq!(session.eval("s").expect("read s").output, "1'bx");
}

#[test]
fn lhs_part_const_direction_mismatch_rejected() {
    let mut session = Session::new();
    session.eval("reg [7:0] r = 8'h00").expect("decl");
    let err = session
        .eval("r[2:5] = 4'h0")
        .expect_err("direction mismatch rejected");
    assert!(
        err.contains("part-select direction"),
        "want direction error, got: {err}"
    );
    assert_eq!(session.eval("r").expect("read r").output, "8'b00000000");
}

#[test]
fn lhs_part_const_x_in_endpoint_rejected() {
    let mut session = Session::new();
    session.eval("reg [3:0] idx").expect("uninit idx is x");
    session.eval("reg [7:0] r = 8'h00").expect("decl r");
    let err = session
        .eval("r[idx:0] = 4'h0")
        .expect_err("x endpoint rejected");
    assert!(
        err.contains("part-select msb contains unknown bits"),
        "want x-endpoint error, got: {err}"
    );
}

#[test]
fn lhs_indexed_width_zero_rejected() {
    let mut session = Session::new();
    session.eval("reg [7:0] r = 8'h00").expect("decl");
    let err = session
        .eval("r[0 +: 0] = 0")
        .expect_err("zero width rejected");
    assert!(
        err.contains("indexed part-select width must be positive"),
        "want zero-width error, got: {err}"
    );
}

#[test]
fn lhs_undeclared_identifier_rejected() {
    let mut session = Session::new();
    let err = session
        .eval("nope = 1'b1")
        .expect_err("undeclared name rejected");
    assert_eq!(err, "Semantic error: undeclared identifier: nope");
}

#[test]
fn direction_error_runs_before_rhs_eval() {
    // LHS structural validation precedes RHS evaluation, so a direction
    // mismatch wins over an undeclared-name RHS error.
    let mut session = Session::new();
    session.eval("reg [7:0] r = 8'h00").expect("decl");
    let err = session
        .eval("r[2:5] = undeclared_rhs")
        .expect_err("direction wins");
    assert!(
        err.contains("part-select direction"),
        "direction error should fire before RHS eval, got: {err}"
    );
}

#[test]
fn lhs_bit_select_real_index_runs_before_rhs_eval() {
    // Real-typed bit-select index is a structural LRM-5.2 violation; it
    // must surface before the RHS is evaluated, so it wins over an
    // undeclared-name RHS error.
    let mut session = Session::new();
    session.eval("reg [3:0] r = 4'h0").expect("decl");
    let err = session
        .eval("r[1.5] = undeclared_rhs")
        .expect_err("real index rejected");
    assert_eq!(err, "Semantic error: bit-select index cannot be real");
    assert_eq!(session.eval("r").expect("read r").output, "4'b0000");
}

#[test]
fn lhs_indexed_part_select_real_base_runs_before_rhs_eval() {
    // Same rule for the `base` half of `+:` / `-:` — real bases are
    // structurally illegal and must outrank an RHS error.
    let mut session = Session::new();
    session.eval("reg [3:0] r = 4'h0").expect("decl");
    let err = session
        .eval("r[1.5 +: 2] = undeclared_rhs")
        .expect_err("real base rejected");
    assert_eq!(err, "Semantic error: indexed part-select base cannot be real");
    assert_eq!(session.eval("r").expect("read r").output, "4'b0000");
}

#[test]
fn lhs_undeclared_in_concat_rejected_all_or_nothing() {
    // Concat with an undeclared leaf must not partially commit the
    // declared leaf.
    let mut session = Session::new();
    session.eval("reg [3:0] a = 4'b0000").expect("decl a");
    let err = session
        .eval("{a, b} = 8'hFF")
        .expect_err("undeclared b rejected");
    assert_eq!(err, "Semantic error: undeclared identifier: b");
    assert_eq!(session.eval("a").expect("read a").output, "4'b0000");
}

#[test]
fn lhs_real_rhs_into_concat_converts() {
    // LRM 3.5.3: real RHS converts to integer (rounded) before
    // distribution. 6.7 → 7, then 7 in 6-bit unsigned = 6'b000111.
    let mut session = Session::new();
    session.eval("reg [1:0] a = 2'b00").expect("decl a");
    session.eval("reg [3:0] b = 4'b0000").expect("decl b");
    assert_eq!(
        session.eval("{a, b} = 6.7").expect("real RHS").output,
        "6'b000111"
    );
    assert_eq!(session.eval("a").expect("read a").output, "2'b00");
    assert_eq!(session.eval("b").expect("read b").output, "4'b0111");
}

#[test]
fn lhs_nan_rhs_fills_all_with_x() {
    // Real → integer with NaN yields the all-x value at the LHS width;
    // every distributed bit is then x.
    let mut session = Session::new();
    session.eval("reg [3:0] a = 4'b0000").expect("decl a");
    session.eval("reg [3:0] b = 4'b0000").expect("decl b");
    assert_eq!(
        session.eval("{a, b} = 0.0/0.0").expect("NaN RHS").output,
        "8'bxxxxxxxx"
    );
    assert_eq!(session.eval("a").expect("read a").output, "4'bxxxx");
    assert_eq!(session.eval("b").expect("read b").output, "4'bxxxx");
}

#[test]
fn lhs_rhs_truncates_to_concat_width() {
    // 16-bit RHS into 8-bit LHS keeps the low byte (0xAD); high byte
    // dropped.
    let mut session = Session::new();
    session.eval("reg [3:0] a = 4'b0000").expect("decl a");
    session.eval("reg [3:0] b = 4'b0000").expect("decl b");
    assert_eq!(
        session
            .eval("{a, b} = 16'hDEAD")
            .expect("truncate")
            .output,
        "8'b10101101"
    );
    assert_eq!(session.eval("a").expect("read a").output, "4'b1010");
    assert_eq!(session.eval("b").expect("read b").output, "4'b1101");
}

#[test]
fn lhs_rhs_zero_extends_to_concat_width() {
    // 4-bit unsigned RHS into 8-bit LHS zero-extends; the high nibble
    // becomes 0 (overwriting whatever the regs held before).
    let mut session = Session::new();
    session.eval("reg [3:0] a = 4'b1111").expect("decl a");
    session.eval("reg [3:0] b = 4'b1111").expect("decl b");
    assert_eq!(
        session.eval("{a, b} = 4'h5").expect("zero-extend").output,
        "8'b00000101"
    );
    assert_eq!(session.eval("a").expect("read a").output, "4'b0000");
    assert_eq!(session.eval("b").expect("read b").output, "4'b0101");
}

#[test]
fn echo_for_bare_name_lhs_uses_reg_metadata() {
    // Sign-extension and the reg's stored base (Binary, set by the decl
    // path) flow through, mirroring the pre-lvalue echo policy. -5 in
    // an 8-bit signed two's-complement is 0b11111011.
    let mut session = Session::new();
    session.eval("reg signed [7:0] r").expect("signed decl");
    assert_eq!(
        session.eval("r = -5").expect("signed assign").output,
        "8'sb11111011"
    );
}

#[test]
fn echo_for_select_lhs_uses_select_width() {
    // Select's width and the reg's base (Binary by default), not the
    // RHS's natural display form.
    let mut session = Session::new();
    session.eval("reg [7:0] r = 8'h00").expect("decl");
    assert_eq!(
        session
            .eval("r[3:0] = 4'hA")
            .expect("select-width echo")
            .output,
        "4'b1010"
    );
}

#[test]
fn echo_for_concat_lhs_uses_leftmost_base() {
    // The concat's width and the leftmost leaf's base — without
    // re-stamping, the RHS's hex base would leak into the echo.
    let mut session = Session::new();
    session.eval("reg [3:0] a = 4'h0").expect("decl a");
    session.eval("reg [3:0] b = 4'h0").expect("decl b");
    assert_eq!(
        session
            .eval("{a, b} = 8'hAB")
            .expect("concat echo")
            .output,
        "8'b10101011"
    );
}

#[test]
fn bare_concat_no_assign_still_parses_as_expression() {
    // The speculative lvalue branch must not poison the standalone-
    // concat-as-expression path: with no `=` following, the parsed
    // concat falls through to a normal expression statement.
    assert_eq!(evaluate_input("{1'b1, 1'b0}").expect("concat expr").output, "2'b10");
}

// ---------------------------------------------------------------------
// Arrays: decl-side coverage (Phase 1 of the array work).
// RHS / LHS / select-within-element behaviors land in later phases.
// ---------------------------------------------------------------------

#[test]
fn array_decl_records_dimension_and_element_count() {
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:15]").expect("decl");
    let (msb, lsb, count) = session.lookup_reg_array("a").expect("array a");
    assert_eq!((msb, lsb, count), (BigInt::from(0u8), BigInt::from(15u8), 16));
}

#[test]
fn array_decl_with_reversed_dimension_is_accepted() {
    // Reversed dimension is allowed; storage direction is private, so
    // we only assert the count and the preserved endpoints.
    let mut session = Session::new();
    session.eval("reg [3:0] a [15:0]").expect("decl");
    let (msb, lsb, count) = session.lookup_reg_array("a").expect("array a");
    assert_eq!((msb, lsb, count), (BigInt::from(15u8), BigInt::from(0u8), 16));
}

#[test]
fn array_decl_with_negative_dimension_endpoints_is_accepted() {
    let mut session = Session::new();
    session.eval("reg [3:0] a [-2:1]").expect("decl");
    let (msb, lsb, count) = session.lookup_reg_array("a").expect("array a");
    assert_eq!((msb, lsb, count), (BigInt::from(-2), BigInt::from(1u8), 4));
}

#[test]
fn array_decl_with_constant_expression_dimension_endpoints() {
    let mut session = Session::new();
    session.eval("reg [3:0] a [3+1:0]").expect("decl");
    let (msb, lsb, count) = session.lookup_reg_array("a").expect("array a");
    assert_eq!((msb, lsb, count), (BigInt::from(4u8), BigInt::from(0u8), 5));
}

#[test]
fn array_decl_rejects_init_expression() {
    // LRM A.2.2.1 variable_type splits `{ dimension }` from
    // `= constant_expression` — an array variable has no init form.
    let mut session = Session::new();
    let err = session
        .eval("reg [3:0] a [0:3] = 4'hF")
        .expect_err("init on array should fail");
    assert!(err.contains("array variable") && err.contains("init"));
}

#[test]
fn array_decl_rejects_multi_dimensional_form() {
    // Multi-dim arrays are out of scope for now; the parser pins them
    // down with a dedicated diagnostic rather than letting the second
    // `[` slide into the operand stream.
    let mut session = Session::new();
    let err = session
        .eval("reg [3:0] a [0:3][0:3]")
        .expect_err("multi-dim should fail");
    assert!(err.contains("multi-dimensional"));
}

#[test]
fn array_decl_rejects_x_or_z_dimension_endpoint() {
    let mut session = Session::new();
    let err = session
        .eval("reg [3:0] a [1'bx:0]")
        .expect_err("x dim endpoint should fail");
    assert!(err.contains("unknown bits"));
}

#[test]
fn array_decl_mixed_with_vector_in_same_list_commits_all_or_nothing() {
    // Each name in a list can independently be array or vector, and a
    // bad later name must not commit the earlier ones.
    let mut session = Session::new();
    session.eval("reg [3:0] a, b [0:1]").expect("mixed decl");
    assert!(session.lookup("a").is_some());
    let (msb, lsb, count) = session.lookup_reg_array("b").expect("array b");
    assert_eq!((msb, lsb, count), (BigInt::from(0u8), BigInt::from(1u8), 2));

    // Bad endpoint on `d` (`1'bx`) must roll back the whole statement,
    // so `c` does not appear.
    let err = session
        .eval("reg [3:0] c, d [1'bx:0]")
        .expect_err("xz dim aborts");
    assert!(err.contains("unknown bits"));
    assert!(session.lookup("c").is_none(), "c should not be bound");
    assert!(session.lookup("d").is_none(), "d should not be bound");
}

#[test]
fn array_decl_rejects_duplicate_name_in_list() {
    let mut session = Session::new();
    let err = session
        .eval("reg [3:0] a [0:3], a [0:7]")
        .expect_err("duplicate name");
    assert!(err.contains("duplicate name"));
}

#[test]
fn array_redeclaration_replaces_prior_binding_completely() {
    // The single-scope REPL convention: a redecl overwrites width,
    // dim, and bits — including converting between vector and array.
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("vector decl");
    assert!(session.lookup_reg_array("a").is_none());

    session.eval("reg [3:0] a [0:3]").expect("array redecl");
    let (msb, lsb, count) = session.lookup_reg_array("a").expect("array a");
    assert_eq!((msb, lsb, count), (BigInt::from(0u8), BigInt::from(3u8), 4));

    session.eval("reg [15:0] a").expect("vector redecl");
    assert!(session.lookup_reg_array("a").is_none());
}

#[test]
fn array_name_cannot_be_used_as_a_value() {
    // Bare array reference is illegal — there is no whole-array
    // primary in Verilog-1364. The diagnostic comes from the shared
    // `require_vector` helper.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    let err = session
        .eval("a + 1")
        .expect_err("array used as value should fail");
    assert!(err.contains("array `a`") && err.contains("cannot be used as a value"));
}

#[test]
fn array_name_cannot_be_assigned_as_a_whole() {
    // The LHS path goes through the same `require_vector` rejection
    // when the user writes `a = …` against an array name.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    let err = session
        .eval("a = 4'hF")
        .expect_err("array assigned as whole should fail");
    assert!(err.contains("array `a`"));
}

// ---------------------------------------------------------------------
// Arrays: RHS whole-element read (Phase 2 of the array work).
// Element-level writes land in Phase 4; these tests rely on the fact
// that a freshly-declared array is all-x to exercise the read path.
// ---------------------------------------------------------------------

#[test]
fn array_element_read_returns_all_x_for_fresh_decl() {
    // A freshly-declared array carries x bits in every element, just
    // like a vector reg of the same packed range. So `a[i]` returns the
    // packed-range's width worth of x.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:15]").expect("decl");
    assert_eq!(session.eval("a[5]").expect("read").output, "4'bxxxx");
}

#[test]
fn array_element_read_with_out_of_range_index_yields_all_x() {
    // LRM 4.2.1 OOB rule, generalised to the unpacked dim per 4.9: an
    // out-of-range element index returns a fresh all-x of the element
    // shape, not a panic or a wrap.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:15]").expect("decl");
    assert_eq!(session.eval("a[100]").expect("oob").output, "4'bxxxx");
}

#[test]
fn array_element_read_with_unknown_index_yields_all_x() {
    // x or z anywhere in the index defeats resolution to a single
    // element, so the result is all-x of the element shape — mirroring
    // the bit-select x/z rule.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:15]").expect("decl");
    assert_eq!(session.eval("a[1'bx]").expect("x idx").output, "4'bxxxx");
    assert_eq!(session.eval("a[1'bz]").expect("z idx").output, "4'bxxxx");
}

#[test]
fn array_element_read_with_negative_index_against_negative_dim() {
    // Dim endpoints can be negative; the index resolves under signed
    // interpretation when the index expression is signed, so a
    // negative-endpoint array indexed by a negative literal lines up.
    let mut session = Session::new();
    session.eval("reg [3:0] a [-2:1]").expect("decl");
    // -2 is in range; element width comes from the packed range.
    assert_eq!(session.eval("a[-2]").expect("neg in range").output, "4'bxxxx");
    // -3 is out of range; same all-x result, but exercises the OOB
    // branch on the lower bound.
    assert_eq!(session.eval("a[-3]").expect("neg oob").output, "4'bxxxx");
}

#[test]
fn array_element_read_rejects_part_select_on_outer_dim() {
    // The unpacked dimension has no part-select form; `a[3:0]` on an
    // array is a structural error rather than a silent reinterpretation.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:15]").expect("decl");
    let err = session
        .eval("a[3:0]")
        .expect_err("part-select on array dim should fail");
    assert!(err.contains("part-select on array `a`"));
}

#[test]
fn array_element_read_rejects_indexed_part_select_on_outer_dim() {
    // Both `+:` and `-:` are part-select forms and apply only to the
    // packed range, so the array's outer bracket rejects them too.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:15]").expect("decl");
    let up_err = session
        .eval("a[0 +: 2]")
        .expect_err("indexed +: on array dim should fail");
    assert!(up_err.contains("part-select on array `a`"));
    let down_err = session
        .eval("a[3 -: 2]")
        .expect_err("indexed -: on array dim should fail");
    assert!(down_err.contains("part-select on array `a`"));
}

#[test]
fn array_element_read_rejects_real_index() {
    // Same shape as bit-select / indexed-part-select: a real index has
    // no defined integer image at the array-element level.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:15]").expect("decl");
    let err = session
        .eval("a[1.0]")
        .expect_err("real index should fail");
    assert!(err.contains("array element index") && err.contains("real"));
}

#[test]
fn array_element_read_propagates_through_arithmetic_context() {
    // The element's shape matches a freshly-declared vector reg, so an
    // arithmetic context widens / extends it the same way. With every
    // element x, the result is all-x at the propagated width.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:15]").expect("decl");
    // 4-bit a[0] plus a 4-bit literal stays 4-bit, with x propagation
    // poisoning the whole sum.
    assert_eq!(
        session.eval("a[0] + 4'd1").expect("arith").output,
        "4'bxxxx"
    );
}

#[test]
fn array_element_read_in_concatenation_contributes_element_width() {
    // Concat width = sum of operand widths (LRM 5.1.14). Element read
    // contributes the packed-range width, so `{a[0], a[1]}` is 8 bits
    // even though both halves are x.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:15]").expect("decl");
    assert_eq!(
        session.eval("{a[0], a[1]}").expect("concat").output,
        "8'bxxxxxxxx"
    );
}

#[test]
fn array_element_read_on_one_bit_array_returns_one_bit_x() {
    // No packed range → each element is a 1-bit scalar-shaped value,
    // and `a[i]` returns that single bit (still x for a fresh decl).
    let mut session = Session::new();
    session.eval("reg a [0:7]").expect("decl");
    assert_eq!(session.eval("a[3]").expect("read").output, "1'bx");
}

#[test]
fn array_element_read_respects_packed_signedness() {
    // A `signed` packed range carries through to the element shape, so
    // the rendered element keeps the `'sb` signed-binary prefix that a
    // fresh `reg signed [3:0]` vector would also use.
    let mut session = Session::new();
    session.eval("reg signed [3:0] a [0:7]").expect("decl");
    assert_eq!(session.eval("a[2]").expect("read").output, "4'sbxxxx");
}

#[test]
fn array_element_index_is_evaluated_in_self_determined_context() {
    // Index uses an arithmetic expression: 2 + 3 → 5, and the array's
    // index 5 is in range (`reg [3:0] a [0:15]`), returning that
    // element's 4-bit value (x).
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:15]").expect("decl");
    assert_eq!(session.eval("a[2 + 3]").expect("arith idx").output, "4'bxxxx");
}

// ---------------------------------------------------------------------
// Arrays: RHS select-within-element (Phase 3 of the array work).
// `a[i][m]`, `a[i][m:l]`, `a[i][b +: w]`, `a[i][b -: w]`. Since Phase 4
// (element writes) isn't in yet, every chosen element is all-x, so the
// inner select reads x bits — but the *shape* (width, base, unsigned-ness,
// OOB partial-fill) is what these tests pin down.
// ---------------------------------------------------------------------

#[test]
fn array_chained_bit_select_returns_single_bit_x() {
    // `a[i][k]` resolves to a 1-bit unsigned read of bit k of element i.
    // With every element all-x, the bit is x.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:15]").expect("decl");
    assert_eq!(session.eval("a[5][2]").expect("chained bit").output, "1'bx");
}

#[test]
fn array_chained_const_part_select_returns_unsigned_slice() {
    // `a[i][m:l]` is a part-select against the chosen element's packed
    // range. Result is always unsigned per LRM 4.7, width = |m-l|+1,
    // base flows from the element (Binary at array decl time).
    let mut session = Session::new();
    session.eval("reg [7:0] a [0:3]").expect("decl");
    assert_eq!(
        session.eval("a[1][5:2]").expect("chained part").output,
        "4'bxxxx"
    );
}

#[test]
fn array_chained_indexed_part_select_up_and_down() {
    // Both `+:` and `-:` forms work against the element's packed range.
    // Width is the constant width half; base is the chosen element's.
    let mut session = Session::new();
    session.eval("reg [7:0] a [0:3]").expect("decl");
    assert_eq!(
        session.eval("a[0][2 +: 3]").expect("chained +:").output,
        "3'bxxx"
    );
    assert_eq!(
        session.eval("a[2][7 -: 4]").expect("chained -:").output,
        "4'bxxxx"
    );
}

#[test]
fn array_chained_inner_select_with_oob_outer_index_yields_xs() {
    // Outer index out-of-range → element fallback is all-x of the
    // packed shape; the inner select then reads x bits at the requested
    // width. Same shape as if the chosen element had been all-x.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    assert_eq!(
        session.eval("a[100][3:1]").expect("oob outer").output,
        "3'bxxx"
    );
    assert_eq!(session.eval("a[100][0]").expect("oob outer bit").output, "1'bx");
}

#[test]
fn array_chained_inner_select_with_xz_outer_index_yields_xs() {
    // x/z in the outer index defeats element resolution; the all-x
    // element fallback feeds the inner select, which still produces a
    // width matching the inner form.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:15]").expect("decl");
    assert_eq!(
        session.eval("a[1'bx][3:0]").expect("x outer").output,
        "4'bxxxx"
    );
    assert_eq!(
        session.eval("a[1'bz][0]").expect("z outer").output,
        "1'bx"
    );
}

#[test]
fn array_chained_inner_bit_select_with_oob_inner_index_yields_x() {
    // Inner bit-select OOB falls under LRM 4.2.1 → result is x. Even on
    // an all-x element the path is exercised through `resolve_reg_index`
    // returning None.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    assert_eq!(session.eval("a[0][9]").expect("inner oob").output, "1'bx");
}

#[test]
fn array_chained_inner_part_select_partially_in_range_fills_oob_with_x() {
    // LRM 4.2.1 OOB rule applies per position: an inner part-select
    // straddling the packed range fills in-range positions from the
    // element and out-of-range positions with x. Since every element bit
    // is x, the full result reads x — but the width is the requested
    // |m-l|+1.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    assert_eq!(
        session.eval("a[0][5:2]").expect("inner straddle").output,
        "4'bxxxx"
    );
}

#[test]
fn array_chained_inner_xz_bit_select_index_yields_x() {
    // x/z in the inner index → 1-bit x, same as a bit-select on a
    // vector reg. The outer element still resolves normally.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    assert_eq!(session.eval("a[0][1'bx]").expect("xz inner").output, "1'bx");
}

#[test]
fn array_chained_inner_real_bit_select_index_errors() {
    // Real indices have no defined integer image — same rejection as a
    // vector reg's bit-select.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    let err = session
        .eval("a[0][1.0]")
        .expect_err("real inner index should fail");
    assert!(err.contains("bit-select index") && err.contains("real"));
}

#[test]
fn array_chained_inner_part_select_direction_mismatch_errors() {
    // Inner part-select direction must match the element's packed range
    // direction. With `reg [3:0]` the inner select must also be
    // `[high:low]`; a reversed inner select is a structural error.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    let err = session
        .eval("a[0][0:3]")
        .expect_err("inner direction mismatch should fail");
    assert!(err.contains("part-select direction"));
}

#[test]
fn array_chained_select_on_scalar_array_element_errors() {
    // `reg a [0:7]` has scalar elements with no packed range to
    // address; the inner select is rejected with the scalar-element
    // diagnostic.
    let mut session = Session::new();
    session.eval("reg a [0:7]").expect("decl");
    let err = session
        .eval("a[0][0]")
        .expect_err("inner select on scalar array element should fail");
    assert!(err.contains("scalar array element"));
}

#[test]
fn array_chained_select_outer_part_select_errors() {
    // The outer bracket of a chained form still has to be an element
    // bit-select — outer part-selects on the array dim are rejected the
    // same way they are without an inner bracket.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    let err = session
        .eval("a[3:0][0]")
        .expect_err("outer part-select should fail");
    assert!(err.contains("part-select on array `a`"));
}

#[test]
fn chained_select_on_vector_reg_errors() {
    // A vector reg select already yields a self-determined integer
    // value with no further sub-structure to address. `a[3:0][0]` on a
    // vector reg is rejected with a clear "not an array" diagnostic.
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("decl");
    let err = session
        .eval("a[3:0][0]")
        .expect_err("chained select on vector reg should fail");
    assert!(err.contains("chained select on `a`"));
    assert!(err.contains("not an array"));
}

#[test]
fn array_chained_select_propagates_through_arithmetic_context() {
    // Same shape as a vector-reg part-select read: the inner-select
    // result widens to the propagated context. With every element x,
    // the addition poisons the whole sum at the unified width.
    let mut session = Session::new();
    session.eval("reg [7:0] a [0:3]").expect("decl");
    assert_eq!(
        session.eval("a[0][3:0] + 4'd1").expect("arith").output,
        "4'bxxxx"
    );
}

#[test]
fn array_chained_select_in_concatenation_contributes_inner_width() {
    // Concat width = sum of operand widths. Each chained-select half
    // contributes its inner-select width — 4 bits from `[3:0]` plus 2
    // bits from `[1:0]` = 6 bits.
    let mut session = Session::new();
    session.eval("reg [7:0] a [0:3]").expect("decl");
    assert_eq!(
        session.eval("{a[0][3:0], a[1][1:0]}").expect("concat").output,
        "6'bxxxxxx"
    );
}

#[test]
fn array_chained_inner_bit_select_uses_index_expression() {
    // Inner index is an arbitrary self-determined integer expression,
    // not just a literal. `1 + 2` lands at bit 3 of element 0, which is
    // x for a fresh array.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    assert_eq!(
        session.eval("a[0][1 + 2]").expect("inner expr").output,
        "1'bx"
    );
}

// ---------------------------------------------------------------------
// Arrays: LHS whole-element write (Phase 4 of the array work).
// `a[i] = expr` replaces the whole packed element at index i. Other
// elements are untouched; OOB / x-z indices echo the RHS without
// performing the write (LRM 4.2.1 + 4.9).
// ---------------------------------------------------------------------

#[test]
fn array_element_write_replaces_the_targeted_element() {
    // Basic write: `a[0] = 4'b1010` stores 4'b1010 at element 0. The
    // echoed output uses the element's shape (4-bit binary unsigned).
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    assert_eq!(
        session.eval("a[0] = 4'b1010").expect("write").output,
        "4'b1010"
    );
    assert_eq!(session.eval("a[0]").expect("readback").output, "4'b1010");
}

#[test]
fn array_element_write_leaves_other_elements_unchanged() {
    // Writing one element does not touch any other element; every other
    // position stays at the all-x decl-time state.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    session.eval("a[3] = 4'b0101").expect("write");
    assert_eq!(session.eval("a[3]").expect("written").output, "4'b0101");
    assert_eq!(session.eval("a[0]").expect("other 0").output, "4'bxxxx");
    assert_eq!(session.eval("a[7]").expect("other 7").output, "4'bxxxx");
}

#[test]
fn array_element_write_with_wider_rhs_truncates_to_element_width() {
    // RHS evaluated in element context (4-bit unsigned). A wider RHS
    // truncates to the element's width — same shape as a vector-reg
    // assignment to a 4-bit reg.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    assert_eq!(
        session.eval("a[1] = 8'b10101111").expect("trunc").output,
        "4'b1111"
    );
    assert_eq!(session.eval("a[1]").expect("readback").output, "4'b1111");
}

#[test]
fn array_element_write_with_narrower_rhs_zero_extends() {
    // Narrower RHS extends to element width. With unsigned element
    // context, extension is zero-fill — `2'b11` becomes `4'b0011`.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    assert_eq!(
        session.eval("a[0] = 2'b11").expect("ext").output,
        "4'b0011"
    );
    assert_eq!(session.eval("a[0]").expect("readback").output, "4'b0011");
}

#[test]
fn array_element_write_signed_element_sign_extends_narrow_signed_rhs() {
    // A signed element context sign-extends a signed narrower RHS:
    // `2'sb11` (signed-binary -1) widens to `4'sb1111` (still -1).
    // Element base is Binary (hardcoded at array decl time), so the
    // canonical rendering keeps the `'sb` signed-binary prefix rather
    // than collapsing to the signed-decimal form.
    let mut session = Session::new();
    session.eval("reg signed [3:0] a [0:3]").expect("decl");
    assert_eq!(
        session.eval("a[2] = 2'sb11").expect("signed").output,
        "4'sb1111"
    );
    assert_eq!(session.eval("a[2]").expect("readback").output, "4'sb1111");
}

#[test]
fn array_element_write_with_oob_index_does_not_modify_any_element() {
    // OOB index → no assignment performed, but the displayed echo still
    // shows the RHS in element shape (LRM 4.2.1).
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    assert_eq!(
        session.eval("a[100] = 4'b0001").expect("oob write").output,
        "4'b0001"
    );
    // No element should have been touched.
    assert_eq!(session.eval("a[0]").expect("e0").output, "4'bxxxx");
    assert_eq!(session.eval("a[7]").expect("e7").output, "4'bxxxx");
}

#[test]
fn array_element_write_with_xz_index_does_not_modify_any_element() {
    // x or z anywhere in the index defeats resolution; same rule as a
    // bit-select with x/z index → no assignment, but the echo stays.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    assert_eq!(
        session.eval("a[1'bx] = 4'b0001").expect("x idx").output,
        "4'b0001"
    );
    assert_eq!(
        session.eval("a[1'bz] = 4'b0010").expect("z idx").output,
        "4'b0010"
    );
    assert_eq!(session.eval("a[0]").expect("e0").output, "4'bxxxx");
}

#[test]
fn array_element_write_with_real_index_errors() {
    // Real index has no defined integer image → structural error,
    // matching the RHS read shape.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    let err = session
        .eval("a[1.0] = 4'b0001")
        .expect_err("real index should fail");
    assert!(err.contains("array element index") && err.contains("real"));
}

#[test]
fn array_element_write_rejects_part_select_on_outer_dim() {
    // `a[3:0] = ...` targets the unpacked dimension's part-select form,
    // which has no LRM meaning. Same diagnostic the RHS read uses.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    let err = session
        .eval("a[3:0] = 16'b0")
        .expect_err("outer part-select write should fail");
    assert!(err.contains("part-select on array `a`"));
}

#[test]
fn array_element_write_rejects_indexed_part_select_on_outer_dim() {
    // Both `+:` and `-:` are part-select forms on the unpacked dim.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    let up_err = session
        .eval("a[0 +: 2] = 8'b0")
        .expect_err("indexed +: write should fail");
    assert!(up_err.contains("part-select on array `a`"));
    let down_err = session
        .eval("a[3 -: 2] = 8'b0")
        .expect_err("indexed -: write should fail");
    assert!(down_err.contains("part-select on array `a`"));
}

#[test]
fn array_element_write_on_scalar_array_element_writes_one_bit() {
    // `reg a [0:7]` has 1-bit scalar elements; the element shape is
    // 1-bit unsigned, so the displayed echo is `1'b1`.
    let mut session = Session::new();
    session.eval("reg a [0:7]").expect("decl");
    assert_eq!(
        session.eval("a[3] = 1'b1").expect("scalar write").output,
        "1'b1"
    );
    assert_eq!(session.eval("a[3]").expect("readback").output, "1'b1");
    assert_eq!(session.eval("a[0]").expect("other").output, "1'bx");
}

#[test]
fn array_element_write_supports_self_reference() {
    // `a[0] = a[0] + 4'd1`: the RHS reads the prior element value, the
    // LHS replaces it with the result. Reading-then-writing the same
    // element is the standard increment idiom.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    session.eval("a[0] = 4'b0011").expect("init");
    assert_eq!(
        session.eval("a[0] = a[0] + 4'd1").expect("self").output,
        "4'b0100"
    );
    assert_eq!(session.eval("a[0]").expect("readback").output, "4'b0100");
}

#[test]
fn array_element_write_supports_cross_element_read() {
    // RHS may reference any other element; the write goes to the LHS
    // element regardless of what the RHS read.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    session.eval("a[1] = 4'b1100").expect("init");
    assert_eq!(
        session.eval("a[2] = a[1]").expect("cross").output,
        "4'b1100"
    );
    assert_eq!(session.eval("a[2]").expect("readback").output, "4'b1100");
    assert_eq!(session.eval("a[1]").expect("source unchanged").output, "4'b1100");
}

#[test]
fn array_element_write_with_real_rhs_rounds_to_integer() {
    // Real RHS implicitly converts per LRM §3.5.3 (round half away from
    // zero), then narrows to the element width. `1.5` rounds to 2,
    // which is `4'b0010` in a 4-bit unsigned element.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    assert_eq!(
        session.eval("a[0] = 1.5").expect("real rhs").output,
        "4'b0010"
    );
    assert_eq!(session.eval("a[0]").expect("readback").output, "4'b0010");
}

#[test]
fn array_element_write_rejects_array_name_as_rhs() {
    // Defense-in-depth: the array's bare name still cannot appear as a
    // value, so `a[0] = a` is rejected — the RHS evaluation surfaces
    // the same "array `a` cannot be used as a value" error.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    let err = session
        .eval("a[0] = a")
        .expect_err("array bare name as RHS should fail");
    assert!(err.contains("array `a` cannot be used as a value"));
}

#[test]
fn array_element_write_inside_lvalue_concat_distributes_bits() {
    // Phase 5: an array element appearing as a concat leaf is valid
    // and distributes the RHS bit stream MSB-first per the LRM. With
    // the concat `{a[0], b} = 8'b00001111`, `a[0]` takes the top
    // nibble (`0000`) and `b` takes the bottom nibble (`1111`).
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    session.eval("reg [3:0] b").expect("decl b");
    assert_eq!(
        session
            .eval("{a[0], b} = 8'b00001111")
            .expect("concat write")
            .output,
        "8'b00001111"
    );
    assert_eq!(session.eval("a[0]").expect("a[0]").output, "4'b0000");
    assert_eq!(session.eval("b").expect("b").output, "4'b1111");
    // Neighbouring elements stay untouched.
    assert_eq!(session.eval("a[1]").expect("a[1]").output, "4'bxxxx");
}

#[test]
fn array_element_write_against_reversed_unpacked_dim() {
    // Reversed unpacked dim (`[15:0]`) resolves the index the same way
    // the RHS read path does. Index 0 still names a valid element; the
    // write succeeds.
    let mut session = Session::new();
    session.eval("reg [3:0] a [15:0]").expect("decl");
    assert_eq!(
        session.eval("a[0] = 4'b1010").expect("write").output,
        "4'b1010"
    );
    assert_eq!(session.eval("a[0]").expect("readback").output, "4'b1010");
    assert_eq!(session.eval("a[15]").expect("other").output, "4'bxxxx");
}

#[test]
fn array_element_write_against_negative_endpoint_unpacked_dim() {
    // Negative dim endpoints (`[-2:1]`) are accepted by the decl path;
    // the write path resolves a negative index against the dim the same
    // way the RHS read does.
    let mut session = Session::new();
    session.eval("reg [3:0] a [-2:1]").expect("decl");
    assert_eq!(
        session.eval("a[-2] = 4'b1001").expect("neg write").output,
        "4'b1001"
    );
    assert_eq!(session.eval("a[-2]").expect("readback").output, "4'b1001");
    // OOB write on the lower side leaves the previously-written value
    // untouched.
    session.eval("a[-3] = 4'b0000").expect("oob");
    assert_eq!(session.eval("a[-2]").expect("still").output, "4'b1001");
}

#[test]
fn array_element_write_atomicity_failed_assignment_leaves_state_intact() {
    // A structural error (part-select on outer dim) must leave the
    // session map untouched — the same all-or-nothing commit the decl
    // path establishes.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    session.eval("a[0] = 4'b1010").expect("write");
    let err = session
        .eval("a[3:0] = 16'b0")
        .expect_err("structural error");
    assert!(err.contains("part-select on array `a`"));
    // The pre-error state must be intact.
    assert_eq!(session.eval("a[0]").expect("preserved").output, "4'b1010");
    assert_eq!(session.eval("a[1]").expect("preserved").output, "4'bxxxx");
}

#[test]
fn array_element_write_with_arithmetic_index_expression() {
    // The index is a self-determined integer expression, so `2 + 3`
    // lands at element 5. The write must hit that element specifically.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    assert_eq!(
        session.eval("a[2 + 3] = 4'b1111").expect("arith idx").output,
        "4'b1111"
    );
    assert_eq!(session.eval("a[5]").expect("readback").output, "4'b1111");
    assert_eq!(session.eval("a[4]").expect("neighbour").output, "4'bxxxx");
    assert_eq!(session.eval("a[6]").expect("neighbour").output, "4'bxxxx");
}

// ---------------------------------------------------------------------
// Phase 5: LHS select-within-element + concat leaves containing array
// elements. LRM 4.9 + 5.2.1/5.2.2: chained `a[i][m:l]` LHS uses the
// inner select's width/base for the assignment context (unsigned per
// 4.7); inner select runs against the chosen element's packed range.
// ---------------------------------------------------------------------

#[test]
fn array_element_bit_select_lhs_writes_only_the_named_bit() {
    // `a[i][n] = expr` distributes a single bit into position `n` of
    // the chosen element, leaving the other bits intact.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    session.eval("a[1] = 4'b1010").expect("seed element");
    // Echo prints in 1-bit unsigned binary context (the inner select's
    // self-determined shape).
    assert_eq!(
        session.eval("a[1][0] = 1'b1").expect("write bit").output,
        "1'b1"
    );
    assert_eq!(session.eval("a[1]").expect("readback").output, "4'b1011");
    // Other elements untouched.
    assert_eq!(session.eval("a[0]").expect("a[0]").output, "4'bxxxx");
}

#[test]
fn array_element_part_select_lhs_writes_only_the_named_slice() {
    // `a[i][m:l] = expr` distributes the slice's bits into positions
    // [m:l] of the chosen element, leaving the rest intact. Echo shape
    // matches the inner select's width / unsigned.
    let mut session = Session::new();
    session.eval("reg [7:0] a [0:3]").expect("decl");
    session.eval("a[2] = 8'b00000000").expect("seed");
    assert_eq!(
        session
            .eval("a[2][5:2] = 4'b1011")
            .expect("write slice")
            .output,
        "4'b1011"
    );
    // Bits [5:2] become 1011, others stay 0.
    assert_eq!(session.eval("a[2]").expect("readback").output, "8'b00101100");
}

#[test]
fn array_element_indexed_part_select_lhs_writes_the_slice() {
    // Indexed part-select on the inner addresses three bits starting
    // at position 2 going up — bits [4:2].
    let mut session = Session::new();
    session.eval("reg [7:0] a [0:3]").expect("decl");
    session.eval("a[0] = 8'b00000000").expect("seed");
    assert_eq!(
        session
            .eval("a[0][2 +: 3] = 3'b111")
            .expect("indexed up")
            .output,
        "3'b111"
    );
    assert_eq!(session.eval("a[0]").expect("readback").output, "8'b00011100");
}

#[test]
fn array_element_inner_part_select_with_xz_outer_index_drops_write() {
    // Outer index x/z → "no assignment performed" for the whole leaf,
    // but the echo still shows the inner-shape RHS.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    session.eval("a[0] = 4'b1010").expect("seed");
    assert_eq!(
        session
            .eval("a[1'bx][2:0] = 3'b111")
            .expect("xz outer")
            .output,
        "3'b111"
    );
    assert_eq!(session.eval("a[0]").expect("untouched").output, "4'b1010");
    assert_eq!(session.eval("a[1]").expect("untouched").output, "4'bxxxx");
}

#[test]
fn array_element_inner_part_select_with_oob_outer_index_drops_write() {
    // Outer index OOB → no element receives the write; readback shows
    // the seeded values are intact.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    session.eval("a[0] = 4'b1010").expect("seed");
    assert_eq!(
        session
            .eval("a[42][2:0] = 3'b111")
            .expect("oob outer")
            .output,
        "3'b111"
    );
    assert_eq!(session.eval("a[0]").expect("untouched").output, "4'b1010");
}

#[test]
fn array_element_inner_bit_select_with_xz_inner_index_drops_just_that_bit() {
    // Inner x/z bit-select index → that one bit drops (LRM 4.2.1), but
    // the surrounding element is otherwise untouched. The bit-cursor
    // still advances so an echo of the RHS is produced.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    session.eval("a[0] = 4'b1010").expect("seed");
    assert_eq!(
        session
            .eval("a[0][1'bx] = 1'b1")
            .expect("xz inner index")
            .output,
        "1'b1"
    );
    // Element untouched because the only bit being written was
    // dropped.
    assert_eq!(session.eval("a[0]").expect("untouched").output, "4'b1010");
}

#[test]
fn array_element_inner_part_select_oob_drops_only_out_of_range_bits() {
    // Inner part-select that runs off the high end of the packed range
    // drops only the OOB positions; in-range positions still get
    // written.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    session.eval("a[0] = 4'b0000").expect("seed");
    assert_eq!(
        session
            .eval("a[0][5:2] = 4'b1111")
            .expect("partial oob")
            .output,
        "4'b1111"
    );
    // Positions [3:2] are in-range and become 1; positions 4 and 5 are
    // OOB and silently drop.
    assert_eq!(session.eval("a[0]").expect("readback").output, "4'b1100");
}

#[test]
fn array_element_chained_select_rejects_reversed_part_direction() {
    // Inner part-select direction must match the element's packed
    // range; structural error wins over RHS evaluation.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    let err = session
        .eval("a[0][0:3] = 4'b1111")
        .expect_err("direction mismatch");
    assert!(err.contains("part-select direction does not match"));
    // Session untouched.
    assert_eq!(session.eval("a[0]").expect("untouched").output, "4'bxxxx");
}

#[test]
fn array_element_chained_select_rejects_real_inner_bit_index() {
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    let err = session
        .eval("a[0][1.0] = 1'b1")
        .expect_err("real inner index");
    assert!(err.contains("bit-select index cannot be real"));
}

#[test]
fn array_element_chained_select_rejects_real_outer_bit_index() {
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    let err = session
        .eval("a[1.0][0] = 1'b1")
        .expect_err("real outer index");
    assert!(err.contains("array element index cannot be real"));
}

#[test]
fn scalar_array_element_rejects_inner_select_on_lhs() {
    // `reg a [0:3]` has no packed range → bit-select on the element is
    // illegal (LRM 5.2.1 scalar-reg rule), mirroring the RHS-path
    // diagnostic.
    let mut session = Session::new();
    session.eval("reg a [0:3]").expect("decl");
    let err = session
        .eval("a[0][0] = 1'b1")
        .expect_err("scalar element");
    assert!(err.contains("scalar array element `a`"));
}

#[test]
fn array_element_chained_select_rejects_part_outer_select() {
    // The outer select on an array must be a `Bit`; using a part-select
    // is rejected with the array-element diagnostic.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    let err = session
        .eval("a[3:0][1:0] = 2'b11")
        .expect_err("part outer");
    assert!(err.contains("part-select on array `a`"));
}

#[test]
fn array_element_lhs_concat_with_inner_select_distributes() {
    // Concat mixing a vector leaf, an array-element inner-select, and
    // a bare array element. RHS bits flow right-to-left:
    //   {b, a[0][2:0], a[1]} = 11'b10110010110
    // ^ MSB end of RHS                LSB end ^
    //   b           = 4'b1011  (top 4 bits)
    //   a[0][2:0]   = 3'b001   (next 3 bits)
    //   a[1]        = 4'b0110  (bottom 4 bits)
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    session.eval("reg [3:0] b").expect("decl b");
    session.eval("a[0] = 4'b0000").expect("seed");
    assert_eq!(
        session
            .eval("{b, a[0][2:0], a[1]} = 11'b10110010110")
            .expect("concat write")
            .output,
        "11'b10110010110"
    );
    assert_eq!(session.eval("b").expect("b").output, "4'b1011");
    // Element a[0]: only bits [2:0] were touched, becoming 001.
    assert_eq!(session.eval("a[0]").expect("a[0]").output, "4'b0001");
    assert_eq!(session.eval("a[1]").expect("a[1]").output, "4'b0110");
}

#[test]
fn array_element_lhs_concat_with_two_array_element_leaves() {
    // Two different array elements as concat leaves both get their
    // share of the RHS bit stream.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl a");
    session.eval("reg [3:0] c [0:3]").expect("decl c");
    assert_eq!(
        session
            .eval("{a[0], c[1]} = 8'b11110000")
            .expect("two elements")
            .output,
        "8'b11110000"
    );
    assert_eq!(session.eval("a[0]").expect("a[0]").output, "4'b1111");
    assert_eq!(session.eval("c[1]").expect("c[1]").output, "4'b0000");
}

#[test]
fn array_element_lhs_concat_xz_index_drops_element_but_cursor_advances() {
    // When an array-element leaf in a concat LHS has an x/z outer index,
    // LRM 4.2.1 says "no assignment performed" — but the bit cursor must
    // still advance by the leaf's nominal width so adjacent leaves receive
    // the correct bits. Here `{a[1'bx], b} = 8'b11110000`: a[x] is
    // dropped (4 bits consumed silently), and `b` receives the low nibble.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl a");
    session.eval("reg [3:0] b").expect("decl b");
    session.eval("a[0] = 4'b1010").expect("seed a[0]");
    assert_eq!(
        session
            .eval("{a[1'bx], b} = 8'b11110000")
            .expect("concat with x-index")
            .output,
        "8'b11110000"
    );
    // b receives the low nibble correctly despite the dropped leaf.
    assert_eq!(session.eval("b").expect("b").output, "4'b0000");
    // a[0] is untouched (the x-index doesn't accidentally hit it).
    assert_eq!(session.eval("a[0]").expect("a[0] preserved").output, "4'b1010");
    // All other array elements remain at their default x.
    assert_eq!(session.eval("a[1]").expect("a[1]").output, "4'bxxxx");
    assert_eq!(session.eval("a[2]").expect("a[2]").output, "4'bxxxx");
}

#[test]
fn array_element_lhs_concat_atomic_failure_leaves_state_intact() {
    // A structural error on one concat leaf (here: chained select on a
    // non-array vector) must abort the whole assignment — even though
    // the array-element leaf would have been writable, no writes are
    // committed.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl a");
    session.eval("reg [3:0] b").expect("decl b");
    session.eval("a[0] = 4'b1010").expect("seed");
    let err = session
        .eval("{a[1], b[0][0]} = 5'b11111")
        .expect_err("chained on non-array");
    assert!(err.contains("chained select on `b`"));
    // a[0] preserved (was seeded), a[1] untouched.
    assert_eq!(session.eval("a[0]").expect("preserved").output, "4'b1010");
    assert_eq!(session.eval("a[1]").expect("preserved").output, "4'bxxxx");
    assert_eq!(session.eval("b").expect("preserved").output, "4'bxxxx");
}

#[test]
fn array_element_chained_inner_select_echo_uses_inner_width_and_unsigned() {
    // The echo for `a[i][m:l] = expr` uses the inner select's shape:
    // width = m - l + 1, signed = false (LRM 4.7), base inherited from
    // the element (Binary by decl-time hardcoding).
    let mut session = Session::new();
    session.eval("reg [7:0] a [0:3]").expect("decl");
    // RHS literal is signed decimal -1 (8'sd255 truncated to 5 bits =
    // 5'b11111). Echo is unsigned 5-bit binary.
    assert_eq!(
        session
            .eval("a[0][4:0] = -1")
            .expect("signed rhs")
            .output,
        "5'b11111"
    );
}

#[test]
fn array_element_chained_select_self_reference_reads_old_value() {
    // `a[0][3:0] = a[0]` — RHS reads the pre-assignment value of
    // a[0], which is all-x; bits land into [3:0] of a[0].
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    session.eval("a[0] = 4'b1010").expect("seed");
    assert_eq!(
        session
            .eval("a[0][3:0] = a[0]")
            .expect("self-ref")
            .output,
        "4'b1010"
    );
    assert_eq!(session.eval("a[0]").expect("readback").output, "4'b1010");
}

#[test]
fn array_element_write_rhs_error_wins_over_outer_index_xz() {
    // Even with an x/z outer index (which would drop the write
    // silently), an RHS error (here: undeclared identifier) takes
    // precedence — matching the Phase 4 precedence rule.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    let err = session
        .eval("a[1'bx] = nope")
        .expect_err("rhs error wins");
    assert!(err.contains("undeclared identifier: nope"));
}

#[test]
fn array_element_lhs_concat_rejects_array_bare_name_leaf() {
    // A bare array name as a concat leaf is still rejected — it's
    // unreadable in any context, including LHS, because there is no
    // way to address all elements in one shot.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    session.eval("reg [3:0] b").expect("decl b");
    let err = session
        .eval("{a, b} = 20'b0")
        .expect_err("bare array leaf");
    assert!(err.contains("array `a` cannot be used as a value"));
}

// ----- `integer` keyword (LRM 4.8) -----
// An `integer` reg is a signed 32-bit decimal-default vector. The
// shared apply_decl / apply_assign paths handle width/sign/base flow
// once the decl is materialized, so the integer-specific tests focus
// on the declaration-level invariants: the implicit signed 32-bit
// shape, the decimal base, the x-default, and parser-level rejection
// of the modifiers that don't apply (`signed`, packed range, unpacked
// dim).

#[test]
fn integer_decl_without_init_defaults_to_signed_32_bit_x() {
    let mut session = Session::new();
    assert!(session.eval("integer i").expect("decl").output.is_empty());
    assert_eq!(session.eval("i").expect("read").output, "32'sdx");
}

#[test]
fn integer_decl_with_init_stores_decimal_value() {
    let mut session = Session::new();
    session.eval("integer i = 5").expect("decl with init");
    assert_eq!(session.eval("i").expect("read").output, "32'sd5");
}

#[test]
fn integer_decl_with_negative_init_sign_extends_to_32_bits() {
    let mut session = Session::new();
    session.eval("integer i = -1").expect("decl");
    assert_eq!(session.eval("i").expect("read").output, "-32'sd1");
}

#[test]
fn integer_decl_with_real_init_rounds_per_lrm_3_5_3() {
    let mut session = Session::new();
    session.eval("integer i = 1.5").expect("ties away from zero");
    assert_eq!(session.eval("i").expect("read").output, "32'sd2");
    let mut session = Session::new();
    session.eval("integer i = -2.5").expect("negative ties away from zero");
    assert_eq!(session.eval("i").expect("read").output, "-32'sd3");
}

#[test]
fn integer_decl_with_nan_init_fills_with_x_bits() {
    let mut session = Session::new();
    session.eval("integer i = 0.0/0.0").expect("NaN init");
    assert_eq!(session.eval("i").expect("read").output, "32'sdx");
}

#[test]
fn integer_decl_bit_select_reads_low_bits() {
    let mut session = Session::new();
    session.eval("integer i = 5").expect("decl");
    assert_eq!(session.eval("i[0]").expect("bit 0").output, "1'd1");
    assert_eq!(session.eval("i[1]").expect("bit 1").output, "1'd0");
    assert_eq!(session.eval("i[2]").expect("bit 2").output, "1'd1");
}

#[test]
fn integer_decl_part_select_reads_low_nibble() {
    let mut session = Session::new();
    session.eval("integer i = 5").expect("decl");
    // Decimal base on the integer flows through to the part-select
    // result; the bit pattern 0101 prints as `4'd5`.
    assert_eq!(session.eval("i[3:0]").expect("part").output, "4'd5");
}

#[test]
fn integer_decl_multiple_names_in_one_statement() {
    let mut session = Session::new();
    session
        .eval("integer i = 1, j = 2, k")
        .expect("multi-name decl");
    assert_eq!(session.eval("i").expect("read i").output, "32'sd1");
    assert_eq!(session.eval("j").expect("read j").output, "32'sd2");
    assert_eq!(session.eval("k").expect("read k").output, "32'sdx");
}

#[test]
fn integer_decl_later_name_sees_earlier_binding_in_same_statement() {
    let mut session = Session::new();
    session
        .eval("integer i = 1, j = i + 1")
        .expect("self-reference");
    assert_eq!(session.eval("j").expect("read j").output, "32'sd2");
}

#[test]
fn integer_decl_rejects_signed_qualifier() {
    let mut session = Session::new();
    let err = session.eval("integer signed i").expect_err("signed banned");
    assert!(err.contains("signed"));
    assert!(err.contains("integer"));
}

#[test]
fn integer_decl_rejects_packed_range() {
    let mut session = Session::new();
    let err = session.eval("integer [3:0] i").expect_err("range banned");
    assert!(err.contains("packed"));
}

#[test]
fn integer_decl_accepts_single_unpacked_dimension() {
    // LRM A.2.2.1 `variable_type ::= variable_identifier { dimension }`
    // — `integer a [0:3]` is a 1-D unpacked array of integers, exactly
    // like the analogous `reg signed [31:0] a [0:3]` form.
    let mut session = Session::new();
    session.eval("integer a [0:3]").expect("array decl");
    let (msb, lsb, count) = session.lookup_reg_array("a").expect("array a");
    assert_eq!((msb, lsb, count), (BigInt::from(0u8), BigInt::from(3u8), 4));
}

#[test]
fn integer_decl_rejects_multi_dimensional_form() {
    // Multi-dim arrays are out of scope (same as `reg`), even though
    // the LRM permits them — the parser rejects the second `[` slot.
    let mut session = Session::new();
    let err = session
        .eval("integer a [0:3][0:3]")
        .expect_err("multi-dim should fail");
    assert!(err.contains("multi-dimensional"));
}

#[test]
fn integer_array_decl_rejects_init_expression() {
    // LRM A.2.2.1: an array variable has no init form (the grammar
    // splits `{ dimension }` from `= constant_expression`).
    let mut session = Session::new();
    let err = session
        .eval("integer a [0:3] = 5")
        .expect_err("init on array should fail");
    assert!(err.contains("array variable") && err.contains("init"));
}

#[test]
fn integer_array_element_read_returns_signed_32_bit_x_for_fresh_decl() {
    // Every element shares the integer-element template (signed 32-bit
    // decimal, all-x at decl time), so `a[i]` returns the same x-bits
    // form as a bare `integer i` would.
    let mut session = Session::new();
    session.eval("integer a [0:3]").expect("decl");
    assert_eq!(session.eval("a[0]").expect("read").output, "32'sdx");
}

#[test]
fn integer_array_element_write_updates_chosen_slot() {
    let mut session = Session::new();
    session.eval("integer a [0:3]").expect("decl");
    session.eval("a[1] = 42").expect("write");
    assert_eq!(session.eval("a[0]").expect("untouched").output, "32'sdx");
    assert_eq!(session.eval("a[1]").expect("written").output, "32'sd42");
    assert_eq!(session.eval("a[2]").expect("untouched").output, "32'sdx");
}

#[test]
fn integer_keyword_rejected_as_variable_name() {
    let mut session = Session::new();
    let err = session.eval("integer integer").expect_err("name banned");
    assert!(err.contains("integer"));
}

// ----- `real` keyword (LRM 4.8) -----
// A `real` reg has no width / sign / base — it's an IEEE 754 binary64
// slot. The default is 0.0 (not x), arithmetic flows through the f64
// pipeline, and a real LHS dispatches through `apply_real_assign`
// rather than the integer-context evaluator.

#[test]
fn real_decl_without_init_defaults_to_zero() {
    let mut session = Session::new();
    assert!(session.eval("real r").expect("decl").output.is_empty());
    assert_eq!(session.eval("r").expect("read").output, "0.0");
}

#[test]
fn real_decl_with_real_init_stores_value() {
    let mut session = Session::new();
    session.eval("real r = 1.5").expect("decl with init");
    assert_eq!(session.eval("r").expect("read").output, "1.5");
}

#[test]
fn real_decl_with_integer_init_promotes_to_real() {
    let mut session = Session::new();
    session.eval("real r = 5").expect("integer init promotes");
    assert_eq!(session.eval("r").expect("read").output, "5.0");
}

#[test]
fn real_decl_with_nan_init_stores_nan() {
    let mut session = Session::new();
    session.eval("real r = 0.0/0.0").expect("NaN init");
    assert_eq!(session.eval("r").expect("read").output, "NaN");
}

#[test]
fn real_assignment_overwrites_stored_value() {
    let mut session = Session::new();
    session.eval("real r = 1.5").expect("decl");
    session.eval("r = 2.5").expect("assign");
    assert_eq!(session.eval("r").expect("read").output, "2.5");
}

#[test]
fn real_assignment_promotes_integer_rhs() {
    let mut session = Session::new();
    session.eval("real r").expect("decl");
    session.eval("r = 3").expect("integer rhs");
    assert_eq!(session.eval("r").expect("read").output, "3.0");
}

#[test]
fn real_value_participates_in_real_arithmetic() {
    let mut session = Session::new();
    session.eval("real r = 2.5").expect("decl");
    assert_eq!(session.eval("r * 2").expect("mul").output, "5.0");
    assert_eq!(session.eval("r + 0.5").expect("add").output, "3.0");
}

#[test]
fn real_value_passed_to_real_math_function() {
    let mut session = Session::new();
    session.eval("real r = 4.0").expect("decl");
    assert_eq!(session.eval("$sqrt(r)").expect("sqrt").output, "2.0");
}

#[test]
fn real_to_integer_assignment_rounds_per_lrm_3_5_3() {
    let mut session = Session::new();
    session.eval("real r = 1.5").expect("real decl");
    session.eval("reg [7:0] a").expect("integer reg");
    session.eval("a = r").expect("assign real to integer reg");
    assert_eq!(session.eval("a").expect("read").output, "8'b00000010");
}

#[test]
fn integer_to_real_assignment_promotes_to_f64() {
    let mut session = Session::new();
    session.eval("reg [7:0] a = 5").expect("integer reg");
    session.eval("real r").expect("real decl");
    session.eval("r = a").expect("assign");
    assert_eq!(session.eval("r").expect("read").output, "5.0");
}

#[test]
fn real_decl_rejects_signed_qualifier() {
    let mut session = Session::new();
    let err = session.eval("real signed r").expect_err("signed banned");
    assert!(err.contains("signed"));
    assert!(err.contains("real"));
}

#[test]
fn real_decl_rejects_packed_range() {
    let mut session = Session::new();
    let err = session.eval("real [3:0] r").expect_err("range banned");
    assert!(err.contains("packed"));
}

#[test]
fn real_decl_accepts_single_unpacked_dimension() {
    // LRM A.2.2.1 `real_type ::= real_identifier { dimension }` —
    // `real r [0:3]` is a 1-D unpacked array of f64s. Elements default
    // to 0.0 (LRM 4.8 init value), not x; we don't expose the slice
    // directly but `lookup_reg_real_array` confirms the shape.
    let mut session = Session::new();
    session.eval("real r [0:3]").expect("array decl");
    let (msb, lsb, count) = session.lookup_reg_real_array("r").expect("real array r");
    assert_eq!((msb, lsb, count), (BigInt::from(0u8), BigInt::from(3u8), 4));
}

#[test]
fn real_decl_rejects_multi_dimensional_form() {
    let mut session = Session::new();
    let err = session
        .eval("real r [0:3][0:3]")
        .expect_err("multi-dim should fail");
    assert!(err.contains("multi-dimensional"));
}

#[test]
fn real_array_decl_rejects_init_expression() {
    let mut session = Session::new();
    let err = session
        .eval("real r [0:3] = 1.5")
        .expect_err("init on array should fail");
    assert!(err.contains("array variable") && err.contains("init"));
}

#[test]
fn real_array_element_read_defaults_to_zero() {
    // LRM 4.8 reals default to 0.0; no x state for a real slot.
    let mut session = Session::new();
    session.eval("real r [0:3]").expect("decl");
    assert_eq!(session.eval("r[0]").expect("read").output, "0.0");
}

#[test]
fn real_array_element_write_updates_chosen_slot() {
    let mut session = Session::new();
    session.eval("real r [0:3]").expect("decl");
    session.eval("r[2] = 3.25").expect("write");
    assert_eq!(session.eval("r[0]").expect("untouched").output, "0.0");
    assert_eq!(session.eval("r[2]").expect("written").output, "3.25");
}

#[test]
fn real_array_element_write_promotes_integer_rhs() {
    // §3.5.3 / §5.1.7: integer RHS converts to f64.
    let mut session = Session::new();
    session.eval("real r [0:3]").expect("decl");
    session.eval("r[1] = 7").expect("integer rhs");
    assert_eq!(session.eval("r[1]").expect("read").output, "7.0");
}

#[test]
fn real_array_element_oob_read_returns_zero() {
    // No x state for reals, so OOB falls back to the LRM 4.8 init value.
    let mut session = Session::new();
    session.eval("real r [0:3]").expect("decl");
    assert_eq!(session.eval("r[10]").expect("oob").output, "0.0");
}

#[test]
fn real_array_element_oob_write_is_dropped_silently() {
    // LRM 4.2.1 OOB writes are dropped; the in-range slot is untouched
    // and the RHS still echoes (mirrors `apply_real_assign`'s echo).
    let mut session = Session::new();
    session.eval("real r [0:3]").expect("decl");
    session.eval("r[1] = 2.5").expect("in-range write");
    let echo = session.eval("r[10] = 9.0").expect("oob write echoes rhs");
    assert_eq!(echo.output, "9.0");
    assert_eq!(session.eval("r[1]").expect("untouched").output, "2.5");
}

#[test]
fn real_array_element_xz_index_write_is_dropped_silently() {
    let mut session = Session::new();
    session.eval("real r [0:3]").expect("decl");
    session.eval("r[1] = 2.5").expect("in-range write");
    let echo = session
        .eval("r[1'bx] = 9.0")
        .expect("xz index write echoes rhs");
    assert_eq!(echo.output, "9.0");
    assert_eq!(session.eval("r[1]").expect("untouched").output, "2.5");
}

#[test]
fn real_array_element_read_in_real_arithmetic() {
    let mut session = Session::new();
    session.eval("real r [0:3]").expect("decl");
    session.eval("r[0] = 1.5").expect("write");
    assert_eq!(session.eval("r[0] + 0.5").expect("arith").output, "2.0");
    assert_eq!(session.eval("$sqrt(r[0] + 2.5)").expect("sqrt").output, "2.0");
}

#[test]
fn real_array_rejects_part_select_on_lhs() {
    let mut session = Session::new();
    session.eval("real r [0:3]").expect("decl");
    let err = session.eval("r[1:0] = 1.0").expect_err("part-select banned");
    assert!(err.contains("part-select on array `r`"));
}

#[test]
fn real_array_rejects_chained_inner_select_on_lhs() {
    let mut session = Session::new();
    session.eval("real r [0:3]").expect("decl");
    let err = session
        .eval("r[0][1:0] = 1.0")
        .expect_err("chained inner banned");
    assert!(err.contains("real-array element `r`"));
}

#[test]
fn real_array_rejects_real_index_on_lhs() {
    let mut session = Session::new();
    session.eval("real r [0:3]").expect("decl");
    let err = session
        .eval("r[1.0] = 1.0")
        .expect_err("real index banned");
    assert!(err.contains("array element index cannot be real"));
}

#[test]
fn real_array_name_cannot_be_assigned_as_a_whole() {
    let mut session = Session::new();
    session.eval("real r [0:3]").expect("decl");
    let err = session
        .eval("r = 1.0")
        .expect_err("whole-array assignment banned");
    assert!(err.contains("array `r`"));
}

#[test]
fn real_array_element_rejected_in_concat_lvalue() {
    // The real-array element is f64-typed, so it can't appear inside a
    // bit-based concat lvalue. The validator catches it before any
    // staged write runs.
    let mut session = Session::new();
    session.eval("reg [3:0] v").expect("vector decl");
    session.eval("real r [0:3]").expect("real array decl");
    let err = session
        .eval("{v, r[0]} = 8'h00")
        .expect_err("real-array in concat lvalue banned");
    assert!(err.contains("real-array element `r[..]`"));
}

#[test]
fn real_keyword_rejected_as_variable_name() {
    let mut session = Session::new();
    let err = session.eval("real real").expect_err("name banned");
    assert!(err.contains("real"));
}

#[test]
fn reg_keyword_rejected_as_variable_name_in_integer_decl() {
    let mut session = Session::new();
    let err = session
        .eval("integer reg")
        .expect_err("reserved word banned");
    assert!(err.contains("reg"));
}

#[test]
fn real_reg_rejected_in_bit_select() {
    // LRM 4.8.1: "Bit-select or part-select references of variables
    // declared as real … is prohibited." The validator catches it.
    let mut session = Session::new();
    session.eval("real r = 1.5").expect("decl");
    let err = session.eval("r[0]").expect_err("bit-select banned");
    assert_eq!(
        err,
        "Semantic error: bit-select or part-select on real variable `r` is not allowed"
    );
}

#[test]
fn real_reg_rejected_in_part_select() {
    // Same LRM 4.8.1 rule applies to part-selects on a scalar real.
    let mut session = Session::new();
    session.eval("real r = 1.5").expect("decl");
    let err = session.eval("r[1:0]").expect_err("part-select banned");
    assert_eq!(
        err,
        "Semantic error: bit-select or part-select on real variable `r` is not allowed"
    );
}

#[test]
fn real_reg_rejected_in_lhs_bit_select() {
    // LRM 4.8.1 applies to the LHS path as well — `r[0] = 1` is
    // prohibited when `r` is a scalar `real`.
    let mut session = Session::new();
    session.eval("real r = 1.5").expect("decl");
    let err = session.eval("r[0] = 1").expect_err("lhs select banned");
    assert_eq!(
        err,
        "Semantic error: bit-select or part-select on real variable `r` is not allowed"
    );
}

#[test]
fn real_reg_storage_round_trip() {
    // Cross-check through the test helper — the f64 stored in the
    // session matches what we read back.
    let mut session = Session::new();
    session.eval("real r = 2.5").expect("decl");
    assert_eq!(session.lookup_reg_real("r"), Some(2.5));
    session.eval("r = -1.25").expect("reassign");
    assert_eq!(session.lookup_reg_real("r"), Some(-1.25));
}

#[test]
fn integer_decl_self_reference_reads_prior_binding() {
    // Like `reg`: the init of a redeclared name sees the prior
    // binding, not the new (still-uninitialized) slot.
    let mut session = Session::new();
    session.eval("integer i = 7").expect("first decl");
    session.eval("integer i = i + 1").expect("redecl reads prior");
    assert_eq!(session.eval("i").expect("read").output, "32'sd8");
}

#[test]
fn integer_decl_failed_init_leaves_session_untouched() {
    // All-or-nothing commit: a malformed second init aborts the whole
    // decl, so the first name does not appear in the session.
    let mut session = Session::new();
    let err = session
        .eval("integer i = 1, j = nope")
        .expect_err("rhs error rolls back");
    assert!(err.contains("nope"));
    let err = session.eval("i").expect_err("i never committed");
    assert!(err.contains("undeclared"));
}

#[test]
fn real_decl_failed_init_leaves_session_untouched() {
    let mut session = Session::new();
    let err = session
        .eval("real r = 1.5, s = nope")
        .expect_err("rhs error rolls back");
    assert!(err.contains("nope"));
    let err = session.eval("r").expect_err("r never committed");
    assert!(err.contains("undeclared"));
}
