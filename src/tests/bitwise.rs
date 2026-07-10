use crate::evaluate_input;
use crate::lexer::{Token, tokenize};
use crate::parser::{BinaryOp, Expr, UnaryOp, parse_expression};

// ---------- Bitwise operators (~, &, |, ^, ~^/^~) ----------
//
// Expected values follow the LRM 1364-2005 §5.1.10 Tables 5-9..5-12 truth
// tables. Per LRM Table 5-22, per-bit `~` and the binary forms are
// context-determined like arithmetic: width = max(L(lhs), L(rhs)), signed
// iff both operands signed.

#[test]
fn tokenizes_bitwise_single_char_operators() {
    // Bare & and | are no longer rejected: they tokenize as their
    // bitwise forms when not followed by a second &/|.
    let amp = tokenize("4'd1 & 4'd0").expect("& should tokenize");
    let pipe = tokenize("4'd1 | 4'd0").expect("| should tokenize");
    let xor = tokenize("4'd1 ^ 4'd0").expect("^ should tokenize");
    let tilde = tokenize("~4'd0").expect("~ should tokenize");

    assert_eq!(amp[1], Token::BitwiseAnd);
    assert_eq!(pipe[1], Token::BitwiseOr);
    assert_eq!(xor[1], Token::BitwiseXor);
    assert_eq!(tilde[0], Token::Tilde);
}

#[test]
fn tokenizes_xnor_with_either_spelling() {
    // LRM 5.1.10: `^~` and `~^` denote the same operator; both lex to a
    // single BitwiseXnor token so downstream code does not branch on
    // spelling.
    let tilde_caret = tokenize("4'd1 ~^ 4'd0").expect("~^ should tokenize");
    let caret_tilde = tokenize("4'd1 ^~ 4'd0").expect("^~ should tokenize");

    assert_eq!(tilde_caret[1], Token::BitwiseXnor);
    assert_eq!(caret_tilde[1], Token::BitwiseXnor);
}

#[test]
fn double_amp_and_pipe_still_lex_as_logical() {
    // Greedy two-char matching must win over the bare bitwise tokens;
    // otherwise && would silently become two & tokens.
    let and = tokenize("4'd1 && 4'd0").expect("&& should tokenize");
    let or = tokenize("4'd1 || 4'd0").expect("|| should tokenize");

    assert_eq!(and[1], Token::LogicalAnd);
    assert_eq!(or[1], Token::LogicalOr);
}

#[test]
fn evaluates_bitwise_not_truth_table() {
    let zero = evaluate_input("~1'b0").expect("~0");
    let one = evaluate_input("~1'b1").expect("~1");
    let x = evaluate_input("~1'bx").expect("~x");
    let z = evaluate_input("~1'bz").expect("~z");

    assert_eq!(zero.output, "1'b1");
    assert_eq!(one.output, "1'b0");
    assert_eq!(x.output, "1'bx");
    assert_eq!(z.output, "1'bx");
}

#[test]
fn bitwise_not_flips_each_bit_independently() {
    // Per-bit operation: x and z fold to x; other bits flip. Crucially
    // there is no all-x short-circuit (unlike arithmetic), so known and
    // unknown bits coexist in the result.
    let mixed = evaluate_input("~4'b01xz").expect("~01xz");
    let all_zeros = evaluate_input("~4'b0000").expect("~0000");

    assert_eq!(mixed.output, "4'b10xx");
    assert_eq!(all_zeros.output, "4'b1111");
}

#[test]
fn bitwise_not_preserves_operand_base() {
    let binary = evaluate_input("~4'b0001").expect("binary ~");
    let hex = evaluate_input("~8'h0a").expect("hex ~");

    assert_eq!(binary.output, "4'b1110");
    assert_eq!(hex.output, "8'hf5");
}

#[test]
fn bitwise_not_chains() {
    // parse_unary recurses, so ~~ parses as ~(~x).
    let result = evaluate_input("~~4'b0101").expect("~~0101");
    assert_eq!(result.output, "4'b0101");
}

