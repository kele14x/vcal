use crate::{Session, evaluate_input, parse_input, run_repl};
use std::io::Cursor;

#[test]
fn singleton_echo_retains_existing_canonical_rendering() {
    let result = evaluate_input("4'ha").expect("singleton echo");
    assert_eq!(result.output, "4'ha");
    assert_eq!(result.value_output, b"4'ha");
    assert!(result.task_output.is_empty());
}

#[test]
fn echo_list_renders_each_unformatted_expression_canonically() {
    let result = evaluate_input("4'ha, 4'b0011, 1.5").expect("echo list");
    assert_eq!(result.output, "4'ha 4'b0011 1.5");
    assert_eq!(result.value_output, b"4'ha 4'b0011 1.5");
}

#[test]
fn echo_list_accepts_variables_and_arbitrary_expressions() {
    let mut session = Session::new();
    session
        .eval("reg [7:0] a = 8'h0a; reg [3:0] b = 4'b0011")
        .expect("declarations");

    let result = session.eval("a, $dec(b + 1)").expect("variable echo list");
    assert_eq!(result.output, "8'h0a 32'd4");
}

#[test]
fn leading_string_in_multi_argument_echo_uses_display_formatting() {
    let mut session = Session::new();
    session.eval("integer a = 10").expect("declaration");

    let result = session
        .eval("\"a is %d (hex %h)\", a, a")
        .expect("formatted echo");
    assert_eq!(result.output, "a is 10 (hex 0000000a)");
    assert_eq!(result.value_output, b"a is 10 (hex 0000000a)");
    assert!(result.task_output.is_empty());
}

#[test]
fn formatted_echo_renders_unconsumed_arguments_canonically() {
    let result = evaluate_input("\"label\", 8'hff").expect("canonical leftover");
    assert_eq!(result.output, "label 8'hff");

    let result = evaluate_input("\"value=%d\", 8'hff, 4'b1010")
        .expect("explicit control plus canonical leftover");
    assert_eq!(result.output, "value=255 4'b1010");

    let result = evaluate_input("\"label\", \"value\"").expect("canonical string leftover");
    assert_eq!(result.output, "label \"value\"");
}

#[test]
fn explicit_display_keeps_decimal_fallback_for_unconsumed_arguments() {
    let result = evaluate_input("$display(\"label\", 8'hff)").expect("display fallback");
    assert_eq!(result.task_output, b"label 255\n");
}

#[test]
fn singleton_string_retains_escaped_canonical_echo() {
    let result = evaluate_input("\"hello\\nworld\"").expect("string echo");
    assert_eq!(result.output, "\"hello\\nworld\"");
}

#[test]
fn nested_commas_remain_inside_their_expressions() {
    let result = evaluate_input("$signed(4'hf), {2'b10, 2'b01}").expect("nested commas");
    assert_eq!(result.output, "4'shf 4'b1001");
}

#[test]
fn null_echo_arguments_match_display_list_spacing() {
    let result = evaluate_input(", 1,, 4'hf,").expect("null echo arguments");
    assert_eq!(result.output, " 32'sd1 4'hf ");
}

#[test]
fn trailing_semicolon_suppresses_the_entire_echo_list() {
    let result = evaluate_input("1, 2;").expect("suppressed echo list");
    assert!(result.value_output.is_empty());
    assert_eq!(result.output, "");
    assert!(result.task_output.is_empty());
}

#[test]
fn semicolon_sequences_assignment_before_echo_list() {
    let mut session = Session::new();
    session.eval("integer a = 0").expect("declaration");

    let result = session.eval("a = 10; a, a + 1").expect("assign then echo");
    assert_eq!(result.output, "32'sd10 32'sd11");
}

#[test]
fn comma_does_not_turn_assignment_into_an_expression() {
    let mut session = Session::new();
    session.eval("integer a = 0").expect("declaration");

    let error = session.eval("a = 10, a").expect_err("assignment comma");
    assert_eq!(
        error,
        "Syntax error: assignment is a statement; use `;` before a REPL echo list"
    );
    assert_eq!(session.eval("a").expect("unchanged a").output, "32'sd0");
}

