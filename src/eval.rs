use std::collections::HashMap;

use num_bigint::{BigInt, BigUint, Sign};
use num_traits::{FromPrimitive, One, Signed, ToPrimitive, Zero};

use crate::parser::{
    BinaryOp, Expr, LValue, MathFunctionKind, RealConversionKind, SelectKind, SystemArg, UnaryOp,
    string_literal_spec,
};
use crate::system_call::{
    SystemCallKind, SystemFunction, classify_system_call, task_in_expression_error,
};
use crate::value;
use crate::value::{
    Base, DisplayStyle, IntegerValue, LogicBit, Value, bits_to_biguint, bitwise_and_bits,
    bitwise_not_bit, bitwise_or_bits, bitwise_xnor_bits, bitwise_xor_bits,
};
use crate::{RegRange, RegValue, Session};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ExprMeta {
    width: usize,
    signed: bool,
    // Inferred display base — leftmost operand wins for binary ops.
    // Used when constructing arithmetic results; ignored when ExprMeta is
    // passed downward as context (literals keep their own base).
    base: Base,
}

// Annotated AST: a parallel tree built from `Expr` in a single bottom-up pass.
// Each node caches its result-type meta and its real/integer dispatch flag, so
// validators and evaluators can read those in O(1) instead of re-walking the
// subtree at every Binary node (which made the integer pipeline O(N²) on a
// long left-leaning chain like `1+1+...+1`).
//
// `meta` is `None` for nodes whose result type is real (the f64 pipeline) or
// whose result has no well-defined integer interpretation (`SystemTask`,
// which the validator rejects as an expression). The integer pipeline's
// `meta()` accessor unwraps with a panic — by the time evaluation reaches an
// integer helper, the dispatch in `evaluate_expr` has already routed real
// expressions to the real pipeline, so a panic here is a real bug.
//
// `expr` keeps a back-reference to the original `Expr` so leaves (`Literal`,
// `Identifier`, `Select`, …) and primitive operator data (`UnaryOp`,
// `BinaryOp`, `MathFunctionKind`, …) can be read directly from `expr` rather
// than duplicated into `kind`. `kind` only mirrors the structural children
// the evaluators need to recurse into.
#[derive(Debug)]
pub(crate) struct Annotated<'a> {
    expr: &'a Expr,
    meta: Option<ExprMeta>,
    kind: AnnotatedKind<'a>,
}

