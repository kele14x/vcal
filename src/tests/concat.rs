use crate::lexer::{Token, tokenize};
use crate::{Session, evaluate_input};

// -------- Concatenation / replication (LRM 5.1.14) --------

#[test]
fn concatenation_joins_operands_msb_first() {
    // Leftmost operand occupies the high bits (LRM §5.1.14).
    let result = evaluate_input("{2'b10, 2'b01}").expect("eval");
    assert_eq!(result.output, "4'b1001");
}

#[test]
fn concatenation_supports_more_than_two_operands() {
    let result = evaluate_input("{1'b1, 2'b00, 1'b1}").expect("eval");
    assert_eq!(result.output, "4'b1001");
}

#[test]
fn concatenation_inherits_leftmost_operand_base() {
    // Same leftmost-wins rule as arithmetic/bitwise/shift (vcal display
    // convention; LRM doesn't prescribe one). The bit pattern (`8'h12`) is
    // per LRM §5.1.14; the base choice is ours.
    let hex_first = evaluate_input("{4'h1, 4'b10}").expect("eval");
    let bin_first = evaluate_input("{4'b10, 4'h1}").expect("eval");
    assert_eq!(hex_first.output, "8'h12");
    assert_eq!(bin_first.output, "8'b00100001");
}

#[test]
fn concatenation_preserves_x_and_z_bits() {
    // Concatenation never reduces unknown bits — each position is copied
    // through.
    let result = evaluate_input("{2'bxz, 2'b01}").expect("eval");
    assert_eq!(result.output, "4'bxz01");
}

#[test]
fn concatenation_result_is_unsigned_even_when_operands_signed() {
    // LRM 5.5.1 last paragraph + 5.1.14: result is unsigned regardless
    // of operand signedness.
    let result = evaluate_input("{4'sb1000, 4'sb0001}").expect("eval");
    assert_eq!(result.output, "8'b10000001");
}

#[test]
fn single_element_concatenation_is_identity_on_bits() {
    // `{x}` is legal LRM syntax (a single-operand concatenation) and
    // produces the operand's bit pattern re-flagged as unsigned.
    let result = evaluate_input("{4'b1010}").expect("eval");
    assert_eq!(result.output, "4'b1010");
}

#[test]
fn concatenation_widens_through_outer_arithmetic_context() {
    // The joined value (4 bits) zero-extends to the outer context width
    // (8 bits) before the addition runs — concatenation is unsigned, so
    // §5.5.4 zero-fills regardless of operand signedness.
    let result = evaluate_input("{4'b1010} + 8'd0").expect("eval");
    assert_eq!(result.output, "8'b00001010");
}

#[test]
fn replication_repeats_inner_concatenation() {
    // {N{...}} — N copies of the inner concatenation joined back to back.
    let single = evaluate_input("{4{1'b1}}").expect("eval");
    let multi = evaluate_input("{2{2'b01, 2'b10}}").expect("eval");
    assert_eq!(single.output, "4'b1111");
    assert_eq!(multi.output, "8'b01100110");
}

#[test]
fn replication_count_can_be_a_constant_expression() {
    // The count is any constant expression (LRM 5.1.14). vcal has no
    // variables, so any well-formed expression qualifies.
    let result = evaluate_input("{(1+3){1'b1}}").expect("eval");
    assert_eq!(result.output, "4'b1111");
}

#[test]
fn replication_can_nest_when_inner_is_braced() {
    // `{2{ {2{1'b1}} }}` — outer rep of an inner replication. Note the
    // inner `{2{1'b1}}` must itself be a brace primary; `{2{2{1'b1}}}`
    // is a syntax error since `2{1'b1}` is not a standalone primary
    // (LRM §5.1.14 grammar).
    let result = evaluate_input("{2{ {2{1'b1}} }}").expect("eval");
    assert_eq!(result.output, "4'b1111");
}

#[test]
fn concatenation_rejects_bare_unsized_literal_operand() {
    // LRM 5.1.14: "Unsized constant numbers shall not be allowed in
    // concatenations."
    let err = evaluate_input("{1, 4'd2}").expect_err("indefinite");
    assert_eq!(
        err,
        "Semantic error: concatenation operand has indefinite width"
    );
}

#[test]
fn concatenation_rejects_arithmetic_with_unsized_operand() {
    // The indefinite-width flag propagates through context-determined
    // arithmetic: `4'd1 + 1` is indefinite because the `1` is unsized.
    let err = evaluate_input("{4'd1 + 1, 4'd2}").expect_err("indefinite");
    assert_eq!(
        err,
        "Semantic error: concatenation operand has indefinite width"
    );
}

#[test]
fn concatenation_accepts_arithmetic_when_all_operands_sized() {
    // `4'd1 + 4'd1` is sized (both operands sized → result is 4-bit), so
    // the operand has a definite width and concatenation succeeds.
    // Expect `00100010` = 8'd34.
    let result = evaluate_input("{4'd1 + 4'd1, 4'd2}").expect("eval");
    assert_eq!(result.output, "8'd34");
}