#[test]
fn bitwise_not_widens_through_outer_arithmetic_context() {
    // Self-determined: ~4'b0001 = 4'b1110. With outer + 0 (32-bit signed
    // 0 makes the shared context 32-bit unsigned), the operand widens to
    // 32 bits BEFORE the negation runs, so we get 32 ones except the
    // LSB. Leftmost-base wins, so result is binary.
    let widened = evaluate_input("~4'b0001 + 0").expect("widened ~");
    assert_eq!(widened.output, "32'b11111111111111111111111111111110");
}

#[test]
fn bitwise_not_binds_tighter_than_power() {
    // LRM Table 5-4: unary ~ is tighter than **, so `~4'd1 ** 2` parses
    // as `(~4'd1) ** 2`. ~4'd1 self-determined at 4-bit unsigned is
    // 4'b1110 = 14; 14**2 = 196; 196 mod 16 = 4. Result base inherits
    // from the lhs (decimal), so 4'd4.
    let expr = parse_expression("~4'd1 ** 2").expect("parse");
    match &expr {
        Expr::Binary {
            op: BinaryOp::Power,
            lhs,
            ..
        } => assert!(matches!(
            lhs.as_ref(),
            Expr::Unary {
                op: UnaryOp::BitwiseNot,
                ..
            }
        )),
        other => panic!("expected top-level **, got {other:?}"),
    }
    let result = evaluate_input("~4'd1 ** 2").expect("eval");
    assert_eq!(result.output, "4'd4");
}

#[test]
fn evaluates_bitwise_and_truth_table() {
    // LRM Table 5-9: 0 dominates AND, x/z elsewhere → x, only 1&1 yields 1.
    let zero_zero = evaluate_input("1'b0 & 1'b0").expect("0&0");
    let one_one = evaluate_input("1'b1 & 1'b1").expect("1&1");
    let zero_x = evaluate_input("1'b0 & 1'bx").expect("0&x");
    let one_x = evaluate_input("1'b1 & 1'bx").expect("1&x");
    let one_z = evaluate_input("1'b1 & 1'bz").expect("1&z");
    let x_z = evaluate_input("1'bx & 1'bz").expect("x&z");

    assert_eq!(zero_zero.output, "1'b0");
    assert_eq!(one_one.output, "1'b1");
    assert_eq!(zero_x.output, "1'b0");
    assert_eq!(one_x.output, "1'bx");
    assert_eq!(one_z.output, "1'bx");
    assert_eq!(x_z.output, "1'bx");
}

#[test]
fn evaluates_bitwise_or_truth_table() {
    // Symmetric to AND with 1 dominating.
    let zero_zero = evaluate_input("1'b0 | 1'b0").expect("0|0");
    let one_zero = evaluate_input("1'b1 | 1'b0").expect("1|0");
    let one_x = evaluate_input("1'b1 | 1'bx").expect("1|x");
    let zero_x = evaluate_input("1'b0 | 1'bx").expect("0|x");
    let zero_z = evaluate_input("1'b0 | 1'bz").expect("0|z");
    let x_z = evaluate_input("1'bx | 1'bz").expect("x|z");

    assert_eq!(zero_zero.output, "1'b0");
    assert_eq!(one_zero.output, "1'b1");
    assert_eq!(one_x.output, "1'b1");
    assert_eq!(zero_x.output, "1'bx");
    assert_eq!(zero_z.output, "1'bx");
    assert_eq!(x_z.output, "1'bx");
}

