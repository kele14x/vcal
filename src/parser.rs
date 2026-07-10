use num_bigint::BigUint;
use std::borrow::Cow;

use crate::lexer::{Token, tokenize};
use crate::value::{
    Base, DisplayStyle, IntegerValue, LogicBit, biguint_bit_len, biguint_to_bits_with_width,
    signed_decimal_bit_len,
};

// AST literal payload: parse-time representation that never allocates a
// `Vec<LogicBit>` of length `width`. `width` itself can be huge (e.g.
// `9999999999999'd1`) — the validator gates it against MAX_BIT_WIDTH before
// the evaluator calls `materialize()`. Separating spec-from-value lets the
// parser stay O(text length) and surfaces the cap as a Semantic error rather
// than a Syntax error.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LiteralSpec {
    pub(crate) width: usize,
    pub(crate) signed: bool,
    pub(crate) base: Base,
    pub(crate) unsized_literal: bool,
    pub(crate) payload: LiteralPayload,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LiteralPayload {
    // Numeric magnitude only — used by parse_unsized_decimal and the
    // non-x/non-z arm of parse_based_decimal. Size O(digits). Materializes
    // via biguint_to_bits_with_width.
    Numeric {
        magnitude: BigUint,
    },
    // Explicit bit pattern from radix digits (binary/octal/hex), plus a fill
    // bit for sign/zero/x/z extension to `width`. Covers parse_based_radix
    // and the all-x / all-z short-circuits in parse_based_decimal (empty
    // low_bits + fill = X|Z). Size O(digits * group_size) — text-bounded.
    Bits {
        low_bits: Vec<LogicBit>,
        fill: LogicBit,
    },
}

