use crate::parser::{BinaryOp, Expr, SystemArg, UnaryOp, parse_expression, parse_integer};
use crate::{Session, evaluate_input};

#[test]
fn long_addition_chain_evaluates_without_quadratic_blowup() {
    // Regression for the O(N²) helper-walk pattern: validation and
    // evaluation used to re-derive real-typedness and result meta by
    // walking lhs/rhs at every Binary node, each one a fresh recursive
    // subtree walk. The annotated-AST refactor caches both up front so
    // each chain level is O(1).
    //
    // Now that `evaluate_annotated` is iterative (no Rust stack frame
    // per chain level), this runs on the default 2 MB test thread.
    let chain: String = std::iter::once("1".to_string())
        .chain(std::iter::repeat_n("+1".to_string(), 2000))
        .collect();
    let evaluation = evaluate_input(&chain).expect("2001-term chain should evaluate");
    assert_eq!(evaluation.output, "32'sd2001");
}

#[test]
fn parses_deeply_nested_parens_without_overflow() {
    // Pre-Phase-3, every `(` cost ~14 stack frames in the recursive Pratt
    // ladder, so input with ~700 nested `(` was enough to overflow the
    // default 2 MB test thread. The state-machine parser handles `(` via
    // the `Pending::Group` frame on the heap stack, so paren depth is
    // bounded by heap rather than Rust call stack.
    //
    // 10^4 parens here exercises the iterative path well past any
    // recursive limit while keeping the test fast.
    let input: String = "(".repeat(10_000) + "1" + &")".repeat(10_000);
    let expr = parse_expression(&input).expect("deep parens should parse");
    // The outermost AST node should be a Grouped (the iterative state
    // machine wraps each `(` reduce step as Grouped). We can't easily
    // recurse into the AST to count layers (that walk would itself
    // overflow), so just check the outermost shape.
    assert!(matches!(&expr, Expr::Grouped(_)));
}

#[test]
fn parses_deeply_nested_concatenation_without_overflow() {
    // Pre-fix, `parse_brace_primary` recursively called `parse_expression`
    // for each concat item ([parser.rs:1390](src/parser.rs)), so
    // `{{{...}}}` overflowed at depth ~7100 in release (~4 frames per
    // level × ~300 bytes/frame on an 8 MB main thread). The iterative
    // driver now consumes `{` directly in `parse_expr_bp` via the
    // `Pending::Brace` frame; concat depth is bounded by heap, not
    // stack. Runs on the default 2 MB test thread.
    let input: String = "{".repeat(100_000) + "1'b1" + &"}".repeat(100_000);
    let expr = parse_expression(&input).expect("deep concatenation should parse");
    assert!(matches!(&expr, Expr::Concatenation { .. }));
}

#[test]
fn parses_deeply_nested_replication_without_overflow() {
    // `{1{{1{{1{...}}}}}}` — every level is a single-item replication
    // wrapping the next-deeper replication. Pre-fix, both
    // `parse_brace_primary` and its inner `parse_concatenation_items`
    // recursed via `parse_expression` for the count and inner items.
    // Now both consume tokens through `Pending::Brace` /
    // `Pending::Replication` heap frames in the same iterative driver.
    let input: String = "{1{".repeat(50_000) + "1'b1" + &"}}".repeat(50_000);
    let expr = parse_expression(&input).expect("deep replication should parse");
    assert!(matches!(&expr, Expr::Replication { .. }));
}

#[test]
fn parses_deeply_nested_system_function_without_overflow() {
    // `$signed($signed($signed(...)))` — single-arg cast nesting. Pre-fix,
    // `parse_system_function_call` re-entered `parse_expression` for the
    // arg ([parser.rs:1321](src/parser.rs)), overflowing at ~8000 in
    // release. The iterative driver now consumes `$name(` and pushes a
    // `Pending::SystemArgs` frame; depth is bounded by heap.
    let input: String = "$signed(".repeat(100_000) + "4'sd1" + &")".repeat(100_000);
    let expr = parse_expression(&input).expect("deep $signed should parse");
    assert!(matches!(&expr, Expr::SystemCall { name, .. } if name == "$signed"));
}

#[test]
fn parses_deeply_nested_math_function_without_overflow() {
    // `$pow(2, $pow(2, $pow(2, ...)))` — two-arg math function nesting
    // along the second arg. Same `Pending::SystemArgs` path, but
    // exercises the comma → next-arg state of the frame as well as the
    // arity check fired at finalization.
    let mut input = String::new();
    let n = 50_000;
    for _ in 0..n {
        input.push_str("$pow(2, ");
    }
    input.push('0');
    for _ in 0..n {
        input.push(')');
    }
    let expr = parse_expression(&input).expect("deep $pow should parse");
    assert!(matches!(&expr, Expr::SystemCall { name, .. } if name == "$pow"));
}