#[test]
fn concatenation_accepts_one_bit_results_with_unsized_subexpressions() {
    // Relational/equality/logical/reduction always produce 1-bit results
    // — they have a definite width even when their operands are unsized.
    // Expect `{1==2, 4'd2}` → `00010` = 5 bits.
    let result = evaluate_input("{1==2, 4'd2}").expect("eval");
    assert_eq!(result.output, "5'b00010");
}

#[test]
fn concatenation_rejects_shift_with_unsized_lhs() {
    // Shifts take their result width from the LHS only (LRM 5.1.12), so
    // an unsized LHS makes the whole expression indefinite.
    let err = evaluate_input("{1 << 1, 4'd2}").expect_err("indefinite");
    assert_eq!(
        err,
        "Semantic error: concatenation operand has indefinite width"
    );
}

#[test]
fn concatenation_rejects_conditional_with_unsized_branch() {
    // Conditional width is max(then, else) (LRM 5.1.13), so an unsized
    // branch makes the whole conditional indefinite.
    let err = evaluate_input("{1'b1 ? 1 : 4'd2, 4'd2}").expect_err("indefinite");
    assert_eq!(
        err,
        "Semantic error: concatenation operand has indefinite width"
    );
}

#[test]
fn concatenation_rejects_power_with_unsized_lhs() {
    // `**` takes its result width from the LHS only (LRM 5.1.5, same
    // shape as shifts), so an unsized LHS makes the whole expression
    // indefinite even when the RHS is sized.
    let err = evaluate_input("{2 ** 4'd3, 4'd2}").expect_err("indefinite");
    assert_eq!(
        err,
        "Semantic error: concatenation operand has indefinite width"
    );
}

#[test]
fn concatenation_accepts_power_with_sized_lhs() {
    // A sized LHS pins the operand's width even when the RHS is
    // unsized — `**` is LHS-determined, so `4'd2 ** 3` is 4 bits wide.
    // 2 ** 3 = 8 → 4'd8 = 0b1000; concatenated with 4'd2 = 0b0010 gives
    // 8'b10000010 = 8'd130.
    let result = evaluate_input("{4'd2 ** 3, 4'd2}").expect("eval");
    assert_eq!(result.output, "8'd130");
}

#[test]
fn top_level_replication_rejects_zero_count() {
    // LRM 5.1.14 only permits zero replication when it sits inside a
    // concatenation with at least one positive-size operand; a top-level
    // `{0{...}}` (no enclosing concat) is rejected.
    let err = evaluate_input("{0{1'b1}}").expect_err("zero count");
    assert_eq!(
        err,
        "Semantic error: replication count must be positive in this context"
    );
}

#[test]
fn zero_replication_inside_concatenation_contributes_no_bits() {
    // LRM 5.1.14: a replication may have a zero count when it is one of
    // the operands of a concatenation whose other operands sum to a
    // positive width. The zero-rep simply contributes nothing — e.g.
    // `{{0{1'b1}}, 1'b1}` → `1`.
    let prefix = evaluate_input("{ {0{1'b1}}, 1'b1 }").expect("zero rep prefix");
    let suffix = evaluate_input("{ 4'b1010, {0{1'b1}} }").expect("zero rep suffix");
    let middle = evaluate_input("{ 1'b1, {0{1'b1}}, 1'b0 }").expect("zero rep middle");
    let multiple = evaluate_input("{ {0{1'b1}}, {0{1'b1}}, 1'b1 }").expect("zero rep many");

    assert_eq!(prefix.output, "1'b1");
    assert_eq!(suffix.output, "4'b1010");
    assert_eq!(middle.output, "2'b10");
    assert_eq!(multiple.output, "1'b1");
}

#[test]
fn zero_replication_through_grouped_is_treated_the_same() {
    // `({0{1'b1}})` is `Grouped(Replication{0, ...})`. vcal looks
    // through `Grouped` to find the underlying Replication node when
    // applying the zero-permission rule from LRM §5.1.14.
    let result = evaluate_input("{ ({0{1'b1}}), 1'b1 }").expect("grouped zero rep");
    assert_eq!(result.output, "1'b1");
}

#[test]
fn zero_replication_inside_nested_replication_inner_list() {
    // The zero-permission also applies to a replication's *inner*
    // concatenation list, since that list is itself a concatenation.
    // Expect `{2{ {0{1'b1}}, 1'b1 }}` → `11`.
    let result = evaluate_input("{2{ {0{1'b1}}, 1'b1 }}").expect("nested zero rep");
    assert_eq!(result.output, "2'b11");
}

#[test]
fn concatenation_of_only_zero_replication_is_rejected() {
    // `{ {0{1'b1}} }` — one operand, and it has zero size. No
    // positive-size sibling, so the surrounding concatenation has no
    // positive-size operand, which LRM 5.1.14 forbids.
    let solo = evaluate_input("{ {0{1'b1}} }").expect_err("solo zero rep in concat");
    let pair =
        evaluate_input("{ {0{1'b1}}, {0{1'b1}} }").expect_err("two zero reps no positive sibling");
    let nested = evaluate_input("{2{ {0{1'b1}} }}").expect_err("outer rep over zero-only inner");
    assert_eq!(
        solo,
        "Semantic error: concatenation must have at least one operand with positive size"
    );
    assert_eq!(
        pair,
        "Semantic error: concatenation must have at least one operand with positive size"
    );
    assert_eq!(
        nested,
        "Semantic error: concatenation must have at least one operand with positive size"
    );
}

