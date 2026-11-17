//! Static-semantic validation over the annotated expression tree.

use super::annotate::annotate;
use super::ir::{Annotated, AnnotatedKind, unwrap_grouped_annotated};
use super::machine::replication_count_value;
use super::ops::{binary_op_name, unary_op_name};

use crate::parser::{BinaryOp, Expr, RealConversionKind, SelectKind, UnaryOp};
use crate::system_call::task_in_expression_error;
use crate::value::Base;
use crate::{Session, value};

// Static-semantics pre-pass. Every public expression-evaluation entry point
// validates its annotation before touching the evaluator, so structural errors
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

// Structural validation for one SelectKind sub-expression (bit-select
// index, part-select endpoint, indexed base / width). Runs the same
// annotate + validate_annotated pipeline the top-level entry points use —
// the sub-expressions are short and self-determined, but deep chains
// inside `a[1+1+...+1]` still need the iterative drivers. Errors return
// unprefixed; the public entry points add the "Semantic error: " stage
// prefix.
fn validate_subexpr_structure(expr: &Expr, session: &Session) -> Result<(), String> {
    let annotated = annotate(expr, session)?;
    validate_annotated(&annotated, session)
}

pub(super) fn validate_select_kind_structure(
    kind: &SelectKind,
    session: &Session,
) -> Result<(), String> {
    match kind {
        SelectKind::Bit { index } => validate_subexpr_structure(index, session),
        SelectKind::PartConst { msb, lsb } => {
            validate_subexpr_structure(msb, session)?;
            validate_subexpr_structure(lsb, session)
        }
        SelectKind::PartIndexedUp { base, width } | SelectKind::PartIndexedDown { base, width } => {
            validate_subexpr_structure(base, session)?;
            validate_subexpr_structure(width, session)
        }
    }
}

// Static-semantic validator over the annotated tree. Reads `is_real()` and
// `meta()` from precomputed annotations instead of re-walking every Binary
// node's lhs/rhs to ask the same questions, dropping the old O(N²) helper-walk
// pattern to O(N) on long chains. Select operands belong to the annotated
// tree and are validated once by their select combiner, before width
// inference evaluates bounds. Their Select arm here is already checked.
// Iterative implementation. Node-local type checks run eagerly at Visit
// time; replication-count evaluation and list-width checks are scheduled
// after the relevant structural checks. Child `Visit` tasks carry the
// structural traversal. `ConcatItem` carries
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
    /// Constant-evaluates a replication count and applies the
    /// position-sensitive zero rule (strict rejects zero, lenient allows
    /// it). Scheduled *after* `Visit(count)` so the count's structure is
    /// validated before any evaluation runs — the evaluator assumes
    /// validated input and would trip its invariants on a count like
    /// `1.0 % 2`.
    PostEvalReplicationCount {
        count: &'b Annotated<'a>,
        strict: bool,
    },
    /// Final pass for a Concatenation/Replication node, run after every
    /// item has been structurally validated. Enforces LRM 5.1.14's two
    /// list-level constraints:
    ///
    /// 1. each operand must have a definite width
    ///    (`is_indefinite_width` per item),
    /// 2. the joined width must be positive (`sum of meta().width`).
    ///
    /// Width comes from cached metadata; indefinite-width checks walk
    /// the raw expression's width-propagating operands. No evaluation
    /// or bit collection is needed. The variant carries `&[Annotated]`
    /// directly so both the metadata and original expression are available.
    PostCheckConcatWidth {
        items: &'b [Annotated<'a>],
    },
}

pub(super) fn validate_annotated(annot: &Annotated, session: &Session) -> Result<(), String> {
    let mut work: Vec<AnnValidateTask> = vec![AnnValidateTask::Visit(annot)];
    while let Some(task) = work.pop() {
        match task {
            AnnValidateTask::Visit(node) => visit_annotated(node, &mut work, session)?,
            AnnValidateTask::ConcatItem { item, role } => {
                let unwrapped = unwrap_grouped_annotated(item);
                if let AnnotatedKind::Replication { count, items } = &unwrapped.kind {
                    push_replication_validation_annotated(
                        count, items, /* strict = */ false, &mut work,
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
            AnnValidateTask::PostEvalReplicationCount { count, strict } => {
                // Runs after Visit(count) has structurally validated the
                // count subtree, so the evaluator's validated-input
                // invariants hold.
                let count_value = replication_count_value(count, session)?;
                if strict && count_value == 0 {
                    return Err("replication count must be positive in this context".to_string());
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
            // Cap literal widths here because the integer leaf evaluator
            // materializes bits trusting this check has run. Also covers
            // paths that skip validation, e.g. evaluate_subexpr_as_integer
            // annotates pre-validated operands but still hits literal
            // leaves through this driver.
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
            _ => unreachable!("AnnotatedKind::Leaf only wraps leaf-shaped Expr variants"),
        },
        // The select combiner already validated every operand before
        // resolving its bounds. Revisiting descendants here would repeat
        // that work once per enclosing select.
        AnnotatedKind::Select(_) => {}
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
            // longer re-walks subtrees the way the eager bit-collection
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
            push_replication_validation_annotated(count, items, /* strict = */ true, work)?;
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
) -> Result<(), String> {
    // Execution order (work stack is LIFO): Visit(count) validates the
    // count's structure first; PostEvalReplicationCount then constant-
    // evaluates it; items follow via ConcatItem; PostCheckConcatWidth
    // runs the list-level width checks last. The count evaluation must
    // not run ahead of Visit(count) — the evaluator assumes validated
    // input, and an unvalidated count like `1.0 % 2` would trip its
    // invariants instead of surfacing the real-operand diagnostic.
    //
    // `strict = true` rejects count = 0 (top-level replication);
    // `strict = false` allows zero (replication directly inside a
    // concatenation list, LRM 5.1.14).
    if count.is_real() {
        return Err("replication count cannot be real".to_string());
    }
    work.push(AnnValidateTask::PostCheckConcatWidth { items });
    for item in items.iter().rev() {
        work.push(AnnValidateTask::ConcatItem {
            item,
            role: "replication",
        });
    }
    work.push(AnnValidateTask::PostEvalReplicationCount { count, strict });
    work.push(AnnValidateTask::Visit(count));
    Ok(())
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
            // Real values are rejected by ConcatItem with a clearer
            // message before this width check runs; mark
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
            | Expr::Select { .. }
            | Expr::ConcatSelect { .. } => {}
            Expr::Truncated => unreachable!(
                "Expr::Truncated is a display-only sentinel; never reaches has_indefinite_width"
            ),
        }
    }
    false
}

fn base_cast_name(base: Base) -> &'static str {
    match base {
        Base::Binary => "$bin",
        Base::Octal => "$oct",
        Base::Decimal => "$dec",
        Base::Hex => "$hex",
    }
}
