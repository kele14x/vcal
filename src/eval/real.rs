//! Real expression scheduling and result combination.

use super::ir::{Annotated, AnnotatedKind};
use super::machine::{EvalTask, push_visit_as_real};
use super::ops::{
    apply_real_math_function, binary_op_name, bits_value_to_real, integer_value_to_f64,
    logical_value, logical_value_of_real, unary_op_name,
};
use super::select::push_select_eval;

use crate::Session;
use crate::parser::{BinaryOp, Expr, MathFunctionKind, RealConversionKind, UnaryOp};
use crate::system_call::task_in_expression_error;
use crate::value::{IntegerValue, LogicBit, Value};

// Real-pipeline combiners. Folded into the unified work loop in
// `evaluate_annotated` (alongside the integer-pipeline `EvalCombiner`):
// Real combiners read/write the f64 value stack and may also bridge
// to/from the IntegerValue stack at $itor / $bitstoreal / implicit
// LRM 3.5.3 coercion points. Putting both pipelines on one work stack
// is what eliminates Rust-stack growth at deep alternating shapes like
// `$rtoi($itor($rtoi($itor(...))))` — the previous design had each
// crossing call into the other driver's loop, adding one C-stack frame
// per crossing.
pub(super) enum RealCombiner<'b, 'a: 'b> {
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

// Real-pipeline counterpart of `visit_eval`. Walks the same `Annotated`
// tree but produces an f64 on `real_vals`. Cross-pipeline transitions
// (`$itor`, `$bitstoreal`, integer subtree under real arith, `?:` cond
// bridging) push tasks onto the shared work stack instead of recursing,
// so deep alternation never grows the Rust call stack.
pub(super) fn visit_real_eval<'b, 'a: 'b>(
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
        AnnotatedKind::Select(select) => {
            if let Some(Value::Real(value)) = select.value.get() {
                real_vals.push(*value);
            } else {
                push_select_eval(node, select, None, work);
            }
        }
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
            Expr::Literal(_) | Expr::StringLiteral(_) => {
                unreachable!("integer literal Leaf isn't real-typed; would be coerced earlier");
            }
            _ => unreachable!("Leaf annotated kind only wraps Literal / RealLiteral / Identifier"),
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
pub(super) fn combine_real_eval<'b, 'a: 'b>(
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
