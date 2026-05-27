use std::collections::HashMap;

use num_bigint::{BigInt, BigUint, Sign};
use num_traits::{FromPrimitive, One, Signed, ToPrimitive, Zero};

use crate::{RegRange, RegValue, Session};
use crate::parser::{
    BinaryOp, Expr, LValue, MathFunctionKind, RealConversionKind, SelectKind, UnaryOp,
};
use crate::value::{
    Base, IntegerValue, LogicBit, Value, bits_to_biguint, bitwise_and_bits, bitwise_not_bit,
    bitwise_or_bits, bitwise_xnor_bits, bitwise_xor_bits,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ExprMeta {
    width: usize,
    signed: bool,
    // Inferred display base — leftmost operand wins for binary ops.
    // Used when constructing arithmetic results; ignored when ExprMeta is
    // passed downward as context (literals keep their own base).
    base: Base,
}

// Static-semantics pre-pass. Every public expression-evaluation entry point
// runs `semantic_check` before touching the evaluator, so structural errors
// like real-typed select indices, $bitstoreal width mismatches, or invalid
// system-task uses surface here — *before* runtime behaviour like short-circuit
// branch choice, zero-rep collapse, or x-bit propagation could hide them.
// Errors raised here carry the "Semantic error: " prefix so the rejection
// stage is visible to the user, paralleling the "Syntax error: " prefix that
// `parse_statements`'s call site adds to lexer/parser errors.
pub(crate) fn semantic_check(expr: &Expr, session: &Session) -> Result<(), String> {
    validate_expr_structure(expr, session).map_err(|e| format!("Semantic error: {e}"))
}

pub(crate) fn evaluate_expr(expr: &Expr, session: &Session) -> Result<Value, String> {
    semantic_check(expr, session)?;
    if expression_is_real(expr, session) {
        evaluate_expr_as_real(expr, session).map(Value::Real)
    } else {
        evaluate_expr_in_context(expr, None, session).map(Value::Integer)
    }
}

// Entrypoint for the blocking-assignment driver in lib.rs. Builds the
// LRM 5.6 context-determined operand context from the reg's declared
// (width, signed, base) and runs the RHS through the standard integer
// pipeline, so a wider/narrower RHS extends or truncates exactly the way
// a literal does in an arithmetic context. The reg's base flows in for
// the leftmost-base inference rule; the caller still re-stamps the
// reg's stored base on the result.
//
// A real RHS is implicitly converted per LRM §3.5.3: round to nearest
// with ties away from zero (distinct from `$rtoi`'s truncate-toward-zero
// rule). NaN / ±∞ have no integer image, so the lvalue is filled with x
// bits at its declared width — matching how `$rtoi` surfaces "no defined
// integer" rather than silently mapping to zero.
pub(crate) fn evaluate_assignment_rhs(
    rhs: &Expr,
    width: usize,
    signed: bool,
    base: Base,
    session: &Session,
) -> Result<IntegerValue, String> {
    semantic_check(rhs, session)?;
    if expression_is_real(rhs, session) {
        let real_val = evaluate_expr_as_real(rhs, session)?;
        return Ok(match real_to_integer_bigint(real_val) {
            Some(bigint) => IntegerValue::from_bigint(bigint, width, signed, base),
            None => IntegerValue::all_x(width, signed, base),
        });
    }
    let context = ExprMeta {
        width,
        signed,
        base,
    };
    evaluate_expr_in_context(rhs, Some(context), session)
}

// Self-determined evaluation of an integer-typed constant expression
// (used by the reg-declaration range halves). Mirrors the `None` context
// path the evaluator takes for the top-level expression in a calculator
// line.
pub(crate) fn evaluate_constant_expr(
    expr: &Expr,
    session: &Session,
) -> Result<IntegerValue, String> {
    semantic_check(expr, session)?;
    evaluate_expr_in_context(expr, None, session)
}


// LRM §5.1.1 / Table 5-2 / §5.1.5: an expression's *result* type is real
// only for arithmetic ops with at least one real operand and for
// conditionals where at least one branch is real. Relational, equality,
// and logical ops always produce a 1-bit integer even with real operands;
// this helper reports that result-type, not whether any operand is real.
// Operators that aren't legal on reals (Table 5-3 — modulus, case
// equality, bitwise, reductions, shifts, concatenation, replication,
// $signed / $unsigned) are reported as integer-typed; their evaluators
// reject real operands explicitly so the diagnostic names the operator.
// LRM 17.4: simulation control tasks have no return value, so they can
// never appear inside an expression. The parser produces an `Expr::SystemTask`
// for `$finish` / `$stop`; the lib-level driver recognises a bare task at the
// top of the AST and exits, but every nested occurrence hits one of the
// evaluator paths below and surfaces this message. Phrased to make clear it
// is the function-call usage that is wrong, not the task itself.
fn task_in_expression_error(name: &str) -> String {
    format!("{name}() is a system task, it cannot be called as a function.")
}

pub(crate) fn expression_is_real(expr: &Expr, session: &Session) -> bool {
    match expr {
        Expr::Literal(_) => false,
        Expr::RealLiteral(_) => true,
        Expr::Grouped(inner) => expression_is_real(inner, session),
        Expr::Unary { op, expr } => match op {
            UnaryOp::Plus | UnaryOp::Minus => expression_is_real(expr, session),
            UnaryOp::BitwiseNot
            | UnaryOp::LogicalNot
            | UnaryOp::ReductionAnd
            | UnaryOp::ReductionNand
            | UnaryOp::ReductionOr
            | UnaryOp::ReductionNor
            | UnaryOp::ReductionXor
            | UnaryOp::ReductionXnor => false,
        },
        Expr::Binary { op, lhs, rhs } => match op {
            BinaryOp::Add
            | BinaryOp::Subtract
            | BinaryOp::Multiply
            | BinaryOp::Divide
            | BinaryOp::Power => {
                expression_is_real(lhs, session) || expression_is_real(rhs, session)
            }
            BinaryOp::Modulus
            | BinaryOp::CaseEqual
            | BinaryOp::CaseNotEqual
            | BinaryOp::BitwiseAnd
            | BinaryOp::BitwiseOr
            | BinaryOp::BitwiseXor
            | BinaryOp::BitwiseXnor
            | BinaryOp::LogicalShiftLeft
            | BinaryOp::LogicalShiftRight
            | BinaryOp::ArithmeticShiftLeft
            | BinaryOp::ArithmeticShiftRight
            | BinaryOp::LessThan
            | BinaryOp::GreaterThan
            | BinaryOp::LessThanOrEqual
            | BinaryOp::GreaterThanOrEqual
            | BinaryOp::Equal
            | BinaryOp::NotEqual
            | BinaryOp::LogicalAnd
            | BinaryOp::LogicalOr => false,
        },
        Expr::Conditional {
            then_expr,
            else_expr,
            ..
        } => {
            expression_is_real(then_expr, session) || expression_is_real(else_expr, session)
        }
        Expr::Concatenation { .. }
        | Expr::Replication { .. }
        | Expr::SignCast { .. }
        | Expr::BaseCast { .. } => false,
        // LRM 17.8: $itor and $bitstoreal yield real values; $rtoi and
        // $realtobits yield integers (32-bit signed and 64-bit unsigned
        // respectively), so only the first two participate in real-result
        // type propagation.
        Expr::RealConversion { kind, .. } => matches!(
            kind,
            RealConversionKind::IntegerToReal | RealConversionKind::BitsToReal
        ),
        // LRM 17.11: every math function except $clog2 returns real.
        Expr::MathFunction { kind, .. } => kind.is_real_result(),
        // System tasks have no type — they are not values. Reporting
        // "not real" routes the rejection through the integer pipeline,
        // which surfaces the task-in-expression diagnostic.
        Expr::SystemTask { .. } => false,
        // An identifier is real-typed iff it resolves to a `real` reg
        // (LRM 4.8). Unknown names resolve to integer here so the
        // downstream integer pipeline can surface the "undeclared
        // identifier" diagnostic at its usual position; treating an
        // unknown name as real would otherwise route the error through
        // the real path and produce a less specific message.
        Expr::Identifier(name) => session
            .lookup(name)
            .map(|reg| reg.is_real())
            .unwrap_or(false),
        // Bit-select / part-select on a vector reg is always
        // integer-typed (LRM 4.7: part-select is unsigned). A select on
        // a real-array reg is the one exception: `r[i]` yields a real
        // element. Selects on a scalar `real` are prohibited per LRM
        // 4.8.1 and the validator (`infer_select_meta`) rejects them
        // before this function runs, so reaching here with a scalar
        // real is a missed validator wiring.
        Expr::Select { name, kind, inner } => match session.lookup(name) {
            Some(reg) if reg.is_real_array() => {
                matches!(kind, SelectKind::Bit { .. }) && inner.is_none()
            }
            Some(reg) if reg.is_real() => {
                unreachable!(
                    "validator rejects select on scalar real `{name}` before evaluation (LRM 4.8.1)"
                );
            }
            _ => false,
        },
    }
}

// LRM §5.1.7: when one operand of a relational / equality is real, "the
// other operand shall be converted to an equivalent real value". Same
// principle applies to arithmetic with mixed real-int operands and to
// conditional branches where one side is real. LRM §3.5.3 specifies that
// x/z bits "shall be treated as zero upon conversion" — `bits_to_biguint`
// already does this, so the conversion runs unconditionally.
//
// `BigInt::to_f64` always returns `Some` (rounding huge magnitudes to ±∞),
// so the unwrap is total.
fn integer_value_to_f64(value: &IntegerValue) -> f64 {
    value
        .as_bigint(value.signed)
        .to_f64()
        .expect("BigInt::to_f64 is total")
}

// LRM §3.5.3: implicit real→integer conversion rounds to nearest with
// ties-away-from-zero. Rust's f64::round implements that exactly. NaN
// and ±∞ have no integer image, surfaced as `None` so callers can apply
// whatever "no integer" handling fits their context (e.g. $itor's chain
// into x→0 below; an integer assignment lvalue would surface 32 bits of x
// the way $rtoi does).
fn real_to_integer_bigint(value: f64) -> Option<BigInt> {
    if value.is_nan() || value.is_infinite() {
        return None;
    }
    let rounded = value.round();
    Some(BigInt::from_f64(rounded).expect("finite f64 rounds to a representable BigInt"))
}

// Reduce a real to its 1-bit logical value for !, &&, ||, and ?: cond.
// Verilog has no defined behavior for NaN, so it folds into x. Zero
// (including -0.0) is logical 0; every other finite value is logical 1.
fn logical_value_of_real(value: f64) -> LogicBit {
    if value.is_nan() {
        LogicBit::X
    } else if value == 0.0 {
        LogicBit::Zero
    } else {
        LogicBit::One
    }
}

// Walks the AST treating every leaf as a real value: integer-typed
// sub-expressions go through the integer pipeline and convert at the
// boundary, real-typed leaves and ops apply f64 directly. Operators
// listed in Table 5-3 (modulus, ===, !==, bitwise, reductions, shifts,
// concatenation, replication, $signed/$unsigned, bitwise NOT) reject
// here because their ancestor was real-typed by `expression_is_real`,
// meaning a real operand reached them; the integer pipeline rejects
// the same operators when the operand is *directly* real.
fn evaluate_expr_as_real(expr: &Expr, session: &Session) -> Result<f64, String> {
    if !expression_is_real(expr, session) {
        return Ok(integer_value_to_f64(&evaluate_expr_in_context(
            expr, None, session,
        )?));
    }

    match expr {
        Expr::Literal(_) => unreachable!("integer literal handled by integer fast-path"),
        Expr::RealLiteral(value) => Ok(*value),
        Expr::Grouped(inner) => evaluate_expr_as_real(inner, session),
        Expr::Unary { op, expr } => {
            let value = evaluate_expr_as_real(expr, session)?;
            match op {
                UnaryOp::Plus => Ok(value),
                UnaryOp::Minus => Ok(-value),
                _ => Err(format!(
                    "operator {} not allowed on real operand",
                    unary_op_name(*op)
                )),
            }
        }
        Expr::Binary { op, lhs, rhs } => {
            let lhs_val = evaluate_expr_as_real(lhs, session)?;
            let rhs_val = evaluate_expr_as_real(rhs, session)?;
            match op {
                BinaryOp::Add => Ok(lhs_val + rhs_val),
                BinaryOp::Subtract => Ok(lhs_val - rhs_val),
                BinaryOp::Multiply => Ok(lhs_val * rhs_val),
                // LRM §5.1.5: "/" on real operands is real division — no
                // truncation, no division-by-zero error, just IEEE 754
                // semantics (returns ±∞ or NaN as appropriate).
                BinaryOp::Divide => Ok(lhs_val / rhs_val),
                // LRM §5.1.5: real ** with the unspecified corners
                // (0**≤0, negative**non-integral) inherits whatever
                // f64::powf returns (1.0 / +∞ / NaN). Documented in the
                // README "Non-standard behavior" section.
                BinaryOp::Power => Ok(lhs_val.powf(rhs_val)),
                _ => Err(format!(
                    "operator {} not allowed on real operand",
                    binary_op_name(*op)
                )),
            }
        }
        Expr::Conditional {
            cond,
            then_expr,
            else_expr,
        } => {
            let cond_logical = logical_value_of_expr(cond, session)?;
            match cond_logical {
                LogicBit::One => evaluate_expr_as_real(then_expr, session),
                LogicBit::Zero => evaluate_expr_as_real(else_expr, session),
                LogicBit::X | LogicBit::Z => {
                    // Real has no per-bit identity to merge; if both
                    // branches numerically agree (including NaN-bit
                    // identity via to_bits), keep the value, otherwise
                    // surface NaN. Mirrors the agree/disagree split the
                    // integer path uses, with the practical caveat that
                    // disagreement in real always collapses to NaN.
                    let then_val = evaluate_expr_as_real(then_expr, session)?;
                    let else_val = evaluate_expr_as_real(else_expr, session)?;
                    if then_val.to_bits() == else_val.to_bits() {
                        Ok(then_val)
                    } else {
                        Ok(f64::NAN)
                    }
                }
            }
        }
        Expr::Concatenation { .. } | Expr::Replication { .. } => {
            unreachable!("concatenation/replication never has real result type")
        }
        Expr::SignCast { .. } => {
            unreachable!("$signed/$unsigned never has real result type")
        }
        Expr::BaseCast { .. } => {
            unreachable!("$bin/$oct/$dec/$hex never has real result type")
        }
        Expr::RealConversion { kind, arg } => match kind {
            RealConversionKind::IntegerToReal => {
                // LRM 17.8: $itor's argument type is `int_val`. The
                // validator (`validate_expr_structure` IntegerToReal arm)
                // rejects a real argument, so only the integer path runs
                // here. When the magnitude exceeds f64 range,
                // `integer_value_to_f64` saturates to ±∞ — that's the
                // value the conversion is supposed to surface.
                if expression_is_real(arg, session) {
                    unreachable!(
                        "validator rejects real $itor arg before evaluation"
                    );
                }
                let int_val = evaluate_expr_in_context(arg, None, session)?;
                Ok(integer_value_to_f64(&int_val))
            }
            RealConversionKind::BitsToReal => {
                // LRM 17.8: reverse of $realtobits. Argument is the 64-bit
                // IEEE 754 bit pattern, so we require an exactly 64-bit
                // self-determined width — narrower operands (e.g. 32-bit
                // unsized literals) and wider ones both get rejected to
                // avoid silent zero-extension or truncation. The
                // "argument cannot be real" rejection lives in the
                // validator (`validate_expr_structure` BitsToReal arm),
                // so a real arg cannot reach here.
                if expression_is_real(arg, session) {
                    unreachable!(
                        "validator rejects real $bitstoreal arg before evaluation"
                    );
                }
                let arg_meta = infer_expr_meta(arg, session)?;
                if arg_meta.width != 64 {
                    return Err(format!(
                        "$bitstoreal argument must be 64 bits wide, got {}",
                        arg_meta.width
                    ));
                }
                let int_val = evaluate_expr_in_context(arg, None, session)?;
                Ok(bits_value_to_real(&int_val))
            }
            RealConversionKind::RealToInteger | RealConversionKind::RealToBits => {
                unreachable!("integer-result conversions handled by integer pipeline")
            }
        },
        // LRM 17.11: real-typed math functions. Each arg evaluates as
        // real (integer args auto-promote via §3.5.3 inside
        // `evaluate_expr_as_real`'s integer-leaf fallback at the top).
        // Rust's `f64::*` methods wrap libm `pow/sin/...`, so the result
        // matches C's standard library exactly. NaN/±∞ propagate through
        // the underlying f64 op — the existing `**` paragraph in README
        // covers the same corner cases.
        Expr::MathFunction { kind, args } => {
            if !kind.is_real_result() {
                unreachable!("integer-result math functions handled by integer pipeline");
            }
            evaluate_real_math_function(*kind, args, session)
        }
        Expr::SystemTask { name } => Err(task_in_expression_error(name)),
        // Only real-typed identifiers reach this arm; integer regs and
        // selects flow through the integer fast-path at the top of the
        // function.
        Expr::Identifier(name) => session
            .lookup(name)
            .and_then(|reg| reg.real())
            .ok_or_else(|| format!("unknown real variable `{name}`")),
        // Real-typed selects are always real-array element selects
        // (`r[i]` where `r` is `real r [0:3]`). The validator rejected
        // non-Bit kinds and any inner select before this point, so
        // `kind` here is always `SelectKind::Bit` and `inner` is None.
        Expr::Select { name, kind, inner } => {
            debug_assert!(inner.is_none(), "validator drops chained selects on real array");
            let index = match kind {
                SelectKind::Bit { index } => index,
                _ => unreachable!("validator rejects part-select on real array"),
            };
            evaluate_real_array_element_select(name, index, session)
        }
    }
}

fn evaluate_real_math_function(
    kind: MathFunctionKind,
    args: &[Expr],
    session: &Session,
) -> Result<f64, String> {
    if kind.arity() == 1 {
        let x = evaluate_expr_as_real(&args[0], session)?;
        return Ok(match kind {
            MathFunctionKind::Ln => x.ln(),
            MathFunctionKind::Log10 => x.log10(),
            MathFunctionKind::Exp => x.exp(),
            MathFunctionKind::Sqrt => x.sqrt(),
            MathFunctionKind::Floor => x.floor(),
            MathFunctionKind::Ceil => x.ceil(),
            MathFunctionKind::Sin => x.sin(),
            MathFunctionKind::Cos => x.cos(),
            MathFunctionKind::Tan => x.tan(),
            MathFunctionKind::Asin => x.asin(),
            MathFunctionKind::Acos => x.acos(),
            MathFunctionKind::Atan => x.atan(),
            MathFunctionKind::Sinh => x.sinh(),
            MathFunctionKind::Cosh => x.cosh(),
            MathFunctionKind::Tanh => x.tanh(),
            MathFunctionKind::Asinh => x.asinh(),
            MathFunctionKind::Acosh => x.acosh(),
            MathFunctionKind::Atanh => x.atanh(),
            MathFunctionKind::Clog2
            | MathFunctionKind::Pow
            | MathFunctionKind::Atan2
            | MathFunctionKind::Hypot => unreachable!("kind handled by other arity branch"),
        });
    }

    let x = evaluate_expr_as_real(&args[0], session)?;
    let y = evaluate_expr_as_real(&args[1], session)?;
    Ok(match kind {
        // LRM 17.11 + README "Real numbers": $pow shares f64::powf with
        // the `**` operator on reals, so corner-case results
        // (0.0**0.0=1.0, negative**non-integral=NaN, 0.0**neg=±∞) match.
        MathFunctionKind::Pow => x.powf(y),
        MathFunctionKind::Atan2 => x.atan2(y),
        MathFunctionKind::Hypot => x.hypot(y),
        _ => unreachable!("kind handled by other arity branch"),
    })
}

// Reduce an arbitrary expression — integer- or real-typed — to its 1-bit
// logical value. Used by ?: cond on both pipelines and by &&/|| operands
// when at least one operand is real.
fn logical_value_of_expr(expr: &Expr, session: &Session) -> Result<LogicBit, String> {
    if expression_is_real(expr, session) {
        Ok(logical_value_of_real(evaluate_expr_as_real(expr, session)?))
    } else {
        Ok(logical_value(&evaluate_expr_in_context(expr, None, session)?))
    }
}

fn validate_select_kind_structure(kind: &SelectKind, session: &Session) -> Result<(), String> {
    match kind {
        SelectKind::Bit { index } => validate_expr_structure(index, session),
        SelectKind::PartConst { msb, lsb } => {
            validate_expr_structure(msb, session)?;
            validate_expr_structure(lsb, session)
        }
        SelectKind::PartIndexedUp { base, width }
        | SelectKind::PartIndexedDown { base, width } => {
            validate_expr_structure(base, session)?;
            validate_expr_structure(width, session)
        }
    }
}

fn validate_select_expr_structure(
    name: &str,
    kind: &SelectKind,
    inner: Option<&SelectKind>,
    session: &Session,
) -> Result<(), String> {
    validate_select_kind_structure(kind, session)?;
    if let Some(inner_kind) = inner {
        validate_select_kind_structure(inner_kind, session)?;
    }
    // `infer_select_meta` routes through `select_meta_width`, which is the
    // shared select-validator used by both the RHS pre-pass and the LHS
    // `lvalue_meta` path. Position-type rules (real-typed bit-select index,
    // real-typed indexed-base, part-select direction match, etc.) live there
    // so the LHS doesn't need its own copy.
    let _ = infer_select_meta(name, kind, inner, session)?;
    Ok(())
}

// Structural validation for a Replication node. The position-sensitive
// zero-count rule (LRM 5.1.14: zero allowed only when the rep is a direct
// operand of a concatenation with at least one positive-size sibling) is
// handed off through `count_check`: top-level calls pass
// `evaluate_replication_count` (strict, rejects zero), and items that sit in
// a concatenation list (either a Concatenation or a Replication's own inner
// list, which is itself a concat list per LRM 5.1.14) pass
// `evaluate_replication_count_allow_zero` (lenient). Everything else — real-
// count / real-operand rejection, recursive structural walks into the count
// and items — applies uniformly.
fn validate_replication_structure(
    count: &Expr,
    items: &[Expr],
    count_check: fn(&Expr, &Session) -> Result<usize, String>,
    session: &Session,
) -> Result<(), String> {
    validate_expr_structure(count, session)?;
    if expression_is_real(count, session) {
        return Err("replication count cannot be real".to_string());
    }
    for item in items {
        validate_concat_list_item(item, "replication", session)?;
    }
    let _ = count_check(count, session)?;
    let _ = collect_concatenation_bits(items, session)?;
    Ok(())
}

// Validate one entry in a concatenation list (the items of `{ ... }` or the
// inner list of `{N{ ... }}`). Replication children get the lenient zero-
// permission rule from LRM 5.1.14; all other expression kinds fall through
// to the generic walker plus a real-position check. `role` distinguishes the
// surrounding form ("concatenation" vs "replication") so the real-operand
// diagnostic names whichever construct the user actually wrote.
fn validate_concat_list_item(
    item: &Expr,
    role: &str,
    session: &Session,
) -> Result<(), String> {
    if let Expr::Replication { count, items } = unwrap_grouped(item) {
        return validate_replication_structure(
            count,
            items,
            evaluate_replication_count_allow_zero,
            session,
        );
    }
    validate_expr_structure(item, session)?;
    if expression_is_real(item, session) {
        return Err(format!("{role} operand cannot be real"));
    }
    Ok(())
}

fn validate_expr_structure(expr: &Expr, session: &Session) -> Result<(), String> {
    match expr {
        Expr::Literal(_) | Expr::RealLiteral(_) => Ok(()),
        Expr::Grouped(inner) => validate_expr_structure(inner, session),
        Expr::Unary { op, expr } => {
            validate_expr_structure(expr, session)?;
            if expression_is_real(expr, session)
                && matches!(
                    op,
                    UnaryOp::BitwiseNot
                        | UnaryOp::ReductionAnd
                        | UnaryOp::ReductionNand
                        | UnaryOp::ReductionOr
                        | UnaryOp::ReductionNor
                        | UnaryOp::ReductionXor
                        | UnaryOp::ReductionXnor
                )
            {
                return Err(format!(
                    "operator {} not allowed on real operand",
                    unary_op_name(*op)
                ));
            }
            Ok(())
        }
        Expr::Binary { op, lhs, rhs } => {
            validate_expr_structure(lhs, session)?;
            validate_expr_structure(rhs, session)?;
            if expression_is_real(lhs, session) || expression_is_real(rhs, session) {
                match op {
                    BinaryOp::Add
                    | BinaryOp::Subtract
                    | BinaryOp::Multiply
                    | BinaryOp::Divide
                    | BinaryOp::Power
                    | BinaryOp::LessThan
                    | BinaryOp::GreaterThan
                    | BinaryOp::LessThanOrEqual
                    | BinaryOp::GreaterThanOrEqual
                    | BinaryOp::Equal
                    | BinaryOp::NotEqual
                    | BinaryOp::LogicalAnd
                    | BinaryOp::LogicalOr => {}
                    _ => {
                        return Err(format!(
                            "operator {} not allowed on real operand",
                            binary_op_name(*op)
                        ));
                    }
                }
            }
            Ok(())
        }
        Expr::Conditional {
            cond,
            then_expr,
            else_expr,
        } => {
            validate_expr_structure(cond, session)?;
            validate_expr_structure(then_expr, session)?;
            validate_expr_structure(else_expr, session)
        }
        Expr::Concatenation { items } => {
            for item in items {
                validate_concat_list_item(item, "concatenation", session)?;
            }
            let _ = collect_concatenation_bits(items, session)?;
            Ok(())
        }
        Expr::Replication { count, items } => validate_replication_structure(
            count,
            items,
            evaluate_replication_count,
            session,
        ),
        Expr::SignCast { signed, arg } => {
            validate_expr_structure(arg, session)?;
            if expression_is_real(arg, session) {
                return Err(format!(
                    "{} argument cannot be real",
                    if *signed { "$signed" } else { "$unsigned" }
                ));
            }
            Ok(())
        }
        Expr::BaseCast { base, arg } => {
            validate_expr_structure(arg, session)?;
            if expression_is_real(arg, session) {
                return Err(format!("{} argument cannot be real", base_cast_name(*base)));
            }
            Ok(())
        }
        Expr::RealConversion { kind, arg } => {
            validate_expr_structure(arg, session)?;
            match kind {
                RealConversionKind::RealToInteger | RealConversionKind::RealToBits => Ok(()),
                RealConversionKind::IntegerToReal => {
                    // LRM 17.8 types the $itor argument as `int_val`, and
                    // simulators diverge on real input (iverilog rounds via
                    // §3.5.3, vcs/xsim pass through unchanged), so accepting
                    // a real argument would silently pick one vendor's
                    // interpretation. Reject up front — the result type is
                    // already real, so a real-typed argument is also
                    // semantically pointless.
                    if expression_is_real(arg, session) {
                        return Err("$itor argument cannot be real".to_string());
                    }
                    Ok(())
                }
                RealConversionKind::BitsToReal => {
                    if expression_is_real(arg, session) {
                        return Err("$bitstoreal argument cannot be real".to_string());
                    }
                    let arg_meta = infer_expr_meta(arg, session)?;
                    if arg_meta.width != 64 {
                        return Err(format!(
                            "$bitstoreal argument must be 64 bits wide, got {}",
                            arg_meta.width
                        ));
                    }
                    Ok(())
                }
            }
        }
        Expr::MathFunction { kind, args } => {
            for arg in args {
                validate_expr_structure(arg, session)?;
            }
            if kind.is_real_result() {
                Ok(())
            } else {
                // LRM 17.11.1 says $clog2's argument "can be an integer or
                // an arbitrary sized vector value" — real is not listed.
                // Mirrors the $itor rejection above: reject up front rather
                // than rely on an implicit §3.5.3 round to integer.
                if expression_is_real(&args[0], session) {
                    return Err(format!("{} argument cannot be real", kind.name()));
                }
                let _ = infer_expr_meta(expr, session)?;
                Ok(())
            }
        }
        Expr::SystemTask { name } => Err(task_in_expression_error(name)),
        Expr::Identifier(name) => {
            let reg = session
                .lookup(name)
                .ok_or_else(|| format!("undeclared identifier: {name}"))?;
            // Real identifiers route through the f64 pipeline, so the
            // vector-only check would wrongly reject them here. Arrays
            // are still rejected because their value-as-a-whole has no
            // numeric type (LRM 4.9 only allows element selects).
            if reg.is_real() {
                Ok(())
            } else {
                let _ = reg.require_vector(name)?;
                Ok(())
            }
        }
        Expr::Select { name, kind, inner } => {
            validate_select_expr_structure(name, kind, inner.as_deref(), session)
        }
    }
}

fn unary_op_name(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Plus => "+",
        UnaryOp::Minus => "-",
        UnaryOp::LogicalNot => "!",
        UnaryOp::BitwiseNot => "~",
        UnaryOp::ReductionAnd => "&",
        UnaryOp::ReductionNand => "~&",
        UnaryOp::ReductionOr => "|",
        UnaryOp::ReductionNor => "~|",
        UnaryOp::ReductionXor => "^",
        UnaryOp::ReductionXnor => "~^",
    }
}