#[test]
fn evaluates_bitwise_xor_truth_table() {
    // XOR has no dominator: any x/z anywhere → x. Otherwise standard XOR.
    let zero_one = evaluate_input("1'b0 ^ 1'b1").expect("0^1");
    let one_one = evaluate_input("1'b1 ^ 1'b1").expect("1^1");
    let zero_zero = evaluate_input("1'b0 ^ 1'b0").expect("0^0");
    let one_x = evaluate_input("1'b1 ^ 1'bx").expect("1^x");
    let zero_z = evaluate_input("1'b0 ^ 1'bz").expect("0^z");

    assert_eq!(zero_one.output, "1'b1");
    assert_eq!(one_one.output, "1'b0");
    assert_eq!(zero_zero.output, "1'b0");
    assert_eq!(one_x.output, "1'bx");
    assert_eq!(zero_z.output, "1'bx");
}

#[test]
fn evaluates_bitwise_xnor_truth_table_with_either_spelling() {
    // ^~ and ~^ are the same operator (NOT-of-XOR semantics).
    let tilde_caret_eq = evaluate_input("1'b0 ~^ 1'b0").expect("0~^0");
    let caret_tilde_eq = evaluate_input("1'b0 ^~ 1'b0").expect("0^~0");
    let one_one = evaluate_input("1'b1 ^~ 1'b1").expect("1^~1");
    let mixed = evaluate_input("1'b1 ~^ 1'b0").expect("1~^0");
    let one_x = evaluate_input("1'b1 ~^ 1'bx").expect("1~^x");

    assert_eq!(tilde_caret_eq.output, "1'b1");
    assert_eq!(caret_tilde_eq.output, "1'b1");
    assert_eq!(one_one.output, "1'b1");
    assert_eq!(mixed.output, "1'b0");
    assert_eq!(one_x.output, "1'bx");
}

#[test]
fn bitwise_binary_zips_known_and_unknown_bits_per_position() {
    // The arithmetic all-x short-circuit does NOT apply: bitwise ops mix
    // known and unknown bits per position. Worked examples (bit 0 = LSB):
    //
    //   4'b1100 & 4'b10x1 → bits: 0&1=0, 0&x=0 (0 dominates), 1&0=0, 1&1=1 → 4'b1000
    //   4'b1100 | 4'b00x1 → bits: 0|1=1, 0|x=x, 1|0=1, 1|0=1            → 4'b11x1
    //   4'b1100 ^ 4'b00x1 → bits: 0^1=1, 0^x=x, 1^0=1, 1^0=1            → 4'b11x1
    let and = evaluate_input("4'b1100 & 4'b10x1").expect("mixed &");
    let or = evaluate_input("4'b1100 | 4'b00x1").expect("mixed |");
    let xor = evaluate_input("4'b1100 ^ 4'b00x1").expect("mixed ^");

    assert_eq!(and.output, "4'b1000");
    assert_eq!(or.output, "4'b11x1");
    assert_eq!(xor.output, "4'b11x1");
}

#[test]
fn bitwise_binary_uses_max_width_of_operands() {
    // Same width: trivially preserved.
    let same = evaluate_input("4'b1100 & 4'b1010").expect("4&4");
    // Mixed width (both unsigned → unsigned context): narrower operand
    // zero-extends to the wider width before zipping.
    //   8'hff = 8'b11111111; 4'b1010 zero-extends to 8'b00001010;
    //   AND → 8'b00001010.
    let mixed = evaluate_input("8'hff & 4'b1010").expect("8&4");

    assert_eq!(same.output, "4'b1000");
    assert_eq!(mixed.output, "8'h0a");
}

#[test]
fn bitwise_binary_signed_only_when_both_signed() {
    // Both signed → context signed → narrower side sign-extends.
    //   4'sb1111 sign-extends to 8'sb11111111;
    //   & 8'sb01010101 → 8'sb01010101.
    let both_signed = evaluate_input("4'sb1111 & 8'sb01010101").expect("both signed");
    // Mixed → context unsigned → narrower zero-extends.
    //   4'sb1111 zero-extends to 8'b00001111;
    //   & 8'b01010101 → 8'b00000101.
    let mixed = evaluate_input("4'sb1111 & 8'b01010101").expect("mixed");

    assert_eq!(both_signed.output, "8'sb01010101");
    assert_eq!(mixed.output, "8'b00000101");
}