#[derive(Debug)]
enum AnnotatedKind<'a> {
    // `Literal`, `RealLiteral`, `Identifier`, `Select`. The evaluator
    // reads sub-data from `Annotated::expr` directly.
    Leaf,
    Grouped(Box<Annotated<'a>>),
    Unary(Box<Annotated<'a>>),
    Binary {
        lhs: Box<Annotated<'a>>,
        rhs: Box<Annotated<'a>>,
    },
    Conditional {
        cond: Box<Annotated<'a>>,
        then_arm: Box<Annotated<'a>>,
        else_arm: Box<Annotated<'a>>,
    },
    Concatenation(Vec<Annotated<'a>>),
    Replication {
        count: Box<Annotated<'a>>,
        items: Vec<Annotated<'a>>,
    },
    // System-call result classes. `annotate` resolves the parsed name
    // via `classify_system_call` and stores the typed kind here so the
    // evaluator reads it off the node without re-classifying.
    SignCast {
        signed: bool,
        arg: Box<Annotated<'a>>,
    },
    BaseCast {
        base: Base,
        arg: Box<Annotated<'a>>,
    },
    RealConversion {
        kind: RealConversionKind,
        arg: Box<Annotated<'a>>,
    },
    MathFunction {
        kind: MathFunctionKind,
        args: Vec<Annotated<'a>>,
    },
    // `$finish` / `$stop` in expression position. Leaf-shaped — args
    // were parsed for syntactic validity and dropped by `annotate`
    // since they're never evaluated (LRM 17.4: vcal prints no exit
    // diagnostic). At the AST root the lib driver hoists this to an
    // exit; anywhere else, every walker rejects it as
    // "task in expression position".
    SystemTask,
}

impl<'a> Annotated<'a> {
    pub(crate) fn is_real(&self) -> bool {
        self.meta.is_none()
    }

    fn meta(&self) -> ExprMeta {
        self.meta
            .expect("integer meta queried on real-typed or non-expression node")
    }
}

// Iterative Drop for Annotated, mirroring `impl Drop for Expr` in parser.rs.
// `AnnotatedKind` carries `Box<Annotated>` / `Vec<Annotated>` children, so a
// 10^5-deep input would otherwise drop via 10^5 recursive auto-drop frames
// once the iterative `annotate` builds such a chain successfully. The
// placeholder is `AnnotatedKind::Leaf` reusing the parent's `&'a Expr` (refs
// are `Copy`) and `meta = None` — a cheap, allocation-free leaf.
impl<'a> Drop for Annotated<'a> {
    fn drop(&mut self) {
        let mut work: Vec<Annotated<'a>> = Vec::new();
        steal_annotated_children(self, &mut work);
        while let Some(mut victim) = work.pop() {
            steal_annotated_children(&mut victim, &mut work);
            // victim's children are now leaves; auto-drop at end of this
            // iteration is shallow.
        }
    }
}

fn steal_annotated_children<'a>(annot: &mut Annotated<'a>, out: &mut Vec<Annotated<'a>>) {
    let parent_expr = annot.expr;
    let placeholder = || Annotated {
        expr: parent_expr,
        meta: None,
        kind: AnnotatedKind::Leaf,
    };
    match &mut annot.kind {
        AnnotatedKind::Leaf | AnnotatedKind::SystemTask => {}
        AnnotatedKind::Grouped(inner) | AnnotatedKind::Unary(inner) => {
            out.push(std::mem::replace(inner.as_mut(), placeholder()));
        }
        AnnotatedKind::SignCast { arg, .. }
        | AnnotatedKind::BaseCast { arg, .. }
        | AnnotatedKind::RealConversion { arg, .. } => {
            out.push(std::mem::replace(arg.as_mut(), placeholder()));
        }
        AnnotatedKind::Binary { lhs, rhs } => {
            out.push(std::mem::replace(lhs.as_mut(), placeholder()));
            out.push(std::mem::replace(rhs.as_mut(), placeholder()));
        }
        AnnotatedKind::Conditional {
            cond,
            then_arm,
            else_arm,
        } => {
            out.push(std::mem::replace(cond.as_mut(), placeholder()));
            out.push(std::mem::replace(then_arm.as_mut(), placeholder()));
            out.push(std::mem::replace(else_arm.as_mut(), placeholder()));
        }
        AnnotatedKind::Concatenation(items) | AnnotatedKind::MathFunction { args: items, .. } => {
            out.append(items);
        }
        AnnotatedKind::Replication { count, items } => {
            out.push(std::mem::replace(count.as_mut(), placeholder()));
            out.append(items);
        }
    }
}

// Annotate the expression tree once, bottom-up. Returns structural errors
// (undeclared identifier, array used as value, system task in an expression
// position) in unprefixed form — the entry-point caller wraps them with
// "Semantic error: " when they surface during the static-semantic phase, and
// passes them through unprefixed when they surface at evaluation time, both
// matching today's error-prefix convention.
//
// Real-result branches store `meta = None`; integer branches store
// `Some(meta)` computed from the children's metas using the same combination
// rules `infer_expr_meta` previously walked the tree for. The Select arm
// stays a leaf in the annotated tree — its index / range sub-expressions are
// short, self-determined, and outside the chain spine, so re-walking them in
// the legacy helpers is a non-issue for the O(N²) regression.
// Iterative CES (control-environment-store) driver: each `Visit` task
// inspects an `Expr` node and, for parent shapes, schedules a `Combine` task
// followed by `Visit` tasks for the children (pushed in reverse so they pop
// in source order). Each `Combine` pops its child annotations off `vals` and
// assembles the parent `Annotated`. Leaves (Literal, RealLiteral, SystemTask,
// Identifier, Select, Truncated-via-unreachable) push their annotation
// directly with no Combine. This keeps Rust stack depth O(1) regardless of
// input nesting depth — paired with `impl Drop for Annotated` above so the
// resulting deep `Box<Annotated>` chain doesn't crash at end-of-scope.
enum AnnotateTask<'a> {
    Visit(&'a Expr),
    Combine(AnnotateCombiner<'a>),
}

enum AnnotateCombiner<'a> {
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
        count_expr: &'a Expr,
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
                        count_expr: count,
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
                    // Inner index / range sub-expressions stay un-annotated —
                    // they're self-determined, short, and not in the chain
                    // spine; the legacy helpers still handle them. Real-typed
                    // selects (real-array element via `r[i]`, no inner
                    // select) carry `meta = None` to route through the f64
                    // pipeline; the validator surfaces the structural
                    // diagnostic for illegal forms before any meta consumer
                    // runs.
                    let is_real_select = matches!(session.lookup(name), Some(reg) if reg.is_real_array())
                        && matches!(kind, SelectKind::Bit { .. })
                        && inner.is_none();
                    let meta = if is_real_select {
                        None
                    } else {
                        Some(infer_select_meta(name, kind, inner.as_deref(), session)?)
                    };
                    vals.push(Annotated {
                        expr,
                        meta,
                        kind: AnnotatedKind::Leaf,
                    });
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
            // validator (concat requires definite bit widths); the meta
            // built here is only consumed if validation passes.
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
        AnnotateCombiner::Replication {
            expr,
            count_expr,
            item_count,
        } => {
            let item_annots = pop_n(vals, item_count);
            let count_annot = vals.pop().expect("Replication: count missing");
            // Width depends on the count's evaluated value. Skip the eager
            // evaluation when the count or any item is real-typed — the
            // validator surfaces "replication count cannot be real" /
            // "replication operand cannot be real" with clearer framing,
            // and trying to evaluate as integer first would only swap that
            // for "real value cannot be used as an integer expression here".
            let count_value = if count_annot.is_real() || item_annots.iter().any(|i| i.is_real()) {
                None
            } else {
                Some(evaluate_replication_count_allow_zero(count_expr, session)?)
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

fn system_arg_expr<'a>(
    name: &str,
    args: &'a [SystemArg],
    index: usize,
) -> Result<&'a Expr, String> {
    match args.get(index) {
        Some(SystemArg::Expr(expr)) => Ok(expr),
        Some(SystemArg::Null) => Err(format!("{name} argument cannot be null")),
        None => Err(format!("{name} is missing argument {}", index + 1)),
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

// Static-semantics pre-pass. Every public expression-evaluation entry point
// runs `semantic_check` before touching the evaluator, so structural errors
// like real-typed select indices, $bitstoreal width mismatches, or invalid
// system-task uses surface here — *before* runtime behaviour like short-circuit
// branch choice, zero-rep collapse, or x-bit propagation could hide them.
// Errors raised here carry the "Semantic error: " prefix so the rejection
// stage is visible to the user, paralleling the "Syntax error: " prefix that
// `parse_statements`'s call site adds to lexer/parser errors.
pub(crate) fn semantic_check(expr: &Expr, session: &Session) -> Result<(), String> {
    let annotated = annotate(expr, session).map_err(|e| format!("Semantic error: {e}"))?;
    validate_annotated(&annotated, session).map_err(|e| format!("Semantic error: {e}"))
}

pub(crate) fn evaluate_expr(expr: &Expr, session: &Session) -> Result<Value, String> {
    let annotated = annotate(expr, session).map_err(|e| format!("Semantic error: {e}"))?;
    validate_annotated(&annotated, session).map_err(|e| format!("Semantic error: {e}"))?;
    if annotated.is_real() {
        evaluate_annotated_as_real(&annotated, session).map(Value::Real)
    } else {
        evaluate_annotated(&annotated, None, session).map(Value::Integer)
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
    let annotated = annotate(rhs, session).map_err(|e| format!("Semantic error: {e}"))?;
    validate_annotated(&annotated, session).map_err(|e| format!("Semantic error: {e}"))?;
    if annotated.is_real() {
        let real_val = evaluate_annotated_as_real(&annotated, session)?;
        return Ok(match real_to_integer_bigint(real_val) {
            Some(bigint) => IntegerValue::from_bigint(bigint, width, signed, base).with_weak_base(),
            None => IntegerValue::all_x(width, signed, base).with_weak_base(),
        });
    }
    let context = ExprMeta {
        width,
        signed,
        base,
    };
    evaluate_annotated(&annotated, Some(context), session)
}

// Self-determined evaluation of an integer-typed constant expression
// (used by the reg-declaration range halves). Mirrors the `None` context
// path the evaluator takes for the top-level expression in a calculator
// line.
pub(crate) fn evaluate_constant_expr(
    expr: &Expr,
    session: &Session,
) -> Result<IntegerValue, String> {
    let annotated = annotate(expr, session).map_err(|e| format!("Semantic error: {e}"))?;
    validate_annotated(&annotated, session).map_err(|e| format!("Semantic error: {e}"))?;
    evaluate_annotated(&annotated, None, session)
}

// Self-determined integer evaluation that goes through the iterative
// `annotate` + `evaluate_annotated` pipeline. Used wherever a leaf-side
// helper would otherwise call the recursive `evaluate_expr_in_context`
// on a user-supplied sub-expression: bit-select index, indexed-base,
// part-select range halves, and the bigint-exponent walker's fallback
// for non-arith / leaf-ish shapes. The enclosing expression has
// already been validated (either by `validate_annotated` at the top of
// `evaluate_expr`/`evaluate_assignment_rhs`, or by per-form structural
// checks like `validate_select_kind_structure`), so we skip
// `validate_annotated` here.
fn evaluate_subexpr_as_integer(expr: &Expr, session: &Session) -> Result<IntegerValue, String> {
    let annotated = annotate(expr, session)?;
    evaluate_annotated(&annotated, None, session)
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
            Expr::Concatenation { .. } | Expr::Replication { .. } => {}
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
            // (`r[i]`) yields real. Selects on a scalar `real` are
            // prohibited (LRM 4.8.1) and the validator rejects them
            // before this function runs.
            Expr::Select { name, kind, inner } => match session.lookup(name) {
                Some(reg)
                    if reg.is_real_array()
                        && matches!(kind, SelectKind::Bit { .. })
                        && inner.is_none() =>
                {
                    return true;
                }
                Some(reg) if reg.is_real() => {
                    unreachable!(
                        "validator rejects select on scalar real `{name}` before evaluation (LRM 4.8.1)"
                    );
                }
                _ => {}
            },
            Expr::Truncated => unreachable!(
                "Expr::Truncated is a display-only sentinel; never reaches expression_is_real"
            ),
        }
    }
    false
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

// Real-pipeline combiners. Folded into the unified work loop in
// `evaluate_annotated` (alongside the integer-pipeline `EvalCombiner`):
// Real combiners read/write the f64 value stack and may also bridge
// to/from the IntegerValue stack at $itor / $bitstoreal / implicit
// LRM 3.5.3 coercion points. Putting both pipelines on one work stack
// is what eliminates Rust-stack growth at deep alternating shapes like
// `$rtoi($itor($rtoi($itor(...))))` — the previous design had each
// crossing call into the other driver's loop, adding one C-stack frame
// per crossing.
enum RealCombiner<'b, 'a: 'b> {
    /// Unary `+` is identity; `-` negates. Pops 1 f64.
    UnaryPlus,
    UnaryMinus,
    /// Add / Subtract / Multiply / Divide / Power on real operands.
    /// Pops 2 f64s (lhs, rhs in push order).
    BinaryArith {
        op: BinaryOp,
    },
    /// Conditional with x/z cond — both branches evaluated and merged
    /// per `f64::to_bits()` agreement. Pops 2 f64s (then, else).
    ConditionalRealMerge,
    /// Real-result math function (`$pow`, `$ln`, `$atan2`, ...). Pops
    /// `arity` f64s in push order; applies the math function; pushes the
    /// f64 result.
    MathFunction {
        kind: MathFunctionKind,
        arity: usize,
    },
    /// LRM 3.5.3 implicit integer→real coercion — for an integer-typed
    /// sub-expression appearing in a real chain (e.g., `1.0 + 1`, or
    /// `$pow(2, 1)` where `2` is an integer literal). Pops 1 IntegerValue
    /// from `int_vals`, pushes `integer_value_to_f64(...)` to `real_vals`.
    CoerceFromInteger,
    /// `$itor` argument consumed: same shape as `CoerceFromInteger` but
    /// emitted explicitly so the rule in [doc/non-standard.md] stays
    /// traceable. Pops 1 IntegerValue, pushes 1 f64.
    ItorFromInt,
    /// `$bitstoreal` argument consumed: pops 1 IntegerValue (must have
    /// width = 64; the validator already enforces this, the runtime check
    /// here is defence-in-depth), pushes `bits_value_to_real(...)`.
    BitstoRealFromInt,
    /// Real-result conditional with integer cond. Pops 1 IntegerValue
    /// (cond) from `int_vals`, reduces via `logical_value`, then pushes
    /// `VisitReal/coerce` for the chosen branch (or both branches +
    /// `ConditionalRealMerge` for x/z cond).
    DispatchIntCondRealResult {
        then_arm: &'b Annotated<'a>,
        else_arm: &'b Annotated<'a>,
    },
    /// Real-result conditional with real cond. Pops 1 f64 from
    /// `real_vals`, reduces via `logical_value_of_real`, dispatches.
    DispatchRealCondRealResult {
        then_arm: &'b Annotated<'a>,
        else_arm: &'b Annotated<'a>,
    },
}

// Apply a real-result math function to `arity` operands popped from
// `real_vals`. Order on the stack matches push order (lhs at the bottom
// of the popped slice, rhs at the top).
fn apply_real_math_function(kind: MathFunctionKind, args: &[f64]) -> f64 {
    if args.len() == 1 {
        let x = args[0];
        return match kind {
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
        };
    }
    let x = args[0];
    let y = args[1];
    match kind {
        // LRM 17.11 + README "Real numbers": $pow shares f64::powf with
        // the `**` operator on reals, so corner-case results
        // (0.0**0.0=1.0, negative**non-integral=NaN, 0.0**neg=±∞) match.
        MathFunctionKind::Pow => x.powf(y),
        MathFunctionKind::Atan2 => x.atan2(y),
        MathFunctionKind::Hypot => x.hypot(y),
        _ => unreachable!("kind handled by other arity branch"),
    }
}

fn validate_select_kind_structure(kind: &SelectKind, session: &Session) -> Result<(), String> {
    match kind {
        SelectKind::Bit { index } => validate_expr_structure(index, session),
        SelectKind::PartConst { msb, lsb } => {
            validate_expr_structure(msb, session)?;
            validate_expr_structure(lsb, session)
        }
        SelectKind::PartIndexedUp { base, width } | SelectKind::PartIndexedDown { base, width } => {
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

// Iterative validator for raw `&Expr`. Used only on Select index / range
// sub-expressions today (the modern annotate pipeline routes everything
// else through `validate_annotated`), but a deep chain inside
// `a[1+1+...+1]` still has to survive — so this gets the same flatten-
// then-drain treatment as `validate_annotated`.
//
// `PostCheck` tasks defer node-local checks until after children's
// subtrees have been validated, matching the original recursive
// walker's diagnostic priority — e.g., for `~undef_var` the
// "undeclared identifier" surfaces before the bitwise-on-real check
// even has a chance to fire.
enum ExprValidateTask<'a> {
    Visit(&'a Expr),
    PostCheck(ExprValidatePostCheck<'a>),
    ConcatItem { item: &'a Expr, role: &'static str },
    PostConcatItemRealCheck { item: &'a Expr, role: &'static str },
    PostCollectBits { items: &'a [Expr] },
}

enum ExprValidatePostCheck<'a> {
    UnaryOpReal {
        op: UnaryOp,
        operand: &'a Expr,
    },
    BinaryOpReal {
        op: BinaryOp,
        lhs: &'a Expr,
        rhs: &'a Expr,
    },
    SignCastArgReal {
        signed: bool,
        arg: &'a Expr,
    },
    BaseCastArgReal {
        base: Base,
        arg: &'a Expr,
    },
    ItorArgReal {
        arg: &'a Expr,
    },
    BitsToRealArgChecks {
        arg: &'a Expr,
    },
    MathFunctionArgChecks {
        kind: MathFunctionKind,
        first_arg: &'a Expr,
        whole: &'a Expr,
    },
    ReplicationCountReal {
        count: &'a Expr,
    },
    /// Constant-evaluates the count and rejects negative / unknown-bits /
    /// out-of-range values (and rejects zero in the strict variant).
    /// Runs after Visit(count) has structurally validated the count
    /// expression, and after every item has been validated.
    ReplicationCountCheck {
        count: &'a Expr,
        count_check: fn(&Expr, &Session) -> Result<usize, String>,
    },
}

fn validate_expr_structure(expr: &Expr, session: &Session) -> Result<(), String> {
    let mut work: Vec<ExprValidateTask> = vec![ExprValidateTask::Visit(expr)];
    while let Some(task) = work.pop() {
        match task {
            ExprValidateTask::Visit(node) => visit_expr_structure(node, &mut work, session)?,
            ExprValidateTask::PostCheck(check) => match check {
                ExprValidatePostCheck::UnaryOpReal { op, operand } => {
                    if expression_is_real(operand, session)
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
                            unary_op_name(op)
                        ));
                    }
                }
                ExprValidatePostCheck::BinaryOpReal { op, lhs, rhs } => {
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
                                    binary_op_name(op)
                                ));
                            }
                        }
                    }
                }
                ExprValidatePostCheck::SignCastArgReal { signed, arg } => {
                    if expression_is_real(arg, session) {
                        return Err(format!(
                            "{} argument cannot be real",
                            if signed { "$signed" } else { "$unsigned" }
                        ));
                    }
                }
                ExprValidatePostCheck::BaseCastArgReal { base, arg } => {
                    if expression_is_real(arg, session) {
                        return Err(format!("{} argument cannot be real", base_cast_name(base)));
                    }
                }
                ExprValidatePostCheck::ItorArgReal { arg } => {
                    if expression_is_real(arg, session) {
                        return Err("$itor argument cannot be real".to_string());
                    }
                }
                ExprValidatePostCheck::BitsToRealArgChecks { arg } => {
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
                }
                ExprValidatePostCheck::MathFunctionArgChecks {
                    kind,
                    first_arg,
                    whole,
                } => {
                    if !kind.is_real_result() {
                        if expression_is_real(first_arg, session) {
                            return Err(format!("{} argument cannot be real", kind.name()));
                        }
                        let _ = infer_expr_meta(whole, session)?;
                    }
                }
                ExprValidatePostCheck::ReplicationCountReal { count } => {
                    if expression_is_real(count, session) {
                        return Err("replication count cannot be real".to_string());
                    }
                }
                ExprValidatePostCheck::ReplicationCountCheck { count, count_check } => {
                    let _ = count_check(count, session)?;
                }
            },
            ExprValidateTask::ConcatItem { item, role } => {
                let unwrapped = unwrap_grouped(item);
                if let Expr::Replication { count, items } = unwrapped {
                    push_replication_validation_expr(
                        count,
                        items,
                        evaluate_replication_count_allow_zero,
                        &mut work,
                    );
                } else {
                    work.push(ExprValidateTask::PostConcatItemRealCheck { item, role });
                    work.push(ExprValidateTask::Visit(item));
                }
            }
            ExprValidateTask::PostConcatItemRealCheck { item, role } => {
                if expression_is_real(item, session) {
                    return Err(format!("{role} operand cannot be real"));
                }
            }
            ExprValidateTask::PostCollectBits { items } => {
                let _ = collect_concatenation_bits(items, session)?;
            }
        }
    }
    Ok(())
}

