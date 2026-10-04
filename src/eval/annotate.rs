//! Bottom-up expression annotation and result-type inference.

use super::ir::{Annotated, AnnotatedKind, ExprMeta};
use super::machine::replication_count_value;
use super::ops::combine_binary_meta;
use super::select::{annotate_select, select_operand_exprs};
use super::validate::validate_annotated;

use crate::parser::{
    BinaryOp, Expr, RealConversionKind, SelectKind, SystemArg, UnaryOp, string_literal_spec,
};
use crate::system_call::{SystemCallKind, SystemFunction, classify_system_call};
use crate::value::Base;
use crate::{Session, value};

// Annotate the expression tree once, bottom-up. Returns structural errors
// (undeclared identifier, array used as value, system task in an expression
// position) in unprefixed form — the entry-point caller wraps them with
// "Semantic error: " when they surface during the static-semantic phase, and
// passes them through unprefixed when they surface at evaluation time, both
// matching today's error-prefix convention.
//
// Real-result branches store `meta = None`; integer branches store
// `Some(meta)` computed from the children's metas using the same combination
// rules the meta walkers previously re-derived from the tree. Select
// operands also join this bottom-up walk, rather than restarting it through
// the raw-Expr select helpers.
// Iterative CES (control-environment-store) driver: each `Visit` task
// inspects an `Expr` node and, for parent shapes, schedules a `Combine` task
// followed by `Visit` tasks for the children (pushed in reverse so they pop
// in source order). Each `Combine` pops its child annotations off `vals` and
// assembles the parent `Annotated`. Leaves (Literal, RealLiteral, SystemTask,
// Identifier, Truncated-via-unreachable) push their annotation
// directly with no Combine. This keeps Rust stack depth O(1) regardless of
// input nesting depth — paired with `impl Drop for Annotated` in ir.rs so the
// resulting deep `Box<Annotated>` chain doesn't crash at end-of-scope.
enum AnnotateTask<'a> {
    Visit(&'a Expr),
    Combine(AnnotateCombiner<'a>),
}

enum AnnotateCombiner<'a> {
    Select {
        expr: &'a Expr,
        operand_count: usize,
    },
    Grouped {
        expr: &'a Expr,
    },
    Unary {
        expr: &'a Expr,
        op: UnaryOp,
    },
    Binary {
        expr: &'a Expr,
        op: BinaryOp,
    },
    Conditional {
        expr: &'a Expr,
    },
    Concatenation {
        expr: &'a Expr,
        item_count: usize,
    },
    Replication {
        expr: &'a Expr,
        item_count: usize,
    },
    SystemCall {
        expr: &'a Expr,
        kind: SystemCallKind,
        arg_count: usize,
        expr_arg_count: usize,
    },
}