fn binary_op_name(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Subtract => "-",
        BinaryOp::Multiply => "*",
        BinaryOp::Divide => "/",
        BinaryOp::Modulus => "%",
        BinaryOp::Power => "**",
        BinaryOp::LessThan => "<",
        BinaryOp::GreaterThan => ">",
        BinaryOp::LessThanOrEqual => "<=",
        BinaryOp::GreaterThanOrEqual => ">=",
        BinaryOp::Equal => "==",
        BinaryOp::NotEqual => "!=",
        BinaryOp::CaseEqual => "===",
        BinaryOp::CaseNotEqual => "!==",
        BinaryOp::LogicalAnd => "&&",
        BinaryOp::LogicalOr => "||",
        BinaryOp::BitwiseAnd => "&",
        BinaryOp::BitwiseOr => "|",
        BinaryOp::BitwiseXor => "^",
        BinaryOp::BitwiseXnor => "~^",
        BinaryOp::LogicalShiftLeft => "<<",
        BinaryOp::LogicalShiftRight => ">>",
        BinaryOp::ArithmeticShiftLeft => "<<<",
        BinaryOp::ArithmeticShiftRight => ">>>",
    }
}

fn evaluate_expr_in_context(
    expr: &Expr,
    context: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    match expr {
        Expr::Literal(value) => Ok(match context {
            Some(context) => value.resized_to_context(context.width, context.signed),
            None => value.clone(),
        }),
        // Reaching the integer pipeline with a real-typed expression at
        // the top means our dispatch missed a real-result case. Surface
        // an error rather than silently fabricating an integer.
        Expr::RealLiteral(_) => {
            Err("real value cannot be used as an integer expression here".to_string())
        }
        Expr::Grouped(expr) => evaluate_expr_in_context(expr, context, session),
        Expr::Unary { op, expr } => evaluate_unary_expr(*op, expr, context, session),
        Expr::Binary { op, lhs, rhs } => evaluate_binary_expr(*op, lhs, rhs, context, session),
        Expr::Conditional {
            cond,
            then_expr,
            else_expr,
        } => evaluate_conditional_expr(cond, then_expr, else_expr, context, session),
        Expr::Concatenation { items } => evaluate_concatenation_expr(items, context, session),
        Expr::Replication { count, items } => {
            evaluate_replication_expr(count, items, context, session)
        }
        Expr::SignCast { signed, arg } => evaluate_sign_cast_expr(*signed, arg, context, session),
        Expr::BaseCast { base, arg } => evaluate_base_cast_expr(*base, arg, context, session),
        Expr::RealConversion { kind, arg } => {
            evaluate_real_conversion_expr(*kind, arg, context, session)
        }
        Expr::MathFunction { kind, args } => {
            evaluate_math_function_expr(*kind, args, context, session)
        }
        Expr::SystemTask { name } => Err(task_in_expression_error(name)),
        // LRM A.8.3: a primary identifier resolves to its declared reg's
        // current value, then follows the same context-extension path a
        // literal does. An unknown name is the user's first sign that they
        // forgot a `reg` decl, so the error is plain.
        Expr::Identifier(name) => {
            let reg = session
                .lookup(name)
                .ok_or_else(|| format!("undeclared identifier: {name}"))?;
            let value = reg.require_vector(name)?;
            Ok(match context {
                Some(context) => value.resized_to_context(context.width, context.signed),
                None => value.clone(),
            })
        }
        // Bit- / part-select on a declared reg: the slice is computed
        // self-determined (its width and unsigned-ness are fixed by the
        // select form), then widens to the outer context the same way an
        // Identifier does. Per LRM 4.7, the result is always unsigned, so
        // a wider signed outer context zero-extends rather than
        // sign-extends — `resized_to_context(width, context.signed)`
        // already implements that because the value itself is unsigned.
        //
        // Chained selects (`a[i][m:l]`) flow through the same outer-context
        // pipeline: `evaluate_select` resolves both selects and yields the
        // inner result, which is then widened to the propagated context.
        Expr::Select { name, kind, inner } => {
            let value = evaluate_select(name, kind, inner.as_deref(), session)?;
            Ok(match context {
                Some(context) => value.resized_to_context(context.width, context.signed),
                None => value,
            })
        }
    }
}