#[test]
fn parses_deeply_nested_concat_inside_replication_inside_signed_without_overflow() {
    // Mixed shape: `$signed({1{$signed({1{...}})}})` — alternating
    // SystemArgs / Replication / Concatenation frames on the heap stack.
    // Verifies the three new Pending variants compose without
    // re-introducing recursion at a transition.
    let n = 30_000;
    let mut input = String::new();
    for _ in 0..n {
        input.push_str("$signed({1{");
    }
    input.push_str("1'b1");
    for _ in 0..n {
        input.push_str("}})");
    }
    let expr = parse_expression(&input).expect("deep mixed should parse");
    assert!(matches!(&expr, Expr::SystemCall { name, .. } if name == "$signed"));
}

#[test]
fn evaluates_deeply_nested_concat_through_full_pipeline() {
    // End-to-end: parser + annotate + validate + evaluate at a depth
    // that pre-fix would crash the parser (~7100). Now all three
    // pipelines are iterative, so this evaluates cleanly on the
    // default 2 MB test thread.
    let n = 50_000;
    let input: String = "{".repeat(n) + "1'b1" + &"}".repeat(n);
    let evaluation = evaluate_input(&input).expect("deep concat should evaluate");
    assert_eq!(evaluation.output, "1'b1");
}

#[test]
fn parses_deep_nested_ternary_without_overflow() {
    // `a ? b : c ? d : e ? f : ...` — right-associative ternary. In the
    // recursive Pratt parser, each `?` added two stack frames (one for
    // the then-branch parse, one for the else-branch recursion). Now both
    // branches are state-machine frames on the heap.
    let n = 10_000;
    let mut input = String::new();
    for i in 0..n {
        input.push_str(&format!("{} ? {} : ", i % 10, i % 10));
    }
    input.push('0');
    let expr = parse_expression(&input).expect("deep ternary should parse");
    assert!(matches!(&expr, Expr::Conditional { .. }));
}

#[test]
fn drop_of_deep_grouped_ast_does_not_overflow() {
    // Without `impl Drop for Expr`, dropping a 10^5-deep `Grouped` chain
    // overflows the stack — auto-derived Drop walks each `Box<Expr>`
    // recursively, costing one frame per layer. The custom Drop in
    // parser.rs flattens the descent into a heap-allocated worklist so
    // this is O(1) Rust stack regardless of depth.
    //
    // Runs on the default test thread (2 MB stack); pre-fix this would
    // crash well before reaching the 10^5 mark.
    let mut e = Expr::Identifier(String::new());
    for _ in 0..100_000 {
        e = Expr::Grouped(Box::new(e));
    }
    drop(e);
}

#[test]
fn drop_of_deep_unary_ast_does_not_overflow() {
    // Same shape as the Grouped test, but exercising the Unary recursion
    // arm of `steal_expr_children`.
    let mut e = Expr::Identifier(String::new());
    for _ in 0..100_000 {
        e = Expr::Unary {
            op: UnaryOp::LogicalNot,
            expr: Box::new(e),
        };
    }
    drop(e);
}

#[test]
fn drop_of_deep_binary_ast_does_not_overflow() {
    // Two-child variant: each level adds one to the lhs spine while the
    // rhs is a fresh leaf. Confirms the Binary arm of
    // `steal_expr_children` peels both children iteratively.
    let mut e = Expr::Identifier(String::new());
    for _ in 0..100_000 {
        e = Expr::Binary {
            op: BinaryOp::Add,
            lhs: Box::new(e),
            rhs: Box::new(Expr::Identifier(String::new())),
        };
    }
    drop(e);
}

#[test]
fn annotate_of_deep_grouped_does_not_overflow() {
    // Pre-fix: `annotate` recursed on `Expr::Grouped(inner)` once per
    // layer, so a 10^5-deep parens chain crashed before any
    // validator/evaluator pass even started. The iterative CES driver
    // bounds Rust stack to O(1) regardless of input depth, and `impl
    // Drop for Annotated` keeps the resulting deep `Box<Annotated>`
    // chain from crashing at end-of-scope. Runs on the default 2 MB
    // test thread.
    let mut e = Expr::Literal(parse_integer("1").expect("literal should parse"));
    for _ in 0..100_000 {
        e = Expr::Grouped(Box::new(e));
    }
    let session = Session::new();
    let annotated = crate::eval::annotate(&e, &session).expect("deep grouped annotates");
    drop(annotated);
}

