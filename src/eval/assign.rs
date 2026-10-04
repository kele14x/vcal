//! Lvalue validation, assignment context, and staged writes.

use std::collections::HashMap;

use super::annotate::expression_is_real;
use super::ir::ExprMeta;
use super::select::{resolve_reg_index, select_meta_width, select_positions};
use super::validate::validate_select_kind_structure;
use super::{evaluate_assignment_rhs, evaluate_subexpr_as_integer};

use crate::parser::{Expr, LValue, SelectKind};
use crate::value::{Base, DisplayStyle, IntegerValue, LogicBit};
use crate::{RegValue, Session, value};

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
    // job `validate_annotated` does for RValues), so its errors carry
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
// to the annotated tree's cached metas so the surrounding
// context-propagation story stays one-paradigm.
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
                    total_width = total_width.checked_add(item_meta.width).ok_or_else(|| {
                        format!("lvalue width exceeds limit {}", value::MAX_BIT_WIDTH)
                    })?;
                    if idx == 0 {
                        leftmost_base = item_meta.base;
                    }
                }
                if total_width == 0 {
                    return Err(
                        "lvalue must have at least one operand with positive size".to_string()
                    );
                }
                value::ensure_bit_width(total_width, "lvalue")?;
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
            // Mirror `annotate_select`'s ordering on the RHS
            // path: validate the index / endpoint subtrees before reading any
            // type off them. The checks below only ask `expression_is_real`,
            // which reports a subtree's *result* type and deliberately skips
            // structural rules, so an integer-typed but illegal index like
            // `1.0 % 2` would pass here and hit an evaluator `unreachable!`
            // during `distribute_bits_to_leaves`.
            validate_select_kind_structure(kind, session)?;
            if let Some(inner_kind) = inner {
                validate_select_kind_structure(inner_kind, session)?;
            }
            if reg.is_real_array() {
                // Validate the shape exactly like `annotate_select`'s
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
                // array name. `annotate_select` enforces
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
                    // chosen element. Mirrors `annotate_select`'s
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
                    // mirroring the `annotate_select` rejection on the
                    // RHS path.
                    return Err(format!(
                        "bit-select or part-select on real variable `{name}` is not allowed"
                    ));
                }
                if inner.is_some() {
                    // Same diagnostic as the RHS chained-select-on-vector
                    // rejection (`annotate_select`).
                    return Err(format!(
                        "chained select on `{name}` is illegal: `{name}` is not an array"
                    ));
                }
                let value = reg.require_vector(name)?;
                // LRM 5.2.1 scalar-reg rejection. Mirrors `annotate_select`.
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