#[test]
fn replication_rejects_negative_count() {
    // `-1` is signed-negative — read as a math integer, sign() = Minus.
    let err = evaluate_input("{-1{1'b1}}").expect_err("negative count");
    assert_eq!(
        err,
        "Semantic error: replication count must be non-negative"
    );
}

// The validator runs as a pre-pass on the whole expression tree, so a
// structural error inside a zero-count replication is caught even though
// the runtime would short-circuit before walking the inner items. These
// three positions used to surface differently:
//   leftmost  — caught by the leftmost-base inference path
//   rightmost — NOT caught (the bug fixed by the pre-pass)
//   middle    — NOT caught (same bug)
// All three now produce the same `Semantic error:` diagnostic.
#[test]
fn zero_replication_inner_items_validated_in_every_position() {
    let mut session = Session::new();
    session.eval("reg [3:0] r").expect("decl");

    let leftmost = session
        .eval("{ {0{r[1.0]}}, 1'b1 }")
        .expect_err("leftmost zero-rep with real index");
    let rightmost = session
        .eval("{ 1'b1, {0{r[1.0]}} }")
        .expect_err("rightmost zero-rep with real index");
    let middle = session
        .eval("{ 1'b1, {0{r[1.0]}}, 1'b0 }")
        .expect_err("middle zero-rep with real index");

    assert_eq!(leftmost, "Semantic error: bit-select index cannot be real");
    assert_eq!(rightmost, "Semantic error: bit-select index cannot be real");
    assert_eq!(middle, "Semantic error: bit-select index cannot be real");
}

#[test]
fn replication_rejects_unknown_count() {
    // A count with any x or z bit is rejected — per LRM 5.1.14 the count
    // must be "a constant expression that is non-negative, non-x, non-z".
    let err = evaluate_input("{1'bx{1'b1}}").expect_err("unknown count");
    assert_eq!(
        err,
        "Semantic error: replication count contains unknown bits"
    );
}

// Regression (P1): annotate() constant-evaluates a replication's count to
// derive the width meta. That evaluation must not run ahead of validation —
// an invalid count (real operand of an integer-only op, or a concat of real
// items) used to reach the evaluator's validated-input `unreachable!` and
// panic instead of surfacing a clean diagnostic. The count subtree is now
// validated before it is evaluated, in both the select-subexpression path
// and the top-level path.
#[test]
fn replication_count_with_real_binary_operand_is_a_clean_error() {
    // Top-level: the count `1.0 % 2` applies `%` to a real operand.
    let err = evaluate_input("{(1.0 % 2){1}}").expect_err("real % in count");
    assert_eq!(
        err,
        "Semantic error: operator % not allowed on real operand"
    );
}

#[test]
fn replication_count_concat_of_real_is_a_clean_error() {
    // Top-level: the count `({1.0})` is a concatenation whose operand is
    // real — concatenation requires definite bit widths.
    let err = evaluate_input("{({1.0}){1}}").expect_err("real concat in count");
    assert_eq!(err, "Semantic error: concatenation operand cannot be real");
}

#[test]
fn invalid_replication_count_in_select_index_is_a_clean_error() {
    // The select-subexpression position routes through the same eager
    // count evaluation via validate_subexpr_structure; it must produce the
    // same diagnostics, not panic.
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("decl");

    let real_op = session
        .eval("a[{(1.0 % 2){1}}]")
        .expect_err("real % in select-index count");
    assert_eq!(
        real_op,
        "Semantic error: operator % not allowed on real operand"
    );

    let real_concat = session
        .eval("a[{({1.0}){1}}]")
        .expect_err("real concat in select-index count");
    assert_eq!(
        real_concat,
        "Semantic error: concatenation operand cannot be real"
    );
}

#[test]
fn real_typed_count_with_structural_error_keeps_specific_diagnostic() {
    // A real-typed count skips the eager constant-evaluation, but its
    // subtree is still validated — the specific structural diagnostic
    // must surface ahead of the generic "replication count cannot be
    // real". Covers the four count shapes that are real-typed *because*
    // of the error they contain.
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("decl");

    let task = session.eval("a[{($finish){1}}]").expect_err("task count");
    assert_eq!(
        task,
        "Semantic error: $finish() is a system task, it cannot be called as a function."
    );

    let op_on_real = session.eval("a[{(~1.0){1}}]").expect_err("~ on real count");
    assert_eq!(
        op_on_real,
        "Semantic error: operator ~ not allowed on real operand"
    );

    let signed_real = session
        .eval("a[{($signed(1.0)){1}}]")
        .expect_err("$signed(real) count");
    assert_eq!(
        signed_real,
        "Semantic error: $signed argument cannot be real"
    );

    let bitstoreal = session
        .eval("a[{($bitstoreal(1)){1}}]")
        .expect_err("$bitstoreal width count");
    assert_eq!(
        bitstoreal,
        "Semantic error: $bitstoreal argument must be 64 bits wide, got 32"
    );

    // A plain real count (no structural error) still gets the generic
    // diagnostic.
    let plain_real = session.eval("a[{(1.0){1}}]").expect_err("plain real count");
    assert_eq!(
        plain_real,
        "Semantic error: replication count cannot be real"
    );
}