#[test]
fn annotate_of_deep_binary_does_not_overflow() {
    // Same shape as the Grouped test, but exercising the Binary arm of
    // the CES driver (one Combine + two child Visits per level).
    let mut e = Expr::Literal(parse_integer("1").expect("literal should parse"));
    for _ in 0..100_000 {
        e = Expr::Binary {
            op: BinaryOp::Add,
            lhs: Box::new(e),
            rhs: Box::new(Expr::Literal(
                parse_integer("1").expect("literal should parse"),
            )),
        };
    }
    let session = Session::new();
    let annotated = crate::eval::annotate(&e, &session).expect("deep binary annotates");
    drop(annotated);
}

#[test]
fn annotate_of_deep_unary_does_not_overflow() {
    // Exercises the Unary arm of the CES driver. `LogicalNot` keeps the
    // result-type meta computation simple (always 1-bit unsigned) while
    // still walking the same Visit/Combine path as the deeper unary
    // ops.
    let mut e = Expr::Literal(parse_integer("1").expect("literal should parse"));
    for _ in 0..100_000 {
        e = Expr::Unary {
            op: UnaryOp::LogicalNot,
            expr: Box::new(e),
        };
    }
    let session = Session::new();
    let annotated = crate::eval::annotate(&e, &session).expect("deep unary annotates");
    drop(annotated);
}

#[test]
fn semantic_check_of_deep_grouped_does_not_overflow() {
    // semantic_check = annotate + validate_annotated. With both
    // iterative, a 10^5-deep Grouped chain validates on the default
    // 2 MB test thread. The evaluator is still recursive in this
    // phase, so we can't go through evaluate_input here.
    let mut e = Expr::Literal(parse_integer("1").expect("literal should parse"));
    for _ in 0..100_000 {
        e = Expr::Grouped(Box::new(e));
    }
    let session = Session::new();
    crate::eval::semantic_check(&e, &session).expect("deep grouped semantic-checks");
}

#[test]
fn semantic_check_of_deep_binary_does_not_overflow() {
    // Exercises validate_annotated's Binary arm, where the real-operand
    // rejection rule reads the children's cached annotations at depth.
    let mut e = Expr::Literal(parse_integer("1").expect("literal should parse"));
    for _ in 0..100_000 {
        e = Expr::Binary {
            op: BinaryOp::Add,
            lhs: Box::new(e),
            rhs: Box::new(Expr::Literal(
                parse_integer("1").expect("literal should parse"),
            )),
        };
    }
    let session = Session::new();
    crate::eval::semantic_check(&e, &session).expect("deep binary semantic-checks");
}

#[test]
fn semantic_check_of_deep_conditional_does_not_overflow() {
    // Right-recursive Conditional spine (else-arm chain) — the shape
    // produced by `1?1:1?1:...0`. Exercises validate_annotated's
    // Conditional arm at depth.
    let mut e = Expr::Literal(parse_integer("0").expect("literal should parse"));
    for _ in 0..100_000 {
        e = Expr::Conditional {
            cond: Box::new(Expr::Literal(
                parse_integer("1").expect("literal should parse"),
            )),
            then_expr: Box::new(Expr::Literal(
                parse_integer("1").expect("literal should parse"),
            )),
            else_expr: Box::new(e),
        };
    }
    let session = Session::new();
    crate::eval::semantic_check(&e, &session).expect("deep conditional semantic-checks");
}

// End-to-end deep-chain regression suite. Every test runs on the default
// 2 MB cargo-test thread and exercises the full annotate → validate →
// evaluate pipeline through `evaluate_input`. Pre-fix (depth ~2000), most
// of these crashed with "fatal runtime error: stack overflow"; post-fix
// they evaluate cleanly. Depth 10^5 matches the existing parser / Drop
// suite at the top of this file.

// End-to-end deep-chain tests run on cargo's 2 MB test thread. The depth
// is set high enough to crash the pre-fix code (recursive walkers crashed
// at ~1500-2000 levels) but low enough to fit comfortably under the 2 MB
// budget in debug builds, where each parser/eval frame consumes ~80 bytes
// of stack space. Release builds and direct-build tests (the
// `*_does_not_overflow` suite above, which constructs Expr trees in
// memory) handle 10^5 depths.
const DEEP_CHAIN_DEPTH: usize = 10_000;