#[test]
fn bitwise_extends_per_5_5_2_not_per_5_1_10() {
    // LRM §5.1.10 says the shorter operand "shall be zero-filled in the
    // most significant bit positions", but §5.5.2 says signed-signed
    // operands unify under a signed propagated context and the narrower
    // side sign-extends. The two rules disagree for `4'shF | 8'sh0`.
    // vcal follows §5.5.2 (the later SystemVerilog clarification drops the
    // §5.1.10 sentence):
    //   - both signed → sign-extend the narrower operand.
    //   - any unsigned → unsigned context → zero-extend.
    let both_signed = evaluate_input("4'shF | 8'sh0").expect("both signed");
    let mixed_unsigned = evaluate_input("4'shF | 8'h0").expect("mixed");

    assert_eq!(both_signed.output, "8'shff");
    assert_eq!(mixed_unsigned.output, "8'h0f");
}

#[test]
fn bitwise_binary_widens_through_outer_arithmetic_context() {
    // Without context-widening these would be 4-bit. With outer + 0
    // (32-bit signed 0 produces 32-bit unsigned shared context), the
    // bitwise operands widen to 32 bits BEFORE zipping. Leftmost-base
    // for the outer + is the bitwise op's binary base.
    let widened_and = evaluate_input("(4'b1100 & 4'b1010) + 0").expect("widened &");
    let widened_or = evaluate_input("(4'b0100 | 4'b1010) + 0").expect("widened |");
    let widened_xor = evaluate_input("(4'b0110 ^ 4'b1010) + 0").expect("widened ^");

    assert_eq!(widened_and.output, "32'b00000000000000000000000000001000");
    assert_eq!(widened_or.output, "32'b00000000000000000000000000001110");
    assert_eq!(widened_xor.output, "32'b00000000000000000000000000001100");
}

#[test]
fn outer_unsigned_context_propagates_through_nested_bitwise() {
    let widened = evaluate_input("(4'sb1000 | 4'sb0000) + 8'b0")
        .expect("nested bitwise expression should evaluate");
    assert_eq!(widened.output, "8'b00001000");
}

#[test]
fn bitwise_band_precedence_below_equality() {
    // `4'd1 == 4'd1 & 4'd1` parses as `(4'd1 == 4'd1) & 4'd1`. The 1-bit
    // 1'b1 zero-extends to 4'b0001 under the unified 4-bit context, then
    // & 4'b0001 → 4'b0001.
    let expr = parse_expression("4'd1 == 4'd1 & 4'd1").expect("parse");
    match &expr {
        Expr::Binary {
            op: BinaryOp::BitwiseAnd,
            lhs,
            ..
        } => assert!(matches!(
            lhs.as_ref(),
            Expr::Binary {
                op: BinaryOp::Equal,
                ..
            }
        )),
        other => panic!("expected top-level &, got {other:?}"),
    }
    let result = evaluate_input("4'd1 == 4'd1 & 4'd1").expect("eval");
    assert_eq!(result.output, "4'b0001");
}

#[test]
fn bitwise_band_precedence_above_logical_and() {
    // `4'd1 & 4'd1 && 4'd0` parses as `(4'd1 & 4'd1) && 4'd0`.
    let expr = parse_expression("4'd1 & 4'd1 && 4'd0").expect("parse");
    match &expr {
        Expr::Binary {
            op: BinaryOp::LogicalAnd,
            lhs,
            ..
        } => assert!(matches!(
            lhs.as_ref(),
            Expr::Binary {
                op: BinaryOp::BitwiseAnd,
                ..
            }
        )),
        other => panic!("expected top-level &&, got {other:?}"),
    }
    let result = evaluate_input("4'd1 & 4'd1 && 4'd0").expect("eval");
    assert_eq!(result.output, "1'b0");
}

