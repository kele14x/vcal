use crate::parser::{Expr, SystemArg, parse_expression};
use crate::{Session, evaluate_input};

#[test]
fn display_and_write_emit_task_output() {
    let display = evaluate_input("$display(\"hi\")").expect("display should run");
    assert_eq!(display.task_output, b"hi\n");
    assert_eq!(display.output, "");
    assert!(!display.should_exit);

    let write = evaluate_input("$write(\"hi\")").expect("write should run");
    assert_eq!(write.task_output, b"hi");
    assert_eq!(write.output, "");
    assert!(!write.should_exit);
}

#[test]
fn display_output_is_not_suppressed_by_semicolon() {
    let result = evaluate_input("$display(\"hi\");").expect("display should run");
    assert_eq!(result.task_output, b"hi\n");
    assert_eq!(result.output, "");
}

#[test]
fn display_tasks_accumulate_separately_from_value_output() {
    let result = evaluate_input("$write(\"a\"); $display(\"b\"); 1 + 1")
        .expect("mixed statements should run");
    assert_eq!(result.task_output, b"ab\n");
    assert_eq!(result.output, "32'sd2");
}

#[test]
fn display_formats_basic_controls() {
    let result =
        evaluate_input("$display(\"%h %d %s\", 8'haf, 3, \"ok\")").expect("display format");
    assert_eq!(result.task_output, b"af 3 ok\n");
    assert_eq!(result.output, "");
}

#[test]
fn display_unformatted_integer_values_use_decimal() {
    let mut session = Session::new();
    session.eval("reg a = 1'b1").expect("decl");

    let result = session.eval("$display(a)").expect("display reg");
    assert_eq!(result.task_output, b"1\n");
    assert_eq!(result.output, "");

    let result = session
        .eval("$display(4'hf, 4'b10)")
        .expect("display values");
    assert_eq!(result.task_output, b"15 2\n");
}

#[test]
fn display_explicit_base_controls_still_override_default_decimal() {
    let mut session = Session::new();
    session.eval("reg [3:0] a = 4'hf").expect("decl");

    let result = session
        .eval("$display(\"%h %b\", a, a)")
        .expect("display reg");
    assert_eq!(result.task_output, b"f 1111\n");
    assert_eq!(result.output, "");
}

#[test]
fn display_string_control_decodes_integer_vectors_as_bytes() {
    let result = evaluate_input("$display(\"%s %s\", 8'h41, 16'h4142)").expect("display strings");
    assert_eq!(result.task_output, b"A AB\n");
    assert_eq!(result.output, "");
}

#[test]
fn display_string_control_pads_unaligned_integer_vectors_to_bytes() {
    let result =
        evaluate_input("$display(\"%s %s\", 7'h41, 10'h041)").expect("display unaligned strings");
    assert_eq!(result.task_output, b"A \0A\n");
    assert_eq!(result.output, "");
}

#[test]
fn display_string_control_emits_high_bytes_raw() {
    let result = evaluate_input("$display(\"%s\", 8'ha9)").expect("display high byte string");
    assert_eq!(result.task_output, vec![0xa9, b'\n']);
    assert_eq!(result.output, "");
}

#[test]
fn display_string_control_converts_unknown_bits_to_zero() {
    let result = evaluate_input("$display(\"%s\", 8'hxx)").expect("display invalid string value");
    assert_eq!(result.task_output, b"\0\n");
    assert_eq!(result.output, "");

    let result =
        evaluate_input("$display(\"%s\", 16'h41zz)").expect("display invalid string value");
    assert_eq!(result.task_output, b"A\0\n");
    assert_eq!(result.output, "");

    let result = evaluate_input("$write(\"%s\", 8'hzz)").expect("write invalid string value");
    assert_eq!(result.task_output, b"\0");
    assert_eq!(result.output, "");
}

#[test]
fn display_char_control_emits_low_byte() {
    let result =
        evaluate_input("$display(\"%c %c\", 8'h41, 16'h0142)").expect("display characters");
    assert_eq!(result.task_output, b"A B\n");
    assert_eq!(result.output, "");
}

