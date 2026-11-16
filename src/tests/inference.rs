use crate::{Session, evaluate_input, run_repl};
use num_bigint::BigInt;
use std::io::Cursor;

// Implicit declaration on assignment (non-standard): a blocking
// assignment to a bare undeclared name creates the binding from the
// self-determined RHS — integer result → vector reg `[width-1:0]`,
// real result → `real`. The shape is fixed afterwards; selects, concat
// leaves, and reads of undeclared names stay errors. See
// doc/non-standard.md → Implicit declaration on assignment.

#[test]
fn bare_assignment_infers_unsigned_vector() {
    let mut session = Session::new();
    let result = session.eval("a = 8'h1 + 8'h0").expect("inference");
    assert_eq!(result.task_output, b"// a: reg [7:0] (inferred)\n");
    assert!(result.output.is_empty());
    assert_eq!(session.eval("a").expect("read").output, "8'h01");
    let (msb, lsb) = session.lookup_reg_range("a").expect("vector range");
    assert_eq!(
        (msb.clone(), lsb.clone()),
        (BigInt::from(7), BigInt::from(0))
    );
}

#[test]
fn inferred_shape_is_fixed_after_creation() {
    let mut session = Session::new();
    session.eval("a = 8'h1").expect("inference");
    // A wider RHS truncates like any declared reg, and an ordinary
    // assignment emits no hint.
    let result = session.eval("a = 16'hffff").expect("assignment");
    assert!(result.task_output.is_empty());
    assert_eq!(session.eval("a").expect("read").output, "8'hff");
}

#[test]
fn explicit_redecl_resets_inferred_binding() {
    let mut session = Session::new();
    session.eval("a = 8'h2a").expect("inference");
    session.eval("reg [3:0] a").expect("redecl");
    assert_eq!(session.eval("a").expect("read").output, "4'bxxxx");
}

#[test]
fn plain_decimal_infers_signed_32_bit() {
    let mut session = Session::new();
    let result = session.eval("a = 1").expect("inference");
    assert_eq!(result.task_output, b"// a: reg signed [31:0] (inferred)\n");
    assert_eq!(session.eval("a").expect("read").output, "32'sd1");
}

#[test]
fn sized_signed_rhs_infers_signed_shape() {
    let mut session = Session::new();
    let result = session.eval("a = 4'shf").expect("inference");
    assert_eq!(result.task_output, b"// a: reg signed [3:0] (inferred)\n");
    assert_eq!(session.eval("a").expect("read").output, "4'shf");
}

#[test]
fn one_bit_rhs_infers_selectable_vector() {
    let mut session = Session::new();
    let result = session.eval("a = 1'b1").expect("inference");
    assert_eq!(result.task_output, b"// a: reg [0:0] (inferred)\n");
    assert_eq!(session.eval("a[0]").expect("bit-select").output, "1'b1");
    session.eval("a[0] = 1'b0").expect("bit write");
    assert_eq!(session.eval("a").expect("read").output, "1'b0");
}

#[test]
fn real_rhs_infers_real_binding() {
    let mut session = Session::new();
    let result = session.eval("r = 1.5").expect("inference");
    assert_eq!(result.task_output, b"// r: real (inferred)\n");
    assert_eq!(session.eval("r").expect("read").output, "1.5");
    // Integer RHS promotes into the inferred real like any declared one.
    session.eval("r = 2").expect("assignment");
    assert_eq!(session.eval("r").expect("read").output, "2.0");
}

#[test]
fn mixed_real_expression_infers_real() {
    let mut session = Session::new();
    let result = session.eval("r = 1 + 0.5").expect("inference");
    assert_eq!(result.task_output, b"// r: real (inferred)\n");
    assert_eq!(session.eval("r").expect("read").output, "1.5");
}

#[test]
fn integer_rhs_infers_reg_not_real() {
    let mut session = Session::new();
    let result = session.eval("r = 1").expect("inference");
    assert_eq!(result.task_output, b"// r: reg signed [31:0] (inferred)\n");
    assert_eq!(session.eval("r").expect("read").output, "32'sd1");
    // A later real RHS converts per §3.5.3 (ties away from zero) into
    // the vector reg instead of reshaping it.
    session.eval("r = 1.5").expect("assignment");
    assert_eq!(session.eval("r").expect("read").output, "32'sd2");
}

