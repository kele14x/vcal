use num_bigint::BigUint;
use std::borrow::Cow;

use crate::lexer::{Token, tokenize};
use crate::value::{
    Base, IntegerValue, LogicBit, biguint_bit_len, biguint_to_bits_with_width,
    signed_decimal_bit_len,
};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Expr {
    Literal(IntegerValue),
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
    // LRM 5.5: `$signed(expr)` / `$unsigned(expr)`. The argument is evaluated
    // as a self-determined expression; the result has the same width and bits
    // but with signedness set to `signed`. Outer-context width still flows
    // back through it (handled in eval).
    SignCast {
        signed: bool,
        arg: Box<Expr>,
    },
    // vcal-specific (non-LRM) display-base cast: `$bin(e)`, `$oct(e)`,
    // `$dec(e)`, `$hex(e)`. The argument is evaluated as a self-determined
    // expression; the result has the same width, signedness, and bits but
    // with the display base set to `base`. Outer-context width still flows
    // back through it (handled in eval), mirroring `SignCast`.
    BaseCast {
        base: Base,
        arg: Box<Expr>,
    },
    // LRM 17.8: real-conversion system functions. Each maps between the
    // integer and real domains with a specific semantic — see
    // RealConversionKind for the four variants.
    RealConversion {
        kind: RealConversionKind,
        arg: Box<Expr>,
    },
    // LRM 17.11: math system functions. `$clog2` returns a 32-bit signed
    // integer; the other 21 are real-typed (1- or 2-arg) and follow the
    // C standard library semantics — Rust's `f64::*` methods wrap libm,
    // so the implementation matches by construction. Arity is validated
    // at parse time; see `MathFunctionKind::arity`.
    MathFunction {
        kind: MathFunctionKind,
        args: Vec<Expr>,
    },
    // LRM 17.4: simulation control system tasks (`$finish`, `$stop`).
    // These are statements, not expressions — they have no return value.
    // Parsed here so the parser handles `$finish`/`$stop` identifier-matching
    // uniformly with system functions; the lib-level driver checks for a bare
    // SystemTask at the top of the AST and exits, while the integer/real
    // evaluators reject any nested occurrence with the task-in-expression
    // error.
    //
    // LRM allows an optional `(n)` argument controlling exit-message
    // verbosity (n ∈ {0,1,2}). vcal does not print exit diagnostics, so the
    // argument is meaningless: the parser accepts any number of arguments
    // (including none) and discards them. Arguments are parsed for
    // syntactic validity only; their values are never evaluated.
    SystemTask {
        name: String,
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
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SelectKind {
    // `r[expr]`. `index` is a self-determined integer expression
    // (LRM 4.2.1). Result is 1-bit unsigned.
    Bit { index: Box<Expr> },
    // `r[m:l]`. Both endpoints are constant expressions (LRM 5.2.1).
    // Direction must match the declared reg direction.
    PartConst {
        msb: Box<Expr>,
        lsb: Box<Expr>,
    },
    // `r[base +: width]`. `base` is a self-determined integer expression;
    // `width` is a positive constant (LRM 5.2.2). Result spans the source
    // range `[base, base + width - 1]`.
    PartIndexedUp {
        base: Box<Expr>,
        width: Box<Expr>,
    },
    // `r[base -: width]`. `base` is a self-determined integer expression;
    // `width` is a positive constant (LRM 5.2.2). Result spans the source
    // range `[base - width + 1, base]`.
    PartIndexedDown {
        base: Box<Expr>,
        width: Box<Expr>,
    },
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
        | Expr::RealLiteral(_)
        | Expr::Identifier(_)
        | Expr::SystemTask { .. } => {}
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
            out.extend(items.drain(..));
        }
        Expr::Replication { count, items } => {
            out.push(std::mem::replace(count.as_mut(), placeholder()));
            out.extend(items.drain(..));
        }
        Expr::SignCast { arg, .. }
        | Expr::BaseCast { arg, .. }
        | Expr::RealConversion { arg, .. } => {
            out.push(std::mem::replace(arg.as_mut(), placeholder()));
        }
        Expr::MathFunction { args, .. } => {
            out.extend(args.drain(..));
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
        SelectKind::PartIndexedUp { base, width }
        | SelectKind::PartIndexedDown { base, width } => {
            out.push(std::mem::replace(base.as_mut(), placeholder()));
            out.push(std::mem::replace(width.as_mut(), placeholder()));
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
    // LRM 17.4: `$finish` / `$stop` hoisted to the statement level so the
    // driver can exit without going through the expression evaluator.
    Task(String),
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

#[cfg(test)]
pub(crate) fn parse_expression(input: &str) -> Result<Expr, String> {
    let tokens = tokenize(input)?;
    if tokens.is_empty() {
        return Err("empty expression".to_string());
    }

    let mut parser = Parser { tokens: &tokens, index: 0 };
    let expression = parser.parse_expression()?;

    if parser.peek().is_some() {
        return Err("unexpected token after end of expression".to_string());
    }

    Ok(expression)
}

pub(crate) fn parse_statements(input: &str) -> Result<Vec<Stmt>, String> {
    let tokens = tokenize(input)?;

    let segments: Vec<&[Token]> = tokens
        .split(|t| matches!(t, Token::Semicolon))
        .filter(|s| !s.is_empty())
        .collect();

    if segments.is_empty() {
        return Ok(Vec::new());
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

    Ok(stmts)
}

impl<'a> Parser<'a> {
    fn parse_expression(&mut self) -> Result<Expr, String> {
        self.parse_expr_bp(0)
    }

    // Pratt-style precedence climb: replaces the previous 12-function
    // precedence ladder (parse_conditional → parse_logical_or → ... →
    // parse_power) with one loop driven by `infix_binding_power`. Same parse
    // tree as before — left-associativity, right-associative `?:`, and the
    // historical left-to-right `**` are all preserved by the (lbp, rbp)
    // table at the top of this module.
    //
    // `min_bp` is the lowest binding power this call will accept. An infix
    // operator with `lbp < min_bp` is left for the caller to bind, which is
    // how the precedence ordering and right-associativity emerge.
    //
    // The `?:` ternary is special-cased rather than table-driven because its
    // right-hand side has two operand slots (`then` and `else`), separated
    // by `:`. The middle parses at `min_bp = 0` (anchored by the upcoming
    // `:`), and the else branch parses at `min_bp = COND_RBP` so chained
    // `a ? b : c ? d : e` becomes `a ? b : (c ? d : e)`.
    fn parse_expr_bp(&mut self, min_bp: u8) -> Result<Expr, String> {
        let mut lhs = self.parse_unary()?;

        loop {
            if matches!(self.peek(), Some(Token::Question)) && COND_LBP >= min_bp {
                self.index += 1;
                let then_expr = self.parse_expr_bp(0)?;
                match self.next() {
                    Some(Token::Colon) => {}
                    _ => return Err("expected `:` in conditional expression".to_string()),
                }
                let else_expr = self.parse_expr_bp(COND_RBP)?;
                lhs = Expr::Conditional {
                    cond: Box::new(lhs),
                    then_expr: Box::new(then_expr),
                    else_expr: Box::new(else_expr),
                };
                continue;
            }

            let (op, lbp, rbp) = match self.peek().and_then(infix_binding_power) {
                Some(triple) => triple,
                None => break,
            };
            if lbp < min_bp {
                break;
            }
            self.index += 1;
            let rhs = self.parse_expr_bp(rbp)?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }

        Ok(lhs)
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
            if let Some(name) = top_level_task_name(&expr) {
                return Ok(Stmt::Task(name));
            }
            return Ok(Stmt::Expr(expr));
        }

        let expr = self.parse_expression()?;
        // Hoist a top-level system task (or one wrapped in redundant
        // parentheses) so the driver can exit without invoking the
        // expression evaluator's task-in-expression rejection.
        if let Some(name) = top_level_task_name(&expr) {
            return Ok(Stmt::Task(name));
        }
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
                _ => return Err(format!(
                    "expected identifier in {} declaration",
                    kind.keyword()
                )),
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
    // `parse_unary` claims them at unary position; the binary side of
    // `infix_binding_power` only sees them after a primary, so dispatch is
    // unambiguous without a token rewrite. `~&` and `~|` are unary-only —
    // no binary BP entry consumes them, so a free-standing `a ~& b`
    // cleanly fails as "unexpected token".
    //
    // Iterative: a chain like `!!!!1` accumulates ops in a Vec rather than
    // recursing once per `!`. With the recursive form a long-enough unary
    // chain would overflow the stack the same way `(((...)))` does in
    // `parse_primary`. Wrapping happens in reverse so the outermost
    // operator (first one read) ends up the outermost `Expr::Unary`.
    fn parse_unary(&mut self) -> Result<Expr, String> {
        let mut ops: Vec<UnaryOp> = Vec::new();
        while let Some(op) = self.peek().and_then(prefix_unary_op) {
            self.index += 1;
            ops.push(op);
        }
        let mut expr = self.parse_primary()?;
        while let Some(op) = ops.pop() {
            expr = Expr::Unary {
                op,
                expr: Box::new(expr),
            };
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        let token = self.next();
        match token {
            Some(Token::IntegerLiteral(text)) => parse_integer(text).map(Expr::Literal),
            Some(Token::RealLiteral(text)) => parse_real(text).map(Expr::RealLiteral),
            Some(Token::SystemIdentifier(name)) => {
                let name = name.clone();
                self.parse_system_function_call(&name)
            }
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
            Some(Token::LParen) => {
                let expr = self.parse_expression()?;
                match self.next() {
                    Some(Token::RParen) => Ok(Expr::Grouped(Box::new(expr))),
                    _ => Err("missing closing parenthesis".to_string()),
                }
            }
            Some(Token::LBrace) => self.parse_brace_primary(),
            Some(Token::RParen) => Err("unexpected closing parenthesis".to_string()),
            Some(_) => Err("expected expression operand".to_string()),
            None => Err("unexpected end of expression".to_string()),
        }
    }

    // LRM 5.5 / 17.8: every supported system function in expression
    // position takes exactly one parenthesised argument. Anything else
    // starting with `$` is rejected with a clear message so the generic
    // "expected expression operand" path doesn't fire for typos.
    fn parse_system_function_call(&mut self, name: &str) -> Result<Expr, String> {
        // LRM 17.4 simulation control tasks: `$finish` / `$stop` are
        // statements, not expressions. Parse them as `Expr::SystemTask`
        // here so identifier-matching is uniform with system functions; the
        // top-level driver in lib.rs distinguishes "task at top of AST"
        // (exit) from "task nested in expression" (rejected by evaluator).
        if matches!(name, "$finish" | "$stop") {
            return self.parse_system_task_call(name);
        }

        enum SystemFn {
            SignCast(bool),
            BaseCast(Base),
            RealConversion(RealConversionKind),
            MathFunction(MathFunctionKind),
        }

        // $signed/$unsigned, the four base casts, and the four real-conversion
        // casts are listed explicitly here; everything else falls through to
        // the MATH_FUNCTIONS table so a new math function only needs adding
        // there. `from_name` is the inverse of `MathFunctionKind::name()`.
        let kind = match name {
            "$signed" => SystemFn::SignCast(true),
            "$unsigned" => SystemFn::SignCast(false),
            "$bin" => SystemFn::BaseCast(Base::Binary),
            "$oct" => SystemFn::BaseCast(Base::Octal),
            "$dec" => SystemFn::BaseCast(Base::Decimal),
            "$hex" => SystemFn::BaseCast(Base::Hex),
            "$rtoi" => SystemFn::RealConversion(RealConversionKind::RealToInteger),
            "$itor" => SystemFn::RealConversion(RealConversionKind::IntegerToReal),
            "$realtobits" => SystemFn::RealConversion(RealConversionKind::RealToBits),
            "$bitstoreal" => SystemFn::RealConversion(RealConversionKind::BitsToReal),
            _ => match MathFunctionKind::from_name(name) {
                Some(math_kind) => SystemFn::MathFunction(math_kind),
                None => return Err(format!("unsupported system function: {name}")),
            },
        };

        match self.next() {
            Some(Token::LParen) => {}
            _ => return Err(format!("expected `(` after {name}")),
        }

        if let SystemFn::MathFunction(math_kind) = kind {
            let mut args = vec![self.parse_expression()?];
            while matches!(self.peek(), Some(Token::Comma)) {
                self.index += 1;
                args.push(self.parse_expression()?);
            }
            match self.next() {
                Some(Token::RParen) => {}
                _ => return Err(format!("expected `)` after {name} argument")),
            }
            let expected = math_kind.arity();
            if args.len() != expected {
                return Err(format!(
                    "{} expects {expected} argument{plural}, got {actual}",
                    math_kind.name(),
                    plural = if expected == 1 { "" } else { "s" },
                    actual = args.len()
                ));
            }
            return Ok(Expr::MathFunction {
                kind: math_kind,
                args,
            });
        }

        let arg = self.parse_expression()?;

        match self.next() {
            Some(Token::RParen) => {}
            _ => return Err(format!("expected `)` after {name} argument")),
        }

        Ok(match kind {
            SystemFn::SignCast(signed) => Expr::SignCast {
                signed,
                arg: Box::new(arg),
            },
            SystemFn::BaseCast(base) => Expr::BaseCast {
                base,
                arg: Box::new(arg),
            },
            SystemFn::RealConversion(kind) => Expr::RealConversion {
                kind,
                arg: Box::new(arg),
            },
            SystemFn::MathFunction(_) => unreachable!("MathFunction handled above"),
        })
    }

    // LRM 17.4: `$finish[(n)]` / `$stop[(n)]`. Per the LRM the argument
    // controls exit-message verbosity (n ∈ {0,1,2}); vcal prints no exit
    // diagnostic, so the argument is meaningless. The parser is therefore
    // lenient: any number of comma-separated arguments is accepted (the LRM
    // 0-or-1 arity check would only teach users a rule vcal itself does not
    // enforce). Each argument is parsed for syntactic validity so genuine
    // typos like `$finish(1 +)` still surface as a parse error, but the
    // resulting expression values are discarded — they are never evaluated.
    fn parse_system_task_call(&mut self, name: &str) -> Result<Expr, String> {
        if !matches!(self.peek(), Some(Token::LParen)) {
            return Ok(Expr::SystemTask {
                name: name.to_string(),
            });
        }
        self.index += 1; // consume `(`
        if matches!(self.peek(), Some(Token::RParen)) {
            self.index += 1;
            return Ok(Expr::SystemTask {
                name: name.to_string(),
            });
        }
        loop {
            let _ = self.parse_expression()?;
            if matches!(self.peek(), Some(Token::Comma)) {
                self.index += 1;
                continue;
            }
            break;
        }
        match self.next() {
            Some(Token::RParen) => {}
            _ => return Err(format!("expected `)` after {name} arguments")),
        }
        Ok(Expr::SystemTask {
            name: name.to_string(),
        })
    }

    // LRM 5.1.14: `{ expr {, expr} }` (concatenation) or
    // `{ count_expr { expr {, expr} } }` (multiple concatenation /
    // replication). Disambiguated by what follows the first inner expression:
    // a `{` starts the inner concatenation list (replication form), anything
    // else (`,` or `}`) means we're in plain concatenation. The leading `{`
    // has already been consumed by `parse_primary`.
    fn parse_brace_primary(&mut self) -> Result<Expr, String> {
        let first = self.parse_expression()?;

        if matches!(self.peek(), Some(Token::LBrace)) {
            self.index += 1;
            let items = self.parse_concatenation_items()?;
            match self.next() {
                Some(Token::RBrace) => {}
                _ => return Err("missing closing brace in replication".to_string()),
            }
            return Ok(Expr::Replication {
                count: Box::new(first),
                items,
            });
        }

        let mut items = vec![first];
        while matches!(self.peek(), Some(Token::Comma)) {
            self.index += 1;
            items.push(self.parse_expression()?);
        }
        match self.next() {
            Some(Token::RBrace) => {}
            _ => return Err("missing closing brace in concatenation".to_string()),
        }
        Ok(Expr::Concatenation { items })
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
                    "chained selects beyond one inner bracket are not supported".to_string(),
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

    fn parse_concatenation_items(&mut self) -> Result<Vec<Expr>, String> {
        let mut items = vec![self.parse_expression()?];
        while matches!(self.peek(), Some(Token::Comma)) {
            self.index += 1;
            items.push(self.parse_expression()?);
        }
        match self.next() {
            Some(Token::RBrace) => Ok(items),
            _ => Err("missing closing brace in concatenation".to_string()),
        }
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

// LRM 17.4: `$finish` / `$stop` are statements that exit. The parser
// produces `Expr::SystemTask` for them so identifier dispatch is uniform;
// this helper hoists a top-level task (optionally wrapped in redundant
// parentheses) up to the `Stmt::Task` layer so the driver can exit before
// the expression evaluator's task-in-expression rule fires.
fn top_level_task_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Grouped(inner) => top_level_task_name(inner),
        Expr::SystemTask { name } => Some(name.clone()),
        _ => None,
    }
}

// LRM A.8.5 `variable_lvalue`. Called after `parse_statement` has parsed
// the LHS as an `Expr` and confirmed `=` follows. Accept only the shapes
// the LRM production allows; reject everything else with a uniform
// "invalid lvalue" diagnostic. Leading `Grouped` layers are unwrapped so
// `(a) = 1` works (matches how `top_level_task_name` walks through parens
// for `($finish)`).
//
// Now that `Expr` has a custom `Drop` (for the deep-AST overflow fix
// further up), Rust forbids moving owned fields out of `Expr` via
// pattern match — so this function uses `mem::replace`/`mem::take` to
// extract owned data while leaving each `Expr` in a leaf-shaped state
// for its drop. The leading-Grouped peel is also iterative so an
// `((((a))))` LHS doesn't recurse N levels.
fn expression_to_lvalue(mut expr: Expr) -> Result<LValue, String> {
    let placeholder = || Expr::Identifier(String::new());

    while let Expr::Grouped(inner) = &mut expr {
        expr = std::mem::replace(inner.as_mut(), placeholder());
    }

    match &mut expr {
        Expr::Identifier(name) => Ok(LValue::Name(std::mem::take(name))),
        // Chained selects (`a[i][m:l]`) pass straight through: on the LHS
        // the evaluator routes the array-element + inner-select case through
        // the same per-position distribution path the vector-reg LHS uses
        // (LRM 4.9). The structural validation (only `Bit` outer on an
        // array, inner forbidden on a vector, inner part-select direction
        // matches the element's packed range) happens in `lvalue_meta`, so
        // the parser stays purely syntactic here.
        Expr::Select { name, kind, inner } => {
            let name = std::mem::take(name);
            let kind_placeholder = SelectKind::Bit {
                index: Box::new(placeholder()),
            };
            let kind = std::mem::replace(kind, kind_placeholder);
            let inner = inner.take();
            Ok(LValue::Select { name, kind, inner })
        }
        Expr::Concatenation { items } => {
            let items = std::mem::take(items);
            let lvalues = items
                .into_iter()
                .map(expression_to_lvalue)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(LValue::Concat(lvalues))
        }
        Expr::Replication { .. } => {
            Err("invalid lvalue: replication is not a variable_lvalue".to_string())
        }
        _ => Err("invalid lvalue: expected name, bit/part-select, or concatenation".to_string()),
    }
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

pub(crate) fn parse_integer(input: &str) -> Result<IntegerValue, String> {
    match input.find('\'') {
        Some(apostrophe_index) => parse_based_integer(input, apostrophe_index),
        None => parse_unsized_decimal(input),
    }
}

fn parse_unsized_decimal(input: &str) -> Result<IntegerValue, String> {
    ensure_no_leading_underscore(input)?;
    let digits = strip_underscores(input);
    ensure_decimal_digits(&digits)?;

    let value = parse_biguint(&digits)?;
    let width = usize::max(signed_decimal_bit_len(&value), 32);

    Ok(IntegerValue {
        width,
        signed: true,
        base: Base::Decimal,
        bits: biguint_to_bits_with_width(&value, width),
        unsized_literal: true,
    })
}

fn parse_based_integer(input: &str, apostrophe_index: usize) -> Result<IntegerValue, String> {
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
) -> Result<IntegerValue, String> {
    let digits = strip_underscores(digits);

    let unsized_literal = width_hint.is_none();

    if digits.chars().all(is_x_digit) {
        let width = width_hint.unwrap_or(32);
        return Ok(IntegerValue {
            width,
            signed,
            base: Base::Decimal,
            bits: vec![LogicBit::X; width],
            unsized_literal,
        });
    }

    if digits.chars().all(is_z_digit) {
        let width = width_hint.unwrap_or(32);
        return Ok(IntegerValue {
            width,
            signed,
            base: Base::Decimal,
            bits: vec![LogicBit::Z; width],
            unsized_literal,
        });
    }

    ensure_decimal_digits(&digits)?;

    let value = parse_biguint(&digits)?;
    let width = width_hint.unwrap_or_else(|| usize::max(biguint_bit_len(&value), 32));

    Ok(IntegerValue {
        width,
        signed,
        base: Base::Decimal,
        bits: biguint_to_bits_with_width(&value, width),
        unsized_literal,
    })
}

fn parse_based_radix(
    width_hint: Option<usize>,
    signed: bool,
    base: Base,
    digits: &str,
) -> Result<IntegerValue, String> {
    let digits = strip_underscores(digits);
    let mut bits = Vec::with_capacity(digits.len() * base.group_size());

    for digit in digits.chars().rev() {
        push_digit_bits(digit, base, &mut bits)?;
    }

    let unsized_literal = width_hint.is_none();
    let width = width_hint.unwrap_or_else(|| usize::max(bits.len(), 32));
    let extension = extension_bit(digits.chars().next().expect("digits is not empty"));

    if bits.len() < width {
        bits.resize(width, extension);
    } else if bits.len() > width {
        bits.truncate(width);
    }

    Ok(IntegerValue {
        width,
        signed,
        base,
        bits,
        unsized_literal,
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
