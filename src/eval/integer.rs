//! Integer expression scheduling and result combination.

use num_bigint::{BigInt, BigUint, Sign};
use num_traits::{ToPrimitive, Zero};

use super::ir::{Annotated, AnnotatedKind, ExprMeta, ResolvedSelectKind, unwrap_grouped_annotated};
use super::machine::{EvalTask, push_visit_as_real};
use super::ops::{
    apply_logical_truth_table, binary_op_name, clog2_result_value, combine_binary_meta,
    comparison_result_value, compute_equality_from_values, compute_relational_from_values,
    concatenated_display_style, evaluate_power, extend_cast_to_outer_context,
    extend_to_outer_context, logical_value, logical_value_of_real, real_to_integer_value,
    reduce_bits, shift_bits_left, shift_bits_right, unary_op_name, widen_relational_result,
};
use super::select::{evaluate_resolved_select, push_select_eval};

use crate::parser::{
    BinaryOp, Expr, MathFunctionKind, RealConversionKind, UnaryOp, string_literal_spec,
};
use crate::system_call::{SystemCallKind, classify_system_call, task_in_expression_error};
use crate::value::{
    Base, DisplayStyle, IntegerValue, LogicBit, Value, bits_to_biguint, bitwise_and_bits,
    bitwise_not_bit, bitwise_or_bits, bitwise_xnor_bits, bitwise_xor_bits,
};
use crate::{Session, value};

// Schedule evaluation of one concatenation operand. The LRM 5.1.14
// "lenient zero-rep" rule — a Replication directly inside a concat list
// may have count = 0 and contribute no bits — is implemented by peeling
// any leading Grouped and routing a Replication item to the same
// `ReplicationCountReceived` combiner the top-level Replication uses,
// but with `strict = false` so a zero count is a no-op rather than an
// error. Non-Replication items take the normal Visit path with `ctx =
// None` (concat operands are self-determined per LRM 5.1.14).
fn push_concat_item_eval<'b, 'a: 'b>(item: &'b Annotated<'a>, work: &mut Vec<EvalTask<'b, 'a>>) {
    let unwrapped = unwrap_grouped_annotated(item);
    if let AnnotatedKind::Replication { count, items } = &unwrapped.kind {
        let leftmost_base = items[0].meta().base;
        work.push(EvalTask::Combine(EvalCombiner::ReplicationCountReceived {
            items,
            leftmost_base,
            ctx: None,
            strict: false,
        }));
        work.push(EvalTask::Visit {
            node: count,
            ctx: None,
        });
    } else {
        work.push(EvalTask::Visit {
            node: item,
            ctx: None,
        });
    }
}

// Leaf-shape integer evaluator. Reached only via `visit_eval`'s
// `AnnotatedKind::Leaf` arm for literals and identifiers. All
// other Expr shapes go through dedicated `AnnotatedKind` arms in the
// iterative CES driver, never through here — they can't appear as a leaf in
// the annotated tree. Surface an `unreachable!` on anything else so a future
// regression that wires a non-leaf shape into the Leaf path fails loudly
// instead of silently re-introducing a recursive per-shape evaluator.
fn evaluate_leaf_expr_in_context(
    expr: &Expr,
    context: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    match expr {
        Expr::Literal(spec) => {
            // First materialization site — the validator has already capped
            // `spec.width` against MAX_BIT_WIDTH, so this allocation is
            // bounded. Resize-to-context follows whether the literal sits in
            // a propagated width/signedness or stands alone (None).
            let value = spec.materialize();
            Ok(match context {
                Some(context) => value.resized_to_context(context.width, context.signed),
                None => value,
            })
        }
        Expr::StringLiteral(bytes) => {
            let value = string_literal_spec(bytes)
                .materialize()
                .with_display_style(DisplayStyle::String);
            Ok(match context {
                Some(context) => value.resized_to_context(context.width, context.signed),
                None => value,
            })
        }
        // Reaching the integer pipeline with a real-typed expression at
        // the top means our dispatch missed a real-result case. Surface
        // an error rather than silently fabricating an integer.
        Expr::RealLiteral(_) => {
            Err("real value cannot be used as an integer expression here".to_string())
        }
        // System-call leaves only arrive here for the task case
        // (`AnnotatedKind::SystemTask`). The annotate pass routes math
        // / cast / real-conversion calls through their own typed
        // `AnnotatedKind` arms, so a non-task `Expr::SystemCall`
        // reaching here would be a dispatch bug.
        Expr::SystemCall { name, .. } => {
            debug_assert!(matches!(
                classify_system_call(name),
                Ok(SystemCallKind::Task(_))
            ));
            Err(task_in_expression_error(name))
        }
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
        Expr::Select { .. }
        | Expr::ConcatSelect { .. }
        | Expr::Grouped(_)
        | Expr::Unary { .. }
        | Expr::Binary { .. }
        | Expr::Conditional { .. }
        | Expr::Concatenation { .. }
        | Expr::Replication { .. } => unreachable!(
            "evaluate_leaf_expr_in_context only handles AnnotatedKind::Leaf shapes; \
             {expr:?} should have its own AnnotatedKind arm in visit_eval"
        ),
        Expr::Truncated => unreachable!(
            "Expr::Truncated is a display-only sentinel; never reaches evaluate_leaf_expr_in_context"
        ),
    }
}