#[test]
fn inferred_real_redecl_resets_to_zero() {
    let mut session = Session::new();
    session.eval("r = 1.5").expect("inference");
    session.eval("real r").expect("redecl");
    assert_eq!(session.eval("r").expect("read").output, "0.0");
}

#[test]
fn inference_chains_through_existing_bindings() {
    let mut session = Session::new();
    session.eval("a = 8'h1").expect("inference");
    let result = session.eval("b = a + 8'h1").expect("chained inference");
    assert_eq!(result.task_output, b"// b: reg [7:0] (inferred)\n");
    assert_eq!(session.eval("b").expect("read").output, "8'h02");
}

#[test]
fn inferred_reg_inherits_weak_base_from_fresh_reg_rhs() {
    let mut session = Session::new();
    session.eval("reg r").expect("decl");
    let result = session.eval("a = r").expect("inference");
    assert_eq!(result.task_output, b"// a: reg [0:0] (inferred)\n");
    assert_eq!(session.eval("a").expect("read").output, "1'bx");
    // The weak base resolves on the first whole-reg assignment, exactly
    // like a declared fresh reg.
    session.eval("a = 1'h1").expect("assignment");
    assert_eq!(session.eval("a").expect("read").output, "1'h1");
}

#[test]
fn string_rhs_infers_numeric_vector() {
    let mut session = Session::new();
    let result = session.eval("s = \"AB\"").expect("inference");
    assert_eq!(result.task_output, b"// s: reg [15:0] (inferred)\n");
    // Stored regs display numerically, matching a normal string assignment.
    assert_eq!(session.eval("s").expect("read").output, "16'h4142");
}

#[test]
fn concat_select_shift_and_conditional_rhs_forms() {
    let mut session = Session::new();
    let result = session.eval("a = {4'h1, 4'h2}").expect("concat RHS");
    assert_eq!(result.task_output, b"// a: reg [7:0] (inferred)\n");
    assert_eq!(session.eval("a").expect("read").output, "8'h12");

    session.eval("reg [7:0] x = 8'hab").expect("decl");
    let result = session.eval("b = x[3:0]").expect("select RHS");
    assert_eq!(result.task_output, b"// b: reg [3:0] (inferred)\n");
    assert_eq!(session.eval("b").expect("read").output, "4'hb");

    let result = session.eval("c = 8'h1 << 2").expect("shift RHS");
    assert_eq!(result.task_output, b"// c: reg [7:0] (inferred)\n");
    assert_eq!(session.eval("c").expect("read").output, "8'h04");

    let result = session
        .eval("d = 1'b1 ? 8'h10 : 8'h20")
        .expect("conditional RHS");
    assert_eq!(result.task_output, b"// d: reg [7:0] (inferred)\n");
    assert_eq!(session.eval("d").expect("read").output, "8'h10");

    let result = session.eval("e = 8'd5").expect("decimal-base RHS");
    assert_eq!(result.task_output, b"// e: reg [7:0] (inferred)\n");
    assert_eq!(session.eval("e").expect("read").output, "8'd5");
}

#[test]
fn cast_rhs_infers_signed_shape() {
    let mut session = Session::new();
    let result = session.eval("a = $signed(8'hff)").expect("inference");
    assert_eq!(result.task_output, b"// a: reg signed [7:0] (inferred)\n");
    assert_eq!(session.eval("a").expect("read").output, "8'shff");
}

#[test]
fn x_rhs_infers_x_filled_binding() {
    let mut session = Session::new();
    session.eval("a = 1'bx").expect("inference");
    assert_eq!(session.eval("a").expect("read").output, "1'bx");
    session.eval("b = 8'hxx").expect("inference");
    assert_eq!(session.eval("b").expect("read").output, "8'hxx");
}

#[test]
fn select_lvalue_on_undeclared_name_creates_nothing() {
    let mut session = Session::new();
    let err = session
        .eval("a[0] = 1'b1")
        .expect_err("select lvalue cannot infer");
    assert_eq!(err, "Semantic error: undeclared identifier: a");
    let err = session.eval("a").expect_err("still undeclared");
    assert_eq!(err, "Semantic error: undeclared identifier: a");
}