impl LiteralSpec {
    // Materializes the full IntegerValue. The only path that allocates
    // `width` bytes — gated by the validator's MAX_BIT_WIDTH check.
    pub(crate) fn materialize(&self) -> IntegerValue {
        match &self.payload {
            LiteralPayload::Numeric { magnitude } => IntegerValue {
                width: self.width,
                signed: self.signed,
                base: self.base,
                base_locked: true,
                display_style: DisplayStyle::Base,
                bits: biguint_to_bits_with_width(magnitude, self.width),
                unsized_literal: self.unsized_literal,
            },
            LiteralPayload::Bits { low_bits, fill } => {
                let mut bits = low_bits.clone();
                if bits.len() < self.width {
                    bits.resize(self.width, *fill);
                } else if bits.len() > self.width {
                    bits.truncate(self.width);
                }
                IntegerValue {
                    width: self.width,
                    signed: self.signed,
                    base: self.base,
                    base_locked: true,
                    display_style: DisplayStyle::Base,
                    bits,
                    unsized_literal: self.unsized_literal,
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Expr {
    Literal(LiteralSpec),
    // LRM 3.6: string literals are packed arrays of 8-bit ASCII codes. Keep
    // the AST leaf distinct from integer literals, then materialize through
    // the same IntegerValue path during evaluation.
    StringLiteral(Vec<u8>),
    // LRM §3.5.2: a real constant is stored as a 64-bit IEEE 754 double.
    // Width / signedness / base / x-z don't apply (Table 5-9 lists real as
    // "Signed, floating point"), so we keep the f64 directly rather than
    // shoe-horning it into IntegerValue.
    RealLiteral(f64),
    Grouped(Box<Expr>),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Conditional {
        cond: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
    },
    // LRM 5.1.14: `{a, b, ...}`. Items are stored in source order — leftmost
    // first — but during evaluation the leftmost item ends up in the most
    // significant bit positions of the result. Result is unsigned (LRM 5.5.1
    // last paragraph) and self-determined; outer context only zero-extends
    // the joined result, never propagates into the items.
    Concatenation {
        items: Vec<Expr>,
    },
    // LRM 5.1.14: `{count{items...}}`. `count` is a constant non-negative
    // non-x/non-z expression (rejected at evaluation time otherwise). `items`
    // is the inner concatenation list — same self-determined semantics as
    // `Concatenation`.
    Replication {
        count: Box<Expr>,
        items: Vec<Expr>,
    },
    // Every `$name` / `$name()` / `$name(args)` parses to this one shape —
    // the parser is purely syntactic for system identifiers. The
    // validator (`classify_system_call` in system_call.rs) owns the name table
    // and decides whether `name` is a math function (with arity), a real
    // conversion, a sign cast, a base cast, a system task, or unknown.
    // Math system functions today (LRM 17.11) carry the typed kind on
    // `AnnotatedKind` after the annotate pass resolves the name.
    //
    // For system tasks (LRM 17.4: `$finish`, `$stop`) the args are parsed
    // for syntactic validity and stored on the AST, but the evaluator
    // never reads them — vcal does not print exit diagnostics, so the
    // verbosity argument has no observable effect.
    SystemCall {
        name: String,
        args: Vec<SystemArg>,
    },
    // LRM A.8.3: a simple identifier as a `primary` — a reference to a
    // previously-declared `reg` (the only variable type vcal currently
    // supports). The evaluator looks it up in the active `Session`; an
    // unknown name surfaces as "undeclared identifier: <name>".
    Identifier(String),
    // LRM 4.2.1 / 5.2.1 / 5.2.2: bit-select and part-select on a declared
    // identifier. Storing `name: String` rather than a nested
    // `Expr::Identifier` is deliberate — it makes the grammar reject
    // `4'b1111[0]` at parse time, because `parse_primary` only enters the
    // bracket-pickup branch from the `Token::Identifier` arm.
    //
    // `inner` carries an optional second select that applies to the result
    // of the first, supporting LRM 4.9 chained array-element selects like
    // `a[i][m:l]` (where `a` is a 1-D unpacked array). The parser doesn't
    // know whether `name` is an array, so it accepts the chained shape
    // syntactically and lets the evaluator decide:
    //   - array reg + `inner.is_some()` → outer must be `Bit` (element
    //     pick), inner is any select kind applied to the chosen element
    //     using the element's packed range.
    //   - vector reg + `inner.is_some()` → rejected (a vector select
    //     already yields a 1-bit / part-select value with no sub-structure
    //     to address).
    // Only one chained level is allowed, mirroring vcal's 1-D-array scope;
    // the parser surfaces a clean error on a third bracket.
    Select {
        name: String,
        kind: SelectKind,
        inner: Option<Box<SelectKind>>,
    },
    // Display-only sentinel inserted by `truncate_expr_for_display` to
    // replace sub-trees that exceed the caller-requested render depth.
    // Never produced by the parser, never consumed by eval (which
    // `unreachable!`s on it). Existing as a distinct variant — rather
    // than reusing `Identifier("…")` — keeps the rendered output
    // unambiguous: a `Truncated` node in `{:#?}` output is obviously a
    // placeholder, while an `Identifier("…")` could plausibly be a
    // valid (if oddly-named) symbol.
    Truncated,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SystemArg {
    Expr(Expr),
    Null,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SelectKind {
    // `r[expr]`. `index` is a self-determined integer expression
    // (LRM 4.2.1). Result is 1-bit unsigned.
    Bit { index: Box<Expr> },
    // `r[m:l]`. Both endpoints are constant expressions (LRM 5.2.1).
    // Direction must match the declared reg direction.
    PartConst { msb: Box<Expr>, lsb: Box<Expr> },
    // `r[base +: width]`. `base` is a self-determined integer expression;
    // `width` is a positive constant (LRM 5.2.2). Result spans the source
    // range `[base, base + width - 1]`.
    PartIndexedUp { base: Box<Expr>, width: Box<Expr> },
    // `r[base -: width]`. `base` is a self-determined integer expression;
    // `width` is a positive constant (LRM 5.2.2). Result spans the source
    // range `[base - width + 1, base]`.
    PartIndexedDown { base: Box<Expr>, width: Box<Expr> },
}

// Truncate an `Expr` so that any sub-tree at depth `>= max_depth` is
// replaced with `Expr::Truncated`. Used by `parse_input` (the
// `--parse-only` debug entry point) before `{:#?}`-rendering: the auto-
// derived `Debug` impl recurses on each Box, so without truncation a
// 10^4-deep `Grouped` chain would overflow the stack while printing
// (separate from the parser-side overflow that the iterative
// state-machine parser fixed). Recursion here is bounded by `max_depth`
// (typically 64), so it can't itself overflow.
//
// Depth counting: the top-level `Expr` is depth 0; its direct children
// are depth 1; etc. With `max_depth = N`, exactly N levels are kept
// (depths 0 .. N-1), and everything at depth N or below is replaced
// with `Truncated`. So `--parse-only --max-depth=2` on `(((1)))`
// renders as `Grouped(Grouped(Truncated))` — two `Grouped` layers plus
// the marker.
pub(crate) fn truncate_expr_for_display(expr: &mut Expr, max_depth: usize) {
    truncate_expr_inner(expr, 0, max_depth);
}

fn truncate_expr_inner(expr: &mut Expr, depth: usize, max_depth: usize) {
    if depth >= max_depth {
        *expr = Expr::Truncated;
        return;
    }
    match expr {
        Expr::Literal(_)
        | Expr::StringLiteral(_)
        | Expr::RealLiteral(_)
        | Expr::Identifier(_)
        | Expr::Truncated => {}
        Expr::Grouped(inner) => truncate_expr_inner(inner.as_mut(), depth + 1, max_depth),
        Expr::Unary { expr: inner, .. } => {
            truncate_expr_inner(inner.as_mut(), depth + 1, max_depth)
        }
        Expr::Binary { lhs, rhs, .. } => {
            truncate_expr_inner(lhs.as_mut(), depth + 1, max_depth);
            truncate_expr_inner(rhs.as_mut(), depth + 1, max_depth);
        }
        Expr::Conditional {
            cond,
            then_expr,
            else_expr,
        } => {
            truncate_expr_inner(cond.as_mut(), depth + 1, max_depth);
            truncate_expr_inner(then_expr.as_mut(), depth + 1, max_depth);
            truncate_expr_inner(else_expr.as_mut(), depth + 1, max_depth);
        }
        Expr::Concatenation { items } => {
            for item in items {
                truncate_expr_inner(item, depth + 1, max_depth);
            }
        }
        Expr::Replication { count, items } => {
            truncate_expr_inner(count.as_mut(), depth + 1, max_depth);
            for item in items {
                truncate_expr_inner(item, depth + 1, max_depth);
            }
        }
        Expr::SystemCall { args, .. } => {
            for arg in args {
                if let SystemArg::Expr(arg) = arg {
                    truncate_expr_inner(arg, depth + 1, max_depth);
                }
            }
        }
        Expr::Select { kind, inner, .. } => {
            truncate_select_kind_inner(kind, depth + 1, max_depth);
            if let Some(inner_kind) = inner.as_mut() {
                truncate_select_kind_inner(inner_kind, depth + 1, max_depth);
            }
        }
    }
}

fn truncate_select_kind_inner(kind: &mut SelectKind, depth: usize, max_depth: usize) {
    match kind {
        SelectKind::Bit { index } => truncate_expr_inner(index.as_mut(), depth, max_depth),
        SelectKind::PartConst { msb, lsb } => {
            truncate_expr_inner(msb.as_mut(), depth, max_depth);
            truncate_expr_inner(lsb.as_mut(), depth, max_depth);
        }
        SelectKind::PartIndexedUp { base, width } | SelectKind::PartIndexedDown { base, width } => {
            truncate_expr_inner(base.as_mut(), depth, max_depth);
            truncate_expr_inner(width.as_mut(), depth, max_depth);
        }
    }
}

// Truncate an `LValue` analogously to `truncate_expr_for_display`: any
// sub-tree at depth `>= max_depth` is replaced with `LValue::Truncated`.
// Necessary because the LHS of `Stmt::Assign` is its own recursive
// shape — `LValue::Concat(Vec<LValue>)` can nest arbitrarily — and the
// `{:#?}` formatter recurses on each level. A bare deep concat LHS like
// `{{{{...a}}}} = 1` would overflow the render stack without this cap.
// Recursion here is bounded by `max_depth`, so the walk itself can't
// overflow.
pub(crate) fn truncate_lvalue_for_display(lvalue: &mut LValue, max_depth: usize) {
    truncate_lvalue_inner(lvalue, 0, max_depth);
}

fn truncate_lvalue_inner(lvalue: &mut LValue, depth: usize, max_depth: usize) {
    if depth >= max_depth {
        *lvalue = LValue::Truncated;
        return;
    }
    match lvalue {
        LValue::Name(_) | LValue::Truncated => {}
        LValue::Select { kind, inner, .. } => {
            // Mirrors `Expr::Select`'s depth accounting: the SelectKind
            // itself is at depth + 1, and `truncate_select_kind_inner`
            // gives the Expr children that same depth.
            truncate_select_kind_inner(kind, depth + 1, max_depth);
            if let Some(inner_kind) = inner.as_mut() {
                truncate_select_kind_inner(inner_kind, depth + 1, max_depth);
            }
        }
        LValue::Concat(items) => {
            for item in items {
                truncate_lvalue_inner(item, depth + 1, max_depth);
            }
        }
    }
}

// Apply `truncate_expr_for_display` / `truncate_lvalue_for_display` to
// every `Expr` / `LValue` reachable from a `Stmt` — used by `parse_input`
// to bound `{:#?}` rendering depth on every expression position (reg
// ranges, init expressions, assignment LHS/RHS, etc.) without each
// callsite repeating the descent.
pub(crate) fn truncate_stmt_for_display(stmt: &mut Stmt, max_depth: usize) {
    match stmt {
        Stmt::Expr(e) => truncate_expr_for_display(e, max_depth),
        Stmt::Decl { range, names, .. } => {
            if let Some((msb, lsb)) = range {
                truncate_expr_for_display(msb, max_depth);
                truncate_expr_for_display(lsb, max_depth);
            }
            for name in names {
                if let Some(init) = name.init.as_mut() {
                    truncate_expr_for_display(init, max_depth);
                }
                if let Some((msb, lsb)) = name.dim.as_mut() {
                    truncate_expr_for_display(msb, max_depth);
                    truncate_expr_for_display(lsb, max_depth);
                }
            }
        }
        Stmt::Assign { lvalue, rhs } => {
            truncate_lvalue_for_display(lvalue, max_depth);
            truncate_expr_for_display(rhs, max_depth);
        }
    }
}

// Iterative Drop for Expr.
//
// `Expr::Grouped(Box<Expr>)`, `Binary { lhs: Box<Expr>, rhs: Box<Expr> }`,
// and friends form a recursive type. The auto-derived destructor walks
// these Boxes recursively — for a 10^5-deep `Grouped` chain that's 10^5
// stack frames during drop, which overflows. Same crash mode as a
// recursive parser, just at end-of-scope instead of parse time.
//
// This impl flattens the descent into a heap-allocated worklist:
//   1. Replace each child Expr with a cheap leaf placeholder, stashing
//      the original in the worklist.
//   2. Pop a victim from the worklist, repeat.
// The auto-drop that fires after this method returns then sees only
// leaf-shaped children, so its recursion is O(1) deep regardless of
// input depth. `SelectKind` carries `Box<Expr>` sub-expressions
// (index/range/base/width), so it is flattened the same way.
//
// The placeholder is `Expr::Identifier(String::new())` — a leaf with no
// children, no allocation (empty String is inline), and a Drop that
// re-enters this impl with an empty worklist.
impl Drop for Expr {
    fn drop(&mut self) {
        let mut work: Vec<Expr> = Vec::new();
        steal_expr_children(self, &mut work);
        while let Some(mut victim) = work.pop() {
            steal_expr_children(&mut victim, &mut work);
            // victim's children are now leaves; auto-drop at end of this
            // iteration is shallow.
        }
    }
}

fn steal_expr_children(expr: &mut Expr, out: &mut Vec<Expr>) {
    let placeholder = || Expr::Identifier(String::new());
    match expr {
        Expr::Literal(_)
        | Expr::StringLiteral(_)
        | Expr::RealLiteral(_)
        | Expr::Identifier(_)
        | Expr::Truncated => {}
        Expr::Grouped(inner) => {
            out.push(std::mem::replace(inner.as_mut(), placeholder()));
        }
        Expr::Unary { expr: inner, .. } => {
            out.push(std::mem::replace(inner.as_mut(), placeholder()));
        }
        Expr::Binary { lhs, rhs, .. } => {
            out.push(std::mem::replace(lhs.as_mut(), placeholder()));
            out.push(std::mem::replace(rhs.as_mut(), placeholder()));
        }
        Expr::Conditional {
            cond,
            then_expr,
            else_expr,
        } => {
            out.push(std::mem::replace(cond.as_mut(), placeholder()));
            out.push(std::mem::replace(then_expr.as_mut(), placeholder()));
            out.push(std::mem::replace(else_expr.as_mut(), placeholder()));
        }
        Expr::Concatenation { items } => {
            out.append(items);
        }
        Expr::Replication { count, items } => {
            out.push(std::mem::replace(count.as_mut(), placeholder()));
            out.append(items);
        }
        Expr::SystemCall { args, .. } => {
            for arg in args {
                if let SystemArg::Expr(arg) = arg {
                    out.push(std::mem::replace(arg, placeholder()));
                }
            }
        }
        Expr::Select { kind, inner, .. } => {
            steal_select_kind_children(kind, out);
            if let Some(boxed_inner) = inner.take() {
                let mut inner_kind = *boxed_inner;
                steal_select_kind_children(&mut inner_kind, out);
                // inner_kind drops here: its children are now leaves, so
                // the auto-drop is shallow.
            }
        }
    }
}

fn steal_select_kind_children(kind: &mut SelectKind, out: &mut Vec<Expr>) {
    let placeholder = || Expr::Identifier(String::new());
    match kind {
        SelectKind::Bit { index } => {
            out.push(std::mem::replace(index.as_mut(), placeholder()));
        }
        SelectKind::PartConst { msb, lsb } => {
            out.push(std::mem::replace(msb.as_mut(), placeholder()));
            out.push(std::mem::replace(lsb.as_mut(), placeholder()));
        }
        SelectKind::PartIndexedUp { base, width } | SelectKind::PartIndexedDown { base, width } => {
            out.push(std::mem::replace(base.as_mut(), placeholder()));
            out.push(std::mem::replace(width.as_mut(), placeholder()));
        }
    }
}

// Iterative Drop for LValue, mirroring `impl Drop for Expr` above. The
// `LValue::Concat(Vec<LValue>)` shape is the deep one: `{{{...a}}} = 1`
// nests a Concat per layer, and the auto-derived destructor would walk
// the chain recursively. Flatten with a heap worklist and replace stolen
// children with cheap `LValue::Name(String::new())` leaves so the auto-
// drop after this method returns is shallow.
//
// `SelectKind` sub-expressions hang off `LValue::Select` via Box<Expr>
// (Bit index / range bounds / indexed-base+width). The Drop on those
// Boxes routes through `impl Drop for Expr` above, which is already
// iterative — so we don't need to flatten SelectKind contents here.
impl Drop for LValue {
    fn drop(&mut self) {
        let mut work: Vec<LValue> = Vec::new();
        steal_lvalue_children(self, &mut work);
        while let Some(mut victim) = work.pop() {
            steal_lvalue_children(&mut victim, &mut work);
            // victim's children are now leaves; auto-drop at end of this
            // iteration is shallow.
        }
    }
}

fn steal_lvalue_children(lvalue: &mut LValue, out: &mut Vec<LValue>) {
    match lvalue {
        LValue::Name(_) | LValue::Select { .. } | LValue::Truncated => {}
        LValue::Concat(items) => {
            out.append(items);
        }
    }
}

// Top-level inputs. A REPL line / piped script segment between semicolons is
// one `Stmt`. Expressions still drive the evaluator, but declarations and
// blocking assignments mutate the session rather than producing a value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Stmt {
    Expr(Expr),
    // LRM A.2.1.3 variable declarations. `kind` distinguishes which
    // keyword introduced the decl — `reg` allows `[signed] [range]` and
    // an optional per-name unpacked dimension; `integer` and `real` per
    // LRM 4.8 are fixed-shape (integer is signed 32-bit, real is IEEE
    // 754 binary64) so the parser rejects `signed` and packed `[range]`
    // on them, but each still accepts an optional per-name unpacked
    // dimension (LRM A.2.2.1 `variable_type ::= … { dimension }`). Each
    // item in the identifier list may also carry an optional
    // `= constant_expression` init; an integer init runs through the
    // same blocking-assignment context the reg form does (real → integer
    // per §3.5.3, width/sign/base propagation), while a real init is
    // evaluated as a real value. `range` is the packed range
    // (constant-evaluated at apply time, reg-only). Multi-dim arrays
    // remain out of scope: only one trailing `[ … ]` after the name is
    // accepted.
    Decl {
        kind: DeclKind,
        signed: bool,
        range: Option<(Expr, Expr)>,
        names: Vec<DeclName>,
    },
    // LRM A.6.2 `blocking_assignment` over the full `variable_lvalue`
    // production (LRM A.8.5): a hierarchical name with optional bit /
    // part / indexed-part select, or an arbitrarily nested
    // concatenation of those. The dedicated `LValue` enum makes
    // operators / literals / replications unrepresentable on the LHS
    // by construction.
    Assign {
        lvalue: LValue,
        rhs: Expr,
    },
}

// Which variable-decl keyword introduced this `Stmt::Decl`. LRM 4.8 lists
// the three keywords vcal supports — `time` is named there too but is out
// of scope. Carrying the keyword through to the apply pass means
// `apply_stmt` can shape each kind's storage / default-init / display base
// (32-bit signed all-x decimal for `integer`, 0.0 for `real`, bits-driven
// vector for `reg`) without re-deciding it from `signed`/`range` shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeclKind {
    Reg,
    Integer,
    Real,
}

impl DeclKind {
    pub(crate) fn keyword(self) -> &'static str {
        match self {
            DeclKind::Reg => "reg",
            DeclKind::Integer => "integer",
            DeclKind::Real => "real",
        }
    }
}

// One entry in a decl's `list_of_variable_identifiers`. Exactly one of
// `init` or `dim` may be present (the LRM `variable_type` grammar is a
// strict `name [= expr]` | `name { dimension }` split); the parser
// rejects an attempted combination up-front. `dim` is only ever populated
// for `DeclKind::Reg`; the integer/real keyword paths reject a trailing
// `[…]` at parse time.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DeclName {
    pub(crate) name: String,
    pub(crate) init: Option<Expr>,
    pub(crate) dim: Option<(Expr, Expr)>,
}

// LRM A.8.5 `variable_lvalue`. Storing this as its own enum (rather than
// reusing `Expr`) keeps the LHS grammar a strict subset and lets
// evaluators match exhaustively without re-checking shape at every
// callsite. `SelectKind` is the same one the RHS-side `Expr::Select`
// uses, so all four select forms (bit-select, [m:l] part-select, and the
// `+:` / `-:` indexed forms) carry across to the LHS unchanged.
// `Concat` items are in source order: leftmost first, which is also the
// MSB side of the assembled bit stream — matching `Expr::Concatenation`.
//
// `inner` on `Select` carries the optional chained-select shape
// (LRM 4.9: `a[i][m:l]` selects a sub-range inside an unpacked-array
// element). It mirrors the `inner` field on `Expr::Select`; on the LHS
// the evaluator routes the array-element case (`reg.is_array()`) through
// the same per-position distribution path the vector-reg LHS uses, with
// the inner select choosing which bits inside the chosen element receive
// RHS bits. The vector-reg LHS still rejects `inner.is_some()` because a
// vector select has no further sub-structure to address.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LValue {
    Name(String),
    Select {
        name: String,
        kind: SelectKind,
        inner: Option<Box<SelectKind>>,
    },
    Concat(Vec<LValue>),
    // Display-only sentinel inserted by `truncate_lvalue_for_display` to
    // replace sub-trees that exceed the caller-requested render depth.
    // Mirrors `Expr::Truncated`: never produced by the parser, never
    // consumed by eval (which `unreachable!`s on it). Exists so a deeply
    // nested concat lvalue (`{{{{a}}}}`) can be capped before `{:#?}`
    // formatter recursion overflows the stack.
    Truncated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RealConversionKind {
    // `$rtoi(real)` — truncates toward zero, returns 32-bit signed integer.
    RealToInteger,
    // `$itor(int)` — converts integer to real per §3.5.3 (x/z → 0). Real
    // arguments are rejected by the validator (simulators diverge on this
    // case and the LRM types the argument as `int_val`).
    IntegerToReal,
    // `$realtobits(real)` — bitcast to 64-bit unsigned vector (IEEE 754).
    RealToBits,
    // `$bitstoreal(int)` — reverse bitcast; takes a 64-bit value and
    // reinterprets the bit pattern as an IEEE 754 double.
    BitsToReal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MathFunctionKind {
    // Integer-result. LRM 17.11.1: argument is integer or vector; real is
    // rejected by the validator.
    Clog2,
    // Real-result, 1 arg. Argument is real-typed; an integer argument
    // implicitly promotes via §3.5.3 (x/z → 0).
    Ln,
    Log10,
    Exp,
    Sqrt,
    Floor,
    Ceil,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Asinh,
    Acosh,
    Atanh,
    // Real-result, 2 args.
    Pow,
    Atan2,
    Hypot,
}

// Single source of truth for the math system function name ↔ kind mapping.
// Both the parser (name → kind, in parse_system_function_call) and
// `MathFunctionKind::name()` (kind → name, used in error messages) drive
// off this slice, so a new function only needs adding here. Lookups are
// O(n) linear scans — fine for n = 22 and called at most once per parsed
// function call, and `name()` is only used to format error messages.
const MATH_FUNCTIONS: &[(&str, MathFunctionKind)] = &[
    ("$clog2", MathFunctionKind::Clog2),
    ("$ln", MathFunctionKind::Ln),
    ("$log10", MathFunctionKind::Log10),
    ("$exp", MathFunctionKind::Exp),
    ("$sqrt", MathFunctionKind::Sqrt),
    ("$floor", MathFunctionKind::Floor),
    ("$ceil", MathFunctionKind::Ceil),
    ("$sin", MathFunctionKind::Sin),
    ("$cos", MathFunctionKind::Cos),
    ("$tan", MathFunctionKind::Tan),
    ("$asin", MathFunctionKind::Asin),
    ("$acos", MathFunctionKind::Acos),
    ("$atan", MathFunctionKind::Atan),
    ("$sinh", MathFunctionKind::Sinh),
    ("$cosh", MathFunctionKind::Cosh),
    ("$tanh", MathFunctionKind::Tanh),
    ("$asinh", MathFunctionKind::Asinh),
    ("$acosh", MathFunctionKind::Acosh),
    ("$atanh", MathFunctionKind::Atanh),
    ("$pow", MathFunctionKind::Pow),
    ("$atan2", MathFunctionKind::Atan2),
    ("$hypot", MathFunctionKind::Hypot),
];

impl MathFunctionKind {
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        MATH_FUNCTIONS
            .iter()
            .find_map(|(n, k)| (*n == name).then_some(*k))
    }

    pub(crate) fn name(self) -> &'static str {
        MATH_FUNCTIONS
            .iter()
            .find_map(|(n, k)| (*k == self).then_some(*n))
            .expect("every MathFunctionKind variant is in MATH_FUNCTIONS")
    }

    pub(crate) fn arity(self) -> usize {
        match self {
            MathFunctionKind::Pow | MathFunctionKind::Atan2 | MathFunctionKind::Hypot => 2,
            _ => 1,
        }
    }

    pub(crate) fn is_real_result(self) -> bool {
        !matches!(self, MathFunctionKind::Clog2)
    }
}

// System tasks (`$display`, `$write`, `$finish`, `$stop`). Owned here
// alongside `MathFunctionKind` / `RealConversionKind` so the name ↔ kind
// table is the single source of truth consulted by both the parser (null-
// argument gating via `SystemTask::from_name`) and `system_call::classify`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SystemTask {
    Finish,
    Stop,
    Display,
    DisplayB,
    DisplayO,
    DisplayH,
    Write,
    WriteB,
    WriteO,
    WriteH,
}

const SYSTEM_TASKS: &[(&str, SystemTask)] = &[
    ("$finish", SystemTask::Finish),
    ("$stop", SystemTask::Stop),
    ("$display", SystemTask::Display),
    ("$displayb", SystemTask::DisplayB),
    ("$displayo", SystemTask::DisplayO),
    ("$displayh", SystemTask::DisplayH),
    ("$write", SystemTask::Write),
    ("$writeb", SystemTask::WriteB),
    ("$writeo", SystemTask::WriteO),
    ("$writeh", SystemTask::WriteH),
];

impl SystemTask {
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        SYSTEM_TASKS
            .iter()
            .find_map(|(n, k)| (*n == name).then_some(*k))
    }