#[test]
fn deep_binary_integer_chain_evaluates() {
    // `1+1+...+1` — left-associative integer chain. Hits the
    // BinaryArith Combine in the CES driver.
    let n = DEEP_CHAIN_DEPTH;
    let chain: String = std::iter::once("1".to_string())
        .chain(std::iter::repeat_n("+1".to_string(), n))
        .collect();
    let evaluation = evaluate_input(&chain).expect("deep + chain evaluates");
    assert_eq!(evaluation.output, format!("32'sd{}", n + 1));
}

#[test]
fn deep_grouped_chain_evaluates() {
    // `((((...1...))))`. Hits the AnnotatedKind::Grouped pass-through
    // in the CES driver and the iterative `Drop for Annotated`.
    let n = DEEP_CHAIN_DEPTH;
    let input: String = "(".repeat(n) + "1" + &")".repeat(n);
    let evaluation = evaluate_input(&input).expect("deep grouped evaluates");
    assert_eq!(evaluation.output, "32'sd1");
}

#[test]
fn deep_unary_logical_not_chain_evaluates() {
    // `!!!!...!1`. Hits the UnaryLogicalNot Combine in the CES driver.
    // Even-depth chain reduces to `1`, odd-depth to `0` (each `!` flips).
    let n = DEEP_CHAIN_DEPTH;
    let input: String = "!".repeat(n) + "1";
    let evaluation = evaluate_input(&input).expect("deep ! chain evaluates");
    let expected = if n.is_multiple_of(2) { "1'b1" } else { "1'b0" };
    assert_eq!(evaluation.output, expected);
}

#[test]
fn deep_unary_minus_chain_evaluates() {
    // `---...-1`. Hits the UnaryArith (Minus) Combine. Each `-` flips
    // the sign; even-depth ⇒ original, odd-depth ⇒ negated.
    let n = DEEP_CHAIN_DEPTH;
    let input: String = "-".repeat(n) + "1";
    let evaluation = evaluate_input(&input).expect("deep unary minus chain evaluates");
    let expected = if n.is_multiple_of(2) {
        "32'sd1"
    } else {
        "-32'sd1"
    };
    assert_eq!(evaluation.output, expected);
}

#[test]
fn deep_conditional_else_chain_evaluates() {
    // `1?1:1?1:...:0`. Right-recursive on the else arm. cond=1 at every
    // level so the chosen branch is the leftmost `then` (1). Tests
    // ConditionalChoose + ConditionalFinalize Combines.
    let n = DEEP_CHAIN_DEPTH;
    let mut input = String::with_capacity(n * 6);
    for _ in 0..n {
        input.push_str("1 ? 1 : ");
    }
    input.push('0');
    let evaluation = evaluate_input(&input).expect("deep ?: chain evaluates");
    assert_eq!(evaluation.output, "32'sd1");
}

#[test]
fn deep_relational_chain_evaluates() {
    // `1 < 1 < 1 < ... < 1`. Left-associative chain of `<`. The
    // BinaryRelational Combine handles each level on the iterative path.
    let n = DEEP_CHAIN_DEPTH;
    let chain: String = std::iter::once("1".to_string())
        .chain(std::iter::repeat_n("<1".to_string(), n))
        .collect();
    evaluate_input(&chain).expect("deep < chain evaluates");
}

#[test]
fn deep_real_addition_chain_evaluates() {
    // `1.0 + 1.0 + ... + 1.0`. Hits evaluate_expr_as_real's iterative
    // CES driver with the BinaryArith real Combine.
    let n = DEEP_CHAIN_DEPTH;
    let chain: String = std::iter::once("1.0".to_string())
        .chain(std::iter::repeat_n("+1.0".to_string(), n))
        .collect();
    let evaluation = evaluate_input(&chain).expect("deep real + chain evaluates");
    let expected = format!("{:?}", (n + 1) as f64);
    assert_eq!(evaluation.output, expected);
}

#[test]
fn deep_power_exponent_chain_evaluates() {
    // `2 ** (1+1+...+1)`. The self-determined exponent is scheduled on the
    // standard iterative integer work stack, so a deep operand chain stays
    // off the Rust call stack.
    let n = DEEP_CHAIN_DEPTH;
    let inner: String = std::iter::once("1".to_string())
        .chain(std::iter::repeat_n("+1".to_string(), n))
        .collect();
    let chain = format!("2 ** ({inner})");
    evaluate_input(&chain).expect("deep power exponent evaluates");
}