#[test]
fn empty_braces_is_a_parse_error() {
    // `{}` — no expressions inside; LRM grammar requires at least one.
    let err = evaluate_input("{}").expect_err("empty");
    assert_eq!(err, "Syntax error: expected expression operand");
}

#[test]
fn unclosed_concatenation_is_a_parse_error() {
    let err = evaluate_input("{4'd1, 4'd2").expect_err("unclosed");
    assert_eq!(err, "Syntax error: missing closing brace in concatenation");
}

#[test]
fn tokenizes_braces_and_comma_as_separate_tokens() {
    // Braces and comma must split adjacent literals — `1,2'b10` should
    // tokenize as `1`, `,`, `2'b10`, not be swallowed into a single
    // integer literal.
    let tokens = tokenize("{1'd1,2'b10}").expect("tokens");
    assert_eq!(
        tokens,
        vec![
            Token::LBrace,
            Token::IntegerLiteral("1'd1".to_string()),
            Token::Comma,
            Token::IntegerLiteral("2'b10".to_string()),
            Token::RBrace,
        ]
    );
}

#[test]
fn replication_widens_through_outer_arithmetic_context() {
    // Same self-determined-then-extend shape as plain concatenation: the
    // 4-bit replication result zero-extends to the 8-bit outer context.
    let result = evaluate_input("{4{1'b1}} + 8'd0").expect("eval");
    assert_eq!(result.output, "8'b00001111");
}

#[test]
fn concat_result_selects_parse_with_the_source_and_all_four_kinds() {
    use crate::parser::{Expr, SelectKind, parse_expression};

    for (input, mode) in [
        ("{32'd100}[0]", 0),
        ("{a,b}[3:0]", 1),
        ("{4'b1010}[0 +: 2]", 2),
        ("{4'b1010}[3 -: 2]", 3),
    ] {
        let expr = parse_expression(input).expect(input);
        let Expr::ConcatSelect { source, kind } = &expr else {
            panic!("expected concat-result select for {input}");
        };
        assert!(matches!(source.as_ref(), Expr::Concatenation { .. }));
        assert!(matches!(
            (mode, kind),
            (0, SelectKind::Bit { .. })
                | (1, SelectKind::PartConst { .. })
                | (2, SelectKind::PartIndexedUp { .. })
                | (3, SelectKind::PartIndexedDown { .. })
        ));
    }
    let expr = parse_expression("{2{4'hA}}[7:4]").expect("replication select");
    assert!(matches!(
        &expr,
        Expr::ConcatSelect { source, kind: SelectKind::PartConst { .. } }
            if matches!(source.as_ref(), Expr::Replication { .. })
    ));
    let expr = parse_expression("~{4'b1010}[1:0]").expect("unary select");
    assert!(matches!(
        &expr,
        Expr::Unary { expr, .. } if matches!(expr.as_ref(), Expr::ConcatSelect { .. })
    ));
}

#[test]
fn concat_result_selects_number_bits_from_zero_and_keep_numeric_base() {
    for (input, expected) in [
        ("{32'd100}[0]", "1'd0"),
        ("{32'd100}[2]", "1'd1"),
        ("{32'd100}[31]", "1'd0"),
        ("{32'd100}[6:2]", "5'd25"),
        ("{4'b1010}[0 +: 2]", "2'b10"),
        ("{4'b1010}[3 -: 2]", "2'b10"),
        ("{2{4'hA}}[7:4]", "4'ha"),
        ("{2{4'hA}}[0]", "1'h0"),
        ("{4'o5}[2:1]", "2'o2"),
        ("{2'bxz, 2'b01}[3:1]", "3'bxz0"),
        ("{2'bxz}[0]", "1'bz"),
    ] {
        assert_eq!(
            evaluate_input(input).expect(input).output,
            expected,
            "{input}"
        );
    }
    let mut session = Session::new();
    session
        .eval("reg [3:0] a = 4'hA, b = 4'b0011")
        .expect("decls");
    for (input, expected) in [
        ("{a,b}[3:0]", "4'h3"),
        ("{a,b}[7:4]", "4'ha"),
        ("{b,a}[3:0]", "4'b1010"),
    ] {
        assert_eq!(
            session.eval(input).expect(input).output,
            expected,
            "{input}"
        );
    }
}

