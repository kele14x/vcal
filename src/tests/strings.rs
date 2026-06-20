use crate::lexer::{Token, tokenize};
use crate::{Base, IntegerValue, Session, evaluate_input};

#[test]
fn tokenizes_string_literals_as_single_tokens() {
    assert_eq!(
        tokenize("\"// not a comment\"").expect("string token"),
        vec![Token::StringLiteral(b"// not a comment".to_vec())]
    );
    assert_eq!(
        tokenize("\"/* not a comment */\"").expect("string token"),
        vec![Token::StringLiteral(b"/* not a comment */".to_vec())]
    );
}

#[test]
fn evaluates_string_literals_as_packed_byte_vectors() {
    assert_eq!(
        evaluate_input("\"A\" == 8'h41")
            .expect("compare string")
            .output,
        "1'b1"
    );
    assert_eq!(
        evaluate_input("\"AB\" == 16'h4142")
            .expect("compare string")
            .output,
        "1'b1"
    );
    assert_eq!(
        evaluate_input("\"\" == 8'h00")
            .expect("compare empty string")
            .output,
        "1'b1"
    );
}

#[test]
fn displays_string_literals_as_escaped_text() {
    assert_eq!(evaluate_input("\"A\"").expect("string").output, "\"A\"");
    assert_eq!(
        evaluate_input("\"AB\"").expect("two-byte string").output,
        "\"AB\""
    );
    assert_eq!(
        evaluate_input("\"\"").expect("empty string").output,
        "\"\\000\""
    );
}

#[test]
fn decodes_string_literal_escapes() {
    assert_eq!(
        evaluate_input("\"\\\"\\\\\"")
            .expect("quote and slash escapes")
            .output,
        "\"\\\"\\\\\""
    );
    assert_eq!(
        evaluate_input("\"\\n\\t\"")
            .expect("control escapes")
            .output,
        "\"\\n\\t\""
    );
    assert_eq!(
        evaluate_input("\"\\101\"").expect("octal escape").output,
        "\"A\""
    );
}

#[test]
fn string_display_survives_string_only_concat_and_replication() {
    assert_eq!(
        evaluate_input("{\"A\", \"B\"}")
            .expect("string concat")
            .output,
        "\"AB\""
    );
    assert_eq!(
        evaluate_input("{\"A\", \"\", \"B\"}")
            .expect("string concat with empty")
            .output,
        "\"A\\000B\""
    );
    assert_eq!(
        evaluate_input("{2{\"A\"}}")
            .expect("string replication")
            .output,
        "\"AA\""
    );
}

#[test]
fn string_display_drops_for_numeric_contexts() {
    assert_eq!(
        evaluate_input("$hex(\"AB\")").expect("hex cast").output,
        "16'h4142"
    );
    assert_eq!(
        evaluate_input("$dec(\"AB\")").expect("decimal cast").output,
        "16'd16706"
    );
    assert_eq!(
        evaluate_input("{\"A\", 8'h42}")
            .expect("mixed concat")
            .output,
        "16'h4142"
    );
}

#[test]
fn empty_string_literal_is_one_nul_byte() {
    assert_eq!(
        evaluate_input("$bin(\"\")")
            .expect("empty string bin cast")
            .output,
        "8'b00000000"
    );
    assert_eq!(
        evaluate_input("$oct(\"\")")
            .expect("empty string oct cast")
            .output,
        "8'o000"
    );
    assert_eq!(
        evaluate_input("$dec(\"\")")
            .expect("empty string dec cast")
            .output,
        "8'd0"
    );
    assert_eq!(
        evaluate_input("$dec($signed(\"\"))")
            .expect("signed empty string dec cast")
            .output,
        "8'sd0"
    );
    assert_eq!(
        evaluate_input("$hex(\"\")")
            .expect("empty string hex cast")
            .output,
        "8'h00"
    );
    assert_eq!(
        evaluate_input("1 ? \"\" : \"\"")
            .expect("empty string conditional")
            .output,
        "8'h00"
    );
}

#[test]
fn internal_zero_width_numeric_display_renders_zero_digit() {
    assert_eq!(
        IntegerValue::computed(0, false, Base::Binary, Vec::new()).canonical(),
        "0'b0"
    );
    assert_eq!(
        IntegerValue::computed(0, false, Base::Octal, Vec::new()).canonical(),
        "0'o0"
    );
    assert_eq!(
        IntegerValue::computed(0, false, Base::Decimal, Vec::new()).canonical(),
        "0'd0"
    );
    assert_eq!(
        IntegerValue::computed(0, true, Base::Decimal, Vec::new()).canonical(),
        "0'sd0"
    );
    assert_eq!(
        IntegerValue::computed(0, false, Base::Hex, Vec::new()).canonical(),
        "0'h0"
    );
}

#[test]
fn string_assignments_follow_existing_width_context() {
    let mut session = Session::new();
    session.eval("reg [15:0] word = \"AB\"").expect("decl");
    assert_eq!(
        session.eval("word == 16'h4142").expect("read").output,
        "1'b1"
    );

    session
        .eval("reg [7:0] byte = \"AB\"")
        .expect("narrow decl");
    assert_eq!(session.eval("byte == 8'h42").expect("read").output, "1'b1");

    session.eval("reg [23:0] wide = \"A\"").expect("wide decl");
    assert_eq!(session.eval("wide == 24'h41").expect("read").output, "1'b1");

    assert_eq!(session.eval("word").expect("reg read").output, "16'h4142");
}

#[test]
fn rejects_malformed_string_literals() {
    assert_eq!(
        evaluate_input("\"unterminated").expect_err("unterminated string"),
        "Syntax error: unterminated string literal"
    );
    assert_eq!(
        evaluate_input("\"a\nb\"").expect_err("raw newline"),
        "Syntax error: newline in string literal"
    );
    assert_eq!(
        evaluate_input("\"\\r\"").expect_err("unsupported escape"),
        "Syntax error: unsupported string escape: \\r"
    );
    assert_eq!(
        evaluate_input("\"\\400\"").expect_err("octal out of range"),
        "Syntax error: octal escape out of byte range: 400"
    );
}