pub(super) enum EvalCombiner<'b, 'a: 'b> {
    Select {
        node: &'b Annotated<'a>,
        ctx: Option<ExprMeta>,
    },
    /// Add / Subtract / Multiply / Divide / Modulus on integer operands.
    /// Pops 2 values (lhs, rhs).
    BinaryArith {
        op: BinaryOp,
        effective_meta: ExprMeta,
        meta: ExprMeta,
    },
    /// BitwiseAnd / Or / Xor / Xnor on integer operands. Pops 2 values.
    BinaryBitwise {
        op: BinaryOp,
        effective_meta: ExprMeta,
    },
    /// Integer power. Pops 2 values (lhs, rhs); the RHS is scheduled through
    /// the same work stack with no propagated context, then applied with
    /// width-bounded modular exponentiation. The base is interpreted under
    /// `effective_meta`'s signedness — the same one the visitor extended it
    /// with — and `effective_meta.base` is the LHS's display base, since
    /// power takes both from the LHS alone.
    BinaryPower {
        effective_meta: ExprMeta,
    },
    /// Less / Greater / LessOrEq / GreaterOrEq on integers. Pops 2 values
    /// already extended to the unified comparison context.
    BinaryRelational {
        op: BinaryOp,
        signed: bool,
        ctx: Option<ExprMeta>,
    },
    /// Equal / NotEqual / CaseEqual / CaseNotEqual on integers. Pops 2.
    BinaryEquality {
        op: BinaryOp,
        ctx: Option<ExprMeta>,
    },
    /// LogicalAnd / LogicalOr on integers. Pops 2 self-determined values.
    BinaryLogical {
        op: BinaryOp,
        ctx: Option<ExprMeta>,
    },
    /// Shifts. Pops 2 values: lhs already extended to effective_meta, rhs
    /// self-determined (treated as unsigned regardless of declared sign).
    BinaryShift {
        op: BinaryOp,
        effective_meta: ExprMeta,
        lhs_base: Base,
    },
    /// Unary integer ops. Pops 1 value. The variant captures everything
    /// needed to compute the result without re-walking the subtree.
    UnaryArith {
        op: UnaryOp,
        effective_meta: ExprMeta,
        base: Base,
    },
    UnaryLogicalNot {
        ctx: Option<ExprMeta>,
    },
    UnaryReduction {
        op: UnaryOp,
        ctx: Option<ExprMeta>,
    },
    /// Cond evaluated → choose branch. May push one Visit (definite cond)
    /// or two Visits + ConditionalMerge (x/z cond). Pops 1 value (cond).
    ConditionalChoose {
        then_arm: &'b Annotated<'a>,
        else_arm: &'b Annotated<'a>,
        effective_meta: ExprMeta,
        result_signed: bool,
        result_base: Base,
    },
    /// Per-bit merge of then / else values when cond was x/z. Pops 2.
    ConditionalMerge {
        effective_meta: ExprMeta,
        result_signed: bool,
        result_base: Base,
    },
    /// Re-stamps the chosen branch's value with the conditional's
    /// effective signedness/base. Needed because a leaf-typed branch
    /// (e.g., `4'sd1` chosen from `1 ? 4'sd1 : 4'd1`) carries its own
    /// signedness through `resized_to_context` when widths match — the
    /// conditional's unified signedness (LRM 5.5.1) has to be applied
    /// at the conditional level, not the branch.
    ConditionalFinalize {
        effective_meta: ExprMeta,
        result_signed: bool,
        result_base: Base,
    },
    /// $signed / $unsigned. Pops 1 self-determined value. The Combine
    /// re-stamps the value's signedness and applies outer-context
    /// extension following the propagated signedness (§5.5.2).
    SignCast {
        signed: bool,
        ctx: Option<ExprMeta>,
    },
    /// $bin / $oct / $dec / $hex. Pops 1 self-determined value. Replaces
    /// the display base; outer-context extension follows propagated
    /// signedness.
    BaseCast {
        base: Base,
        ctx: Option<ExprMeta>,
    },
    /// LRM 5.1.14 concatenation. Pops `item_count` already-evaluated
    /// item values (in source order on the value stack: items[0] at the
    /// bottom, items[N-1] on top). Joins their bits MSB-first → LSB-last
    /// to build an unsigned natural-width result, then extends to the
    /// outer ctx if the ctx is wider. Emits the "must have at least one
    /// operand with positive size" error when every popped item
    /// contributes zero bits (only possible when every item is a zero-
    /// count Replication).
    Concatenation {
        item_count: usize,
        leftmost_base: Base,
        ctx: Option<ExprMeta>,
    },
    /// First half of a Replication evaluation: the count expression has
    /// just been evaluated and sits on top of the value stack. Validates
    /// the count (unknown bits, sign, fits-in-usize, strict-positive)
    /// and either short-circuits (count = 0 in lenient/inside-concat
    /// position → push a zero-bit value) or pushes a `ReplicationFinalize`
    /// Combine plus per-item Visits to evaluate the inner items. Strict
    /// mode is used at the top level (`{N{...}}` with no surrounding
    /// concat) where LRM 5.1.14 forbids count = 0; lenient mode is used
    /// when a Replication appears as a concatenation operand.
    ReplicationCountReceived {
        items: &'b [Annotated<'a>],
        leftmost_base: Base,
        ctx: Option<ExprMeta>,
        strict: bool,
    },
    /// Final pass of a Replication: the inner items have been evaluated
    /// (`item_count` IntegerValues on top of the stack). Joins them into
    /// `inner_bits` and replicates `count` times. In strict mode (top-
    /// level) an empty `inner_bits` is rejected with the same diagnostic
    /// the concatenation combine emits; in lenient mode
    /// (inside a concat) the result is allowed to be zero-width.
    ReplicationFinalize {
        item_count: usize,
        count: usize,
        leftmost_base: Base,
        ctx: Option<ExprMeta>,
        strict: bool,
    },
    /// `$rtoi` / `$realtobits`: real-result-typed argument has just been
    /// evaluated and sits on `real_vals`. Pops 1 f64; produces 1
    /// IntegerValue per LRM 17.7.1 / 17.8 then applies outer-context
    /// extension.
    RealConversionToInt {
        kind: RealConversionKind,
        ctx: Option<ExprMeta>,
    },
    /// `$clog2`: integer-typed argument has just been evaluated. Pops 1
    /// IntegerValue, applies the LRM 17.11.1 ceiling-log; outer-context
    /// extension follows. Inlines the legacy `evaluate_clog2` body so
    /// `$clog2($clog2(...))` no longer falls through to the recursive
    /// walker.
    Clog2 {
        ctx: Option<ExprMeta>,
    },
    /// `!real_operand`: pops 1 f64 from `real_vals`, applies LRM 5.1.9
    /// logical-NOT to the f64's reduced logical value, widens to the
    /// outer context per `widen_relational_result`.
    UnaryLogicalNotReal {
        ctx: Option<ExprMeta>,
    },
    /// Relational comparison with at least one real operand. Both
    /// operands have been evaluated to `real_vals` (with implicit
    /// LRM 3.5.3 coercion via `RealCombiner::CoerceFromInteger` if the
    /// raw operand was integer-typed). Pops 2 f64s; produces a 1-bit
    /// IntegerValue.
    BinaryRealRelational {
        op: BinaryOp,
        ctx: Option<ExprMeta>,
    },
    /// `==` / `!=` on real operands. IEEE 754 unordered semantics — both
    /// false for `==` and true for `!=` when either operand is NaN.
    /// Pops 2 f64s; produces 1-bit.
    BinaryRealEquality {
        op: BinaryOp,
        ctx: Option<ExprMeta>,
    },
    /// `&&` / `||` with at least one real operand. LRM 5.1.9 truth table
    /// after each operand reduces via `logical_value_of_real`. NaN → x.
    BinaryRealLogical {
        op: BinaryOp,
        ctx: Option<ExprMeta>,
    },
    /// `&&` / `||` where exactly one operand is real and the other is
    /// integer. The real operand comes off `real_vals` and the integer
    /// operand off `vals`, each reduced in its own type so the integer
    /// side's x/z bits survive instead of being flattened to 0.0 by the
    /// LRM 3.5.3 real conversion.
    BinaryRealLogicalMixed {
        op: BinaryOp,
        ctx: Option<ExprMeta>,
        lhs_is_real: bool,
    },
    /// Real-typed `?:` cond on an integer-result conditional. Pops 1
    /// f64 (cond) and dispatches: definite cond pushes
    /// `Visit(chosen, ctx)` + `ConditionalFinalize`; x/z cond pushes
    /// both branches + `ConditionalMerge`.
    ConditionalChooseRealCond {
        then_arm: &'b Annotated<'a>,
        else_arm: &'b Annotated<'a>,
        effective_meta: ExprMeta,
        result_signed: bool,
        result_base: Base,
    },
}

