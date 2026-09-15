use crate::lexer::{Token, tokenize};
use crate::{Session, evaluate_input, run_repl};
use std::io::Cursor;

#[test]
fn runs_repl_until_exit_command() {
    let mut input = Cursor::new("42\n$finish\nignored\n");
    let mut output = Vec::new();

    run_repl(&mut input, &mut output).expect("REPL should run");

    let output = String::from_utf8(output).expect("output should be valid UTF-8");
    assert_eq!(output, "In [0]: Out[0]: 32'sd42\n\nIn [1]: \n");
}

#[test]
fn repl_prints_display_task_output_without_out_prefix() {
    let mut input = Cursor::new("$display(\"hi\")\n$finish\n");
    let mut output = Vec::new();

    run_repl(&mut input, &mut output).expect("REPL should run");

    let output = String::from_utf8(output).expect("output should be valid UTF-8");
    assert_eq!(output, "In [0]: hi\n\nIn [1]: \n");
}

#[test]
fn repl_writes_display_task_output_as_raw_bytes() {
    let mut input = Cursor::new("$display(\"%s\", 8'ha9)\n$finish\n");
    let mut output = Vec::new();

    run_repl(&mut input, &mut output).expect("REPL should run");

    assert_eq!(output, b"In [0]: \xa9\n\nIn [1]: \n");
}

#[test]
fn repl_emits_error_lines_and_continues_to_next_prompt() {
    // On evaluation failure the REPL prints the message on its own line
    // (the message already carries a stage prefix like `Syntax error:`
    // / `Semantic error:` when one applies) followed by a blank
    // separator, then advances the index and prompts for the next
    // input — it does not abort or skip the index. Sequence: bad input
    // → error, then valid input → result, then exit.
    let mut input = Cursor::new("1 +\n42\n$finish\n");
    let mut output = Vec::new();

    run_repl(&mut input, &mut output).expect("REPL should run");

    let output = String::from_utf8(output).expect("output should be valid UTF-8");
    assert_eq!(
        output,
        "In [0]: Syntax error: unexpected end of expression\n\
         \n\
         In [1]: Out[1]: 32'sd42\n\
         \n\
         In [2]: \n",
    );
}

// A `$display` / `$write` that already ran is a side effect that happened, so
// an error later in the same input must not swallow it. The diagnostic still
// aborts the remaining statements and still suppresses the value echo.
#[test]
fn repl_preserves_completed_task_output_before_error() {
    let mut input = Cursor::new("$display(\"one\"); $display(\"two\"); missing\n$finish\n");
    let mut output = Vec::new();

    run_repl(&mut input, &mut output).expect("REPL should run");

    assert_eq!(
        output,
        b"In [0]: one\ntwo\nSemantic error: undeclared identifier: missing\n\
          \n\
          In [1]: \n"
    );
}

#[test]
fn repl_preserves_write_output_without_adding_newline() {
    let mut input =
        Cursor::new("integer a = 0;\na = 1; $write(\"checkpoint\"); missing\n$finish\n");
    let mut output = Vec::new();

    run_repl(&mut input, &mut output).expect("REPL should run");

    // `$write` emits no trailing newline, so the diagnostic follows the
    // preserved bytes directly.
    assert_eq!(
        output,
        b"In [0]: \nIn [1]: checkpointSemantic error: undeclared identifier: missing\n\
          \n\
          In [2]: \n"
    );
}

#[test]
fn error_carries_completed_task_output_and_keeps_earlier_mutation() {
    let mut session = Session::new();
    session.eval("integer a = 0;").expect("decl");

    let err = session
        .eval("a = 1; $display(\"checkpoint\"); missing")
        .expect_err("undeclared identifier");

    assert_eq!(err.task_output, b"checkpoint\n");
    assert!(
        err.contains("undeclared identifier: missing"),
        "unexpected diagnostic: {err}"
    );
    // The assignment from the statement before the failure still stands.
    assert_eq!(session.eval("a").expect("session alive").output, "32'sd1");
}

#[test]
fn error_still_suppresses_value_echo() {
    let mut input = Cursor::new("1+1; missing\n$finish\n");
    let mut output = Vec::new();

    run_repl(&mut input, &mut output).expect("REPL should run");

    // Preserving task output must not start echoing values from a failed
    // input — no `Out[n]` line here, matching the IPython-style behaviour.
    assert_eq!(
        output,
        b"In [0]: Semantic error: undeclared identifier: missing\n\
          \n\
          In [1]: \n"
    );
}

#[test]
fn error_in_first_statement_carries_no_task_output() {
    let err = evaluate_input("missing; $display(\"never\")").expect_err("first statement fails");

    // Nothing completed, and the statements after the failure never ran.
    assert!(err.task_output.is_empty());
}

