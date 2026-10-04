//! Shared iterative evaluation loop and integer/real work-stack bridges.

use num_bigint::Sign;
use num_traits::ToPrimitive;

use super::integer::{EvalCombiner, combine_eval, visit_eval};
use super::ir::{Annotated, ExprMeta};
use super::real::{RealCombiner, combine_real_eval, visit_real_eval};

use crate::Session;
use crate::value::IntegerValue;

// Iterative annotated-tree evaluator (CES driver: control-environment-store).
//
// The work stack contains `EvalTask::Visit` nodes that need to be evaluated
// and `EvalTask::Combine` continuations that take their popped child values
// and produce the parent's result. The value stack holds `IntegerValue`s
// in the order they were pushed, so each Combine pops them in reverse of
// the push order (rhs first, then lhs).
//
// Integer and real evaluation share this iterative driver. Bridges between
// the two value stacks, plus self-determined power exponents, are represented
// as work-stack tasks as well, so every expression shape stays O(1) in Rust
// stack regardless of nesting depth.
//
// Conditional needs a 2-stage Combine: first the cond_value is popped to
// decide which branch to evaluate (LRM 5.1.13: cond=0 → only else, cond=1
// → only then, cond=x/z → both branches and merge per-bit). A
// `ConditionalChoose` Combine handles the decision and pushes either one
// `Visit` (for definite cond) or a `Visit` for each branch plus a
// `ConditionalMerge` Combine (for x/z cond).
pub(super) enum EvalTask<'b, 'a: 'b> {
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

pub(super) fn evaluate_annotated(
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
pub(super) fn evaluate_annotated_as_real(
    root: &Annotated,
    session: &Session,
) -> Result<f64, String> {
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
pub(super) fn push_visit_as_real<'b, 'a: 'b>(
    node: &'b Annotated<'a>,
    work: &mut Vec<EvalTask<'b, 'a>>,
) {
    if node.is_real() {
        work.push(EvalTask::VisitReal { node });
    } else {
        work.push(EvalTask::RealCombine(RealCombiner::CoerceFromInteger));
        work.push(EvalTask::Visit { node, ctx: None });
    }
}

// Replication count must be a constant, non-negative, non-x, non-z value
// (LRM 5.1.14). `to_usize` doubles as the "fits in addressable space" check;
// vcal uses `usize` for widths, so an oversized count surfaces as a clean
// error rather than overflowing. Zero is allowed here — the
// position-sensitive rule (zero is valid only inside a concatenation whose
// other operands sum to positive width) is enforced separately by the
// validator's strict count check (top-level replication) and the lenient
// concat-item dispatch (`push_concat_item_eval`).
//
// Operates on an already-annotated count subtree. Both callers validate the
// count before getting here (annotate's Replication combine runs
// `validate_annotated` on the count first; the validator schedules this after
// `Visit(count)`), so the `evaluate_annotated` call below runs on validated
// input and can't trip the evaluator's real-operand invariants.
pub(super) fn replication_count_value(
    count: &Annotated<'_>,
    session: &Session,
) -> Result<usize, String> {
    // Route through the iterative annotated driver so a deep
    // `{(1+1+...+1){...}}` count chain stays off the C stack.
    let value = evaluate_annotated(count, None, session)?;
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