#[test]
fn deeply_right_nested_integer_power_stays_on_work_stack() {
    // `1 ** (1 ** (... ** 1))`. Each RHS power must be scheduled on the
    // evaluator's heap work stack; recursively starting a fresh evaluator for
    // every exponent overflows the default test-thread stack around depth 1K.
    let n = DEEP_CHAIN_DEPTH;
    let chain = "1**(".repeat(n) + "1" + &")".repeat(n);
    let evaluation = evaluate_input(&chain).expect("right-nested power chain evaluates");
    assert_eq!(evaluation.output, "32'sd1");
}

#[test]
fn deep_select_index_chain_evaluates() {
    // `r[1+1+...+1]` — bit-select index is a deep chain. Hits the
    // iterative `evaluate_select_subexpr` path that replaced the
    // recursive `evaluate_expr_in_context` call inside
    // `evaluate_bit_select`.
    let n = DEEP_CHAIN_DEPTH;
    let mut session = Session::new();
    session.eval("reg [3:0] r").expect("decl");
    let chain: String = std::iter::once("1".to_string())
        .chain(std::iter::repeat_n("+1".to_string(), n))
        .collect();
    let outcome = session
        .eval(&format!("r[{chain}]"))
        .expect("deep select index evaluates");
    // Index out of range → 1'bx (LRM 4.2.1).
    assert_eq!(outcome.output, "1'bx");
}

// Direct-build deep-concat regression suite. The parser uses a recursive
// `parse_expression` for each concat item, so an end-to-end `{{{…}}}` at
// 10^5 levels overflows in the parser before reaching the evaluator. To
// exercise the eval pipeline at full depth, these tests construct the
// `Expr` tree in memory and feed it directly to `annotate` /
// `semantic_check` / `evaluate_annotated` — the same convention the
// `*_does_not_overflow` suite earlier in this file uses for Grouped and
// Binary chains.
//
// Pre-fix: `evaluate_annotated`'s CES driver fell through to the
// recursive `evaluate_expr_in_context` for `AnnotatedKind::Concatenation`,
// and `validate_annotated`'s eager bit-collection task re-entered the
// recursive concatenation walker. Both walkers re-walked nested
// items per level, which combined a stack-overflow risk with O(N²)
// re-walk cost (leftmost-item meta inference + `is_indefinite_width(item)`
// per concat level). Post-fix: the eval CES handles Concatenation /
// Replication directly off cached `Annotated::meta()`, and validation
// reads the cached widths without re-walking.

#[test]
fn annotate_of_deep_concatenation_does_not_overflow() {
    // Wraps `1'b1` in 10^5 single-item concatenations. annotate's
    // Concatenation Combine pops one child Annotated and builds the
    // parent. impl Drop for Annotated handles the resulting deep
    // Vec<Annotated> chain iteratively at end-of-scope.
    let mut e = Expr::Literal(parse_integer("1'b1").expect("literal should parse"));
    for _ in 0..100_000 {
        e = Expr::Concatenation { items: vec![e] };
    }
    let session = Session::new();
    let annotated = crate::eval::annotate(&e, &session).expect("deep concat annotates");
    drop(annotated);
}

#[test]
fn semantic_check_of_deep_concatenation_does_not_overflow() {
    // semantic_check = annotate + validate_annotated. The validator
    // dispatches each concat via ConcatItem (iterative) and the new
    // PostCheckConcatWidth reads the cached `meta().width` rather than
    // re-walking. Pre-fix this overflowed via the eager bit-collection
    // task re-entering the recursive concatenation walker.
    let mut e = Expr::Literal(parse_integer("1'b1").expect("literal should parse"));
    for _ in 0..100_000 {
        e = Expr::Concatenation { items: vec![e] };
    }
    let session = Session::new();
    crate::eval::semantic_check(&e, &session).expect("deep concat semantic-checks");
}

#[test]
fn evaluate_of_deep_concatenation_does_not_overflow() {
    // End-to-end eval through the iterative annotate + validate +
    // evaluate pipeline. Each concat layer wraps a single `1'b1` so the
    // joined width stays at 1 and the result is `1'b1` independent of
    // depth. Hits the new EvalCombiner::Concatenation arm at every
    // level, popping one IntegerValue per Combine.
    let mut e = Expr::Literal(parse_integer("1'b1").expect("literal should parse"));
    for _ in 0..100_000 {
        e = Expr::Concatenation { items: vec![e] };
    }
    let session = Session::new();
    let value = crate::eval::evaluate_expr(&e, &session).expect("deep concat evaluates");
    assert_eq!(value.canonical(), "1'b1");
}