fn infer_expr_meta(expr: &Expr, session: &Session) -> Result<ExprMeta, String> {
    match expr {
        Expr::Literal(value) => Ok(ExprMeta {
            width: value.width,
            signed: value.signed,
            base: value.base,
        }),
        // Real has no width/sign/base; reaching this branch means an
        // integer-pipeline operator looked at a real-typed sub-expression
        // for context, which the dispatch should have prevented.
        Expr::RealLiteral(_) => {
            Err("real value has no integer width or signedness".to_string())
        }
        Expr::Grouped(expr) => infer_expr_meta(expr, session),
        Expr::Unary { op, expr } => match op {
            UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitwiseNot => infer_expr_meta(expr, session),
            UnaryOp::LogicalNot
            | UnaryOp::ReductionAnd
            | UnaryOp::ReductionNand
            | UnaryOp::ReductionOr
            | UnaryOp::ReductionNor
            | UnaryOp::ReductionXor
            | UnaryOp::ReductionXnor => Ok(ExprMeta {
                width: 1,
                signed: false,
                base: Base::Binary,
            }),
        },
        Expr::Binary { op, lhs, rhs } => {
            let lhs_meta = infer_expr_meta(lhs, session)?;
            let rhs_meta = infer_expr_meta(rhs, session)?;
            Ok(combine_binary_meta(*op, lhs_meta, rhs_meta))
        }
        // LRM 5.1.13: cond is self-determined and contributes nothing to
        // the result meta; then/else are context-determined and unify
        // width (max) and signedness (any unsigned → unsigned, §5.5.1).
        Expr::Conditional {
            cond: _,
            then_expr,
            else_expr,
        } => {
            let then_meta = infer_expr_meta(then_expr, session)?;
            let else_meta = infer_expr_meta(else_expr, session)?;
            Ok(ExprMeta {
                width: usize::max(then_meta.width, else_meta.width),
                signed: then_meta.signed && else_meta.signed,
                base: then_meta.base,
            })
        }
        // LRM 5.1.14: width = sum of operand widths, always unsigned. Base
        // follows leftmost-wins (consistent with arithmetic/bitwise/shift).
        Expr::Concatenation { items } => {
            let mut total_width = 0usize;
            let mut leftmost_base = Base::Binary;
            for (idx, item) in items.iter().enumerate() {
                let item_meta = infer_expr_meta(item, session)?;
                total_width = total_width.saturating_add(item_meta.width);
                if idx == 0 {
                    leftmost_base = item_meta.base;
                }
            }
            Ok(ExprMeta {
                width: total_width,
                signed: false,
                base: leftmost_base,
            })
        }
        // Replication width depends on the constant count value, so we
        // evaluate it eagerly. We use the lenient count helper here — a
        // zero-replication is structurally valid (it just yields width 0)
        // and the per-position constraint is enforced at evaluation time
        // by `evaluate_replication_expr` (top-level) or
        // `collect_concatenation_bits` (the surrounding-list check).
        Expr::Replication { count, items } => {
            let count = evaluate_replication_count_allow_zero(count, session)?;
            let mut inner_width = 0usize;
            let mut leftmost_base = Base::Binary;
            for (idx, item) in items.iter().enumerate() {
                let item_meta = infer_expr_meta(item, session)?;
                inner_width = inner_width.saturating_add(item_meta.width);
                if idx == 0 {
                    leftmost_base = item_meta.base;
                }
            }
            Ok(ExprMeta {
                width: inner_width.saturating_mul(count),
                signed: false,
                base: leftmost_base,
            })
        }
        // LRM 5.5: width and base come from the argument; signedness is
        // whatever the cast specifies. The argument is self-determined inside
        // the cast, but the cast's meta is what context-propagation sees from
        // outside, so it must reflect the cast's signedness.
        Expr::SignCast { signed, arg } => {
            let arg_meta = infer_expr_meta(arg, session)?;
            Ok(ExprMeta {
                width: arg_meta.width,
                signed: *signed,
                base: arg_meta.base,
            })
        }
        // vcal-specific display-base cast: width and signedness come from the
        // argument; only the inferred base flips to the cast's target so the
        // leftmost-base propagation rule sees the cast's base, not the
        // argument's.
        Expr::BaseCast { base, arg } => {
            let arg_meta = infer_expr_meta(arg, session)?;
            Ok(ExprMeta {
                width: arg_meta.width,
                signed: arg_meta.signed,
                base: *base,
            })
        }
        // LRM 17.8: $rtoi yields a 32-bit signed integer; $realtobits
        // yields a 64-bit unsigned vector. The real-result variants
        // ($itor/$bitstoreal) shouldn't reach the integer pipeline at all,
        // so querying their integer meta is a structural surprise.
        Expr::RealConversion { kind, arg: _ } => match kind {
            RealConversionKind::RealToInteger => Ok(ExprMeta {
                width: 32,
                signed: true,
                base: Base::Decimal,
            }),
            RealConversionKind::RealToBits => Ok(ExprMeta {
                width: 64,
                signed: false,
                base: Base::Hex,
            }),
            RealConversionKind::IntegerToReal | RealConversionKind::BitsToReal => {
                Err("real value has no integer width or signedness".to_string())
            }
        },
        // LRM 17.11: $clog2 yields a 32-bit signed integer (mirrors $rtoi).
        // Real-result math functions don't have an integer meta and reaching
        // this branch means the dispatch missed a real-result expression.
        Expr::MathFunction { kind, args: _ } => {
            if kind.is_real_result() {
                Err("real value has no integer width or signedness".to_string())
            } else {
                Ok(ExprMeta {
                    width: 32,
                    signed: true,
                    base: Base::Decimal,
                })
            }
        }
        Expr::SystemTask { name } => Err(task_in_expression_error(name)),
        // A reg's meta is exactly the IntegerValue's stored (width, signed,
        // base) — same shape `Expr::Literal` produces from its value.
        Expr::Identifier(name) => {
            let reg = session
                .lookup(name)
                .ok_or_else(|| format!("undeclared identifier: {name}"))?;
            let value = reg.require_vector(name)?;
            Ok(ExprMeta {
                width: value.width,
                signed: value.signed,
                base: value.base,
            })
        }
        // LRM 4.7 / 5.2.1 / 5.2.2: select width depends on its form
        // (1 for bit-select, |msb-lsb|+1 for constant part-select,
        // the constant `width` for indexed part-selects). The result is
        // always unsigned; the display base flows from the reg's stored
        // base so an arithmetic context still gets a meaningful render.
        //
        // Array-element selects (LRM 4.9) are the exception: `a[i]`
        // returns the whole packed element, so its meta is the element's
        // (width, signed, base) — same shape a vector reg of the same
        // packed range produces from `Expr::Identifier`. Part-selects on
        // the unpacked dimension are illegal and surface the same
        // diagnostic the evaluator does.
        //
        // Chained selects (`a[i][m:l]`) take their width from the inner
        // select form (always unsigned, base inherited from the element
        // — which is always `Binary` at array decl time today).
        // `Expr::Select` arm here mirrors `evaluate_select`'s dispatch so
        // the two-pass context propagation sees the same shape the
        // materialised value will have.
        Expr::Select { name, kind, inner } => infer_select_meta(name, kind, inner.as_deref(), session),
    }
}

fn infer_select_meta(
    name: &str,
    kind: &SelectKind,
    inner: Option<&SelectKind>,
    session: &Session,
) -> Result<ExprMeta, String> {
    let reg = session
        .lookup(name)
        .ok_or_else(|| format!("undeclared identifier: {name}"))?;
    if reg.is_real_array() {
        // Real-array element select: only `r[i]` is legal — part-selects
        // and chained inner selects have no LRM meaning on a real
        // element (no bits to slice). The validator runs through here
        // for structural checks; the actual value path goes through
        // `evaluate_expr_as_real`'s `Expr::Select` arm. The returned
        // meta is a placeholder (width 0) that never reaches a width
        // / sign / base consumer because real-typed selects don't
        // participate in integer context propagation.
        match kind {
            SelectKind::Bit { index } => {
                if expression_is_real(index, session) {
                    return Err("array element index cannot be real".to_string());
                }
            }
            SelectKind::PartConst { .. }
            | SelectKind::PartIndexedUp { .. }
            | SelectKind::PartIndexedDown { .. } => {
                return Err(format!(
                    "part-select on array `{name}` is illegal; use `{name}[i]` to select an element"
                ));
            }
        }
        if inner.is_some() {
            return Err(format!(
                "bit-select or part-select on real-array element `{name}` is illegal"
            ));
        }
        return Ok(ExprMeta {
            width: 0,
            signed: false,
            base: crate::Base::Binary,
        });
    }
    if reg.is_array() {
        let index = match kind {
            SelectKind::Bit { index } => index,
            SelectKind::PartConst { .. }
            | SelectKind::PartIndexedUp { .. }
            | SelectKind::PartIndexedDown { .. } => {
                return Err(format!(
                    "part-select on array `{name}` is illegal; use `{name}[i]` to select an element"
                ));
            }
        };
        if expression_is_real(index, session) {
            return Err("array element index cannot be real".to_string());
        }
        let (_, elements) = reg
            .array()
            .expect("is_array() => array() returns Some");
        let template = &elements[0];
        if let Some(inner_kind) = inner {
            let element_range = reg.range.as_ref().ok_or_else(|| {
                format!("bit-select or part-select on scalar array element `{name}` is illegal")
            })?;
            let width = select_meta_width(inner_kind, element_range, session)?;
            return Ok(ExprMeta {
                width,
                signed: false,
                base: template.base,
            });
        }
        return Ok(ExprMeta {
            width: template.width,
            signed: template.signed,
            base: template.base,
        });
    }
    if reg.is_real() {
        // LRM 4.8.1: "Bit-select or part-select references of variables
        // declared as real … is prohibited." The scalar `real` has no
        // packed bits, so no select kind is meaningful — reject outright
        // regardless of the select shape.
        return Err(format!(
            "bit-select or part-select on real variable `{name}` is not allowed"
        ));
    }
    if inner.is_some() {
        return Err(format!(
            "chained select on `{name}` is illegal: `{name}` is not an array"
        ));
    }
    let value = reg.require_vector(name)?;
    let range = reg
        .range
        .as_ref()
        .ok_or_else(|| format!("bit-select or part-select on scalar reg `{name}` is illegal"))?;
    let width = select_meta_width(kind, range, session)?;
    Ok(ExprMeta {
        width,
        signed: false,
        base: value.base,
    })
}

fn combine_binary_meta(op: BinaryOp, lhs_meta: ExprMeta, rhs_meta: ExprMeta) -> ExprMeta {
    match op {
        BinaryOp::Add
        | BinaryOp::Subtract
        | BinaryOp::Multiply
        | BinaryOp::Divide
        | BinaryOp::Modulus
        | BinaryOp::BitwiseAnd
        | BinaryOp::BitwiseOr
        | BinaryOp::BitwiseXor
        | BinaryOp::BitwiseXnor => ExprMeta {
            width: usize::max(lhs_meta.width, rhs_meta.width),
            signed: lhs_meta.signed && rhs_meta.signed,
            base: lhs_meta.base,
        },
        BinaryOp::Power => ExprMeta {
            width: lhs_meta.width,
            signed: lhs_meta.signed,
            base: lhs_meta.base,
        },
        // LRM 5.1.12: result width and signedness derive from the LHS only;
        // the RHS is self-determined and treated as unsigned, so it cannot
        // widen the result or flip its signedness.
        BinaryOp::LogicalShiftLeft
        | BinaryOp::LogicalShiftRight
        | BinaryOp::ArithmeticShiftLeft
        | BinaryOp::ArithmeticShiftRight => ExprMeta {
            width: lhs_meta.width,
            signed: lhs_meta.signed,
            base: lhs_meta.base,
        },
        BinaryOp::LessThan
        | BinaryOp::GreaterThan
        | BinaryOp::LessThanOrEqual
        | BinaryOp::GreaterThanOrEqual
        | BinaryOp::Equal
        | BinaryOp::NotEqual
        | BinaryOp::CaseEqual
        | BinaryOp::CaseNotEqual
        | BinaryOp::LogicalAnd
        | BinaryOp::LogicalOr => ExprMeta {
            width: 1,
            signed: false,
            base: Base::Binary,
        },
    }
}

fn evaluate_unary_expr(
    op: UnaryOp,
    expr: &Expr,
    context: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    // LRM Table 5-3: bitwise ~ and reductions are illegal on reals.
    // LRM Table 5-2: !, unary +, and unary - are legal on reals; +/- are
    // only reachable here when the *result* type is integer (an
    // arithmetic +/- on a real operand has real result, handled by the
    // real path), so a real operand to + or - here is a structural
    // surprise and we reject it consistently with the operator-name
    // diagnostic shape used elsewhere.
    if expression_is_real(expr, session) {
        match op {
            UnaryOp::LogicalNot => {
                let value = evaluate_expr_as_real(expr, session)?;
                let bit = match logical_value_of_real(value) {
                    LogicBit::One => LogicBit::Zero,
                    LogicBit::Zero => LogicBit::One,
                    LogicBit::X | LogicBit::Z => LogicBit::X,
                };
                return Ok(widen_relational_result(
                    comparison_result_value(bit),
                    context,
                ));
            }
            UnaryOp::BitwiseNot
            | UnaryOp::ReductionAnd
            | UnaryOp::ReductionNand
            | UnaryOp::ReductionOr
            | UnaryOp::ReductionNor
            | UnaryOp::ReductionXor
            | UnaryOp::ReductionXnor => {
                // Per LRM Table 5-3, the validator
                // (`validate_expr_structure` Unary arm) rejects these
                // operators on a real operand before evaluation runs.
                unreachable!(
                    "validator rejects real operand of {} before evaluation",
                    unary_op_name(op)
                );
            }
            UnaryOp::Plus | UnaryOp::Minus => {
                unreachable!("unary +/- on real is handled by the real path")
            }
        }
    }

    if op == UnaryOp::LogicalNot {
        // LRM 5.4: logical operands are self-determined — evaluate without
        // pushing a context down, reduce to the operand's logical value, then
        // apply the !-truth table from §5.1.9.
        let operand = evaluate_expr_in_context(expr, None, session)?;
        let bit = match logical_value(&operand) {
            LogicBit::One => LogicBit::Zero,
            LogicBit::Zero => LogicBit::One,
            LogicBit::X | LogicBit::Z => LogicBit::X,
        };
        return Ok(widen_relational_result(
            comparison_result_value(bit),
            context,
        ));
    }

    if is_reduction_op(op) {
        // LRM 5.1.11: reduction operands are self-determined (LRM Table 5-22)
        // and the result is always 1-bit unsigned. Same outer-context
        // widening shape as `!`/`&&`/`||`/relational/equality.
        let operand = evaluate_expr_in_context(expr, None, session)?;
        let bit = reduce_bits(op, &operand.bits);
        return Ok(widen_relational_result(
            comparison_result_value(bit),
            context,
        ));
    }

    let meta = infer_expr_meta(expr, session)?;
    // LRM 5.5.2: unary +/-/~ is context-determined — propagated size AND
    // signedness must reach the inner primary. Falling back to the operand's
    // own signedness here would sign-extend a signed leaf even when the
    // surrounding comparison/arithmetic unified to unsigned, mis-encoding
    // the value before negation.
    let effective_meta = ExprMeta {
        width: context.map_or(meta.width, |ctx| usize::max(ctx.width, meta.width)),
        signed: context.map_or(meta.signed, |ctx| ctx.signed),
        base: meta.base,
    };
    let operand = evaluate_expr_in_context(expr, Some(effective_meta), session)?;

    if op == UnaryOp::Plus {
        return Ok(operand);
    }

    if op == UnaryOp::BitwiseNot {
        // Per-bit flip: x and z both fold to x; no all-x short-circuit since
        // bitwise ops mix known and unknown bits per position.
        let bits: Vec<LogicBit> = operand.bits.iter().copied().map(bitwise_not_bit).collect();
        return Ok(IntegerValue::computed(
            effective_meta.width,
            effective_meta.signed,
            meta.base,
            bits,
        ));
    }

    if operand.has_unknown_bits() {
        return Ok(IntegerValue::all_x(
            effective_meta.width,
            effective_meta.signed,
            meta.base,
        ));
    }

    // Only UnaryOp::Minus reaches here: Plus is returned above, LogicalNot,
    // BitwiseNot, and the six reductions are all early-returned from their
    // own paths.
    debug_assert!(op == UnaryOp::Minus);
    let result = -operand.as_bigint(effective_meta.signed);

    Ok(IntegerValue::from_bigint(
        result,
        effective_meta.width,
        effective_meta.signed,
        meta.base,
    ))
}