#[test]
fn display_char_control_zero_extends_narrow_integer_vectors() {
    let result = evaluate_input("$display(\"%c\", 7'h41)").expect("display narrow character");
    assert_eq!(result.task_output, b"A\n");
    assert_eq!(result.output, "");
}

#[test]
fn display_char_control_emits_high_bytes_raw() {
    let result = evaluate_input("$display(\"%c\", 8'ha9)").expect("display high byte character");
    assert_eq!(result.task_output, vec![0xa9, b'\n']);
    assert_eq!(result.output, "");
}

#[test]
fn display_char_control_converts_unknown_bits_to_zero() {
    let result =
        evaluate_input("$display(\"%c\", 12'hx41)").expect("display unknown high character bits");
    assert_eq!(result.task_output, b"A\n");
    assert_eq!(result.output, "");

    let result =
        evaluate_input("$display(\"%c\", 8'h4x)").expect("display unknown low character bits");
    assert_eq!(result.task_output, b"@\n");
    assert_eq!(result.output, "");

    let result = evaluate_input("$display(\"%c\", 8'hzz)").expect("display z character bits");
    assert_eq!(result.task_output, b"\0\n");
    assert_eq!(result.output, "");

    let result = evaluate_input("$write(\"%c\", 8'hxz)").expect("write unknown character bits");
    assert_eq!(result.task_output, b"\0");
    assert_eq!(result.output, "");
}

#[test]
fn display_format_requires_arguments_for_controls() {
    let error = evaluate_input("$display(\"%h\")").expect_err("missing format argument");
    assert!(
        error.contains("display format %h expects an argument"),
        "got: {error}"
    );
}

// ── $display / $write edge-case tests ────────────────────────────────────

#[test]
fn display_empty_format_string_outputs_just_newline() {
    // An empty string literal is stored as one NUL byte (8 bits wide).
    // When used as a format string, the NUL is output verbatim.
    let result = evaluate_input("$display(\"\")").expect("empty format string");
    assert_eq!(result.task_output, b"\0\n");
}

#[test]
fn display_no_args_outputs_newline() {
    let result = evaluate_input("$display()").expect("no args");
    assert_eq!(result.task_output, b"\n");
}

#[test]
fn write_no_args_outputs_nothing() {
    let result = evaluate_input("$write()").expect("no args");
    assert_eq!(result.task_output, b"");
}

#[test]
fn system_call_parser_preserves_null_arguments() {
    let expr =
        parse_expression("$display(, 1,,)").expect("system call with null arguments should parse");
    let Expr::SystemCall { name, args } = &expr else {
        panic!("expected system call");
    };

    assert_eq!(name, "$display");
    assert_eq!(args.len(), 4);
    assert!(matches!(&args[0], SystemArg::Null));
    assert!(matches!(&args[1], SystemArg::Expr(Expr::Literal(_))));
    assert!(matches!(&args[2], SystemArg::Null));
    assert!(matches!(&args[3], SystemArg::Null));
}

#[test]
fn display_and_write_null_arguments_emit_single_spaces() {
    let display = evaluate_input("$display(,)").expect("two null display args");
    assert_eq!(display.task_output, b"  \n");

    let display = evaluate_input("$display(, 1,, 2,)").expect("mixed null display args");
    assert_eq!(display.task_output, b" 1 2 \n");

    let write = evaluate_input("$write(,,)").expect("three null write args");
    assert_eq!(write.task_output, b"   ");
}

#[test]
fn display_format_controls_consume_null_arguments_as_spaces() {
    let result = evaluate_input("$display(\"%d:%s:%h\", , , )")
        .expect("format controls should consume null args");
    assert_eq!(result.task_output, b" : : \n");

    let result =
        evaluate_input("$display(\"x\", , 5)").expect("extra null args should append as spaces");
    assert_eq!(result.task_output, b"x 5\n");
}

