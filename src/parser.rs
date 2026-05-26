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
}

// Top-level inputs. A REPL line / piped script segment between semicolons is
// one `Stmt`. Expressions still drive the evaluator, but declarations and
// blocking assignments mutate the session rather than producing a value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Stmt {
    Expr(Expr),
    // LRM A.2.1.3: `reg [signed] [range] list_of_variable_identifiers ;`,
    // where each item in the identifier list may carry an optional
    // `= constant_expression` init. `range` is `Some((msb_expr, lsb_expr))`
    // and constant-evaluated at apply time. Each name's optional init
    // expression is evaluated at decl time using the same path as a
    // blocking assignment so real→integer conversion (LRM §3.5.3) and
    // width / sign / base context propagate identically. The unpacked
    // `{ dimension }` form (2D arrays) is intentionally out of scope.
    Decl {
        signed: bool,
        range: Option<(Expr, Expr)>,
        names: Vec<(String, Option<Expr>)>,
    },
    // LRM A.6.2 `blocking_assignment`. Only the variable-name LHS form is
    // supported; bit-/part-select LHSs are out of scope for this round.
    Assign {
        name: String,
        rhs: Expr,
    },
    // LRM 17.4: `$finish` / `$stop` hoisted to the statement level so the
    // driver can exit without going through the expression evaluator.
    Task(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RealConversionKind {
    // `$rtoi(real)` — truncates toward zero, returns 32-bit signed integer.
    RealToInteger,
    // `$itor(int)` — converts integer to real per §3.5.3 (x/z → 0).
    IntegerToReal,
    // `$realtobits(real)` — bitcast to 64-bit unsigned vector (IEEE 754).
    RealToBits,
    // `$bitstoreal(int)` — reverse bitcast; takes a 64-bit value and
    // reinterprets the bit pattern as an IEEE 754 double.
    BitsToReal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MathFunctionKind {
    // Integer-result. Argument is integer-typed; a real argument
    // implicitly rounds via §3.5.3.
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
        self.parse_conditional()
    }

    // Statement-level dispatch (LRM A.2.1.3 reg decl / A.6.2 blocking
    // assignment / expression as a calculator line). Keyword recognition is
    // string-based on `Token::Identifier`; with only two keywords (`reg`,
    // `signed`) a dedicated `Token::Keyword` would be premature.
    fn parse_statement(&mut self) -> Result<Stmt, String> {
        if matches!(self.peek(), Some(Token::Identifier(name)) if name == "reg") {
            self.index += 1;
            return self.parse_decl();
        }

        // Blocking assignment: bare identifier followed by `=`. Anything
        // more elaborate (bit-select LHS etc.) falls through to the
        // expression path and surfaces a parse error there.
        if let (Some(Token::Identifier(_)), Some(Token::Assign)) =
            (self.tokens.get(self.index), self.tokens.get(self.index + 1))
        {
            let name = match self.next() {
                Some(Token::Identifier(n)) => n.clone(),
                _ => unreachable!("guarded above"),
            };
            self.index += 1; // consume `=`
            let rhs = self.parse_expression()?;
            return Ok(Stmt::Assign { name, rhs });
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

    fn parse_decl(&mut self) -> Result<Stmt, String> {
        let signed = if matches!(self.peek(), Some(Token::Identifier(n)) if n == "signed") {
            self.index += 1;
            true
        } else {
            false
        };

        let range = if matches!(self.peek(), Some(Token::LBracket)) {
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
        // LRM A.2.3 variable_type ::=
        //     variable_identifier { dimension }
        //   | variable_identifier = constant_expression
        // The `{ dimension }` (2D unpacked array) branch is out of scope, so
        // each item is just `name [= expr]`. The init expression is parsed
        // with `parse_expression`; commas naturally bind to the outer list,
        // never to the init RHS, since no expression-level operator consumes
        // a bare `,`. Inits are evaluated sequentially at apply time so
        // `reg [3:0] a = 1, b = a + 1` sees `a = 1` when binding `b`.
        let mut names: Vec<(String, Option<Expr>)> = Vec::new();
        loop {
            let name = match self.next() {
                Some(Token::Identifier(n)) => n.clone(),
                _ => return Err("expected identifier in reg declaration".to_string()),
            };
            if matches!(name.as_str(), "reg" | "signed") {
                return Err(format!("`{name}` cannot be used as a reg name"));
            }
            if names.iter().any(|(existing, _)| existing == &name) {
                return Err(format!("duplicate name in reg declaration: {name}"));
            }
            let init = if matches!(self.peek(), Some(Token::Assign)) {
                self.index += 1;
                Some(self.parse_expression()?)
            } else {
                None
            };
            names.push((name, init));
            if matches!(self.peek(), Some(Token::Comma)) {
                self.index += 1;
                continue;
            }
            break;
        }

        Ok(Stmt::Decl {
            signed,
            range,
            names,
        })
    }

    // LRM Table 5-4: `?:` sits below `||`, above the lowest level.
    // Right-associative — the middle parses as a full expression so a
    // nested `?:` in the middle is anchored by the upcoming `:`, and the
    // else recurses into parse_conditional so `a ? b : c ? d : e` becomes
    // `a ? b : (c ? d : e)`.
    fn parse_conditional(&mut self) -> Result<Expr, String> {
        let cond = self.parse_logical_or()?;
        if !matches!(self.peek(), Some(Token::Question)) {
            return Ok(cond);
        }
        self.index += 1;
        let then_expr = self.parse_expression()?;
        match self.next() {
            Some(Token::Colon) => {}
            _ => return Err("expected `:` in conditional expression".to_string()),
        }
        let else_expr = self.parse_conditional()?;
        Ok(Expr::Conditional {
            cond: Box::new(cond),
            then_expr: Box::new(then_expr),
            else_expr: Box::new(else_expr),
        })
    }

    fn parse_logical_or(&mut self) -> Result<Expr, String> {
        let mut expression = self.parse_logical_and()?;

        while matches!(self.peek(), Some(Token::LogicalOr)) {
            self.index += 1;
            let rhs = self.parse_logical_and()?;
            expression = Expr::Binary {
                op: BinaryOp::LogicalOr,
                lhs: Box::new(expression),
                rhs: Box::new(rhs),
            };
        }

        Ok(expression)
    }

    fn parse_logical_and(&mut self) -> Result<Expr, String> {
        let mut expression = self.parse_bitwise_or()?;

        while matches!(self.peek(), Some(Token::LogicalAnd)) {
            self.index += 1;
            let rhs = self.parse_bitwise_or()?;
            expression = Expr::Binary {
                op: BinaryOp::LogicalAnd,
                lhs: Box::new(expression),
                rhs: Box::new(rhs),
            };
        }

        Ok(expression)
    }

    // LRM Table 5-4: bitwise binary band sits between `&&` and `==`, with
    // internal order `&` (tightest) > `^` `~^` `^~` > `|` (loosest).
    fn parse_bitwise_or(&mut self) -> Result<Expr, String> {
        let mut expression = self.parse_bitwise_xor()?;

        while matches!(self.peek(), Some(Token::BitwiseOr)) {
            self.index += 1;
            let rhs = self.parse_bitwise_xor()?;
            expression = Expr::Binary {
                op: BinaryOp::BitwiseOr,
                lhs: Box::new(expression),
                rhs: Box::new(rhs),
            };
        }

        Ok(expression)
    }

    fn parse_bitwise_xor(&mut self) -> Result<Expr, String> {
        let mut expression = self.parse_bitwise_and()?;

        while matches!(self.peek(), Some(Token::BitwiseXor | Token::BitwiseXnor)) {
            let op = match self.peek() {
                Some(Token::BitwiseXor) => BinaryOp::BitwiseXor,
                Some(Token::BitwiseXnor) => BinaryOp::BitwiseXnor,
                _ => unreachable!("guarded by while condition"),
            };
            self.index += 1;
            let rhs = self.parse_bitwise_and()?;
            expression = Expr::Binary {
                op,
                lhs: Box::new(expression),
                rhs: Box::new(rhs),
            };
        }

        Ok(expression)
    }

    fn parse_bitwise_and(&mut self) -> Result<Expr, String> {
        let mut expression = self.parse_equality()?;

        while matches!(self.peek(), Some(Token::BitwiseAnd)) {
            self.index += 1;
            let rhs = self.parse_equality()?;
            expression = Expr::Binary {
                op: BinaryOp::BitwiseAnd,
                lhs: Box::new(expression),
                rhs: Box::new(rhs),
            };
        }

        Ok(expression)
    }

    fn parse_equality(&mut self) -> Result<Expr, String> {
        let mut expression = self.parse_relational()?;

        loop {
            let op = match self.peek() {
                Some(Token::EqualEqual) => BinaryOp::Equal,
                Some(Token::NotEqual) => BinaryOp::NotEqual,
                Some(Token::CaseEqual) => BinaryOp::CaseEqual,
                Some(Token::CaseNotEqual) => BinaryOp::CaseNotEqual,
                _ => break,
            };
            self.index += 1;

            let rhs = self.parse_relational()?;
            expression = Expr::Binary {
                op,
                lhs: Box::new(expression),
                rhs: Box::new(rhs),
            };
        }

        Ok(expression)
    }

    fn parse_relational(&mut self) -> Result<Expr, String> {
        let mut expression = self.parse_shift()?;

        loop {
            let op = match self.peek() {
                Some(Token::Less) => BinaryOp::LessThan,
                Some(Token::Greater) => BinaryOp::GreaterThan,
                Some(Token::LessEqual) => BinaryOp::LessThanOrEqual,
                Some(Token::GreaterEqual) => BinaryOp::GreaterThanOrEqual,
                _ => break,
            };
            self.index += 1;

            let rhs = self.parse_shift()?;
            expression = Expr::Binary {
                op,
                lhs: Box::new(expression),
                rhs: Box::new(rhs),
            };
        }

        Ok(expression)
    }

    // LRM Table 5-4: shifts sit between additive and relational. Left
    // associative; `<<<`/`>>>` share this level with `<<`/`>>` (LRM 5.1.12).
    fn parse_shift(&mut self) -> Result<Expr, String> {
        let mut expression = self.parse_additive()?;

        loop {
            let op = match self.peek() {
                Some(Token::LogicalShiftLeft) => BinaryOp::LogicalShiftLeft,
                Some(Token::LogicalShiftRight) => BinaryOp::LogicalShiftRight,
                Some(Token::ArithmeticShiftLeft) => BinaryOp::ArithmeticShiftLeft,
                Some(Token::ArithmeticShiftRight) => BinaryOp::ArithmeticShiftRight,
                _ => break,
            };
            self.index += 1;

            let rhs = self.parse_additive()?;
            expression = Expr::Binary {
                op,
                lhs: Box::new(expression),
                rhs: Box::new(rhs),
            };
        }

        Ok(expression)
    }

    fn parse_additive(&mut self) -> Result<Expr, String> {
        let mut expression = self.parse_multiplicative()?;

        loop {
            let op = match self.peek() {
                Some(Token::Plus) => BinaryOp::Add,
                Some(Token::Minus) => BinaryOp::Subtract,
                _ => break,
            };
            self.index += 1;

            let rhs = self.parse_multiplicative()?;
            expression = Expr::Binary {
                op,
                lhs: Box::new(expression),
                rhs: Box::new(rhs),
            };
        }

        Ok(expression)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, String> {
        let mut expression = self.parse_power()?;

        loop {
            let op = match self.peek() {
                Some(Token::Star) => BinaryOp::Multiply,
                Some(Token::Slash) => BinaryOp::Divide,
                Some(Token::Percent) => BinaryOp::Modulus,
                _ => break,
            };
            self.index += 1;

            let rhs = self.parse_power()?;
            expression = Expr::Binary {
                op,
                lhs: Box::new(expression),
                rhs: Box::new(rhs),
            };
        }

        Ok(expression)
    }

    // Unary binds tighter than `**` (LRM 1364-2005 Table 22), so both sides of
    // `**` go through `parse_unary`. The while loop accumulates left-to-right.
    fn parse_power(&mut self) -> Result<Expr, String> {
        let mut expression = self.parse_unary()?;

        while matches!(self.peek(), Some(Token::Power)) {
            self.index += 1;
            let rhs = self.parse_unary()?;
            expression = Expr::Binary {
                op: BinaryOp::Power,
                lhs: Box::new(expression),
                rhs: Box::new(rhs),
            };
        }

        Ok(expression)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        // Position-based disambiguation: `&`/`|`/`^`/`~^` (and the alt
        // spelling `^~`) are binary OR unary depending on parse position.
        // `parse_unary` claims them at unary position; the binary
        // `parse_bitwise_{and,xor,or}` levels only see them after a primary,
        // so dispatch is unambiguous without a token rewrite. `~&` and `~|`
        // are unary-only — no binary parse level consumes them, so a
        // free-standing `a ~& b` cleanly fails as "unexpected token".
        let op = match self.peek() {
            Some(Token::Plus) => Some(UnaryOp::Plus),
            Some(Token::Minus) => Some(UnaryOp::Minus),
            Some(Token::Bang) => Some(UnaryOp::LogicalNot),
            Some(Token::Tilde) => Some(UnaryOp::BitwiseNot),
            Some(Token::BitwiseAnd) => Some(UnaryOp::ReductionAnd),
            Some(Token::BitwiseOr) => Some(UnaryOp::ReductionOr),
            Some(Token::BitwiseXor) => Some(UnaryOp::ReductionXor),
            Some(Token::BitwiseXnor) => Some(UnaryOp::ReductionXnor),
            Some(Token::BitwiseNand) => Some(UnaryOp::ReductionNand),
            Some(Token::BitwiseNor) => Some(UnaryOp::ReductionNor),
            _ => None,
        };

        if let Some(op) = op {
            self.index += 1;
            let expr = self.parse_unary()?;
            Ok(Expr::Unary {
                op,
                expr: Box::new(expr),
            })
        } else {
            self.parse_primary()
        }
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
            Some(Token::Identifier(name)) => Ok(Expr::Identifier(name.clone())),
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
