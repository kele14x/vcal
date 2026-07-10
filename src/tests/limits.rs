use crate::{Session, evaluate_input, parse_input, parse_input_with_depth};

// Bug #2 regression suite — bit-vector size cap rejects huge widths
// before any `Vec<LogicBit>` allocation. Cap is MAX_BIT_WIDTH = 2**24
// (16,777,216 bits); the boundary is inclusive — exactly cap accepted,
// cap+1 rejected. All diagnostics use the `Semantic error:` prefix —
// the literal cap moved to the validator after the lazy `Expr::Literal`
// refactor, so all paths surface the cap as a semantic check rather than
// a parse-time check.

#[test]
fn huge_sized_literal_width_rejected() {
    // FINDINGS.md #2 repro 1: 10 trillion-bit literal. The parser builds a
    // LiteralSpec carrying width=9999999999999 without allocating it;
    // validate_expr_structure runs ensure_bit_width before the evaluator
    // would try to materialize.
    let err = evaluate_input("9999999999999'd1").unwrap_err();
    assert_eq!(
        err,
        "Semantic error: literal width 9999999999999 exceeds limit 16777216"
    );
}

#[test]
fn huge_unsized_literal_magnitude_rejected() {
    // Unsized literal whose magnitude needs > MAX_BIT_WIDTH bits: a
    // hex value with > 4_194_304 nibbles. The parser allocates a
    // text-bounded low_bits vec (~16 MB for the digits themselves) but
    // never the full `width` vec; validator rejects on the derived width.
    let mut input = String::from("'h");
    // 2**24 / 4 = 4_194_304 hex digits hit cap; one more digit pushes over.
    for _ in 0..4_194_305 {
        input.push('f');
    }
    let err = evaluate_input(&input).unwrap_err();
    assert!(
        err.starts_with("Semantic error: literal width "),
        "expected literal-width error, got: {err}"
    );
    assert!(
        err.ends_with(" exceeds limit 16777216"),
        "expected exceeds-limit suffix, got: {err}"
    );
}

#[test]
fn parser_accepts_huge_literal_width_without_allocating() {
    // Sanity check that the lazy LiteralSpec is doing its job: parsing
    // `9999999999999'd1` produces a well-formed AST in O(text) — no
    // 10 TB `Vec<LogicBit>`. Under the old eager AST this call would
    // hang the kernel committing pages; under the lazy form it returns
    // Ok within microseconds and the rendered AST contains the literal
    // width as a number, not bits.
    //
    // parse_input is the right oracle here: it goes through the full
    // parser + AST renderer but skips eval. If anything tried to expand
    // the bit vector this test would never return.
    let rendered = parse_input("9999999999999'd1").expect("parser must accept");
    assert!(
        rendered.contains("9999999999999"),
        "expected the literal width to appear in the AST, got: {rendered}"
    );
}

#[test]
fn parse_input_with_depth_accepts_huge_literal_width() {
    // The parse-only entry used by `vcal --parse-only` is purely
    // syntactic — it accepts inputs the validator would reject
    // (undeclared identifiers, out-of-range literal widths, etc.). The
    // contract is "did this parse?" not "is this a valid program?".
    let rendered = parse_input_with_depth("9999999999999'd1", 64)
        .expect("parse-only must accept syntactically valid input");
    assert!(rendered.contains("9999999999999"));
}

#[test]
fn huge_reg_decl_width_rejected() {
    // `reg [16777216:0]` = 16,777,217 bits, one over the cap.
    let err = evaluate_input("reg [16777216:0] r;").unwrap_err();
    assert_eq!(
        err,
        "Semantic error: reg width 16777217 exceeds limit 16777216"
    );
}

#[test]
fn huge_array_dim_rejected() {
    // Array dimension width also flows through RegRange::width.
    // 16,777,217-element array of 4-bit regs.
    let err = evaluate_input("reg [3:0] a [0:16777216];").unwrap_err();
    assert_eq!(
        err,
        "Semantic error: reg width 16777217 exceeds limit 16777216"
    );
}

#[test]
fn array_element_count_limit_applies_to_vector_and_real_arrays() {
    for declaration in ["reg a [0:65536];", "real r [0:65536];"] {
        let err = evaluate_input(declaration).unwrap_err();
        assert_eq!(
            err,
            "Semantic error: array element count 65537 exceeds limit 65536"
        );
    }
}

#[test]
fn vector_array_total_width_at_cap_accepted() {
    // 4096-bit elements * 4096 elements = 16,777,216 total bits.
    let mut session = Session::new();
    session
        .eval("reg [4095:0] a [0:4095];")
        .expect("array total exactly at cap is accepted");
}