fn evaluate_binary_expr(
    op: BinaryOp,
    lhs: &Expr,
    rhs: &Expr,
    context: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    // LRM Table 5-3: %, ===, !==, bitwise, and shift are all illegal on
    // reals. Arithmetic with a real operand is real-typed and handled by
    // the real path before reaching this evaluator. Relational, equality,
    // and logical ops are 1-bit-integer-typed even with real operands, so
    // they branch into a real-comparison path inside their helpers.
    if expression_is_real(lhs, session) || expression_is_real(rhs, session) {
        match op {
            BinaryOp::Add
            | BinaryOp::Subtract
            | BinaryOp::Multiply
            | BinaryOp::Divide
            | BinaryOp::Power => {
                unreachable!("real arithmetic should be handled by the real path")
            }
            BinaryOp::Modulus
            | BinaryOp::CaseEqual
            | BinaryOp::CaseNotEqual
            | BinaryOp::BitwiseAnd
            | BinaryOp::BitwiseOr
            | BinaryOp::BitwiseXor
            | BinaryOp::BitwiseXnor
            | BinaryOp::LogicalShiftLeft
            | BinaryOp::LogicalShiftRight
            | BinaryOp::ArithmeticShiftLeft
            | BinaryOp::ArithmeticShiftRight => {
                // Per LRM Table 5-3, the validator
                // (`validate_expr_structure` Binary arm) rejects these
                // operators on a real operand before evaluation runs.
                unreachable!(
                    "validator rejects real operand of {} before evaluation",
                    binary_op_name(op)
                );
            }
            BinaryOp::LessThan
            | BinaryOp::GreaterThan
            | BinaryOp::LessThanOrEqual
            | BinaryOp::GreaterThanOrEqual => {
                return evaluate_real_relational_expr(op, lhs, rhs, context, session);
            }
            BinaryOp::Equal | BinaryOp::NotEqual => {
                return evaluate_real_equality_expr(op, lhs, rhs, context, session);
            }
            BinaryOp::LogicalAnd | BinaryOp::LogicalOr => {
                return evaluate_real_logical_expr(op, lhs, rhs, context, session);
            }
        }
    }

    let lhs_meta = infer_expr_meta(lhs, session)?;
    let rhs_meta = infer_expr_meta(rhs, session)?;

    if matches!(
        op,
        BinaryOp::LessThan
            | BinaryOp::GreaterThan
            | BinaryOp::LessThanOrEqual
            | BinaryOp::GreaterThanOrEqual
    ) {
        return evaluate_relational_expr(op, lhs, rhs, lhs_meta, rhs_meta, context, session);
    }

    if matches!(
        op,
        BinaryOp::Equal | BinaryOp::NotEqual | BinaryOp::CaseEqual | BinaryOp::CaseNotEqual
    ) {
        return evaluate_equality_expr(op, lhs, rhs, lhs_meta, rhs_meta, context, session);
    }

    if matches!(op, BinaryOp::LogicalAnd | BinaryOp::LogicalOr) {
        return evaluate_logical_expr(op, lhs, rhs, context, session);
    }

    if matches!(
        op,
        BinaryOp::LogicalShiftLeft
            | BinaryOp::LogicalShiftRight
            | BinaryOp::ArithmeticShiftLeft
            | BinaryOp::ArithmeticShiftRight
    ) {
        return evaluate_shift_expr(op, lhs, rhs, lhs_meta, context, session);
    }

    let meta = combine_binary_meta(op, lhs_meta, rhs_meta);
    let effective_meta = ExprMeta {
        width: context.map_or(meta.width, |ctx| usize::max(ctx.width, meta.width)),
        signed: meta.signed,
        base: meta.base,
    };

    match op {
        BinaryOp::Add
        | BinaryOp::Subtract
        | BinaryOp::Multiply
        | BinaryOp::Divide
        | BinaryOp::Modulus => {
            let lhs_value = evaluate_expr_in_context(lhs, Some(effective_meta), session)?;
            let rhs_value = evaluate_expr_in_context(rhs, Some(effective_meta), session)?;

            if lhs_value.has_unknown_bits() || rhs_value.has_unknown_bits() {
                return Ok(IntegerValue::all_x(
                    effective_meta.width,
                    meta.signed,
                    meta.base,
                ));
            }

            let lhs_int = lhs_value.as_bigint(meta.signed);
            let rhs_int = rhs_value.as_bigint(meta.signed);
            let result = match op {
                BinaryOp::Add => lhs_int + rhs_int,
                BinaryOp::Subtract => lhs_int - rhs_int,
                BinaryOp::Multiply => lhs_int * rhs_int,
                BinaryOp::Divide => {
                    if rhs_int.is_zero() {
                        return Ok(IntegerValue::all_x(
                            effective_meta.width,
                            meta.signed,
                            meta.base,
                        ));
                    }
                    lhs_int / rhs_int
                }
                BinaryOp::Modulus => {
                    if rhs_int.is_zero() {
                        return Ok(IntegerValue::all_x(
                            effective_meta.width,
                            meta.signed,
                            meta.base,
                        ));
                    }
                    lhs_int % rhs_int
                }
                _ => unreachable!("handled by outer match"),
            };

            Ok(IntegerValue::from_bigint(
                result,
                effective_meta.width,
                meta.signed,
                meta.base,
            ))
        }
        BinaryOp::Power => {
            let lhs_context = ExprMeta {
                width: effective_meta.width,
                signed: lhs_meta.signed,
                base: lhs_meta.base,
            };
            let lhs_value = evaluate_expr_in_context(lhs, Some(lhs_context), session)?;
            let rhs_value = evaluate_expr_in_context(rhs, Some(rhs_meta), session)?;

            if lhs_value.has_unknown_bits() || rhs_value.has_unknown_bits() {
                return Ok(IntegerValue::all_x(
                    effective_meta.width,
                    lhs_meta.signed,
                    lhs_meta.base,
                ));
            }

            let base_value = lhs_value.as_bigint(lhs_meta.signed);
            let exponent_value = evaluate_expr_as_math_bigint(rhs, session)?;
            let result = match evaluate_power(base_value, exponent_value) {
                Ok(result) => result,
                Err(_) => {
                    return Ok(IntegerValue::all_x(
                        effective_meta.width,
                        lhs_meta.signed,
                        lhs_meta.base,
                    ));
                }
            };

            Ok(IntegerValue::from_bigint(
                result,
                effective_meta.width,
                lhs_meta.signed,
                lhs_meta.base,
            ))
        }
        BinaryOp::BitwiseAnd | BinaryOp::BitwiseOr | BinaryOp::BitwiseXor | BinaryOp::BitwiseXnor => {
            // Both operands inherit the unified width/sign context, so each
            // side's leaf primary extends consistently before we zip bits.
            let lhs_value = evaluate_expr_in_context(lhs, Some(effective_meta), session)?;
            let rhs_value = evaluate_expr_in_context(rhs, Some(effective_meta), session)?;

            let combine = match op {
                BinaryOp::BitwiseAnd => bitwise_and_bits,
                BinaryOp::BitwiseOr => bitwise_or_bits,
                BinaryOp::BitwiseXor => bitwise_xor_bits,
                BinaryOp::BitwiseXnor => bitwise_xnor_bits,
                _ => unreachable!("guarded by outer match"),
            };

            let bits: Vec<LogicBit> = lhs_value
                .bits
                .iter()
                .zip(rhs_value.bits.iter())
                .map(|(l, r)| combine(*l, *r))
                .collect();

            Ok(IntegerValue::computed(
                effective_meta.width,
                meta.signed,
                meta.base,
                bits,
            ))
        }
        BinaryOp::LessThan
        | BinaryOp::GreaterThan
        | BinaryOp::LessThanOrEqual
        | BinaryOp::GreaterThanOrEqual => {
            unreachable!("relational ops dispatched to evaluate_relational_expr")
        }
        BinaryOp::Equal | BinaryOp::NotEqual | BinaryOp::CaseEqual | BinaryOp::CaseNotEqual => {
            unreachable!("equality ops dispatched to evaluate_equality_expr")
        }
        BinaryOp::LogicalAnd | BinaryOp::LogicalOr => {
            unreachable!("logical ops dispatched to evaluate_logical_expr")
        }
        BinaryOp::LogicalShiftLeft
        | BinaryOp::LogicalShiftRight
        | BinaryOp::ArithmeticShiftLeft
        | BinaryOp::ArithmeticShiftRight => {
            unreachable!("shift ops dispatched to evaluate_shift_expr")
        }
    }
}

// LRM 5.1.12: the LHS is context-determined like arithmetic — its width
// widens to max(L(lhs), L(context)) and the propagated signedness drives
// extension at its leaf primary. The RHS is self-determined (LRM Table 5-22)
// and "always treated as an unsigned number ... has no effect on the
// signedness of the result", so we pass it `None` for the context and read
// its bits as unsigned regardless of the operand's declared signedness.
//
// `>>>` (arithmetic right shift) fills vacated MSB positions with the LHS
// sign bit only when the propagated context is signed. Under an unsigned
// outer context the same operator zero-fills. The other three shift forms
// always zero-fill.
fn evaluate_shift_expr(
    op: BinaryOp,
    lhs: &Expr,
    rhs: &Expr,
    lhs_meta: ExprMeta,
    context: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    let effective_meta = ExprMeta {
        width: context.map_or(lhs_meta.width, |ctx| usize::max(ctx.width, lhs_meta.width)),
        signed: context.map_or(lhs_meta.signed, |ctx| ctx.signed),
        base: lhs_meta.base,
    };

    let lhs_value = evaluate_expr_in_context(lhs, Some(effective_meta), session)?;
    // RHS is self-determined: do NOT push effective_meta; let it evaluate at
    // its own width, then reinterpret its bits as unsigned for the count.
    let rhs_value = evaluate_expr_in_context(rhs, None, session)?;

    if rhs_value.has_unknown_bits() {
        return Ok(IntegerValue::all_x(
            effective_meta.width,
            effective_meta.signed,
            lhs_meta.base,
        ));
    }

    let shift_count = bits_to_biguint(&rhs_value.bits);
    // BigUint shift counts can dwarf usize; clamp to the result width since
    // any larger count produces the same all-fill output.
    let max_shift = BigUint::from(effective_meta.width);
    let clamped_shift = if shift_count >= max_shift {
        effective_meta.width
    } else {
        shift_count
            .to_usize()
            .expect("shift count smaller than width fits in usize")
    };

    let result_bits = match op {
        BinaryOp::LogicalShiftLeft | BinaryOp::ArithmeticShiftLeft => {
            shift_bits_left(&lhs_value.bits, clamped_shift)
        }
        BinaryOp::LogicalShiftRight => {
            shift_bits_right(&lhs_value.bits, clamped_shift, LogicBit::Zero)
        }
        BinaryOp::ArithmeticShiftRight => {
            let fill = if effective_meta.signed {
                lhs_value.bits.last().copied().unwrap_or(LogicBit::Zero)
            } else {
                LogicBit::Zero
            };
            shift_bits_right(&lhs_value.bits, clamped_shift, fill)
        }
        _ => unreachable!("evaluate_shift_expr called with non-shift op"),
    };

    Ok(IntegerValue::computed(
        effective_meta.width,
        effective_meta.signed,
        lhs_meta.base,
        result_bits,
    ))
}

fn shift_bits_left(bits: &[LogicBit], shift: usize) -> Vec<LogicBit> {
    let width = bits.len();
    (0..width)
        .map(|i| {
            if i < shift {
                LogicBit::Zero
            } else {
                bits[i - shift]
            }
        })
        .collect()
}

fn shift_bits_right(bits: &[LogicBit], shift: usize, fill: LogicBit) -> Vec<LogicBit> {
    let width = bits.len();
    (0..width)
        .map(|i| match i.checked_add(shift) {
            Some(src) if src < width => bits[src],
            _ => fill,
        })
        .collect()
}

// LRM 5.5.2: relational/equality operands form a shared context — size =
// max(L(i), L(j)), signed iff both signed. The propagated type drives extension
// at the leaf primary (sign-extend only when propagated type is signed), so the
// unified `comparison_signed` is what each operand sees.
fn unify_comparison_operands(
    lhs: &Expr,
    rhs: &Expr,
    lhs_meta: ExprMeta,
    rhs_meta: ExprMeta,
    session: &Session,
) -> Result<(IntegerValue, IntegerValue, bool), String> {
    let operand_width = usize::max(lhs_meta.width, rhs_meta.width);
    let comparison_signed = lhs_meta.signed && rhs_meta.signed;

    let lhs_context = ExprMeta {
        width: operand_width,
        signed: comparison_signed,
        base: lhs_meta.base,
    };
    let rhs_context = ExprMeta {
        width: operand_width,
        signed: comparison_signed,
        base: rhs_meta.base,
    };

    let lhs_value = evaluate_expr_in_context(lhs, Some(lhs_context), session)?;
    let rhs_value = evaluate_expr_in_context(rhs, Some(rhs_context), session)?;

    Ok((lhs_value, rhs_value, comparison_signed))
}

fn comparison_result_value(bit: LogicBit) -> IntegerValue {
    IntegerValue::computed(1, false, Base::Binary, vec![bit])
}

// LRM 5.1.9: an operand reduces to its logical value before the
// !/&&/|| truth table applies. Any 1 bit makes the operand definitely
// true; all-zero is definitely false; otherwise (any x/z, no 1) the
// operand is ambiguous and reduces to x.
fn logical_value(value: &IntegerValue) -> LogicBit {
    if value.bits.contains(&LogicBit::One) {
        LogicBit::One
    } else if value.bits.iter().all(|bit| *bit == LogicBit::Zero) {
        LogicBit::Zero
    } else {
        LogicBit::X
    }
}

fn evaluate_logical_expr(
    op: BinaryOp,
    lhs: &Expr,
    rhs: &Expr,
    context: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    // LRM 5.4: each operand is self-determined, so we evaluate them in
    // isolation rather than unifying widths the way relational/equality do.
    let lhs_logical = logical_value(&evaluate_expr_in_context(lhs, None, session)?);
    let rhs_logical = logical_value(&evaluate_expr_in_context(rhs, None, session)?);

    // LRM 5.1.9 Table 5-7: a definite false defeats x in &&, a definite true
    // defeats x in ||.
    let bit = match op {
        BinaryOp::LogicalAnd => match (lhs_logical, rhs_logical) {
            (LogicBit::Zero, _) | (_, LogicBit::Zero) => LogicBit::Zero,
            (LogicBit::One, LogicBit::One) => LogicBit::One,
            _ => LogicBit::X,
        },
        BinaryOp::LogicalOr => match (lhs_logical, rhs_logical) {
            (LogicBit::One, _) | (_, LogicBit::One) => LogicBit::One,
            (LogicBit::Zero, LogicBit::Zero) => LogicBit::Zero,
            _ => LogicBit::X,
        },
        _ => unreachable!("non-logical op in evaluate_logical_expr"),
    };

    Ok(widen_relational_result(comparison_result_value(bit), context))
}

// LRM §5.1.7: with at least one real operand, relational comparison runs
// in real space — the integer side is converted via §3.5.3 conversion
// rules (handled by `integer_value_to_f64`). NaN comparisons follow IEEE
// 754: every ordered comparison is false, so e.g. `NaN < x` is `1'b0`.
fn evaluate_real_relational_expr(
    op: BinaryOp,
    lhs: &Expr,
    rhs: &Expr,
    context: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    let lhs_val = evaluate_expr_as_real(lhs, session)?;
    let rhs_val = evaluate_expr_as_real(rhs, session)?;
    let result = match op {
        BinaryOp::LessThan => lhs_val < rhs_val,
        BinaryOp::GreaterThan => lhs_val > rhs_val,
        BinaryOp::LessThanOrEqual => lhs_val <= rhs_val,
        BinaryOp::GreaterThanOrEqual => lhs_val >= rhs_val,
        _ => unreachable!("non-relational op in evaluate_real_relational_expr"),
    };
    let bit = if result { LogicBit::One } else { LogicBit::Zero };
    Ok(widen_relational_result(comparison_result_value(bit), context))
}

// `==` and `!=` on reals follow f64 equality: both NaN-tainted ops are
// false for `==` and true for `!=` (IEEE 754 unordered semantics). LRM
// Table 5-3 forbids `===`/`!==` on reals — those are rejected upstream
// in evaluate_binary_expr.
fn evaluate_real_equality_expr(
    op: BinaryOp,
    lhs: &Expr,
    rhs: &Expr,
    context: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    let lhs_val = evaluate_expr_as_real(lhs, session)?;
    let rhs_val = evaluate_expr_as_real(rhs, session)?;
    let result = match op {
        BinaryOp::Equal => lhs_val == rhs_val,
        BinaryOp::NotEqual => lhs_val != rhs_val,
        _ => unreachable!("non-equality op in evaluate_real_equality_expr"),
    };
    let bit = if result { LogicBit::One } else { LogicBit::Zero };
    Ok(widen_relational_result(comparison_result_value(bit), context))
}

// `&&` and `||` on reals follow the §5.1.9 truth table after each
// operand reduces to a 1-bit logical value via `logical_value_of_expr`.
// NaN reduces to x, so NaN || 1 is 1, NaN && 0 is 0, mirroring how the
// integer path treats unknown bits.
fn evaluate_real_logical_expr(
    op: BinaryOp,
    lhs: &Expr,
    rhs: &Expr,
    context: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    let lhs_logical = logical_value_of_expr(lhs, session)?;
    let rhs_logical = logical_value_of_expr(rhs, session)?;
    let bit = match op {
        BinaryOp::LogicalAnd => match (lhs_logical, rhs_logical) {
            (LogicBit::Zero, _) | (_, LogicBit::Zero) => LogicBit::Zero,
            (LogicBit::One, LogicBit::One) => LogicBit::One,
            _ => LogicBit::X,
        },
        BinaryOp::LogicalOr => match (lhs_logical, rhs_logical) {
            (LogicBit::One, _) | (_, LogicBit::One) => LogicBit::One,
            (LogicBit::Zero, LogicBit::Zero) => LogicBit::Zero,
            _ => LogicBit::X,
        },
        _ => unreachable!("non-logical op in evaluate_real_logical_expr"),
    };
    Ok(widen_relational_result(comparison_result_value(bit), context))
}

fn evaluate_relational_expr(
    op: BinaryOp,
    lhs: &Expr,
    rhs: &Expr,
    lhs_meta: ExprMeta,
    rhs_meta: ExprMeta,
    context: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    let (lhs_value, rhs_value, comparison_signed) =
        unify_comparison_operands(lhs, rhs, lhs_meta, rhs_meta, session)?;

    if lhs_value.has_unknown_bits() || rhs_value.has_unknown_bits() {
        return Ok(widen_relational_result(
            IntegerValue::all_x(1, false, Base::Binary),
            context,
        ));
    }

    let lhs_int = lhs_value.as_bigint(comparison_signed);
    let rhs_int = rhs_value.as_bigint(comparison_signed);

    let comparison_result = match op {
        BinaryOp::LessThan => lhs_int < rhs_int,
        BinaryOp::GreaterThan => lhs_int > rhs_int,
        BinaryOp::LessThanOrEqual => lhs_int <= rhs_int,
        BinaryOp::GreaterThanOrEqual => lhs_int >= rhs_int,
        _ => unreachable!("non-relational op in evaluate_relational_expr"),
    };

    let bit = if comparison_result {
        LogicBit::One
    } else {
        LogicBit::Zero
    };
    Ok(widen_relational_result(comparison_result_value(bit), context))
}

fn evaluate_equality_expr(
    op: BinaryOp,
    lhs: &Expr,
    rhs: &Expr,
    lhs_meta: ExprMeta,
    rhs_meta: ExprMeta,
    context: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    // Bit-level comparison; the unified signedness only matters for operand
    // extension (already done inside `unify_comparison_operands`), not for the
    // comparison itself.
    let (lhs_value, rhs_value, _comparison_signed) =
        unify_comparison_operands(lhs, rhs, lhs_meta, rhs_meta, session)?;

    let bit = match op {
        // LRM 5.1.8: ==/!= are 1-bit x only when the relation is *ambiguous*.
        // A single definite bit mismatch (0 vs 1) makes the operands unequal
        // regardless of any x/z elsewhere; only when no bit definitively
        // mismatches AND at least one bit involves x or z is the result x.
        BinaryOp::Equal | BinaryOp::NotEqual => {
            let mut definite_mismatch = false;
            let mut has_unknown = false;
            for (lb, rb) in lhs_value.bits.iter().zip(rhs_value.bits.iter()) {
                match (lb, rb) {
                    (LogicBit::Zero, LogicBit::One) | (LogicBit::One, LogicBit::Zero) => {
                        definite_mismatch = true;
                    }
                    (LogicBit::X | LogicBit::Z, _) | (_, LogicBit::X | LogicBit::Z) => {
                        has_unknown = true;
                    }
                    _ => {}
                }
            }
            if !definite_mismatch && has_unknown {
                return Ok(widen_relational_result(
                    IntegerValue::all_x(1, false, Base::Binary),
                    context,
                ));
            }
            let equal = !definite_mismatch;
            let result = if matches!(op, BinaryOp::Equal) {
                equal
            } else {
                !equal
            };
            if result { LogicBit::One } else { LogicBit::Zero }
        }
        // LRM 5.1.8: ===/!== compare bit-for-bit including x and z; the result
        // is always a known 0 or 1, never x. Operands are already the same
        // length after unification.
        BinaryOp::CaseEqual | BinaryOp::CaseNotEqual => {
            let equal = lhs_value.bits == rhs_value.bits;
            let result = if matches!(op, BinaryOp::CaseEqual) {
                equal
            } else {
                !equal
            };
            if result { LogicBit::One } else { LogicBit::Zero }
        }
        _ => unreachable!("non-equality op in evaluate_equality_expr"),
    };

    Ok(widen_relational_result(comparison_result_value(bit), context))
}