#[test]
fn parse_error_carries_no_task_output() {
    // Parsing happens before any statement runs, so there is nothing to
    // preserve even though a `$display` appears earlier in the input.
    let err = evaluate_input("$display(\"checkpoint\"); 1 +").expect_err("trailing operator");

    assert!(err.task_output.is_empty());
    assert!(err.starts_with("Syntax error:"));
}

// Stage-prefix sanity: the `Syntax error:` / `Semantic error:` prefixes
// tell the user which phase rejected their input. Parser/lexer errors get
// the syntax prefix; static-semantics errors get the semantic prefix.
// Genuine runtime conditions (division by zero, out-of-range bit-select,
// etc.) are absorbed into x bits per LRM and never surface as Err, so every
// `Err` a user can reach carries one of the two prefixes.
//
// Display format controls nearly broke that invariant. They are a static
// property of the input — a format string can only be literal-derived, since
// `DisplayStyle::String` survives no packed `reg` — but the check used to sit
// in the renderer, so its three rejections surfaced with no prefix at all.
// They are validated while the display argument list is prepared instead;
// `format_prefix_covers_display_format_rejections` below pins the result.
#[test]
fn syntax_error_prefix_distinguishes_stage() {
    let err = evaluate_input("1 +").expect_err("trailing operator");
    assert!(
        err.starts_with("Syntax error:"),
        "parser errors should carry the Syntax error prefix, got: {err}"
    );
}

#[test]
fn semantic_error_prefix_distinguishes_stage() {
    // `$bitstoreal(1'b0)` parses cleanly; the rejection is the static
    // "argument must be 64 bits wide" check that lives in the validator.
    let err = evaluate_input("$bitstoreal(1'b0)").expect_err("wrong-width $bitstoreal");
    assert!(
        err.starts_with("Semantic error:"),
        "validator errors should carry the Semantic error prefix, got: {err}"
    );
}

#[test]
fn format_prefix_covers_display_format_rejections() {
    // Format-string rejections are static-semantic checks, so they carry the
    // same prefix as any other validator rejection. These compare the whole
    // message rather than using `contains`: the failure modes being guarded
    // are a *missing* prefix and a *doubled* one, and a substring assertion
    // is blind to both — which is how the unprefixed form went unnoticed.
    let err = evaluate_input("$display(\"%h\")").expect_err("control with no argument");
    assert_eq!(
        err,
        "Semantic error: display format %h expects an argument, got 0"
    );

    let err = evaluate_input("$display(\"%z\", 42)").expect_err("unsupported control");
    assert_eq!(
        err,
        "Semantic error: unsupported display format control `%z`"
    );

    let err = evaluate_input("$display(\"value: %\")").expect_err("trailing percent");
    assert_eq!(
        err,
        "Semantic error: display format control `%` is missing a specifier"
    );

    // The formatted `display_expression` echo shares the same validation, so
    // it picks up the same prefix.
    let err = evaluate_input("\"a=%z\", 1").expect_err("echo path");
    assert_eq!(
        err,
        "Semantic error: unsupported display format control `%z`"
    );

    // Guard against over-applying the check: a *lone* string stays a canonical
    // echo rather than becoming a zero-argument format string, so its trailing
    // `%` is ordinary text and not a rejected control.
    let result = evaluate_input("\"tail %\"").expect("lone string is still an echo");
    assert_eq!(result.output, "\"tail %\"");
}

#[test]
fn trailing_semicolons_suppress_expression_output() {
    // IPython-style suppression: trailing `;` makes the REPL print a
    // blank line instead of the expression's value. Any number of
    // trailing `;` is treated the same (the empty statements between
    // them produce nothing of their own).
    let result = evaluate_input("1 + 1;;").expect("eval");
    assert_eq!(result.output, "");
}

#[test]
fn trailing_semicolons_with_intervening_whitespace_still_suppress() {
    // Whitespace between (and after) `;` separators doesn't change the
    // suppression rule: the last meaningful token is still a `;`.
    let result = evaluate_input("1 + 1 ; ; ;").expect("eval");
    assert_eq!(result.output, "");
}

#[test]
fn only_semicolons_produces_empty_output() {
    let result = evaluate_input(";;;").expect("eval");
    assert_eq!(result.output, "");
}

// IPython-style output suppression: the REPL prints the last
// statement's value iff it's an expression AND the input does not end
// with `;`. Everything else collapses to a blank line. Assignments,
// declarations, and system tasks never echo a value.

#[test]
fn expr_without_semicolon_prints_value() {
    let result = evaluate_input("1 + 1").expect("eval");
    assert_eq!(result.output, "32'sd2");
}