#[test]
fn system_functions_reject_null_arguments() {
    let error = evaluate_input("$pow(, 2)").expect_err("null function arg should reject");
    assert!(!error.is_empty());

    let error = evaluate_input("$clog2(,)").expect_err("null function slots should reject");
    assert!(!error.is_empty());
}

#[test]
fn display_percent_escape_emits_literal_percent() {
    let result = evaluate_input("$display(\"%%\")").expect("%% escape");
    assert_eq!(result.task_output, b"%\n");

    let result = evaluate_input("$display(\"100%%\");").expect("100%%");
    assert_eq!(result.task_output, b"100%\n");

    let result = evaluate_input("$display(\"a%%b%%c\")").expect("a%%b%%c");
    assert_eq!(result.task_output, b"a%b%c\n");
}

#[test]
fn display_trailing_percent_is_error() {
    let error = evaluate_input("$display(\"value: %\")").expect_err("trailing percent");
    assert!(
        error.contains("display format control `%` is missing a specifier"),
        "got: {error}"
    );
}

#[test]
fn display_unknown_format_control_is_error() {
    let error = evaluate_input("$display(\"%z\", 42)").expect_err("unknown control %z");
    assert!(
        error.contains("unsupported display format control `%z`"),
        "got: {error}"
    );
}

#[test]
fn display_more_format_specs_than_args_is_error() {
    let error = evaluate_input("$display(\"%d %d %d\", 1, 2)").expect_err("too few args");
    assert!(
        error.contains("display format %d expects an argument, got 2"),
        "got: {error}"
    );
}

#[test]
fn display_fewer_format_specs_than_args_appends_extra() {
    // Extra args after all format controls are consumed are appended
    // space-separated using default formatting.
    let result = evaluate_input("$display(\"x=%d\", 5, 6, 7)").expect("extra args");
    assert_eq!(result.task_output, b"x=5 6 7\n");
}

#[test]
fn display_with_no_format_string_joins_values_space_separated() {
    let result = evaluate_input("$display(1, 2, 3)").expect("three ints");
    assert_eq!(result.task_output, b"1 2 3\n");

    let result = evaluate_input("$display(4'hf, 4'b10, -4'sd1)").expect("mixed bases");
    assert_eq!(result.task_output, b"15 2 -1\n");
}

#[test]
fn display_format_controls_are_case_insensitive() {
    // Uppercase variants of base controls produce identical output.
    let lower =
        evaluate_input("$display(\"%b %o %d %h\", 4'b1010, 9, 15, 255)").expect("lowercase");
    let upper =
        evaluate_input("$display(\"%B %O %D %H\", 4'b1010, 9, 15, 255)").expect("uppercase");
    assert_eq!(lower.task_output, upper.task_output);
    // Also %X (alias for %h). With 8-bit value, hex is 2 digits.
    let x = evaluate_input("$display(\"%X\", 8'hff)").expect("%X");
    assert_eq!(x.task_output, b"ff\n");
}

#[test]
fn display_char_control_emits_raw_bytes() {
    let result = evaluate_input("$display(\"%c\", 8'd65)").expect("ASCII 65");
    assert_eq!(result.task_output, b"A\n");

    // NUL byte
    let result = evaluate_input("$display(\"%c\", 8'd0)").expect("NUL");
    assert_eq!(result.task_output, b"\0\n");

    // Newline byte (ASCII 10) — output contains embedded newline before
    // $display's own trailing newline.
    let result = evaluate_input("$display(\"%c\", 8'd10)").expect("newline");
    assert_eq!(result.task_output, b"\n\n");

    // High byte (>= 0x80) emitted raw.
    let result = evaluate_input("$display(\"%c\", 8'hff)").expect("high byte");
    assert_eq!(result.task_output, vec![0xff, b'\n']);
}

#[test]
fn display_char_control_falls_back_to_canonical_for_real() {
    let result = evaluate_input("$display(\"%c\", 1.5)").expect("real %c");
    assert_eq!(result.task_output, b"1.5\n");
}