#[test]
fn vector_array_total_width_over_cap_rejected() {
    // 4097-bit elements * 4096 elements = 16,781,312 total bits.
    let err = evaluate_input("reg [4096:0] a [0:4095];").unwrap_err();
    assert_eq!(
        err,
        "Semantic error: array total width 16781312 exceeds limit 16777216"
    );
}

#[test]
fn lvalue_concatenation_over_width_cap_is_rejected() {
    // Keep the stored reg small while repeating it enough times for the LHS
    // context to cross MAX_BIT_WIDTH: 65,536 * 257 = 16,842,752 bits.
    let mut session = Session::new();
    session
        .eval("reg [65535:0] a;")
        .expect("source reg declaration");
    let leaves = std::iter::repeat_n("a", 257).collect::<Vec<_>>().join(",");
    let err = session.eval(&format!("{{{leaves}}}=1")).unwrap_err();
    assert_eq!(
        err,
        "Semantic error: lvalue width 16842752 exceeds limit 16777216"
    );
}

#[test]
fn huge_replication_count_rejected() {
    // `{count{1'b0}}` with count one over the cap. Cap is on the
    // *result* width (inner_bits.len() * count), so a 1-bit element
    // hits the cap at exactly count = MAX_BIT_WIDTH and trips at +1.
    let err = evaluate_input("{16777217{1'b0}}").unwrap_err();
    assert_eq!(
        err,
        "Semantic error: replication width 16777217 exceeds limit 16777216"
    );
}

#[test]
fn concatenation_of_wide_operands_rejected() {
    // `{a, a}` where each `a` is half the cap or more sums past the cap.
    // Without a gate here, the concat would silently produce a vector
    // larger than MAX_BIT_WIDTH (32 Mbit garbage in this case). Each
    // operand individually fits — only the sum exceeds.
    let mut session = Session::new();
    session
        .eval("reg [16777215:0] a;")
        .expect("16M-bit reg decl at cap is accepted");
    let err = session.eval("{a, a};").unwrap_err();
    assert_eq!(
        err,
        "Semantic error: concatenation width 33554432 exceeds limit 16777216"
    );
}

#[test]
fn concatenation_of_wide_operands_at_cap_accepted() {
    // Concatenation total exactly at MAX_BIT_WIDTH must still pass —
    // the inclusive-accept boundary mirrors the replication / part-
    // select gates.
    let mut session = Session::new();
    session
        .eval("reg [8388607:0] a;")
        .expect("8M-bit reg decl is accepted");
    session
        .eval("{a, a};")
        .expect("concatenation result exactly at cap is accepted");
}

#[test]
fn nested_concat_inside_replication_rejected_at_inner_concat() {
    // `{N{...wide concat...}}` would balloon the replication-side
    // multiplier, but the inner concat itself trips the gate first —
    // confirming collect_concatenation_bits enforces the running total
    // before it ever returns to the replication multiplier.
    let mut session = Session::new();
    session
        .eval("reg [9000000:0] a;")
        .expect("9M-bit reg decl is accepted");
    let err = session.eval("{4{a, a}};").unwrap_err();
    assert!(
        err.starts_with("Semantic error: ")
            && (err.contains("concatenation width ") || err.contains("replication width ")),
        "expected concatenation- or replication-width error, got: {err}"
    );
}

#[test]
fn huge_indexed_part_select_width_rejected() {
    // FINDINGS.md #2 repro 3: `r[0 +: huge]`. Width flows through
    // evaluate_indexed_select_width.
    let mut session = Session::new();
    session.eval("reg [3:0] r;").expect("reg decl evaluates");
    let err = session.eval("r[0 +: 16777217];").unwrap_err();
    assert_eq!(
        err,
        "Semantic error: part-select width 16777217 exceeds limit 16777216"
    );
}

#[test]
fn at_cap_literal_width_accepted() {
    // Boundary: exactly 2**24 bits is accepted. Sanity-check the cap
    // is inclusive on the accept side — otherwise the off-by-one would
    // surface as a silent regression on the largest legal value.
    let evaluation = evaluate_input("16777216'd1").expect("at-cap literal width is accepted");
    assert!(
        evaluation.output.starts_with("16777216'd"),
        "expected 16777216-wide output, got: {}",
        evaluation.output
    );
}

#[test]
fn at_cap_reg_decl_width_accepted() {
    // `reg [16777215:0]` = exactly 16,777,216 bits = MAX_BIT_WIDTH.
    // Confirms the reg path's boundary mirrors the literal path's.
    let mut session = Session::new();
    session
        .eval("reg [16777215:0] r;")
        .expect("at-cap reg decl is accepted");
}

