//! Annotated expression representation, cached metadata, and iterative drop.

use std::cell::OnceCell;

use num_bigint::BigInt;

use crate::RegRange;
use crate::parser::{Expr, MathFunctionKind, RealConversionKind};
use crate::value::{Base, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ExprMeta {
    pub(super) width: usize,
    pub(super) signed: bool,
    // Inferred display base — leftmost operand wins for binary ops.
    // Used when constructing arithmetic results; ignored when ExprMeta is
    // passed downward as context (literals keep their own base).
    pub(super) base: Base,
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
// `Identifier`, …) and primitive operator data (`UnaryOp`,
// `BinaryOp`, `MathFunctionKind`, …) can be read directly from `expr` rather
// than duplicated into `kind`. `kind` only mirrors the structural children
// the evaluators need to visit.
#[derive(Debug)]
pub(crate) struct Annotated<'a> {
    pub(super) expr: &'a Expr,
    pub(super) meta: Option<ExprMeta>,
    pub(super) kind: AnnotatedKind<'a>,
}

#[derive(Debug)]
pub(super) enum AnnotatedKind<'a> {
    // `Literal`, `RealLiteral`, `Identifier`. The evaluator
    // reads sub-data from `Annotated::expr` directly.
    Leaf,
    Select(Box<AnnotatedSelect<'a>>),
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

// Select operands are self-determined. Bounds needed for width inference
// are resolved once during annotation; indices/bases are scheduled on the
// evaluator's work stack. Memoize only the natural select result, before
// any outer context is applied, for enclosing select bounds to reuse.
#[derive(Debug)]
pub(super) struct AnnotatedSelect<'a> {
    pub(super) operands: Vec<Annotated<'a>>,
    // Concatenation sources occupy operands[0], rebased to [width - 1:0].
    pub(super) source_range: Option<RegRange>,
    pub(super) kind: ResolvedSelectKind,
    pub(super) inner: Option<ResolvedSelectKind>,
    pub(super) value: OnceCell<Value>,
}

#[derive(Debug)]
pub(super) enum ResolvedSelectKind {
    Bit {
        index: usize,
    },
    PartConst {
        msb: BigInt,
        lsb: BigInt,
        width: usize,
    },
    PartIndexed {
        base: usize,
        width: usize,
        is_up: bool,
    },
}

impl ResolvedSelectKind {
    pub(super) fn dynamic_operand(&self) -> Option<usize> {
        match self {
            Self::Bit { index } => Some(*index),
            Self::PartIndexed { base, .. } => Some(*base),
            Self::PartConst { .. } => None,
        }
    }

    pub(super) fn width(&self) -> usize {
        match self {
            Self::Bit { .. } => 1,
            Self::PartConst { width, .. } | Self::PartIndexed { width, .. } => *width,
        }
    }
}

impl<'a> Annotated<'a> {
    pub(crate) fn is_real(&self) -> bool {
        self.meta.is_none()
    }

    pub(super) fn meta(&self) -> ExprMeta {
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
        AnnotatedKind::Select(select) => out.append(&mut select.operands),
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

pub(super) fn unwrap_grouped_annotated<'a, 'b>(annot: &'b Annotated<'a>) -> &'b Annotated<'a> {
    let mut cur = annot;
    while let AnnotatedKind::Grouped(inner) = &cur.kind {
        cur = inner;
    }
    cur
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