// LRM 5.1.13: cond is self-determined and reduced to a 1-bit logical the
// way `&&`/`||`/`!` reduce their operands. then/else are context-determined
// and unify width/sign with each other AND with the propagated outer
// context. As with the shift path, signedness must consult the propagated
// context — if the surrounding expression is unsigned (§5.5.1), the leaf
// primaries of then/else must zero-fill rather than sign-fill.
//
// When cond is x or z, evaluate both branches and merge per bit: agreeing
// bits stay, disagreeing bits become x. This preserves x/z agreement
// (e.g. x ∩ x → x) and reduces any disagreement (including 0 vs x) to x.
fn evaluate_conditional_expr(
    cond: &Expr,
    then_expr: &Expr,
    else_expr: &Expr,
    context: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    if expression_is_real(then_expr, session) || expression_is_real(else_expr, session) {
        unreachable!("real-typed conditional should be handled by the real path")
    }
    let then_meta = infer_expr_meta(then_expr, session)?;
    let else_meta = infer_expr_meta(else_expr, session)?;
    let meta = ExprMeta {
        width: usize::max(then_meta.width, else_meta.width),
        signed: then_meta.signed && else_meta.signed,
        base: then_meta.base,
    };
    let effective_meta = ExprMeta {
        width: context.map_or(meta.width, |ctx| usize::max(ctx.width, meta.width)),
        signed: context.map_or(meta.signed, |ctx| ctx.signed),
        base: meta.base,
    };

    let cond_logical = logical_value_of_expr(cond, session)?;

    let bits = match cond_logical {
        LogicBit::One => evaluate_expr_in_context(then_expr, Some(effective_meta), session)?.bits,
        LogicBit::Zero => evaluate_expr_in_context(else_expr, Some(effective_meta), session)?.bits,
        LogicBit::X | LogicBit::Z => {
            let then_value = evaluate_expr_in_context(then_expr, Some(effective_meta), session)?;
            let else_value = evaluate_expr_in_context(else_expr, Some(effective_meta), session)?;
            then_value
                .bits
                .iter()
                .zip(else_value.bits.iter())
                .map(|(t, e)| if t == e { *t } else { LogicBit::X })
                .collect()
        }
    };

    Ok(IntegerValue::computed(
        effective_meta.width,
        effective_meta.signed,
        meta.base,
        bits,
    ))
}

// LRM 5.1.14: every concatenation operand "shall be sized" — an operand
// with indefinite width (i.e. one whose self-determined width comes from an
// unsized literal) is rejected. The flag propagates through context-determined
// operators that take width from their operands (arithmetic/bitwise/power,
// shift LHS, conditional branches, unary +/-/~), but stops at any operator
// with a definite 1-bit result (relational/equality/logical/reduction) and at
// concatenation/replication themselves (their result widths are summed/
// multiplied integers, never indefinite). E.g. `{4'd1 + 1, 4'd2}` is rejected
// because the unsized `1` has indefinite width.
fn is_indefinite_width(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(value) => value.unsized_literal,
        // Real values are always rejected from concatenation by
        // `evaluate_concatenation_expr` with a clearer message; mark
        // them as indefinite-width here so any reachable check still
        // refuses them.
        Expr::RealLiteral(_) => true,
        Expr::Grouped(inner) => is_indefinite_width(inner),
        Expr::Unary { op, expr } => match op {
            UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitwiseNot => is_indefinite_width(expr),
            UnaryOp::LogicalNot
            | UnaryOp::ReductionAnd
            | UnaryOp::ReductionNand
            | UnaryOp::ReductionOr
            | UnaryOp::ReductionNor
            | UnaryOp::ReductionXor
            | UnaryOp::ReductionXnor => false,
        },
        Expr::Binary { op, lhs, rhs } => match op {
            BinaryOp::Add
            | BinaryOp::Subtract
            | BinaryOp::Multiply
            | BinaryOp::Divide
            | BinaryOp::Modulus
            | BinaryOp::BitwiseAnd
            | BinaryOp::BitwiseOr
            | BinaryOp::BitwiseXor
            | BinaryOp::BitwiseXnor => is_indefinite_width(lhs) || is_indefinite_width(rhs),
            BinaryOp::Power => is_indefinite_width(lhs),
            BinaryOp::LogicalShiftLeft
            | BinaryOp::LogicalShiftRight
            | BinaryOp::ArithmeticShiftLeft
            | BinaryOp::ArithmeticShiftRight => is_indefinite_width(lhs),
            BinaryOp::LessThan
            | BinaryOp::GreaterThan
            | BinaryOp::LessThanOrEqual
            | BinaryOp::GreaterThanOrEqual
            | BinaryOp::Equal
            | BinaryOp::NotEqual
            | BinaryOp::CaseEqual
            | BinaryOp::CaseNotEqual
            | BinaryOp::LogicalAnd
            | BinaryOp::LogicalOr => false,
        },
        Expr::Conditional {
            cond: _,
            then_expr,
            else_expr,
        } => is_indefinite_width(then_expr) || is_indefinite_width(else_expr),
        Expr::Concatenation { .. } | Expr::Replication { .. } => false,
        // `$signed`/`$unsigned` lock in the argument's evaluated width, so
        // the cast result is always sized regardless of the argument shape.
        Expr::SignCast { .. } => false,
        // `$bin`/`$oct`/`$dec`/`$hex` lock in width the same way — they only
        // change the display base, never the bit count.
        Expr::BaseCast { .. } => false,
        // $rtoi / $realtobits have fixed result widths (32 / 64); the
        // real-result conversions never reach width-sensitive paths.
        Expr::RealConversion { .. } => false,
        // $clog2 is fixed 32-bit; real-result math functions never reach
        // width-sensitive paths the way real conversions don't.
        Expr::MathFunction { .. } => false,
        // Reporting "definite width" lets the surrounding evaluator path
        // surface the precise task-in-expression error rather than the
        // generic "indefinite width" diagnostic from the concatenation
        // pre-check.
        Expr::SystemTask { .. } => false,
        // A reg always has an explicit declared width, so an identifier is
        // never indefinite. The session lookup happens later in the
        // evaluator pipeline; the structural check here only needs the
        // type-shape answer.
        Expr::Identifier(_) => false,
        // Bit-/part-select width is fixed by its form (1 for bit-select,
        // |m-l|+1 for constant part-select, the constant `width` for the
        // indexed forms), so the result width is always definite.
        Expr::Select { .. } => false,
    }
}

// Replication count must be a constant, non-negative, non-x, non-z value
// (LRM 5.1.14). `to_usize` doubles as the "fits in addressable space" check;
// vcal uses `usize` for widths, so an oversized count surfaces as a clean
// error rather than overflowing. Zero is allowed at parse-meta level — the
// position-sensitive rule (zero is valid only inside a concatenation whose
// other operands sum to positive width) is enforced separately in
// `evaluate_replication_count` (top-level) and `collect_concatenation_bits`
// (the surrounding-list check).
fn evaluate_replication_count_allow_zero(
    count_expr: &Expr,
    session: &Session,
) -> Result<usize, String> {
    let value = evaluate_expr_in_context(count_expr, None, session)?;
    if value.has_unknown_bits() {
        return Err("replication count contains unknown bits".to_string());
    }
    let count = value.as_bigint(value.signed);
    if count.sign() == Sign::Minus {
        return Err("replication count must be non-negative".to_string());
    }
    count
        .to_usize()
        .ok_or_else(|| "replication count too large".to_string())
}

// Strict variant: a top-level replication (one whose result is the whole
// expression, or whose only consumers are non-concatenation operators) needs
// a positive count, since it would otherwise produce a zero-width
// `IntegerValue` in a position where vcal can't represent it.
fn evaluate_replication_count(count_expr: &Expr, session: &Session) -> Result<usize, String> {
    let count = evaluate_replication_count_allow_zero(count_expr, session)?;
    if count == 0 {
        return Err("replication count must be positive in this context".to_string());
    }
    Ok(count)
}

// Walk through `Grouped` wrappers without evaluating. Used so that
// `({0{1'b1}})` is treated the same as `{0{1'b1}}` when the parent is
// looking for a Replication child to allow zero replication on.
fn unwrap_grouped(expr: &Expr) -> &Expr {
    match expr {
        Expr::Grouped(inner) => unwrap_grouped(inner),
        other => other,
    }
}

// LRM 5.1.14: each operand is self-determined (no context propagated down)
// and must have a definite width — bare unsized literals (and any expression
// whose width derives from one) are rejected. Bits are joined MSB-first: the
// leftmost item ends up in the high bits of the result. Result is always
// unsigned; outer context can only widen the joined value (zero-extending),
// never reach into the operands.
fn evaluate_concatenation_expr(
    items: &[Expr],
    context: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    // LRM Table 5-3 rejection of real items is handled by `semantic_check`
    // before evaluation begins.
    let bits = collect_concatenation_bits(items, session)?;
    let leftmost_base = infer_expr_meta(&items[0], session)?.base;
    let natural_width = bits.len();
    let result = IntegerValue::computed(natural_width, false, leftmost_base, bits);
    Ok(extend_to_outer_context(result, context))
}

fn evaluate_replication_expr(
    count_expr: &Expr,
    items: &[Expr],
    context: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    // Real-count and real-item rejection is handled by `semantic_check`
    // before evaluation begins.
    let count = evaluate_replication_count(count_expr, session)?;
    let inner_bits = collect_concatenation_bits(items, session)?;
    let leftmost_base = infer_expr_meta(&items[0], session)?.base;

    let mut bits = Vec::with_capacity(inner_bits.len().saturating_mul(count));
    for _ in 0..count {
        bits.extend(inner_bits.iter().copied());
    }
    let natural_width = bits.len();
    let result = IntegerValue::computed(natural_width, false, leftmost_base, bits);
    Ok(extend_to_outer_context(result, context))
}

// Joins the bit patterns of every item in a concatenation list (used both
// for plain `{a, b, ...}` and for the inner list of `{N{a, b, ...}}`).
//
// LRM 5.1.14 lets a replication's count be zero when it sits directly inside
// a concatenation — the zero-rep contributes no bits, but the surrounding
// list must still have at least one operand of positive size. So we
// special-case Replication items here (looking through `Grouped`) to permit
// a zero count, then verify the joined width is non-zero. This rejects
// `{ {0{1'b1}} }` and `{N{ {0{1'b1}} }}` (no positive-size sibling) while
// accepting `{ {0{1'b1}}, 1'b1 }` and `{N{ {0{1'b1}}, 1'b1 }}`.
fn collect_concatenation_bits(items: &[Expr], session: &Session) -> Result<Vec<LogicBit>, String> {
    if items.is_empty() {
        return Err("concatenation requires at least one operand".to_string());
    }
    for item in items {
        if is_indefinite_width(item) {
            return Err("concatenation operand has indefinite width".to_string());
        }
    }
    // Items are in source order (leftmost first → MSB-side). Our bit vectors
    // are LSB-first, so we feed bits starting from the rightmost item.
    let mut bits = Vec::new();
    for item in items.iter().rev() {
        bits.extend(evaluate_concatenation_item_bits(item, session)?);
    }
    if bits.is_empty() {
        // Every operand collapsed to zero width — the concatenation has no
        // positive-size operand, which is the case LRM 5.1.14 forbids.
        return Err(
            "concatenation must have at least one operand with positive size".to_string(),
        );
    }
    Ok(bits)
}

fn evaluate_concatenation_item_bits(
    item: &Expr,
    session: &Session,
) -> Result<Vec<LogicBit>, String> {
    if let Expr::Replication { count, items } = unwrap_grouped(item) {
        let count = evaluate_replication_count_allow_zero(count, session)?;
        if count == 0 {
            return Ok(Vec::new());
        }
        let inner_bits = collect_concatenation_bits(items, session)?;
        let mut bits = Vec::with_capacity(inner_bits.len().saturating_mul(count));
        for _ in 0..count {
            bits.extend(inner_bits.iter().copied());
        }
        return Ok(bits);
    }
    let value = evaluate_expr_in_context(item, None, session)?;
    Ok(value.bits)
}

// Concatenation/replication results are unsigned; if an outer context is
// wider, zero-extend (reusing the existing §5.5.4 path via
// `resized_to_context` with context_signed = false). If the outer context is
// narrower or absent we keep the natural width — concatenation is
// self-determined, so the joined width never shrinks below itself.
fn extend_to_outer_context(value: IntegerValue, context: Option<ExprMeta>) -> IntegerValue {
    match context {
        Some(ctx) if ctx.width > value.width => value.resized_to_context(ctx.width, false),
        _ => value,
    }
}

// Outer-context widening for cast / conversion results ($signed, $unsigned,
// $rtoi, $realtobits, $clog2). Differs from `extend_to_outer_context` in that
// extension follows the *propagated* context's signedness (§5.5.2) rather than
// forcing zero-extension — the cast's own type already lives in the result.
fn extend_cast_to_outer_context(value: IntegerValue, context: Option<ExprMeta>) -> IntegerValue {
    match context {
        Some(ctx) if ctx.width > value.width => value.resized_to_context(ctx.width, ctx.signed),
        _ => value,
    }
}

// LRM 5.5: `$signed(e)` / `$unsigned(e)` evaluates `e` as a self-determined
// expression and returns a value with the same size and bit pattern but with
// the type set by the cast. The cast's type only feeds back into LRM 5.5.1's
// "all operands signed?" check (which determines the propagated type of the
// surrounding expression); extension at the cast leaf still follows the
// propagated context per §5.5.2 ("each operand shall be sign-/zero-extended"
// based on the propagated type), not the cast's own signedness. So
// `$signed(4'b1111) + 8'b0` zero-extends the cast result (unsigned propagated
// type) rather than sign-extending it.
fn evaluate_sign_cast_expr(
    signed: bool,
    arg: &Expr,
    context: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    // $signed/$unsigned are integer-only — applying them to a real value
    // has no meaning under §5.5 (signedness is a property of the integer
    // value set, not the floating-point one). The validator
    // (`validate_expr_structure` SignCast arm) rejects a real arg before
    // evaluation, so a real arg cannot reach here.
    if expression_is_real(arg, session) {
        unreachable!(
            "validator rejects real {} arg before evaluation",
            if signed { "$signed" } else { "$unsigned" }
        );
    }
    let arg_value = evaluate_expr_in_context(arg, None, session)?;
    let cast_value = IntegerValue::computed(
        arg_value.width,
        signed,
        arg_value.base,
        arg_value.bits,
    );
    Ok(extend_cast_to_outer_context(cast_value, context))
}

// vcal-specific display-base cast (`$bin` / `$oct` / `$dec` / `$hex`).
// Mirrors `evaluate_sign_cast_expr`: argument is evaluated self-determined,
// width/signedness/bits pass through unchanged, only the `Base` field is
// overridden. Outer-context width still flows back through the cast result
// per §5.5.2 via `extend_cast_to_outer_context` — the same shape the sign
// casts use, so chained casts and arithmetic on the cast result behave
// consistently. Real arguments are rejected (a real has no display base).
fn evaluate_base_cast_expr(
    base: Base,
    arg: &Expr,
    context: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    // The validator (`validate_expr_structure` BaseCast arm) rejects a
    // real arg before evaluation, so a real arg cannot reach here.
    if expression_is_real(arg, session) {
        unreachable!(
            "validator rejects real {} arg before evaluation",
            base_cast_name(base)
        );
    }
    let arg_value = evaluate_expr_in_context(arg, None, session)?;
    let cast_value = IntegerValue::computed(
        arg_value.width,
        arg_value.signed,
        base,
        arg_value.bits,
    );
    Ok(extend_cast_to_outer_context(cast_value, context))
}

fn base_cast_name(base: Base) -> &'static str {
    match base {
        Base::Binary => "$bin",
        Base::Octal => "$oct",
        Base::Decimal => "$dec",
        Base::Hex => "$hex",
    }
}

// LRM 17.8: dispatch the integer-result real conversions ($rtoi and
// $realtobits). The real-result variants ($itor, $bitstoreal) are handled
// by `evaluate_expr_as_real`. Outer-context widening mirrors $signed /
// $unsigned: the cast's natural width drives the result, but a wider
// propagated context extends per its own signedness (§5.5.2).
fn evaluate_real_conversion_expr(
    kind: RealConversionKind,
    arg: &Expr,
    context: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    let result = match kind {
        RealConversionKind::RealToInteger => {
            // LRM 17.8: "$rtoi converts real values to integers by
            // truncating the real value." Argument is real (or auto-promotes
            // from integer per §3.5.3). NaN / ±∞ have no integer image, so
            // we return 32 bits of x to surface "no defined integer";
            // out-of-range finite values wrap mod 2^32, consistent with the
            // rest of the integer pipeline's overflow handling.
            let real_val = evaluate_expr_as_real(arg, session)?;
            real_to_integer_value(real_val)
        }
        RealConversionKind::RealToBits => {
            // LRM 17.8: bitcast a real to its 64-bit IEEE 754
            // representation. Display the result in hex since the value is a
            // bit pattern, not a magnitude.
            let real_val = evaluate_expr_as_real(arg, session)?;
            let bits = real_val.to_bits();
            IntegerValue::from_bigint(BigInt::from(bits), 64, false, Base::Hex)
        }
        RealConversionKind::IntegerToReal | RealConversionKind::BitsToReal => {
            unreachable!("real-result conversions handled by evaluate_expr_as_real")
        }
    };

    Ok(extend_cast_to_outer_context(result, context))
}

