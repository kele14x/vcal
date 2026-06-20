use crate::{Session, evaluate_input};
use num_bigint::BigInt;

// `reg` declarations and blocking assignment — the smallest end-to-end
// variable type. These tests cover decl forms (with / without range, signed,
// reversed range, multi-name), default x-initialization, width/sign behavior
// of blocking assignment, weak display-base resolution for regs, error
// surfaces for undeclared / redeclared identifiers and real RHS, and Session
// state persistence across multiple `eval` calls.

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

    assert_eq!(
        session.eval("a + 1").expect("a + 1").output,
        "32'b00000000000000000000000000001011"
    );
    assert_eq!(
        session.eval("b + 1").expect("b + 1").output,
        "32'b00000000000000000000000000001011"
    );
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

    assert_eq!(
        session.eval("a = 1'b1; a").expect("assign a").output,
        "1'b1"
    );
    assert_eq!(
        session.eval("b = 1'b1; b").expect("assign b").output,
        "1'b1"
    );
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
    assert_eq!(session.eval("a = 8'hff; a").expect("assign").output, "4'hf");
}

#[test]
fn assignment_sign_extends_narrower_rhs_into_signed_reg() {
    let mut session = Session::new();
    session.eval("reg signed [7:0] a").expect("decl");
    assert_eq!(
        session.eval("a = 4'shf; a").expect("assign").output,
        "8'shff"
    );
}

#[test]
fn assignment_preserves_x_and_z_bits() {
    let mut session = Session::new();
    session.eval("reg [3:0] a").expect("decl");
    assert_eq!(
        session.eval("a = 4'b10xz; a").expect("assign").output,
        "4'b10xz"
    );
}

#[test]
fn reg_assignment_resolves_weak_display_base_once() {
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("decl");
    assert_eq!(
        session.eval("a").expect("unresolved read").output,
        "8'bxxxxxxxx"
    );
    assert_eq!(
        session.eval("a = 8'h00; a").expect("first assign").output,
        "8'h00"
    );
    assert_eq!(
        session.eval("a = 8'b1; a").expect("later assign").output,
        "8'h01"
    );
}

#[test]
fn reg_init_resolves_display_base_but_not_signedness() {
    let mut session = Session::new();
    session
        .eval("reg signed [7:0] a = 8'hff")
        .expect("signed hex init");
    assert_eq!(session.eval("a").expect("read").output, "8'shff");
}

#[test]
fn real_assignment_does_not_resolve_weak_reg_display_base() {
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("decl");
    assert_eq!(
        session.eval("a = 1.5; a").expect("real assign").output,
        "8'b00000010"
    );
    assert_eq!(
        session.eval("a = 8'h03; a").expect("integer assign").output,
        "8'h03"
    );
}

#[test]
fn select_assignment_does_not_resolve_whole_reg_display_base() {
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("decl");
    assert_eq!(
        session
            .eval("a[3:0] = 4'hf; a")
            .expect("select assign")
            .output,
        "8'bxxxx1111"
    );
    assert_eq!(
        session.eval("a = 8'h00; a").expect("whole assign").output,
        "8'h00"
    );
}

#[test]
fn reg_value_participates_in_later_expression_with_its_own_base() {
    // After storing 4'h0a into an 8-bit reg, the weak declaration base
    // resolves to hex. Later expressions propagate that stored base.
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("decl");
    session.eval("a = 4'h0a").expect("assign");
    assert_eq!(session.eval("a + 4'b1").expect("expr").output, "8'h0b");
}

#[test]
fn assignment_of_real_value_implicitly_converts_per_lrm_3_5_3() {
    // LRM §3.5.3: implicit real→integer conversion rounds to nearest
    // with ties away from zero (distinct from `$rtoi`'s truncation). So
    // `1.5` rounds to 2, not truncates to 1.
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("decl");
    assert_eq!(
        session.eval("a = 1.5; a").expect("real RHS rounds").output,
        "8'b00000010"
    );
    assert_eq!(
        session
            .eval("a = -2.5; a")
            .expect("ties away from zero")
            .output,
        "8'b11111101"
    );
    assert_eq!(
        session.eval("a = 3.4; a").expect("rounds toward 3").output,
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
        session.eval("a = 0.0/0.0; a").expect("NaN").output,
        "4'bxxxx"
    );
    assert_eq!(
        session.eval("a = 1.0/0.0; a").expect("+inf").output,
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
    // a reg's metadata. The new decl wipes width / signed / display base /
    // value; the new reg starts at all-x just like a fresh one.
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("first decl");
    session.eval("a = 8'h2a").expect("populate");
    assert_eq!(session.eval("a").expect("read").output, "8'h2a");
    session.eval("reg [3:0] a").expect("redecl narrower");
    assert_eq!(
        session.eval("a").expect("read after redecl").output,
        "4'bxxxx"
    );
}

#[test]
fn reg_decl_accepts_negative_range_endpoint() {
    let mut session = Session::new();
    session
        .eval("reg [-1:0] a")
        .expect("negative endpoint should be accepted");
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
        session.eval("a = 4'hF + 4'hF; a").expect("assign").output,
        "8'h1e"
    );
    assert_eq!(session.eval("a").expect("read").output, "8'h1e");
}