#[test]
fn bitwise_internal_precedence_and_tightest_or_loosest() {
    // & > ^ > | per LRM Table 5-4.
    //
    //   4'b0110 ^ 4'b0011 & 4'b1100  →  4'b0110 ^ (4'b0011 & 4'b1100)
    //                                = 4'b0110 ^ 4'b0000 = 4'b0110
    //   4'b1000 | 4'b0001 ^ 4'b1010  →  4'b1000 | (4'b0001 ^ 4'b1010)
    //                                = 4'b1000 | 4'b1011 = 4'b1011
    let and_under_xor = evaluate_input("4'b0110 ^ 4'b0011 & 4'b1100").expect("eval");
    let xor_under_or = evaluate_input("4'b1000 | 4'b0001 ^ 4'b1010").expect("eval");

    assert_eq!(and_under_xor.output, "4'b0110");
    assert_eq!(xor_under_or.output, "4'b1011");
}

#[test]
fn bitwise_binary_is_left_associative() {
    // Same shape check used elsewhere: a OP b OP c parses as (a OP b) OP c.
    let expr = parse_expression("4'd1 & 4'd2 & 4'd3").expect("parse");
    match &expr {
        Expr::Binary {
            op: BinaryOp::BitwiseAnd,
            lhs,
            ..
        } => assert!(matches!(
            lhs.as_ref(),
            Expr::Binary {
                op: BinaryOp::BitwiseAnd,
                ..
            }
        )),
        other => panic!("expected top-level &, got {other:?}"),
    }

    // Cross-check XOR chain: (1^2)^3 = 3^3 = 0. Leftmost-base is
    // decimal (all operands `'d`), so the result renders as `4'd0`.
    let xor_chain = evaluate_input("4'd1 ^ 4'd2 ^ 4'd3").expect("eval");
    assert_eq!(xor_chain.output, "4'd0");
}

#[test]
fn bitwise_binary_inherits_leftmost_base() {
    // Same leftmost-wins rule as arithmetic.
    let hex_then_binary = evaluate_input("8'h0a & 8'b00001111").expect("hex&binary");
    let binary_then_hex = evaluate_input("8'b00001111 & 8'h0a").expect("binary&hex");

    assert_eq!(hex_then_binary.output, "8'h0a");
    assert_eq!(binary_then_hex.output, "8'b00001010");
}

// ---------- Reduction unary operators (& ~& | ~| ^ ~^/^~) ----------
//
// Expected values follow the LRM 1364-2005 §5.1.11 fold rules. Per LRM
// Table 5-22, reduction operands are self-determined and the result is
// always 1-bit unsigned that widens through outer arithmetic context like
// `!`, `&&`, `||`, relational, equality.

#[test]
fn tokenizes_nand_and_nor_as_single_tokens() {
    // ~& and ~| are unary-only operators (LRM A.8.6: binary_operator does
    // not list them). They must lex greedily as one token so the parser
    // can claim them at unary position without re-splitting.
    let nand = tokenize("~&4'b1111").expect("~& should tokenize");
    let nor = tokenize("~|4'b0000").expect("~| should tokenize");

    assert_eq!(nand[0], Token::BitwiseNand);
    assert_eq!(nor[0], Token::BitwiseNor);
}

#[test]
fn bare_tilde_unaffected_by_reduction_lexing() {
    // After the ~&/~|/~^ greedy paths, a bare ~ followed by anything else
    // (whitespace, digit-start, paren) must still produce a Tilde token.
    let spaced = tokenize("~ 4'd1").expect("~ + space");
    let parened = tokenize("~(4'd1)").expect("~(...)");

    assert_eq!(spaced[0], Token::Tilde);
    assert_eq!(parened[0], Token::Tilde);
}