fn real_to_integer_value(value: f64) -> IntegerValue {
    if value.is_nan() || value.is_infinite() {
        return IntegerValue::all_x(32, true, Base::Decimal);
    }
    let truncated = value.trunc();
    let bigint = BigInt::from_f64(truncated)
        .expect("finite f64 truncates to a representable BigInt");
    IntegerValue::from_bigint(bigint, 32, true, Base::Decimal)
}

// LRM 17.11: dispatch the integer-result math functions. Today only
// $clog2 lands here; real-result kinds are handled by
// `evaluate_expr_as_real`. Outer-context widening mirrors
// `evaluate_real_conversion_expr` for $rtoi / $realtobits.
fn evaluate_math_function_expr(
    kind: MathFunctionKind,
    args: &[Expr],
    context: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    let result = match kind {
        MathFunctionKind::Clog2 => evaluate_clog2(&args[0], session)?,
        _ => unreachable!("real-result math functions handled by evaluate_expr_as_real"),
    };

    Ok(extend_cast_to_outer_context(result, context))
}

// LRM 17.11.1: $clog2 returns the ceiling of log base 2 of the unsigned
// argument; $clog2(0) is defined to be 0. The argument is integer or
// vector — real arguments are rejected by the validator
// (`validate_expr_structure` MathFunction arm).
//
// Bits with x/z anywhere collapse the result to 32'sdx. The LRM is silent
// on x/z in $clog2; vcal surfaces "no defined image" rather than silently
// mapping x/z → 0, matching the $rtoi NaN/±∞ rule. The width used for the
// unsigned interpretation is the operand's natural width, so
// $clog2(64'hFFFF…F) is 64.
fn evaluate_clog2(arg: &Expr, session: &Session) -> Result<IntegerValue, String> {
    if expression_is_real(arg, session) {
        unreachable!("validator rejects real $clog2 arg before evaluation");
    }

    let value = evaluate_expr_in_context(arg, None, session)?;
    if value.has_unknown_bits() {
        return Ok(IntegerValue::all_x(32, true, Base::Decimal));
    }
    let unsigned = bits_to_biguint(&value.bits);
    Ok(clog2_result_value(unsigned))
}

fn clog2_result_value(value: BigUint) -> IntegerValue {
    let result = if value.is_zero() {
        BigUint::zero()
    } else {
        BigUint::from((value - BigUint::one()).bits())
    };
    IntegerValue::from_bigint(BigInt::from(result), 32, true, Base::Decimal)
}

// LRM 17.8: $bitstoreal reinterprets a 64-bit operand as an IEEE 754
// double. Width is enforced to be exactly 64 by the caller, so the loop
// just packs the 64 LogicBits into a u64. x/z bits map to 0, mirroring
// §3.5.3's integer-to-real conversion rule — `$bitstoreal` is a sibling
// conversion in the same clause and the LRM doesn't carve out a different
// rule for it.
//
// Must stay a pure u64-pack-then-`f64::from_bits` transmute: any NaN
// canonicalization here would break the
// `$realtobits($bitstoreal(x)) == x` round-trip on non-finite payloads
// (matching iverilog), which the test suite pins down.
fn bits_value_to_real(value: &IntegerValue) -> f64 {
    let mut bits = 0u64;
    for (index, bit) in value.bits.iter().enumerate() {
        if *bit == LogicBit::One {
            bits |= 1u64 << index;
        }
    }
    f64::from_bits(bits)
}

fn widen_relational_result(result: IntegerValue, context: Option<ExprMeta>) -> IntegerValue {
    match context {
        Some(ctx) if ctx.width > 1 => result.resized_to_context(ctx.width, false),
        _ => result,
    }
}

// Used for the RHS of integer `**`, where the exponent must keep
// arbitrary precision rather than be clamped to the result width. Two
// strategies:
//   - Arithmetic operators (+, -, *, /, %, **) and unary +/- recurse in
//     bigint, so the computation stays width-free.
//   - Everything else has a width- or signedness-dependent result that
//     can't be reconstructed from a raw bigint, so we route through the
//     standard pipeline (which materialises width/signedness on the
//     `IntegerValue`), then read the bigint out via `value_to_math_bigint`.
// `value_to_math_bigint` also rejects x/z bits — at the math-bigint layer
// we have no way to represent unknown bits, so any unknown surfaces a
// clean "expression contains unknown bits" error.
fn evaluate_expr_as_math_bigint(expr: &Expr, session: &Session) -> Result<BigInt, String> {
    match expr {
        Expr::Literal(value) => {
            if value.has_unknown_bits() {
                return Err("expression contains unknown bits".to_string());
            }

            Ok(value.as_bigint(value.signed))
        }
        // Reaching this helper with a real-typed expression means the
        // integer-power exponent path was entered with a real exponent,
        // which expression_is_real would have caught earlier. Surface a
        // clear error rather than fabricating an integer.
        Expr::RealLiteral(_) => Err("real value cannot be used as an integer here".to_string()),
        Expr::Grouped(expr) => evaluate_expr_as_math_bigint(expr, session),
        // Unary +/- recurse so the inner exponent computation stays in
        // arbitrary precision. Every other unary op has a width-dependent
        // result — bitwise NOT preserves the operand's width, logical NOT
        // and reductions yield 1-bit unsigned — so route them through
        // evaluate_unary_expr and read the materialised value out.
        Expr::Unary { op, expr } => match op {
            UnaryOp::Plus => evaluate_expr_as_math_bigint(expr, session),
            UnaryOp::Minus => Ok(-evaluate_expr_as_math_bigint(expr, session)?),
            UnaryOp::LogicalNot
            | UnaryOp::BitwiseNot
            | UnaryOp::ReductionAnd
            | UnaryOp::ReductionNand
            | UnaryOp::ReductionOr
            | UnaryOp::ReductionNor
            | UnaryOp::ReductionXor
            | UnaryOp::ReductionXnor => {
                value_to_math_bigint(evaluate_unary_expr(*op, expr, None, session)?)
            }
        },
        Expr::Binary { op, lhs, rhs } => {
            // Non-arithmetic binaries (relational, equality, logical,
            // bitwise, shift) all have results we can't reconstruct in
            // bigint alone: bitwise and shift depend on the unified
            // operand width, the rest are 1-bit unsigned by construction.
            // evaluate_binary_expr itself dispatches to the
            // relational/equality/logical/shift helpers, so a single call
            // covers all five cases.
            if !matches!(
                op,
                BinaryOp::Add
                    | BinaryOp::Subtract
                    | BinaryOp::Multiply
                    | BinaryOp::Divide
                    | BinaryOp::Modulus
                    | BinaryOp::Power
            ) {
                return value_to_math_bigint(evaluate_binary_expr(*op, lhs, rhs, None, session)?);
            }

            let lhs_value = evaluate_expr_as_math_bigint(lhs, session)?;
            let rhs_value = evaluate_expr_as_math_bigint(rhs, session)?;

            match op {
                BinaryOp::Add => Ok(lhs_value + rhs_value),
                BinaryOp::Subtract => Ok(lhs_value - rhs_value),
                BinaryOp::Multiply => Ok(lhs_value * rhs_value),
                BinaryOp::Divide => {
                    if rhs_value.is_zero() {
                        Err("expression division by zero".to_string())
                    } else {
                        Ok(lhs_value / rhs_value)
                    }
                }
                BinaryOp::Modulus => {
                    if rhs_value.is_zero() {
                        Err("expression modulus by zero".to_string())
                    } else {
                        Ok(lhs_value % rhs_value)
                    }
                }
                BinaryOp::Power => evaluate_power(lhs_value, rhs_value),
                _ => unreachable!("non-arithmetic ops handled by the early return above"),
            }
        }
        // Conditional, concatenation/replication, sign casts, real
        // conversions, and math functions all have width- and
        // signedness-dependent results that can't be reconstructed from a
        // raw bigint: the conditional merges per bit under an x/z cond,
        // concat/replication is fixed unsigned width (LRM 5.1.14: "the
        // result of a concatenation is treated as an unsigned vector"),
        // sign casts lock in signedness, and $rtoi / $realtobits / $clog2
        // have pinned widths (32 signed / 64 unsigned / 32 signed). Route
        // through the standard pipeline so those properties are
        // materialised before the bigint read.
        Expr::Conditional { .. }
        | Expr::Concatenation { .. }
        | Expr::Replication { .. }
        | Expr::SignCast { .. }
        | Expr::BaseCast { .. }
        | Expr::RealConversion { .. }
        | Expr::MathFunction { .. }
        // A reg has fixed width/signedness, so reading it through the
        // standard pipeline and then converting to bigint matches the
        // shape used by every other width-dependent leaf above. A select
        // is the same shape — fixed width determined by its form, always
        // unsigned per LRM 4.7.
        | Expr::Identifier(_)
        | Expr::Select { .. } => {
            value_to_math_bigint(evaluate_expr_in_context(expr, None, session)?)
        }
        Expr::SystemTask { name } => Err(task_in_expression_error(name)),
    }
}

// Reject x/z (math-bigint has no representation for unknown bits) and
// convert the materialised integer to BigInt using the value's own
// signedness flag. Note: concat/replication and the 1-bit
// relational/equality/logical/reduction results all carry signed = false
// by construction, so `value.signed` produces the same bigint as an
// explicit `false` would; passing the flag through keeps the rule
// uniform for the callers that do preserve signedness (bitwise, shift,
// sign casts, $rtoi, etc.).
fn value_to_math_bigint(value: IntegerValue) -> Result<BigInt, String> {
    if value.has_unknown_bits() {
        return Err("expression contains unknown bits".to_string());
    }
    Ok(value.as_bigint(value.signed))
}

fn evaluate_power(base: BigInt, exponent: BigInt) -> Result<BigInt, String> {
    if exponent.is_zero() {
        return Ok(BigInt::one());
    }

    if exponent.sign() == Sign::Minus {
        if base.is_zero() {
            return Err("power result is undefined".to_string());
        }

        if base == BigInt::one() {
            return Ok(BigInt::one());
        }

        if base == BigInt::from(-1) {
            // Parity is sign-invariant under num-bigint's two's-complement
            // BitAnd (e.g. -3 & 1 == 3 & 1 == 1), so we read the low bit
            // of the exponent directly instead of allocating its absolute
            // value first. Consumes `exponent`, which is fine — we return
            // immediately on every branch below.
            let is_odd = (exponent & BigInt::one()) == BigInt::one();
            return Ok(if is_odd {
                BigInt::from(-1)
            } else {
                BigInt::one()
            });
        }

        return Ok(BigInt::zero());
    }

    let exponent = exponent
        .to_biguint()
        .expect("non-negative exponent should convert to BigUint");

    let mut result = BigInt::one();
    let mut factor = base;
    let mut remaining = exponent;

    while !remaining.is_zero() {
        if remaining.bit(0) {
            result *= &factor;
        }

        remaining >>= 1u32;
        if !remaining.is_zero() {
            factor = &factor * &factor;
        }
    }

    Ok(result)
}

// LRM 5.1.11 reduction: fold the binary operator across all operand bits.
// Identity element matches the operator (AND uses 1; OR and XOR use 0);
// the negated forms NAND/NOR/XNOR invert the fold result. Reusing the
// binary truth tables from Phase 6a keeps x/z propagation identical: e.g.
// AND-reduction still gives 0 when any bit is 0 (even with x/z elsewhere),
// because `bitwise_and_bits(0, x)` returns 0.
fn reduce_bits(op: UnaryOp, bits: &[LogicBit]) -> LogicBit {
    let folded = match op {
        UnaryOp::ReductionAnd | UnaryOp::ReductionNand => bits
            .iter()
            .copied()
            .fold(LogicBit::One, bitwise_and_bits),
        UnaryOp::ReductionOr | UnaryOp::ReductionNor => bits
            .iter()
            .copied()
            .fold(LogicBit::Zero, bitwise_or_bits),
        UnaryOp::ReductionXor | UnaryOp::ReductionXnor => bits
            .iter()
            .copied()
            .fold(LogicBit::Zero, bitwise_xor_bits),
        _ => unreachable!("reduce_bits called with non-reduction op"),
    };
    match op {
        UnaryOp::ReductionNand | UnaryOp::ReductionNor | UnaryOp::ReductionXnor => {
            bitwise_not_bit(folded)
        }
        _ => folded,
    }
}

fn is_reduction_op(op: UnaryOp) -> bool {
    matches!(
        op,
        UnaryOp::ReductionAnd
            | UnaryOp::ReductionNand
            | UnaryOp::ReductionOr
            | UnaryOp::ReductionNor
            | UnaryOp::ReductionXor
            | UnaryOp::ReductionXnor
    )
}

// LRM 4.2.1 / 5.2.1 / 5.2.2 bit-/part-select dispatch. The reg lookup
// happens once here so each kind helper receives `&RegValue` directly
// rather than re-resolving the name. Every helper produces an unsigned
// self-determined IntegerValue; outer-context widening is applied by
// the `Expr::Select` arm of `evaluate_expr_in_context`.
//
// `inner` carries the second select in `a[i][...]`. It is only meaningful
// when `reg` is an array (the outer `kind` must then be a `Bit` element
// pick); on a vector reg `inner.is_some()` is rejected because a vector
// select already produces a self-determined integer value with no further
// sub-structure to address.
fn evaluate_select(
    name: &str,
    kind: &SelectKind,
    inner: Option<&SelectKind>,
    session: &Session,
) -> Result<IntegerValue, String> {
    let reg = session
        .lookup(name)
        .ok_or_else(|| format!("undeclared identifier: {name}"))?;
    // LRM 4.9: an array reference is `name[expr]` — exactly one element
    // index that yields the whole packed-vector element. Part-select and
    // indexed-part-select forms apply to the packed range, not to the
    // unpacked dimension, so they have no meaning on the array's outer
    // bracket and are rejected here. Selecting *inside* a chosen element
    // (e.g. `a[i][m:l]`) routes through `evaluate_array_chained_select`
    // when `inner` is present.
    if reg.is_array() {
        return match inner {
            None => evaluate_array_element_select(name, reg, kind, session),
            Some(inner_kind) => {
                evaluate_array_chained_select(name, reg, kind, inner_kind, session)
            }
        };
    }
    if reg.is_real_array() {
        // A real-array element select is real-typed (handled by
        // `evaluate_expr_as_real`); reaching here means the surrounding
        // expression expected an integer but got the real result. The
        // validator catches invalid select shapes (part-select, chained
        // inner) before this point, so the only legal-shape case is a
        // bare `r[i]` flowing into an integer-only consumer.
        return Err(format!(
            "real-array element `{name}[..]` cannot be used as an integer value"
        ));
    }
    if reg.is_real() {
        // The validator (`infer_select_meta`) rejects any select on a
        // scalar `real` per LRM 4.8.1 before evaluation runs.
        unreachable!(
            "validator rejects select on scalar real `{name}` before evaluation"
        );
    }
    if inner.is_some() {
        return Err(format!(
            "chained select on `{name}` is illegal: `{name}` is not an array"
        ));
    }
    let value = reg.require_vector(name)?;
    // LRM 5.2.1: "A bit-select or part-select of a scalar ... shall be
    // illegal." A reg declared without a range is a scalar even when
    // its width happens to be 1, distinct from the 1-bit vector
    // `reg [0:0] a` which does accept selects.
    let range = reg.range.as_ref().ok_or_else(|| {
        format!("bit-select or part-select on scalar reg `{name}` is illegal")
    })?;
    let base = value.base;
    apply_select_kind(value, range, kind, base, session)
}

// Dispatch a `SelectKind` against an already-resolved (value, range)
// pair. Factored out of `evaluate_select` so the chained array-element
// path can reuse the exact same per-kind logic against the chosen
// element's value/range, keeping vector and array inner-selects
// bit-identical to a plain vector-reg select.
fn apply_select_kind(
    value: &IntegerValue,
    range: &RegRange,
    kind: &SelectKind,
    result_base: Base,
    session: &Session,
) -> Result<IntegerValue, String> {
    match kind {
        SelectKind::Bit { index } => evaluate_bit_select(value, range, index, result_base, session),
        SelectKind::PartConst { msb, lsb } => {
            evaluate_part_const_select(value, range, msb, lsb, result_base, session)
        }
        SelectKind::PartIndexedUp {
            base: base_expr,
            width,
        } => evaluate_part_indexed_select(
            value, range, base_expr, width, result_base, session, true,
        ),
        SelectKind::PartIndexedDown {
            base: base_expr,
            width,
        } => evaluate_part_indexed_select(
            value, range, base_expr, width, result_base, session, false,
        ),
    }
}

// Real-array element select — sibling of `evaluate_array_element_select`
// for the f64-element form (`real r [0:3]`). The validator rejects
// non-Bit kinds and chained inner selects before this point, so the
// only legal shape is `r[i]`. x/z in the index or an OOB index falls
// back to 0.0 — LRM 4.2.1 says OOB array reads return x for vector
// elements, but a real has no x state, and `0.0` is the LRM 4.8 init
// value for an unwritten real slot, so it is the closest analog.
fn evaluate_real_array_element_select(
    name: &str,
    index: &Expr,
    session: &Session,
) -> Result<f64, String> {
    let reg = session
        .lookup(name)
        .ok_or_else(|| format!("undeclared identifier: {name}"))?;
    let (_, elements) = reg
        .real_array()
        .expect("evaluate_real_array_element_select called on a non-real-array reg");
    Ok(match resolve_real_array_element_index(name, index, session)? {
        Some(internal) => elements[internal],
        None => 0.0,
    })
}