#[test]
fn evaluate_of_deep_replication_does_not_overflow() {
    // Single-item Replication at every level: `{1{ {1{ ... {1{1'b1}} ... }} }}`.
    // Each layer's count = 1, so the result stays 1-bit. Hits
    // EvalCombiner::ReplicationCountReceived (the count's Visit
    // resolves first) and EvalCombiner::ReplicationFinalize at every
    // level — no re-walk because the inner-item value is popped off
    // the value stack.
    let mut e = Expr::Literal(parse_integer("1'b1").expect("literal should parse"));
    for _ in 0..100_000 {
        e = Expr::Replication {
            count: Box::new(Expr::Literal(parse_integer("1").expect("count literal"))),
            items: vec![e],
        };
    }
    let session = Session::new();
    let value = crate::eval::evaluate_expr(&e, &session).expect("deep replication evaluates");
    assert_eq!(value.canonical(), "1'b1");
}

#[test]
fn evaluate_of_deep_concat_inside_replication_does_not_overflow() {
    // Mixes the two: every other layer alternates Concatenation /
    // Replication. Verifies the lenient-replication-inside-concat
    // dispatch (`push_concat_item_eval` peels Grouped + detects
    // Replication) survives at depth.
    let mut e = Expr::Literal(parse_integer("1'b1").expect("literal should parse"));
    for i in 0usize..100_000 {
        e = if i.is_multiple_of(2) {
            Expr::Concatenation { items: vec![e] }
        } else {
            Expr::Replication {
                count: Box::new(Expr::Literal(parse_integer("1").expect("count literal"))),
                items: vec![e],
            }
        };
    }
    let session = Session::new();
    let value = crate::eval::evaluate_expr(&e, &session).expect("deep mixed evaluates");
    assert_eq!(value.canonical(), "1'b1");
}

#[test]
fn evaluate_of_deep_concatenation_two_items_per_layer_does_not_overflow() {
    // Each concat layer has two items: the first is the next-deeper
    // concat, the second is a fresh `1'b1` sibling. Total result
    // width grows linearly with depth — at depth 10^4 the result is
    // 10001 bits wide, all 1s. Tests that the value-stack drain in
    // EvalCombiner::Concatenation handles wider stacks.
    let n = 10_000usize;
    let mut e = Expr::Literal(parse_integer("1'b1").expect("literal should parse"));
    for _ in 0..n {
        e = Expr::Concatenation {
            items: vec![
                e,
                Expr::Literal(parse_integer("1'b1").expect("sibling literal")),
            ],
        };
    }
    let session = Session::new();
    let value = crate::eval::evaluate_expr(&e, &session).expect("deep two-item concat evaluates");
    let total_bits = n + 1;
    let expected_digits = "1".repeat(total_bits);
    assert_eq!(
        value.canonical(),
        format!("{total_bits}'b{expected_digits}")
    );
}

#[test]
fn evaluate_of_deep_real_math_chain_does_not_overflow() {
    // `$ln($ln(...$ln(1.0)))` chain. Real-result arity-1 math fn. The
    // pre-fix recursion came from `evaluate_real_math_function` calling
    // `evaluate_expr_as_real` per arg; the unified driver dispatches the
    // arg via a `RealCombiner::MathFunction` task, so depth turns into
    // work-stack growth, not Rust-stack growth.
    let mut e = Expr::RealLiteral(1.0);
    for _ in 0..50_000 {
        e = Expr::SystemCall {
            name: "$ln".to_string(),
            args: vec![SystemArg::Expr(e)],
        };
    }
    let session = Session::new();
    let value = crate::eval::evaluate_expr(&e, &session).expect("deep $ln chain evaluates");
    // $ln(1.0) = 0.0; $ln(0.0) = -inf; $ln(-inf) = NaN; thereafter NaN.
    // Surface result is NaN regardless of exact transition; we just
    // assert it's a real value (no crash).
    assert!(matches!(value, crate::value::Value::Real(_)));
}

#[test]
fn evaluate_of_deep_pow_chain_does_not_overflow() {
    // `$pow(2, $pow(2, ... $pow(2, 1)))` — real-result arity-2.
    // Exercises the recursive `evaluate_real_math_function` path that
    // crashed pre-fix at ~10K depth.
    let mut e = Expr::Literal(parse_integer("1").expect("inner literal"));
    for _ in 0..50_000 {
        e = Expr::SystemCall {
            name: "$pow".to_string(),
            args: vec![
                SystemArg::Expr(Expr::Literal(parse_integer("2").expect("base literal"))),
                SystemArg::Expr(e),
            ],
        };
    }
    let session = Session::new();
    let value = crate::eval::evaluate_expr(&e, &session).expect("deep $pow chain evaluates");
    assert!(matches!(value, crate::value::Value::Real(_)));
}