pub(super) fn visit_eval<'b, 'a: 'b>(
    node: &'b Annotated<'a>,
    ctx: Option<ExprMeta>,
    work: &mut Vec<EvalTask<'b, 'a>>,
    vals: &mut Vec<IntegerValue>,
    real_vals: &mut Vec<f64>,
    session: &Session,
) -> Result<(), String> {
    match &node.kind {
        AnnotatedKind::Select(select) => {
            if let Some(Value::Integer(value)) = select.value.get() {
                vals.push(match ctx {
                    Some(ctx) => value.resized_to_context(ctx.width, ctx.signed),
                    None => value.clone(),
                });
            } else {
                push_select_eval(node, select, ctx, work);
            }
        }
        // Grouped is transparent in semantics — visit the inner
        // annotation with the same context.
        AnnotatedKind::Grouped(inner) => work.push(EvalTask::Visit { node: inner, ctx }),
        AnnotatedKind::Binary { lhs, rhs } => {
            let op = match node.expr {
                Expr::Binary { op, .. } => *op,
                _ => unreachable!("AnnotatedKind::Binary only wraps Expr::Binary"),
            };
            visit_binary_eval(op, lhs, rhs, ctx, work, vals, real_vals, session)?;
        }
        AnnotatedKind::Unary(operand) => {
            let op = match node.expr {
                Expr::Unary { op, .. } => *op,
                _ => unreachable!("AnnotatedKind::Unary only wraps Expr::Unary"),
            };
            visit_unary_eval(op, operand, ctx, work, vals, real_vals, session)?;
        }
        AnnotatedKind::Conditional {
            cond,
            then_arm,
            else_arm,
        } => {
            visit_conditional_eval(
                cond, then_arm, else_arm, ctx, work, vals, real_vals, session,
            )?;
        }
        AnnotatedKind::SignCast { signed, arg } => {
            // LRM 5.5: argument is evaluated self-determined; the cast
            // re-stamps signedness; the result is then extended to the
            // outer context (extension follows propagated signedness per
            // §5.5.2). Visit arg with ctx=None; Combine does the rest.
            work.push(EvalTask::Combine(EvalCombiner::SignCast {
                signed: *signed,
                ctx,
            }));
            work.push(EvalTask::Visit {
                node: arg,
                ctx: None,
            });
        }
        AnnotatedKind::BaseCast { base, arg } => {
            // Same shape as SignCast: arg self-determined, base re-stamped,
            // outer-context extended at combine time.
            work.push(EvalTask::Combine(EvalCombiner::BaseCast {
                base: *base,
                ctx,
            }));
            work.push(EvalTask::Visit {
                node: arg,
                ctx: None,
            });
        }
        // LRM 5.1.14 concatenation: each operand is self-determined; the
        // joined width comes from summing their widths; the result is
        // unsigned and uses the leftmost item's base. Items are pushed
        // in reverse so items[0] visits first and lands at the bottom of
        // the value stack — the Combine drains them in source order.
        // Replication-inside-concat is dispatched lenient (count = 0 is
        // a no-op contributing zero bits) via `push_concat_item_eval`.
        AnnotatedKind::Concatenation(items) => {
            // `meta()` is the cached LRM 5.1.14 meta computed in
            // `annotate`'s Combine; reading items[0]'s base is O(1) and
            // replaces a per-call subtree re-walk for the leftmost item.
            let leftmost_base = items[0].meta().base;
            work.push(EvalTask::Combine(EvalCombiner::Concatenation {
                item_count: items.len(),
                leftmost_base,
                ctx,
            }));
            for item in items.iter().rev() {
                push_concat_item_eval(item, work);
            }
        }
        // Top-level Replication (`{N{a, b, ...}}` not inside a surrounding
        // concat). Strict count: zero is rejected. The count is evaluated
        // first via the same iterative driver — its IntegerValue lands on
        // the value stack and `ReplicationCountReceived` pops it.
        AnnotatedKind::Replication { count, items } => {
            let leftmost_base = items[0].meta().base;
            work.push(EvalTask::Combine(EvalCombiner::ReplicationCountReceived {
                items,
                leftmost_base,
                ctx,
                strict: true,
            }));
            work.push(EvalTask::Visit {
                node: count,
                ctx: None,
            });
        }
        // `$rtoi` / `$realtobits`: argument is real-typed (validator
        // rule). Push the bridge combiner that converts the popped f64
        // to an IntegerValue per LRM 17.7.1 / 17.8, then queue the
        // argument as a real-side visit so its result lands on
        // `real_vals` ready for the bridge to pop. Iterative across
        // any depth — alternating `$rtoi($itor(...))` is the case the
        // unified driver was added to handle.
        AnnotatedKind::RealConversion { kind, arg } => match kind {
            RealConversionKind::RealToInteger | RealConversionKind::RealToBits => {
                work.push(EvalTask::Combine(EvalCombiner::RealConversionToInt {
                    kind: *kind,
                    ctx,
                }));
                push_visit_as_real(arg, work);
            }
            RealConversionKind::IntegerToReal | RealConversionKind::BitsToReal => {
                unreachable!(
                    "$itor / $bitstoreal are real-result; integer driver should not see them"
                );
            }
        },
        // Math function: the only integer-result kind is `$clog2`. Real-
        // result math functions (Pow, Atan2, ..., real-arity-1 family)
        // are handled by the real driver, never by `visit_eval`.
        AnnotatedKind::MathFunction { kind, args } => match kind {
            MathFunctionKind::Clog2 => {
                debug_assert_eq!(args.len(), 1, "$clog2 has arity 1");
                work.push(EvalTask::Combine(EvalCombiner::Clog2 { ctx }));
                work.push(EvalTask::Visit {
                    node: &args[0],
                    ctx: None,
                });
            }
            _ => unreachable!("real-result math functions handled by evaluate_annotated_as_real"),
        },
        AnnotatedKind::SystemTask => {
            let name = match node.expr {
                Expr::SystemCall { name, .. } => name.as_str(),
                _ => unreachable!("AnnotatedKind::SystemTask wraps only Expr::SystemCall"),
            };
            return Err(task_in_expression_error(name));
        }
        // Integer literals, string literals, and identifiers have no
        // children and use `evaluate_leaf_expr_in_context`. Real literals
        // go through `visit_real_eval`; selects and system tasks have
        // dedicated annotation kinds above.
        AnnotatedKind::Leaf => {
            vals.push(evaluate_leaf_expr_in_context(node.expr, ctx, session)?);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn visit_binary_eval<'b, 'a: 'b>(
    op: BinaryOp,
    lhs: &'b Annotated<'a>,
    rhs: &'b Annotated<'a>,
    ctx: Option<ExprMeta>,
    work: &mut Vec<EvalTask<'b, 'a>>,
    _vals: &mut Vec<IntegerValue>,
    _real_vals: &mut Vec<f64>,
    _session: &Session,
) -> Result<(), String> {
    // LRM Table 5-3 dispatch for real-typed operands. Relational /
    // equality / logical produce an integer result from real(s) — push
    // a bridge combiner that pops 2 reals from `real_vals` and produces
    // the 1-bit IntegerValue, with each operand visited via
    // `push_visit_as_real` so an integer subtree (e.g. `1.0 < 1`) gets
    // implicit LRM 3.5.3 coercion through `RealCombiner::CoerceFromInteger`.
    if lhs.is_real() || rhs.is_real() {
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
                unreachable!(
                    "validator rejects real operand of {} before evaluation",
                    binary_op_name(op)
                );
            }
            BinaryOp::LessThan
            | BinaryOp::GreaterThan
            | BinaryOp::LessThanOrEqual
            | BinaryOp::GreaterThanOrEqual => {
                work.push(EvalTask::Combine(EvalCombiner::BinaryRealRelational {
                    op,
                    ctx,
                }));
                push_visit_as_real(rhs, work);
                push_visit_as_real(lhs, work);
                return Ok(());
            }
            BinaryOp::Equal | BinaryOp::NotEqual => {
                work.push(EvalTask::Combine(EvalCombiner::BinaryRealEquality {
                    op,
                    ctx,
                }));
                push_visit_as_real(rhs, work);
                push_visit_as_real(lhs, work);
                return Ok(());
            }
            BinaryOp::LogicalAnd | BinaryOp::LogicalOr => {
                if lhs.is_real() == rhs.is_real() {
                    work.push(EvalTask::Combine(EvalCombiner::BinaryRealLogical {
                        op,
                        ctx,
                    }));
                    push_visit_as_real(rhs, work);
                    push_visit_as_real(lhs, work);
                    return Ok(());
                }
                // Exactly one operand is real: each side is reduced in its own
                // type. Coercing the integer side to real first would flatten
                // its x/z bits to 0.0 (LRM 3.5.3) and answer 0 where the LRM
                // 5.1.9 truth table says x — `1'bx && 1.0` is x, not 0.
                work.push(EvalTask::Combine(EvalCombiner::BinaryRealLogicalMixed {
                    op,
                    ctx,
                    lhs_is_real: lhs.is_real(),
                }));
                if rhs.is_real() {
                    push_visit_as_real(rhs, work);
                } else {
                    work.push(EvalTask::Visit {
                        node: rhs,
                        ctx: None,
                    });
                }
                if lhs.is_real() {
                    push_visit_as_real(lhs, work);
                } else {
                    work.push(EvalTask::Visit {
                        node: lhs,
                        ctx: None,
                    });
                }
                return Ok(());
            }
        }
    }

    let lhs_meta = lhs.meta();
    let rhs_meta = rhs.meta();

    // Relational, equality, logical, shift: each has a distinct context-
    // propagation rule for lhs/rhs, encoded below.
    if matches!(
        op,
        BinaryOp::LessThan
            | BinaryOp::GreaterThan
            | BinaryOp::LessThanOrEqual
            | BinaryOp::GreaterThanOrEqual
    ) {
        // LRM 5.5.2: relational operands form a shared context — width =
        // max, signed iff both signed. The unified sign drives leaf-level
        // extension; the comparison itself follows the unified sign.
        let unified_width = usize::max(lhs_meta.width, rhs_meta.width);
        let signed = lhs_meta.signed && rhs_meta.signed;
        let lhs_ctx = ExprMeta {
            width: unified_width,
            signed,
            base: lhs_meta.base,
        };
        let rhs_ctx = ExprMeta {
            width: unified_width,
            signed,
            base: rhs_meta.base,
        };
        work.push(EvalTask::Combine(EvalCombiner::BinaryRelational {
            op,
            signed,
            ctx,
        }));
        work.push(EvalTask::Visit {
            node: rhs,
            ctx: Some(rhs_ctx),
        });
        work.push(EvalTask::Visit {
            node: lhs,
            ctx: Some(lhs_ctx),
        });
        return Ok(());
    }

    if matches!(
        op,
        BinaryOp::Equal | BinaryOp::NotEqual | BinaryOp::CaseEqual | BinaryOp::CaseNotEqual
    ) {
        let unified_width = usize::max(lhs_meta.width, rhs_meta.width);
        let signed = lhs_meta.signed && rhs_meta.signed;
        let lhs_ctx = ExprMeta {
            width: unified_width,
            signed,
            base: lhs_meta.base,
        };
        let rhs_ctx = ExprMeta {
            width: unified_width,
            signed,
            base: rhs_meta.base,
        };
        work.push(EvalTask::Combine(EvalCombiner::BinaryEquality { op, ctx }));
        work.push(EvalTask::Visit {
            node: rhs,
            ctx: Some(rhs_ctx),
        });
        work.push(EvalTask::Visit {
            node: lhs,
            ctx: Some(lhs_ctx),
        });
        return Ok(());
    }

    if matches!(op, BinaryOp::LogicalAnd | BinaryOp::LogicalOr) {
        // LRM 5.4: each operand is self-determined. No context propagation.
        work.push(EvalTask::Combine(EvalCombiner::BinaryLogical { op, ctx }));
        work.push(EvalTask::Visit {
            node: rhs,
            ctx: None,
        });
        work.push(EvalTask::Visit {
            node: lhs,
            ctx: None,
        });
        return Ok(());
    }

    if matches!(
        op,
        BinaryOp::LogicalShiftLeft
            | BinaryOp::LogicalShiftRight
            | BinaryOp::ArithmeticShiftLeft
            | BinaryOp::ArithmeticShiftRight
    ) {
        // LRM 5.1.12: lhs is context-determined (max(L(lhs), L(ctx)));
        // rhs is self-determined.
        let effective_meta = ExprMeta {
            width: ctx.map_or(lhs_meta.width, |c| usize::max(c.width, lhs_meta.width)),
            signed: ctx.map_or(lhs_meta.signed, |c| c.signed),
            base: lhs_meta.base,
        };
        work.push(EvalTask::Combine(EvalCombiner::BinaryShift {
            op,
            effective_meta,
            lhs_base: lhs_meta.base,
        }));
        work.push(EvalTask::Visit {
            node: rhs,
            ctx: None,
        });
        work.push(EvalTask::Visit {
            node: lhs,
            ctx: Some(effective_meta),
        });
        return Ok(());
    }

    // Arith, bitwise, power.
    let meta = combine_binary_meta(op, lhs_meta, rhs_meta);
    let effective_meta = ExprMeta {
        width: ctx.map_or(meta.width, |c| usize::max(c.width, meta.width)),
        // Arithmetic, bitwise, and power results all take the surrounding
        // expression's propagated signedness: LRM 5.5.1 makes an unsigned
        // operand anywhere in the expression turn the whole expression
        // unsigned, and that unsignedness propagates back down into
        // context-determined operands. Power's *exponent* stays
        // self-determined — see the Power arm below.
        signed: ctx.map_or(meta.signed, |c| c.signed),
        base: meta.base,
    };

    match op {
        BinaryOp::Add
        | BinaryOp::Subtract
        | BinaryOp::Multiply
        | BinaryOp::Divide
        | BinaryOp::Modulus => {
            work.push(EvalTask::Combine(EvalCombiner::BinaryArith {
                op,
                effective_meta,
                meta,
            }));
            work.push(EvalTask::Visit {
                node: rhs,
                ctx: Some(effective_meta),
            });
            work.push(EvalTask::Visit {
                node: lhs,
                ctx: Some(effective_meta),
            });
        }
        BinaryOp::Power => {
            // lhs takes the result width and the *propagated* signedness, so an
            // unsigned outer context zero-extends the base instead of
            // sign-extending it: `(4'shf ** 2) + 8'h0` is 8'he1 (15 ** 2), not
            // 8'h01 ((-1) ** 2). The same propagation decides how a negative
            // exponent reads the base — `(4'shf ** -1) + 4'h0` is 0, because
            // 15 ** -1 truncates to zero, whereas self-determined
            // `4'shf ** -1` stays -1. Its display base still comes from lhs.
            // rhs (the exponent) is self-determined per LRM Table 5-3:
            // evaluated at its own width in the BinaryPower combiner via the
            // standard integer pipeline, then applied with modular
            // exponentiation so the result stays bounded by the result width.
            let lhs_inner_ctx = ExprMeta {
                width: effective_meta.width,
                signed: effective_meta.signed,
                base: effective_meta.base,
            };
            work.push(EvalTask::Combine(EvalCombiner::BinaryPower {
                effective_meta,
            }));
            work.push(EvalTask::Visit {
                node: rhs,
                ctx: None,
            });
            work.push(EvalTask::Visit {
                node: lhs,
                ctx: Some(lhs_inner_ctx),
            });
        }
        BinaryOp::BitwiseAnd
        | BinaryOp::BitwiseOr
        | BinaryOp::BitwiseXor
        | BinaryOp::BitwiseXnor => {
            work.push(EvalTask::Combine(EvalCombiner::BinaryBitwise {
                op,
                effective_meta,
            }));
            work.push(EvalTask::Visit {
                node: rhs,
                ctx: Some(effective_meta),
            });
            work.push(EvalTask::Visit {
                node: lhs,
                ctx: Some(effective_meta),
            });
        }
        _ => unreachable!("non-arith / non-bitwise / non-power Binary handled above"),
    }
    Ok(())
}

