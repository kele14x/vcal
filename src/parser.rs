use num_bigint::BigUint;
use std::borrow::Cow;

use crate::lexer::{Token, tokenize};
use crate::value::{
    Base, IntegerValue, LogicBit, biguint_bit_len, biguint_to_bits_with_width,
    signed_decimal_bit_len,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Expr {
    Literal(IntegerValue),
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

struct Parser {
    tokens: Vec<Token>,
    index: usize,
}

pub(crate) fn parse_expression(input: &str) -> Result<Expr, String> {
    let tokens = tokenize(input)?;
    if tokens.is_empty() {
        return Err("empty expression".to_string());
    }

    let mut parser = Parser { tokens, index: 0 };
    let expression = parser.parse_expression()?;

    if parser.peek().is_some() {
        return Err("unexpected token after end of expression".to_string());
    }

    Ok(expression)
}

impl Parser {
    fn parse_expression(&mut self) -> Result<Expr, String> {
        self.parse_conditional()
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
        match self.next() {
            Some(Token::IntegerLiteral(text)) => parse_integer(&text).map(Expr::Literal),
            Some(Token::SystemIdentifier(name)) => self.parse_system_function_call(&name),
            Some(Token::LParen) => {
                let expr = self.parse_expression()?;
                match self.next() {
                    Some(Token::RParen) => Ok(Expr::Grouped(Box::new(expr))),
                    _ => Err("missing closing parenthesis".to_string()),
                }
            }
            Some(Token::LBrace) => self.parse_brace_primary(),
            Some(Token::RParen) => Err("unexpected closing parenthesis".to_string()),
            Some(Token::Plus) | Some(Token::Minus) | Some(Token::Star) | Some(Token::Slash)
            | Some(Token::Percent) | Some(Token::Power) | Some(Token::Less)
            | Some(Token::Greater) | Some(Token::LessEqual) | Some(Token::GreaterEqual)
            | Some(Token::EqualEqual) | Some(Token::NotEqual) | Some(Token::CaseEqual)
            | Some(Token::CaseNotEqual) | Some(Token::Bang) | Some(Token::LogicalAnd)
            | Some(Token::LogicalOr) | Some(Token::Tilde) | Some(Token::BitwiseAnd)
            | Some(Token::BitwiseOr) | Some(Token::BitwiseXor) | Some(Token::BitwiseXnor)
            | Some(Token::BitwiseNand) | Some(Token::BitwiseNor)
            | Some(Token::LogicalShiftLeft) | Some(Token::LogicalShiftRight)
            | Some(Token::ArithmeticShiftLeft) | Some(Token::ArithmeticShiftRight)
            | Some(Token::Question) | Some(Token::Colon)
            | Some(Token::RBrace) | Some(Token::Comma) => {
                Err("expected expression operand".to_string())
            }
            None => Err("unexpected end of expression".to_string()),
        }
    }

    // LRM 5.5: `$signed(expr)` / `$unsigned(expr)` — exactly one argument,
    // parentheses required. Other system identifiers aren't legal in
    // expression position yet, so we reject them with a clear message instead
    // of leaving the generic "expected expression operand" path to fire.
    fn parse_system_function_call(&mut self, name: &str) -> Result<Expr, String> {
        let signed = match name {
            "$signed" => true,
            "$unsigned" => false,
            _ => return Err(format!("unsupported system function: {name}")),
        };

        match self.next() {
            Some(Token::LParen) => {}
            _ => return Err(format!("expected `(` after {name}")),
        }

        let arg = self.parse_expression()?;

        match self.next() {
            Some(Token::RParen) => {}
            _ => return Err(format!("expected `)` after {name} argument")),
        }

        Ok(Expr::SignCast {
            signed,
            arg: Box::new(arg),
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

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.index).cloned();
        if token.is_some() {
            self.index += 1;
        }
        token
    }
}

pub(crate) fn parse_integer(input: &str) -> Result<IntegerValue, String> {
    match input.find('\'') {
        Some(apostrophe_index) => parse_based_integer(input, apostrophe_index),
        None => parse_unsized_decimal(input),
    }
}

fn parse_unsized_decimal(input: &str) -> Result<IntegerValue, String> {
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
        bits.extend(digit_to_bits(digit, base)?);
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

fn digit_to_bits(digit: char, base: Base) -> Result<Vec<LogicBit>, String> {
    let digit = digit.to_ascii_lowercase();

    match base {
        Base::Binary => match digit {
            '0' => Ok(vec![LogicBit::Zero]),
            '1' => Ok(vec![LogicBit::One]),
            'x' => Ok(vec![LogicBit::X]),
            'z' | '?' => Ok(vec![LogicBit::Z]),
            _ => Err(format!("invalid binary digit: {digit}")),
        },
        Base::Octal => match digit {
            'x' => Ok(vec![LogicBit::X; 3]),
            'z' | '?' => Ok(vec![LogicBit::Z; 3]),
            '0'..='7' => Ok(integer_bits((digit as u8) - b'0', 3)),
            _ => Err(format!("invalid octal digit: {digit}")),
        },
        Base::Hex => match digit {
            'x' => Ok(vec![LogicBit::X; 4]),
            'z' | '?' => Ok(vec![LogicBit::Z; 4]),
            '0'..='9' => Ok(integer_bits((digit as u8) - b'0', 4)),
            'a'..='f' => Ok(integer_bits((digit as u8) - b'a' + 10, 4)),
            _ => Err(format!("invalid hex digit: {digit}")),
        },
        Base::Decimal => Err("decimal digits are parsed separately".to_string()),
    }
}

fn integer_bits(value: u8, width: usize) -> Vec<LogicBit> {
    (0..width)
        .map(|shift| {
            if value & (1 << shift) == 0 {
                LogicBit::Zero
            } else {
                LogicBit::One
            }
        })
        .collect()
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
