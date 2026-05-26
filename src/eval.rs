use num_bigint::{BigInt, BigUint, Sign};
use num_traits::{FromPrimitive, One, Signed, ToPrimitive, Zero};

use crate::{RegRange, RegValue, Session};
use crate::parser::{BinaryOp, Expr, MathFunctionKind, RealConversionKind, SelectKind, UnaryOp};
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

pub(crate) fn evaluate_expr(expr: &Expr, session: &Session) -> Result<Value, String> {
    if expression_is_real(expr) {
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
// with ties away from zero (the same rule `$itor`'s internal real→int
// step uses). NaN / ±∞ have no integer image, so the lvalue is filled
// with x bits at its declared width — matching how `$rtoi` surfaces
// "no defined integer" rather than silently mapping to zero.
pub(crate) fn evaluate_assignment_rhs(
    rhs: &Expr,
    width: usize,
    signed: bool,
    base: Base,
    session: &Session,
) -> Result<IntegerValue, String> {
    if expression_is_real(rhs) {
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

pub(crate) fn expression_is_real(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(_) => false,
        Expr::RealLiteral(_) => true,
        Expr::Grouped(inner) => expression_is_real(inner),
        Expr::Unary { op, expr } => match op {
            UnaryOp::Plus | UnaryOp::Minus => expression_is_real(expr),
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
            | BinaryOp::Power => expression_is_real(lhs) || expression_is_real(rhs),
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
        } => expression_is_real(then_expr) || expression_is_real(else_expr),
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
        // Reg variables are integer-only — the only declarable type so
        // far. The Session lookup happens later in the integer pipeline;
        // here we only need the result-type, and that's always integer.
        Expr::Identifier(_) => false,
        // Bit-select / part-select on a reg is always integer-typed
        // (LRM 4.7: part-select is unsigned).
        Expr::Select { .. } => false,
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
    if !expression_is_real(expr) {
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
                // LRM 17.8 + §3.5.3: a real operand goes through implicit
                // real→integer→real. The implicit real→integer step rounds
                // to nearest with ties away from zero (§3.5.3), so e.g.
                // $itor(-2.6) is -3.0, not -2.0 (which is what $rtoi's
                // truncation gives). NaN / ±∞ have no integer image, so
                // `real_to_integer_bigint` returns `None` — matching the
                // rule $rtoi already documents. §3.5.3's int→real then maps
                // every x bit to 0, so the chain collapses to 0.0, keeping
                // $itor self-consistent with $rtoi.
                //
                // An integer-typed operand must skip that round-trip: when
                // the magnitude exceeds f64 range, `integer_value_to_f64`
                // saturates to ±∞ (BigInt::to_f64's documented behavior),
                // and routing that ±∞ back through real→integer would
                // collapse it to 0.0 — destroying the value the conversion
                // was supposed to surface.
                if expression_is_real(arg) {
                    let real_val = evaluate_expr_as_real(arg, session)?;
                    match real_to_integer_bigint(real_val) {
                        Some(bigint) => Ok(bigint.to_f64().expect("BigInt::to_f64 is total")),
                        None => Ok(0.0),
                    }
                } else {
                    let int_val = evaluate_expr_in_context(arg, None, session)?;
                    Ok(integer_value_to_f64(&int_val))
                }
            }
            RealConversionKind::BitsToReal => {
                // LRM 17.8: reverse of $realtobits. Argument is the 64-bit
                // IEEE 754 bit pattern, so we require an exactly 64-bit
                // self-determined width — narrower operands (e.g. 32-bit
                // unsized literals) and wider ones both get rejected to
                // avoid silent zero-extension or truncation. Real operand
                // has no defined bit-cast here, so reject it too.
                if expression_is_real(arg) {
                    return Err("$bitstoreal argument cannot be real".to_string());
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
        // A reg is integer-only, so the integer fast-path at the top of
        // this function would have routed any identifier through
        // `evaluate_expr_in_context`. Reaching this branch means the
        // dispatch missed an integer leaf in a real-result expression.
        Expr::Identifier(_) => unreachable!("identifier always has integer result type"),
        Expr::Select { .. } => unreachable!("select always has integer result type"),
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
    if expression_is_real(expr) {
        Ok(logical_value_of_real(evaluate_expr_as_real(expr, session)?))
    } else {
        Ok(logical_value(&evaluate_expr_in_context(expr, None, session)?))
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
            let value = session
                .lookup(name)
                .ok_or_else(|| format!("undeclared identifier: {name}"))?;
            Ok(match context {
                Some(context) => value.value.resized_to_context(context.width, context.signed),
                None => value.value.clone(),
            })
        }
        // Bit- / part-select on a declared reg: the slice is computed
        // self-determined (its width and unsigned-ness are fixed by the
        // select form), then widens to the outer context the same way an
        // Identifier does. Per LRM 4.7, the result is always unsigned, so
        // a wider signed outer context zero-extends rather than
        // sign-extends — `resized_to_context(width, context.signed)`
        // already implements that because the value itself is unsigned.
        Expr::Select { name, kind } => {
            let value = evaluate_select(name, kind, session)?;
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
            let value = session
                .lookup(name)
                .ok_or_else(|| format!("undeclared identifier: {name}"))?;
            Ok(ExprMeta {
                width: value.value.width,
                signed: value.value.signed,
                base: value.value.base,
            })
        }
        // LRM 4.7 / 5.2.1 / 5.2.2: select width depends on its form
        // (1 for bit-select, |msb-lsb|+1 for constant part-select,
        // the constant `width` for indexed part-selects). The result is
        // always unsigned; the display base flows from the reg's stored
        // base so an arithmetic context still gets a meaningful render.
        Expr::Select { name, kind } => {
            let reg = session
                .lookup(name)
                .ok_or_else(|| format!("undeclared identifier: {name}"))?;
            let width = select_result_width(kind, session)?;
            Ok(ExprMeta {
                width,
                signed: false,
                base: reg.value.base,
            })
        }
    }
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
    if expression_is_real(expr) {
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
                return Err(format!(
                    "operator {} not allowed on real operand",
                    unary_op_name(op)
                ));
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
    if expression_is_real(lhs) || expression_is_real(rhs) {
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
                return Err(format!(
                    "operator {} not allowed on real operand",
                    binary_op_name(op)
                ));
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
    if expression_is_real(then_expr) || expression_is_real(else_expr) {
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
    // LRM Table 5-3: concatenation is illegal on reals. Detect it here
    // before `collect_concatenation_bits` would surface the less-helpful
    // "indefinite width" error from `is_indefinite_width`.
    for item in items {
        if expression_is_real(item) {
            return Err("concatenation operand cannot be real".to_string());
        }
    }
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
    if expression_is_real(count_expr) {
        return Err("replication count cannot be real".to_string());
    }
    for item in items {
        if expression_is_real(item) {
            return Err("replication operand cannot be real".to_string());
        }
    }
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
    // value set, not the floating-point one).
    if expression_is_real(arg) {
        return Err(format!(
            "{} argument cannot be real",
            if signed { "$signed" } else { "$unsigned" }
        ));
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
    if expression_is_real(arg) {
        return Err(format!(
            "{} argument cannot be real",
            base_cast_name(base)
        ));
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

// LRM 17.11: $clog2 returns the ceiling of log base 2 of the unsigned
// argument; $clog2(0) is defined to be 0.
//
// Argument typing follows the user's "implicit type conversion" rule:
//   - real argument: rounded to integer via §3.5.3 (round half away from
//     zero). NaN/±∞ has no integer image, so the result is 32 bits of x —
//     mirroring the $rtoi NaN/±∞ rule. Finite reals wrap mod 2^32 to
//     match $rtoi's "implicit integer data type is 32 bits signed"
//     behavior, then are interpreted as unsigned per LRM.
//   - integer argument: bits with x/z anywhere collapse the result to
//     32'sdx. The LRM is silent on x/z in $clog2; vcal surfaces "no
//     defined image" rather than silently mapping x/z → 0, matching the
//     $rtoi NaN/±∞ rule. The width used for the unsigned interpretation
//     is the operand's natural width, so $clog2(64'hFFFF…F) is 64.
fn evaluate_clog2(arg: &Expr, session: &Session) -> Result<IntegerValue, String> {
    if expression_is_real(arg) {
        let real_val = evaluate_expr_as_real(arg, session)?;
        let Some(bigint) = real_to_integer_bigint(real_val) else {
            return Ok(IntegerValue::all_x(32, true, Base::Decimal));
        };
        // Wrap to a 32-bit pattern, matching $rtoi's truncation domain;
        // the resulting IntegerValue's bits are then interpreted as the
        // unsigned 32-bit value the LRM requires.
        let wrapped = IntegerValue::from_bigint(bigint, 32, true, Base::Decimal);
        let unsigned = bits_to_biguint(&wrapped.bits);
        return Ok(clog2_result_value(unsigned));
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
fn evaluate_select(
    name: &str,
    kind: &SelectKind,
    session: &Session,
) -> Result<IntegerValue, String> {
    let reg = session
        .lookup(name)
        .ok_or_else(|| format!("undeclared identifier: {name}"))?;
    // LRM 5.2.1: "A bit-select or part-select of a scalar ... shall be
    // illegal." A reg declared without a range is a scalar even when
    // its width happens to be 1, distinct from the 1-bit vector
    // `reg [0:0] a` which does accept selects.
    let range = reg.range.as_ref().ok_or_else(|| {
        format!("bit-select or part-select on scalar reg `{name}` is illegal")
    })?;
    let base = reg.value.base;
    match kind {
        SelectKind::Bit { index } => evaluate_bit_select(reg, range, index, base, session),
        SelectKind::PartConst { msb, lsb } => {
            evaluate_part_const_select(reg, range, msb, lsb, base, session)
        }
        SelectKind::PartIndexedUp {
            base: base_expr,
            width,
        } => evaluate_part_indexed_select(reg, range, base_expr, width, base, session, true),
        SelectKind::PartIndexedDown {
            base: base_expr,
            width,
        } => evaluate_part_indexed_select(reg, range, base_expr, width, base, session, false),
    }
}

// LRM 4.2.1: a bit-select with an x/z anywhere in the index yields x;
// an out-of-range index also yields x. The index is self-determined and
// interpreted under its own signedness so negative-endpoint regs
// (e.g. `reg [-1:2]`) and signed-indexed selects line up.
fn evaluate_bit_select(
    reg: &RegValue,
    range: &RegRange,
    index: &Expr,
    result_base: Base,
    session: &Session,
) -> Result<IntegerValue, String> {
    if expression_is_real(index) {
        return Err("bit-select index cannot be real".to_string());
    }
    let index_value = evaluate_expr_in_context(index, None, session)?;
    if index_value.has_unknown_bits() {
        return Ok(IntegerValue::all_x(1, false, result_base));
    }
    let src_index = index_value.as_bigint(index_value.signed);
    let bit = match resolve_reg_index(range, &src_index) {
        Some(internal) => reg.value.bits[internal],
        None => LogicBit::X,
    };
    Ok(IntegerValue::computed(1, false, result_base, vec![bit]))
}

// LRM 5.2.1 `[msb:lsb]` part-select. The endpoints are runtime
// expressions in vcal (the LRM requires constants, but the REPL has no
// separate elaboration stage), and their direction must match the
// declared reg.
fn evaluate_part_const_select(
    reg: &RegValue,
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
    materialize_part_select(reg, range, &msb_sel, &lsb_sel, width, result_base)
}

// LRM 5.2.2 indexed part-select. `width` is a positive constant; `base`
// is a self-determined integer expression. The source range is always
// numerically [base, base+w-1] for `+:` and [base-w+1, base] for `-:`,
// independent of the declared reg direction; which end of that source
// range is the result's MSB depends on the reg's declared direction.
fn evaluate_part_indexed_select(
    reg: &RegValue,
    range: &RegRange,
    base_expr: &Expr,
    width_expr: &Expr,
    result_base: Base,
    session: &Session,
    is_up: bool,
) -> Result<IntegerValue, String> {
    let width = evaluate_indexed_select_width(width_expr, session)?;
    if expression_is_real(base_expr) {
        return Err("indexed part-select base cannot be real".to_string());
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
    materialize_part_select(reg, range, &msb_sel, &lsb_sel, width, result_base)
}

// Copies the bits of a part-select into the result, LSB-first. LRM
// 4.2.1's "out-of-range → x" rule is applied per position, so in-range
// source bits keep their value and only the out-of-range positions
// become x (e.g. `reg [3:0] a = 4'b0101; a[4:3]` → `2'bx0`).
fn materialize_part_select(
    reg: &RegValue,
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
            Some(internal) => reg.value.bits[internal],
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
    if expression_is_real(expr) {
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
    if expression_is_real(expr) {
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

// Same dispatch shape as `evaluate_select`, but only computes the
// result width — used by `infer_expr_meta` so the surrounding
// expression's two-pass width/sign inference matches what the
// materialised select will produce.
fn select_result_width(kind: &SelectKind, session: &Session) -> Result<usize, String> {
    match kind {
        SelectKind::Bit { .. } => Ok(1),
        SelectKind::PartConst { msb, lsb } => {
            let msb_sel = evaluate_constant_range_endpoint(msb, session, "msb")?;
            let lsb_sel = evaluate_constant_range_endpoint(lsb, session, "lsb")?;
            compute_select_width(&msb_sel, &lsb_sel)
        }
        SelectKind::PartIndexedUp { width, .. }
        | SelectKind::PartIndexedDown { width, .. } => {
            evaluate_indexed_select_width(width, session)
        }
    }
}