#[test]
fn parse_input_returns_ast_without_evaluating() {
    // The --parse-only debug entry point: parser runs, AST renders via
    // {:#?}, validation/evaluation are skipped. We just check the render
    // mentions the expected operator — exact format isn't part of the
    // contract.
    let rendered = parse_input("1 + 2").expect("should parse");
    assert!(rendered.contains("Add"), "expected Add op in: {rendered}");
    assert!(
        rendered.contains("Binary"),
        "expected Binary node in: {rendered}"
    );

    // Empty input is the same no-op contract as evaluate_input.
    assert_eq!(parse_input("").unwrap(), "");
    assert_eq!(parse_input("   \n").unwrap(), "");

    // Syntax errors surface with the same `Syntax error:` prefix as the
    // eval path so callers get a uniform diagnostic shape.
    let err = parse_input("1 +").expect_err("trailing op should fail");
    assert!(err.starts_with("Syntax error:"), "got: {err}");
}

#[test]
fn parse_input_with_depth_respects_caller_specified_cap() {
    // A shallow cap truncates aggressively: every Grouped at depth >
    // cap collapses to the `…` placeholder. Verify the cap is honored
    // by checking that a small depth produces fewer Grouped layers in
    // the rendered output than a larger depth.
    let input: String = "(".repeat(20) + "1" + &")".repeat(20);

    let shallow = parse_input_with_depth(&input, 5).expect("parse at depth 5");
    let deep = parse_input_with_depth(&input, 50).expect("parse at depth 50");

    let count_grouped = |s: &str| s.matches("Grouped").count();
    assert!(
        count_grouped(&shallow) < count_grouped(&deep),
        "shallow cap should produce fewer Grouped nodes; \
         shallow={}, deep={}",
        count_grouped(&shallow),
        count_grouped(&deep)
    );
    // Shallow render must show the truncation marker; deep render
    // shouldn't (input only goes 20 deep).
    assert!(
        shallow.contains("Truncated"),
        "expected `Truncated` marker at depth 5"
    );
    assert!(
        !deep.contains("Truncated"),
        "should not truncate at depth 50"
    );
}

#[test]
fn parse_input_renders_deep_input_without_overflow() {
    // The parser is iterative (Phase 3), but the auto-derived `Debug`
    // impl on `Expr` would still recurse on each `Grouped` layer when
    // formatting via `{:#?}`. parse_input applies a depth cap before
    // formatting so the bounded-recursion render is safe.
    //
    // 10^4 parens is well past the recursive-Debug overflow threshold.
    let input: String = "(".repeat(10_000) + "1" + &")".repeat(10_000);
    let rendered = parse_input(&input).expect("deep parens should parse and render");
    // The truncation marker should appear; the AST was deeper than 64.
    assert!(
        rendered.contains("Truncated"),
        "expected `Truncated` marker for input deeper than the display cap"
    );
}

#[test]
fn parse_input_truncates_deep_concat_lvalue() {
    // Regression for the LValue-side display cap: a deeply nested concat
    // lvalue like `{{{{...a}}}} = 1` has its own recursive shape on the
    // LHS, and the `{:#?}` formatter recurses one frame per `Concat`
    // layer. Without `truncate_lvalue_for_display` the rendered output
    // would overflow the formatter stack — separate from the Expr-side
    // truncation, which only protects the RHS.
    //
    // We need a reg declared first so the LHS parses; the parser doesn't
    // need it (parse_input skips eval), but expression_to_lvalue runs
    // during parse to convert the LHS Expr into an LValue.
    let n = 200;
    let lhs = "{".repeat(n) + "a" + &"}".repeat(n);
    let input = format!("{lhs} = 1");
    let rendered =
        parse_input_with_depth(&input, 10).expect("deep concat lvalue should parse and render");
    assert!(
        rendered.contains("Truncated"),
        "expected LValue::Truncated marker when concat lvalue exceeds depth cap"
    );
}

#[test]
fn parse_input_skips_semantic_errors() {
    // Inputs that parse cleanly but would fail at validate/eval time
    // must succeed under parse_input — the whole point is to isolate
    // the parser stage. `undefined_var` is a perfectly valid Identifier
    // expression as far as the parser is concerned; semantic_check is
    // what would reject it.
    let rendered = parse_input("undefined_var + 1").expect("parser accepts identifier");
    assert!(rendered.contains("Identifier"));
    assert!(rendered.contains("Add"));
}
