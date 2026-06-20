use crate::Session;
use num_bigint::BigInt;

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
    session
        .eval("integer i = 1.5")
        .expect("ties away from zero");
    assert_eq!(session.eval("i").expect("read").output, "32'sd2");
    let mut session = Session::new();
    session
        .eval("integer i = -2.5")
        .expect("negative ties away from zero");
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
    // and the assignment statement produces blank output per the
    // IPython-style suppression rule.
    let mut session = Session::new();
    session.eval("real r [0:3]").expect("decl");
    session.eval("r[1] = 2.5").expect("in-range write");
    let echo = session.eval("r[10] = 9.0").expect("oob write is dropped");
    assert_eq!(echo.output, "");
    assert_eq!(session.eval("r[1]").expect("untouched").output, "2.5");
}

#[test]
fn real_array_element_xz_index_write_is_dropped_silently() {
    let mut session = Session::new();
    session.eval("real r [0:3]").expect("decl");
    session.eval("r[1] = 2.5").expect("in-range write");
    let echo = session
        .eval("r[1'bx] = 9.0")
        .expect("xz index write is dropped");
    assert_eq!(echo.output, "");
    assert_eq!(session.eval("r[1]").expect("untouched").output, "2.5");
}

#[test]
fn real_array_element_read_in_real_arithmetic() {
    let mut session = Session::new();
    session.eval("real r [0:3]").expect("decl");
    session.eval("r[0] = 1.5").expect("write");
    assert_eq!(session.eval("r[0] + 0.5").expect("arith").output, "2.0");
    assert_eq!(
        session.eval("$sqrt(r[0] + 2.5)").expect("sqrt").output,
        "2.0"
    );
}

#[test]
fn real_array_rejects_part_select_on_lhs() {
    let mut session = Session::new();
    session.eval("real r [0:3]").expect("decl");
    let err = session
        .eval("r[1:0] = 1.0")
        .expect_err("part-select banned");
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
    let err = session.eval("r[1.0] = 1.0").expect_err("real index banned");
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
    session
        .eval("integer i = i + 1")
        .expect("redecl reads prior");
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