#[test]
fn evaluate_of_deep_clog2_chain_does_not_overflow() {
    // `$clog2($clog2(...$clog2(4)))` — integer-result. Pre-fix
    // recursion came from `evaluate_clog2` calling
    // `evaluate_expr_in_context` for the arg; now dispatched via the
    // `EvalCombiner::Clog2` bridge.
    let mut e = Expr::Literal(parse_integer("4").expect("inner literal"));
    for _ in 0..50_000 {
        e = Expr::SystemCall {
            name: "$clog2".to_string(),
            args: vec![SystemArg::Expr(e)],
        };
    }
    let session = Session::new();
    let value = crate::eval::evaluate_expr(&e, &session).expect("deep $clog2 chain evaluates");
    assert!(matches!(value, crate::value::Value::Integer(_)));
}

#[test]
fn evaluate_of_deep_rtoi_itor_alternation_does_not_overflow() {
    // Strict alternation of `$rtoi($itor($rtoi($itor(...))))` — the
    // case the unified driver was added for. Pre-fix this recursed
    // through `evaluate_real_conversion_expr` ↔ `visit_real_eval`'s
    // IntegerToReal arm at every level (4 frames per pair).
    let mut e = Expr::Literal(parse_integer("1").expect("inner literal"));
    for i in 0..50_000usize {
        e = if i.is_multiple_of(2) {
            Expr::SystemCall {
                name: "$itor".to_string(),
                args: vec![SystemArg::Expr(e)],
            }
        } else {
            Expr::SystemCall {
                name: "$rtoi".to_string(),
                args: vec![SystemArg::Expr(e)],
            }
        };
    }
    // Outer is RealToInteger (i=49_999, odd) → integer result.
    let session = Session::new();
    let value =
        crate::eval::evaluate_expr(&e, &session).expect("deep $rtoi/$itor alternation evaluates");
    assert!(matches!(value, crate::value::Value::Integer(_)));
}

#[test]
fn evaluate_of_deep_realtobits_bitstoreal_alternation_does_not_overflow() {
    // Sister test for $realtobits/$bitstoreal — same shape, tests the
    // 64-bit bitcast bridge path.
    let mut e = Expr::RealLiteral(1.0);
    for i in 0..30_000usize {
        e = if i.is_multiple_of(2) {
            Expr::SystemCall {
                name: "$realtobits".to_string(),
                args: vec![SystemArg::Expr(e)],
            }
        } else {
            Expr::SystemCall {
                name: "$bitstoreal".to_string(),
                args: vec![SystemArg::Expr(e)],
            }
        };
    }
    let session = Session::new();
    let value = crate::eval::evaluate_expr(&e, &session)
        .expect("deep $realtobits/$bitstoreal alternation evaluates");
    // Outer kind decides result type.
    let _ = value;
}

#[test]
fn evaluate_of_pow_with_deep_integer_arg_does_not_overflow() {
    // `$pow(2, 1+1+...+1)` — pre-fix the integer arg fell through
    // `visit_real_eval`'s integer fallback to the recursive
    // `evaluate_expr_in_context`. Now an integer subtree visited from a
    // real-mode parent goes through the integer CES with a
    // `RealCombiner::CoerceFromInteger` bridge.
    let mut chain = Expr::Literal(parse_integer("1").expect("seed literal"));
    for _ in 0..50_000 {
        chain = Expr::Binary {
            op: BinaryOp::Add,
            lhs: Box::new(chain),
            rhs: Box::new(Expr::Literal(parse_integer("1").expect("step literal"))),
        };
    }
    let e = Expr::SystemCall {
        name: "$pow".to_string(),
        args: vec![
            SystemArg::Expr(Expr::Literal(parse_integer("2").expect("base literal"))),
            SystemArg::Expr(chain),
        ],
    };
    let session = Session::new();
    let value =
        crate::eval::evaluate_expr(&e, &session).expect("$pow(2, deep integer chain) evaluates");
    assert!(matches!(value, crate::value::Value::Real(_)));
}