#[test]
fn only_the_last_echo_list_in_a_line_is_visible() {
    let result = evaluate_input("1, 2; 3, 4").expect("multiple echo lists");
    assert_eq!(result.output, "32'sd3 32'sd4");
}

#[test]
fn singleton_system_task_keeps_side_effect_semantics() {
    let result = evaluate_input("$display(\"hi\");").expect("explicit display");
    assert_eq!(result.task_output, b"hi\n");
    assert!(result.value_output.is_empty());
}

#[test]
fn system_task_cannot_be_an_item_in_an_echo_list() {
    let error = evaluate_input("$display(\"hi\"), 1").expect_err("task in echo list");
    assert!(
        error.contains("$display() is a system task, it cannot be called as a function"),
        "got: {error}"
    );
}

#[test]
fn formatted_echo_preserves_raw_bytes() {
    let result = evaluate_input("\"%s\", 8'ha9").expect("raw formatted echo");
    assert_eq!(result.value_output, vec![0xa9]);
    assert!(result.task_output.is_empty());
}

#[test]
fn piped_repl_prefixes_formatted_raw_echo_with_out_slot() {
    let mut input = Cursor::new("\"%s\", 8'ha9\n$finish\n");
    let mut output = Vec::new();

    run_repl(&mut input, &mut output).expect("REPL should run");

    assert_eq!(output, b"In [0]: Out[0]: \xa9\n\nIn [1]: \n");
}

// Exercise the private terminal styling path without depending on a real TTY.
// Public evaluate_input / Session::eval and the piped REPL still return raw bytes.
fn highlighted_echo(input: &str, enabled: bool) -> (crate::Evaluation, Vec<u8>) {
    let (result, spans) =
        crate::evaluate_input_with_session(&mut Session::new(), input).expect("styled echo");
    let bytes = crate::color::highlight_bytes(&result.value_output, &spans, enabled).into_owned();
    (result, bytes)
}

#[test]
fn canonical_echo_highlights_size_base_and_digits() {
    for (input, expected) in [
        (
            "4'ha",
            "\x1b[33m4\x1b[0m\x1b[2;33m'h\x1b[0m\x1b[33ma\x1b[0m",
        ),
        (
            "4'b10xz",
            "\x1b[33m4\x1b[0m\x1b[2;33m'b\x1b[0m\x1b[33m10xz\x1b[0m",
        ),
        (
            "6'o7z",
            "\x1b[33m6\x1b[0m\x1b[2;33m'o\x1b[0m\x1b[33m7z\x1b[0m",
        ),
        (
            "42",
            "\x1b[33m32\x1b[0m\x1b[2;33m'sd\x1b[0m\x1b[33m42\x1b[0m",
        ),
        (
            "-1",
            "-\x1b[33m32\x1b[0m\x1b[2;33m'sd\x1b[0m\x1b[33m1\x1b[0m",
        ),
        (
            "8'dx",
            "\x1b[33m8\x1b[0m\x1b[2;33m'd\x1b[0m\x1b[33mx\x1b[0m",
        ),
    ] {
        let (result, bytes) = highlighted_echo(input, true);
        assert_eq!(bytes, expected.as_bytes(), "{input}");
        assert!(!result.value_output.contains(&0x1b));
    }
}

#[test]
fn canonical_string_echo_highlights_quotes_and_escapes_together() {
    let input = "\"hello\\n// 8'hff\"";
    let (result, bytes) = highlighted_echo(input, true);
    assert_eq!(bytes, format!("\x1b[36m{input}\x1b[0m").as_bytes());
    assert_eq!(result.output, input);
}

#[test]
fn canonical_real_echo_highlights_finite_and_nonfinite_values() {
    for input in ["1.5", "-1.5", "1.0e20", "$ln(0.0)", "$sqrt(-1.0)"] {
        let (result, bytes) = highlighted_echo(input, true);
        assert_eq!(
            bytes,
            format!("\x1b[33m{}\x1b[0m", result.output).as_bytes()
        );
    }
}