#[test]
fn reduction_nand_nor_rejected_as_binary() {
    // No parse_bitwise_* level consumes BitwiseNand/BitwiseNor, so
    // `a ~& b` cleanly fails after the lhs is reduced to a primary.
    let nand = evaluate_input("4'd1 ~& 4'd1").expect_err("binary ~& rejected");
    let nor = evaluate_input("4'd0 ~| 4'd0").expect_err("binary ~| rejected");

    assert_eq!(
        nand,
        "Syntax error: unexpected token after end of statement"
    );
    assert_eq!(nor, "Syntax error: unexpected token after end of statement");
}

#[test]
fn evaluates_reduction_and_single_bit_truth_table() {
    // Single-bit reduction degenerates to identity for known values and
    // x for x/z (LRM §5.1.11).
    let zero = evaluate_input("&1'b0").expect("&0");
    let one = evaluate_input("&1'b1").expect("&1");
    let x = evaluate_input("&1'bx").expect("&x");
    let z = evaluate_input("&1'bz").expect("&z");

    assert_eq!(zero.output, "1'b0");
    assert_eq!(one.output, "1'b1");
    assert_eq!(x.output, "1'bx");
    assert_eq!(z.output, "1'bx");
}

#[test]
fn evaluates_reduction_or_single_bit_truth_table() {
    let zero = evaluate_input("|1'b0").expect("|0");
    let one = evaluate_input("|1'b1").expect("|1");
    let x = evaluate_input("|1'bx").expect("|x");
    let z = evaluate_input("|1'bz").expect("|z");

    assert_eq!(zero.output, "1'b0");
    assert_eq!(one.output, "1'b1");
    assert_eq!(x.output, "1'bx");
    assert_eq!(z.output, "1'bx");
}

#[test]
fn evaluates_reduction_xor_single_bit_truth_table() {
    let zero = evaluate_input("^1'b0").expect("^0");
    let one = evaluate_input("^1'b1").expect("^1");
    let x = evaluate_input("^1'bx").expect("^x");
    let z = evaluate_input("^1'bz").expect("^z");

    assert_eq!(zero.output, "1'b0");
    assert_eq!(one.output, "1'b1");
    assert_eq!(x.output, "1'bx");
    assert_eq!(z.output, "1'bx");
}

#[test]
fn evaluates_negated_reduction_single_bit_truth_tables() {
    // The negated forms are NOT-of-positive (per LRM 5.1.11 last
    // sentence). Single-bit cases: known → flipped, x/z → x.
    let nand_one = evaluate_input("~&1'b1").expect("~&1");
    let nand_zero = evaluate_input("~&1'b0").expect("~&0");
    let nand_x = evaluate_input("~&1'bx").expect("~&x");
    let nor_one = evaluate_input("~|1'b1").expect("~|1");
    let nor_zero = evaluate_input("~|1'b0").expect("~|0");
    let nor_z = evaluate_input("~|1'bz").expect("~|z");
    let xnor_one = evaluate_input("~^1'b1").expect("~^1");
    let xnor_zero = evaluate_input("~^1'b0").expect("~^0");
    let xnor_x = evaluate_input("~^1'bx").expect("~^x");

    assert_eq!(nand_one.output, "1'b0");
    assert_eq!(nand_zero.output, "1'b1");
    assert_eq!(nand_x.output, "1'bx");
    assert_eq!(nor_one.output, "1'b0");
    assert_eq!(nor_zero.output, "1'b1");
    assert_eq!(nor_z.output, "1'bx");
    assert_eq!(xnor_one.output, "1'b0");
    assert_eq!(xnor_zero.output, "1'b1");
    assert_eq!(xnor_x.output, "1'bx");
}

#[test]
fn xnor_reduction_accepts_either_spelling() {
    // ^~ and ~^ are the same operator at unary position too.
    let tilde_caret = evaluate_input("~^4'b1100").expect("~^");
    let caret_tilde = evaluate_input("^~4'b1100").expect("^~");

    // 4'b1100 has two 1s → XOR parity = 0 → XNOR = 1.
    assert_eq!(tilde_caret.output, "1'b1");
    assert_eq!(caret_tilde.output, "1'b1");
}