    // LRM 17.1: `$display`/`$write` default to decimal, the `b`/`o`/`h`
    // suffixed variants default to binary / octal / hex respectively.
    // Explicit format controls in the format string still override this.
    pub(crate) fn default_base(self) -> Base {
        match self {
            SystemTask::Display | SystemTask::Write => Base::Decimal,
            SystemTask::DisplayB | SystemTask::WriteB => Base::Binary,
            SystemTask::DisplayO | SystemTask::WriteO => Base::Octal,
            SystemTask::DisplayH | SystemTask::WriteH => Base::Hex,
            SystemTask::Finish | SystemTask::Stop => Base::Decimal,
        }
    }

    pub(crate) fn appends_newline(self) -> bool {
        matches!(
            self,
            SystemTask::Display
                | SystemTask::DisplayB
                | SystemTask::DisplayO
                | SystemTask::DisplayH
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnaryOp {
    Plus,
    Minus,
    LogicalNot,
    BitwiseNot,
    ReductionAnd,
    ReductionNand,
    ReductionOr,
    ReductionNor,
    ReductionXor,
    ReductionXnor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulus,
    Power,
    LessThan,
    GreaterThan,
    LessThanOrEqual,
    GreaterThanOrEqual,
    Equal,
    NotEqual,
    CaseEqual,
    CaseNotEqual,
    LogicalAnd,
    LogicalOr,
    BitwiseAnd,
    BitwiseOr,
    BitwiseXor,
    BitwiseXnor,
    LogicalShiftLeft,
    LogicalShiftRight,
    ArithmeticShiftLeft,
    ArithmeticShiftRight,
}

// Verilog binary-operator precedence per LRM Table 5-4. Higher binds
// tighter; the `(lbp, rbp)` pair is the standard Pratt left/right
// binding-power encoding — left-associative ops use `rbp = lbp + 1` (so a
// same-precedence operator on the right doesn't bind onto the rhs),
// right-associative ops use `rbp <= lbp`. `**` is left-associative here
// to preserve the previous `parse_power` behavior (the LRM 1364-2005
// Table 22 pins unary tighter than `**`, but does not mandate
// associativity of `**` itself; iverilog evaluates left-to-right).
//
// Returns `None` for any token that isn't a binary operator — the Pratt
// loop uses that as the signal to stop extending an expression.
fn infix_binding_power(token: &Token) -> Option<(BinaryOp, u8, u8)> {
    let (op, lbp, rbp) = match token {
        Token::LogicalOr => (BinaryOp::LogicalOr, 20, 21),
        Token::LogicalAnd => (BinaryOp::LogicalAnd, 30, 31),
        Token::BitwiseOr => (BinaryOp::BitwiseOr, 40, 41),
        Token::BitwiseXor => (BinaryOp::BitwiseXor, 50, 51),
        Token::BitwiseXnor => (BinaryOp::BitwiseXnor, 50, 51),
        Token::BitwiseAnd => (BinaryOp::BitwiseAnd, 60, 61),
        Token::EqualEqual => (BinaryOp::Equal, 70, 71),
        Token::NotEqual => (BinaryOp::NotEqual, 70, 71),
        Token::CaseEqual => (BinaryOp::CaseEqual, 70, 71),
        Token::CaseNotEqual => (BinaryOp::CaseNotEqual, 70, 71),
        Token::Less => (BinaryOp::LessThan, 80, 81),
        Token::Greater => (BinaryOp::GreaterThan, 80, 81),
        Token::LessEqual => (BinaryOp::LessThanOrEqual, 80, 81),
        Token::GreaterEqual => (BinaryOp::GreaterThanOrEqual, 80, 81),
        Token::LogicalShiftLeft => (BinaryOp::LogicalShiftLeft, 90, 91),
        Token::LogicalShiftRight => (BinaryOp::LogicalShiftRight, 90, 91),
        Token::ArithmeticShiftLeft => (BinaryOp::ArithmeticShiftLeft, 90, 91),
        Token::ArithmeticShiftRight => (BinaryOp::ArithmeticShiftRight, 90, 91),
        Token::Plus => (BinaryOp::Add, 100, 101),
        Token::Minus => (BinaryOp::Subtract, 100, 101),
        Token::Star => (BinaryOp::Multiply, 110, 111),
        Token::Slash => (BinaryOp::Divide, 110, 111),
        Token::Percent => (BinaryOp::Modulus, 110, 111),
        Token::Power => (BinaryOp::Power, 120, 121),
        _ => return None,
    };
    Some((op, lbp, rbp))
}

// `?:` ternary precedence per LRM Table 5-4: sits below `||`, right-
// associative. Special-cased in `parse_expr_bp` because the right-hand
// side has two parts (then/else) instead of one. Right-associative
// chaining (`a ? b : c ? d : e` → `a ? b : (c ? d : e)`) falls out of
// recursing into the else branch with `min_bp = COND_RBP < COND_LBP`.
const COND_LBP: u8 = 10;
const COND_RBP: u8 = 9;

// Prefix unary operators per LRM 5.1.2 / Table 5-3 plus the reduction
// forms (LRM 5.1.11). Returns `None` for any token that isn't a unary
// operator at this position. Drives the iterative prefix-op accumulator
// in `parse_unary`.
fn prefix_unary_op(token: &Token) -> Option<UnaryOp> {
    Some(match token {
        Token::Plus => UnaryOp::Plus,
        Token::Minus => UnaryOp::Minus,
        Token::Bang => UnaryOp::LogicalNot,
        Token::Tilde => UnaryOp::BitwiseNot,
        Token::BitwiseAnd => UnaryOp::ReductionAnd,
        Token::BitwiseOr => UnaryOp::ReductionOr,
        Token::BitwiseXor => UnaryOp::ReductionXor,
        Token::BitwiseXnor => UnaryOp::ReductionXnor,
        Token::BitwiseNand => UnaryOp::ReductionNand,
        Token::BitwiseNor => UnaryOp::ReductionNor,
        _ => return None,
    })
}

struct Parser<'a> {
    tokens: &'a [Token],
    index: usize,
}

// Continuation frame for the iterative `parse_expr_bp` state machine.
// Each frame represents work deferred while a sub-expression is being
// parsed — the heap-allocated equivalent of a recursive parse_expr_bp
// call. `saved_min_bp` restores the surrounding precedence context when
// the frame reduces.
enum Pending {
    /// `(` consumed, parsing the inner expression. `unary_wrap` carries
    /// any prefix unary operators that appeared immediately before this
    /// `(`, so they apply *after* the matching `)` closes — this is what
    /// makes `!(((1)))` end up with `Unary` outside `Grouped` rather than
    /// the other way around.
    Group {
        unary_wrap: Vec<UnaryOp>,
        saved_min_bp: u8,
    },
    /// `lhs op` consumed; awaiting the right-hand side of a binary op.
    BinaryAwaitRhs {
        lhs: Expr,
        op: BinaryOp,
        saved_min_bp: u8,
    },
    /// `cond ?` consumed; parsing the then-branch at min_bp = 0 (anchored
    /// by the upcoming `:`).
    ConditionalThen { cond: Expr, saved_min_bp: u8 },
    /// `cond ? then :` consumed; parsing the else-branch at
    /// min_bp = COND_RBP (so chained `a ? b : c ? d : e` becomes right-
    /// associative).
    ConditionalElse {
        cond: Expr,
        then: Expr,
        saved_min_bp: u8,
    },
    /// `{` consumed; parsing the items of either a concatenation or the
    /// count of a replication. The disambiguation happens after the
    /// first inner expression reduces: a following `{` triggers the
    /// transition to `Replication`; `,` keeps collecting concat items;
    /// `}` finalizes as `Concatenation`. `unary_wrap` is the prefix
    /// unary chain that appeared immediately before the `{`, applied
    /// to the finalized result the same way `Group` does for `(`.
    Brace {
        items: Vec<Expr>,
        unary_wrap: Vec<UnaryOp>,
        saved_min_bp: u8,
    },
    /// `{ count {` consumed; collecting comma-separated inner items.
    /// Closes with `}}` (inner `}` then outer `}`). The `count`
    /// expression came from the `Brace` frame's first reduced item.
    Replication {
        count: Expr,
        items: Vec<Expr>,
        unary_wrap: Vec<UnaryOp>,
        saved_min_bp: u8,
    },
    /// `$name(` consumed; collecting comma-separated args. Closes with
    /// `)`. No name dispatch happens here — every `$name(args)` builds
    /// `Expr::SystemCall { name, args }`. Name validation, arity, and
    /// real-arg checks all live in the validator (`classify_system_call`
    /// in system_call.rs).
    SystemCallArgs {
        name: String,
        args: Vec<SystemArg>,
        unary_wrap: Vec<UnaryOp>,
        saved_min_bp: u8,
    },
}

// Wrap an `Expr` with a chain of prefix unary operators in source
// order — the operators are applied right-to-left so the rightmost
// (innermost) prefix wraps `expr` first. Used by `parse_expr_bp` when
// finalizing a primary or a frame-built Expr that carried prefix ops
// across the open delimiter.
fn apply_prefix_unary_ops(mut expr: Expr, ops: Vec<UnaryOp>) -> Expr {
    for op in ops.into_iter().rev() {
        expr = Expr::Unary {
            op,
            expr: Box::new(expr),
        };
    }
    expr
}

fn system_call_accepts_null_args(name: &str) -> bool {
    SystemTask::from_name(name).is_some()
}

#[cfg(test)]
pub(crate) fn parse_expression(input: &str) -> Result<Expr, String> {
    let tokens = tokenize(input)?;
    if tokens.is_empty() {
        return Err("empty expression".to_string());
    }

    let mut parser = Parser {
        tokens: &tokens,
        index: 0,
    };
    let expression = parser.parse_expression()?;

    if parser.peek().is_some() {
        return Err("unexpected token after end of expression".to_string());
    }

    Ok(expression)
}

// Returns the parsed statements and a flag indicating whether the input
// ended with a `;` (after the final non-semicolon token). The REPL uses
// the flag as an IPython-style output-suppression marker — see
// `doc/non-standard.md`'s "Trailing semicolons" section.
pub(crate) fn parse_statements(input: &str) -> Result<(Vec<Stmt>, bool), String> {
    let tokens = tokenize(input)?;
    let trailing_semicolon = matches!(tokens.last(), Some(Token::Semicolon));

    let segments: Vec<&[Token]> = tokens
        .split(|t| matches!(t, Token::Semicolon))
        .filter(|s| !s.is_empty())
        .collect();

    if segments.is_empty() {
        return Ok((Vec::new(), trailing_semicolon));
    }

    let mut stmts = Vec::with_capacity(segments.len());
    for segment in segments {
        let mut parser = Parser {
            tokens: segment,
            index: 0,
        };
        let stmt = parser.parse_statement()?;
        if parser.peek().is_some() {
            return Err("unexpected token after end of statement".to_string());
        }
        stmts.push(stmt);
    }

    Ok((stmts, trailing_semicolon))
}

impl<'a> Parser<'a> {
    fn parse_expression(&mut self) -> Result<Expr, String> {
        self.parse_expr_bp(0)
    }

    // Iterative shift-reduce Pratt parser. Replaces the recursive Pratt
    // version (which still recursed for binary RHS, ternary then/else, and
    // — most importantly — every `(` via parse_primary's LParen branch).
    // The Rust call stack stays at one frame regardless of expression
    // depth; the equivalent of the call stack lives on the heap as
    // `stack: Vec<Pending>`.
    //
    // The driver alternates between two states:
    //   - `value.is_none()` — need to read an operand. Collect prefix
    //     unary ops; if the next token is `(`, push a Group frame
    //     (carrying the unary ops so they apply *after* `)` closes); else
    //     read a non-paren primary via parse_primary and wrap with the
    //     collected unary ops.
    //   - `value.is_some()` — have an operand. Try to extend with `?`,
    //     a binary op (lbp >= min_bp), or fall through to "reduce": pop
    //     a pending frame and combine.
    //
    // Same parse tree as the recursive version: left-associativity and
    // right-associative `?:` come from the (lbp, rbp) table; `**` is left-
    // associative; precedence ordering matches LRM Table 5-4.
    //
    // The `min_bp` parameter is kept for API compatibility but only ever
    // called as `parse_expr_bp(0)` from `parse_expression`. The state
    // machine's local `min_bp` variable is what does the actual work.
    fn parse_expr_bp(&mut self, initial_min_bp: u8) -> Result<Expr, String> {
        let mut min_bp = initial_min_bp;
        let mut stack: Vec<Pending> = Vec::new();
        let mut value: Option<Expr> = None;

        loop {
            if value.is_none() {
                if matches!(
                    stack.last(),
                    Some(Pending::SystemCallArgs { name, .. })
                        if system_call_accepts_null_args(name)
                ) {
                    if matches!(self.peek(), Some(Token::Comma)) {
                        self.index += 1;
                        let Some(Pending::SystemCallArgs { args, .. }) = stack.last_mut() else {
                            unreachable!("matched SystemCallArgs above");
                        };
                        args.push(SystemArg::Null);
                        continue;
                    }

                    if matches!(self.peek(), Some(Token::RParen)) {
                        self.index += 1;
                        let frame = stack.pop().expect("just inspected via last()");
                        let Pending::SystemCallArgs {
                            name,
                            mut args,
                            unary_wrap,
                            saved_min_bp,
                        } = frame
                        else {
                            unreachable!("matched SystemCallArgs above");
                        };
                        args.push(SystemArg::Null);
                        value = Some(apply_prefix_unary_ops(
                            Expr::SystemCall { name, args },
                            unary_wrap,
                        ));
                        min_bp = saved_min_bp;
                        continue;
                    }
                }

                // State: need an operand. Read prefix unary ops, then
                // dispatch on the next token. The opening tokens that
                // would otherwise re-enter `parse_expression` (`(`, `{`,
                // `$name`) are handled here by pushing a heap frame onto
                // `stack` and continuing; that's what keeps the Rust
                // call stack at one frame regardless of input depth.
                let mut prefix_ops: Vec<UnaryOp> = Vec::new();
                while let Some(op) = self.peek().and_then(prefix_unary_op) {
                    self.index += 1;
                    prefix_ops.push(op);
                }

                if matches!(self.peek(), Some(Token::LParen)) {
                    self.index += 1;
                    stack.push(Pending::Group {
                        unary_wrap: prefix_ops,
                        saved_min_bp: min_bp,
                    });
                    min_bp = 0;
                    continue;
                }

                if matches!(self.peek(), Some(Token::LBrace)) {
                    self.index += 1;
                    stack.push(Pending::Brace {
                        items: Vec::new(),
                        unary_wrap: prefix_ops,
                        saved_min_bp: min_bp,
                    });
                    min_bp = 0;
                    continue;
                }

                if matches!(self.peek(), Some(Token::SystemIdentifier(_))) {
                    let name = match self.next() {
                        Some(Token::SystemIdentifier(n)) => n.clone(),
                        _ => unreachable!("matches! guard guarantees SystemIdentifier"),
                    };
                    // Every `$name` / `$name()` / `$name(args)` builds
                    // the same generic `Expr::SystemCall { name, args }`.
                    // The validator (`classify_system_call`) owns the
                    // name → kind table and decides whether the name is
                    // a math fn (with arity check), real conversion,
                    // sign cast, base cast, task, or unknown.
                    if !matches!(self.peek(), Some(Token::LParen)) {
                        value = Some(apply_prefix_unary_ops(
                            Expr::SystemCall {
                                name,
                                args: Vec::new(),
                            },
                            prefix_ops,
                        ));
                        continue;
                    }
                    self.index += 1; // consume `(`
                    if matches!(self.peek(), Some(Token::RParen)) {
                        self.index += 1;
                        value = Some(apply_prefix_unary_ops(
                            Expr::SystemCall {
                                name,
                                args: Vec::new(),
                            },
                            prefix_ops,
                        ));
                        continue;
                    }
                    stack.push(Pending::SystemCallArgs {
                        name,
                        args: Vec::new(),
                        unary_wrap: prefix_ops,
                        saved_min_bp: min_bp,
                    });
                    min_bp = 0;
                    continue;
                }

                // Non-paren / non-brace / non-system primary.
                // parse_primary's LParen / LBrace / SystemIdentifier
                // branches are unreachable from here because we just
                // handled them; everything else (literals, identifiers,
                // identifier-with-`[...]`) flows through.
                let primary = self.parse_primary()?;
                value = Some(apply_prefix_unary_ops(primary, prefix_ops));
                continue;
            }

            // State: have an operand. Try to extend.
            if matches!(self.peek(), Some(Token::Question)) && COND_LBP >= min_bp {
                self.index += 1;
                let cond = value.take().expect("value is Some in this branch");
                stack.push(Pending::ConditionalThen {
                    cond,
                    saved_min_bp: min_bp,
                });
                min_bp = 0;
                continue;
            }

            if let Some((op, lbp, rbp)) = self.peek().and_then(infix_binding_power)
                && lbp >= min_bp
            {
                self.index += 1;
                let lhs = value.take().expect("value is Some in this branch");
                stack.push(Pending::BinaryAwaitRhs {
                    lhs,
                    op,
                    saved_min_bp: min_bp,
                });
                min_bp = rbp;
                continue;
            }

            // No infix / `?` extension possible. Reduce or close. The
            // collector frames (Brace, Replication, SystemArgs,
            // SystemTask) dispatch on the next token: `,` keeps the
            // frame alive and starts a new operand; the matching
            // closer pops the frame and produces the parent `Expr`.
            // Single-sub frames (Group, BinaryAwaitRhs, Conditional*)
            // pop unconditionally and combine, same as the original
            // iterative driver.
            match stack.last() {
                None => {
                    return Ok(value.take().expect("value is Some at end of expression"));
                }
                Some(Pending::Group { .. }) => {
                    match self.next() {
                        Some(Token::RParen) => {}
                        _ => return Err("missing closing parenthesis".to_string()),
                    }
                    let inner = value.take().expect("value is Some when reducing");
                    let frame = stack.pop().expect("just inspected via last()");
                    let Pending::Group {
                        unary_wrap,
                        saved_min_bp,
                    } = frame
                    else {
                        unreachable!("matched Group above");
                    };
                    let wrapped =
                        apply_prefix_unary_ops(Expr::Grouped(Box::new(inner)), unary_wrap);
                    value = Some(wrapped);
                    min_bp = saved_min_bp;
                }
                Some(Pending::BinaryAwaitRhs { .. }) => {
                    let rhs = value.take().expect("value is Some when reducing");
                    let frame = stack.pop().expect("just inspected via last()");
                    let Pending::BinaryAwaitRhs {
                        lhs,
                        op,
                        saved_min_bp,
                    } = frame
                    else {
                        unreachable!("matched BinaryAwaitRhs above");
                    };
                    value = Some(Expr::Binary {
                        op,
                        lhs: Box::new(lhs),
                        rhs: Box::new(rhs),
                    });
                    min_bp = saved_min_bp;
                }
                Some(Pending::ConditionalThen { .. }) => {
                    match self.next() {
                        Some(Token::Colon) => {}
                        _ => return Err("expected `:` in conditional expression".to_string()),
                    }
                    let then = value.take().expect("value is Some when reducing");
                    let frame = stack.pop().expect("just inspected via last()");
                    let Pending::ConditionalThen { cond, saved_min_bp } = frame else {
                        unreachable!("matched ConditionalThen above");
                    };
                    stack.push(Pending::ConditionalElse {
                        cond,
                        then,
                        saved_min_bp,
                    });
                    min_bp = COND_RBP;
                    value = None;
                }
                Some(Pending::ConditionalElse { .. }) => {
                    let else_expr = value.take().expect("value is Some when reducing");
                    let frame = stack.pop().expect("just inspected via last()");
                    let Pending::ConditionalElse {
                        cond,
                        then,
                        saved_min_bp,
                    } = frame
                    else {
                        unreachable!("matched ConditionalElse above");
                    };
                    value = Some(Expr::Conditional {
                        cond: Box::new(cond),
                        then_expr: Box::new(then),
                        else_expr: Box::new(else_expr),
                    });
                    min_bp = saved_min_bp;
                }
                Some(Pending::Brace { items, .. }) => {
                    let items_empty = items.is_empty();
                    if matches!(self.peek(), Some(Token::Comma)) {
                        // Continue collecting concat items. Push the
                        // just-parsed value into the frame's `items`
                        // list and reset to operand-needed state.
                        self.index += 1;
                        let v = value.take().expect("value is Some when reducing");
                        let Some(Pending::Brace { items, .. }) = stack.last_mut() else {
                            unreachable!("matched Brace above");
                        };
                        items.push(v);
                    } else if matches!(self.peek(), Some(Token::RBrace)) {
                        // Finalize as Concatenation.
                        self.index += 1;
                        let v = value.take().expect("value is Some when reducing");
                        let frame = stack.pop().expect("just inspected via last()");
                        let Pending::Brace {
                            mut items,
                            unary_wrap,
                            saved_min_bp,
                        } = frame
                        else {
                            unreachable!("matched Brace above");
                        };
                        items.push(v);
                        value = Some(apply_prefix_unary_ops(
                            Expr::Concatenation { items },
                            unary_wrap,
                        ));
                        min_bp = saved_min_bp;
                    } else if items_empty && matches!(self.peek(), Some(Token::LBrace)) {
                        // First inner expression was followed by `{`:
                        // transition to Replication. The just-parsed
                        // value is the count; the `Brace`'s unary_wrap
                        // / saved_min_bp carry over to the new frame.
                        self.index += 1; // consume `{`
                        let count = value.take().expect("value is Some when reducing");
                        let frame = stack.pop().expect("just inspected via last()");
                        let Pending::Brace {
                            unary_wrap,
                            saved_min_bp,
                            ..
                        } = frame
                        else {
                            unreachable!("matched Brace above");
                        };
                        stack.push(Pending::Replication {
                            count,
                            items: Vec::new(),
                            unary_wrap,
                            saved_min_bp,
                        });
                        min_bp = 0;
                    } else {
                        return Err("missing closing brace in concatenation".to_string());
                    }
                }
                Some(Pending::Replication { .. }) => {
                    if matches!(self.peek(), Some(Token::Comma)) {
                        self.index += 1;
                        let v = value.take().expect("value is Some when reducing");
                        let Some(Pending::Replication { items, .. }) = stack.last_mut() else {
                            unreachable!("matched Replication above");
                        };
                        items.push(v);
                    } else if matches!(self.peek(), Some(Token::RBrace)) {
                        // Inner `}` consumed; expect the outer `}` to
                        // finalize the replication. The two-`}`
                        // sequence matches the legacy walker's
                        // `parse_concatenation_items` returning at the
                        // inner `}` and the outer `parse_brace_primary`
                        // requiring its own `}`.
                        self.index += 1;
                        match self.next() {
                            Some(Token::RBrace) => {}
                            _ => return Err("missing closing brace in replication".to_string()),
                        }
                        let v = value.take().expect("value is Some when reducing");
                        let frame = stack.pop().expect("just inspected via last()");
                        let Pending::Replication {
                            count,
                            mut items,
                            unary_wrap,
                            saved_min_bp,
                        } = frame
                        else {
                            unreachable!("matched Replication above");
                        };
                        items.push(v);
                        value = Some(apply_prefix_unary_ops(
                            Expr::Replication {
                                count: Box::new(count),
                                items,
                            },
                            unary_wrap,
                        ));
                        min_bp = saved_min_bp;
                    } else {
                        return Err("missing closing brace in concatenation".to_string());
                    }
                }
                Some(Pending::SystemCallArgs { name, .. }) => {
                    if matches!(self.peek(), Some(Token::Comma)) {
                        self.index += 1;
                        let v = value.take().expect("value is Some when reducing");
                        let Some(Pending::SystemCallArgs { args, .. }) = stack.last_mut() else {
                            unreachable!("matched SystemCallArgs above");
                        };
                        args.push(SystemArg::Expr(v));
                    } else if matches!(self.peek(), Some(Token::RParen)) {
                        self.index += 1;
                        let v = value.take().expect("value is Some when reducing");
                        let frame = stack.pop().expect("just inspected via last()");
                        let Pending::SystemCallArgs {
                            name,
                            mut args,
                            unary_wrap,
                            saved_min_bp,
                        } = frame
                        else {
                            unreachable!("matched SystemCallArgs above");
                        };
                        args.push(SystemArg::Expr(v));
                        value = Some(apply_prefix_unary_ops(
                            Expr::SystemCall { name, args },
                            unary_wrap,
                        ));
                        min_bp = saved_min_bp;
                    } else {
                        return Err(format!("expected `)` after {name} argument"));
                    }
                }
            }
        }
    }

    // Statement-level dispatch (LRM A.2.1.3 reg decl / A.6.2 blocking
    // assignment / expression as a calculator line). Keyword recognition is
    // string-based on `Token::Identifier`; with only two keywords (`reg`,
    // `signed`) a dedicated `Token::Keyword` would be premature.
    //
    // The blocking-assignment LHS can be a bare name, a bit/part-select on
    // a name, or an arbitrarily nested concatenation of those — `name`,
    // `name[...]`, and `{...}` are all already valid `Expr` shapes, so we
    // parse the LHS as an `Expr` first and convert it to an `LValue` via
    // `expression_to_lvalue` only after spotting `=`. If `=` doesn't follow
    // we keep the parsed `Expr` as the statement payload — no rewind, no
    // double parse. The leading-token gate keeps the existing
    // `$finish`/expression path undisturbed.
    fn parse_statement(&mut self) -> Result<Stmt, String> {
        if let Some(Token::Identifier(name)) = self.peek() {
            let kind = match name.as_str() {
                "reg" => Some(DeclKind::Reg),
                "integer" => Some(DeclKind::Integer),
                "real" => Some(DeclKind::Real),
                _ => None,
            };
            if let Some(kind) = kind {
                self.index += 1;
                return self.parse_decl(kind);
            }
        }

        if matches!(self.peek(), Some(Token::Identifier(_) | Token::LBrace)) {
            let expr = self.parse_expression()?;
            if matches!(self.peek(), Some(Token::Assign)) {
                let lvalue = expression_to_lvalue(expr)?;
                self.index += 1; // consume `=`
                let rhs = self.parse_expression()?;
                return Ok(Stmt::Assign { lvalue, rhs });
            }
            return Ok(Stmt::Expr(expr));
        }

        // Top-level system tasks (`$finish`, optionally wrapped in
        // parens) are no longer hoisted here — the lib driver
        // (`apply_stmt`) walks the parsed expression through the
        // iterative `unwrap_grouped` + `classify_system_call` to spot
        // the task and exit. Keeping that recognition out of the parser
        // means `((($finish)))` doesn't pay for a recursive walker.
        let expr = self.parse_expression()?;
        Ok(Stmt::Expr(expr))
    }

    fn parse_decl(&mut self, kind: DeclKind) -> Result<Stmt, String> {
        // LRM 4.8 `integer` is fixed at signed 32-bit and `real` is IEEE
        // 754 binary64 — neither takes a `signed` qualifier or a packed
        // `[range]`. Surface the rejection up front so a typo like
        // `integer signed i` doesn't fall through into the identifier
        // list and produce a confusing "expected identifier" message.
        let signed = if matches!(self.peek(), Some(Token::Identifier(n)) if n == "signed") {
            if !matches!(kind, DeclKind::Reg) {
                return Err(format!(
                    "`signed` qualifier is not allowed on {} declarations",
                    kind.keyword()
                ));
            }
            self.index += 1;
            true
        } else {
            false
        };

        let range = if matches!(self.peek(), Some(Token::LBracket)) {
            if !matches!(kind, DeclKind::Reg) {
                return Err(format!(
                    "packed range `[..]` is not allowed on {} declarations",
                    kind.keyword()
                ));
            }
            self.index += 1;
            let msb = self.parse_expression()?;
            match self.next() {
                Some(Token::Colon) => {}
                _ => return Err("expected `:` in reg range".to_string()),
            }
            let lsb = self.parse_expression()?;
            match self.next() {
                Some(Token::RBracket) => {}
                _ => return Err("expected `]` after reg range".to_string()),
            }
            Some((msb, lsb))
        } else {
            None
        };

        // LRM A.2.3 list_of_variable_identifiers ::=
        //     variable_type { , variable_type }
        // LRM A.2.2.1 variable_type ::=
        //     variable_identifier { dimension }
        //   | variable_identifier = constant_expression
        // The two arms are mutually exclusive in the LRM — an array
        // variable has no init expression — so each item is either
        // `name [ msb : lsb ]` or `name [= expr]`. We accept at most one
        // trailing dimension bracket (vcal's 1-D scope; multi-dim is
        // deferred) and only for `reg` (integer/real arrays are out of
        // scope). The init expression is parsed with `parse_expression`;
        // commas naturally bind to the outer list, never to the init RHS,
        // since no expression-level operator consumes a bare `,`. Inits
        // are evaluated sequentially at apply time so
        // `reg [3:0] a = 1, b = a + 1` sees `a = 1` when binding `b`.
        let mut names: Vec<DeclName> = Vec::new();
        loop {
            let name = match self.next() {
                Some(Token::Identifier(n)) => n.clone(),
                _ => {
                    return Err(format!(
                        "expected identifier in {} declaration",
                        kind.keyword()
                    ));
                }
            };
            if matches!(name.as_str(), "reg" | "integer" | "real" | "signed") {
                return Err(format!(
                    "`{name}` cannot be used as a {} name",
                    kind.keyword()
                ));
            }
            if names.iter().any(|existing| existing.name == name) {
                return Err(format!(
                    "duplicate name in {} declaration: {name}",
                    kind.keyword()
                ));
            }
            // Try the unpacked-dimension form first: a `[` immediately
            // after the name is always an array dimension here, not a
            // select (selects don't appear at decl position). Per LRM
            // A.2.1.3 / A.2.3 the dimension is legal on all three
            // declared kinds — `reg [3:0] a [0:7]` is a vector array,
            // `integer a [0:3]` is an integer array, `real r [0:3]` is
            // a real array. Only the multi-dim form is out of scope
            // (rejected just below).
            let dim = if matches!(self.peek(), Some(Token::LBracket)) {
                self.index += 1;
                let msb = self.parse_expression()?;
                match self.next() {
                    Some(Token::Colon) => {}
                    _ => return Err("expected `:` in array dimension".to_string()),
                }
                let lsb = self.parse_expression()?;
                match self.next() {
                    Some(Token::RBracket) => {}
                    _ => return Err("expected `]` after array dimension".to_string()),
                }
                if matches!(self.peek(), Some(Token::LBracket)) {
                    return Err(
                        "multi-dimensional arrays are not supported (only one `[…]` after the name)"
                            .to_string(),
                    );
                }
                Some((msb, lsb))
            } else {
                None
            };
            let init = if matches!(self.peek(), Some(Token::Assign)) {
                if dim.is_some() {
                    return Err(format!(
                        "array variable `{name}` cannot have an init expression"
                    ));
                }
                self.index += 1;
                Some(self.parse_expression()?)
            } else {
                None
            };
            names.push(DeclName { name, init, dim });
            if matches!(self.peek(), Some(Token::Comma)) {
                self.index += 1;
                continue;
            }
            break;
        }

        Ok(Stmt::Decl {
            kind,
            signed,
            range,
            names,
        })
    }

    // Position-based disambiguation: `&`/`|`/`^`/`~^` (and the alt
    // spelling `^~`) are binary OR unary depending on parse position.
    // The state machine in `parse_expr_bp` claims them at unary position
    // via `prefix_unary_op` before reading a primary; the binary side of
    // `infix_binding_power` only sees them after a primary, so dispatch
    // is unambiguous without a token rewrite. `~&` and `~|` are
    // unary-only — no binary BP entry consumes them, so a free-standing
    // `a ~& b` cleanly fails as "unexpected token".
    //
    // `parse_primary` reads only non-paren primaries: literals,
    // identifiers (with optional bit/part-select), system function calls,
    // and `{...}` brace forms. The iterative state machine handles `(`
    // itself before reaching here, so an LParen at this point would mean
    // we were called out of context (e.g., from external code that
    // doesn't first peek for `(`). Surface that as an explicit error
    // instead of recursing into parse_expression.
    fn parse_primary(&mut self) -> Result<Expr, String> {
        let token = self.next();
        match token {
            Some(Token::IntegerLiteral(text)) => parse_integer(text).map(Expr::Literal),
            Some(Token::StringLiteral(bytes)) => Ok(Expr::StringLiteral(bytes.clone())),
            Some(Token::RealLiteral(text)) => parse_real(text).map(Expr::RealLiteral),
            Some(Token::Identifier(name)) => {
                let name = name.clone();
                // Bit-select / part-select picked up here, not at
                // statement level — so `r[0]` in expression position works
                // while `4'b1111[0]` (literal primary) still parse-errors
                // because we never reach this branch.
                if matches!(self.peek(), Some(Token::LBracket)) {
                    self.index += 1;
                    self.parse_select_after_bracket(name)
                } else {
                    Ok(Expr::Identifier(name))
                }
            }
            // `(`, `{`, and `$name` are all consumed by `parse_expr_bp`'s
            // iterative driver before it calls `parse_primary` — the
            // driver pushes a heap frame instead of recursing, which is
            // the whole point of the iterative parser. Any of these
            // arms reaching `parse_primary` means a future caller
            // skipped that dispatch, which is a programming-contract
            // violation, not a user-input error.
            Some(Token::LParen) => unreachable!(
                "parse_primary must not be called on `(`; parse_expr_bp consumes LParen itself"
            ),
            Some(Token::LBrace) => unreachable!(
                "parse_primary must not be called on `{{`; parse_expr_bp consumes LBrace itself"
            ),
            Some(Token::SystemIdentifier(_)) => unreachable!(
                "parse_primary must not be called on `$name`; parse_expr_bp consumes SystemIdentifier itself"
            ),
            Some(Token::RParen) => Err("unexpected closing parenthesis".to_string()),
            Some(_) => Err("expected expression operand".to_string()),
            None => Err("unexpected end of expression".to_string()),
        }
    }

    // Caller has consumed the `[` after an identifier; dispatch on the
    // separator after the first sub-expression to pick the select form.
    // Whitespace-around-`+`/`-` in the indexed-select forms doesn't pass
    // through here because the lexer rejects it: `+:`/`-:` are
    // adjacency-only tokens.
    //
    // After the first bracket pair is consumed we peek for a second `[`.
    // If present, we parse another `SelectKind` (LRM 4.9 chained
    // array-element select like `a[i][m:l]`). A third bracket is rejected
    // up-front since vcal only supports 1-D unpacked arrays — chaining
    // further would have no LRM meaning under the current grammar.
    fn parse_select_after_bracket(&mut self, name: String) -> Result<Expr, String> {
        let kind = self.parse_select_kind()?;
        let inner = if matches!(self.peek(), Some(Token::LBracket)) {
            self.index += 1;
            let inner_kind = self.parse_select_kind()?;
            if matches!(self.peek(), Some(Token::LBracket)) {
                return Err(
                    "chained selects beyond one inner bracket are not supported".to_string()
                );
            }
            Some(Box::new(inner_kind))
        } else {
            None
        };
        Ok(Expr::Select { name, kind, inner })
    }

    // Parse one `SelectKind` from inside a `[...]` group. The opening `[`
    // has already been consumed by the caller; this method consumes the
    // closing `]` for the matched form. Shared by the outer-bracket parse
    // path and the chained inner-bracket path so both grammars stay in
    // lockstep — adding a new select form here lights up both surfaces.
    fn parse_select_kind(&mut self) -> Result<SelectKind, String> {
        let first = self.parse_expression()?;
        let kind = match self.peek() {
            Some(Token::RBracket) => {
                self.index += 1;
                SelectKind::Bit {
                    index: Box::new(first),
                }
            }
            Some(Token::Colon) => {
                self.index += 1;
                let lsb = self.parse_expression()?;
                match self.next() {
                    Some(Token::RBracket) => {}
                    _ => return Err("expected `]` after part-select range".to_string()),
                }
                SelectKind::PartConst {
                    msb: Box::new(first),
                    lsb: Box::new(lsb),
                }
            }
            Some(Token::PlusColon) => {
                self.index += 1;
                let width = self.parse_expression()?;
                match self.next() {
                    Some(Token::RBracket) => {}
                    _ => return Err("expected `]` after indexed part-select width".to_string()),
                }
                SelectKind::PartIndexedUp {
                    base: Box::new(first),
                    width: Box::new(width),
                }
            }
            Some(Token::MinusColon) => {
                self.index += 1;
                let width = self.parse_expression()?;
                match self.next() {
                    Some(Token::RBracket) => {}
                    _ => return Err("expected `]` after indexed part-select width".to_string()),
                }
                SelectKind::PartIndexedDown {
                    base: Box::new(first),
                    width: Box::new(width),
                }
            }
            _ => return Err("expected `]`, `:`, `+:`, or `-:` in select".to_string()),
        };
        Ok(kind)
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.index)
    }

    fn next(&mut self) -> Option<&Token> {
        let token = self.tokens.get(self.index);
        if token.is_some() {
            self.index += 1;
        }
        token
    }
}

// LRM A.8.5 `variable_lvalue`. Called after `parse_statement` has parsed
// the LHS as an `Expr` and confirmed `=` follows. Accept only the shapes
// the LRM production allows; reject everything else with a uniform
// "invalid lvalue" diagnostic. Leading `Grouped` layers are unwrapped so
// `(a) = 1` works (mirroring how the lib driver walks parens around a
// top-level `$finish` via `unwrap_grouped`).
//
// Now that `Expr` has a custom `Drop` (for the deep-AST overflow fix
// further up), Rust forbids moving owned fields out of `Expr` via
// pattern match — so this function uses `mem::replace`/`mem::take` to
// extract owned data while leaving each `Expr` in a leaf-shaped state
// for its drop. The leading-Grouped peel is also iterative so an
// `((((a))))` LHS doesn't recurse N levels.
//
// The Concat arm is also iterative: `{{{...a}}} = 1` nests a
// `Concatenation` per layer, and a recursive `.map(expression_to_lvalue)`
// would overflow at ~50K depth on the default thread stack. Use a
// `Visit`/`BuildConcat` CES driver so depth becomes heap-allocated work-
// stack growth rather than C-stack frames.
fn expression_to_lvalue(root: Expr) -> Result<LValue, String> {
    let placeholder = || Expr::Identifier(String::new());
    let mut work: Vec<LValueTask> = vec![LValueTask::Visit(root)];
    let mut vals: Vec<LValue> = Vec::new();

    while let Some(task) = work.pop() {
        match task {
            LValueTask::Visit(mut expr) => {
                while let Expr::Grouped(inner) = &mut expr {
                    expr = std::mem::replace(inner.as_mut(), placeholder());
                }
                match &mut expr {
                    Expr::Identifier(name) => {
                        vals.push(LValue::Name(std::mem::take(name)));
                    }
                    // Chained selects (`a[i][m:l]`) pass straight through: on
                    // the LHS the evaluator routes the array-element + inner-
                    // select case through the same per-position distribution
                    // path the vector-reg LHS uses (LRM 4.9). The structural
                    // validation (only `Bit` outer on an array, inner
                    // forbidden on a vector, inner part-select direction
                    // matches the element's packed range) happens in
                    // `lvalue_meta`, so the parser stays purely syntactic
                    // here.
                    Expr::Select { name, kind, inner } => {
                        let name = std::mem::take(name);
                        let kind_placeholder = SelectKind::Bit {
                            index: Box::new(placeholder()),
                        };
                        let kind = std::mem::replace(kind, kind_placeholder);
                        let inner = inner.take();
                        vals.push(LValue::Select { name, kind, inner });
                    }
                    Expr::Concatenation { items } => {
                        let items = std::mem::take(items);
                        let count = items.len();
                        work.push(LValueTask::BuildConcat(count));
                        // Push in reverse so items[0] visits first, lands
                        // first on `vals`, and BuildConcat drains them in
                        // source order.
                        for item in items.into_iter().rev() {
                            work.push(LValueTask::Visit(item));
                        }
                    }
                    Expr::Replication { .. } => {
                        return Err(
                            "invalid lvalue: replication is not a variable_lvalue".to_string()
                        );
                    }
                    _ => {
                        return Err(
                            "invalid lvalue: expected name, bit/part-select, or concatenation"
                                .to_string(),
                        );
                    }
                }
            }
            LValueTask::BuildConcat(count) => {
                let start = vals.len() - count;
                let items: Vec<LValue> = vals.drain(start..).collect();
                vals.push(LValue::Concat(items));
            }
        }
    }

    debug_assert_eq!(
        vals.len(),
        1,
        "expression_to_lvalue produced {} values",
        vals.len()
    );
    Ok(vals
        .pop()
        .expect("driver invariant: one root produces one LValue"))
}

enum LValueTask {
    Visit(Expr),
    BuildConcat(usize),
}

// LRM §3.5.2: real constants follow IEEE 754 binary64. The lexer has
// already validated the digit-on-each-side rule and the optional exponent
// form, so here we only strip underscores (legal anywhere except the first
// position, ignored per §3.5.2) and hand the result to f64::from_str.
pub(crate) fn parse_real(input: &str) -> Result<f64, String> {
    let stripped = strip_underscores(input);
    stripped
        .parse::<f64>()
        .map_err(|_| format!("invalid real literal: {input}"))
}

pub(crate) fn parse_integer(input: &str) -> Result<LiteralSpec, String> {
    match input.find('\'') {
        Some(apostrophe_index) => parse_based_integer(input, apostrophe_index),
        None => parse_unsized_decimal(input),
    }
}

pub(crate) fn string_literal_spec(bytes: &[u8]) -> LiteralSpec {
    let width = bytes.len().max(1) * 8;
    let mut low_bits = Vec::with_capacity(width);
    // Source order maps left-to-right onto MSB-to-LSB. Since IntegerValue
    // stores bits LSB-first, push bytes from the right end of the string.
    if bytes.is_empty() {
        push_integer_bits(0, 8, &mut low_bits);
    } else {
        for byte in bytes.iter().rev() {
            push_integer_bits(*byte, 8, &mut low_bits);
        }
    }
    LiteralSpec {
        width,
        signed: false,
        base: Base::Hex,
        unsized_literal: false,
        payload: LiteralPayload::Bits {
            low_bits,
            fill: LogicBit::Zero,
        },
    }
}

fn parse_unsized_decimal(input: &str) -> Result<LiteralSpec, String> {
    ensure_no_leading_underscore(input)?;
    let digits = strip_underscores(input);
    ensure_decimal_digits(&digits)?;

    let magnitude = parse_biguint(&digits)?;
    let width = usize::max(signed_decimal_bit_len(&magnitude), 32);

    Ok(LiteralSpec {
        width,
        signed: true,
        base: Base::Decimal,
        unsized_literal: true,
        payload: LiteralPayload::Numeric { magnitude },
    })
}

fn parse_based_integer(input: &str, apostrophe_index: usize) -> Result<LiteralSpec, String> {
    let (size_part, rest) = input.split_at(apostrophe_index);
    let mut rest = &rest[1..];
    let width = if size_part.is_empty() {
        None
    } else {
        Some(parse_size(size_part)?)
    };

    let signed = match rest.chars().next() {
        Some('s' | 'S') => {
            rest = &rest[1..];
            true
        }
        _ => false,
    };

    let base_char = rest
        .chars()
        .next()
        .ok_or_else(|| "missing base after apostrophe".to_string())?;
    rest = &rest[base_char.len_utf8()..];

    let base = match base_char.to_ascii_lowercase() {
        'b' => Base::Binary,
        'o' => Base::Octal,
        'd' => Base::Decimal,
        'h' => Base::Hex,
        _ => return Err(format!("unsupported integer base: {base_char}")),
    };

    ensure_no_leading_underscore(rest)?;
    let digits = strip_underscores(rest);
    if digits.is_empty() {
        return Err("missing digits in integer literal".to_string());
    }

    match base {
        Base::Decimal => parse_based_decimal(width, signed, &digits),
        Base::Binary | Base::Octal | Base::Hex => parse_based_radix(width, signed, base, &digits),
    }
}

fn parse_based_decimal(
    width_hint: Option<usize>,
    signed: bool,
    digits: &str,
) -> Result<LiteralSpec, String> {
    let digits = strip_underscores(digits);

    let unsized_literal = width_hint.is_none();

    // All-x and all-z decimal short-circuits — used to allocate the full
    // width directly. Now stored as an empty bit prefix plus an X/Z fill;
    // materialize() expands at eval time after the validator caps width.
    if digits.chars().all(is_x_digit) {
        let width = width_hint.unwrap_or(32);
        return Ok(LiteralSpec {
            width,
            signed,
            base: Base::Decimal,
            unsized_literal,
            payload: LiteralPayload::Bits {
                low_bits: Vec::new(),
                fill: LogicBit::X,
            },
        });
    }

    if digits.chars().all(is_z_digit) {
        let width = width_hint.unwrap_or(32);
        return Ok(LiteralSpec {
            width,
            signed,
            base: Base::Decimal,
            unsized_literal,
            payload: LiteralPayload::Bits {
                low_bits: Vec::new(),
                fill: LogicBit::Z,
            },
        });
    }

    ensure_decimal_digits(&digits)?;

    let magnitude = parse_biguint(&digits)?;
    // For unsized `'sd`, widen by one extra bit so auto-sizing never lands the
    // value's MSB on the sign-bit position and silently flips the literal
    // negative (e.g. `'sd9999999999999999999999999`). Sized forms respect the
    // caller's width verbatim; unsized `'d` keeps its natural unsigned width.
    let width = width_hint.unwrap_or_else(|| {
        let natural = if signed {
            signed_decimal_bit_len(&magnitude)
        } else {
            biguint_bit_len(&magnitude)
        };
        usize::max(natural, 32)
    });

    Ok(LiteralSpec {
        width,
        signed,
        base: Base::Decimal,
        unsized_literal,
        payload: LiteralPayload::Numeric { magnitude },
    })
}

fn parse_based_radix(
    width_hint: Option<usize>,
    signed: bool,
    base: Base,
    digits: &str,
) -> Result<LiteralSpec, String> {
    let digits = strip_underscores(digits);
    let mut low_bits = Vec::with_capacity(digits.len() * base.group_size());

    for digit in digits.chars().rev() {
        push_digit_bits(digit, base, &mut low_bits)?;
    }

    let unsized_literal = width_hint.is_none();
    let width = width_hint.unwrap_or_else(|| usize::max(low_bits.len(), 32));
    let fill = extension_bit(digits.chars().next().expect("digits is not empty"));

    // No `bits.resize(width, fill)` here — materialization happens at eval
    // time, after the validator gates `width` against MAX_BIT_WIDTH. low_bits
    // stays text-bounded so a `9999999999999'h1` parses in O(text).
    Ok(LiteralSpec {
        width,
        signed,
        base,
        unsized_literal,
        payload: LiteralPayload::Bits { low_bits, fill },
    })
}

fn parse_size(input: &str) -> Result<usize, String> {
    ensure_no_leading_underscore(input)?;
    let digits = strip_underscores(input);
    if digits.is_empty() {
        return Err("missing integer size".to_string());
    }

    let mut chars = digits.chars();
    let first = chars.next().expect("digits is not empty");
    if !('1'..='9').contains(&first) || !chars.all(|ch| ch.is_ascii_digit()) {
        return Err(format!("invalid integer size: {input}"));
    }

    digits
        .parse::<usize>()
        .map_err(|_| format!("integer size is too large: {input}"))
}

fn strip_underscores(input: &str) -> Cow<'_, str> {
    if input.contains('_') {
        Cow::Owned(input.chars().filter(|ch| *ch != '_').collect())
    } else {
        Cow::Borrowed(input)
    }
}