fn visit_expr_structure<'a>(
    node: &'a Expr,
    work: &mut Vec<ExprValidateTask<'a>>,
    session: &Session,
) -> Result<(), String> {
    match node {
        // MAX_BIT_WIDTH cap was historically gated in the parser, but
        // `LiteralSpec` defers materialization to eval time precisely so
        // this check can live in the validator phase. Without this gate,
        // `9999999999999'd1` would parse cleanly (the spec carries width as
        // a number, not bits) and only blow up at `materialize()` time.
        Expr::Literal(spec) => value::ensure_bit_width(spec.width, "literal")?,
        Expr::StringLiteral(bytes) => {
            value::ensure_bit_width(bytes.len().max(1).saturating_mul(8), "string literal")?
        }
        Expr::RealLiteral(_) => {}
        Expr::Grouped(inner) => work.push(ExprValidateTask::Visit(inner)),
        Expr::Unary { op, expr } => {
            work.push(ExprValidateTask::PostCheck(
                ExprValidatePostCheck::UnaryOpReal {
                    op: *op,
                    operand: expr,
                },
            ));
            work.push(ExprValidateTask::Visit(expr));
        }
        Expr::Binary { op, lhs, rhs } => {
            work.push(ExprValidateTask::PostCheck(
                ExprValidatePostCheck::BinaryOpReal { op: *op, lhs, rhs },
            ));
            work.push(ExprValidateTask::Visit(rhs));
            work.push(ExprValidateTask::Visit(lhs));
        }
        Expr::Conditional {
            cond,
            then_expr,
            else_expr,
        } => {
            work.push(ExprValidateTask::Visit(else_expr));
            work.push(ExprValidateTask::Visit(then_expr));
            work.push(ExprValidateTask::Visit(cond));
        }
        Expr::Concatenation { items } => {
            work.push(ExprValidateTask::PostCollectBits { items });
            for item in items.iter().rev() {
                work.push(ExprValidateTask::ConcatItem {
                    item,
                    role: "concatenation",
                });
            }
        }
        Expr::Replication { count, items } => {
            push_replication_validation_expr(count, items, evaluate_replication_count, work);
        }
        Expr::SystemCall { name, args } => {
            // Classify resolves the name (surfacing "unknown system
            // identifier" up front) and selects the per-kind structural
            // checks. Arity is already enforced by `annotate` before
            // structural validation runs, so a wrong-arity SystemCall
            // never reaches here.
            let kind = classify_system_call(name)?;
            match kind {
                SystemCallKind::Function(SystemFunction::SignCast { signed }) => {
                    let arg = system_arg_expr(name, args, 0)?;
                    work.push(ExprValidateTask::PostCheck(
                        ExprValidatePostCheck::SignCastArgReal { signed, arg },
                    ));
                    work.push(ExprValidateTask::Visit(arg));
                }
                SystemCallKind::Function(SystemFunction::BaseCast(base)) => {
                    let arg = system_arg_expr(name, args, 0)?;
                    work.push(ExprValidateTask::PostCheck(
                        ExprValidatePostCheck::BaseCastArgReal { base, arg },
                    ));
                    work.push(ExprValidateTask::Visit(arg));
                }
                SystemCallKind::Function(SystemFunction::RealConversion(conv_kind)) => {
                    let arg = system_arg_expr(name, args, 0)?;
                    match conv_kind {
                        RealConversionKind::RealToInteger | RealConversionKind::RealToBits => {}
                        RealConversionKind::IntegerToReal => {
                            work.push(ExprValidateTask::PostCheck(
                                ExprValidatePostCheck::ItorArgReal { arg },
                            ));
                        }
                        RealConversionKind::BitsToReal => {
                            work.push(ExprValidateTask::PostCheck(
                                ExprValidatePostCheck::BitsToRealArgChecks { arg },
                            ));
                        }
                    }
                    work.push(ExprValidateTask::Visit(arg));
                }
                SystemCallKind::Function(SystemFunction::Math(math_kind)) => {
                    let first_arg = system_arg_expr(name, args, 0)?;
                    work.push(ExprValidateTask::PostCheck(
                        ExprValidatePostCheck::MathFunctionArgChecks {
                            kind: math_kind,
                            first_arg,
                            whole: node,
                        },
                    ));
                    for arg in args.iter().rev() {
                        match arg {
                            SystemArg::Expr(arg) => work.push(ExprValidateTask::Visit(arg)),
                            SystemArg::Null => {
                                return Err(format!("{name} argument cannot be null"));
                            }
                        }
                    }
                }
                SystemCallKind::Task(_) => return Err(task_in_expression_error(name)),
            }
        }
        Expr::Identifier(name) => {
            let reg = session
                .lookup(name)
                .ok_or_else(|| format!("undeclared identifier: {name}"))?;
            // Real identifiers route through the f64 pipeline, so the
            // vector-only check would wrongly reject them here. Arrays
            // are still rejected because their value-as-a-whole has no
            // numeric type (LRM 4.9 only allows element selects).
            if !reg.is_real() {
                let _ = reg.require_vector(name)?;
            }
        }
        Expr::Select { name, kind, inner } => {
            validate_select_expr_structure(name, kind, inner.as_deref(), session)?;
        }
        Expr::Truncated => unreachable!(
            "Expr::Truncated is a display-only sentinel; never reaches validate_expr_structure"
        ),
    }
    Ok(())
}

fn push_replication_validation_expr<'a>(
    count: &'a Expr,
    items: &'a [Expr],
    count_check: fn(&Expr, &Session) -> Result<usize, String>,
    work: &mut Vec<ExprValidateTask<'a>>,
) {
    // Schedule the same sequence the recursive walker did:
    //   1. Visit(count) — full structural recursion on count
    //   2. PostCheck::ReplicationCountReal — local real-rejection
    //   3. ConcatItem(item) for each item — recurses + role real-check
    //   4. PostCollectBits + count_check  — final whole-replication checks
    // The strict/lenient count_check choice is encoded in step 4 only;
    // by the time we reach it, count's structural validation has
    // succeeded, so feeding it to evaluate_replication_count{,_allow_zero}
    // is safe.
    work.push(ExprValidateTask::PostCollectBits { items });
    work.push(ExprValidateTask::PostCheck(
        ExprValidatePostCheck::ReplicationCountCheck { count, count_check },
    ));
    for item in items.iter().rev() {
        work.push(ExprValidateTask::ConcatItem {
            item,
            role: "replication",
        });
    }
    work.push(ExprValidateTask::PostCheck(
        ExprValidatePostCheck::ReplicationCountReal { count },
    ));
    work.push(ExprValidateTask::Visit(count));
}

// Annotated counterpart to `validate_expr_structure`. Reads `is_real()` and
// `meta()` from precomputed annotations instead of re-walking every Binary
// node's lhs/rhs to ask the same questions, dropping the old O(N²) helper-walk
// pattern to O(N) on long chains. Sub-expressions that aren't annotated yet
// (index / range expressions inside `SelectKind`) are still validated by the
// legacy `validate_expr_structure` — those are short, self-determined, and
// outside the chain spine that drove the regression.
// Iterative implementation. Each parent's node-local checks (real-operand
// rejection, $bitstoreal width, $clog2 real-arg, replication count_check,
// concatenation bit-collection) run eagerly at Visit time, then child
// `Visit` tasks are pushed for structural recursion. `ConcatItem` carries
// the role string used in the "X operand cannot be real" diagnostic and
// special-cases a Replication directly inside a concat list (lenient
// zero-count rule per LRM 5.1.14).
//
// Annotate already errors out for undeclared identifiers, so eager local
// checks here can read child `is_real()` / `meta()` from the cached
// Annotated nodes without re-walking subtrees, and the ordering swap
// (parent local checks before child structural recursion) doesn't
// surface different undeclared-identifier diagnostics than the original
// recursive walker — annotate is the only producer of that error and it
// runs before validate_annotated.
enum AnnValidateTask<'b, 'a: 'b> {
    Visit(&'b Annotated<'a>),
    /// First half of a concatenation/replication item dispatch. Unwraps
    /// any leading Grouped, branches into a lenient-replication walk if
    /// the unwrapped item is a Replication, otherwise schedules
    /// `Visit(item)` followed by `PostConcatItemRealCheck` so the
    /// structural recursion runs first and the role-tagged real-check
    /// runs after — matching the original recursive walker, which
    /// rejected `{$finish, ...}` with the system-task error before any
    /// real-operand check could fire.
    ConcatItem {
        item: &'b Annotated<'a>,
        role: &'static str,
    },
    PostConcatItemRealCheck {
        item: &'b Annotated<'a>,
        role: &'static str,
    },
    /// Final pass for a Concatenation/Replication node, run after every
    /// item has been structurally validated. Enforces LRM 5.1.14's two
    /// list-level constraints:
    ///
    /// 1. each operand must have a definite width
    ///    (`is_indefinite_width` per item),
    /// 2. the joined width must be positive (`sum of meta().width`).
    ///
    /// Both reads are cached on the Annotated children, so this is O(N)
    /// in the operand count rather than O(N) in the subtree size — the
    /// O(N²) re-walk that used to live in `collect_concatenation_bits`
    /// is gone. The variant carries `&[Annotated]` directly; raw `&Expr`
    /// is no longer needed since width comes from cached meta.
    PostCheckConcatWidth {
        items: &'b [Annotated<'a>],
    },
}

fn validate_annotated(annot: &Annotated, session: &Session) -> Result<(), String> {
    let mut work: Vec<AnnValidateTask> = vec![AnnValidateTask::Visit(annot)];
    while let Some(task) = work.pop() {
        match task {
            AnnValidateTask::Visit(node) => visit_annotated(node, &mut work, session)?,
            AnnValidateTask::ConcatItem { item, role } => {
                let unwrapped = unwrap_grouped_annotated(item);
                if let AnnotatedKind::Replication { count, items } = &unwrapped.kind {
                    push_replication_validation_annotated(
                        count, items, /* strict = */ false, &mut work, session,
                    )?;
                } else {
                    // Schedule Visit first (surfaces system-task / structural
                    // errors), then the role real-check after.
                    work.push(AnnValidateTask::PostConcatItemRealCheck { item, role });
                    work.push(AnnValidateTask::Visit(item));
                }
            }
            AnnValidateTask::PostConcatItemRealCheck { item, role } => {
                if item.is_real() {
                    return Err(format!("{role} operand cannot be real"));
                }
            }
            AnnValidateTask::PostCheckConcatWidth { items } => {
                let mut total_width: usize = 0;
                for item in items.iter() {
                    if is_indefinite_width(item.expr) {
                        return Err("concatenation operand has indefinite width".to_string());
                    }
                    total_width = total_width.saturating_add(item.meta().width);
                }
                if total_width == 0 {
                    return Err(
                        "concatenation must have at least one operand with positive size"
                            .to_string(),
                    );
                }
            }
        }
    }
    Ok(())
}