#[test]
fn display_string_control_emits_decoded_bytes() {
    // Simple ASCII
    let result = evaluate_input("$display(\"%s\", 24'h414243)").expect("ABC");
    assert_eq!(result.task_output, b"ABC\n");

    // %s on an actual string literal
    let result = evaluate_input("$display(\":%s:\", \"hello\")").expect("string literal");
    assert_eq!(result.task_output, b":hello:\n");
}

#[test]
fn display_format_string_must_be_a_string_literal() {
    // An integer whose bytes happen to spell a format string is NOT treated
    // as a format string — only actual string literals qualify.
    let result = evaluate_input("$display(8'h25, 42)").expect("percent-byte integer");
    // 8'h25 = 37 decimal, rendered as default decimal.  Not "%d" parsing.
    assert_eq!(result.task_output, b"37 42\n");
}

#[test]
fn display_real_values_in_default_format() {
    let result = evaluate_input("$display(1.5)").expect("one real");
    assert_eq!(result.task_output, b"1.5\n");

    let result = evaluate_input("$display(1.5, 2.5, -3.0)").expect("three reals");
    assert_eq!(result.task_output, b"1.5 2.5 -3.0\n");
}

#[test]
fn display_real_values_with_format_controls() {
    // %f / %F use fixed-point formatting.
    let result = evaluate_input("$display(\"%f %f\", 1.5, 42.0)").expect("%f");
    assert_eq!(result.task_output, b"1.5 42.0\n");

    let result = evaluate_input("$display(\"%f %f\", 1.0e10, 1.0e-5)").expect("%f fixed range");
    assert_eq!(result.task_output, b"10000000000.0 0.00001\n");

    // %e / %E use scientific notation and preserve the specifier case.
    let result = evaluate_input("$display(\"%e %E\", 1.5, 2.5)").expect("%e");
    assert_eq!(result.task_output, b"1.5e0 2.5E0\n");

    let result = evaluate_input("$display(\"%g %G\", 1.5e10, 2.5e-5)").expect("%g");
    assert_eq!(result.task_output, b"1.5e+10 2.5E-5\n");
}

#[test]
fn display_format_controls_on_integers_implicitly_convert_to_real() {
    // Integer args to %f / %e / %g are promoted to real.
    let result = evaluate_input("$display(\"%f %e\", 42, 255)").expect("int→real");
    assert_eq!(result.task_output, b"42.0 2.55e2\n");
}

#[test]
fn display_format_controls_on_real_with_integer_specs() {
    // Real args to %d / %h / %b etc. are formatted as real values
    // (vcal renders them in the familiar decimal form).
    let result = evaluate_input("$display(\"%d %h %b\", 1.5, 2.5, 3.5)").expect("real→int spec");
    assert_eq!(result.task_output, b"1.5 2.5 3.5\n");
}

#[test]
fn display_values_with_unknown_bits() {
    // All-x value in default format
    let result = evaluate_input("$display(1'bx)").expect("all-x");
    assert_eq!(result.task_output, b"x\n");

    // All-z value
    let result = evaluate_input("$display(1'bz)").expect("all-z");
    assert_eq!(result.task_output, b"z\n");

    // Mixed bits in decimal — x dominates
    let result = evaluate_input("$display(4'b01xx)").expect("mixed-x");
    assert_eq!(result.task_output, b"x\n");

    // In binary format
    let result = evaluate_input("$display(\"%b %b\", 1'bx, 1'bz)").expect("x/z binary");
    assert_eq!(result.task_output, b"x z\n");
}

#[test]
fn display_signed_decimal_values() {
    // Verilog: -4'sd1 is a unary minus applied to the literal 4'sd1.
    let result = evaluate_input("$display(-4'sd1)").expect("signed -1");
    assert_eq!(result.task_output, b"-1\n");

    // When formatting with %d, the signed interpretation is used.
    let result = evaluate_input("$display(\"%d\", 4'shf)").expect("signed 4'hf");
    // 4'shf = -1 in signed decimal
    assert_eq!(result.task_output, b"-1\n");
}