#[test]
fn concat_result_selects_compose_with_unary_binary_conditional_and_index_expressions() {
    for (input, expected) in [
        ("~{4'b0101}[1:0]", "2'b10"),
        ("~{4'b0101}[1:0] + 4'b0", "4'b1110"),
        ("-{4'd1}[1:0]", "2'd3"),
        ("!{4'b1010}[0]", "1'b1"),
        ("&{4'b1010}[3:2]", "1'b0"),
        ("^{4'b1010}[3:2]", "1'b1"),
        ("2'd1 + {4'd10}[3:2] * 2'd2", "2'd1"),
        ("{4'b1010}[1] && {4'b1010}[0]", "1'b0"),
        ("1'b1 ? {4'b1010}[3:2] : 4'b0", "4'b0010"),
        ("{4'b1010}[0] ? 4'd3 : {4'd10}[3:2]", "4'd2"),
        ("{4'b1010}[1 ? 1 : 0]", "1'b1"),
        ("{4'b1010}[1 + 1 +: 1 ? 2 : 1]", "2'b10"),
        ("{4'b1010}[1 ? 3 : 2 : 1 ? 2 : 0]", "2'b10"),
        ("{{4'b1010}[1:0], 2'b01}[3:0]", "4'b1001"),
        ("{2{{4'b1010}[1:0]}}", "4'b1010"),
    ] {
        assert_eq!(
            evaluate_input(input).expect(input).output,
            expected,
            "{input}"
        );
    }
}

#[test]
fn concat_result_selects_are_unsigned_and_barrier_source_and_index_widths() {
    for (input, expected) in [
        ("{-4'sd1}[3:0]", "4'd15"),
        ("{-4'sd1}[3:0] + 8'sd0", "8'd15"),
        ("{-4'sd1}[3:0] < 8'sd0", "1'b0"),
        ("{4'd15 + 4'd1}[3:0] + 8'd1", "8'd1"),
        ("{4'd15 + 4'd1}[4] + 8'd0", "8'dx"),
        ("{2'd3 + 2'd1, 2'b01}[3:0]", "4'd1"),
        ("{4'b1010}[2'd3 + 2'd1] + 16'b0", "16'b0000000000000000"),
        ("{4'b1010}[2'd3 + 2'd1 +: 2] + 8'b0", "8'b00000010"),
        ("{4'd15 + 4'd1}[0 +: 8]", "8'dx"),
        ("1'b1 ? {4'd15 + 4'd1}[3:0] : 16'd1", "16'd0"),
    ] {
        assert_eq!(
            evaluate_input(input).expect(input).output,
            expected,
            "{input}"
        );
    }
    for (rhs, expected) in [
        ("{-4'sd1}[3:0]", "16'd15"),
        ("{4'd15 + 4'd1}[3:0] + 8'd1", "16'd1"),
        ("{4'b1010}[2'd3 + 2'd1]", "16'b0000000000000000"),
        ("{4'b1010}[0 +: {4'd5}[2:0]]", "16'b00000000000x1010"),
    ] {
        let mut session = Session::new();
        session.eval("reg [15:0] dst").expect("decl");
        session.eval(&format!("dst = {rhs}")).expect(rhs);
        assert_eq!(
            session.eval("dst").expect("assigned value").output,
            expected,
            "{rhs}"
        );
    }
}

#[test]
fn concat_result_selects_rebase_reversed_ranges_and_accept_scalar_operands() {
    let mut session = Session::new();
    session
        .eval("reg [0:3] a = 4'b1010; reg [-1:2] b = 4'b1010; reg scalar = 1'b1")
        .expect("decls");
    assert_eq!(session.eval("a[0]").expect("declared msb").output, "1'b1");
    for (input, expected) in [
        ("{a}[0]", "1'b0"),
        ("{a}[3]", "1'b1"),
        ("{a}[3:0]", "4'b1010"),
        ("{a}[0 +: 2]", "2'b10"),
        ("{a}[3 -: 2]", "2'b10"),
        ("{b}[3:0]", "4'b1010"),
        ("{scalar}[0]", "1'b1"),
        ("{scalar}[0:0]", "1'b1"),
        ("{scalar}[0 +: 1]", "1'b1"),
        ("{scalar}[0 -: 1]", "1'b1"),
        ("{scalar, 1'b0}[1:0]", "2'b10"),
    ] {
        assert_eq!(
            session.eval(input).expect(input).output,
            expected,
            "{input}"
        );
    }
    for input in ["{a}[0:3]", "{scalar}[0:1]", "{1'b1}[0:1]"] {
        assert!(
            session
                .eval(input)
                .expect_err(input)
                .contains("direction does not match")
        );
    }
    assert!(
        session
            .eval("scalar[0]")
            .expect_err("scalar select")
            .contains("scalar reg")
    );
}

