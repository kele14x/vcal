use crate::Session;
use num_bigint::BigInt;

// ---------------------------------------------------------------------
// Arrays: decl-side coverage (Phase 1 of the array work).
// RHS / LHS / select-within-element behaviors land in later phases.
// ---------------------------------------------------------------------

#[test]
fn array_decl_records_dimension_and_element_count() {
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:15]").expect("decl");
    let (msb, lsb, count) = session.lookup_reg_array("a").expect("array a");
    assert_eq!(
        (msb, lsb, count),
        (BigInt::from(0u8), BigInt::from(15u8), 16)
    );
}

#[test]
fn array_decl_with_reversed_dimension_is_accepted() {
    // Reversed dimension is allowed; storage direction is private, so
    // we only assert the count and the preserved endpoints.
    let mut session = Session::new();
    session.eval("reg [3:0] a [15:0]").expect("decl");
    let (msb, lsb, count) = session.lookup_reg_array("a").expect("array a");
    assert_eq!(
        (msb, lsb, count),
        (BigInt::from(15u8), BigInt::from(0u8), 16)
    );
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
    assert_eq!(
        session.eval("a[-2]").expect("neg in range").output,
        "4'bxxxx"
    );
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
    let err = session.eval("a[1.0]").expect_err("real index should fail");
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
    assert_eq!(
        session.eval("a[2 + 3]").expect("arith idx").output,
        "4'bxxxx"
    );
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
    assert_eq!(
        session.eval("a[100][0]").expect("oob outer bit").output,
        "1'bx"
    );
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
    assert_eq!(session.eval("a[1'bz][0]").expect("z outer").output, "1'bx");
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
        session
            .eval("{a[0][3:0], a[1][1:0]}")
            .expect("concat")
            .output,
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
        session.eval("a[0] = 4'b1010; a[0]").expect("write").output,
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
        session
            .eval("a[1] = 8'b10101111; a[1]")
            .expect("trunc")
            .output,
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
        session.eval("a[0] = 2'b11; a[0]").expect("ext").output,
        "4'b0011"
    );
    assert_eq!(session.eval("a[0]").expect("readback").output, "4'b0011");
}

#[test]
fn array_element_write_signed_element_sign_extends_narrow_signed_rhs() {
    // A signed element context sign-extends a signed narrower RHS:
    // `2'sb11` (signed-binary -1) widens to `4'sb1111` (still -1).
    // Reg-array elements keep the fresh-reg binary fallback, so the
    // canonical rendering keeps the `'sb` signed-binary prefix rather
    // than collapsing to the signed-decimal form.
    let mut session = Session::new();
    session.eval("reg signed [3:0] a [0:3]").expect("decl");
    assert_eq!(
        session.eval("a[2] = 2'sb11; a[2]").expect("signed").output,
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
        session
            .eval("a[100] = 4'b0001; 4'b0001")
            .expect("oob write")
            .output,
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
        session
            .eval("a[1'bx] = 4'b0001; 4'b0001")
            .expect("x idx")
            .output,
        "4'b0001"
    );
    assert_eq!(
        session
            .eval("a[1'bz] = 4'b0010; 4'b0010")
            .expect("z idx")
            .output,
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
        session
            .eval("a[3] = 1'b1; a[3]")
            .expect("scalar write")
            .output,
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
        session
            .eval("a[0] = a[0] + 4'd1; a[0]")
            .expect("self")
            .output,
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
        session.eval("a[2] = a[1]; a[2]").expect("cross").output,
        "4'b1100"
    );
    assert_eq!(session.eval("a[2]").expect("readback").output, "4'b1100");
    assert_eq!(
        session.eval("a[1]").expect("source unchanged").output,
        "4'b1100"
    );
}

#[test]
fn array_element_write_with_real_rhs_rounds_to_integer() {
    // Real RHS implicitly converts per LRM §3.5.3 (round half away from
    // zero), then narrows to the element width. `1.5` rounds to 2,
    // which is `4'b0010` in a 4-bit unsigned element.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:3]").expect("decl");
    assert_eq!(
        session.eval("a[0] = 1.5; a[0]").expect("real rhs").output,
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
            .eval("{a[0], b} = 8'b00001111; {a[0], b}")
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
        session.eval("a[0] = 4'b1010; a[0]").expect("write").output,
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
        session
            .eval("a[-2] = 4'b1001; a[-2]")
            .expect("neg write")
            .output,
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
        session
            .eval("a[2 + 3] = 4'b1111; a[5]")
            .expect("arith idx")
            .output,
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
        session
            .eval("a[1][0] = 1'b1; a[1][0]")
            .expect("write bit")
            .output,
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
            .eval("a[2][5:2] = 4'b1011; a[2][5:2]")
            .expect("write slice")
            .output,
        "4'b1011"
    );
    // Bits [5:2] become 1011, others stay 0.
    assert_eq!(
        session.eval("a[2]").expect("readback").output,
        "8'b00101100"
    );
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
            .eval("a[0][2 +: 3] = 3'b111; a[0][2 +: 3]")
            .expect("indexed up")
            .output,
        "3'b111"
    );
    assert_eq!(
        session.eval("a[0]").expect("readback").output,
        "8'b00011100"
    );
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
            .eval("a[1'bx][2:0] = 3'b111; 3'b111")
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
            .eval("a[42][2:0] = 3'b111; 3'b111")
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
            .eval("a[0][1'bx] = 1'b1; 1'b1")
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
            .eval("a[0][5:2] = 4'b1111; 4'b1111")
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
    let err = session.eval("a[0][0] = 1'b1").expect_err("scalar element");
    assert!(err.contains("scalar array element `a`"));
}

#[test]
fn array_element_chained_select_rejects_part_outer_select() {
    // The outer select on an array must be a `Bit`; using a part-select
    // is rejected with the array-element diagnostic.
    let mut session = Session::new();
    session.eval("reg [3:0] a [0:7]").expect("decl");
    let err = session.eval("a[3:0][1:0] = 2'b11").expect_err("part outer");
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
            .eval("{b, a[0][2:0], a[1]} = 11'b10110010110; {b, a[0][2:0], a[1]}")
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
            .eval("{a[0], c[1]} = 8'b11110000; {a[0], c[1]}")
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
            .eval("{a[1'bx], b} = 8'b11110000; 8'b11110000")
            .expect("concat with x-index")
            .output,
        "8'b11110000"
    );
    // b receives the low nibble correctly despite the dropped leaf.
    assert_eq!(session.eval("b").expect("b").output, "4'b0000");
    // a[0] is untouched (the x-index doesn't accidentally hit it).
    assert_eq!(
        session.eval("a[0]").expect("a[0] preserved").output,
        "4'b1010"
    );
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
            .eval("a[0][4:0] = -1; a[0][4:0]")
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
            .eval("a[0][3:0] = a[0]; a[0][3:0]")
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
    let err = session.eval("a[1'bx] = nope").expect_err("rhs error wins");
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
    let err = session.eval("{a, b} = 20'b0").expect_err("bare array leaf");
    assert!(err.contains("array `a` cannot be used as a value"));
}