pub(crate) fn annotate<'a>(root: &'a Expr, session: &Session) -> Result<Annotated<'a>, String> {
    let mut work: Vec<AnnotateTask<'a>> = vec![AnnotateTask::Visit(root)];
    let mut vals: Vec<Annotated<'a>> = Vec::new();

    while let Some(task) = work.pop() {
        match task {
            AnnotateTask::Visit(expr) => match expr {
                Expr::Literal(value) => vals.push(Annotated {
                    expr,
                    meta: Some(ExprMeta {
                        width: value.width,
                        signed: value.signed,
                        base: value.base,
                    }),
                    kind: AnnotatedKind::Leaf,
                }),
                Expr::StringLiteral(bytes) => {
                    let spec = string_literal_spec(bytes);
                    vals.push(Annotated {
                        expr,
                        meta: Some(ExprMeta {
                            width: spec.width,
                            signed: spec.signed,
                            base: spec.base,
                        }),
                        kind: AnnotatedKind::Leaf,
                    });
                }
                Expr::RealLiteral(_) => vals.push(Annotated {
                    expr,
                    meta: None,
                    kind: AnnotatedKind::Leaf,
                }),
                Expr::Grouped(inner) => {
                    work.push(AnnotateTask::Combine(AnnotateCombiner::Grouped { expr }));
                    work.push(AnnotateTask::Visit(inner));
                }
                Expr::Unary { op, expr: operand } => {
                    work.push(AnnotateTask::Combine(AnnotateCombiner::Unary {
                        expr,
                        op: *op,
                    }));
                    work.push(AnnotateTask::Visit(operand));
                }
                Expr::Binary { op, lhs, rhs } => {
                    work.push(AnnotateTask::Combine(AnnotateCombiner::Binary {
                        expr,
                        op: *op,
                    }));
                    work.push(AnnotateTask::Visit(rhs));
                    work.push(AnnotateTask::Visit(lhs));
                }
                Expr::Conditional {
                    cond,
                    then_expr,
                    else_expr,
                } => {
                    work.push(AnnotateTask::Combine(AnnotateCombiner::Conditional {
                        expr,
                    }));
                    work.push(AnnotateTask::Visit(else_expr));
                    work.push(AnnotateTask::Visit(then_expr));
                    work.push(AnnotateTask::Visit(cond));
                }
                Expr::Concatenation { items } => {
                    work.push(AnnotateTask::Combine(AnnotateCombiner::Concatenation {
                        expr,
                        item_count: items.len(),
                    }));
                    for item in items.iter().rev() {
                        work.push(AnnotateTask::Visit(item));
                    }
                }
                Expr::Replication { count, items } => {
                    work.push(AnnotateTask::Combine(AnnotateCombiner::Replication {
                        expr,
                        item_count: items.len(),
                    }));
                    for item in items.iter().rev() {
                        work.push(AnnotateTask::Visit(item));
                    }
                    work.push(AnnotateTask::Visit(count));
                }
                // One arm for every `$name(args)` shape. `classify_system_call`
                // resolves the name into the typed `SystemCallKind`; arity is
                // enforced at combine time. Unknown name → "unknown system
                // identifier" error (the only place the name table lives).
                // Tasks (`$finish` / `$stop`) become a leaf SystemTask with
                // args discarded — LRM 17.4 message-verbosity argument has no
                // observable effect in vcal, and the args were parsed for
                // syntactic validity by the parser already.
                Expr::SystemCall { name, args } => {
                    let kind = classify_system_call(name)?;
                    match kind {
                        SystemCallKind::Task(_) => vals.push(Annotated {
                            expr,
                            meta: None,
                            kind: AnnotatedKind::SystemTask,
                        }),
                        _ => {
                            let expr_arg_count = args
                                .iter()
                                .filter(|arg| matches!(arg, SystemArg::Expr(_)))
                                .count();
                            work.push(AnnotateTask::Combine(AnnotateCombiner::SystemCall {
                                expr,
                                kind,
                                arg_count: args.len(),
                                expr_arg_count,
                            }));
                            for arg in args.iter().rev() {
                                if let SystemArg::Expr(arg) = arg {
                                    work.push(AnnotateTask::Visit(arg));
                                }
                            }
                        }
                    }
                }
                Expr::Identifier(name) => {
                    let reg = session
                        .lookup(name)
                        .ok_or_else(|| format!("undeclared identifier: {name}"))?;
                    // Real reg → None meta (real pipeline). Array regs also
                    // stamp None — they have no value as a whole, so the
                    // structural validator surfaces the friendlier diagnostic
                    // before any consumer touches the meta. Only a plain
                    // vector reg produces a concrete `ExprMeta`.
                    let meta = if reg.is_real() || reg.is_array() {
                        None
                    } else {
                        let value = reg.require_vector(name)?;
                        Some(ExprMeta {
                            width: value.width,
                            signed: value.signed,
                            base: value.base,
                        })
                    };
                    vals.push(Annotated {
                        expr,
                        meta,
                        kind: AnnotatedKind::Leaf,
                    });
                }
                Expr::Select { name, kind, inner } => {
                    // Preserve undeclared-name precedence before walking operands.
                    session
                        .lookup(name)
                        .ok_or_else(|| format!("undeclared identifier: {name}"))?;
                    let mut operands = select_operand_exprs(kind);
                    if let Some(inner) = inner {
                        operands.extend(select_operand_exprs(inner));
                    }
                    work.push(AnnotateTask::Combine(AnnotateCombiner::Select {
                        expr,
                        operand_count: operands.len(),
                    }));
                    for operand in operands.into_iter().rev() {
                        work.push(AnnotateTask::Visit(operand));
                    }
                }
                Expr::ConcatSelect { source, kind } => {
                    let mut operands = vec![source.as_ref()];
                    operands.extend(select_operand_exprs(kind));
                    work.push(AnnotateTask::Combine(AnnotateCombiner::Select {
                        expr,
                        operand_count: operands.len(),
                    }));
                    for operand in operands.into_iter().rev() {
                        work.push(AnnotateTask::Visit(operand));
                    }
                }
                Expr::Truncated => unreachable!(
                    "Expr::Truncated is a display-only sentinel; never reaches annotate"
                ),
            },
            AnnotateTask::Combine(combiner) => {
                let parent = annotate_combine(combiner, &mut vals, session)?;
                vals.push(parent);
            }
        }
    }

    debug_assert_eq!(
        vals.len(),
        1,
        "annotate produced {} root values",
        vals.len()
    );
    Ok(vals
        .pop()
        .expect("driver invariant: one root produces one value"))
}