fn visit_annotated<'b, 'a: 'b>(
    annot: &'b Annotated<'a>,
    work: &mut Vec<AnnValidateTask<'b, 'a>>,
    session: &Session,
) -> Result<(), String> {
    match &annot.kind {
        AnnotatedKind::Leaf => match annot.expr {
            // Same MAX_BIT_WIDTH cap as the structural validator
            // (visit_expr_structure) — covers paths that go through the
            // Annotated tree directly without the structural pass first
            // (e.g. evaluate_subexpr_as_integer's pre-validated callers
            // that still hit literal leaves through this driver).
            Expr::Literal(spec) => value::ensure_bit_width(spec.width, "literal")?,
            Expr::StringLiteral(bytes) => {
                value::ensure_bit_width(bytes.len().max(1).saturating_mul(8), "string literal")?
            }
            Expr::RealLiteral(_) => {}
            Expr::Identifier(name) => {
                let reg = session
                    .lookup(name)
                    .ok_or_else(|| format!("undeclared identifier: {name}"))?;
                if !reg.is_real() {
                    let _ = reg.require_vector(name)?;
                }
            }
            Expr::Select { name, kind, inner } => {
                validate_select_expr_structure(name, kind, inner.as_deref(), session)?;
            }
            _ => unreachable!("AnnotatedKind::Leaf only wraps leaf-shaped Expr variants"),
        },
        AnnotatedKind::SystemTask => {
            // `$finish` / `$stop` in expression position. The lib driver
            // (`apply_stmt`) catches the top-level case before evaluation
            // runs and exits cleanly; any nested occurrence reaches here
            // and is rejected with the task-in-expression diagnostic.
            let name = match annot.expr {
                Expr::SystemCall { name, .. } => name.as_str(),
                _ => unreachable!("AnnotatedKind::SystemTask wraps only Expr::SystemCall"),
            };
            return Err(task_in_expression_error(name));
        }
        AnnotatedKind::Grouped(inner) => work.push(AnnValidateTask::Visit(inner)),
        AnnotatedKind::Unary(operand) => {
            let op = match annot.expr {
                Expr::Unary { op, .. } => *op,
                _ => unreachable!("AnnotatedKind::Unary only wraps Expr::Unary"),
            };
            if operand.is_real()
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
                    unary_op_name(op)
                ));
            }
            work.push(AnnValidateTask::Visit(operand));
        }
        AnnotatedKind::Binary { lhs, rhs } => {
            let op = match annot.expr {
                Expr::Binary { op, .. } => *op,
                _ => unreachable!("AnnotatedKind::Binary only wraps Expr::Binary"),
            };
            if lhs.is_real() || rhs.is_real() {
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
                            binary_op_name(op)
                        ));
                    }
                }
            }
            work.push(AnnValidateTask::Visit(rhs));
            work.push(AnnValidateTask::Visit(lhs));
        }
        AnnotatedKind::Conditional {
            cond,
            then_arm,
            else_arm,
        } => {
            work.push(AnnValidateTask::Visit(else_arm));
            work.push(AnnValidateTask::Visit(then_arm));
            work.push(AnnValidateTask::Visit(cond));
        }
        AnnotatedKind::Concatenation(items) => {
            // PostCheckConcatWidth runs LAST so per-item real-rejection
            // (via ConcatItem) and per-item structural errors surface
            // before the indefinite-width / positive-width checks —
            // matching the original recursive walker's diagnostic
            // priority. Width is read from cached meta(), so this no
            // longer re-walks subtrees the way `collect_concatenation_bits`
            // did.
            work.push(AnnValidateTask::PostCheckConcatWidth { items });
            for item in items.iter().rev() {
                work.push(AnnValidateTask::ConcatItem {
                    item,
                    role: "concatenation",
                });
            }
        }
        AnnotatedKind::Replication { count, items } => {
            push_replication_validation_annotated(
                count, items, /* strict = */ true, work, session,
            )?;
        }
        AnnotatedKind::SignCast { signed, arg } => {
            if arg.is_real() {
                return Err(format!(
                    "{} argument cannot be real",
                    if *signed { "$signed" } else { "$unsigned" }
                ));
            }
            work.push(AnnValidateTask::Visit(arg));
        }
        AnnotatedKind::BaseCast { base, arg } => {
            if arg.is_real() {
                return Err(format!("{} argument cannot be real", base_cast_name(*base)));
            }
            work.push(AnnValidateTask::Visit(arg));
        }
        AnnotatedKind::RealConversion { kind, arg } => {
            match kind {
                RealConversionKind::RealToInteger | RealConversionKind::RealToBits => {}
                RealConversionKind::IntegerToReal => {
                    if arg.is_real() {
                        return Err("$itor argument cannot be real".to_string());
                    }
                }
                RealConversionKind::BitsToReal => {
                    if arg.is_real() {
                        return Err("$bitstoreal argument cannot be real".to_string());
                    }
                    let arg_meta = arg.meta();
                    if arg_meta.width != 64 {
                        return Err(format!(
                            "$bitstoreal argument must be 64 bits wide, got {}",
                            arg_meta.width
                        ));
                    }
                }
            }
            work.push(AnnValidateTask::Visit(arg));
        }
        AnnotatedKind::MathFunction { kind, args } => {
            if !kind.is_real_result() && args[0].is_real() {
                return Err(format!("{} argument cannot be real", kind.name()));
            }
            for arg in args.iter().rev() {
                work.push(AnnValidateTask::Visit(arg));
            }
        }
    }
    Ok(())
}

fn push_replication_validation_annotated<'b, 'a: 'b>(
    count: &'b Annotated<'a>,
    items: &'b [Annotated<'a>],
    strict: bool,
    work: &mut Vec<AnnValidateTask<'b, 'a>>,
    session: &Session,
) -> Result<(), String> {
    // The original recursive validator did:
    //   1. validate(count)        // structural recursion
    //   2. real-check count       // local
    //   3. validate each item     // structural recursion
    //   4. count_check            // constant-eval count
    //   5. collect_concat_bits    // evaluate each item & combine
    // We keep the ordering: real-check runs first (cheap, local); the
    // eager count constant-eval runs before pushing the count's Visit
    // (mirroring how the legacy walker called `count_check` outside the
    // recursion); items are validated via ConcatItem; the final
    // indefinite-width / positive-width check fires through
    // PostCheckConcatWidth.
    //
    // The count is evaluated through `evaluate_annotated` (iterative)
    // rather than the legacy recursive `evaluate_expr_in_context`, so a
    // deep arithmetic chain inside the count no longer crashes.
    // `strict = true` rejects count = 0 (top-level replication); `strict
    // = false` allows zero (replication directly inside a concat list).
    if count.is_real() {
        return Err("replication count cannot be real".to_string());
    }
    let count_val = evaluate_annotated(count, None, session)?;
    if count_val.has_unknown_bits() {
        return Err("replication count contains unknown bits".to_string());
    }
    let count_bigint = count_val.as_bigint(count_val.signed);
    if count_bigint.sign() == Sign::Minus {
        return Err("replication count must be non-negative".to_string());
    }
    let count_usize = count_bigint
        .to_usize()
        .ok_or_else(|| "replication count too large".to_string())?;
    if strict && count_usize == 0 {
        return Err("replication count must be positive in this context".to_string());
    }
    work.push(AnnValidateTask::PostCheckConcatWidth { items });
    for item in items.iter().rev() {
        work.push(AnnValidateTask::ConcatItem {
            item,
            role: "replication",
        });
    }
    work.push(AnnValidateTask::Visit(count));
    Ok(())
}

fn unwrap_grouped_annotated<'a, 'b>(annot: &'b Annotated<'a>) -> &'b Annotated<'a> {
    let mut cur = annot;
    while let AnnotatedKind::Grouped(inner) = &cur.kind {
        cur = inner;
    }
    cur
}

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

// Leaf-shape integer evaluator. Reached only via `visit_eval`'s
// `AnnotatedKind::Leaf` arm, which `annotate` emits for `Literal`,
// `RealLiteral`, `SystemTask`, `Identifier`, and `Select` only. All
// other Expr shapes go through dedicated `AnnotatedKind` arms in the
// iterative CES driver, never through here — they can't appear as a
// leaf in the annotated tree, and the bigint-exponent walker now
// routes its sub-expressions through `evaluate_subexpr_as_integer`
// (annotate + evaluate_annotated) rather than re-entering the legacy
// per-shape helpers. Surface an `unreachable!` on anything else so a
// future regression that wires a non-leaf shape into the Leaf path
// fails loudly instead of silently re-introducing the recursive walker.
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
        Expr::Grouped(_)
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

// Iterative annotated-tree evaluator (CES driver: control-environment-store).
//
// The work stack contains `EvalTask::Visit` nodes that need to be evaluated
// and `EvalTask::Combine` continuations that take their popped child values
// and produce the parent's result. The value stack holds `IntegerValue`s
// in the order they were pushed, so each Combine pops them in reverse of
// the push order (rhs first, then lhs).
//
// Only the integer pipeline drives the iterative driver. Real-typed
// operands and the bigint exponent of `Power` route to the legacy
// `evaluate_expr_as_real` / `evaluate_expr_as_math_bigint` helpers, which
// are still recursive at this stage — P4 / P5 will iterate them. Deep
// chains in those positions still crash here; deep chains anywhere else
// (Binary integer, Unary integer, Conditional, Grouped, Concatenation,
// Replication, casts, conversions, integer math functions) are now O(1)
// in Rust stack regardless of nesting depth.
//
// Conditional needs a 2-stage Combine: first the cond_value is popped to
// decide which branch to evaluate (LRM 5.1.13: cond=0 → only else, cond=1
// → only then, cond=x/z → both branches and merge per-bit). A
// `ConditionalChoose` Combine handles the decision and pushes either one
// `Visit` (for definite cond) or a `Visit` for each branch plus a
// `ConditionalMerge` Combine (for x/z cond).
enum EvalTask<'b, 'a: 'b> {
    Visit {
        node: &'b Annotated<'a>,
        ctx: Option<ExprMeta>,
    },
    Combine(EvalCombiner<'b, 'a>),
    /// Visit a real-typed annotated node: leave 1 f64 on `real_vals`.
    /// Sister of `Visit`; both share the same work loop in the unified
    /// driver so cross-pipeline transitions (`$rtoi(real_arg)`,
    /// `$itor(int_arg)`, `1.0 + 1`, `real ? then : else`) become
    /// regular work-stack pushes instead of nested function calls.
    VisitReal {
        node: &'b Annotated<'a>,
    },
    /// Real-pipeline combiner. Pops/pushes f64s on `real_vals` and may
    /// also bridge between `int_vals` and `real_vals` (see RealCombiner).
    RealCombine(RealCombiner<'b, 'a>),
}

enum EvalCombiner<'b, 'a: 'b> {
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
    /// Integer power. Pops 1 value (lhs); the exponent is evaluated as a
    /// BigInt via the legacy walker at combine time. `rhs_expr` is the
    /// raw exponent expression; deep exponent chains still crash until P5.
    BinaryPower {
        effective_meta: ExprMeta,
        lhs_meta: ExprMeta,
        rhs_expr: &'a Expr,
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
    /// the legacy `collect_concatenation_bits` emitted; in lenient mode
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

fn evaluate_annotated(
    root: &Annotated,
    root_ctx: Option<ExprMeta>,
    session: &Session,
) -> Result<IntegerValue, String> {
    let (mut int_vals, real_vals) = run_eval_loop(
        EvalTask::Visit {
            node: root,
            ctx: root_ctx,
        },
        session,
    )?;
    debug_assert_eq!(
        int_vals.len(),
        1,
        "evaluate_annotated produced {} integer values",
        int_vals.len()
    );
    debug_assert!(
        real_vals.is_empty(),
        "evaluate_annotated leaked {} real values",
        real_vals.len()
    );
    Ok(int_vals
        .pop()
        .expect("driver invariant: one root produces one integer value"))
}

// Real-result entry point. Sister of `evaluate_annotated`; both share
// `run_eval_loop` so a tree that mixes real and integer subtrees runs
// on a single work stack. `root.is_real()` must be true.
fn evaluate_annotated_as_real(root: &Annotated, session: &Session) -> Result<f64, String> {
    debug_assert!(
        root.is_real(),
        "evaluate_annotated_as_real called on integer-typed root"
    );
    let (int_vals, mut real_vals) = run_eval_loop(EvalTask::VisitReal { node: root }, session)?;
    debug_assert_eq!(
        real_vals.len(),
        1,
        "evaluate_annotated_as_real produced {} real values",
        real_vals.len()
    );
    debug_assert!(
        int_vals.is_empty(),
        "evaluate_annotated_as_real leaked {} integer values",
        int_vals.len()
    );
    Ok(real_vals
        .pop()
        .expect("driver invariant: one real root produces one f64"))
}

// Unified work-loop shared by both entry points. Holds two value stacks
// — `int_vals` for IntegerValue results, `real_vals` for f64 results —
// and dispatches each `EvalTask` to the appropriate pipeline. Cross-
// pipeline transitions (`$rtoi`, `$itor`, `!real`, `real cond ? ... : ...`,
// implicit LRM 3.5.3 coercion of integer subtrees in real chains) become
// task pushes here, so deep alternation doesn't grow the Rust call stack.
fn run_eval_loop<'b, 'a: 'b>(
    initial: EvalTask<'b, 'a>,
    session: &Session,
) -> Result<(Vec<IntegerValue>, Vec<f64>), String> {
    let mut work: Vec<EvalTask<'b, 'a>> = vec![initial];
    let mut int_vals: Vec<IntegerValue> = Vec::new();
    let mut real_vals: Vec<f64> = Vec::new();

    while let Some(task) = work.pop() {
        match task {
            EvalTask::Visit { node, ctx } => {
                visit_eval(node, ctx, &mut work, &mut int_vals, &mut real_vals, session)?;
            }
            EvalTask::Combine(combiner) => {
                combine_eval(combiner, &mut work, &mut int_vals, &mut real_vals, session)?;
            }
            EvalTask::VisitReal { node } => {
                visit_real_eval(node, &mut work, &mut int_vals, &mut real_vals, session)?;
            }
            EvalTask::RealCombine(combiner) => {
                combine_real_eval(combiner, &mut work, &mut int_vals, &mut real_vals)?;
            }
        }
    }
    Ok((int_vals, real_vals))
}

