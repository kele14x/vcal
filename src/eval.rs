//! Expression-evaluation facade: entry points and pipeline orchestration.
//!
//! Implementation details are kept in private modules under `eval/`.

mod annotate;
mod assign;
mod integer;
mod ir;
mod machine;
mod ops;
mod real;
mod select;
mod validate;

pub(crate) use annotate::{annotate, expression_is_real};
pub(crate) use assign::evaluate_lvalue_assignment;
pub(crate) use ir::unwrap_grouped;
pub(crate) use select::resolve_real_array_element_index;
pub(crate) use validate::semantic_check;

use ir::ExprMeta;
use machine::{evaluate_annotated, evaluate_annotated_as_real};
use ops::real_to_integer_bigint;
use validate::validate_annotated;

use crate::Session;
use crate::parser::Expr;
use crate::value::{Base, IntegerValue, Value};

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
// assignment context from the destination's declared width and base, then
// runs the RHS through the standard integer pipeline, so a wider/narrower
// RHS extends or truncates exactly the way a literal does in an arithmetic
// context. The destination's base flows in for the leftmost-base inference
// rule; the caller still re-stamps the reg's stored base on the result.
//
// LRM 5.5.1 propagates only the destination's *width* into the RHS — its
// signedness does not cross the assignment. The RHS keeps the signedness its
// own operands give it, and that is what drives leaf extension, so
// `sa = 4'd15 / 4'd2` on a `reg signed [7:0] sa` is 7 (both operands stay
// unsigned) rather than 0 (-1 / 2), and `ua = 4'shf` on a `reg [7:0] ua` is
// 255 because the signed RHS sign-extends. Both callers re-stamp the
// destination's signedness on the stored value afterwards.
//
// A real RHS is implicitly converted per LRM §3.5.3: round to nearest
// with ties away from zero (distinct from `$rtoi`'s truncate-toward-zero
// rule). A real has no signedness of its own, so that branch sizes and signs
// the converted integer by the destination. NaN / ±∞ have no integer image,
// so the lvalue is filled with x bits at its declared width — matching how
// `$rtoi` surfaces "no defined integer" rather than silently mapping to zero.
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
        signed: annotated.meta().signed,
        base,
    };
    evaluate_annotated(&annotated, Some(context), session)
}

// Self-determined evaluation of an integer-typed constant expression
// (used by the reg-declaration range halves). Mirrors the `None` context
// path the evaluator takes for the top-level expression in a calculator
// line. Errors return unprefixed; callers that are public entry points add
// the "Semantic error: " stage prefix, while the select helpers reached
// through `select_meta_width` rely on the validator's own prefixing.
pub(crate) fn evaluate_constant_expr(
    expr: &Expr,
    session: &Session,
) -> Result<IntegerValue, String> {
    let annotated = annotate(expr, session)?;
    validate_annotated(&annotated, session)?;
    evaluate_annotated(&annotated, None, session)
}

// Self-determined integer evaluation for pre-validated raw-Expr operands
// in the lvalue index/base helpers. Nested selects join the iterative
// annotated pipeline, rather than calling back into a raw-Expr evaluator.
fn evaluate_subexpr_as_integer(expr: &Expr, session: &Session) -> Result<IntegerValue, String> {
    let annotated = annotate(expr, session)?;
    evaluate_annotated(&annotated, None, session)
}
