use crate::evaluate_input;

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
    let evaluation = evaluate_input("8'Sd255").expect("signed decimal should parse");
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
    let expr = evaluate_input("8 'd 6 + 1").expect("spaced based literal expression should parse");

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
    assert_eq!(
        split_signed_base,
        "Syntax error: missing base after signed marker"
    );
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

// Unsized `'sd` whose magnitude exactly fills the auto-chosen width would
// otherwise leave the MSB set and flip the literal negative. The parser
// widens by one bit so the sign bit stays free.
#[test]
fn unsized_signed_decimal_does_not_flip_negative_on_msb() {
    let evaluation = evaluate_input("'sd9999999999999999999999999")
        .expect("wide unsized signed decimal should parse");
    assert_eq!(evaluation.output, "85'sd9999999999999999999999999");
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
    let evaluation = evaluate_input("'shFFFFFFFF | 64'b0").expect("expression should evaluate");
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
    let unsigned_hex = evaluate_input("'h7FFFFFFF | 64'b0").expect("expression should evaluate");
    assert_eq!(unsigned_hex.output, "64'h000000007fffffff");

    let signed_decimal = evaluate_input("42 + 64'sb0").expect("expression should evaluate");
    assert_eq!(signed_decimal.output, "64'sd42");
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
