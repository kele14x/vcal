use crate::evaluate_input;

#[test]
fn accepts_finish_and_stop_with_optional_parens() {
    let finish = evaluate_input("$finish()").expect("$finish() should parse");
    let stop = evaluate_input("$stop();").expect("$stop() should parse");

    assert_eq!(finish.output, "");
    assert!(finish.should_exit);
    assert_eq!(stop.output, "");
    assert!(stop.should_exit);
}

// LRM 17.4 allows `$finish[(n)]` where n is a verbosity level. vcal prints
// no exit diagnostic so the argument is meaningless — accept and discard.
#[test]
fn accepts_finish_with_single_integer_argument() {
    for input in ["$finish(0)", "$finish(1)", "$finish(2)"] {
        let result = evaluate_input(input).unwrap_or_else(|err| panic!("{input}: {err}"));
        assert!(result.should_exit, "{input}");
    }
}

// vcal is intentionally lenient about the LRM 0-or-1 arity rule: enforcing
// it would teach users a restriction vcal itself does not act on, since the
// argument list is never inspected.
#[test]
fn accepts_finish_and_stop_with_extra_arguments() {
    let finish = evaluate_input("$finish(0, 1, 2)").expect("multi-arg finish");
    assert!(finish.should_exit);
    let stop = evaluate_input("$stop(1, 2, 3)").expect("multi-arg stop");
    assert!(stop.should_exit);
}

// Null arguments (empty comma slots) are accepted by all system tasks,
// not just `$display`/`$write`. For `$finish`/`$stop` the args are
// discarded anyway, so nulls are harmless — this test guards the
// task/function split in the parser's null-arg gate.
#[test]
fn finish_and_stop_accept_null_arguments() {
    let finish = evaluate_input("$finish(,)").expect("null args should parse");
    assert!(finish.should_exit);
    let stop = evaluate_input("$stop(,,)").expect("null args should parse");
    assert!(stop.should_exit);
    let mixed = evaluate_input("$finish(, 0, )").expect("mixed null args should parse");
    assert!(mixed.should_exit);
}

// Argument is parsed for syntactic validity but never evaluated, so a
// would-be runtime error inside the argument still exits cleanly.
#[test]
fn finish_argument_is_not_evaluated() {
    let result = evaluate_input("$finish(1/0)").expect("expression arg should parse and discard");
    assert!(result.should_exit);
}

// `($finish)` collapses to a bare SystemTask after the top-level Grouped
// unwrap, so parenthesisation does not change exit behavior.
#[test]
fn grouped_system_task_still_exits() {
    let result = evaluate_input("($finish)").expect("grouped task should exit");
    assert!(result.should_exit);
    let nested = evaluate_input("(($stop()))").expect("nested-grouped task should exit");
    assert!(nested.should_exit);
}

// Identifier match is exact: `$finisher` is a valid system_function_identifier
// per LRM A.9.3 but is not in vcal's supported set, so it surfaces the
// validator's "unknown system identifier" message rather than the
// task-in-expression one.
#[test]
fn task_like_identifier_with_trailing_chars_is_unknown_function() {
    let error = evaluate_input("$finisher").expect_err("$finisher is not supported");
    assert!(
        error.contains("unknown system identifier: $finisher"),
        "got: {error}"
    );
    let error = evaluate_input("$stop_clock").expect_err("$stop_clock is not supported");
    assert!(
        error.contains("unknown system identifier: $stop_clock"),
        "got: {error}"
    );
}

// When `$finish` / `$stop` appears inside an expression — at any position,
// with or without parens, with or without arguments — the evaluator
// surfaces the task-in-expression diagnostic. The message intentionally
// uses the `$name()` function-call form to convey what the user is doing
// wrong, regardless of how they wrote the task.
#[test]
fn system_task_in_expression_is_rejected() {
    for input in [
        "1 + $finish",
        "$finish + 1",
        "$finish() + 1",
        "1 + $finish(0)",
        "1 + $stop",
        "$stop() ? 1 : 2",
        "1 + $display(\"x\")",
        "$write(\"x\") ? 1 : 2",
        "-$finish",
        "{$finish, 4'b0}",
    ] {
        let error = evaluate_input(input).expect_err(input);
        assert!(
            error.contains("is a system task") && error.contains("cannot be called as a function"),
            "{input}: got {error}"
        );
    }
}

// Syntactic malformation inside the argument list still surfaces a parse
// error — leniency is about value/arity/null-args, not malformed syntax.
// A trailing comma is NOT malformed for tasks: it produces a null argument.
#[test]
fn system_task_with_malformed_argument_is_parse_error() {
    let error = evaluate_input("$finish(1 +)").expect_err("trailing + should be a parse error");
    assert!(!error.is_empty());
    let error = evaluate_input("$finish(").expect_err("unclosed paren should be a parse error");
    assert!(!error.is_empty());
}