// Push `node` so it deposits a real value on `real_vals`. If the node is
// real-typed, that's a single `VisitReal`; if it's integer-typed, queue
// `Visit { ctx: None }` followed by `RealCombine(CoerceFromInteger)` so
// the IntegerValue produced by the integer pipeline is bridged to f64
// via LRM 3.5.3 implicit coercion. Children of real-typed parents
// (e.g. `1.0 + 1`) go through this helper.
fn push_visit_as_real<'b, 'a: 'b>(node: &'b Annotated<'a>, work: &mut Vec<EvalTask<'b, 'a>>) {
    if node.is_real() {
        work.push(EvalTask::VisitReal { node });
    } else {
        work.push(EvalTask::RealCombine(RealCombiner::CoerceFromInteger));
        work.push(EvalTask::Visit { node, ctx: None });
    }
}

// Real-pipeline counterpart of `visit_eval`. Walks the same `Annotated`
// tree but produces an f64 on `real_vals`. Cross-pipeline transitions
// (`$itor`, `$bitstoreal`, integer subtree under real arith, `?:` cond
// bridging) push tasks onto the shared work stack instead of recursing,
// so deep alternation never grows the Rust call stack.
fn visit_real_eval<'b, 'a: 'b>(
    node: &'b Annotated<'a>,
    work: &mut Vec<EvalTask<'b, 'a>>,
    _vals: &mut Vec<IntegerValue>,
    real_vals: &mut Vec<f64>,
    session: &Session,
) -> Result<(), String> {
    debug_assert!(
        node.is_real(),
        "visit_real_eval invoked on integer-typed node — caller should have used push_visit_as_real"
    );
    match &node.kind {
        AnnotatedKind::Grouped(inner) => {
            // Grouped is transparent. Inner of a real-typed Grouped is
            // itself real-typed.
            push_visit_as_real(inner, work);
        }
        AnnotatedKind::Unary(operand) => {
            let op = match node.expr {
                Expr::Unary { op, .. } => *op,
                _ => unreachable!(),
            };
            match op {
                UnaryOp::Plus => {
                    work.push(EvalTask::RealCombine(RealCombiner::UnaryPlus));
                    push_visit_as_real(operand, work);
                }
                UnaryOp::Minus => {
                    work.push(EvalTask::RealCombine(RealCombiner::UnaryMinus));
                    push_visit_as_real(operand, work);
                }
                _ => {
                    return Err(format!(
                        "operator {} not allowed on real operand",
                        unary_op_name(op)
                    ));
                }
            }
        }
        AnnotatedKind::Binary { lhs, rhs } => {
            let op = match node.expr {
                Expr::Binary { op, .. } => *op,
                _ => unreachable!(),
            };
            match op {
                BinaryOp::Add
                | BinaryOp::Subtract
                | BinaryOp::Multiply
                | BinaryOp::Divide
                | BinaryOp::Power => {
                    work.push(EvalTask::RealCombine(RealCombiner::BinaryArith { op }));
                    push_visit_as_real(rhs, work);
                    push_visit_as_real(lhs, work);
                }
                _ => {
                    return Err(format!(
                        "operator {} not allowed on real operand",
                        binary_op_name(op)
                    ));
                }
            }
        }
        AnnotatedKind::Conditional {
            cond,
            then_arm,
            else_arm,
        } => {
            // Cond may be integer- or real-typed; both branches are
            // real-typed at this point (the conditional itself is real,
            // so integer branches implicitly coerce per LRM 3.5.3 via
            // `push_visit_as_real`). Definite cond short-circuits to
            // one branch (matching legacy walker's behavior); x/z cond
            // evaluates both branches and merges via
            // `ConditionalRealMerge`.
            //
            // The dispatch is done by a combiner that pops the cond
            // value (from `int_vals` or `real_vals` depending on cond
            // type) and pushes the appropriate downstream tasks.
            if cond.is_real() {
                work.push(EvalTask::RealCombine(
                    RealCombiner::DispatchRealCondRealResult { then_arm, else_arm },
                ));
                work.push(EvalTask::VisitReal { node: cond });
            } else {
                work.push(EvalTask::RealCombine(
                    RealCombiner::DispatchIntCondRealResult { then_arm, else_arm },
                ));
                work.push(EvalTask::Visit {
                    node: cond,
                    ctx: None,
                });
            }
        }
        AnnotatedKind::Concatenation(_) | AnnotatedKind::Replication { .. } => {
            unreachable!("concatenation/replication never has real result type");
        }
        AnnotatedKind::SignCast { .. } | AnnotatedKind::BaseCast { .. } => {
            unreachable!("$signed/$unsigned/$bin/$oct/$dec/$hex never has real result type");
        }
        AnnotatedKind::RealConversion { kind, arg } => match kind {
            RealConversionKind::IntegerToReal => {
                // $itor: pop 1 IntegerValue (self-determined arg
                // evaluated via integer pipeline), push 1 f64 via
                // LRM 3.5.3 implicit conversion.
                work.push(EvalTask::RealCombine(RealCombiner::ItorFromInt));
                work.push(EvalTask::Visit {
                    node: arg,
                    ctx: None,
                });
            }
            RealConversionKind::BitsToReal => {
                work.push(EvalTask::RealCombine(RealCombiner::BitstoRealFromInt));
                work.push(EvalTask::Visit {
                    node: arg,
                    ctx: None,
                });
            }
            RealConversionKind::RealToInteger | RealConversionKind::RealToBits => {
                unreachable!("integer-result conversions handled by integer pipeline");
            }
        },
        AnnotatedKind::MathFunction { kind, args } => {
            debug_assert!(
                kind.is_real_result(),
                "integer-result math function `{}` reached real driver",
                kind.name()
            );
            // Push the combiner first (executes last), then args in
            // reverse — so args[0] visits first and lands at the bottom
            // of the popped slice.
            work.push(EvalTask::RealCombine(RealCombiner::MathFunction {
                kind: *kind,
                arity: args.len(),
            }));
            for arg in args.iter().rev() {
                push_visit_as_real(arg, work);
            }
        }
        AnnotatedKind::SystemTask => {
            let name = match node.expr {
                Expr::SystemCall { name, .. } => name.as_str(),
                _ => unreachable!("AnnotatedKind::SystemTask wraps only Expr::SystemCall"),
            };
            return Err(task_in_expression_error(name));
        }
        AnnotatedKind::Leaf => match node.expr {
            Expr::RealLiteral(value) => real_vals.push(*value),
            Expr::Identifier(name) => {
                let v = session
                    .lookup(name)
                    .and_then(|reg| reg.real())
                    .ok_or_else(|| format!("unknown real variable `{name}`"))?;
                real_vals.push(v);
            }
            Expr::Select { name, kind, inner } => {
                debug_assert!(
                    inner.is_none(),
                    "validator drops chained selects on real array"
                );
                let index = match kind {
                    SelectKind::Bit { index } => index,
                    _ => unreachable!("validator rejects part-select on real array"),
                };
                real_vals.push(evaluate_real_array_element_select(name, index, session)?);
            }
            Expr::Literal(_) | Expr::StringLiteral(_) => {
                unreachable!("integer literal Leaf isn't real-typed; would be coerced earlier");
            }
            _ => unreachable!(
                "Leaf annotated kind only wraps Literal / RealLiteral / Identifier / Select"
            ),
        },
    }
    Ok(())
}

// Real-pipeline combiner. Reads/writes `real_vals` (and may also pop
// from `int_vals` for the bridge variants). The bridge combiners
// (`CoerceFromInteger`, `ItorFromInt`, `BitstoRealFromInt`) consume one
// IntegerValue produced by an integer-side `Visit` task and convert to
// f64 — they are how integer subtrees appearing inside real chains
// (LRM 3.5.3 implicit coercion, `$itor`, `$bitstoreal`) deposit a real
// value on `real_vals` without recursing.
fn combine_real_eval<'b, 'a: 'b>(
    combiner: RealCombiner<'b, 'a>,
    work: &mut Vec<EvalTask<'b, 'a>>,
    int_vals: &mut Vec<IntegerValue>,
    real_vals: &mut Vec<f64>,
) -> Result<(), String> {
    match combiner {
        RealCombiner::UnaryPlus => {
            // Identity; leave the popped value on the stack.
        }
        RealCombiner::UnaryMinus => {
            let v = real_vals.pop().expect("UnaryMinus: operand missing");
            real_vals.push(-v);
        }
        RealCombiner::BinaryArith { op } => {
            let rhs = real_vals.pop().expect("BinaryArith: rhs missing");
            let lhs = real_vals.pop().expect("BinaryArith: lhs missing");
            let result = match op {
                BinaryOp::Add => lhs + rhs,
                BinaryOp::Subtract => lhs - rhs,
                BinaryOp::Multiply => lhs * rhs,
                BinaryOp::Divide => lhs / rhs,
                BinaryOp::Power => lhs.powf(rhs),
                _ => unreachable!("BinaryArith Combine got {:?}", op),
            };
            real_vals.push(result);
        }
        RealCombiner::ConditionalRealMerge => {
            let else_val = real_vals.pop().expect("ConditionalRealMerge: else missing");
            let then_val = real_vals.pop().expect("ConditionalRealMerge: then missing");
            let merged = if then_val.to_bits() == else_val.to_bits() {
                then_val
            } else {
                f64::NAN
            };
            real_vals.push(merged);
        }
        RealCombiner::MathFunction { kind, arity } => {
            let start = real_vals.len() - arity;
            let args: Vec<f64> = real_vals.drain(start..).collect();
            let result = apply_real_math_function(kind, &args);
            real_vals.push(result);
        }
        RealCombiner::CoerceFromInteger => {
            let v = int_vals.pop().expect("CoerceFromInteger: int missing");
            real_vals.push(integer_value_to_f64(&v));
        }
        RealCombiner::ItorFromInt => {
            let v = int_vals.pop().expect("ItorFromInt: int missing");
            real_vals.push(integer_value_to_f64(&v));
        }
        RealCombiner::BitstoRealFromInt => {
            let v = int_vals.pop().expect("BitstoRealFromInt: int missing");
            // LRM 17.8: $bitstoreal requires exactly 64-bit operand.
            // Validator catches the bad case earlier; defence in depth.
            if v.width != 64 {
                return Err(format!(
                    "$bitstoreal argument must be 64 bits wide, got {}",
                    v.width
                ));
            }
            real_vals.push(bits_value_to_real(&v));
        }
        RealCombiner::DispatchIntCondRealResult { then_arm, else_arm } => {
            let cond_value = int_vals
                .pop()
                .expect("DispatchIntCondRealResult: cond missing");
            push_real_cond_branches(logical_value(&cond_value), then_arm, else_arm, work);
        }
        RealCombiner::DispatchRealCondRealResult { then_arm, else_arm } => {
            let cond_value = real_vals
                .pop()
                .expect("DispatchRealCondRealResult: cond missing");
            push_real_cond_branches(logical_value_of_real(cond_value), then_arm, else_arm, work);
        }
    }
    Ok(())
}

// Push the downstream tasks for a real-result conditional after the
// cond has been reduced to a `LogicBit`. Definite cond → just visit the
// chosen branch as real; x/z → visit both then merge per LRM §5.1.13's
// `f64::to_bits()`-equality rule.
fn push_real_cond_branches<'b, 'a: 'b>(
    cond_logical: LogicBit,
    then_arm: &'b Annotated<'a>,
    else_arm: &'b Annotated<'a>,
    work: &mut Vec<EvalTask<'b, 'a>>,
) {
    match cond_logical {
        LogicBit::One => push_visit_as_real(then_arm, work),
        LogicBit::Zero => push_visit_as_real(else_arm, work),
        LogicBit::X | LogicBit::Z => {
            work.push(EvalTask::RealCombine(RealCombiner::ConditionalRealMerge));
            push_visit_as_real(else_arm, work);
            push_visit_as_real(then_arm, work);
        }
    }
}