#[test]
fn failed_rhs_leaves_no_binding() {
    let mut session = Session::new();
    let err = session.eval("a = nope").expect_err("undeclared RHS");
    assert_eq!(err, "Semantic error: undeclared identifier: nope");
    assert!(err.task_output.is_empty());
    let err = session.eval("a").expect_err("still undeclared");
    assert_eq!(err, "Semantic error: undeclared identifier: a");

    // Self-reference without a prior binding is the same undeclared-RHS
    // case, not a self-init.
    let err = session.eval("b = b").expect_err("self-reference");
    assert_eq!(err, "Semantic error: undeclared identifier: b");

    // A semantic error in the RHS also aborts before any binding exists.
    session.eval("c = 1.0 % 2").expect_err("real modulus");
    let err = session.eval("c").expect_err("still undeclared");
    assert_eq!(err, "Semantic error: undeclared identifier: c");
}

#[test]
fn reserved_names_cannot_be_inferred() {
    // The decl grammar reserves these names, so inference must reject
    // them too — otherwise the binding could never be redeclared.
    // `signed` is the one
    // reserved name that parses as an assignment lvalue; the decl-start
    // keywords never reach the assignment path at all.
    let mut session = Session::new();
    let err = session
        .eval("signed = 1")
        .expect_err("reserved name rejected");
    assert_eq!(
        err,
        "Semantic error: `signed` cannot be used as a variable name"
    );
    assert!(err.task_output.is_empty());
    // A second attempt hits the same rejection, proving no binding was
    // created (an existing binding would assign normally).
    let err = session.eval("signed = 2").expect_err("still rejected");
    assert_eq!(
        err,
        "Semantic error: `signed` cannot be used as a variable name"
    );
    // The name check outranks RHS errors, matching the LHS-first
    // ordering of the declared-lvalue path, and covers the real branch.
    let err = session
        .eval("signed = nope")
        .expect_err("name check before RHS");
    assert_eq!(
        err,
        "Semantic error: `signed` cannot be used as a variable name"
    );
    let err = session
        .eval("signed = 1.5")
        .expect_err("real RHS still rejected");
    assert_eq!(
        err,
        "Semantic error: `signed` cannot be used as a variable name"
    );
    for name in ["reg", "integer", "real"] {
        let err = session
            .eval(&format!("{name} = 1"))
            .expect_err("decl dispatch");
        assert_eq!(
            err.message,
            format!("Syntax error: expected identifier in {name} declaration")
        );
    }
}

#[test]
fn decl_parser_still_rejects_reserved_names() {
    // The other end of the shared rule: declaring the shape a hint
    // describes with a reserved name stays a syntax error.
    let err = crate::evaluate_input("reg signed [31:0] signed").expect_err("reserved");
    assert_eq!(err, "Syntax error: `signed` cannot be used as a reg name");
    let err = crate::evaluate_input("real real").expect_err("reserved");
    assert_eq!(err, "Syntax error: `real` cannot be used as a real name");
}

#[test]
fn hint_survives_trailing_semicolon() {
    let mut session = Session::new();
    let result = session.eval("a = 8'h1;").expect("inference");
    assert_eq!(result.task_output, b"// a: reg [7:0] (inferred)\n");
    assert!(result.output.is_empty());
}

#[test]
fn hint_and_echo_on_same_line() {
    let mut session = Session::new();
    let result = session.eval("a = 8'h1; a").expect("inference + echo");
    assert_eq!(result.task_output, b"// a: reg [7:0] (inferred)\n");
    assert_eq!(result.output, "8'h01");
}

#[test]
fn error_carries_earlier_inference_hint() {
    let mut session = Session::new();
    let err = session
        .eval("a = 8'h1; b = nope")
        .expect_err("second statement fails");
    assert_eq!(err.task_output, b"// a: reg [7:0] (inferred)\n");
    assert!(err.contains("undeclared identifier: nope"));
    // The completed inference stands even though the line failed.
    assert_eq!(session.eval("a").expect("read").output, "8'h01");
}

#[test]
fn stateless_evaluate_input_supports_inference() {
    let result = evaluate_input("a = 8'h1; a").expect("inference + echo");
    assert_eq!(result.task_output, b"// a: reg [7:0] (inferred)\n");
    assert_eq!(result.output, "8'h01");
}

#[test]
fn repl_prints_inferred_hint() {
    let mut input = Cursor::new("a = 8'h1\na\n$finish\n");
    let mut output = Vec::new();
    run_repl(&mut input, &mut output).expect("REPL should run");
    assert_eq!(
        output,
        b"In [0]: // a: reg [7:0] (inferred)\n\
          \n\
          In [1]: Out[1]: 8'h01\n\
          \n\
          In [2]: \n"
    );
}