#[test]
fn evaluate_of_itor_with_deep_integer_arg_does_not_overflow() {
    // `$itor(1+1+...+1)` — forces a single int→real bridge over a deep
    // integer subtree.
    let mut chain = Expr::Literal(parse_integer("1").expect("seed literal"));
    for _ in 0..50_000 {
        chain = Expr::Binary {
            op: BinaryOp::Add,
            lhs: Box::new(chain),
            rhs: Box::new(Expr::Literal(parse_integer("1").expect("step literal"))),
        };
    }
    let e = Expr::SystemCall {
        name: "$itor".to_string(),
        args: vec![SystemArg::Expr(chain)],
    };
    let session = Session::new();
    let value =
        crate::eval::evaluate_expr(&e, &session).expect("$itor(deep integer chain) evaluates");
    assert!(matches!(value, crate::value::Value::Real(_)));
}

#[test]
fn evaluate_of_power_with_deep_non_arith_exponent_does_not_overflow() {
    // `2 ** (1<1<...<1)` — the annotated exponent is scheduled directly on
    // the iterative evaluator work stack, so depth stays off the C stack.
    let mut chain = Expr::Literal(parse_integer("1").expect("seed literal"));
    for _ in 0..50_000 {
        chain = Expr::Binary {
            op: BinaryOp::LessThan,
            lhs: Box::new(chain),
            rhs: Box::new(Expr::Literal(parse_integer("1").expect("step literal"))),
        };
    }
    let e = Expr::Binary {
        op: BinaryOp::Power,
        lhs: Box::new(Expr::Literal(parse_integer("2").expect("base"))),
        rhs: Box::new(Expr::Grouped(Box::new(chain))),
    };
    let session = Session::new();
    let value = crate::eval::evaluate_expr(&e, &session)
        .expect("`2 ** (deep < chain)` evaluates without overflow");
    assert!(matches!(value, crate::value::Value::Integer(_)));
}

#[test]
fn evaluate_of_replication_with_deep_count_does_not_overflow() {
    // `{(1+1+...+1){1'b1}}` — pre-fix
    // `evaluate_replication_count_allow_zero` evaluated the count
    // through the recursive `evaluate_expr_in_context`. Re-routed
    // through `evaluate_subexpr_as_integer` so the deep count chain
    // doesn't overflow.
    let mut count_chain = Expr::Literal(parse_integer("1").expect("seed literal"));
    for _ in 0..50_000 {
        count_chain = Expr::Binary {
            op: BinaryOp::Add,
            lhs: Box::new(count_chain),
            rhs: Box::new(Expr::Literal(parse_integer("1").expect("step literal"))),
        };
    }
    let e = Expr::Replication {
        count: Box::new(Expr::Grouped(Box::new(count_chain))),
        items: vec![Expr::Literal(
            parse_integer("1'b1").expect("inner bit literal"),
        )],
    };
    let session = Session::new();
    let _ = crate::eval::evaluate_expr(&e, &session);
    // Result is a 50_001-bit vector of 1s; we only need to confirm
    // no crash. (The reified value would be huge, so don't materialise.)
}

#[test]
fn assign_with_deeply_nested_lvalue_concat_does_not_overflow() {
    // `{{{...{a}...}}} = 1'b1` — pre-fix this crashed at ~50K depth
    // because `expression_to_lvalue` recursed on Concat items,
    // `lvalue_meta` walked Concat layers recursively, and
    // `flatten_lvalue_leaves` did too. Plus the resulting
    // `Box<LValue>` chain dropped recursively at end of scope without
    // an `impl Drop for LValue`. All four sites are now iterative.
    let depth = 50_000usize;
    let mut input = String::with_capacity(depth * 2 + 16);
    input.push_str("reg a;");
    for _ in 0..depth {
        input.push('{');
    }
    input.push('a');
    for _ in 0..depth {
        input.push('}');
    }
    input.push_str("=1'b1");
    let mut session = Session::new();
    session
        .eval(&input)
        .expect("deep lvalue concat assignment evaluates without overflow");
}

#[test]
fn assign_with_wide_lvalue_concat_does_not_overflow() {
    // `{a,a,...,a} = N'b0...0` — the wide-flat shape of the same
    // path. Exercises `flatten_lvalue_leaves` over a single Concat
    // with thousands of sibling items rather than a deep stack.
    let count = 30_000usize;
    let mut input = String::with_capacity(count * 4 + 32);
    input.push_str("reg a;");
    input.push('{');
    input.push('a');
    for _ in 0..count {
        input.push_str(",a");
    }
    input.push_str("}=");
    input.push_str(&(count + 1).to_string());
    input.push_str("'b");
    for _ in 0..(count + 1) {
        input.push('0');
    }
    let mut session = Session::new();
    session
        .eval(&input)
        .expect("wide lvalue concat assignment evaluates without overflow");
}
