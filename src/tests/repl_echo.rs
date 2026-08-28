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

#[test]
fn parse_only_exposes_the_unified_echo_ast() {
    let singleton = parse_input("1").expect("singleton AST");
    let multiple = parse_input("1, 2").expect("list AST");

    assert!(singleton.contains("ReplEcho"), "got: {singleton}");
    assert!(multiple.contains("ReplEcho"), "got: {multiple}");
    assert_eq!(multiple.matches("Expr(").count(), 2, "got: {multiple}");
}