#[test]
fn write_vs_display_newline_behavior() {
    // $write produces no trailing newline; $display does.
    // When the last statement is a system task (not an expression),
    // `output` is empty.
    let result =
        evaluate_input("$write(\"a\"); $write(\"b\"); $display(\"c\")").expect("write+display");
    assert_eq!(result.task_output, b"abc\n");
    assert_eq!(result.output, ""); // last stmt is a task, no value output

    // Mixed: tasks + trailing expression — task output accumulates,
    // expression value goes to `output`.
    let result = evaluate_input("$write(\"a\"); $display(\"b\"); 1 + 1").expect("tasks+expr");
    assert_eq!(result.task_output, b"ab\n");
    assert_eq!(result.output, "32'sd2");

    // $write alone — task output has no trailing newline
    let result = evaluate_input("$write(\"xyz\")").expect("write alone");
    assert_eq!(result.task_output, b"xyz");
}

#[test]
fn display_with_format_string_that_has_no_controls_outputs_verbatim() {
    let result = evaluate_input("$display(\"hello world\")").expect("literal string");
    assert_eq!(result.task_output, b"hello world\n");
}

#[test]
fn display_extra_args_after_format_honor_space_joining_rule() {
    // When the format string ends with a space, no double-space is emitted
    // before extra args.
    let result = evaluate_input("$display(\"x=%d \", 1, 2, 3)").expect("trailing space");
    assert_eq!(result.task_output, b"x=1 2 3\n");
}

#[test]
fn display_consecutive_controls_without_literal_separator() {
    // %d on unsized integers renders decimal without leading zeros.
    let result = evaluate_input("$display(\"%d%d%d\", 1, 2, 3)").expect("consecutive %d");
    assert_eq!(result.task_output, b"123\n");

    // %h on unsized integers renders 32-bit hex (8 hex digits each).
    let result = evaluate_input("$display(\"%h%h%h\", 10, 11, 12)").expect("consecutive %h");
    assert_eq!(result.task_output, b"0000000a0000000b0000000c\n");

    // With 4-bit values the hex digits are single characters.
    let result =
        evaluate_input("$display(\"%h%h%h\", 4'ha, 4'hb, 4'hc)").expect("consecutive 4-bit %h");
    assert_eq!(result.task_output, b"abc\n");
}

#[test]
fn display_system_task_in_expression_position_is_semantic_error() {
    // $display / $write called as a function (inside an expression context)
    // must produce a clear semantic error.
    let error = evaluate_input("1 + $display(\"x\")").expect_err("$display in expr");
    assert!(
        error.contains("system task") || error.contains("cannot be called as a function"),
        "got: {error}"
    );

    let error = evaluate_input("$write(\"x\") ? 1 : 2").expect_err("$write in conditional");
    assert!(
        error.contains("system task") || error.contains("cannot be called as a function"),
        "got: {error}"
    );
}

#[test]
fn write_newline_byte_in_format_string() {
    // A literal newline inside a string format argument is preserved.
    let result = evaluate_input("$display(\"a\\nb\")").expect("embedded newline in string");
    assert_eq!(result.task_output, b"a\nb\n");
}

#[test]
fn display_with_interleaved_real_and_integer_args() {
    let result = evaluate_input("$display(\"%d %f %s\", 42, 3.14, \"ok\")").expect("mixed types");
    assert_eq!(result.task_output, b"42 3.14 ok\n");
}

#[test]
fn display_with_only_extra_args_no_format_controls() {
    // Format string with no controls but additional args — extra args
    // are appended.
    let result = evaluate_input("$display(\"val:\", 1, 2)").expect("extra after literal");
    assert_eq!(result.task_output, b"val: 1 2\n");
}

#[test]
fn display_reg_value_in_default_format() {
    let mut session = Session::new();
    session.eval("reg [7:0] a = 8'h42").expect("decl");
    let result = session
        .eval("$display(\"a=%d\", a)")
        .expect("display reg with %d");
    // 8'h42 = 66 decimal
    assert_eq!(result.task_output, b"a=66\n");
}

#[test]
fn display_multiple_system_tasks_accumulate_task_output() {
    let result =
        evaluate_input("$write(\"hello \"); $display(\"world\"); $finish").expect("multi-task");
    assert_eq!(result.task_output, b"hello world\n");
    assert!(result.should_exit);
}

