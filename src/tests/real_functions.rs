use crate::evaluate_input;

// LRM 17.8: $rtoi truncates toward zero (NOT round). The example values
// 123.45 → 123 and -22.7 → -22 come straight from the LRM clause. Result is
// 32-bit signed (Verilog's `integer` type), displayed in decimal.
#[test]
fn rtoi_truncates_toward_zero() {
    assert_eq!(
        evaluate_input("$rtoi(123.45)")
            .expect("$rtoi positive")
            .output,
        "32'sd123"
    );
    assert_eq!(
        evaluate_input("$rtoi(-22.7)")
            .expect("$rtoi negative")
            .output,
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
        evaluate_input("$rtoi(0.0 / 0.0)")
            .expect("$rtoi NaN")
            .output,
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
    assert_eq!(
        evaluate_input("$itor(0)").expect("$itor zero").output,
        "0.0"
    );
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
        evaluate_input(&huge_pos).expect("$itor 10**309").output,
        "inf"
    );
    let huge_neg = format!("$itor(-1{})", "0".repeat(309));
    assert_eq!(
        evaluate_input(&huge_neg).expect("$itor -10**309").output,
        "-inf"
    );
    // 10**308 is still within f64 range — the boundary stays representable.
    let in_range = format!("$itor(1{})", "0".repeat(308));
    assert_eq!(
        evaluate_input(&in_range).expect("$itor 10**308").output,
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
        evaluate_input("$bitstoreal(64'bx)")
            .expect("all-x → +0.0")
            .output,
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

// Same parser/validator split as the sign-cast diagnostics: bare
// `$rtoi` parses as a zero-arg call; the leftover `1.0` is the
// statement-level trailing-token error.
#[test]
fn rejects_real_conversion_missing_parenthesis() {
    assert_eq!(
        evaluate_input("$rtoi 1.0").expect_err("missing `(`"),
        "Syntax error: unexpected token after end of statement"
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
        evaluate_input("$clog2(1'bx)")
            .expect("$clog2 pure x")
            .output,
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
    assert_eq!(evaluate_input("$sqrt(4.0)").expect("$sqrt").output, "2.0");
    assert_eq!(evaluate_input("$ln(1.0)").expect("$ln(1)").output, "0.0");
    assert_eq!(
        evaluate_input("$log10(100.0)").expect("$log10").output,
        "2.0"
    );
    assert_eq!(evaluate_input("$exp(0.0)").expect("$exp(0)").output, "1.0");
    assert_eq!(evaluate_input("$floor(2.7)").expect("$floor").output, "2.0");
    assert_eq!(evaluate_input("$ceil(2.3)").expect("$ceil").output, "3.0");
    assert_eq!(evaluate_input("$sin(0.0)").expect("$sin(0)").output, "0.0");
    assert_eq!(evaluate_input("$cos(0.0)").expect("$cos(0)").output, "1.0");
    assert_eq!(evaluate_input("$tan(0.0)").expect("$tan(0)").output, "0.0");
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
    assert_eq!(evaluate_input("$sqrt(4)").expect("$sqrt int").output, "2.0");
    // 4'b01x0 → x/z→0 → 0100 → 4 → sqrt = 2.0
    assert_eq!(
        evaluate_input("$sqrt(4'b01x0)")
            .expect("$sqrt with x bits")
            .output,
        "2.0"
    );
    assert_eq!(evaluate_input("$exp(0)").expect("$exp int").output, "1.0");
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
    assert_eq!(evaluate_input("$ln(0.0)").expect("$ln(0)").output, "-inf");
    assert_eq!(evaluate_input("$ln(-1.0)").expect("$ln neg").output, "NaN");
    assert_eq!(
        evaluate_input("$acos(2.0)")
            .expect("$acos out of range")
            .output,
        "NaN"
    );
}

// Parser + validator diagnostics for $name(args). Parser is purely
// syntactic (parses bare `$name` as a zero-arg call, leaves arity /
// name-table checks to the validator), so `$sqrt 4.0` lands as
// `$sqrt` + leftover `4.0`. Arity / unknown-name errors come from
// `Semantic error:` (validator), unchanged in wording.
#[test]
fn math_function_parser_errors() {
    assert_eq!(
        evaluate_input("$sqrt 4.0").expect_err("missing `(`, trailing input"),
        "Syntax error: unexpected token after end of statement"
    );
    assert_eq!(
        evaluate_input("$pow(2.0").expect_err("missing `)`"),
        "Syntax error: expected `)` after $pow argument"
    );
    assert_eq!(
        evaluate_input("$pow(1.0)").expect_err("$pow 1 arg"),
        "Semantic error: $pow expects 2 arguments, got 1"
    );
    assert_eq!(
        evaluate_input("$sqrt(1.0, 2.0)").expect_err("$sqrt 2 args"),
        "Semantic error: $sqrt expects 1 argument, got 2"
    );
    assert_eq!(
        evaluate_input("$clog2(1, 2)").expect_err("$clog2 2 args"),
        "Semantic error: $clog2 expects 1 argument, got 2"
    );
}