fn annotate_combine<'a>(
    combiner: AnnotateCombiner<'a>,
    vals: &mut Vec<Annotated<'a>>,
    session: &Session,
) -> Result<Annotated<'a>, String> {
    // Helper: pop N children left-to-right (the value stack has them in
    // push order, i.e. children appear in source order at positions
    // [len - n .. len)).
    fn pop_n<'a>(vals: &mut Vec<Annotated<'a>>, n: usize) -> Vec<Annotated<'a>> {
        let start = vals.len() - n;
        vals.drain(start..).collect()
    }

    match combiner {
        AnnotateCombiner::Select {
            expr,
            operand_count,
        } => {
            let operands = pop_n(vals, operand_count);
            annotate_select(expr, operands, session)
        }
        AnnotateCombiner::Grouped { expr } => {
            let inner = vals.pop().expect("Grouped: inner missing");
            Ok(Annotated {
                expr,
                meta: inner.meta,
                kind: AnnotatedKind::Grouped(Box::new(inner)),
            })
        }
        AnnotateCombiner::Unary { expr, op } => {
            let operand = vals.pop().expect("Unary: operand missing");
            // LRM 5.1.5: unary `+` / `-` preserve operand's type and are
            // real iff the operand is real. `~` is integer-only and
            // preserves the operand's meta. Logical / reduction ops yield
            // 1-bit unsigned binary regardless of operand type; the
            // real-operand validity check lives in the validator.
            let meta = match op {
                UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitwiseNot => operand.meta,
                UnaryOp::LogicalNot
                | UnaryOp::ReductionAnd
                | UnaryOp::ReductionNand
                | UnaryOp::ReductionOr
                | UnaryOp::ReductionNor
                | UnaryOp::ReductionXor
                | UnaryOp::ReductionXnor => Some(ExprMeta {
                    width: 1,
                    signed: false,
                    base: Base::Binary,
                }),
            };
            Ok(Annotated {
                expr,
                meta,
                kind: AnnotatedKind::Unary(Box::new(operand)),
            })
        }
        AnnotateCombiner::Binary { expr, op } => {
            let rhs = vals.pop().expect("Binary: rhs missing");
            let lhs = vals.pop().expect("Binary: lhs missing");
            let meta = binary_result_meta(op, &lhs, &rhs);
            Ok(Annotated {
                expr,
                meta,
                kind: AnnotatedKind::Binary {
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            })
        }
        AnnotateCombiner::Conditional { expr } => {
            let else_arm = vals.pop().expect("Conditional: else missing");
            let then_arm = vals.pop().expect("Conditional: then missing");
            let cond = vals.pop().expect("Conditional: cond missing");
            // LRM 5.1.13: result type is real iff either branch is real.
            // Cond contributes nothing to result meta. For integer branches
            // we combine widths (max) and signedness (any unsigned →
            // unsigned).
            let meta = if then_arm.is_real() || else_arm.is_real() {
                None
            } else {
                let then_meta = then_arm.meta();
                let else_meta = else_arm.meta();
                Some(ExprMeta {
                    width: usize::max(then_meta.width, else_meta.width),
                    signed: then_meta.signed && else_meta.signed,
                    base: then_meta.base,
                })
            };
            Ok(Annotated {
                expr,
                meta,
                kind: AnnotatedKind::Conditional {
                    cond: Box::new(cond),
                    then_arm: Box::new(then_arm),
                    else_arm: Box::new(else_arm),
                },
            })
        }
        AnnotateCombiner::Concatenation { expr, item_count } => {
            let item_annots = pop_n(vals, item_count);
            // LRM 5.1.14: width = sum of operand widths, always unsigned,
            // base from leftmost item. Real items are rejected by the
            // validator (concat requires definite bit widths), but the width
            // published here is consumed before that runs — context
            // propagation uses it to resize *sibling* leaves — so it has to
            // respect the cap on its own, exactly like the replication arm.
            let mut total_width = 0usize;
            let mut leftmost_base = Base::Binary;
            for (idx, item) in item_annots.iter().enumerate() {
                if let Some(item_meta) = item.meta {
                    total_width = total_width.saturating_add(item_meta.width);
                    if idx == 0 {
                        leftmost_base = item_meta.base;
                    }
                }
            }
            value::ensure_bit_width(total_width, "concatenation")?;
            Ok(Annotated {
                expr,
                meta: Some(ExprMeta {
                    width: total_width,
                    signed: false,
                    base: leftmost_base,
                }),
                kind: AnnotatedKind::Concatenation(item_annots),
            })
        }
        AnnotateCombiner::Replication { expr, item_count } => {
            let item_annots = pop_n(vals, item_count);
            let count_annot = vals.pop().expect("Replication: count missing");
            // Validate the count subtree before anything else: a real-typed
            // count can still carry a structural error (task-in-expression,
            // operator-on-real, $signed-on-real, $bitstoreal-width, ...),
            // and that specific diagnostic must surface ahead of the
            // generic "replication count cannot be real" the validator
            // emits for plain real counts.
            validate_annotated(&count_annot, session)?;
            // Width depends on the count's evaluated value. Skip the eager
            // evaluation when the count or any item is real-typed — the
            // validator surfaces "replication count cannot be real" /
            // "replication operand cannot be real" with clearer framing,
            // and trying to evaluate as integer first would only swap that
            // for "real value cannot be used as an integer expression here".
            let count_value = if count_annot.is_real() || item_annots.iter().any(|i| i.is_real()) {
                None
            } else {
                // The count is constant-evaluated here — before the
                // whole-tree validate pass runs — because the width meta
                // needs its value. Evaluators assume validated input
                // (`unreachable!` on validator-rejected shapes like
                // `1.0 % 2`); the validate_annotated call above guarantees
                // that invariant holds.
                Some(replication_count_value(&count_annot, session)?)
            };
            let mut inner_width = 0usize;
            let mut leftmost_base = Base::Binary;
            for (idx, item) in item_annots.iter().enumerate() {
                if let Some(item_meta) = item.meta {
                    inner_width = inner_width.saturating_add(item_meta.width);
                    if idx == 0 {
                        leftmost_base = item_meta.base;
                    }
                }
            }
            let total_width = count_value.map_or(0, |c| inner_width.saturating_mul(c));
            // Cap the inferred width here, not only at `ReplicationFinalize`.
            // Annotation publishes this width into `ExprMeta`, and context
            // propagation uses it to resize *sibling* leaves — so an uncapped
            // width allocates before this replication node is ever evaluated
            // (`1 ? 1'b1 : {64'hffff_ffff_ffff_ffff{1'b0}}` aborts with
            // `capacity overflow`), and a width merely above the cap silently
            // violates it. Same invariant `annotate_select` enforces for
            // part-selects.
            value::ensure_bit_width(total_width, "replication")?;
            Ok(Annotated {
                expr,
                meta: Some(ExprMeta {
                    width: total_width,
                    signed: false,
                    base: leftmost_base,
                }),
                kind: AnnotatedKind::Replication {
                    count: Box::new(count_annot),
                    items: item_annots,
                },
            })
        }
        AnnotateCombiner::SystemCall {
            expr,
            kind,
            arg_count,
            expr_arg_count,
        } => {
            // Pop args first so the early-return arity-error path drops
            // them at end of scope (via Annotated's iterative Drop) rather
            // than leaving them on the value stack.
            let mut arg_annots = pop_n(vals, expr_arg_count);
            let (system_call_name, system_args) = match expr {
                Expr::SystemCall { name, args } => (name.as_str(), args.as_slice()),
                _ => unreachable!("AnnotateCombiner::SystemCall wraps only Expr::SystemCall"),
            };
            // Single source for the "wrong arity" diagnostic. Reads slightly
            // off the kind to keep the wording consistent across all four
            // function-shape classes.
            fn expect_arity(name: &str, got: usize, want: usize) -> Result<(), String> {
                if got == want {
                    Ok(())
                } else {
                    Err(format!(
                        "{name} expects {want} argument{plural}, got {got}",
                        plural = if want == 1 { "" } else { "s" }
                    ))
                }
            }
            match kind {
                SystemCallKind::Task(_) => {
                    unreachable!("Task kind is built as a leaf in annotate's Visit arm")
                }
                SystemCallKind::Function(SystemFunction::SignCast { signed }) => {
                    expect_arity(system_call_name, arg_count, 1)?;
                    reject_null_system_args(system_call_name, system_args)?;
                    let arg = arg_annots.pop().expect("len == 1");
                    // LRM 5.5: width / base from arg, signedness from cast.
                    // Real arg rejected by validator; meta computed here is a
                    // no-op for real cases (validator surfaces the error first).
                    let meta = arg.meta.map(|arg_meta| ExprMeta {
                        width: arg_meta.width,
                        signed,
                        base: arg_meta.base,
                    });
                    Ok(Annotated {
                        expr,
                        meta,
                        kind: AnnotatedKind::SignCast {
                            signed,
                            arg: Box::new(arg),
                        },
                    })
                }
                SystemCallKind::Function(SystemFunction::BaseCast(base)) => {
                    expect_arity(system_call_name, arg_count, 1)?;
                    reject_null_system_args(system_call_name, system_args)?;
                    let arg = arg_annots.pop().expect("len == 1");
                    let meta = arg.meta.map(|arg_meta| ExprMeta {
                        width: arg_meta.width,
                        signed: arg_meta.signed,
                        base,
                    });
                    Ok(Annotated {
                        expr,
                        meta,
                        kind: AnnotatedKind::BaseCast {
                            base,
                            arg: Box::new(arg),
                        },
                    })
                }
                SystemCallKind::Function(SystemFunction::RealConversion(conv_kind)) => {
                    expect_arity(system_call_name, arg_count, 1)?;
                    reject_null_system_args(system_call_name, system_args)?;
                    let arg = arg_annots.pop().expect("len == 1");
                    // LRM 17.8: $rtoi is 32-bit signed decimal; $realtobits is
                    // 64-bit unsigned hex; $itor and $bitstoreal are real-typed.
                    let meta = match conv_kind {
                        RealConversionKind::RealToInteger => Some(ExprMeta {
                            width: 32,
                            signed: true,
                            base: Base::Decimal,
                        }),
                        RealConversionKind::RealToBits => Some(ExprMeta {
                            width: 64,
                            signed: false,
                            base: Base::Hex,
                        }),
                        RealConversionKind::IntegerToReal | RealConversionKind::BitsToReal => None,
                    };
                    Ok(Annotated {
                        expr,
                        meta,
                        kind: AnnotatedKind::RealConversion {
                            kind: conv_kind,
                            arg: Box::new(arg),
                        },
                    })
                }
                SystemCallKind::Function(SystemFunction::Math(math_kind)) => {
                    expect_arity(system_call_name, arg_count, math_kind.arity())?;
                    reject_null_system_args(system_call_name, system_args)?;
                    // LRM 17.11: $clog2 → 32-bit signed decimal; the rest yield real.
                    let meta = if math_kind.is_real_result() {
                        None
                    } else {
                        Some(ExprMeta {
                            width: 32,
                            signed: true,
                            base: Base::Decimal,
                        })
                    };
                    Ok(Annotated {
                        expr,
                        meta,
                        kind: AnnotatedKind::MathFunction {
                            kind: math_kind,
                            args: arg_annots,
                        },
                    })
                }
            }
        }
    }
}