#[test]
fn canonical_echo_highlighting_preserves_null_spacing() {
    let (result, bytes) = highlighted_echo(", 4'ha,, 1.5,", true);
    assert_eq!(result.value_output, b" 4'ha 1.5 ");
    assert_eq!(
        bytes,
        b" \x1b[33m4\x1b[0m\x1b[2;33m'h\x1b[0m\x1b[33ma\x1b[0m \x1b[33m1.5\x1b[0m "
    );
}

#[test]
fn format_text_and_substitutions_are_not_tokenized() {
    for input in [
        "\"8'hff // %d\", 42",
        "\"%h %s %c\", 8'hff, 8'ha9, 8'hff",
        "\"\\\"unterminated $bogus %% %d\", 1",
    ] {
        let (result, spans) =
            crate::evaluate_input_with_session(&mut Session::new(), input).expect("formatted echo");
        assert!(spans.is_empty(), "{input}");
        assert_eq!(
            crate::color::highlight_bytes(&result.value_output, &spans, true).as_ref(),
            result.value_output,
        );
    }
}

#[test]
fn formatted_echo_highlights_only_canonical_leftovers() {
    let (result, bytes) = highlighted_echo("\"8'hff %d\", 1, 4'b1010, \"text\"", true);
    assert_eq!(result.value_output, b"8'hff 1 4'b1010 \"text\"");
    assert_eq!(
        bytes,
        b"8'hff 1 \x1b[33m4\x1b[0m\x1b[2;33m'b\x1b[0m\x1b[33m1010\x1b[0m \x1b[36m\"text\"\x1b[0m"
    );
}

#[test]
fn raw_formatted_bytes_do_not_shift_canonical_spans() {
    let (result, bytes) = highlighted_echo("\"%s%c\", 8'ha9, 8'hff, 4'ha", true);
    assert_eq!(result.value_output, b"\xa9\xff 4'ha");
    assert_eq!(
        bytes,
        b"\xa9\xff \x1b[33m4\x1b[0m\x1b[2;33m'h\x1b[0m\x1b[33ma\x1b[0m"
    );
}

#[test]
fn unicode_formatted_text_preserves_byte_offsets() {
    let (result, bytes) = highlighted_echo("\"h%sllo\", 16'hc3a9, 4'ha", true);
    assert_eq!(result.output, "héllo 4'ha");
    assert_eq!(
        bytes,
        "héllo \x1b[33m4\x1b[0m\x1b[2;33m'h\x1b[0m\x1b[33ma\x1b[0m".as_bytes()
    );
}

#[test]
fn disabled_output_highlighting_preserves_all_original_bytes() {
    for input in ["4'ha, 1.5, \"hello\"", "\"%s\", 8'ha9, 4'ha"] {
        let (result, bytes) = highlighted_echo(input, false);
        assert_eq!(bytes, result.value_output);
    }
}

#[test]
fn suppressed_and_task_outputs_have_no_value_spans() {
    for input in [
        "",
        "1;",
        "1; integer a",
        "1; $finish",
        "$display(42)",
        "$write(8'hff)",
    ] {
        let (result, spans) =
            crate::evaluate_input_with_session(&mut Session::new(), input).expect("no echo");
        assert!(result.value_output.is_empty(), "{input}");
        assert!(spans.is_empty(), "{input}");
        assert!(!result.task_output.contains(&0x1b));
    }
}

#[test]
fn only_final_echo_has_highlighting_spans() {
    let (result, bytes) = highlighted_echo("4'ha; 1.5", true);
    assert_eq!(result.value_output, b"1.5");
    assert_eq!(bytes, b"\x1b[33m1.5\x1b[0m");
}

#[test]
fn parse_only_exposes_the_unified_echo_ast() {
    let singleton = parse_input("1").expect("singleton AST");
    let multiple = parse_input("1, 2").expect("list AST");

    assert!(singleton.contains("ReplEcho"), "got: {singleton}");
    assert!(multiple.contains("ReplEcho"), "got: {multiple}");
    assert_eq!(multiple.matches("Expr(").count(), 2, "got: {multiple}");
}