fn parse_biguint(digits: &str) -> Result<BigUint, String> {
    BigUint::parse_bytes(digits.as_bytes(), 10)
        .ok_or_else(|| format!("invalid decimal integer: {digits}"))
}

fn ensure_decimal_digits(digits: &str) -> Result<(), String> {
    if digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(format!("invalid decimal digits: {digits}"));
    }

    Ok(())
}

// LRM A.8.7: every number grammar — `unsigned_number`,
// `non_zero_unsigned_number`, and the per-base `*_value` rules — has the
// shape `<digit> { _ | <digit> }`. The leading character is always a
// digit (or `x_digit`/`z_digit` for the based forms); `_` is a separator,
// not a prefix. This must run *before* `strip_underscores`, otherwise an
// illegal leading underscore is silently absorbed and `_1` parses as `1`.
fn ensure_no_leading_underscore(input: &str) -> Result<(), String> {
    if input.starts_with('_') {
        return Err(format!("number cannot start with underscore: {input}"));
    }
    Ok(())
}

fn push_digit_bits(digit: char, base: Base, out: &mut Vec<LogicBit>) -> Result<(), String> {
    let digit = digit.to_ascii_lowercase();

    match base {
        Base::Binary => match digit {
            '0' => out.push(LogicBit::Zero),
            '1' => out.push(LogicBit::One),
            'x' => out.push(LogicBit::X),
            'z' | '?' => out.push(LogicBit::Z),
            _ => return Err(format!("invalid binary digit: {digit}")),
        },
        Base::Octal => match digit {
            'x' => out.extend_from_slice(&[LogicBit::X; 3]),
            'z' | '?' => out.extend_from_slice(&[LogicBit::Z; 3]),
            '0'..='7' => push_integer_bits((digit as u8) - b'0', 3, out),
            _ => return Err(format!("invalid octal digit: {digit}")),
        },
        Base::Hex => match digit {
            'x' => out.extend_from_slice(&[LogicBit::X; 4]),
            'z' | '?' => out.extend_from_slice(&[LogicBit::Z; 4]),
            '0'..='9' => push_integer_bits((digit as u8) - b'0', 4, out),
            'a'..='f' => push_integer_bits((digit as u8) - b'a' + 10, 4, out),
            _ => return Err(format!("invalid hex digit: {digit}")),
        },
        Base::Decimal => return Err("decimal digits are parsed separately".to_string()),
    }
    Ok(())
}

fn push_integer_bits(value: u8, width: usize, out: &mut Vec<LogicBit>) {
    for shift in 0..width {
        out.push(if value & (1 << shift) == 0 {
            LogicBit::Zero
        } else {
            LogicBit::One
        });
    }
}

fn extension_bit(digit: char) -> LogicBit {
    if is_x_digit(digit) {
        LogicBit::X
    } else if is_z_digit(digit) {
        LogicBit::Z
    } else {
        LogicBit::Zero
    }
}

fn is_x_digit(ch: char) -> bool {
    matches!(ch, 'x' | 'X')
}

fn is_z_digit(ch: char) -> bool {
    matches!(ch, 'z' | 'Z' | '?')
}