// Resolves the unpacked-dim index for a real-array element access. Shared
// by the RHS read path (`evaluate_real_array_element_select`) and the
// LHS write path (`lib::apply_real_array_element_assign`). Returns
// `Some(internal_index)` for an in-range integer index, `None` for x/z
// in the index or an OOB index — both cases the caller treats as
// "no slot": reads fall back to 0.0; writes drop silently per LRM 4.2.1.
pub(crate) fn resolve_real_array_element_index(
    name: &str,
    index: &Expr,
    session: &Session,
) -> Result<Option<usize>, String> {
    let reg = session
        .lookup(name)
        .ok_or_else(|| format!("undeclared identifier: {name}"))?;
    let (dim, _) = reg
        .real_array()
        .expect("resolve_real_array_element_index called on a non-real-array reg");
    if expression_is_real(index, session) {
        unreachable!("validator rejects real array-element index before evaluation");
    }
    let index_value = evaluate_expr_in_context(index, None, session)?;
    if index_value.has_unknown_bits() {
        return Ok(None);
    }
    let src_index = index_value.as_bigint(index_value.signed);
    Ok(resolve_reg_index(dim, &src_index))
}

// LRM 4.9 unpacked-array element select. `a[i]` resolves `i` against
// the declared unpacked dimension (`a [msb:lsb]`) and returns the
// whole packed-vector element at that position. The element's
// (width, signed, base) is the packed vector's shape, identical to a
// freshly-declared vector reg of the same packed range.
//
// Out-of-range index / x or z in the index both surface as an all-x
// value of the element's width — mirroring LRM 4.2.1's bit-select OOB
// rule and §4.2.1's "x/z in index → x" rule. The element's signedness
// and base flow through unchanged so an arithmetic context on top of
// `a[i]` lines up with the same context on a vector reg of the same
// packed range.
//
// Only `SelectKind::Bit` is legal on the outer bracket — part-selects
// and indexed-part-selects on the unpacked dimension have no LRM
// meaning (the packed and unpacked dimensions form distinct namespaces
// per §4.9), so we reject them with a dedicated diagnostic instead of
// quietly reinterpreting them.
fn evaluate_array_element_select(
    name: &str,
    reg: &RegValue,
    kind: &SelectKind,
    session: &Session,
) -> Result<IntegerValue, String> {
    let (dim, elements) = reg
        .array()
        .expect("evaluate_array_element_select called on a non-array reg");
    let index = match kind {
        SelectKind::Bit { index } => index,
        SelectKind::PartConst { .. }
        | SelectKind::PartIndexedUp { .. }
        | SelectKind::PartIndexedDown { .. } => {
            return Err(format!(
                "part-select on array `{name}` is illegal; use `{name}[i]` to select an element"
            ));
        }
    };
    // The validator (`infer_select_meta` for RHS, `lvalue_meta` for LHS)
    // rejects a real array-element index before evaluation.
    if expression_is_real(index, session) {
        unreachable!("validator rejects real array-element index before evaluation");
    }
    // Every element shares the packed-range shape, so the OOB / x-z
    // fallback can read its width/signed/base off any one of them. The
    // dim's width is always >= 1 (RegRange::width enforces that at
    // decl time), so `elements[0]` always exists.
    let template = &elements[0];
    let index_value = evaluate_expr_in_context(index, None, session)?;
    if index_value.has_unknown_bits() {
        return Ok(IntegerValue::all_x(
            template.width,
            template.signed,
            template.base,
        ));
    }
    let src_index = index_value.as_bigint(index_value.signed);
    let element = match resolve_reg_index(dim, &src_index) {
        Some(internal) => elements[internal].clone(),
        None => IntegerValue::all_x(template.width, template.signed, template.base),
    };
    Ok(element)
}

// LRM 4.9 + 5.2.1/5.2.2: `a[i][...]` — pick an unpacked element, then
// run a bit-/part-select against the chosen element's packed range.
// Element selection (outer `kind`) shares all rules with
// `evaluate_array_element_select`: only `SelectKind::Bit` is legal on
// the outer bracket; real indices, x/z indices, and OOB indices all
// fall back to an all-x element of the packed shape. The inner select
// (`inner_kind`) then runs against either the chosen element or the
// all-x fallback, producing a self-determined unsigned value with the
// element's display base — bit-identical to the same select on a plain
// vector reg of the same packed range. Scalar array elements (regs
// declared without a packed range, e.g. `reg a [0:7]`) have no bits to
// address, so the inner select is rejected with the same diagnostic
// shape the vector-reg path uses for scalars.
fn evaluate_array_chained_select(
    name: &str,
    reg: &RegValue,
    kind: &SelectKind,
    inner_kind: &SelectKind,
    session: &Session,
) -> Result<IntegerValue, String> {
    let (dim, elements) = reg
        .array()
        .expect("evaluate_array_chained_select called on a non-array reg");
    let index = match kind {
        SelectKind::Bit { index } => index,
        SelectKind::PartConst { .. }
        | SelectKind::PartIndexedUp { .. }
        | SelectKind::PartIndexedDown { .. } => {
            return Err(format!(
                "part-select on array `{name}` is illegal; use `{name}[i]` to select an element"
            ));
        }
    };
    // The validator (`infer_select_meta` for RHS, `lvalue_meta` for LHS)
    // rejects a real array-element index before evaluation.
    if expression_is_real(index, session) {
        unreachable!("validator rejects real array-element index before evaluation");
    }
    // A bit-/part-select on the chosen element requires the element to
    // have a packed range — scalar array elements have no bits to
    // address (LRM 5.2.1 scalar-reg rule).
    let range = reg.range.as_ref().ok_or_else(|| {
        format!("bit-select or part-select on scalar array element `{name}` is illegal")
    })?;
    let template = &elements[0];
    let element = {
        let index_value = evaluate_expr_in_context(index, None, session)?;
        if index_value.has_unknown_bits() {
            IntegerValue::all_x(template.width, template.signed, template.base)
        } else {
            let src_index = index_value.as_bigint(index_value.signed);
            match resolve_reg_index(dim, &src_index) {
                Some(internal) => elements[internal].clone(),
                None => IntegerValue::all_x(template.width, template.signed, template.base),
            }
        }
    };
    apply_select_kind(&element, range, inner_kind, element.base, session)
}

// LRM 4.2.1: a bit-select with an x/z anywhere in the index yields x;
// an out-of-range index also yields x. The index is self-determined and
// interpreted under its own signedness so negative-endpoint regs
// (e.g. `reg [-1:2]`) and signed-indexed selects line up.
fn evaluate_bit_select(
    value: &IntegerValue,
    range: &RegRange,
    index: &Expr,
    result_base: Base,
    session: &Session,
) -> Result<IntegerValue, String> {
    // The validator (`select_meta_width` via
    // `validate_select_expr_structure`) rejects a real bit-select index
    // before evaluation.
    if expression_is_real(index, session) {
        unreachable!("validator rejects real bit-select index before evaluation");
    }
    let index_value = evaluate_expr_in_context(index, None, session)?;
    if index_value.has_unknown_bits() {
        return Ok(IntegerValue::all_x(1, false, result_base));
    }
    let src_index = index_value.as_bigint(index_value.signed);
    let bit = match resolve_reg_index(range, &src_index) {
        Some(internal) => value.bits[internal],
        None => LogicBit::X,
    };
    Ok(IntegerValue::computed(1, false, result_base, vec![bit]))
}

// LRM 5.2.1 `[msb:lsb]` part-select. The endpoints are runtime
// expressions in vcal (the LRM requires constants, but the REPL has no
// separate elaboration stage), and their direction must match the
// declared reg.
fn evaluate_part_const_select(
    value: &IntegerValue,
    range: &RegRange,
    msb_expr: &Expr,
    lsb_expr: &Expr,
    result_base: Base,
    session: &Session,
) -> Result<IntegerValue, String> {
    let msb_sel = evaluate_constant_range_endpoint(msb_expr, session, "msb")?;
    let lsb_sel = evaluate_constant_range_endpoint(lsb_expr, session, "lsb")?;
    check_part_select_direction(range, &msb_sel, &lsb_sel)?;
    let width = compute_select_width(&msb_sel, &lsb_sel)?;
    materialize_part_select(value, range, &msb_sel, &lsb_sel, width, result_base)
}

// LRM 5.2.2 indexed part-select. `width` is a positive constant; `base`
// is a self-determined integer expression. The source range is always
// numerically [base, base+w-1] for `+:` and [base-w+1, base] for `-:`,
// independent of the declared reg direction; which end of that source
// range is the result's MSB depends on the reg's declared direction.
fn evaluate_part_indexed_select(
    value: &IntegerValue,
    range: &RegRange,
    base_expr: &Expr,
    width_expr: &Expr,
    result_base: Base,
    session: &Session,
    is_up: bool,
) -> Result<IntegerValue, String> {
    let width = evaluate_indexed_select_width(width_expr, session)?;
    // The validator (`select_meta_width` via
    // `validate_select_expr_structure`) rejects a real indexed-base
    // before evaluation.
    if expression_is_real(base_expr, session) {
        unreachable!("validator rejects real indexed part-select base before evaluation");
    }
    let base_value = evaluate_expr_in_context(base_expr, None, session)?;
    if base_value.has_unknown_bits() {
        return Ok(IntegerValue::all_x(width, false, result_base));
    }
    let base_int = base_value.as_bigint(base_value.signed);
    let span = BigInt::from(width - 1);
    let (src_lo, src_hi) = if is_up {
        let hi = &base_int + &span;
        (base_int, hi)
    } else {
        let lo = &base_int - &span;
        (lo, base_int)
    };
    // Forward decl (msb_decl >= lsb_decl): larger source index is more
    // significant. Reversed decl: smaller source index is more
    // significant.
    let (msb_sel, lsb_sel) = if range.msb < range.lsb {
        (src_lo, src_hi)
    } else {
        (src_hi, src_lo)
    };
    materialize_part_select(value, range, &msb_sel, &lsb_sel, width, result_base)
}

// Copies the bits of a part-select into the result, LSB-first. LRM
// 4.2.1's "out-of-range → x" rule is applied per position, so in-range
// source bits keep their value and only the out-of-range positions
// become x (e.g. `reg [3:0] a = 4'b0101; a[4:3]` → `2'bx0`).
fn materialize_part_select(
    value: &IntegerValue,
    range: &RegRange,
    msb_sel: &BigInt,
    lsb_sel: &BigInt,
    width: usize,
    result_base: Base,
) -> Result<IntegerValue, String> {
    let step: BigInt = if msb_sel >= lsb_sel {
        BigInt::one()
    } else {
        -BigInt::one()
    };
    let mut bits = Vec::with_capacity(width);
    let mut src = lsb_sel.clone();
    for _ in 0..width {
        let bit = match resolve_reg_index(range, &src) {
            Some(internal) => value.bits[internal],
            None => LogicBit::X,
        };
        bits.push(bit);
        src += &step;
    }
    Ok(IntegerValue::computed(width, false, result_base, bits))
}

// Source-index → internal-bits-index mapping. The formula
// `internal = |src - lsb_decl|` works uniformly for forward decls
// (`[7:0]`), reversed decls (`[0:7]`), and negative-endpoint decls
// (`[-1:2]`). Scalar regs are rejected upstream in `evaluate_select`
// per LRM 5.2.1, so a `range` is always available here.
fn resolve_reg_index(range: &RegRange, src: &BigInt) -> Option<usize> {
    let (lo, hi) = if range.msb >= range.lsb {
        (&range.lsb, &range.msb)
    } else {
        (&range.msb, &range.lsb)
    };
    if src < lo || src > hi {
        return None;
    }
    (src - &range.lsb).abs().to_usize()
}

// Shape mirrors lib.rs::evaluate_range_endpoint — same "constant
// integer, no x/z" contract a reg-decl range half follows, just with a
// "part-select" diagnostic prefix so the error attributes correctly.
fn evaluate_constant_range_endpoint(
    expr: &Expr,
    session: &Session,
    role: &str,
) -> Result<BigInt, String> {
    if expression_is_real(expr, session) {
        return Err(format!("part-select {role} cannot be real"));
    }
    let value = evaluate_constant_expr(expr, session)?;
    if value.has_unknown_bits() {
        return Err(format!("part-select {role} contains unknown bits"));
    }
    Ok(value.as_bigint(value.signed))
}

// LRM 5.2.2: the `width` half of an indexed part-select is a
// constant_expression and "shall be a positive constant". Reject
// real, x/z, zero, and negative values up front so the materialise
// step can assume a usize-fitting positive count.
fn evaluate_indexed_select_width(expr: &Expr, session: &Session) -> Result<usize, String> {
    if expression_is_real(expr, session) {
        return Err("indexed part-select width cannot be real".to_string());
    }
    let value = evaluate_constant_expr(expr, session)?;
    if value.has_unknown_bits() {
        return Err("indexed part-select width contains unknown bits".to_string());
    }
    let width = value.as_bigint(value.signed);
    if width.sign() != Sign::Plus {
        return Err("indexed part-select width must be positive".to_string());
    }
    width
        .to_usize()
        .ok_or_else(|| "indexed part-select width too large".to_string())
}

// LRM 5.2.1: "the first expression shall address a more significant bit
// than the second expression". Read strictly: the relative ordering of
// msb_sel vs lsb_sel must match the declared reg's direction. iverilog
// merely warns; we treat it as an error because the rule is unambiguous
// and silent reinterpretation hides a real bug.
fn check_part_select_direction(
    range: &RegRange,
    msb_sel: &BigInt,
    lsb_sel: &BigInt,
) -> Result<(), String> {
    let forward = range.msb >= range.lsb;
    let ok = if forward {
        msb_sel >= lsb_sel
    } else {
        msb_sel <= lsb_sel
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "part-select direction does not match reg declaration: \
             reg is [{}:{}], select is [{}:{}]",
            range.msb, range.lsb, msb_sel, lsb_sel
        ))
    }
}

fn compute_select_width(msb_sel: &BigInt, lsb_sel: &BigInt) -> Result<usize, String> {
    let diff = (msb_sel - lsb_sel).abs() + BigInt::one();
    diff.to_usize()
        .ok_or_else(|| "part-select width too large".to_string())
}

// LHS structural validation + width for one `SelectKind` against a
// `range` (which is either the named vector reg's packed range, or the
// chosen array element's packed range for a chained `a[i][...]` LHS).
// Runs the same checks the RHS materialisers run (real-typed bit-select
// index / indexed part-select base, part-select direction match against
// `range`, indexed-width is a positive constant), so any malformed
// select on the LHS surfaces before the RHS is even looked at — keeping
// the precedence "LHS structural error wins over RHS error" identical
// to the bare-vector case.
fn select_meta_width(
    kind: &SelectKind,
    range: &RegRange,
    session: &Session,
) -> Result<usize, String> {
    match kind {
        SelectKind::Bit { index } => {
            if expression_is_real(index, session) {
                return Err("bit-select index cannot be real".to_string());
            }
            Ok(1)
        }
        SelectKind::PartConst { msb, lsb } => {
            let msb_sel = evaluate_constant_range_endpoint(msb, session, "msb")?;
            let lsb_sel = evaluate_constant_range_endpoint(lsb, session, "lsb")?;
            check_part_select_direction(range, &msb_sel, &lsb_sel)?;
            compute_select_width(&msb_sel, &lsb_sel)
        }
        SelectKind::PartIndexedUp { base, width }
        | SelectKind::PartIndexedDown { base, width } => {
            if expression_is_real(base, session) {
                return Err("indexed part-select base cannot be real".to_string());
            }
            evaluate_indexed_select_width(width, session)
        }
    }
}

// LRM A.6.2 blocking assignment, full A.8.5 variable_lvalue form. The
// entrypoint runs every structural check (declared-name lookup,
// scalar-reg-with-select rejection, part-select direction match, x/z in
// constant endpoints, zero indexed width, real-typed bit-select index /
// indexed part-select base) *before* the RHS is evaluated, then
// distributes the RHS bits into a staged variable map.
// The caller swaps the staged map into the live session on success, so a
// failure anywhere — even after some leaves' indices have been resolved
// — leaves the session untouched.
//
// The returned `IntegerValue` is the RHS evaluated in the total-LHS
// context (width = sum of leaf widths, signed = false for any
// select/concat leaf, base = leftmost-leaf base). For a bare-name LHS
// this matches the reg's stored `(width, signed, base)` so the printed
// canonical form is bit-identical to the pre-lvalue behavior.
//
// LRM 4.9 array-element writes — bare `a[i] = expr`, chained
// `a[i][m:l] = expr`, and concat leaves carrying either form — route
// through the same `lvalue_meta` / `distribute_bits_to_leaves` pipeline
// the vector-reg case uses. `LeafTarget::ArrayElement` carries the
// chosen unpacked-dim index alongside the per-position internal-bit map,
// so a single rightward walk through the rhs bit stream uniformly
// services every leaf shape; the outer-index x/z and OOB cases drop the
// element via `element: None` while still advancing the cursor by the
// leaf's nominal width (matching LRM 4.2.1's "no assignment performed").
pub(crate) fn evaluate_lvalue_assignment(
    lvalue: &LValue,
    rhs: &Expr,
    session: &Session,
) -> Result<(HashMap<String, RegValue>, IntegerValue), String> {
    // `lvalue_meta` plays the structural pre-pass role for LValues (the same
    // job `validate_expr_structure` does for RValues), so its errors carry
    // the "Semantic error: " stage prefix to stay consistent with the RHS
    // path. The RHS is prefixed via `evaluate_assignment_rhs` -> `semantic_check`.
    let meta = lvalue_meta(lvalue, session).map_err(|e| format!("Semantic error: {e}"))?;
    let mut leaves: Vec<&LValue> = Vec::new();
    flatten_lvalue_leaves(lvalue, &mut leaves);
    let rhs_value = evaluate_assignment_rhs(rhs, meta.width, meta.signed, meta.base, session)?;
    // Per LRM 5.5.1 the integer pipeline widens to `max(ctx_width,
    // expr_width)` (so `r = -5` on an 8-bit `r` returns 32 bits — the
    // unsized literal's natural width wins). Distributing those over the
    // LHS requires exactly `meta.width` bits: truncate the LSB-end if
    // the RHS came back wider, extend with context-fill if narrower.
    // Re-stamp `(width, signed, base)` from the lvalue context for the
    // same reason — the leftmost-base inference would otherwise let the
    // RHS's display base leak into the echo (so `a = 4'hF + 4'hF` would
    // render in hex).
    let sized = rhs_value.resized_to_context(meta.width, meta.signed);
    let displayed = IntegerValue {
        width: meta.width,
        signed: meta.signed,
        base: meta.base,
        bits: sized.bits.clone(),
        unsized_literal: false,
    };
    let mut staged = session.variables.clone();
    distribute_bits_to_leaves(&leaves, &sized.bits, &mut staged, session)?;
    Ok((staged, displayed))
}

