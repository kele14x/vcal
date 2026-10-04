//! Select annotation, bound resolution, reads, and assignment bit mapping.

use std::cell::OnceCell;

use num_bigint::{BigInt, Sign};
use num_traits::{One, Signed, ToPrimitive, Zero};

use super::annotate::expression_is_real;
use super::integer::EvalCombiner;
use super::ir::{Annotated, AnnotatedKind, AnnotatedSelect, ExprMeta, ResolvedSelectKind};
use super::machine::{EvalTask, evaluate_annotated};
use super::validate::validate_annotated;
use super::{evaluate_constant_expr, evaluate_subexpr_as_integer};

use crate::parser::{Expr, SelectKind};
use crate::value::{Base, IntegerValue, LogicBit, Value};
use crate::{RegRange, Session, value};

pub(super) fn select_operand_exprs(kind: &SelectKind) -> Vec<&Expr> {
    match kind {
        SelectKind::Bit { index } => vec![index],
        SelectKind::PartConst { msb, lsb } => vec![msb, lsb],
        SelectKind::PartIndexedUp { base, width } | SelectKind::PartIndexedDown { base, width } => {
            vec![base, width]
        }
    }
}

pub(super) fn annotate_select<'a>(
    expr: &'a Expr,
    operands: Vec<Annotated<'a>>,
    session: &Session,
) -> Result<Annotated<'a>, String> {
    // Nested selects have already validated their own operands during
    // this bottom-up walk; their validator arm does not visit them again.
    for operand in &operands {
        validate_annotated(operand, session)?;
    }
    if let Expr::ConcatSelect { kind, .. } = expr {
        let source_meta = operands[0].meta();
        let range = RegRange {
            msb: BigInt::from(source_meta.width - 1),
            lsb: BigInt::zero(),
        };
        let kind = resolve_annotated_select_kind(kind, 1, &operands, &range, session)?;
        return Ok(Annotated {
            expr,
            meta: Some(ExprMeta {
                width: kind.width(),
                signed: false,
                base: source_meta.base,
            }),
            kind: AnnotatedKind::Select(Box::new(AnnotatedSelect {
                operands,
                source_range: Some(range),
                kind,
                inner: None,
                value: OnceCell::new(),
            })),
        });
    }
    let Expr::Select { name, kind, inner } = expr else {
        unreachable!("select combiner requires a select expression");
    };
    let reg = session.lookup(name).expect("select name checked at Visit");
    let (resolved, resolved_inner, meta) = if reg.is_real_array() || reg.is_array() {
        if !matches!(kind, SelectKind::Bit { .. }) {
            return Err(format!(
                "part-select on array `{name}` is illegal; use `{name}[i]` to select an element"
            ));
        }
        if operands[0].is_real() {
            return Err("array element index cannot be real".to_string());
        }
        let resolved = ResolvedSelectKind::Bit { index: 0 };
        if reg.is_real_array() {
            if inner.is_some() {
                return Err(format!(
                    "bit-select or part-select on real-array element `{name}` is illegal"
                ));
            }
            (resolved, None, None)
        } else {
            let (_, elements) = reg.array().expect("vector array");
            let template = &elements[0];
            if let Some(inner) = inner {
                let range = reg.range.as_ref().ok_or_else(|| {
                    format!("bit-select or part-select on scalar array element `{name}` is illegal")
                })?;
                let inner = resolve_annotated_select_kind(inner, 1, &operands, range, session)?;
                let meta = ExprMeta {
                    width: inner.width(),
                    signed: false,
                    base: template.base,
                };
                (resolved, Some(inner), Some(meta))
            } else {
                let meta = ExprMeta {
                    width: template.width,
                    signed: template.signed,
                    base: template.base,
                };
                (resolved, None, Some(meta))
            }
        }
    } else {
        if reg.is_real() {
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
        let range = reg.range.as_ref().ok_or_else(|| {
            format!("bit-select or part-select on scalar reg `{name}` is illegal")
        })?;
        let kind = resolve_annotated_select_kind(kind, 0, &operands, range, session)?;
        let meta = ExprMeta {
            width: kind.width(),
            signed: false,
            base: value.base,
        };
        (kind, None, Some(meta))
    };
    Ok(Annotated {
        expr,
        meta,
        kind: AnnotatedKind::Select(Box::new(AnnotatedSelect {
            operands,
            source_range: None,
            kind: resolved,
            inner: resolved_inner,
            value: OnceCell::new(),
        })),
    })
}

