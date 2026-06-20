use crate::{Session, evaluate_input};

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
        session.eval("a = 8'hAB; a").expect("bare assign").output,
        "8'hab"
    );
    assert_eq!(session.eval("a").expect("read").output, "8'hab");
}

#[test]
fn bit_select_lhs_writes_single_bit() {
    let mut session = Session::new();
    session.eval("reg [7:0] r = 8'h00").expect("decl");
    assert_eq!(
        session
            .eval("r[2] = 1'b1; r[2]")
            .expect("bit-select assign")
            .output,
        "1'h1"
    );
    assert_eq!(session.eval("r").expect("read r").output, "8'h04");
}

#[test]
fn part_const_lhs_writes_slice() {
    let mut session = Session::new();
    session.eval("reg [7:0] r = 8'h00").expect("decl");
    assert_eq!(
        session
            .eval("r[5:2] = 4'hF; r[5:2]")
            .expect("part-const assign")
            .output,
        "4'hf"
    );
    assert_eq!(session.eval("r").expect("read r").output, "8'h3c");
}

#[test]
fn part_indexed_up_lhs_writes_slice() {
    // `r[2 +: 4]` covers source indices 2..5 (LSB-first); for forward
    // range [7:0] that maps to internal indices [2,3,4,5].
    let mut session = Session::new();
    session.eval("reg [7:0] r = 8'h00").expect("decl");
    assert_eq!(
        session
            .eval("r[2 +: 4] = 4'b1010; r[2 +: 4]")
            .expect("indexed-up assign")
            .output,
        "4'ha"
    );
    assert_eq!(session.eval("r").expect("read r").output, "8'h28");
}

#[test]
fn part_indexed_down_lhs_writes_slice() {
    // `r[5 -: 4]` covers source indices 2..5 — bit-for-bit equivalent to
    // `r[2 +: 4]` on a forward range — so the result matches the up form.
    let mut session = Session::new();
    session.eval("reg [7:0] r = 8'h00").expect("decl");
    assert_eq!(
        session
            .eval("r[5 -: 4] = 4'b1010; r[5 -: 4]")
            .expect("indexed-down assign")
            .output,
        "4'ha"
    );
    assert_eq!(session.eval("r").expect("read r").output, "8'h28");
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
            .eval("{a, b} = 8'hAB; {a, b}")
            .expect("concat assign")
            .output,
        "8'hab"
    );
    assert_eq!(session.eval("a").expect("read a").output, "4'ha");
    assert_eq!(session.eval("b").expect("read b").output, "4'hb");
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
            .eval("{x, {y, z}} = 6'b110010; {x, {y, z}}")
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
            .eval("{a[3:0], b[7:4]} = 8'hAB; {a[3:0], b[7:4]}")
            .expect("concat-of-selects assign")
            .output,
        "8'hab"
    );
    // a[3:0] receives the MSB-side nibble 0xA.
    assert_eq!(session.eval("a").expect("read a").output, "8'h0a");
    // b[7:4] receives the LSB-side nibble 0xB.
    assert_eq!(session.eval("b").expect("read b").output, "8'hb0");
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
            .eval("r[hi:2] = 4'hF; r[hi:2]")
            .expect("runtime endpoint")
            .output,
        "4'hf"
    );
    assert_eq!(session.eval("r").expect("read r").output, "8'h3c");
}

#[test]
fn lhs_select_on_negative_endpoint_reg() {
    // Reversed reg with a negative endpoint: same source-index → internal
    // mapping the RHS select forms use, just applied on the write side.
    let mut session = Session::new();
    session.eval("reg [-1:2] r = 4'b0000").expect("decl");
    assert_eq!(
        session
            .eval("r[-1] = 1'b1; r[-1]")
            .expect("write MSB")
            .output,
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
        session
            .eval("r[7] = 1'b1; r[7]")
            .expect("oob bit-select")
            .output,
        "1'bx"
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
            .eval("r[5:2] = 4'b1111; r[5:2]")
            .expect("partial overlap")
            .output,
        "4'hx"
    );
    assert_eq!(session.eval("r").expect("read r").output, "4'hc");
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
            .eval("r[idx] = 1'b1; r[idx]")
            .expect("x-index bit-select")
            .output,
        "1'bx"
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
            .eval("{a[0], a[0]} = 2'b10; {a[0], a[0]}")
            .expect("duplicate-bit lvalue is not an error")
            .output,
        "2'h3"
    );
    assert_eq!(session.eval("a").expect("read a").output, "4'h1");
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
    assert_eq!(session.eval("r").expect("read r").output, "8'h00");
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
    assert_eq!(session.eval("r").expect("read r").output, "4'h0");
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
    assert_eq!(
        err,
        "Semantic error: indexed part-select base cannot be real"
    );
    assert_eq!(session.eval("r").expect("read r").output, "4'h0");
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
        session
            .eval("{a, b} = 6.7; {a, b}")
            .expect("real RHS")
            .output,
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
        session
            .eval("{a, b} = 0.0/0.0; {a, b}")
            .expect("NaN RHS")
            .output,
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
            .eval("{a, b} = 16'hDEAD; {a, b}")
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
        session
            .eval("{a, b} = 4'h5; {a, b}")
            .expect("zero-extend")
            .output,
        "8'b00000101"
    );
    assert_eq!(session.eval("a").expect("read a").output, "4'b0000");
    assert_eq!(session.eval("b").expect("read b").output, "4'b0101");
}

#[test]
fn echo_for_bare_name_lhs_uses_reg_metadata() {
    // The first whole-reg assignment learns the RHS decimal display base.
    // Signedness still comes from the declaration.
    let mut session = Session::new();
    session.eval("reg signed [7:0] r").expect("signed decl");
    assert_eq!(
        session.eval("r = -5; r").expect("signed assign").output,
        "-8'sd5"
    );
}

#[test]
fn echo_for_select_lhs_uses_select_width() {
    // Select's width and the reg's learned base, not the
    // RHS's natural display form.
    let mut session = Session::new();
    session.eval("reg [7:0] r = 8'h00").expect("decl");
    assert_eq!(
        session
            .eval("r[3:0] = 4'hA; r[3:0]")
            .expect("select-width echo")
            .output,
        "4'ha"
    );
}

#[test]
fn echo_for_concat_lhs_uses_leftmost_base() {
    // The concat's width and the leftmost leaf's learned base.
    let mut session = Session::new();
    session.eval("reg [3:0] a = 4'h0").expect("decl a");
    session.eval("reg [3:0] b = 4'h0").expect("decl b");
    assert_eq!(
        session
            .eval("{a, b} = 8'hAB; {a, b}")
            .expect("concat echo")
            .output,
        "8'hab"
    );
}

#[test]
fn bare_concat_no_assign_still_parses_as_expression() {
    // The speculative lvalue branch must not poison the standalone-
    // concat-as-expression path: with no `=` following, the parsed
    // concat falls through to a normal expression statement.
    assert_eq!(
        evaluate_input("{1'b1, 1'b0}").expect("concat expr").output,
        "2'b10"
    );
}