#[test]
fn concat_result_select_unknown_and_outside_indices_produce_x_per_position() {
    for index in [
        "4",
        "-1",
        "1'bx",
        "1'bz",
        "128'd18446744073709551616",
        "-128'sd18446744073709551616",
    ] {
        let input = format!("{{4'b1010}}[{index}]");
        assert_eq!(
            evaluate_input(&input).expect(&input).output,
            "1'bx",
            "{input}"
        );
    }
    for (input, expected) in [
        ("{4'b1010}[5:2]", "4'bxx10"),
        ("{4'b1010}[1:-2]", "4'b10xx"),
        ("{4'b1010}[4:3]", "2'bx1"),
        ("{4'b1010}[-1:-3]", "3'bxxx"),
        ("{4'b1010}[2 +: 4]", "4'bxx10"),
        ("{4'b1010}[-1 +: 4]", "4'b010x"),
        ("{4'b1010}[1 -: 4]", "4'b10xx"),
        ("{4'b1010}[-1 -: 3]", "3'bxxx"),
        ("{4'b1010}[1'bx +: 3]", "3'bxxx"),
        ("{4'b1010}[1'bz -: 3]", "3'bxxx"),
        ("{4'b1010}[128'd18446744073709551616 +: 2]", "2'bxx"),
        ("{4'b1010}[128'd18446744073709551616 -: 2]", "2'bxx"),
        ("{1'b1}[1:0]", "2'bx1"),
    ] {
        assert_eq!(
            evaluate_input(input).expect(input).output,
            expected,
            "{input}"
        );
    }
}

#[test]
fn concat_result_select_bounds_and_widths_keep_existing_diagnostics() {
    for (suffix, diagnostic) in [
        ("[1.0]", "bit-select index cannot be real"),
        ("[1.0:0]", "part-select msb cannot be real"),
        ("[3:1.0]", "part-select lsb cannot be real"),
        ("[1'bx:0]", "part-select msb contains unknown bits"),
        ("[3:1'bz]", "part-select lsb contains unknown bits"),
        ("[0:3]", "part-select direction does not match"),
        ("[1.0 +: 2]", "indexed part-select base cannot be real"),
        ("[1.0 -: 2]", "indexed part-select base cannot be real"),
        ("[0 +: 1.0]", "indexed part-select width cannot be real"),
        ("[0 -: 1.0]", "indexed part-select width cannot be real"),
        (
            "[0 +: 1'bx]",
            "indexed part-select width contains unknown bits",
        ),
        (
            "[0 -: 1'bz]",
            "indexed part-select width contains unknown bits",
        ),
        ("[0 +: 0]", "indexed part-select width must be positive"),
        ("[0 -: -1]", "indexed part-select width must be positive"),
        (
            "[0 +: 128'd18446744073709551616]",
            "indexed part-select width too large",
        ),
        (
            "[128'd18446744073709551616:0]",
            "part-select width too large",
        ),
    ] {
        let input = format!("{{4'b1010}}{suffix}");
        let err = evaluate_input(&input).expect_err(&input);
        assert!(
            err.starts_with("Semantic error:") && err.contains(diagnostic),
            "{input}: {err}"
        );
    }
}

#[test]
fn concat_result_select_does_not_hide_invalid_or_oversized_sources() {
    for (input, diagnostic) in [
        ("{100}[0]", "concatenation operand has indefinite width"),
        (
            "{4'd1 + 1}[0]",
            "concatenation operand has indefinite width",
        ),
        (
            "{1 << 4'd1}[0]",
            "concatenation operand has indefinite width",
        ),
        ("{1.0}[0]", "concatenation operand cannot be real"),
        ("{2{1.0}}[0]", "replication operand cannot be real"),
        ("{$finish}[0]", "is a system task"),
        ("{$display(\"not printed\")}[0]", "is a system task"),
        (
            "{0{1'b1}}[0]",
            "replication count must be positive in this context",
        ),
        (
            "{{0{1'b1}}}[0]",
            "concatenation must have at least one operand with positive size",
        ),
        (
            "{{0{1'b1}}, {0{1'b0}}}[0]",
            "concatenation must have at least one operand with positive size",
        ),
        (
            "{{0{1'b1}}[0], 1'b1}",
            "replication count must be positive in this context",
        ),
        (
            "{16777217'b0}[0]",
            "concatenation width 16777217 exceeds limit 16777216",
        ),
        (
            "{16777217{1'b0}}[0]",
            "replication width 16777217 exceeds limit 16777216",
        ),
        (
            "{9000000'b0, 9000000'b0}[0]",
            "concatenation width 18000000 exceeds limit 16777216",
        ),
        (
            "{1'b1}[16777216:0]",
            "part-select width 16777217 exceeds limit 16777216",
        ),
        (
            "{1'b1}[0 +: 16777217]",
            "part-select width 16777217 exceeds limit 16777216",
        ),
        (
            "{1'b1}[0 -: 16777217]",
            "part-select width 16777217 exceeds limit 16777216",
        ),
        (
            "1'b0 ? {100}[0] : 1'b1",
            "concatenation operand has indefinite width",
        ),
        (
            "{1'b1, {0{{1.0}[0]}}}",
            "concatenation operand cannot be real",
        ),
    ] {
        let err = evaluate_input(input).expect_err(input);
        assert!(
            err.starts_with("Semantic error:") && err.contains(diagnostic),
            "{input}: {err}"
        );
        assert!(err.task_output.is_empty(), "{input}");
    }
    assert_eq!(
        evaluate_input("{{0{1'b0}}, 1'b1}[0]")
            .expect("zero sibling")
            .output,
        "1'b1"
    );
}