fn resolve_annotated_select_kind(
    kind: &SelectKind,
    offset: usize,
    operands: &[Annotated<'_>],
    range: &RegRange,
    session: &Session,
) -> Result<ResolvedSelectKind, String> {
    match kind {
        SelectKind::Bit { .. } => {
            if operands[offset].is_real() {
                return Err("bit-select index cannot be real".to_string());
            }
            Ok(ResolvedSelectKind::Bit { index: offset })
        }
        SelectKind::PartConst { .. } => {
            let msb = annotated_range_endpoint(&operands[offset], session, "msb")?;
            let lsb = annotated_range_endpoint(&operands[offset + 1], session, "lsb")?;
            check_part_select_direction(range, &msb, &lsb)?;
            let width = compute_select_width(&msb, &lsb)?;
            Ok(ResolvedSelectKind::PartConst { msb, lsb, width })
        }
        SelectKind::PartIndexedUp { .. } | SelectKind::PartIndexedDown { .. } => {
            if operands[offset].is_real() {
                return Err("indexed part-select base cannot be real".to_string());
            }
            let width_operand = &operands[offset + 1];
            if width_operand.is_real() {
                return Err("indexed part-select width cannot be real".to_string());
            }
            let value = evaluate_annotated(width_operand, None, session)?;
            let width = indexed_select_width_value(&value)?;
            Ok(ResolvedSelectKind::PartIndexed {
                base: offset,
                width,
                is_up: matches!(kind, SelectKind::PartIndexedUp { .. }),
            })
        }
    }
}

fn annotated_range_endpoint(
    operand: &Annotated<'_>,
    session: &Session,
    role: &str,
) -> Result<BigInt, String> {
    if operand.is_real() {
        return Err(format!("part-select {role} cannot be real"));
    }
    let value = evaluate_annotated(operand, None, session)?;
    if value.has_unknown_bits() {
        return Err(format!("part-select {role} contains unknown bits"));
    }
    Ok(value.as_bigint(value.signed))
}

pub(super) fn push_select_eval<'b, 'a: 'b>(
    node: &'b Annotated<'a>,
    select: &'b AnnotatedSelect<'a>,
    ctx: Option<ExprMeta>,
    work: &mut Vec<EvalTask<'b, 'a>>,
) {
    work.push(EvalTask::Combine(EvalCombiner::Select { node, ctx }));
    if let Some(index) = select
        .inner
        .as_ref()
        .and_then(ResolvedSelectKind::dynamic_operand)
    {
        work.push(EvalTask::Visit {
            node: &select.operands[index],
            ctx: None,
        });
    }
    if let Some(index) = select.kind.dynamic_operand() {
        work.push(EvalTask::Visit {
            node: &select.operands[index],
            ctx: None,
        });
    }
    if select.source_range.is_some() {
        work.push(EvalTask::Visit {
            node: &select.operands[0],
            ctx: None,
        });
    }
}

pub(super) fn evaluate_resolved_select(
    expr: &Expr,
    select: &AnnotatedSelect<'_>,
    operands: &[IntegerValue],
    session: &Session,
) -> Result<Value, String> {
    if let Some(range) = &select.source_range {
        return apply_resolved_select(&operands[0], range, &select.kind, operands.get(1))
            .map(Value::Integer);
    }
    let Expr::Select { name, .. } = expr else {
        unreachable!("select expression")
    };
    let reg = session.lookup(name).expect("select name validated");
    if let Some((dim, elements)) = reg.real_array() {
        let index = resolved_array_index(dim, &operands[0]);
        return Ok(Value::Real(index.map_or(0.0, |index| elements[index])));
    }
    let selected = if let Some((dim, elements)) = reg.array() {
        let template = &elements[0];
        let index = resolved_array_index(dim, &operands[0]);
        let element = index.map_or_else(
            || IntegerValue::all_x(template.width, template.signed, template.base),
            |index| elements[index].clone(),
        );
        if let Some(inner) = &select.inner {
            let range = reg.range.as_ref().expect("packed array range validated");
            apply_resolved_select(&element, range, inner, operands.get(1))?
        } else {
            element
        }
    } else {
        let value = reg.require_vector(name)?;
        let range = reg.range.as_ref().expect("vector range validated");
        apply_resolved_select(value, range, &select.kind, operands.first())?
    };
    Ok(Value::Integer(selected))
}

fn resolved_array_index(dim: &RegRange, index: &IntegerValue) -> Option<usize> {
    if index.has_unknown_bits() {
        None
    } else {
        resolve_reg_index(dim, &index.as_bigint(index.signed))
    }
}

fn apply_resolved_select(
    value: &IntegerValue,
    range: &RegRange,
    kind: &ResolvedSelectKind,
    dynamic: Option<&IntegerValue>,
) -> Result<IntegerValue, String> {
    match kind {
        ResolvedSelectKind::Bit { .. } => {
            let index = dynamic.expect("bit index evaluated");
            let bit =
                resolved_array_index(range, index).map_or(LogicBit::X, |index| value.bits[index]);
            Ok(IntegerValue::computed(1, false, value.base, vec![bit]))
        }
        ResolvedSelectKind::PartConst { msb, lsb, width } => {
            materialize_part_select(value, range, msb, lsb, *width, value.base)
        }
        ResolvedSelectKind::PartIndexed { width, is_up, .. } => {
            let base = dynamic.expect("indexed base evaluated");
            if base.has_unknown_bits() {
                return Ok(IntegerValue::all_x(*width, false, value.base));
            }
            let base = base.as_bigint(base.signed);
            let span = BigInt::from(width - 1);
            let (lo, hi) = if *is_up {
                (base.clone(), base + span)
            } else {
                (base.clone() - span, base)
            };
            let (msb, lsb) = if range.msb >= range.lsb {
                (hi, lo)
            } else {
                (lo, hi)
            };
            materialize_part_select(value, range, &msb, &lsb, *width, value.base)
        }
    }
}

// Resolves the unpacked-dim index for a real-array element access. Shared
// by the LHS write path (`lib::apply_real_array_element_assign`). Returns
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
// (`[-1:2]`). Scalar regs are rejected upstream in `annotate_select`
// per LRM 5.2.1, so a `range` is always available here.
pub(super) fn resolve_reg_index(range: &RegRange, src: &BigInt) -> Option<usize> {
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
    indexed_select_width_value(&value)
}

fn indexed_select_width_value(value: &IntegerValue) -> Result<usize, String> {
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
            "part-select direction does not match source range: \
             source is [{}:{}], select is [{}:{}]",
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
pub(super) fn select_meta_width(
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
pub(super) fn select_positions(
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
            // Same direction logic as `apply_resolved_select`:
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
