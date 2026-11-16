use crate::{Session, evaluate_input, run_repl};
use std::io::Cursor;

// Unified declaration hints: every successfully declared variable emits
// one comment-form hint line as task output — `// a: reg [7:0]` for
// explicit decls (redecls included), `// b: reg [7:0] (inferred)` for
// implicit declarations. Hints survive trailing `;`, travel ahead of
// later errors, render dim only in the color interactive frontend, and
// paste back as harmless comments. See doc/non-standard.md →
// Declaration hints.

const DIM: &[u8] = b"\x1b[2m";
const RESET: &[u8] = b"\x1b[0m";

#[test]
fn explicit_reg_decl_emits_hint() {
    let mut session = Session::new();
    let result = session.eval("reg [7:0] a").expect("decl");
    assert_eq!(result.task_output, b"// a: reg [7:0]\n");
    assert!(result.output.is_empty());
    assert!(result.value_output.is_empty());
}

#[test]
fn reg_shape_forms_render_resolved_ranges() {
    let mut session = Session::new();
    for (input, expected) in [
        ("reg signed [3:0] s", "// s: reg signed [3:0]\n"),
        ("reg a", "// a: reg\n"),
        ("reg signed b", "// b: reg signed\n"),
        ("reg [0:7] r", "// r: reg [0:7]\n"),
        ("reg [-1:2] n", "// n: reg [-1:2]\n"),
        ("reg [0:0] v", "// v: reg [0:0]\n"),
    ] {
        let result = session.eval(input).expect("decl");
        assert_eq!(result.task_output, expected.as_bytes(), "{input}");
    }
}

#[test]
fn integer_and_real_hints_use_their_keywords() {
    let mut session = Session::new();
    for (input, expected) in [
        ("integer i", "// i: integer\n"),
        ("integer j = 5", "// j: integer\n"),
        ("real r", "// r: real\n"),
        ("real s = 1.5", "// s: real\n"),
    ] {
        let result = session.eval(input).expect("decl");
        assert_eq!(result.task_output, expected.as_bytes(), "{input}");
    }
}

#[test]
fn multi_name_decl_emits_one_hint_per_name() {
    let mut session = Session::new();
    let result = session.eval("reg [3:0] a, b, c").expect("decl");
    assert_eq!(
        result.task_output,
        b"// a: reg [3:0]\n// b: reg [3:0]\n// c: reg [3:0]\n"
    );
    let result = session.eval("integer i = 1, j = 2").expect("decl");
    assert_eq!(result.task_output, b"// i: integer\n// j: integer\n");
}

#[test]
fn array_hints_show_the_unpacked_dimension() {
    let mut session = Session::new();
    for (input, expected) in [
        ("reg [3:0] a [0:15]", "// a: reg [3:0] [0:15]\n"),
        ("reg signed [3:0] s [1:2]", "// s: reg signed [3:0] [1:2]\n"),
        ("integer ia [0:3]", "// ia: integer [0:3]\n"),
        ("real ra [1:4]", "// ra: real [1:4]\n"),
        ("reg [3:0] b [15:0]", "// b: reg [3:0] [15:0]\n"),
    ] {
        let result = session.eval(input).expect("decl");
        assert_eq!(result.task_output, expected.as_bytes(), "{input}");
    }
}

#[test]
fn redeclaration_emits_a_hint_without_inferred_marker() {
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("decl");
    let result = session.eval("reg [3:0] a").expect("redecl");
    assert_eq!(result.task_output, b"// a: reg [3:0]\n");
    // An inferred binding redeclared explicitly loses the marker.
    session.eval("b = 8'h1").expect("inference");
    let result = session.eval("reg [3:0] b").expect("redecl");
    assert_eq!(result.task_output, b"// b: reg [3:0]\n");
}

#[test]
fn failed_decls_emit_no_hint_and_commit_nothing() {
    let mut session = Session::new();
    let err = session.eval("reg [3:0] a = nope").expect_err("bad init");
    assert!(err.task_output.is_empty());
    let err = session.eval("a").expect_err("no binding");
    assert_eq!(err, "Semantic error: undeclared identifier: a");
    // All-or-nothing: the earlier name in the list gets no hint either.
    let err = session
        .eval("integer i = 1, j = nope")
        .expect_err("second init fails");
    assert!(err.task_output.is_empty());
    let err = session.eval("i").expect_err("no binding");
    assert_eq!(err, "Semantic error: undeclared identifier: i");
    // A malformed range aborts before any name is staged.
    let err = session.eval("reg [1'bx:0] r").expect_err("x endpoint");
    assert!(err.task_output.is_empty());
}

#[test]
fn hint_survives_trailing_semicolon() {
    let mut session = Session::new();
    let result = session.eval("reg [7:0] a;").expect("decl");
    assert_eq!(result.task_output, b"// a: reg [7:0]\n");
    assert!(result.output.is_empty());
}

