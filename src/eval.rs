use num_bigint::{BigInt, BigUint, Sign};
use num_traits::{One, ToPrimitive, Zero};

use crate::parser::{BinaryOp, Expr, UnaryOp};
use crate::value::{
    Base, IntegerValue, LogicBit, bits_to_biguint, bitwise_and_bits, bitwise_not_bit,
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

pub(crate) fn evaluate_expr(expr: &Expr) -> Result<IntegerValue, String> {
    evaluate_expr_in_context(expr, None)
}

fn evaluate_expr_in_context(
    expr: &Expr,
    context: Option<ExprMeta>,
) -> Result<IntegerValue, String> {
    match expr {
        Expr::Literal(value) => Ok(match context {
            Some(context) => value.resized_to_context(context.width, context.signed),
            None => value.clone(),
        }),
        Expr::Grouped(expr) => evaluate_expr_in_context(expr, context),
        Expr::Unary { op, expr } => evaluate_unary_expr(*op, expr, context),
        Expr::Binary { op, lhs, rhs } => evaluate_binary_expr(*op, lhs, rhs, context),
        Expr::Conditional {
            cond,
            then_expr,
            else_expr,
        } => evaluate_conditional_expr(cond, then_expr, else_expr, context),
        Expr::Concatenation { items } => evaluate_concatenation_expr(items, context),
        Expr::Replication { count, items } => evaluate_replication_expr(count, items, context),
    }
}

fn infer_expr_meta(expr: &Expr) -> Result<ExprMeta, String> {
    match expr {
        Expr::Literal(value) => Ok(ExprMeta {
            width: value.width,
            signed: value.signed,
            base: value.base,
        }),
        Expr::Grouped(expr) => infer_expr_meta(expr),
        Expr::Unary { op, expr } => match op {
            UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitwiseNot => infer_expr_meta(expr),
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
            let lhs_meta = infer_expr_meta(lhs)?;
            let rhs_meta = infer_expr_meta(rhs)?;
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
            let then_meta = infer_expr_meta(then_expr)?;
            let else_meta = infer_expr_meta(else_expr)?;
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
                let item_meta = infer_expr_meta(item)?;
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
            let count = evaluate_replication_count_allow_zero(count)?;
            let mut inner_width = 0usize;
            let mut leftmost_base = Base::Binary;
            for (idx, item) in items.iter().enumerate() {
                let item_meta = infer_expr_meta(item)?;
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
) -> Result<IntegerValue, String> {
    if op == UnaryOp::LogicalNot {
        // LRM 5.4: logical operands are self-determined — evaluate without
        // pushing a context down, reduce to the operand's logical value, then
        // apply the !-truth table from §5.1.9.
        let operand = evaluate_expr_in_context(expr, None)?;
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
        let operand = evaluate_expr_in_context(expr, None)?;
        let bit = reduce_bits(op, &operand.bits);
        return Ok(widen_relational_result(
            comparison_result_value(bit),
            context,
        ));
    }

    let meta = infer_expr_meta(expr)?;
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
    let operand = evaluate_expr_in_context(expr, Some(effective_meta))?;

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

    let value = operand.as_bigint(effective_meta.signed);
    let result = match op {
        UnaryOp::Minus => -value,
        UnaryOp::Plus => unreachable!("handled before arithmetic evaluation"),
        UnaryOp::LogicalNot => unreachable!("handled by early-return path"),
        UnaryOp::BitwiseNot => unreachable!("handled by early-return path"),
        UnaryOp::ReductionAnd
        | UnaryOp::ReductionNand
        | UnaryOp::ReductionOr
        | UnaryOp::ReductionNor
        | UnaryOp::ReductionXor
        | UnaryOp::ReductionXnor => unreachable!("handled by early-return path"),
    };

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
) -> Result<IntegerValue, String> {
    let lhs_meta = infer_expr_meta(lhs)?;
    let rhs_meta = infer_expr_meta(rhs)?;

    if matches!(
        op,
        BinaryOp::LessThan
            | BinaryOp::GreaterThan
            | BinaryOp::LessThanOrEqual
            | BinaryOp::GreaterThanOrEqual
    ) {
        return evaluate_relational_expr(op, lhs, rhs, lhs_meta, rhs_meta, context);
    }

    if matches!(
        op,
        BinaryOp::Equal | BinaryOp::NotEqual | BinaryOp::CaseEqual | BinaryOp::CaseNotEqual
    ) {
        return evaluate_equality_expr(op, lhs, rhs, lhs_meta, rhs_meta, context);
    }

    if matches!(op, BinaryOp::LogicalAnd | BinaryOp::LogicalOr) {
        return evaluate_logical_expr(op, lhs, rhs, context);
    }

    if matches!(
        op,
        BinaryOp::LogicalShiftLeft
            | BinaryOp::LogicalShiftRight
            | BinaryOp::ArithmeticShiftLeft
            | BinaryOp::ArithmeticShiftRight
    ) {
        return evaluate_shift_expr(op, lhs, rhs, lhs_meta, context);
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
            let lhs_value = evaluate_expr_in_context(lhs, Some(effective_meta))?;
            let rhs_value = evaluate_expr_in_context(rhs, Some(effective_meta))?;

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
            let lhs_value = evaluate_expr_in_context(lhs, Some(lhs_context))?;
            let rhs_value = evaluate_expr_in_context(rhs, Some(rhs_meta))?;

            if lhs_value.has_unknown_bits() || rhs_value.has_unknown_bits() {
                return Ok(IntegerValue::all_x(
                    effective_meta.width,
                    lhs_meta.signed,
                    lhs_meta.base,
                ));
            }

            let base_value = lhs_value.as_bigint(lhs_meta.signed);
            let exponent_value = evaluate_expr_as_math_bigint(rhs)?;
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
            let lhs_value = evaluate_expr_in_context(lhs, Some(effective_meta))?;
            let rhs_value = evaluate_expr_in_context(rhs, Some(effective_meta))?;

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
) -> Result<IntegerValue, String> {
    let meta = ExprMeta {
        width: lhs_meta.width,
        signed: lhs_meta.signed,
        base: lhs_meta.base,
    };
    let effective_meta = ExprMeta {
        width: context.map_or(meta.width, |ctx| usize::max(ctx.width, meta.width)),
        signed: context.map_or(meta.signed, |ctx| ctx.signed),
        base: meta.base,
    };

    let lhs_value = evaluate_expr_in_context(lhs, Some(effective_meta))?;
    // RHS is self-determined: do NOT push effective_meta; let it evaluate at
    // its own width, then reinterpret its bits as unsigned for the count.
    let rhs_value = evaluate_expr_in_context(rhs, None)?;

    if rhs_value.has_unknown_bits() {
        return Ok(IntegerValue::all_x(
            effective_meta.width,
            effective_meta.signed,
            meta.base,
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
        meta.base,
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

    let lhs_value = evaluate_expr_in_context(lhs, Some(lhs_context))?;
    let rhs_value = evaluate_expr_in_context(rhs, Some(rhs_context))?;

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
    if value.bits.iter().any(|bit| *bit == LogicBit::One) {
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
) -> Result<IntegerValue, String> {
    // LRM 5.4: each operand is self-determined, so we evaluate them in
    // isolation rather than unifying widths the way relational/equality do.
    let lhs_logical = logical_value(&evaluate_expr_in_context(lhs, None)?);
    let rhs_logical = logical_value(&evaluate_expr_in_context(rhs, None)?);

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

fn evaluate_relational_expr(
    op: BinaryOp,
    lhs: &Expr,
    rhs: &Expr,
    lhs_meta: ExprMeta,
    rhs_meta: ExprMeta,
    context: Option<ExprMeta>,
) -> Result<IntegerValue, String> {
    let (lhs_value, rhs_value, comparison_signed) =
        unify_comparison_operands(lhs, rhs, lhs_meta, rhs_meta)?;

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
) -> Result<IntegerValue, String> {
    // Bit-level comparison; the unified signedness only matters for operand
    // extension (already done inside `unify_comparison_operands`), not for the
    // comparison itself.
    let (lhs_value, rhs_value, _comparison_signed) =
        unify_comparison_operands(lhs, rhs, lhs_meta, rhs_meta)?;

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
) -> Result<IntegerValue, String> {
    let then_meta = infer_expr_meta(then_expr)?;
    let else_meta = infer_expr_meta(else_expr)?;
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

    let cond_value = evaluate_expr_in_context(cond, None)?;
    let cond_logical = logical_value(&cond_value);

    let then_value = evaluate_expr_in_context(then_expr, Some(effective_meta))?;
    let else_value = evaluate_expr_in_context(else_expr, Some(effective_meta))?;

    let bits = match cond_logical {
        LogicBit::One => then_value.bits,
        LogicBit::Zero => else_value.bits,
        LogicBit::X | LogicBit::Z => then_value
            .bits
            .iter()
            .zip(else_value.bits.iter())
            .map(|(t, e)| if t == e { *t } else { LogicBit::X })
            .collect(),
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
fn evaluate_replication_count_allow_zero(count_expr: &Expr) -> Result<usize, String> {
    let value = evaluate_expr_in_context(count_expr, None)?;
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
fn evaluate_replication_count(count_expr: &Expr) -> Result<usize, String> {
    let count = evaluate_replication_count_allow_zero(count_expr)?;
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
) -> Result<IntegerValue, String> {
    let bits = collect_concatenation_bits(items)?;
    let leftmost_base = infer_expr_meta(&items[0])?.base;
    let natural_width = bits.len();
    let result = IntegerValue::computed(natural_width, false, leftmost_base, bits);
    Ok(extend_to_outer_context(result, context))
}

fn evaluate_replication_expr(
    count_expr: &Expr,
    items: &[Expr],
    context: Option<ExprMeta>,
) -> Result<IntegerValue, String> {
    let count = evaluate_replication_count(count_expr)?;
    let inner_bits = collect_concatenation_bits(items)?;
    let leftmost_base = infer_expr_meta(&items[0])?.base;

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
fn collect_concatenation_bits(items: &[Expr]) -> Result<Vec<LogicBit>, String> {
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
        bits.extend(evaluate_concatenation_item_bits(item)?);
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

fn evaluate_concatenation_item_bits(item: &Expr) -> Result<Vec<LogicBit>, String> {
    if let Expr::Replication { count, items } = unwrap_grouped(item) {
        let count = evaluate_replication_count_allow_zero(count)?;
        if count == 0 {
            return Ok(Vec::new());
        }
        let inner_bits = collect_concatenation_bits(items)?;
        let mut bits = Vec::with_capacity(inner_bits.len().saturating_mul(count));
        for _ in 0..count {
            bits.extend(inner_bits.iter().copied());
        }
        return Ok(bits);
    }
    let value = evaluate_expr_in_context(item, None)?;
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

fn widen_relational_result(result: IntegerValue, context: Option<ExprMeta>) -> IntegerValue {
    match context {
        Some(ctx) if ctx.width > 1 => result.resized_to_context(ctx.width, false),
        _ => result,
    }
}

fn evaluate_expr_as_math_bigint(expr: &Expr) -> Result<BigInt, String> {
    match expr {
        Expr::Literal(value) => {
            if value.has_unknown_bits() {
                return Err("expression contains unknown bits".to_string());
            }

            Ok(value.as_bigint(value.signed))
        }
        Expr::Grouped(expr) => evaluate_expr_as_math_bigint(expr),
        Expr::Unary { op, expr } => {
            if matches!(op, UnaryOp::LogicalNot) || is_reduction_op(*op) {
                // Both produce a 1-bit unsigned result via the same
                // early-return shape, so a single bigint conversion works.
                let value = evaluate_unary_expr(*op, expr, None)?;
                if value.has_unknown_bits() {
                    return Err("expression contains unknown bits".to_string());
                }
                return Ok(value.as_bigint(false));
            }
            if matches!(op, UnaryOp::BitwiseNot) {
                // Bitwise NOT depends on operand width, so go through
                // evaluate_unary_expr rather than negating in bigint.
                let value = evaluate_unary_expr(*op, expr, None)?;
                if value.has_unknown_bits() {
                    return Err("expression contains unknown bits".to_string());
                }
                return Ok(value.as_bigint(value.signed));
            }
            let value = evaluate_expr_as_math_bigint(expr)?;
            Ok(match op {
                UnaryOp::Plus => value,
                UnaryOp::Minus => -value,
                UnaryOp::LogicalNot => unreachable!("handled above"),
                UnaryOp::BitwiseNot => unreachable!("handled above"),
                UnaryOp::ReductionAnd
                | UnaryOp::ReductionNand
                | UnaryOp::ReductionOr
                | UnaryOp::ReductionNor
                | UnaryOp::ReductionXor
                | UnaryOp::ReductionXnor => unreachable!("handled above"),
            })
        }
        Expr::Binary { op, lhs, rhs } => {
            if matches!(
                op,
                BinaryOp::LessThan
                    | BinaryOp::GreaterThan
                    | BinaryOp::LessThanOrEqual
                    | BinaryOp::GreaterThanOrEqual
            ) {
                let lhs_meta = infer_expr_meta(lhs)?;
                let rhs_meta = infer_expr_meta(rhs)?;
                let value = evaluate_relational_expr(*op, lhs, rhs, lhs_meta, rhs_meta, None)?;
                if value.has_unknown_bits() {
                    return Err("expression contains unknown bits".to_string());
                }
                return Ok(value.as_bigint(false));
            }

            if matches!(
                op,
                BinaryOp::Equal
                    | BinaryOp::NotEqual
                    | BinaryOp::CaseEqual
                    | BinaryOp::CaseNotEqual
            ) {
                let lhs_meta = infer_expr_meta(lhs)?;
                let rhs_meta = infer_expr_meta(rhs)?;
                let value = evaluate_equality_expr(*op, lhs, rhs, lhs_meta, rhs_meta, None)?;
                if value.has_unknown_bits() {
                    return Err("expression contains unknown bits".to_string());
                }
                return Ok(value.as_bigint(false));
            }

            if matches!(op, BinaryOp::LogicalAnd | BinaryOp::LogicalOr) {
                let value = evaluate_logical_expr(*op, lhs, rhs, None)?;
                if value.has_unknown_bits() {
                    return Err("expression contains unknown bits".to_string());
                }
                return Ok(value.as_bigint(false));
            }

            if matches!(
                op,
                BinaryOp::BitwiseAnd
                    | BinaryOp::BitwiseOr
                    | BinaryOp::BitwiseXor
                    | BinaryOp::BitwiseXnor
            ) {
                // Bitwise binaries depend on the unified operand width, so
                // evaluate them through the normal pipeline rather than
                // applying bigint operators directly.
                let value = evaluate_binary_expr(*op, lhs, rhs, None)?;
                if value.has_unknown_bits() {
                    return Err("expression contains unknown bits".to_string());
                }
                return Ok(value.as_bigint(value.signed));
            }

            if matches!(
                op,
                BinaryOp::LogicalShiftLeft
                    | BinaryOp::LogicalShiftRight
                    | BinaryOp::ArithmeticShiftLeft
                    | BinaryOp::ArithmeticShiftRight
            ) {
                // Same reasoning as bitwise: shifts depend on the LHS width
                // and signedness, so route through the standard pipeline
                // rather than reaching into bigint shift operators directly.
                let value = evaluate_binary_expr(*op, lhs, rhs, None)?;
                if value.has_unknown_bits() {
                    return Err("expression contains unknown bits".to_string());
                }
                return Ok(value.as_bigint(value.signed));
            }

            let lhs_value = evaluate_expr_as_math_bigint(lhs)?;
            let rhs_value = evaluate_expr_as_math_bigint(rhs)?;

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
                BinaryOp::LessThan
                | BinaryOp::GreaterThan
                | BinaryOp::LessThanOrEqual
                | BinaryOp::GreaterThanOrEqual
                | BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::CaseEqual
                | BinaryOp::CaseNotEqual
                | BinaryOp::LogicalAnd
                | BinaryOp::LogicalOr
                | BinaryOp::BitwiseAnd
                | BinaryOp::BitwiseOr
                | BinaryOp::BitwiseXor
                | BinaryOp::BitwiseXnor
                | BinaryOp::LogicalShiftLeft
                | BinaryOp::LogicalShiftRight
                | BinaryOp::ArithmeticShiftLeft
                | BinaryOp::ArithmeticShiftRight => unreachable!("handled above"),
            }
        }
        // The result depends on width-aware extension of then/else and on
        // a per-bit merge under an x/z cond, so route through the standard
        // pipeline rather than reaching into bigint directly.
        Expr::Conditional { .. } => {
            let value = evaluate_expr_in_context(expr, None)?;
            if value.has_unknown_bits() {
                return Err("expression contains unknown bits".to_string());
            }
            Ok(value.as_bigint(value.signed))
        }
        // Concatenation/replication results are bit-pattern values — read
        // them as unsigned integers (LRM 5.1.14: "The result of a
        // concatenation is treated as an unsigned vector").
        Expr::Concatenation { .. } | Expr::Replication { .. } => {
            let value = evaluate_expr_in_context(expr, None)?;
            if value.has_unknown_bits() {
                return Err("expression contains unknown bits".to_string());
            }
            Ok(value.as_bigint(false))
        }
    }
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
            let is_odd = (&(-exponent.clone()) & BigInt::one()) == BigInt::one();
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
        if (&remaining & BigUint::one()) == BigUint::one() {
            result *= &factor;
        }

        remaining >>= 1;
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
