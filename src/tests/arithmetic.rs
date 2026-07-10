use crate::evaluate_input;

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
    let even = evaluate_input("(-4'sd1) ** 2").expect("even negative-base power should evaluate");
    let reciprocal = evaluate_input("(-4'sd1) ** -3").expect("negative exponent should evaluate");

    assert_eq!(odd.output, "-4'sd1");
    assert_eq!(even.output, "4'sd1");
    assert_eq!(reciprocal.output, "-4'sd1");
}

#[test]
fn power_with_huge_exponent_truncates_to_result_width() {
    // LRM Table 5-3: `**` result width is L(base). We compute
    // base ** exp mod 2^width via modular exponentiation, so a huge
    // exponent evaluates instantly instead of materialising a
    // multi-hundred-megabit intermediate. Values match iverilog / Verilator.
    let big = evaluate_input("3 ** 32'd200000000").expect("huge exponent should evaluate");
    let big_unsigned = evaluate_input("5 ** 32'd200000000").expect("huge exponent should evaluate");
    let wraps_to_zero = evaluate_input("2 ** 200").expect("power should evaluate");

    assert_eq!(big.output, "-32'sd1314592767");
    assert_eq!(big_unsigned.output, "32'sd958265345");
    assert_eq!(wraps_to_zero.output, "32'sd0");

    // Negative signed base exercises the two's-complement residue fold
    // (base % 2^width lands negative, then folds up into [0, 2^width)).
    let neg_odd = evaluate_input("(-3) ** 32'd200000001").expect("neg base should evaluate");
    let neg_even = evaluate_input("(-3) ** 32'd200000000").expect("neg base should evaluate");
    // Explicitly unsigned base/result: same magnitude, unsigned rendering.
    let unsigned = evaluate_input("32'd3 ** 32'd200000001").expect("unsigned base should evaluate");

    assert_eq!(neg_odd.output, "-32'sd351188995");
    assert_eq!(neg_even.output, "-32'sd1314592767");
    assert_eq!(unsigned.output, "32'd351188995");
}

#[test]
fn nested_power_exponent_is_self_determined() {
    // LRM Table 5-3: the exponent is self-determined — evaluated at its own
    // width, so it wraps mod 2^width just like any other subexpression. Thus
    // `2 ** 40` is 0 (2^40 mod 2^32), which makes `2 ** (2 ** 40)` == 2^0 == 1.
    // Matches iverilog / Verilator, which do not keep the exponent at full
    // precision.
    let inner = evaluate_input("2 ** 40").expect("power should evaluate");
    let nested_two = evaluate_input("2 ** (2 ** 40)").expect("nested power should evaluate");
    let nested_three = evaluate_input("3 ** (2 ** 40)").expect("nested power should evaluate");
    let nested_huge =
        evaluate_input("3 ** (5 ** 32'd200000000)").expect("nested huge power should evaluate");

    assert_eq!(inner.output, "32'sd0");
    assert_eq!(nested_two.output, "32'sd1");
    assert_eq!(nested_three.output, "32'sd1");
    assert_eq!(nested_huge.output, "32'sd1374756867");
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