#[test]
fn concat_result_selects_work_as_nested_bounds_widths_counts_and_lvalue_indices() {
    for (input, expected) in [
        ("{4'b1010}[{2'b01}[0]]", "1'b1"),
        ("{4'b1010}[{2'b11}[1:0]:{1'b0}[0]]", "4'b1010"),
        ("{4'b1010}[{2'b01}[1:0] +: {2'b10}[1:0]]", "2'b01"),
        ("{4'b1010}[{2'b11}[1:0] -: {2'b10}[1:0]]", "2'b10"),
        ("{{2'b10}[1:0]{1'b1}}[1:0]", "2'b11"),
    ] {
        assert_eq!(
            evaluate_input(input).expect(input).output,
            expected,
            "{input}"
        );
    }
    let mut session = Session::new();
    session
        .eval("reg [7:0] r = 8'b0; reg [3:0] a [0:1]")
        .expect("decls");
    session.eval("r[{2'b01}[0]] = 1'b1").expect("bit lhs index");
    assert_eq!(session.eval("r").expect("bit write").output, "8'b00000010");
    session
        .eval("r[{3'b101}[2:0]:{2'b10}[1:0]] = 4'b1010")
        .expect("part lhs bounds");
    assert_eq!(session.eval("r").expect("part write").output, "8'b00101010");
    session
        .eval("r[{3'b100}[2:0] +: {2'b10}[1:0]] = 2'b01")
        .expect("indexed lhs");
    assert_eq!(
        session.eval("r").expect("indexed write").output,
        "8'b00011010"
    );
    session
        .eval("a[{2'b01}[0]][{2'b10}[1:0] -: {2'b10}[1:0]] = 2'b11")
        .expect("array lhs");
    assert_eq!(
        session.eval("a[1][2:1]").expect("array read").output,
        "2'b11"
    );
    session
        .eval("reg [{2'b11}[1:0]:{1'b0}[0]] dst = 4'hA")
        .expect("packed bounds");
    session
        .eval("reg [0:0] arr [{1'b0}[0]:{1'b1}[0]]; arr[1] = 1'b1")
        .expect("unpacked bounds");
    assert_eq!(session.eval("dst").expect("decl value").output, "4'ha");
    assert_eq!(session.eval("arr[1]").expect("array value").output, "1'b1");
}

#[test]
fn concat_result_select_bound_caches_refresh_and_do_not_widen() {
    let mut session = Session::new();
    session
        .eval("reg [7:0] r = 8'h55; reg [3:0] width = 4'd5; reg [15:0] dst")
        .expect("decls");
    let input = "{r}[0 +: {width}[3:0]]";
    assert_eq!(
        session
            .eval(&format!("{input} + 16'h0"))
            .expect("wide arithmetic")
            .output,
        "16'h0015"
    );
    session
        .eval(&format!("dst = {input}"))
        .expect("wide assignment");
    assert_eq!(session.eval("dst").expect("assigned").output, "16'h0015");
    assert_eq!(
        session
            .eval(&format!("width = 4'd1; {input}"))
            .expect("refreshed width")
            .output,
        "1'h1"
    );
    assert_eq!(
        session
            .eval(&format!("width = 4'd0; {input}"))
            .expect_err("zero width"),
        "Semantic error: indexed part-select width must be positive"
    );
    assert_eq!(
        session
            .eval(&format!("width = 4'd5; {input}"))
            .expect("restored width")
            .output,
        "5'h15"
    );
}

#[test]
fn concat_result_select_clears_string_style_but_keeps_packed_string_bits() {
    for (input, expected) in [
        ("{\"AB\"}[7:0]", "8'h42"),
        ("{\"A\", \"B\"}[15:8]", "8'h41"),
        ("{\"AB\"}[0]", "1'h0"),
        ("{\"AB\"}[1]", "1'h1"),
        ("{2{\"A\"}}[15:0]", "16'h4141"),
        ("{\"AB\"}[0 +: 8]", "8'h42"),
        ("{\"AB\"}[15 -: 8]", "8'h41"),
        ("{\"\"}[7:0]", "8'h00"),
        ("{\"\\377\"}[7:0]", "8'hff"),
        ("{{\"AB\"}[7:0], \"C\"}", "16'h4243"),
    ] {
        assert_eq!(
            evaluate_input(input).expect(input).output,
            expected,
            "{input}"
        );
    }
    let mut session = Session::new();
    session
        .eval("reg [15:0] dst = {\"AB\"}[7:0]")
        .expect("string bits assignment");
    assert_eq!(
        session.eval("dst").expect("numeric read").output,
        "16'h0042"
    );
}