fn reject_null_system_args(name: &str, args: &[SystemArg]) -> Result<(), String> {
    if args.iter().any(|arg| matches!(arg, SystemArg::Null)) {
        Err(format!("{name} argument cannot be null"))
    } else {
        Ok(())
    }
}

// Compute the integer result-type meta for a binary op given annotated
// children. Returns `None` when the result type is real (LRM 5.1.5: arithmetic
// with at least one real operand), matching `expression_is_real`'s rules.
fn binary_result_meta(op: BinaryOp, lhs: &Annotated<'_>, rhs: &Annotated<'_>) -> Option<ExprMeta> {
    match op {
        BinaryOp::Add
        | BinaryOp::Subtract
        | BinaryOp::Multiply
        | BinaryOp::Divide
        | BinaryOp::Power => {
            if lhs.is_real() || rhs.is_real() {
                None
            } else {
                Some(combine_binary_meta(op, lhs.meta(), rhs.meta()))
            }
        }
        // Modulus, ===, !==, bitwise, shift: rejected on real by validator;
        // result type is integer in any case it's allowed.
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
            // Use the children's metas only when both are integer-typed;
            // for the operators above this is always the validated case.
            // Real-tainted operands hit the validator before any consumer
            // reads this meta, so a placeholder integer meta is harmless.
            let lhs_meta = lhs.meta.unwrap_or(ExprMeta {
                width: 1,
                signed: false,
                base: Base::Binary,
            });
            let rhs_meta = rhs.meta.unwrap_or(ExprMeta {
                width: 1,
                signed: false,
                base: Base::Binary,
            });
            Some(combine_binary_meta(op, lhs_meta, rhs_meta))
        }
        // Relational / equality / logical → 1-bit unsigned binary,
        // regardless of operand types.
        BinaryOp::LessThan
        | BinaryOp::GreaterThan
        | BinaryOp::LessThanOrEqual
        | BinaryOp::GreaterThanOrEqual
        | BinaryOp::Equal
        | BinaryOp::NotEqual
        | BinaryOp::LogicalAnd
        | BinaryOp::LogicalOr => Some(ExprMeta {
            width: 1,
            signed: false,
            base: Base::Binary,
        }),
    }
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
// Iterative implementation: an expression is real iff any sub-expression
// in a real-result position is real, so we OR-fold via a worklist. Whenever
// we discover a real-typed leaf or operator, return early; otherwise push
// the recursive children onto the worklist and continue. Every operator
// either short-circuits to a fixed result (no recursion needed) or falls
// into the "OR over some children" pattern, so no Combine frame is needed.
pub(crate) fn expression_is_real(expr: &Expr, session: &Session) -> bool {
    let mut work: Vec<&Expr> = vec![expr];
    while let Some(node) = work.pop() {
        match node {
            Expr::Literal(_) | Expr::StringLiteral(_) => {}
            Expr::RealLiteral(_) => return true,
            Expr::Grouped(inner) => work.push(inner),
            Expr::Unary { op, expr } => match op {
                UnaryOp::Plus | UnaryOp::Minus => work.push(expr),
                UnaryOp::BitwiseNot
                | UnaryOp::LogicalNot
                | UnaryOp::ReductionAnd
                | UnaryOp::ReductionNand
                | UnaryOp::ReductionOr
                | UnaryOp::ReductionNor
                | UnaryOp::ReductionXor
                | UnaryOp::ReductionXnor => {}
            },
            Expr::Binary { op, lhs, rhs } => match op {
                BinaryOp::Add
                | BinaryOp::Subtract
                | BinaryOp::Multiply
                | BinaryOp::Divide
                | BinaryOp::Power => {
                    work.push(lhs);
                    work.push(rhs);
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
                | BinaryOp::LogicalOr => {}
            },
            Expr::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                work.push(then_expr);
                work.push(else_expr);
            }
            Expr::Concatenation { .. } | Expr::Replication { .. } | Expr::ConcatSelect { .. } => {}
            // `$name(args)`: classify by name to decide result-type.
            //   - SignCast / BaseCast / unknown name → integer-typed
            //     (unknowns surface their diagnostic at validate time).
            //   - RealConversion: $itor / $bitstoreal yield real; $rtoi /
            //     $realtobits yield integer (LRM 17.8).
            //   - MathFunction: every kind except $clog2 returns real
            //     (LRM 17.11).
            //   - Task: not really an expression — reporting "not real"
            //     routes the rejection through the integer pipeline,
            //     which surfaces the task-in-expression diagnostic.
            Expr::SystemCall { name, .. } => match classify_system_call(name) {
                Ok(SystemCallKind::Function(SystemFunction::RealConversion(
                    RealConversionKind::IntegerToReal | RealConversionKind::BitsToReal,
                ))) => return true,
                Ok(SystemCallKind::Function(SystemFunction::Math(kind)))
                    if kind.is_real_result() =>
                {
                    return true;
                }
                _ => {}
            },
            // An identifier is real-typed iff it resolves to a `real` reg
            // (LRM 4.8). Unknown names resolve to integer here so the
            // downstream integer pipeline can surface the "undeclared
            // identifier" diagnostic at its usual position.
            Expr::Identifier(name) => {
                if session.lookup(name).is_some_and(|reg| reg.is_real()) {
                    return true;
                }
            }
            // Bit-select / part-select on a vector reg is always
            // integer-typed (LRM 4.7). A real-array element select
            // (`r[i]`) yields real. A select on a scalar `real` is
            // prohibited (LRM 4.8.1), but this runs during annotation —
            // ahead of the validator that rejects it — so report
            // integer-typed and let `annotate_select` surface the
            // diagnostic instead of aborting here.
            Expr::Select { name, kind, inner } => {
                let real_array_element = matches!(kind, SelectKind::Bit { .. })
                    && inner.is_none()
                    && session.lookup(name).is_some_and(|reg| reg.is_real_array());
                if real_array_element {
                    return true;
                }
            }
            Expr::Truncated => unreachable!(
                "Expr::Truncated is a display-only sentinel; never reaches expression_is_real"
            ),
        }
    }
    false
}