#[test]
fn hint_and_spans_preserved_before_later_error() {
    // The spans ride in the crate-private wrapper; the public EvalError
    // keeps only message + task_output.
    let mut session = Session::new();
    let failure = crate::evaluate_input_with_session(&mut session, "reg [7:0] a; missing")
        .expect_err("second statement fails");
    let err = &failure.error;
    assert_eq!(err.task_output, b"// a: reg [7:0]\n");
    assert_eq!(failure.task_spans.len(), 1);
    assert_eq!(failure.task_spans[0].start, 0);
    assert_eq!(failure.task_spans[0].end, b"// a: reg [7:0]".len());
    assert_eq!(
        failure.task_spans[0].class,
        crate::highlight::TokenClass::Comment
    );
    // The error path renders the preserved hint dim as well.
    let styled = crate::color::highlight_bytes(&err.task_output, &failure.task_spans, true);
    assert_eq!(
        styled.as_ref(),
        [DIM, b"// a: reg [7:0]", RESET, b"\n"].concat()
    );
    // The completed decl stands even though the line failed.
    assert_eq!(session.eval("a").expect("read").output, "8'bxxxxxxxx");
}

#[test]
fn public_eval_error_keeps_its_two_field_shape() {
    // External callers must be able to construct EvalError and destructure
    // it exhaustively; styling metadata travels in a crate-private wrapper
    // instead of a private field. Both operations stop compiling if a
    // field is ever added to the public struct.
    let err = crate::EvalError {
        message: String::from("Semantic error: probe"),
        task_output: b"// a: reg\n".to_vec(),
    };
    let crate::EvalError {
        message,
        task_output,
    } = err;
    assert_eq!(message, "Semantic error: probe");
    assert_eq!(task_output, b"// a: reg\n");
}

#[test]
fn ordinary_assignment_to_existing_variable_stays_silent() {
    let mut session = Session::new();
    session.eval("reg [7:0] a").expect("decl");
    let result = session.eval("a = 8'h1").expect("assignment");
    assert!(result.task_output.is_empty());
    session.eval("b = 8'h1").expect("inference");
    let result = session.eval("b = 8'h2").expect("assignment");
    assert!(result.task_output.is_empty());
}

#[test]
fn pasted_hints_are_harmless_comments() {
    let result = evaluate_input("// a: reg [7:0]").expect("comment-only input");
    assert!(result.output.is_empty());
    assert!(result.task_output.is_empty());
    let mut session = Session::new();
    session
        .eval("// a: reg [7:0]\n// b: real (inferred)")
        .expect("pasted hints");
    let err = session.eval("a").expect_err("no binding created");
    assert_eq!(err, "Semantic error: undeclared identifier: a");
    let err = session.eval("b").expect_err("no binding created");
    assert_eq!(err, "Semantic error: undeclared identifier: b");
}

#[test]
fn piped_repl_hint_bytes() {
    let mut input = Cursor::new("reg [7:0] a\nb = 8'h1\na = 8'h2\na\n$finish\n");
    let mut output = Vec::new();
    run_repl(&mut input, &mut output).expect("REPL should run");
    assert_eq!(
        output,
        b"In [0]: // a: reg [7:0]\n\
          \n\
          In [1]: // b: reg [7:0] (inferred)\n\
          \n\
          In [2]: \n\
          In [3]: Out[3]: 8'h02\n\
          \n\
          In [4]: \n"
    );
    assert!(!output.contains(&0x1b), "piped output must stay ANSI-free");
}

#[test]
fn hint_spans_render_dim_only_in_the_styled_path() {
    let (result, spans) =
        crate::evaluate_input_with_session(&mut Session::new(), "reg [7:0] a; b = 8'h1")
            .expect("decl + inference");
    assert_eq!(
        result.task_output,
        b"// a: reg [7:0]\n// b: reg [7:0] (inferred)\n"
    );
    assert_eq!(spans.task.len(), 2);
    assert!(spans.value.is_empty());
    assert!(!result.task_output.contains(&0x1b));
    let styled = crate::color::highlight_bytes(&result.task_output, &spans.task, true);
    assert_eq!(
        styled.as_ref(),
        [
            DIM,
            b"// a: reg [7:0]",
            RESET,
            b"\n",
            DIM,
            b"// b: reg [7:0] (inferred)",
            RESET,
            b"\n"
        ]
        .concat()
    );
    let plain = crate::color::highlight_bytes(&result.task_output, &spans.task, false);
    assert_eq!(plain.as_ref(), result.task_output);
}

#[test]
fn hint_spans_offset_correctly_past_task_output() {
    let (result, spans) = crate::evaluate_input_with_session(
        &mut Session::new(),
        "$display(\"x\"); reg a; $display(\"y\")",
    )
    .expect("tasks around a decl");
    assert_eq!(result.task_output, b"x\n// a: reg\ny\n");
    assert_eq!(spans.task.len(), 1);
    assert_eq!(spans.task[0].start, 2);
    assert_eq!(spans.task[0].end, 2 + b"// a: reg".len());
    let styled = crate::color::highlight_bytes(&result.task_output, &spans.task, true);
    assert_eq!(
        styled.as_ref(),
        [b"x\n", DIM, b"// a: reg", RESET, b"\ny\n"].concat()
    );
}

#[test]
fn exit_path_keeps_hint_spans() {
    let (result, spans) = crate::evaluate_input_with_session(&mut Session::new(), "reg a; $finish")
        .expect("decl then exit");
    assert!(result.should_exit);
    assert_eq!(result.task_output, b"// a: reg\n");
    assert_eq!(spans.task.len(), 1);
}
