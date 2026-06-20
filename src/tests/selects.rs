use crate::{Session, evaluate_input};

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
    // learned reg base flows through.
    let mut session = Session::new();
    session.eval("reg [7:0] r = 8'hAB").expect("decl with init");
    assert_eq!(session.eval("r[3:0]").expect("low nibble").output, "4'hb");
    assert_eq!(session.eval("r[7:4]").expect("high nibble").output, "4'ha");
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
    session.eval("reg [7:0] r = 8'hAB").expect("decl with init");
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
    session.eval("reg [7:0] r = 8'hAB").expect("decl with init");
    assert_eq!(session.eval("r[8]").expect("above range").output, "1'hx");
    assert_eq!(session.eval("r[-1]").expect("below range").output, "1'hx");
}

#[test]
fn out_of_range_part_select_fills_only_the_out_of_range_bits_with_x() {
    // LRM 4.2.1's "out-of-range → x" rule applies per position, so the
    // in-range bits keep their value and only the off-the-end positions
    // become x. 8'hAB = 8'b10101011, so bits 6 and 7 are `1` and `0`,
    // and bits 8 / 9 are out of range.
    let mut session = Session::new();
    session.eval("reg [7:0] r = 8'hAB").expect("decl with init");
    assert_eq!(
        session
            .eval("r[9:6]")
            .expect("constant partial overlap")
            .output,
        "4'hx"
    );
    assert_eq!(
        session
            .eval("r[6 +: 4]")
            .expect("indexed partial overlap")
            .output,
        "4'hx"
    );
    // The example from the bug report, exact wording: `reg [3:0] a =
    // 4'b0101; a[4:3]` → `2'bx0` (bit 4 oob → x; bit 3 in range → 0).
    let mut session = Session::new();
    session
        .eval("reg [3:0] a = 4'b0101")
        .expect("decl with init");
    assert_eq!(
        session.eval("a[4:3]").expect("partial overlap").output,
        "2'bx0"
    );
}

#[test]
fn xz_in_bit_select_index_yields_x() {
    let mut session = Session::new();
    session.eval("reg [7:0] r = 8'hAB").expect("decl with init");
    session
        .eval("reg [3:0] i = 4'bxx10")
        .expect("decl with x bits");
    // i has x bits anywhere → bit-select index unknown → result 1'bx.
    assert_eq!(session.eval("r[i]").expect("x in index").output, "1'hx");
}

#[test]
fn xz_in_indexed_part_select_base_yields_all_x() {
    let mut session = Session::new();
    session.eval("reg [7:0] r = 8'hAB").expect("decl with init");
    session
        .eval("reg [3:0] i = 4'bxx10")
        .expect("decl with x bits");
    assert_eq!(session.eval("r[i +: 4]").expect("x in base").output, "4'hx");
}

#[test]
fn xz_in_constant_part_select_endpoint_errors() {
    let mut session = Session::new();
    session.eval("reg [7:0] r = 8'hAB").expect("decl with init");
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
    session.eval("reg [7:0] r = 8'hAB").expect("decl with init");
    let err_xz = session
        .eval("r[0 +: 4'bxxxx]")
        .expect_err("x in width errors");
    assert!(
        err_xz.contains("indexed part-select width contains unknown bits"),
        "error should mention unknown bits, got: {err_xz}"
    );
    let err_zero = session.eval("r[0 +: 0]").expect_err("zero width errors");
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
    // an unsigned 8-bit value in the reg's learned decimal base.
    let mut session = Session::new();
    session
        .eval("reg signed [7:0] s = -8'sd1")
        .expect("signed decl");
    assert_eq!(
        session.eval("s[7:0]").expect("full select").output,
        "8'd255"
    );
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
    session.eval("reg [7:0] r = 8'hAB").expect("decl with init");
    assert_eq!(
        session.eval("r[3:0] + 16'b0").expect("widened").output,
        "16'h000b"
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
    session.eval("reg [7:0] r = 8'hAB").expect("decl with init");
    assert_eq!(
        session.eval("r[2 +: 4]").expect("adjacent ok").output,
        "4'ha"
    );
    let err = session
        .eval("r[2 + : 4]")
        .expect_err("space-separated rejected");
    assert!(
        !err.is_empty(),
        "space-separated `+ :` should not parse as indexed select"
    );
}