#[test]
fn concat_result_selects_reject_other_expression_suffixes_chaining_and_lvalues() {
    let mut session = Session::new();
    session
        .eval("reg [3:0] a = 4'b1010, b = 4'b0101")
        .expect("decls");
    for input in [
        "4'b1010[0]",
        "(4'b1010)[0]",
        "(a + b)[0]",
        "(a)[0]",
        "\"A\"[0]",
        "({a,b})[0]",
        "({2{a}})[0]",
        "{a,b}[0][0]",
        "{a,b}[3:0][1]",
        "{2{a}}[3:0][0]",
        "{a,b}[0] = 1'b0",
        "{a,b}[3:0] = 4'b0",
        "{a,b}[0 +: 2] = 2'b0",
        "{a,b}[3 -: 2] = 2'b0",
        "{2{a}}[0] = 1'b0",
        "{{a,b}[0], a} = 5'b0",
    ] {
        let err = session.eval(input).expect_err(input);
        assert!(err.starts_with("Syntax error:"), "{input}: {err}");
        assert_eq!(session.eval("a").expect("a unchanged").output, "4'b1010");
        assert_eq!(session.eval("b").expect("b unchanged").output, "4'b0101");
    }
}

#[test]
fn malformed_concat_result_selects_recover_without_executing_earlier_statements() {
    let mut session = Session::new();
    session.eval("reg [3:0] a = 4'b1010").expect("decl");
    for malformed in [
        "{a}[",
        "{a}[]",
        "{a}[0",
        "{a}[1:]",
        "{a}[:0]",
        "{a}[3:0",
        "{a}[0 +:]",
        "{a}[0 -:]",
        "{a}[0 +: 1",
        "{a}[0 + : 1]",
        "{a}[0 - : 1]",
        "{a}[0,1]",
        "{a}[0][0]",
        "{2{a}}[0",
        "{{a}[0]}[",
    ] {
        let input = format!("$display(\"not executed\"); a = 4'b0; reg fresh; {malformed}");
        let err = session.eval(&input).expect_err(malformed);
        assert!(err.starts_with("Syntax error:"), "{malformed}: {err}");
        assert!(err.task_output.is_empty(), "{malformed}");
        assert!(session.lookup("fresh").is_none(), "{malformed}");
        assert_eq!(session.eval("a").expect("a unchanged").output, "4'b1010");
        assert_eq!(
            session.eval("{a}[0]").expect("session recovered").output,
            "1'b0"
        );
    }
}

#[test]
fn invalid_concat_result_select_subexpressions_report_errors_without_panics_or_tasks() {
    let mut session = Session::new();
    session.eval("reg [3:0] a = 4'b1010").expect("decl");
    for (bad, diagnostic) in [
        ("{1.0}[0]", "concatenation operand cannot be real"),
        ("{$finish}[0]", "is a system task"),
    ] {
        for template in [
            "a[BAD]",
            "{a}[BAD]",
            "{a}[BAD:0]",
            "{a}[3:BAD]",
            "{a}[BAD +: 1]",
            "{a}[0 -: BAD]",
            "{(BAD){1'b1}}",
            "a = BAD",
            "a[BAD] = 1'b0",
            "reg [BAD:0] fresh",
            "reg [3:0] fresh = BAD",
            "1'b0 ? BAD : 1'b1",
            "{1'b1, {0{BAD}}}",
        ] {
            let input = template.replace("BAD", bad);
            let err = session.eval(&input).expect_err(&input);
            assert!(
                err.starts_with("Semantic error:") && err.contains(diagnostic),
                "{input}: {err}"
            );
            assert!(err.task_output.is_empty(), "{input}");
            assert!(session.lookup("fresh").is_none(), "{input}");
            assert_eq!(
                session.eval("a").expect("session unchanged").output,
                "4'b1010"
            );
        }
    }
}

#[test]
fn invalid_concat_result_selects_leave_assignment_and_declaration_state_unchanged() {
    let mut session = Session::new();
    session
        .eval("reg [3:0] a = 4'b1010, b = 4'b0101")
        .expect("decls");
    for rhs in [
        "{100}[0]",
        "{1.0}[0]",
        "{$finish}[0]",
        "{a}[0 +: 0]",
        "{a}[0:3]",
        "{a}[1.0]",
    ] {
        let err = session.eval(&format!("a = {rhs}")).expect_err(rhs);
        assert!(err.starts_with("Semantic error:"), "{rhs}: {err}");
        assert!(err.task_output.is_empty(), "{rhs}");
        assert_eq!(session.eval("a").expect("a unchanged").output, "4'b1010");
    }
    let err = session
        .eval("reg [3:0] a = 4'b0, fresh = {100}[0]")
        .expect_err("failed initializer");
    assert!(err.contains("concatenation operand has indefinite width"));
    assert!(session.lookup("fresh").is_none());
    assert_eq!(
        session.eval("a").expect("prior binding preserved").output,
        "4'b1010"
    );
    let err = session
        .eval("{a, b[{1.0}[0]]} = 5'b0")
        .expect_err("failed lhs index");
    assert!(err.contains("concatenation operand cannot be real"));
    assert_eq!(session.eval("a").expect("a unchanged").output, "4'b1010");
    assert_eq!(session.eval("b").expect("b unchanged").output, "4'b0101");
}