#[test]
fn expr_with_trailing_semicolon_is_suppressed() {
    let result = evaluate_input("1 + 1;").expect("eval");
    assert_eq!(result.output, "");
}

#[test]
fn multi_expr_prints_only_last_value() {
    let result = evaluate_input("1; 2; 3").expect("eval");
    assert_eq!(result.output, "32'sd3");
}

#[test]
fn multi_expr_with_trailing_semicolon_is_suppressed() {
    let result = evaluate_input("1; 2; 3;").expect("eval");
    assert_eq!(result.output, "");
}

#[test]
fn decl_only_is_suppressed() {
    let mut session = Session::new();
    let result = session.eval("reg [7:0] a").expect("decl");
    assert_eq!(result.output, "");
}

#[test]
fn assign_only_is_suppressed() {
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("decl");
    let result = session.eval("a = 5").expect("assign");
    assert_eq!(result.output, "");
}

#[test]
fn decl_then_expr_prints_expr() {
    let mut session = Session::new();
    let result = session.eval("reg [7:0] a; a").expect("decl then expr");
    assert_eq!(result.output, "8'bxxxxxxxx");
}

#[test]
fn expr_then_decl_is_suppressed() {
    let mut session = Session::new();
    let result = session.eval("1 + 1; reg [7:0] a").expect("expr then decl");
    assert_eq!(result.output, "");
}

#[test]
fn repl_multi_expr_input_uses_one_in_out_slot() {
    // Integration test for the fix to the "1; 2; 3" issue: each input
    // line gets exactly one `In [n]:` and at most one `Out[n]:` slot,
    // even when it contains multiple statements.
    let mut input = Cursor::new("1; 2; 3\n$finish\n");
    let mut output = Vec::new();

    run_repl(&mut input, &mut output).expect("REPL should run");

    let output = String::from_utf8(output).expect("output should be valid UTF-8");
    assert_eq!(output, "In [0]: Out[0]: 32'sd3\n\nIn [1]: \n");
}

#[test]
fn empty_input_produces_empty_output() {
    let result = evaluate_input("").expect("eval");
    assert_eq!(result.output, "");
}

#[test]
fn whitespace_only_input_produces_empty_output() {
    let result = evaluate_input("   \t  ").expect("eval");
    assert_eq!(result.output, "");
}

#[test]
fn line_comment_is_skipped() {
    let only = evaluate_input("// foo").expect("eval");
    assert_eq!(only.output, "");

    let trailing = evaluate_input("1 + 2 // tail").expect("eval");
    assert_eq!(trailing.output, "32'sd3");
}

#[test]
fn block_comment_is_skipped() {
    let only = evaluate_input("/* foo */").expect("eval");
    assert_eq!(only.output, "");

    let inline = evaluate_input("1 + /* x */ 2").expect("eval");
    assert_eq!(inline.output, "32'sd3");

    let many = evaluate_input("/* a */ 1 /* b */ + /* c */ 2 // tail").expect("eval");
    assert_eq!(many.output, "32'sd3");
}

#[test]
fn line_comment_ends_at_newline() {
    let tokens = tokenize("a // b\nc").expect("tokenize");
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[0], Token::Identifier("a".to_string()));
    assert_eq!(tokens[1], Token::Identifier("c".to_string()));
}

#[test]
fn unterminated_block_comment_is_an_error() {
    let err = evaluate_input("/* unterminated").expect_err("should error");
    assert_eq!(err, "Syntax error: unterminated block comment");
}

// The Windows-console byte policy: raw formatter bytes pass through untouched
// on every destination except a Windows console, where Rust's stdio rejects
// non-UTF-8 writes. Valid UTF-8 (which is all canonical output) is a no-op
// everywhere, so only deliberate `%s` / `%c` raw bytes are affected.

#[test]
fn console_safe_bytes_preserves_raw_bytes_off_windows_console() {
    let raw = [0xa9u8, 0xff];
    assert_eq!(crate::console_safe_bytes(&raw, false).as_ref(), raw);
}

#[test]
fn console_safe_bytes_preserves_valid_utf8_everywhere() {
    let utf8 = "label 8'hff";
    assert_eq!(
        crate::console_safe_bytes(utf8.as_bytes(), false).as_ref(),
        utf8.as_bytes()
    );
    assert_eq!(
        crate::console_safe_bytes(utf8.as_bytes(), true).as_ref(),
        utf8.as_bytes()
    );
}

#[test]
fn console_safe_bytes_lossily_converts_raw_bytes_on_windows_console() {
    let raw = [0xa9u8, 0xff];
    let expected = "\u{fffd}\u{fffd}".as_bytes();
    assert_eq!(crate::console_safe_bytes(&raw, true).as_ref(), expected);
}