#[test]
fn reg_decl_init_value_populates_bits() {
    let mut session = Session::new();
    session.eval("reg [7:0] a = 8'h2a").expect("decl with init");
    assert_eq!(session.eval("a").expect("read").output, "8'h2a");
}

#[test]
fn reg_decl_init_truncates_wider_literal_to_reg_width() {
    // The init RHS goes through the same width context as a blocking
    // assignment, so an 8-bit literal narrowed to a 4-bit reg keeps the
    // low 4 bits.
    let mut session = Session::new();
    session.eval("reg [3:0] a = 8'hff").expect("decl with init");
    assert_eq!(session.eval("a").expect("read").output, "4'hf");
    let mut session = Session::new();
    session.eval("reg [3:0] a = 8'h1f").expect("decl with init");
    assert_eq!(session.eval("a").expect("read").output, "4'hf");
}

#[test]
fn reg_decl_init_sign_extends_into_signed_reg() {
    // `-1` is unsized signed 32-bit; flowing through a signed 4-bit reg
    // sign-extends down to all-ones — same path as `a = -1`.
    let mut session = Session::new();
    session
        .eval("reg signed [3:0] s = -1")
        .expect("decl with signed init");
    assert_eq!(session.eval("s").expect("read").output, "-4'sd1");
}

#[test]
fn reg_decl_init_real_value_implicitly_converts_per_lrm_3_5_3() {
    // Real init triggers the same implicit real→integer conversion as a
    // blocking assignment: round half away from zero.
    let mut session = Session::new();
    session.eval("reg [7:0] a = 1.5").expect("real init rounds");
    assert_eq!(session.eval("a").expect("read").output, "8'b00000010");
    let mut session = Session::new();
    session
        .eval("reg signed [7:0] a = -2.5")
        .expect("ties away from zero");
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
    assert_eq!(session.eval("b").expect("read b").output, "1'd1");
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
    assert_eq!(session.eval("a").expect("read a").output, "4'd1");
    assert_eq!(session.eval("b").expect("read b").output, "4'd2");
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
    assert_eq!(session.eval("a").expect("read").output, "4'd7");
}

#[test]
fn reg_decl_self_referencing_init_narrows_prior_binding() {
    // Narrowing redecl with `= a` carries the prior bits through the
    // assignment-RHS width context, dropping high bits. With prior
    // `reg [1:0] a = 2'b11` (=3) and a new 1-bit `reg a = a`, the low
    // bit survives.
    let mut session = Session::new();
    session.eval("reg [1:0] a = 2'b11").expect("first decl");
    session
        .eval("reg a = a")
        .expect("redecl narrower with self-init");
    assert_eq!(session.eval("a").expect("read").output, "1'b1");
}

#[test]
fn reg_decl_self_referencing_init_without_prior_binding_errors() {
    // No prior binding means the identifier in the init RHS is genuinely
    // undeclared at evaluation time — surface the same error path as a
    // normal expression.
    let err = evaluate_input("reg a = a").expect_err("self-init without prior binding errors");
    assert_eq!(err, "Semantic error: undeclared identifier: a");
}

#[test]
fn reg_decl_init_can_reference_previously_declared_reg() {
    let mut session = Session::new();
    session.eval("reg [3:0] a = 5").expect("first decl");
    session
        .eval("reg [7:0] b = a + 1")
        .expect("init from prior reg");
    assert_eq!(session.eval("b").expect("read").output, "8'd6");
}

#[test]
fn reg_decl_init_preserves_declared_reg_range_metadata() {
    // The init applies after the RegValue is inserted, so the range
    // metadata stored at decl time is still present afterwards.
    let mut session = Session::new();
    session
        .eval("reg [0:3] a = 4'b1010")
        .expect("decl with init");
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
    let err = evaluate_input("reg [3:0] a = nope").expect_err("undeclared identifier in init");
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
    assert_eq!(session.eval("a").expect("read a").output, "4'd7");
    assert!(session.lookup("b").is_none(), "b should not be bound");
}

#[test]
fn reg_decl_rejects_duplicate_names_even_with_init() {
    let err = evaluate_input("reg [3:0] a = 1, a = 2").expect_err("duplicate names rejected");
    assert!(
        err.contains("duplicate name"),
        "error should mention duplicate name, got: {err}"
    );
}
