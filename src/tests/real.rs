use crate::evaluate_input;

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
    assert_eq!(evaluate_input("3.0 - 1.5").expect("subtract").output, "1.5");
    assert_eq!(evaluate_input("2.0 * 3.0").expect("multiply").output, "6.0");
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
    assert_eq!(evaluate_input("0.0 ** 0.0").expect("0**0").output, "1.0");
    assert_eq!(evaluate_input("0.0 ** -1.0").expect("0**neg").output, "inf");
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
    assert_eq!(evaluate_input("1.0 == 1").expect("mixed ==").output, "1'b1");
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
    assert_eq!(evaluate_input("1.0 && 0").expect("mixed &&").output, "1'b0");
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
        evaluate_input("1.0 ? 1 : 2")
            .expect("real cond, int branches")
            .output,
        "32'sd1"
    );
}

// LRM Table 5-3: every operator listed there must reject a real operand.
#[test]
fn rejects_modulus_on_real() {
    let err = evaluate_input("1.0 % 2.0").expect_err("modulus on real");
    assert_eq!(
        err,
        "Semantic error: operator % not allowed on real operand"
    );
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
        evaluate_input("1.0 + 2 * 3")
            .expect("real propagates")
            .output,
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
        evaluate_input("{(1.5 + 0.5){1'b1}}").expect_err("real-typed expression as count"),
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