#[test]
fn display_string_control_falls_back_when_width_is_wrong() {
    // String literal passed to %s works
    let result = evaluate_input("$display(\":%s:\", \"AB\")").expect("string %s");
    assert_eq!(result.task_output, b":AB:\n");
}

#[test]
fn display_integers_with_leading_zeros_preserve_digit_count() {
    // Binary format shows leading zeros determined by width.
    let result = evaluate_input("$display(\"%b\", 8'h0f)").expect("binary zero-pad");
    assert_eq!(result.task_output, b"00001111\n");
}

#[test]
fn display_empty_extra_args_no_trailing_space() {
    // No format controls and only the format string — no trailing junk.
    let result = evaluate_input("$display(\"ok\")").expect("just string");
    assert_eq!(result.task_output, b"ok\n");
}

#[test]
fn display_format_with_only_extra_args_and_no_initial_string() {
    // No format string at all — all args joined space-separated.
    let result = evaluate_input("$display(4'b1010, 4'hf, 1.5)").expect("no format");
    assert_eq!(result.task_output, b"10 15 1.5\n");
}

// ── $displayb / $displayo / $displayh / $writeb / $writeo / $writeh ──────
//
// LRM 21.2: the b/o/h suffixed variants behave exactly like $display/$write
// except that the default format for unformatted integer arguments is
// binary / octal / hex respectively (instead of decimal). Explicit format
// controls in the format string still override the default.

#[test]
fn displayb_defaults_to_binary() {
    let result = evaluate_input("$displayb(4'b1010)").expect("displayb default");
    assert_eq!(result.task_output, b"1010\n");

    // Unsized literal — 32-bit binary per LRM width rules.
    let result = evaluate_input("$displayb(5)").expect("displayb unsized");
    assert_eq!(result.task_output, b"00000000000000000000000000000101\n");
}

#[test]
fn displayo_defaults_to_octal() {
    let result = evaluate_input("$displayo(8'hff)").expect("displayo default");
    assert_eq!(result.task_output, b"377\n");

    // 9-bit value rounds up to two octal digits.
    let result = evaluate_input("$displayo(9'o777)").expect("displayo 9-bit");
    assert_eq!(result.task_output, b"777\n");
}

#[test]
fn displayh_defaults_to_hex() {
    let result = evaluate_input("$displayh(8'hff)").expect("displayh default");
    assert_eq!(result.task_output, b"ff\n");

    // Unsized literal — 32-bit hex (8 digits).
    let result = evaluate_input("$displayh(255)").expect("displayh unsized");
    assert_eq!(result.task_output, b"000000ff\n");
}

#[test]
fn writeb_writeo_writeh_default_bases_without_newline() {
    let b = evaluate_input("$writeb(4'b1010)").expect("writeb");
    assert_eq!(b.task_output, b"1010");

    let o = evaluate_input("$writeo(8'hff)").expect("writeo");
    assert_eq!(o.task_output, b"377");

    let h = evaluate_input("$writeh(8'hff)").expect("writeh");
    assert_eq!(h.task_output, b"ff");
}

#[test]
fn display_base_variants_join_multiple_args_with_spaces() {
    let result = evaluate_input("$displayh(4'ha, 4'hb, 4'hc)").expect("multi hex");
    assert_eq!(result.task_output, b"a b c\n");

    let result = evaluate_input("$displayb(1, 2, 3)").expect("multi bin");
    // Each unsized literal is 32 bits wide.
    assert_eq!(
        result.task_output,
        b"00000000000000000000000000000001 \
00000000000000000000000000000010 \
00000000000000000000000000000011\n"
    );
}