fn visit_unary_eval<'b, 'a: 'b>(
    op: UnaryOp,
    operand: &'b Annotated<'a>,
    ctx: Option<ExprMeta>,
    work: &mut Vec<EvalTask<'b, 'a>>,
    _vals: &mut Vec<IntegerValue>,
    _real_vals: &mut Vec<f64>,
    _session: &Session,
) -> Result<(), String> {
    if operand.is_real() {
        // Real unary: only LogicalNot is integer-result on a real
        // operand. Plus/Minus on real have real result (handled by real
        // path); BitwiseNot/Reductions are validator-rejected.
        match op {
            UnaryOp::LogicalNot => {
                // Bridge: pop 1 f64 from `real_vals`, push 1-bit
                // IntegerValue per LRM 5.1.9. Operand visit goes through
                // `push_visit_as_real` so `!1` (integer operand) implicitly
                // coerces; `!1.0` (real operand) directly visits as real.
                work.push(EvalTask::Combine(EvalCombiner::UnaryLogicalNotReal { ctx }));
                push_visit_as_real(operand, work);
                return Ok(());
            }
            UnaryOp::BitwiseNot
            | UnaryOp::ReductionAnd
            | UnaryOp::ReductionNand
            | UnaryOp::ReductionOr
            | UnaryOp::ReductionNor
            | UnaryOp::ReductionXor
            | UnaryOp::ReductionXnor => {
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

    match op {
        UnaryOp::LogicalNot => {
            // Operand is self-determined (ctx=None); Combine reduces it.
            work.push(EvalTask::Combine(EvalCombiner::UnaryLogicalNot { ctx }));
            work.push(EvalTask::Visit {
                node: operand,
                ctx: None,
            });
        }
        UnaryOp::ReductionAnd
        | UnaryOp::ReductionNand
        | UnaryOp::ReductionOr
        | UnaryOp::ReductionNor
        | UnaryOp::ReductionXor
        | UnaryOp::ReductionXnor => {
            work.push(EvalTask::Combine(EvalCombiner::UnaryReduction { op, ctx }));
            work.push(EvalTask::Visit {
                node: operand,
                ctx: None,
            });
        }
        UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitwiseNot => {
            // LRM 5.5.2: width AND signedness propagate from the outer
            // context to the operand's leaf primary.
            let meta = operand.meta();
            let effective_meta = ExprMeta {
                width: ctx.map_or(meta.width, |c| usize::max(c.width, meta.width)),
                signed: ctx.map_or(meta.signed, |c| c.signed),
                base: meta.base,
            };
            work.push(EvalTask::Combine(EvalCombiner::UnaryArith {
                op,
                effective_meta,
                base: meta.base,
            }));
            work.push(EvalTask::Visit {
                node: operand,
                ctx: Some(effective_meta),
            });
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn visit_conditional_eval<'b, 'a: 'b>(
    cond: &'b Annotated<'a>,
    then_arm: &'b Annotated<'a>,
    else_arm: &'b Annotated<'a>,
    ctx: Option<ExprMeta>,
    work: &mut Vec<EvalTask<'b, 'a>>,
    _vals: &mut Vec<IntegerValue>,
    _real_vals: &mut Vec<f64>,
    _session: &Session,
) -> Result<(), String> {
    if then_arm.is_real() || else_arm.is_real() {
        unreachable!("real-typed conditional should be handled by the real path");
    }
    let then_meta = then_arm.meta();
    let else_meta = else_arm.meta();
    let meta = ExprMeta {
        width: usize::max(then_meta.width, else_meta.width),
        signed: then_meta.signed && else_meta.signed,
        base: then_meta.base,
    };
    let effective_meta = ExprMeta {
        width: ctx.map_or(meta.width, |c| usize::max(c.width, meta.width)),
        signed: ctx.map_or(meta.signed, |c| c.signed),
        base: meta.base,
    };

    // Cond may be real-typed even when result is integer (LRM lets cond
    // be any type). Real cond goes through `ConditionalChooseRealCond`
    // which pops the f64 cond off `real_vals` and dispatches the chosen
    // branch; the cond visit pushes onto `real_vals` via `VisitReal`.
    if cond.is_real() {
        work.push(EvalTask::Combine(EvalCombiner::ConditionalChooseRealCond {
            then_arm,
            else_arm,
            effective_meta,
            result_signed: effective_meta.signed,
            result_base: meta.base,
        }));
        work.push(EvalTask::VisitReal { node: cond });
        return Ok(());
    }

    // Integer cond: evaluate it self-determined, decide at Combine.
    work.push(EvalTask::Combine(EvalCombiner::ConditionalChoose {
        then_arm,
        else_arm,
        effective_meta,
        result_signed: effective_meta.signed,
        result_base: meta.base,
    }));
    work.push(EvalTask::Visit {
        node: cond,
        ctx: None,
    });
    Ok(())
}

pub(super) fn combine_eval<'b, 'a: 'b>(
    combiner: EvalCombiner<'b, 'a>,
    work: &mut Vec<EvalTask<'b, 'a>>,
    vals: &mut Vec<IntegerValue>,
    real_vals: &mut Vec<f64>,
    session: &Session,
) -> Result<(), String> {
    match combiner {
        EvalCombiner::Select { node, ctx } => {
            let AnnotatedKind::Select(select) = &node.kind else {
                unreachable!("select combiner requires a select annotation");
            };
            let count = usize::from(select.kind.dynamic_operand().is_some())
                + usize::from(
                    select
                        .inner
                        .as_ref()
                        .and_then(ResolvedSelectKind::dynamic_operand)
                        .is_some(),
                )
                + usize::from(select.source_range.is_some());
            let operands = vals.split_off(vals.len() - count);
            let value = evaluate_resolved_select(node.expr, select, &operands, session)?;
            let _ = select.value.set(value.clone());
            match value {
                Value::Integer(value) => vals.push(match ctx {
                    Some(ctx) => value.resized_to_context(ctx.width, ctx.signed),
                    None => value,
                }),
                Value::Real(value) => real_vals.push(value),
            }
        }
        EvalCombiner::BinaryArith {
            op,
            effective_meta,
            meta,
        } => {
            let rhs_value = vals.pop().expect("BinaryArith: rhs missing");
            let lhs_value = vals.pop().expect("BinaryArith: lhs missing");
            if lhs_value.has_unknown_bits() || rhs_value.has_unknown_bits() {
                vals.push(IntegerValue::all_x(
                    effective_meta.width,
                    effective_meta.signed,
                    meta.base,
                ));
                return Ok(());
            }
            let lhs_int = lhs_value.as_bigint(effective_meta.signed);
            let rhs_int = rhs_value.as_bigint(effective_meta.signed);
            let result = match op {
                BinaryOp::Add => lhs_int + rhs_int,
                BinaryOp::Subtract => lhs_int - rhs_int,
                BinaryOp::Multiply => lhs_int * rhs_int,
                BinaryOp::Divide => {
                    if rhs_int.is_zero() {
                        vals.push(IntegerValue::all_x(
                            effective_meta.width,
                            effective_meta.signed,
                            meta.base,
                        ));
                        return Ok(());
                    }
                    lhs_int / rhs_int
                }
                BinaryOp::Modulus => {
                    if rhs_int.is_zero() {
                        vals.push(IntegerValue::all_x(
                            effective_meta.width,
                            effective_meta.signed,
                            meta.base,
                        ));
                        return Ok(());
                    }
                    lhs_int % rhs_int
                }
                _ => unreachable!("BinaryArith Combine got non-arith op"),
            };
            vals.push(IntegerValue::from_bigint(
                result,
                effective_meta.width,
                effective_meta.signed,
                meta.base,
            ));
        }
        EvalCombiner::BinaryBitwise { op, effective_meta } => {
            let rhs_value = vals.pop().expect("BinaryBitwise: rhs missing");
            let lhs_value = vals.pop().expect("BinaryBitwise: lhs missing");
            let combine = match op {
                BinaryOp::BitwiseAnd => bitwise_and_bits,
                BinaryOp::BitwiseOr => bitwise_or_bits,
                BinaryOp::BitwiseXor => bitwise_xor_bits,
                BinaryOp::BitwiseXnor => bitwise_xnor_bits,
                _ => unreachable!("BinaryBitwise Combine got non-bitwise op"),
            };
            let bits: Vec<LogicBit> = lhs_value
                .bits
                .iter()
                .zip(rhs_value.bits.iter())
                .map(|(l, r)| combine(*l, *r))
                .collect();
            vals.push(IntegerValue::computed(
                effective_meta.width,
                effective_meta.signed,
                effective_meta.base,
                bits,
            ));
        }
        EvalCombiner::BinaryPower { effective_meta } => {
            let exponent_value = vals.pop().expect("BinaryPower: rhs missing");
            let lhs_value = vals.pop().expect("BinaryPower: lhs missing");
            if lhs_value.has_unknown_bits() {
                vals.push(IntegerValue::all_x(
                    effective_meta.width,
                    effective_meta.signed,
                    effective_meta.base,
                ));
                return Ok(());
            }
            // LRM 5.1.6 / Table 5-3: the exponent was visited with no context,
            // so each nested operator has already truncated at its own width.
            // Unknown exponent bits make the whole result x.
            if exponent_value.has_unknown_bits() {
                vals.push(IntegerValue::all_x(
                    effective_meta.width,
                    effective_meta.signed,
                    effective_meta.base,
                ));
                return Ok(());
            }
            let exponent_value = exponent_value.as_bigint(exponent_value.signed);
            // The base must be read with the same signedness the visitor
            // extended it under. Using the LHS's own signedness instead would
            // reinterpret an already zero-extended base as negative, so
            // `(4'shf ** -1) + 4'h0` would answer -1 where iverilog says 0.
            let base_value = lhs_value.as_bigint(effective_meta.signed);
            let result = match evaluate_power(base_value, exponent_value, effective_meta.width) {
                Ok(r) => r,
                Err(_) => {
                    vals.push(IntegerValue::all_x(
                        effective_meta.width,
                        effective_meta.signed,
                        effective_meta.base,
                    ));
                    return Ok(());
                }
            };
            vals.push(IntegerValue::from_bigint(
                result,
                effective_meta.width,
                effective_meta.signed,
                effective_meta.base,
            ));
        }
        EvalCombiner::BinaryRelational { op, signed, ctx } => {
            let rhs_value = vals.pop().expect("BinaryRelational: rhs missing");
            let lhs_value = vals.pop().expect("BinaryRelational: lhs missing");
            vals.push(compute_relational_from_values(
                op, &lhs_value, &rhs_value, signed, ctx,
            ));
        }
        EvalCombiner::BinaryEquality { op, ctx } => {
            let rhs_value = vals.pop().expect("BinaryEquality: rhs missing");
            let lhs_value = vals.pop().expect("BinaryEquality: lhs missing");
            vals.push(compute_equality_from_values(
                op, &lhs_value, &rhs_value, ctx,
            ));
        }
        EvalCombiner::BinaryLogical { op, ctx } => {
            let rhs_value = vals.pop().expect("BinaryLogical: rhs missing");
            let lhs_value = vals.pop().expect("BinaryLogical: lhs missing");
            let lhs_logical = logical_value(&lhs_value);
            let rhs_logical = logical_value(&rhs_value);
            let bit = apply_logical_truth_table(op, lhs_logical, rhs_logical);
            vals.push(widen_relational_result(comparison_result_value(bit), ctx));
        }
        EvalCombiner::BinaryShift {
            op,
            effective_meta,
            lhs_base,
        } => {
            let rhs_value = vals.pop().expect("BinaryShift: rhs missing");
            let lhs_value = vals.pop().expect("BinaryShift: lhs missing");
            if rhs_value.has_unknown_bits() {
                vals.push(IntegerValue::all_x(
                    effective_meta.width,
                    effective_meta.signed,
                    lhs_base,
                ));
                return Ok(());
            }
            let shift_count = bits_to_biguint(&rhs_value.bits);
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
                _ => unreachable!("BinaryShift Combine got non-shift op"),
            };
            vals.push(IntegerValue::computed(
                effective_meta.width,
                effective_meta.signed,
                lhs_base,
                result_bits,
            ));
        }
        EvalCombiner::UnaryArith {
            op,
            effective_meta,
            base,
        } => {
            let operand = vals.pop().expect("UnaryArith: operand missing");
            let result = match op {
                UnaryOp::Plus => operand,
                UnaryOp::BitwiseNot => {
                    let bits: Vec<LogicBit> =
                        operand.bits.iter().copied().map(bitwise_not_bit).collect();
                    IntegerValue::computed(effective_meta.width, effective_meta.signed, base, bits)
                }
                UnaryOp::Minus => {
                    if operand.has_unknown_bits() {
                        IntegerValue::all_x(effective_meta.width, effective_meta.signed, base)
                    } else {
                        let neg = -operand.as_bigint(effective_meta.signed);
                        IntegerValue::from_bigint(
                            neg,
                            effective_meta.width,
                            effective_meta.signed,
                            base,
                        )
                    }
                }
                _ => unreachable!("UnaryArith Combine got {:?}", op),
            };
            vals.push(result);
        }
        EvalCombiner::UnaryLogicalNot { ctx } => {
            let operand = vals.pop().expect("UnaryLogicalNot: operand missing");
            let bit = match logical_value(&operand) {
                LogicBit::One => LogicBit::Zero,
                LogicBit::Zero => LogicBit::One,
                LogicBit::X | LogicBit::Z => LogicBit::X,
            };
            vals.push(widen_relational_result(comparison_result_value(bit), ctx));
        }
        EvalCombiner::UnaryReduction { op, ctx } => {
            let operand = vals.pop().expect("UnaryReduction: operand missing");
            let bit = reduce_bits(op, &operand.bits);
            vals.push(widen_relational_result(comparison_result_value(bit), ctx));
        }
        EvalCombiner::ConditionalChoose {
            then_arm,
            else_arm,
            effective_meta,
            result_signed,
            result_base,
        } => {
            let cond_value = vals.pop().expect("ConditionalChoose: cond missing");
            let cond_logical = logical_value(&cond_value);
            match cond_logical {
                LogicBit::One => {
                    work.push(EvalTask::Combine(EvalCombiner::ConditionalFinalize {
                        effective_meta,
                        result_signed,
                        result_base,
                    }));
                    work.push(EvalTask::Visit {
                        node: then_arm,
                        ctx: Some(effective_meta),
                    });
                }
                LogicBit::Zero => {
                    work.push(EvalTask::Combine(EvalCombiner::ConditionalFinalize {
                        effective_meta,
                        result_signed,
                        result_base,
                    }));
                    work.push(EvalTask::Visit {
                        node: else_arm,
                        ctx: Some(effective_meta),
                    });
                }
                LogicBit::X | LogicBit::Z => {
                    work.push(EvalTask::Combine(EvalCombiner::ConditionalMerge {
                        effective_meta,
                        result_signed,
                        result_base,
                    }));
                    work.push(EvalTask::Visit {
                        node: else_arm,
                        ctx: Some(effective_meta),
                    });
                    work.push(EvalTask::Visit {
                        node: then_arm,
                        ctx: Some(effective_meta),
                    });
                }
            }
        }
        EvalCombiner::ConditionalMerge {
            effective_meta,
            result_signed,
            result_base,
        } => {
            let else_value = vals.pop().expect("ConditionalMerge: else missing");
            let then_value = vals.pop().expect("ConditionalMerge: then missing");
            let bits: Vec<LogicBit> = then_value
                .bits
                .iter()
                .zip(else_value.bits.iter())
                .map(|(t, e)| if t == e { *t } else { LogicBit::X })
                .collect();
            vals.push(IntegerValue::computed(
                effective_meta.width,
                result_signed,
                result_base,
                bits,
            ));
        }
        EvalCombiner::ConditionalFinalize {
            effective_meta,
            result_signed,
            result_base,
        } => {
            let chosen = vals.pop().expect("ConditionalFinalize: chosen missing");
            // The branch was evaluated with effective_meta as ctx, so its
            // bit-width should match. Re-stamp signedness/base to the
            // conditional's unified type per LRM 5.5.1.
            vals.push(IntegerValue::computed(
                effective_meta.width,
                result_signed,
                result_base,
                chosen.bits,
            ));
        }
        EvalCombiner::SignCast { signed, ctx } => {
            let arg = vals.pop().expect("SignCast: arg missing");
            let cast_value = IntegerValue::computed(arg.width, signed, arg.base, arg.bits);
            vals.push(extend_cast_to_outer_context(cast_value, ctx));
        }
        EvalCombiner::BaseCast { base, ctx } => {
            let arg = vals.pop().expect("BaseCast: arg missing");
            let cast_value = IntegerValue::computed(arg.width, arg.signed, base, arg.bits);
            vals.push(extend_cast_to_outer_context(cast_value, ctx));
        }
        EvalCombiner::Concatenation {
            item_count,
            leftmost_base,
            ctx,
        } => {
            // Drain item_count items from the value stack in source order
            // (items[0] at the bottom). Bit vectors are LSB-first; concat
            // joins items leftmost → MSB-side, so iterate items in reverse
            // and extend bits in that order.
            let start = vals.len() - item_count;
            let items: Vec<IntegerValue> = vals.drain(start..).collect();
            // Cap the sum of operand widths before allocating. Without this,
            // `{a, a}` with two 16M-bit operands silently produces a 32M-bit
            // garbage result. Use saturating_add so a usize-overflow degrades
            // gracefully to the rejection path.
            let total = items
                .iter()
                .fold(0usize, |acc, item| acc.saturating_add(item.bits.len()));
            value::ensure_bit_width(total, "concatenation")
                .map_err(|e| format!("Semantic error: {e}"))?;
            let mut bits = Vec::with_capacity(total);
            for item in items.iter().rev() {
                bits.extend(item.bits.iter().copied());
            }
            if bits.is_empty() {
                return Err(
                    "concatenation must have at least one operand with positive size".to_string(),
                );
            }
            let display_style = concatenated_display_style(&items);
            let result = IntegerValue::computed(bits.len(), false, leftmost_base, bits)
                .with_display_style(display_style);
            vals.push(extend_to_outer_context(result, ctx));
        }
        EvalCombiner::ReplicationCountReceived {
            items,
            leftmost_base,
            ctx,
            strict,
        } => {
            let count_val = vals.pop().expect("ReplicationCountReceived: count missing");
            if count_val.has_unknown_bits() {
                return Err("replication count contains unknown bits".to_string());
            }
            let count_bigint = count_val.as_bigint(count_val.signed);
            if count_bigint.sign() == Sign::Minus {
                return Err("replication count must be non-negative".to_string());
            }
            let count = count_bigint
                .to_usize()
                .ok_or_else(|| "replication count too large".to_string())?;
            if count == 0 {
                if strict {
                    return Err("replication count must be positive in this context".to_string());
                }
                // Lenient (inside-concat): contribute zero bits. The
                // surrounding Concatenation Combine still enforces the
                // "must have at least one operand with positive size"
                // rule by inspecting the joined bits.
                vals.push(IntegerValue::computed(0, false, leftmost_base, Vec::new()));
                return Ok(());
            }
            work.push(EvalTask::Combine(EvalCombiner::ReplicationFinalize {
                item_count: items.len(),
                count,
                leftmost_base,
                ctx,
                strict,
            }));
            for item in items.iter().rev() {
                push_concat_item_eval(item, work);
            }
        }
        EvalCombiner::ReplicationFinalize {
            item_count,
            count,
            leftmost_base,
            ctx,
            strict,
        } => {
            let start = vals.len() - item_count;
            let items: Vec<IntegerValue> = vals.drain(start..).collect();
            let mut inner_bits = Vec::new();
            for item in items.iter().rev() {
                inner_bits.extend(item.bits.iter().copied());
            }
            if strict && inner_bits.is_empty() {
                return Err(
                    "concatenation must have at least one operand with positive size".to_string(),
                );
            }
            let total = inner_bits.len().saturating_mul(count);
            value::ensure_bit_width(total, "replication")
                .map_err(|e| format!("Semantic error: {e}"))?;
            let mut bits = Vec::with_capacity(total);
            for _ in 0..count {
                bits.extend(inner_bits.iter().copied());
            }
            let display_style = concatenated_display_style(&items);
            let result = IntegerValue::computed(bits.len(), false, leftmost_base, bits)
                .with_display_style(display_style);
            vals.push(extend_to_outer_context(result, ctx));
        }
        // ----- Bridge variants (real_vals → int_vals or read int_vals to
        // produce an integer-pipeline value). -----
        EvalCombiner::RealConversionToInt { kind, ctx } => {
            let real_val = real_vals
                .pop()
                .expect("RealConversionToInt: real arg missing");
            let result = match kind {
                RealConversionKind::RealToInteger => real_to_integer_value(real_val),
                RealConversionKind::RealToBits => {
                    // LRM 17.8: bitcast a real to its 64-bit IEEE 754
                    // representation. Display as hex since the value is a
                    // bit pattern, not a magnitude.
                    let bits = real_val.to_bits();
                    IntegerValue::from_bigint(BigInt::from(bits), 64, false, Base::Hex)
                }
                RealConversionKind::IntegerToReal | RealConversionKind::BitsToReal => {
                    unreachable!("real-result conversions don't reach RealConversionToInt");
                }
            };
            vals.push(extend_cast_to_outer_context(result, ctx));
        }
        EvalCombiner::Clog2 { ctx } => {
            let arg = vals.pop().expect("Clog2: arg missing");
            let result = if arg.has_unknown_bits() {
                IntegerValue::all_x(32, true, Base::Decimal)
            } else {
                clog2_result_value(bits_to_biguint(&arg.bits))
            };
            vals.push(extend_cast_to_outer_context(result, ctx));
        }
        EvalCombiner::UnaryLogicalNotReal { ctx } => {
            let value = real_vals
                .pop()
                .expect("UnaryLogicalNotReal: operand missing");
            let bit = match logical_value_of_real(value) {
                LogicBit::One => LogicBit::Zero,
                LogicBit::Zero => LogicBit::One,
                LogicBit::X | LogicBit::Z => LogicBit::X,
            };
            vals.push(widen_relational_result(comparison_result_value(bit), ctx));
        }
        EvalCombiner::BinaryRealRelational { op, ctx } => {
            let rhs_val = real_vals.pop().expect("BinaryRealRelational: rhs missing");
            let lhs_val = real_vals.pop().expect("BinaryRealRelational: lhs missing");
            let result = match op {
                BinaryOp::LessThan => lhs_val < rhs_val,
                BinaryOp::GreaterThan => lhs_val > rhs_val,
                BinaryOp::LessThanOrEqual => lhs_val <= rhs_val,
                BinaryOp::GreaterThanOrEqual => lhs_val >= rhs_val,
                _ => unreachable!("non-relational op in BinaryRealRelational"),
            };
            let bit = if result {
                LogicBit::One
            } else {
                LogicBit::Zero
            };
            vals.push(widen_relational_result(comparison_result_value(bit), ctx));
        }
        EvalCombiner::BinaryRealEquality { op, ctx } => {
            let rhs_val = real_vals.pop().expect("BinaryRealEquality: rhs missing");
            let lhs_val = real_vals.pop().expect("BinaryRealEquality: lhs missing");
            let result = match op {
                BinaryOp::Equal => lhs_val == rhs_val,
                BinaryOp::NotEqual => lhs_val != rhs_val,
                _ => unreachable!("non-equality op in BinaryRealEquality"),
            };
            let bit = if result {
                LogicBit::One
            } else {
                LogicBit::Zero
            };
            vals.push(widen_relational_result(comparison_result_value(bit), ctx));
        }
        EvalCombiner::BinaryRealLogical { op, ctx } => {
            let rhs_val = real_vals.pop().expect("BinaryRealLogical: rhs missing");
            let lhs_val = real_vals.pop().expect("BinaryRealLogical: lhs missing");
            let lhs_logical = logical_value_of_real(lhs_val);
            let rhs_logical = logical_value_of_real(rhs_val);
            let bit = apply_logical_truth_table(op, lhs_logical, rhs_logical);
            vals.push(widen_relational_result(comparison_result_value(bit), ctx));
        }
        EvalCombiner::BinaryRealLogicalMixed {
            op,
            ctx,
            lhs_is_real,
        } => {
            // Exactly one operand is real. Reducing both to real first (what
            // the plain `BinaryRealLogical` path does) maps the integer side's
            // x/z bits to 0.0 via LRM 3.5.3, so `1'bx && 1.0` would answer 0
            // instead of x. Reduce each operand in its own type instead: the
            // real side through `logical_value_of_real`, the integer side
            // through `logical_value` (reduction-OR, x/z preserved).
            let (lhs_logical, rhs_logical) = if lhs_is_real {
                let lhs_val = real_vals
                    .pop()
                    .expect("BinaryRealLogicalMixed: lhs real missing");
                let rhs_val = vals
                    .pop()
                    .expect("BinaryRealLogicalMixed: rhs integer missing");
                (logical_value_of_real(lhs_val), logical_value(&rhs_val))
            } else {
                let lhs_val = vals
                    .pop()
                    .expect("BinaryRealLogicalMixed: lhs integer missing");
                let rhs_val = real_vals
                    .pop()
                    .expect("BinaryRealLogicalMixed: rhs real missing");
                (logical_value(&lhs_val), logical_value_of_real(rhs_val))
            };
            let bit = apply_logical_truth_table(op, lhs_logical, rhs_logical);
            vals.push(widen_relational_result(comparison_result_value(bit), ctx));
        }
        EvalCombiner::ConditionalChooseRealCond {
            then_arm,
            else_arm,
            effective_meta,
            result_signed,
            result_base,
        } => {
            let cond_val = real_vals
                .pop()
                .expect("ConditionalChooseRealCond: cond missing");
            match logical_value_of_real(cond_val) {
                LogicBit::One => {
                    work.push(EvalTask::Combine(EvalCombiner::ConditionalFinalize {
                        effective_meta,
                        result_signed,
                        result_base,
                    }));
                    work.push(EvalTask::Visit {
                        node: then_arm,
                        ctx: Some(effective_meta),
                    });
                }
                LogicBit::Zero => {
                    work.push(EvalTask::Combine(EvalCombiner::ConditionalFinalize {
                        effective_meta,
                        result_signed,
                        result_base,
                    }));
                    work.push(EvalTask::Visit {
                        node: else_arm,
                        ctx: Some(effective_meta),
                    });
                }
                LogicBit::X | LogicBit::Z => {
                    work.push(EvalTask::Combine(EvalCombiner::ConditionalMerge {
                        effective_meta,
                        result_signed,
                        result_base,
                    }));
                    work.push(EvalTask::Visit {
                        node: else_arm,
                        ctx: Some(effective_meta),
                    });
                    work.push(EvalTask::Visit {
                        node: then_arm,
                        ctx: Some(effective_meta),
                    });
                }
            }
        }
    }
    Ok(())
}