#[test]
fn reduction_and_folds_multi_bit_operand() {
    // 0 dominates AND-reduction even against x/z (because
    // bitwise_and_bits(0, x) = 0). Otherwise: any x/z → x; all-1 → 1.
    let all_ones = evaluate_input("&4'b1111").expect("&1111");
    let has_zero = evaluate_input("&4'b1101").expect("&1101");
    let zero_dominates_over_x = evaluate_input("&4'b110x").expect("&110x");
    let unknown_no_zero = evaluate_input("&4'b111x").expect("&111x");
    let unknown_no_zero_z = evaluate_input("&4'b111z").expect("&111z");
    let unknown_mixed = evaluate_input("&4'b1x1z").expect("&1x1z");

    assert_eq!(all_ones.output, "1'b1");
    assert_eq!(has_zero.output, "1'b0");
    assert_eq!(zero_dominates_over_x.output, "1'b0");
    assert_eq!(unknown_no_zero.output, "1'bx");
    assert_eq!(unknown_no_zero_z.output, "1'bx");
    assert_eq!(unknown_mixed.output, "1'bx");
}

#[test]
fn reduction_or_folds_multi_bit_operand() {
    // Symmetric: 1 dominates OR-reduction. all-0 → 0; any 1 → 1;
    // otherwise any x/z → x.
    let all_zeros = evaluate_input("|4'b0000").expect("|0000");
    let has_one = evaluate_input("|4'b0010").expect("|0010");
    let one_dominates_over_x = evaluate_input("|4'b001x").expect("|001x");
    let unknown_no_one = evaluate_input("|4'b000x").expect("|000x");
    let unknown_no_one_z = evaluate_input("|4'b000z").expect("|000z");

    assert_eq!(all_zeros.output, "1'b0");
    assert_eq!(has_one.output, "1'b1");
    assert_eq!(one_dominates_over_x.output, "1'b1");
    assert_eq!(unknown_no_one.output, "1'bx");
    assert_eq!(unknown_no_one_z.output, "1'bx");
}

#[test]
fn reduction_xor_folds_to_parity_and_x_on_unknowns() {
    // XOR has no dominator: any x/z anywhere → x. Otherwise standard
    // odd-parity.
    let even_parity = evaluate_input("^4'b1111").expect("^1111");
    let odd_parity = evaluate_input("^4'b1110").expect("^1110");
    let zero = evaluate_input("^4'b0000").expect("^0000");
    let unknown = evaluate_input("^4'b111x").expect("^111x");
    let unknown_with_zero = evaluate_input("^4'b110x").expect("^110x");
    let unknown_z = evaluate_input("^4'b00z0").expect("^00z0");

    assert_eq!(even_parity.output, "1'b0");
    assert_eq!(odd_parity.output, "1'b1");
    assert_eq!(zero.output, "1'b0");
    assert_eq!(unknown.output, "1'bx");
    assert_eq!(unknown_with_zero.output, "1'bx");
    assert_eq!(unknown_z.output, "1'bx");
}

#[test]
fn negated_reductions_fold_then_invert() {
    // Spot-check that NAND/NOR/XNOR are exactly NOT-of-positive across
    // multi-bit operands too.
    let nand_all_ones = evaluate_input("~&4'b1111").expect("~&1111");
    let nand_has_zero = evaluate_input("~&4'b1101").expect("~&1101");
    let nand_unknown = evaluate_input("~&4'b111x").expect("~&111x");
    let nor_all_zeros = evaluate_input("~|4'b0000").expect("~|0000");
    let nor_has_one = evaluate_input("~|4'b0010").expect("~|0010");
    let xnor_even = evaluate_input("~^4'b1111").expect("~^1111");
    let xnor_odd = evaluate_input("~^4'b1110").expect("~^1110");
    let xnor_unknown = evaluate_input("~^4'b111x").expect("~^111x");

    assert_eq!(nand_all_ones.output, "1'b0");
    assert_eq!(nand_has_zero.output, "1'b1");
    assert_eq!(nand_unknown.output, "1'bx");
    assert_eq!(nor_all_zeros.output, "1'b1");
    assert_eq!(nor_has_one.output, "1'b0");
    assert_eq!(xnor_even.output, "1'b1");
    assert_eq!(xnor_odd.output, "1'b0");
    assert_eq!(xnor_unknown.output, "1'bx");
}