fn visit_eval<'b, 'a: 'b>(
    node: &'b Annotated<'a>,
    ctx: Option<ExprMeta>,
    work: &mut Vec<EvalTask<'b, 'a>>,
    vals: &mut Vec<IntegerValue>,
    real_vals: &mut Vec<f64>,
    session: &Session,
) -> Result<(), String> {
    match &node.kind {
        // Grouped is transparent in semantics — recurse into the inner
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
            // replaces the legacy `infer_expr_meta(items[0])` re-walk.
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
        // Leaves (Literal, Identifier, Select, SystemTask) are evaluated
        // by `evaluate_leaf_expr_in_context`. They have no children deeper
        // than the select index/range, which the leaf evaluator routes
        // through the iterative `evaluate_subexpr_as_integer`. RealLiteral
        // can't reach here — it's real-typed and goes through
        // `visit_real_eval`.
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
                work.push(EvalTask::Combine(EvalCombiner::BinaryRealLogical {
                    op,
                    ctx,
                }));
                push_visit_as_real(rhs, work);
                push_visit_as_real(lhs, work);
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
        signed: meta.signed,
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
            // lhs takes the result width but lhs's own signedness/base.
            // rhs (the exponent) is self-determined per LRM Table 5-3:
            // evaluated at its own width in the BinaryPower combiner via the
            // standard integer pipeline, then applied with modular
            // exponentiation so the result stays bounded by the result width.
            let lhs_inner_ctx = ExprMeta {
                width: effective_meta.width,
                signed: lhs_meta.signed,
                base: lhs_meta.base,
            };
            work.push(EvalTask::Combine(EvalCombiner::BinaryPower {
                effective_meta,
                lhs_meta,
                rhs_expr: rhs.expr,
            }));
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

fn combine_eval<'b, 'a: 'b>(
    combiner: EvalCombiner<'b, 'a>,
    work: &mut Vec<EvalTask<'b, 'a>>,
    vals: &mut Vec<IntegerValue>,
    real_vals: &mut Vec<f64>,
    session: &Session,
) -> Result<(), String> {
    match combiner {
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
                    meta.signed,
                    meta.base,
                ));
                return Ok(());
            }
            let lhs_int = lhs_value.as_bigint(meta.signed);
            let rhs_int = rhs_value.as_bigint(meta.signed);
            let result = match op {
                BinaryOp::Add => lhs_int + rhs_int,
                BinaryOp::Subtract => lhs_int - rhs_int,
                BinaryOp::Multiply => lhs_int * rhs_int,
                BinaryOp::Divide => {
                    if rhs_int.is_zero() {
                        vals.push(IntegerValue::all_x(
                            effective_meta.width,
                            meta.signed,
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
                            meta.signed,
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
                meta.signed,
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
        EvalCombiner::BinaryPower {
            effective_meta,
            lhs_meta,
            rhs_expr,
        } => {
            let lhs_value = vals.pop().expect("BinaryPower: lhs missing");
            if lhs_value.has_unknown_bits() {
                vals.push(IntegerValue::all_x(
                    effective_meta.width,
                    lhs_meta.signed,
                    lhs_meta.base,
                ));
                return Ok(());
            }
            // LRM 5.1.6 / Table 5-3: the exponent is self-determined — it is
            // evaluated at its own width (each nested operator truncating to
            // its own width in turn), NOT at arbitrary precision. So we route
            // it through the standard self-determined integer pipeline rather
            // than a full-precision bigint walker. Unknown exponent bits make
            // the whole result x.
            let exponent_value = evaluate_subexpr_as_integer(rhs_expr, session)?;
            if exponent_value.has_unknown_bits() {
                vals.push(IntegerValue::all_x(
                    effective_meta.width,
                    lhs_meta.signed,
                    lhs_meta.base,
                ));
                return Ok(());
            }
            let exponent_value = exponent_value.as_bigint(exponent_value.signed);
            let base_value = lhs_value.as_bigint(lhs_meta.signed);
            let result = match evaluate_power(base_value, exponent_value, effective_meta.width) {
                Ok(r) => r,
                Err(_) => {
                    vals.push(IntegerValue::all_x(
                        effective_meta.width,
                        lhs_meta.signed,
                        lhs_meta.base,
                    ));
                    return Ok(());
                }
            };
            vals.push(IntegerValue::from_bigint(
                result,
                effective_meta.width,
                lhs_meta.signed,
                lhs_meta.base,
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
                _ => unreachable!("BinaryLogical Combine got non-logical op"),
            };
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
                _ => unreachable!("non-logical op in BinaryRealLogical"),
            };
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

// Pure value-level relational comparison; mirrors `evaluate_relational_expr`'s
// bit-comparison logic but on already-evaluated, already-extended values.
fn compute_relational_from_values(
    op: BinaryOp,
    lhs_value: &IntegerValue,
    rhs_value: &IntegerValue,
    signed: bool,
    context: Option<ExprMeta>,
) -> IntegerValue {
    if lhs_value.has_unknown_bits() || rhs_value.has_unknown_bits() {
        return widen_relational_result(IntegerValue::all_x(1, false, Base::Binary), context);
    }
    let lhs_int = lhs_value.as_bigint(signed);
    let rhs_int = rhs_value.as_bigint(signed);
    let comparison_result = match op {
        BinaryOp::LessThan => lhs_int < rhs_int,
        BinaryOp::GreaterThan => lhs_int > rhs_int,
        BinaryOp::LessThanOrEqual => lhs_int <= rhs_int,
        BinaryOp::GreaterThanOrEqual => lhs_int >= rhs_int,
        _ => unreachable!("non-relational op in compute_relational_from_values"),
    };
    let bit = if comparison_result {
        LogicBit::One
    } else {
        LogicBit::Zero
    };
    widen_relational_result(comparison_result_value(bit), context)
}

// Pure value-level equality comparison; mirrors `evaluate_equality_expr`.
fn compute_equality_from_values(
    op: BinaryOp,
    lhs_value: &IntegerValue,
    rhs_value: &IntegerValue,
    context: Option<ExprMeta>,
) -> IntegerValue {
    let bit = match op {
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
                return widen_relational_result(
                    IntegerValue::all_x(1, false, Base::Binary),
                    context,
                );
            }
            let equal = !definite_mismatch;
            let result = if matches!(op, BinaryOp::Equal) {
                equal
            } else {
                !equal
            };
            if result {
                LogicBit::One
            } else {
                LogicBit::Zero
            }
        }
        BinaryOp::CaseEqual | BinaryOp::CaseNotEqual => {
            let equal = lhs_value.bits == rhs_value.bits;
            let result = if matches!(op, BinaryOp::CaseEqual) {
                equal
            } else {
                !equal
            };
            if result {
                LogicBit::One
            } else {
                LogicBit::Zero
            }
        }
        _ => unreachable!("non-equality op in compute_equality_from_values"),
    };
    widen_relational_result(comparison_result_value(bit), context)
}

// CES-style iterative implementation. Parent shapes (Grouped, Unary
// width-preserving, Binary, Conditional, Concatenation, Replication,
// SignCast, BaseCast) push a Combine task that knows how to fold their
// children's metas; leaves push their meta directly onto `vals`. Each
// node contributes O(1) Rust stack depth.
enum InferMetaTask<'a> {
    Visit(&'a Expr),
    Combine(InferMetaCombiner<'a>),
}

enum InferMetaCombiner<'a> {
    GroupedOrPropagate,
    Binary {
        op: BinaryOp,
    },
    Conditional,
    Concatenation {
        item_count: usize,
    },
    // Replication's count expression is self-determined and gets evaluated
    // at combine time (the iterative `evaluate_constant_expr` path is the
    // recursion break). We carry the original count expression for that
    // call.
    Replication {
        count_expr: &'a Expr,
        item_count: usize,
    },
    SignCast {
        signed: bool,
    },
    BaseCast {
        base: Base,
    },
}

fn infer_expr_meta(expr: &Expr, session: &Session) -> Result<ExprMeta, String> {
    let mut work: Vec<InferMetaTask> = vec![InferMetaTask::Visit(expr)];
    let mut vals: Vec<ExprMeta> = Vec::new();

    while let Some(task) = work.pop() {
        match task {
            InferMetaTask::Visit(node) => match node {
                Expr::Literal(value) => vals.push(ExprMeta {
                    width: value.width,
                    signed: value.signed,
                    base: value.base,
                }),
                Expr::StringLiteral(bytes) => {
                    let spec = string_literal_spec(bytes);
                    vals.push(ExprMeta {
                        width: spec.width,
                        signed: spec.signed,
                        base: spec.base,
                    });
                }
                // Real has no width/sign/base; reaching this branch means
                // an integer-pipeline operator looked at a real-typed
                // sub-expression for context, which the dispatch should
                // have prevented.
                Expr::RealLiteral(_) => {
                    return Err("real value has no integer width or signedness".to_string());
                }
                Expr::Grouped(inner) => {
                    work.push(InferMetaTask::Combine(
                        InferMetaCombiner::GroupedOrPropagate,
                    ));
                    work.push(InferMetaTask::Visit(inner));
                }
                Expr::Unary { op, expr: operand } => match op {
                    UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitwiseNot => {
                        work.push(InferMetaTask::Combine(
                            InferMetaCombiner::GroupedOrPropagate,
                        ));
                        work.push(InferMetaTask::Visit(operand));
                    }
                    UnaryOp::LogicalNot
                    | UnaryOp::ReductionAnd
                    | UnaryOp::ReductionNand
                    | UnaryOp::ReductionOr
                    | UnaryOp::ReductionNor
                    | UnaryOp::ReductionXor
                    | UnaryOp::ReductionXnor => vals.push(ExprMeta {
                        width: 1,
                        signed: false,
                        base: Base::Binary,
                    }),
                },
                Expr::Binary { op, lhs, rhs } => {
                    work.push(InferMetaTask::Combine(InferMetaCombiner::Binary {
                        op: *op,
                    }));
                    work.push(InferMetaTask::Visit(rhs));
                    work.push(InferMetaTask::Visit(lhs));
                }
                // LRM 5.1.13: cond is self-determined and contributes nothing
                // to the result meta; then/else are context-determined and
                // unify width (max) and signedness (any unsigned → unsigned,
                // §5.5.1).
                Expr::Conditional {
                    then_expr,
                    else_expr,
                    ..
                } => {
                    work.push(InferMetaTask::Combine(InferMetaCombiner::Conditional));
                    work.push(InferMetaTask::Visit(else_expr));
                    work.push(InferMetaTask::Visit(then_expr));
                }
                // LRM 5.1.14: width = sum of operand widths, always
                // unsigned. Base follows leftmost-wins.
                Expr::Concatenation { items } => {
                    work.push(InferMetaTask::Combine(InferMetaCombiner::Concatenation {
                        item_count: items.len(),
                    }));
                    for item in items.iter().rev() {
                        work.push(InferMetaTask::Visit(item));
                    }
                }
                // Replication width depends on the constant count value, so
                // we evaluate it eagerly at Combine time. The lenient count
                // helper accepts zero — that's structurally valid and the
                // per-position constraint is enforced separately by
                // `evaluate_replication_expr` / `collect_concatenation_bits`.
                Expr::Replication { count, items } => {
                    work.push(InferMetaTask::Combine(InferMetaCombiner::Replication {
                        count_expr: count,
                        item_count: items.len(),
                    }));
                    for item in items.iter().rev() {
                        work.push(InferMetaTask::Visit(item));
                    }
                }
                // `$name(args)`: dispatch by classified kind. Sign /
                // base casts inherit width / base from the argument.
                // LRM 17.8: $rtoi → 32-bit signed; $realtobits → 64-bit
                // unsigned hex; the real-result variants ($itor / $bitstoreal)
                // shouldn't reach the integer pipeline. LRM 17.11:
                // $clog2 yields 32-bit signed; real-result math functions
                // have no integer meta. Tasks reject up-front.
                Expr::SystemCall { name, args } => {
                    let kind = classify_system_call(name)?;
                    match kind {
                        SystemCallKind::Function(SystemFunction::SignCast { signed }) => {
                            work.push(InferMetaTask::Combine(InferMetaCombiner::SignCast {
                                signed,
                            }));
                            work.push(InferMetaTask::Visit(system_arg_expr(name, args, 0)?));
                        }
                        SystemCallKind::Function(SystemFunction::BaseCast(base)) => {
                            work.push(InferMetaTask::Combine(InferMetaCombiner::BaseCast { base }));
                            work.push(InferMetaTask::Visit(system_arg_expr(name, args, 0)?));
                        }
                        SystemCallKind::Function(SystemFunction::RealConversion(conv_kind)) => {
                            match conv_kind {
                                RealConversionKind::RealToInteger => vals.push(ExprMeta {
                                    width: 32,
                                    signed: true,
                                    base: Base::Decimal,
                                }),
                                RealConversionKind::RealToBits => vals.push(ExprMeta {
                                    width: 64,
                                    signed: false,
                                    base: Base::Hex,
                                }),
                                RealConversionKind::IntegerToReal
                                | RealConversionKind::BitsToReal => {
                                    return Err(
                                        "real value has no integer width or signedness".to_string()
                                    );
                                }
                            }
                        }
                        SystemCallKind::Function(SystemFunction::Math(math_kind)) => {
                            if math_kind.is_real_result() {
                                return Err(
                                    "real value has no integer width or signedness".to_string()
                                );
                            }
                            vals.push(ExprMeta {
                                width: 32,
                                signed: true,
                                base: Base::Decimal,
                            });
                        }
                        SystemCallKind::Task(_) => return Err(task_in_expression_error(name)),
                    }
                }
                // A reg's meta is exactly the IntegerValue's stored
                // (width, signed, base) — same shape `Expr::Literal`
                // produces from its value.
                Expr::Identifier(name) => {
                    let reg = session
                        .lookup(name)
                        .ok_or_else(|| format!("undeclared identifier: {name}"))?;
                    let value = reg.require_vector(name)?;
                    vals.push(ExprMeta {
                        width: value.width,
                        signed: value.signed,
                        base: value.base,
                    });
                }
                // Select width is fixed by its form; index/range
                // sub-expressions stay outside this fold (handled by
                // `infer_select_meta` directly). Selects are always
                // leaf-shaped from this function's perspective.
                Expr::Select { name, kind, inner } => {
                    vals.push(infer_select_meta(name, kind, inner.as_deref(), session)?);
                }
                Expr::Truncated => unreachable!(
                    "Expr::Truncated is a display-only sentinel; never reaches infer_expr_meta"
                ),
            },
            InferMetaTask::Combine(combiner) => match combiner {
                InferMetaCombiner::GroupedOrPropagate => {
                    // Grouped + width-preserving Unary (+, -, ~) propagate the
                    // child meta verbatim. The child is already on top of
                    // `vals`; nothing to do.
                }
                InferMetaCombiner::Binary { op } => {
                    let rhs_meta = vals.pop().expect("Binary infer: rhs missing");
                    let lhs_meta = vals.pop().expect("Binary infer: lhs missing");
                    vals.push(combine_binary_meta(op, lhs_meta, rhs_meta));
                }
                InferMetaCombiner::Conditional => {
                    let else_meta = vals.pop().expect("Conditional infer: else missing");
                    let then_meta = vals.pop().expect("Conditional infer: then missing");
                    vals.push(ExprMeta {
                        width: usize::max(then_meta.width, else_meta.width),
                        signed: then_meta.signed && else_meta.signed,
                        base: then_meta.base,
                    });
                }
                InferMetaCombiner::Concatenation { item_count } => {
                    let start = vals.len() - item_count;
                    let items: Vec<ExprMeta> = vals.drain(start..).collect();
                    let mut total_width = 0usize;
                    let mut leftmost_base = Base::Binary;
                    for (idx, m) in items.iter().enumerate() {
                        total_width = total_width.saturating_add(m.width);
                        if idx == 0 {
                            leftmost_base = m.base;
                        }
                    }
                    vals.push(ExprMeta {
                        width: total_width,
                        signed: false,
                        base: leftmost_base,
                    });
                }
                InferMetaCombiner::Replication {
                    count_expr,
                    item_count,
                } => {
                    let start = vals.len() - item_count;
                    let items: Vec<ExprMeta> = vals.drain(start..).collect();
                    let count = evaluate_replication_count_allow_zero(count_expr, session)?;
                    let mut inner_width = 0usize;
                    let mut leftmost_base = Base::Binary;
                    for (idx, m) in items.iter().enumerate() {
                        inner_width = inner_width.saturating_add(m.width);
                        if idx == 0 {
                            leftmost_base = m.base;
                        }
                    }
                    vals.push(ExprMeta {
                        width: inner_width.saturating_mul(count),
                        signed: false,
                        base: leftmost_base,
                    });
                }
                InferMetaCombiner::SignCast { signed } => {
                    let arg_meta = vals.pop().expect("SignCast infer: arg missing");
                    vals.push(ExprMeta {
                        width: arg_meta.width,
                        signed,
                        base: arg_meta.base,
                    });
                }
                InferMetaCombiner::BaseCast { base } => {
                    let arg_meta = vals.pop().expect("BaseCast infer: arg missing");
                    vals.push(ExprMeta {
                        width: arg_meta.width,
                        signed: arg_meta.signed,
                        base,
                    });
                }
            },
        }
    }

    debug_assert_eq!(
        vals.len(),
        1,
        "infer_expr_meta produced {} values",
        vals.len()
    );
    Ok(vals
        .pop()
        .expect("driver invariant: one root produces one meta"))
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
        let (_, elements) = reg.array().expect("is_array() => array() returns Some");
        debug_assert!(!elements.is_empty());
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

fn comparison_result_value(bit: LogicBit) -> IntegerValue {
    IntegerValue::computed(1, false, Base::Binary, vec![bit])
}

fn concatenated_display_style(items: &[IntegerValue]) -> DisplayStyle {
    let mut saw_string_bytes = false;
    for item in items {
        if item.width == 0 {
            continue;
        }
        if item.display_style != DisplayStyle::String || item.width % 8 != 0 {
            return DisplayStyle::Base;
        }
        saw_string_bytes = true;
    }
    if saw_string_bytes {
        DisplayStyle::String
    } else {
        DisplayStyle::Base
    }
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

// LRM 5.1.14: every concatenation operand "shall be sized" — an operand
// with indefinite width (i.e. one whose self-determined width comes from an
// unsized literal) is rejected. The flag propagates through context-determined
// operators that take width from their operands (arithmetic/bitwise/power,
// shift LHS, conditional branches, unary +/-/~), but stops at any operator
// with a definite 1-bit result (relational/equality/logical/reduction) and at
// concatenation/replication themselves (their result widths are summed/
// multiplied integers, never indefinite). E.g. `{4'd1 + 1, 4'd2}` is rejected
// because the unsized `1` has indefinite width.
// Iterative implementation: same OR-fold shape as `expression_is_real`. A
// node is indefinite-width iff at least one of its width-propagating
// children is. Width-blocking operators (relational/equality/logical/
// reduction, concat, replication, casts, conversions) and definite-width
// leaves (Identifier, Select) contribute nothing, so they never push.
fn is_indefinite_width(expr: &Expr) -> bool {
    let mut work: Vec<&Expr> = vec![expr];
    while let Some(node) = work.pop() {
        match node {
            Expr::Literal(value) => {
                if value.unsized_literal {
                    return true;
                }
            }
            Expr::StringLiteral(_) => {}
            // Real values are always rejected from concatenation by
            // `evaluate_concatenation_expr` with a clearer message; mark
            // them as indefinite-width here so any reachable check still
            // refuses them.
            Expr::RealLiteral(_) => return true,
            Expr::Grouped(inner) => work.push(inner),
            Expr::Unary { op, expr } => match op {
                UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitwiseNot => work.push(expr),
                UnaryOp::LogicalNot
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
                | BinaryOp::Modulus
                | BinaryOp::BitwiseAnd
                | BinaryOp::BitwiseOr
                | BinaryOp::BitwiseXor
                | BinaryOp::BitwiseXnor => {
                    work.push(lhs);
                    work.push(rhs);
                }
                BinaryOp::Power
                | BinaryOp::LogicalShiftLeft
                | BinaryOp::LogicalShiftRight
                | BinaryOp::ArithmeticShiftLeft
                | BinaryOp::ArithmeticShiftRight => work.push(lhs),
                BinaryOp::LessThan
                | BinaryOp::GreaterThan
                | BinaryOp::LessThanOrEqual
                | BinaryOp::GreaterThanOrEqual
                | BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::CaseEqual
                | BinaryOp::CaseNotEqual
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
            // Concat/replication, system calls (casts, conversions, math
            // functions, tasks), identifiers, and selects all have
            // definite widths fixed by their form (LRM 5.1.14, 17.7-17.11).
            // Per-form reasoning lives on the original recursive walker;
            // here we just collapse them all into "no indefinite width".
            Expr::Concatenation { .. }
            | Expr::Replication { .. }
            | Expr::SystemCall { .. }
            | Expr::Identifier(_)
            | Expr::Select { .. } => {}
            Expr::Truncated => unreachable!(
                "Expr::Truncated is a display-only sentinel; never reaches has_indefinite_width"
            ),
        }
    }
    false
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
    // Route through the iterative annotated driver so a deep
    // `{(1+1+...+1){...}}` count chain stays off the C stack.
    let value = evaluate_subexpr_as_integer(count_expr, session)?;
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
// looking for a Replication child to allow zero replication on, and
// by `apply_stmt` in lib.rs to spot a top-level `$finish` / `$stop`
// wrapped in redundant parens.
pub(crate) fn unwrap_grouped(expr: &Expr) -> &Expr {
    let mut cur = expr;
    while let Expr::Grouped(inner) = cur {
        cur = inner;
    }
    cur
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
    // Cap the running total after each item so a `{a, a, a, ...}` concat
    // inside replication rejects before the inner buffer ever gets near
    // the OS allocator. saturating_add keeps usize overflow from masking
    // the cap.
    let mut bits = Vec::new();
    for item in items.iter().rev() {
        let item_bits = evaluate_concatenation_item_bits(item, session)?;
        let total = bits.len().saturating_add(item_bits.len());
        value::ensure_bit_width(total, "concatenation")
            .map_err(|e| format!("Semantic error: {e}"))?;
        bits.extend(item_bits);
    }
    if bits.is_empty() {
        // Every operand collapsed to zero width — the concatenation has no
        // positive-size operand, which is the case LRM 5.1.14 forbids.
        return Err("concatenation must have at least one operand with positive size".to_string());
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
        let total = inner_bits.len().saturating_mul(count);
        value::ensure_bit_width(total, "replication")
            .map_err(|e| format!("Semantic error: {e}"))?;
        let mut bits = Vec::with_capacity(total);
        for _ in 0..count {
            bits.extend(inner_bits.iter().copied());
        }
        return Ok(bits);
    }
    // Route through the iterative annotated driver so a deep concat
    // item (e.g. `{1+1+...+1}` in a validator pre-collect) stays off
    // the C stack.
    let value = evaluate_subexpr_as_integer(item, session)?;
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

fn base_cast_name(base: Base) -> &'static str {
    match base {
        Base::Binary => "$bin",
        Base::Octal => "$oct",
        Base::Decimal => "$dec",
        Base::Hex => "$hex",
    }
}

fn real_to_integer_value(value: f64) -> IntegerValue {
    if value.is_nan() || value.is_infinite() {
        return IntegerValue::all_x(32, true, Base::Decimal);
    }
    let truncated = value.trunc();
    let bigint =
        BigInt::from_f64(truncated).expect("finite f64 truncates to a representable BigInt");
    IntegerValue::from_bigint(bigint, 32, true, Base::Decimal)
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

// Integer `**` per LRM Table 5-3. `width` is the result width (L(base));
// the LRM guarantees the result is only `width` bits wide, so we compute
// `base ** exponent mod 2^width` via modular exponentiation instead of
// materialising the full-precision power and truncating afterwards. This
// keeps every intermediate bounded by `width` bits, matching iverilog /
// Verilator and avoiding the pathological blow-up on huge exponents
// (e.g. `3 ** 32'd200000000`). The special small-magnitude results
// (0 / 1 / -1) are returned directly and truncated to `width` by the
// caller's `IntegerValue::from_bigint`.
fn evaluate_power(base: BigInt, exponent: BigInt, width: usize) -> Result<BigInt, String> {
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

    // Modulus is 2^width. Reduce the (possibly negative) base into
    // [0, 2^width) first — `(a mod m)^e mod m == a^e mod m`, and folding a
    // negative base up by the modulus yields its two's-complement residue,
    // which is exactly what the caller reinterprets under the result's
    // signedness. A zero-width result collapses to the empty modulus 1.
    let modulus = BigUint::one() << width;
    let mut base_res = base % BigInt::from(modulus.clone());
    if base_res.sign() == Sign::Minus {
        base_res += BigInt::from(modulus.clone());
    }
    let base_res = base_res
        .to_biguint()
        .expect("residue folded into [0, 2^width) is non-negative");
    let result = base_res.modpow(&exponent, &modulus);
    Ok(BigInt::from(result))
}

// LRM 5.1.11 reduction: fold the binary operator across all operand bits.
// Identity element matches the operator (AND uses 1; OR and XOR use 0);
// the negated forms NAND/NOR/XNOR invert the fold result. Reusing the
// binary truth tables from Phase 6a keeps x/z propagation identical: e.g.
// AND-reduction still gives 0 when any bit is 0 (even with x/z elsewhere),
// because `bitwise_and_bits(0, x)` returns 0.
fn reduce_bits(op: UnaryOp, bits: &[LogicBit]) -> LogicBit {
    let folded = match op {
        UnaryOp::ReductionAnd | UnaryOp::ReductionNand => {
            bits.iter().copied().fold(LogicBit::One, bitwise_and_bits)
        }
        UnaryOp::ReductionOr | UnaryOp::ReductionNor => {
            bits.iter().copied().fold(LogicBit::Zero, bitwise_or_bits)
        }
        UnaryOp::ReductionXor | UnaryOp::ReductionXnor => {
            bits.iter().copied().fold(LogicBit::Zero, bitwise_xor_bits)
        }
        _ => unreachable!("reduce_bits called with non-reduction op"),
    };
    match op {
        UnaryOp::ReductionNand | UnaryOp::ReductionNor | UnaryOp::ReductionXnor => {
            bitwise_not_bit(folded)
        }
        _ => folded,
    }
}

// LRM 4.2.1 / 5.2.1 / 5.2.2 bit-/part-select dispatch. The reg lookup
// happens once here so each kind helper receives `&RegValue` directly
// rather than re-resolving the name. Every helper produces an unsigned
// self-determined IntegerValue; outer-context widening is applied by
// the `Expr::Select` arm of `evaluate_leaf_expr_in_context`.
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
            Some(inner_kind) => evaluate_array_chained_select(name, reg, kind, inner_kind, session),
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
        unreachable!("validator rejects select on scalar real `{name}` before evaluation");
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
    let range = reg
        .range
        .as_ref()
        .ok_or_else(|| format!("bit-select or part-select on scalar reg `{name}` is illegal"))?;
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
        } => {
            evaluate_part_indexed_select(value, range, base_expr, width, result_base, session, true)
        }
        SelectKind::PartIndexedDown {
            base: base_expr,
            width,
        } => evaluate_part_indexed_select(
            value,
            range,
            base_expr,
            width,
            result_base,
            session,
            false,
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
    Ok(
        match resolve_real_array_element_index(name, index, session)? {
            Some(internal) => elements[internal],
            None => 0.0,
        },
    )
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
    let index_value = evaluate_subexpr_as_integer(index, session)?;
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
    debug_assert!(!elements.is_empty());
    let template = &elements[0];
    let index_value = evaluate_subexpr_as_integer(index, session)?;
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
    debug_assert!(!elements.is_empty());
    let template = &elements[0];
    let element = {
        let index_value = evaluate_subexpr_as_integer(index, session)?;
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
    let index_value = evaluate_subexpr_as_integer(index, session)?;
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
    let base_value = evaluate_subexpr_as_integer(base_expr, session)?;
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
    let width = width
        .to_usize()
        .ok_or_else(|| "indexed part-select width too large".to_string())?;
    value::ensure_bit_width(width, "part-select")?;
    Ok(width)
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
    let width = diff
        .to_usize()
        .ok_or_else(|| "part-select width too large".to_string())?;
    value::ensure_bit_width(width, "part-select")?;
    Ok(width)
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
        SelectKind::PartIndexedUp { base, width } | SelectKind::PartIndexedDown { base, width } => {
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
    // Re-stamp width/sign from the lvalue context. Base normally comes
    // from the lvalue too; the exception is a bare-name reg whose display
    // base is still weak, where a successful integer RHS resolves it.
    let sized = rhs_value.resized_to_context(meta.width, meta.signed);
    let mut staged = session.variables.clone();
    distribute_bits_to_leaves(&leaves, &sized.bits, &mut staged, session)?;
    let mut display_base = meta.base;
    let mut display_base_locked = true;
    if let LValue::Name(name) = lvalue {
        let value = staged
            .get_mut(name)
            .expect("lvalue_meta already resolved bare-name lvalue")
            .vector_mut()
            .expect("lvalue_meta already rejected non-vector bare-name lvalue");
        if !value.base_locked && rhs_value.base_locked {
            value.base = rhs_value.base;
            value.base_locked = true;
        }
        display_base = value.base;
        display_base_locked = value.base_locked;
    }
    let displayed = IntegerValue {
        width: meta.width,
        signed: meta.signed,
        base: display_base,
        base_locked: display_base_locked,
        display_style: DisplayStyle::Base,
        bits: sized.bits.clone(),
        unsized_literal: false,
    };
    Ok((staged, displayed))
}

// LRM 5.6 LHS context derivation. Runs the same constant-endpoint /
// direction / scalar-reg / indexed-width checks the RHS select helpers
// do, so any structural problem on the LHS surfaces before the RHS is
// even looked at. Returning an `ExprMeta` keeps the call shape parallel
// to `infer_expr_meta` so the surrounding context-propagation story
// stays one-paradigm.
//
// Iterative CES driver to handle `{{{...a}}}` deep-concat lvalues
// without overflowing the C stack on a recursive `lvalue_meta(item)`.
// Concat layers push a `BuildConcat(n)` followed by Visit tasks for
// each item; non-Concat shapes (Name / Select) compute their leaf meta
// inline and push the result on `vals`.
fn lvalue_meta(root: &LValue, session: &Session) -> Result<ExprMeta, String> {
    let mut work: Vec<LValueMetaTask> = vec![LValueMetaTask::Visit(root)];
    let mut vals: Vec<ExprMeta> = Vec::new();
    while let Some(task) = work.pop() {
        match task {
            LValueMetaTask::Visit(lvalue) => {
                visit_lvalue_meta(lvalue, &mut work, &mut vals, session)?
            }
            LValueMetaTask::BuildConcat(count) => {
                let start = vals.len() - count;
                let mut total_width = 0usize;
                let mut leftmost_base = Base::Binary;
                for (idx, item_meta) in vals.drain(start..).enumerate() {
                    total_width = total_width.saturating_add(item_meta.width);
                    if idx == 0 {
                        leftmost_base = item_meta.base;
                    }
                }
                if total_width == 0 {
                    return Err(
                        "lvalue must have at least one operand with positive size".to_string()
                    );
                }
                vals.push(ExprMeta {
                    width: total_width,
                    signed: false,
                    base: leftmost_base,
                });
            }
        }
    }
    debug_assert_eq!(vals.len(), 1, "lvalue_meta produced {} values", vals.len());
    Ok(vals
        .pop()
        .expect("driver invariant: one root produces one ExprMeta"))
}

enum LValueMetaTask<'a> {
    Visit(&'a LValue),
    BuildConcat(usize),
}

fn visit_lvalue_meta<'a>(
    lvalue: &'a LValue,
    work: &mut Vec<LValueMetaTask<'a>>,
    vals: &mut Vec<ExprMeta>,
    session: &Session,
) -> Result<(), String> {
    if let LValue::Concat(items) = lvalue {
        if items.is_empty() {
            return Err("lvalue concatenation requires at least one operand".to_string());
        }
        work.push(LValueMetaTask::BuildConcat(items.len()));
        for item in items.iter().rev() {
            work.push(LValueMetaTask::Visit(item));
        }
        return Ok(());
    }
    let meta = leaf_lvalue_meta(lvalue, session)?;
    vals.push(meta);
    Ok(())
}

// Leaf-only LValue meta: Name and Select. Concat is dispatched by
// `visit_lvalue_meta` onto the iterative work stack; Truncated never
// reaches here.
fn leaf_lvalue_meta(lvalue: &LValue, session: &Session) -> Result<ExprMeta, String> {
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
                // shares. Reg arrays keep the fresh-reg binary fallback
                // in the element template, so `(width, signed, base)` is
                // read off `elements[0]` (always present:
                // RegRange::width enforces count >= 1 at decl time).
                let (_, elements) = reg.array().expect("is_array() => array() returns Some");
                debug_assert!(!elements.is_empty());
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
        LValue::Concat(_) => {
            unreachable!("Concat is dispatched iteratively by visit_lvalue_meta")
        }
        LValue::Truncated => {
            unreachable!("LValue::Truncated is a display-only sentinel; never reaches lvalue_meta")
        }
    }
}

// Left-to-right (MSB-side first) leaf enumeration. Used by both the
// write-collision pass and the bit-distribution pass; both walk the
// resulting slice in reverse so the rightmost leaf consumes the LSB end
// of the RHS bit stream.
//
// Iterative to handle `{{{...a}}} = 1` deep-nested concat lvalues
// without overflowing the C stack. A heap worklist of LValue refs
// expands Concat layers into their items (pushed in reverse so the
// leftmost item is popped first, preserving MSB-side-first leaf order).
fn flatten_lvalue_leaves<'a>(root: &'a LValue, out: &mut Vec<&'a LValue>) {
    let mut work: Vec<&'a LValue> = vec![root];
    while let Some(lvalue) = work.pop() {
        match lvalue {
            LValue::Name(_) | LValue::Select { .. } => out.push(lvalue),
            LValue::Concat(items) => {
                for item in items.iter().rev() {
                    work.push(item);
                }
            }
            LValue::Truncated => unreachable!(
                "LValue::Truncated is a display-only sentinel; never reaches flatten_lvalue_leaves"
            ),
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
            let index_value = evaluate_subexpr_as_integer(index, session)?;
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
            let base_value = evaluate_subexpr_as_integer(base_expr, session)?;
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
                let (dim, elements) = reg.array().expect("is_array() => array() returns Some");
                let SelectKind::Bit { index } = kind else {
                    unreachable!("lvalue_meta rejected non-Bit outer select on array");
                };
                let index_value = evaluate_subexpr_as_integer(index, session)?;
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
                    let element_range = reg
                        .range
                        .as_ref()
                        .expect("chained inner select on scalar element rejected by lvalue_meta");
                    select_positions(inner_kind, element_range, session)?
                } else {
                    // Whole-element write — every internal bit is
                    // present (LSB-first).
                    debug_assert!(!elements.is_empty());
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
                let range = reg
                    .range
                    .as_ref()
                    .expect("scalar-reg-with-select rejected by lvalue_meta");
                let positions = select_positions(kind, range, session)?;
                Ok(LeafTarget::Vector {
                    name: name.clone(),
                    positions,
                })
            }
        }
        // Concats aren't leaves — flatten_lvalue_leaves never reaches them.
        LValue::Concat(_) => unreachable!("leaf_target called on a Concat"),
        LValue::Truncated => {
            unreachable!("LValue::Truncated is a display-only sentinel; never reaches leaf_target")
        }
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