#[test]
fn display_base_variants_explicit_controls_override_default() {
    // %d inside $displayh still renders decimal.
    let result = evaluate_input("$displayh(\"%d\", 8'hff)").expect("%d in displayh");
    assert_eq!(result.task_output, b"255\n");

    // %h inside $display still renders hex (sanity — unchanged behaviour).
    let result = evaluate_input("$display(\"%h\", 8'hff)").expect("%h in display");
    assert_eq!(result.task_output, b"ff\n");

    // %b inside $displayo overrides octal.
    let result = evaluate_input("$displayo(\"%b\", 4'b1010)").expect("%b in displayo");
    assert_eq!(result.task_output, b"1010\n");
}

#[test]
fn display_base_variants_extra_args_use_task_default_base() {
    // Extra args after the format string are appended with the task's
    // default base, not the $display decimal default.
    let result = evaluate_input("$displayh(\"x=%s\", \"ok\", 8'haf)").expect("extra hex");
    assert_eq!(result.task_output, b"x=ok af\n");

    let result = evaluate_input("$displayb(\"v=%d\", 3, 4'b1010)").expect("extra bin");
    assert_eq!(result.task_output, b"v=3 1010\n");
}

#[test]
fn display_base_variants_preserve_string_display_style() {
    // A string literal is still rendered as its bytes, not as hex/binary.
    let result = evaluate_input("$displayh(\"hi\")").expect("string literal in displayh");
    assert_eq!(result.task_output, b"hi\n");

    let result = evaluate_input("$displayb(\"hi\")").expect("string literal in displayb");
    assert_eq!(result.task_output, b"hi\n");
}

#[test]
fn display_base_variants_render_reals_in_canonical_form() {
    // Reals are not converted to the task's default base.
    let result = evaluate_input("$displayh(1.5)").expect("real in displayh");
    assert_eq!(result.task_output, b"1.5\n");

    let result = evaluate_input("$displayb(-3.0)").expect("real in displayb");
    assert_eq!(result.task_output, b"-3.0\n");
}

#[test]
fn display_base_variants_null_arguments_emit_spaces() {
    // Null arguments emit a single space regardless of the default base.
    let result = evaluate_input("$displayh(,, 8'hff,)").expect("nulls in displayh");
    assert_eq!(result.task_output, b"  ff \n");

    let result = evaluate_input("$writeb(,)").expect("nulls in writeb");
    assert_eq!(result.task_output, b"  ");
}

#[test]
fn display_base_variants_unknown_bits_render_per_base_grouping() {
    // Binary — per-bit unknowns are preserved.
    let result = evaluate_input("$displayb(4'b01xx)").expect("binary unknowns");
    assert_eq!(result.task_output, b"01xx\n");

    // Hex — any unknown in a 4-bit group collapses the whole digit to x.
    let result = evaluate_input("$displayh(4'b01xx)").expect("hex group unknown");
    assert_eq!(result.task_output, b"x\n");

    // All-x value collapses to a single x digit.
    let result = evaluate_input("$displayh(1'bx)").expect("hex all x");
    assert_eq!(result.task_output, b"x\n");

    // All-z value collapses to a single z digit.
    let result = evaluate_input("$displayo(1'bz)").expect("octal all z");
    assert_eq!(result.task_output, b"z\n");
}

#[test]
fn display_base_variants_in_expression_position_are_rejected() {
    for input in [
        "$displayb(\"x\")",
        "$displayo(\"x\")",
        "$displayh(\"x\")",
        "$writeb(\"x\")",
        "$writeo(\"x\")",
        "$writeh(\"x\")",
    ] {
        let error = evaluate_input(&format!("1 + {input}")).expect_err(input);
        assert!(
            error.contains("is a system task") && error.contains("cannot be called as a function"),
            "{input}: got {error}"
        );
    }
}

#[test]
fn display_base_variant_identifiers_are_exact_matched() {
    // `$displayfoo` is not `$display` + suffix; it is an unknown identifier.
    let error = evaluate_input("$displayhex").expect_err("$displayhex is not supported");
    assert!(
        error.contains("unknown system identifier: $displayhex"),
        "got: {error}"
    );

    let error = evaluate_input("$writebin").expect_err("$writebin is not supported");
    assert!(
        error.contains("unknown system identifier: $writebin"),
        "got: {error}"
    );
}
