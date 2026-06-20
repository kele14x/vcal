use crate::evaluate_input;

// LRM §5.5 examples: $signed/$unsigned preserve size and bit pattern; only
// the type label changes. `$signed(4'b1100)` flips the unsigned 12 to the
// signed -4; `$unsigned(-4'sd4)` flips the signed -4 to the unsigned 12.
#[test]
fn signed_unsigned_match_lrm_examples() {
    let signed_from_binary = evaluate_input("$signed(4'b1100)").expect("$signed should evaluate");
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
    assert_eq!(err, "Semantic error: unknown system identifier: $bogus");
}

#[test]
fn rejects_sign_cast_missing_parenthesis() {
    // Parser no longer demands `(` after `$signed` — bare `$signed`
    // parses as a zero-arg call, then the validator surfaces the arity
    // error. `$signed 1` has trailing input after the zero-arg call,
    // which the statement-level parser flags as the leftover.
    let missing_open = evaluate_input("$signed 1").expect_err("missing `(` should error");
    let missing_close = evaluate_input("$signed(1").expect_err("missing `)` should error");

    assert_eq!(
        missing_open,
        "Syntax error: unexpected token after end of statement"
    );
    assert_eq!(
        missing_close,
        "Syntax error: expected `)` after $signed argument"
    );
}

// vcal-specific display-base casts: `$bin` / `$oct` / `$dec` / `$hex` change
// only the `Base` field — width, signedness, and bits pass through unchanged.

#[test]
fn base_casts_change_display_base_in_each_direction() {
    assert_eq!(evaluate_input("$bin(4'hf)").expect("bin").output, "4'b1111");
    assert_eq!(evaluate_input("$hex(4'b1010)").expect("hex").output, "4'ha");
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
    assert_eq!(concat.output, "36'b000000000000000000000000000000010000");
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
    // See `rejects_sign_cast_missing_parenthesis` for the rationale —
    // bare `$bin` is a zero-arg call now; `1` is leftover.
    let missing_open = evaluate_input("$bin 1").expect_err("missing `(` should error");
    let missing_close = evaluate_input("$bin(1").expect_err("missing `)` should error");

    assert_eq!(
        missing_open,
        "Syntax error: unexpected token after end of statement"
    );
    assert_eq!(
        missing_close,
        "Syntax error: expected `)` after $bin argument"
    );
}