#[test]
fn reduction_result_renders_in_binary_regardless_of_operand_base() {
    // Operand bases vary but the 1-bit reduction result is always
    // binary, like `!`/relational/equality.
    let hex_and = evaluate_input("&8'hff").expect("&hex");
    let hex_xor = evaluate_input("^8'h05").expect("^hex");
    let dec_or = evaluate_input("|4'd0").expect("|dec");

    assert_eq!(hex_and.output, "1'b1");
    assert_eq!(hex_xor.output, "1'b0");
    assert_eq!(dec_or.output, "1'b0");
}

#[test]
fn reduction_widens_through_outer_arithmetic_context() {
    // (&4'b1111) → 1'b1; outer + widens to the parent's 32-bit decimal
    // context. The reduction result's binary base wins the leftmost-base
    // rule, so the parent + renders in binary.
    let widened = evaluate_input("(&4'b1111) + 0").expect("widened &");
    let widened_xnor = evaluate_input("(~^4'b1110) + 0").expect("widened ~^");

    assert_eq!(widened.output, "32'b00000000000000000000000000000001");
    assert_eq!(widened_xnor.output, "32'b00000000000000000000000000000000");
}

#[test]
fn reduction_position_disambiguates_unary_from_binary_and() {
    // `&4'b1111` — pure unary reduction (1).
    // `4'd1 & &4'b1111` — binary AND with rhs = unary reduction (1 & 1 = 1).
    // `4'd1 & 4'd2` — pure binary AND (0).
    let pure_unary = evaluate_input("&4'b1111").expect("pure unary");
    let mixed = evaluate_input("4'd1 & &4'b1111").expect("binary + unary");
    let pure_binary = evaluate_input("4'd1 & 4'd2").expect("pure binary");

    assert_eq!(pure_unary.output, "1'b1");
    // 4'd1 (4 bits) & (reduced 1'b1, zero-extended to 4 bits = 4'b0001) = 4'b0001.
    // Leftmost-base is decimal (4'd1).
    assert_eq!(mixed.output, "4'd1");
    assert_eq!(pure_binary.output, "4'd0");
}

#[test]
fn reduction_chains_through_recursive_parse_unary() {
    // !!, ~~, and reduction stacks all flow through parse_unary
    // recursively. `&|4'b0110` parses as `&(|4'b0110)` → &(1'b1) → 1.
    // `~~&4'b1111` parses as `~(~(&4'b1111))` → ~(~1) → ~0 → 1.
    let nested_reductions = evaluate_input("&|4'b0110").expect("&|0110");
    let not_chains_into_reduction = evaluate_input("~~&4'b1111").expect("~~&1111");

    assert_eq!(nested_reductions.output, "1'b1");
    assert_eq!(not_chains_into_reduction.output, "1'b1");
}

#[test]
fn reduction_binds_tighter_than_power() {
    // LRM Table 5-4: unary reductions are at the unary level, tighter
    // than **. So `&4'b1111 ** 2` parses as `(&4'b1111) ** 2` = 1**2 = 1.
    let expr = parse_expression("&4'b1111 ** 2").expect("parse");
    match &expr {
        Expr::Binary {
            op: BinaryOp::Power,
            lhs,
            ..
        } => assert!(matches!(
            lhs.as_ref(),
            Expr::Unary {
                op: UnaryOp::ReductionAnd,
                ..
            }
        )),
        other => panic!("expected top-level **, got {other:?}"),
    }
    let result = evaluate_input("&4'b1111 ** 2").expect("eval");
    assert_eq!(result.output, "1'b1");
}