// LRM 5.6 LHS context derivation. Runs the same constant-endpoint /
// direction / scalar-reg / indexed-width checks the RHS select helpers
// do, so any structural problem on the LHS surfaces before the RHS is
// even looked at. Returning an `ExprMeta` keeps the call shape parallel
// to `infer_expr_meta` so the surrounding context-propagation story
// stays one-paradigm.
fn lvalue_meta(lvalue: &LValue, session: &Session) -> Result<ExprMeta, String> {
    match lvalue {
        LValue::Name(name) => {
            let reg = session
                .lookup(name)
                .ok_or_else(|| format!("undeclared identifier: {name}"))?;
            let value = reg.require_vector(name)?;
            Ok(ExprMeta {
                width: value.width,
                signed: value.signed,
                base: value.base,
            })
        }
        LValue::Select { name, kind, inner } => {
            let reg = session
                .lookup(name)
                .ok_or_else(|| format!("undeclared identifier: {name}"))?;
            if reg.is_real_array() {
                // Validate the shape exactly like `infer_select_meta`'s
                // real-array branch: only `r[i]` is structurally legal.
                // The legal `r[i] = expr` case is intercepted in
                // `lib::apply_assign` and routed to the real pipeline
                // before this validator runs, so reaching here means
                // the element appears inside a vector context (a concat
                // lvalue, etc.) — the integer-bits pipeline cannot
                // consume an f64, so we reject with a context-aware
                // diagnostic.
                match kind {
                    SelectKind::Bit { index } => {
                        if expression_is_real(index, session) {
                            return Err("array element index cannot be real".to_string());
                        }
                    }
                    SelectKind::PartConst { .. }
                    | SelectKind::PartIndexedUp { .. }
                    | SelectKind::PartIndexedDown { .. } => {
                        return Err(format!(
                            "part-select on array `{name}` is illegal; use `{name}[i]` to select an element"
                        ));
                    }
                }
                if inner.is_some() {
                    return Err(format!(
                        "bit-select or part-select on real-array element `{name}` is illegal"
                    ));
                }
                return Err(format!(
                    "real-array element `{name}[..]` cannot appear in a vector lvalue"
                ));
            }
            if reg.is_array() {
                // LRM 4.9: only `Bit` is legal as the outer select on an
                // array name. `evaluate_array_element_select` enforces
                // the same rejection on the RHS path; surfacing it here
                // keeps the LHS structural error class consistent.
                let index = match kind {
                    SelectKind::Bit { index } => index,
                    SelectKind::PartConst { .. }
                    | SelectKind::PartIndexedUp { .. }
                    | SelectKind::PartIndexedDown { .. } => {
                        return Err(format!(
                            "part-select on array `{name}` is illegal; use `{name}[i]` to select an element"
                        ));
                    }
                };
                if expression_is_real(index, session) {
                    return Err("array element index cannot be real".to_string());
                }
                // The chosen element's shape is the one every element
                // shares — array decl bakes Base::Binary into the
                // element template, so `(width, signed, base)` is read
                // off `elements[0]` (always present: RegRange::width
                // enforces count >= 1 at decl time).
                let (_, elements) = reg
                    .array()
                    .expect("is_array() => array() returns Some");
                let template = &elements[0];
                if let Some(inner_kind) = inner {
                    // `a[i][...]` — inner select runs against the
                    // chosen element. Mirrors `evaluate_array_chained_select`'s
                    // scalar-element rejection so the structural error
                    // matches the RHS-path diagnostic.
                    let element_range = reg.range.as_ref().ok_or_else(|| {
                        format!(
                            "bit-select or part-select on scalar array element `{name}` is illegal"
                        )
                    })?;
                    let width = select_meta_width(inner_kind, element_range, session)?;
                    Ok(ExprMeta {
                        width,
                        signed: false,
                        base: template.base,
                    })
                } else {
                    // `a[i] = expr` — element-shape context.
                    Ok(ExprMeta {
                        width: template.width,
                        signed: template.signed,
                        base: template.base,
                    })
                }
            } else {
                if reg.is_real() {
                    // LRM 4.8.1: select on a scalar real is prohibited,
                    // mirroring the `infer_select_meta` rejection on the
                    // RHS path.
                    return Err(format!(
                        "bit-select or part-select on real variable `{name}` is not allowed"
                    ));
                }
                if inner.is_some() {
                    // Same diagnostic as the RHS chained-select-on-vector
                    // rejection (`evaluate_select`).
                    return Err(format!(
                        "chained select on `{name}` is illegal: `{name}` is not an array"
                    ));
                }
                let value = reg.require_vector(name)?;
                // LRM 5.2.1 scalar-reg rejection. Mirrors `evaluate_select`.
                let range = reg.range.as_ref().ok_or_else(|| {
                    format!("bit-select or part-select on scalar reg `{name}` is illegal")
                })?;
                let width = select_meta_width(kind, range, session)?;
                Ok(ExprMeta {
                    width,
                    signed: false,
                    base: value.base,
                })
            }
        }
        LValue::Concat(items) => {
            if items.is_empty() {
                return Err("lvalue concatenation requires at least one operand".to_string());
            }
            let mut total_width = 0usize;
            let mut leftmost_base = Base::Binary;
            for (idx, item) in items.iter().enumerate() {
                let item_meta = lvalue_meta(item, session)?;
                total_width = total_width.saturating_add(item_meta.width);
                if idx == 0 {
                    leftmost_base = item_meta.base;
                }
            }
            if total_width == 0 {
                return Err("lvalue must have at least one operand with positive size".to_string());
            }
            Ok(ExprMeta {
                width: total_width,
                signed: false,
                base: leftmost_base,
            })
        }
    }
}

// Left-to-right (MSB-side first) leaf enumeration. Used by both the
// write-collision pass and the bit-distribution pass; both walk the
// resulting slice in reverse so the rightmost leaf consumes the LSB end
// of the RHS bit stream.
fn flatten_lvalue_leaves<'a>(lvalue: &'a LValue, out: &mut Vec<&'a LValue>) {
    match lvalue {
        LValue::Name(_) | LValue::Select { .. } => out.push(lvalue),
        LValue::Concat(items) => {
            for item in items {
                flatten_lvalue_leaves(item, out);
            }
        }
    }
}

// Where a leaf's RHS bits land. `Vector` is the existing scalar/vector
// reg target; `ArrayElement` carries the chosen unpacked-dim index
// alongside the per-position internal-bit map. `positions.len()` always
// equals the leaf's nominal width (so the bit-cursor advances uniformly
// even on dropped positions). The split lets `distribute_bits_to_leaves`
// run one rightward walk through the RHS bit stream and dispatch on the
// target variant per leaf, without two parallel codepaths.
//
// For `ArrayElement`, `element: None` means the outer-index was x/z or
// OOB (LRM 4.9 + 4.2.1 "no assignment performed") — the slot still
// consumes `positions.len()` cursor bits so the surrounding concat
// distribution stays aligned, but no element is written.
enum LeafTarget {
    Vector {
        name: String,
        positions: Vec<Option<usize>>,
    },
    ArrayElement {
        name: String,
        element: Option<usize>,
        positions: Vec<Option<usize>>,
    },
}

impl LeafTarget {
    fn positions(&self) -> &[Option<usize>] {
        match self {
            Self::Vector { positions, .. } | Self::ArrayElement { positions, .. } => positions,
        }
    }
}

// LSB-first per-position resolver shared by the const and indexed
// part-select arms (LHS distribution + the outer-element-dropped
// inner-select fallback). Steps from `lsb_sel` toward `msb_sel` (±1
// depending on which is larger) and maps each source index through
// `resolve_reg_index`. Mirrors `materialize_part_select` so the LHS
// distribution lines up bit-for-bit with the equivalent RHS read.
fn per_position_indices(
    range: &RegRange,
    msb_sel: &BigInt,
    lsb_sel: &BigInt,
    width: usize,
) -> Vec<Option<usize>> {
    let step: BigInt = if msb_sel >= lsb_sel {
        BigInt::one()
    } else {
        -BigInt::one()
    };
    let mut indices = Vec::with_capacity(width);
    let mut src = lsb_sel.clone();
    for _ in 0..width {
        indices.push(resolve_reg_index(range, &src));
        src += &step;
    }
    indices
}

// LSB-first per-position internal-bit map for one `SelectKind` against a
// `range`. Used by both the vector-reg distribution and the chained
// `a[i][...]` distribution (the latter passes the element's packed
// range). Assumes `lvalue_meta` has already done the structural checks
// (direction match, indexed-width positive, real-index rejection) so
// only the *value-dependent* resolution (index x/z → all-`None`,
// out-of-range index → `None` at that slot) happens here. Returning a
// `Vec` keyed off the leaf's nominal width keeps the bit cursor in
// `distribute_bits_to_leaves` uniform even when the entire select is
// dropped.
fn select_positions(
    kind: &SelectKind,
    range: &RegRange,
    session: &Session,
) -> Result<Vec<Option<usize>>, String> {
    match kind {
        SelectKind::Bit { index } => {
            let index_value = evaluate_expr_in_context(index, None, session)?;
            if index_value.has_unknown_bits() {
                // LRM 4.2.1: x/z index → no assignment performed.
                return Ok(vec![None]);
            }
            let src = index_value.as_bigint(index_value.signed);
            Ok(vec![resolve_reg_index(range, &src)])
        }
        SelectKind::PartConst { msb, lsb } => {
            // Endpoints / direction / width were validated by
            // `lvalue_meta`; re-evaluating here is cheap and keeps this
            // helper standalone.
            let msb_sel = evaluate_constant_range_endpoint(msb, session, "msb")?;
            let lsb_sel = evaluate_constant_range_endpoint(lsb, session, "lsb")?;
            let width = compute_select_width(&msb_sel, &lsb_sel)?;
            Ok(per_position_indices(range, &msb_sel, &lsb_sel, width))
        }
        SelectKind::PartIndexedUp {
            base: base_expr,
            width: width_expr,
        }
        | SelectKind::PartIndexedDown {
            base: base_expr,
            width: width_expr,
        } => {
            let is_up = matches!(kind, SelectKind::PartIndexedUp { .. });
            let width = evaluate_indexed_select_width(width_expr, session)?;
            let base_value = evaluate_expr_in_context(base_expr, None, session)?;
            if base_value.has_unknown_bits() {
                // LRM 5.2.2 / 4.2.1: x/z in the base means every
                // position is unresolved → all dropped.
                return Ok(vec![None; width]);
            }
            let base_int = base_value.as_bigint(base_value.signed);
            let span = BigInt::from(width - 1);
            let (src_lo, src_hi) = if is_up {
                let hi = &base_int + &span;
                (base_int, hi)
            } else {
                let lo = &base_int - &span;
                (lo, base_int)
            };
            // Same direction logic as `evaluate_part_indexed_select`:
            // for a forward decl, the larger source index is the MSB
            // side; for a reversed decl, the smaller is.
            let (msb_sel, lsb_sel) = if range.msb < range.lsb {
                (src_lo, src_hi)
            } else {
                (src_hi, src_lo)
            };
            Ok(per_position_indices(range, &msb_sel, &lsb_sel, width))
        }
    }
}

// Resolves a leaf to a `LeafTarget`. For a bare vector name the target
// is `Vector` with every position present; for a vector select it is
// `Vector` with the chosen per-position internals (some `None` for
// x/z-index or OOB drops). For an array-element leaf (`a[i]` or
// `a[i][...]`) it is `ArrayElement` carrying the resolved outer index
// (or `None` if the outer index was x/z or OOB) and the per-position
// map for the inner span — bare-element if no inner select (every
// internal bit), inner-select if one is present. Structural checks have
// already run inside `lvalue_meta`, so this helper only resolves
// value-dependent indices.
fn leaf_target(leaf: &LValue, session: &Session) -> Result<LeafTarget, String> {
    match leaf {
        LValue::Name(name) => {
            let reg = session
                .lookup(name)
                .ok_or_else(|| format!("undeclared identifier: {name}"))?;
            let value = reg.require_vector(name)?;
            // Bare-name leaf spans every internal bit, LSB-first.
            let positions = (0..value.width).map(Some).collect();
            Ok(LeafTarget::Vector {
                name: name.clone(),
                positions,
            })
        }
        LValue::Select { name, kind, inner } => {
            let reg = session
                .lookup(name)
                .ok_or_else(|| format!("undeclared identifier: {name}"))?;
            if reg.is_array() {
                // Outer kind is guaranteed `Bit` by `lvalue_meta`.
                let (dim, elements) = reg
                    .array()
                    .expect("is_array() => array() returns Some");
                let SelectKind::Bit { index } = kind else {
                    unreachable!("lvalue_meta rejected non-Bit outer select on array");
                };
                let index_value = evaluate_expr_in_context(index, None, session)?;
                let element = if index_value.has_unknown_bits() {
                    None
                } else {
                    let src = index_value.as_bigint(index_value.signed);
                    resolve_reg_index(dim, &src)
                };
                let positions = if let Some(inner_kind) = inner {
                    // Inner select runs against the chosen element's
                    // packed range. Resolve positions regardless of
                    // whether the outer index dropped — both branches
                    // produce a vec of length `inner_width`, so the
                    // bit-cursor stays aligned. Index/base value errors
                    // in the inner select still surface here.
                    let element_range = reg.range.as_ref().expect(
                        "chained inner select on scalar element rejected by lvalue_meta",
                    );
                    select_positions(inner_kind, element_range, session)?
                } else {
                    // Whole-element write — every internal bit is
                    // present (LSB-first).
                    let template = &elements[0];
                    (0..template.width).map(Some).collect()
                };
                Ok(LeafTarget::ArrayElement {
                    name: name.clone(),
                    element,
                    positions,
                })
            } else {
                // Vector leaf — structural validation (incl. scalar-reg
                // rejection) already happened in `lvalue_meta`.
                let _ = reg.require_vector(name)?;
                let range = reg.range.as_ref().expect(
                    "scalar-reg-with-select rejected by lvalue_meta",
                );
                let positions = select_positions(kind, range, session)?;
                Ok(LeafTarget::Vector {
                    name: name.clone(),
                    positions,
                })
            }
        }
        // Concats aren't leaves — flatten_lvalue_leaves never reaches them.
        LValue::Concat(_) => unreachable!("leaf_target called on a Concat"),
    }
}

// Walks leaves right-to-left (rightmost leaf = LSB end of the RHS bit
// stream) with a cursor into `rhs_bits` (LSB-first). For each leaf,
// `leaf_target` returns the `LeafTarget` describing where each internal
// position lands; in-range positions receive the corresponding RHS bit,
// out-of-range / x-z-index positions consume their cursor slot silently
// (LRM 4.2.1 "no assignment performed"). For an `ArrayElement` whose
// outer index x/z'd or went OOB, the entire leaf drops (LRM 4.9 +
// 4.2.1) but still consumes its nominal width's worth of cursor bits.
// The expect on the get_mut is safe because every leaf name was
// resolved by `lvalue_meta` before the staged map was created.
//
// IEEE 1364-2005 does not say what happens when an lvalue
// concatenation names the same target bit more than once
// (`{a[0], a[0]} = ...`), so the result is implementation-defined.
// vcal's natural right-to-left walk just lets each write overwrite the
// staged bit, so the leaf closer to the MSB end of the concat wins —
// it is processed last because the rightmost leaf is processed first.
fn distribute_bits_to_leaves(
    leaves: &[&LValue],
    rhs_bits: &[LogicBit],
    staged: &mut HashMap<String, RegValue>,
    session: &Session,
) -> Result<(), String> {
    let mut cursor = 0usize;
    for leaf in leaves.iter().rev() {
        let target = leaf_target(leaf, session)?;
        let width = target.positions().len();
        match target {
            LeafTarget::Vector { name, positions } => {
                let reg = staged
                    .get_mut(&name)
                    .expect("leaf name validated by lvalue_meta");
                let value = reg
                    .vector_mut()
                    .expect("vector leaf confirmed by lvalue_meta");
                for opt_internal in positions {
                    let bit = rhs_bits[cursor];
                    if let Some(internal) = opt_internal {
                        value.bits[internal] = bit;
                    }
                    cursor += 1;
                }
            }
            LeafTarget::ArrayElement {
                name,
                element,
                positions,
            } => {
                if let Some(element_index) = element {
                    let reg = staged
                        .get_mut(&name)
                        .expect("leaf name validated by lvalue_meta");
                    let (_, elements) = reg
                        .array_mut()
                        .expect("array leaf confirmed by lvalue_meta");
                    let target_element = &mut elements[element_index];
                    for opt_internal in positions {
                        let bit = rhs_bits[cursor];
                        if let Some(internal) = opt_internal {
                            target_element.bits[internal] = bit;
                        }
                        cursor += 1;
                    }
                } else {
                    // Outer-index x/z or OOB: no assignment performed,
                    // but the cursor still advances by the leaf's
                    // nominal width so adjacent leaves stay aligned.
                    cursor += width;
                }
            }
        }
    }
    debug_assert_eq!(cursor, rhs_bits.len(), "leaf widths sum to total LHS width");
    Ok(())
}
