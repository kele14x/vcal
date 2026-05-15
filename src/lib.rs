use num_bigint::{BigInt, BigUint, Sign};
use num_traits::{One, Zero};
use std::borrow::Cow;
use std::io::{self, BufRead, Write};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicBit {
    Zero,
    One,
    X,
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    Binary,
    Octal,
    Decimal,
    Hex,
}

impl Base {
    fn char(self) -> char {
        match self {
            Self::Binary => 'b',
            Self::Octal => 'o',
            Self::Decimal => 'd',
            Self::Hex => 'h',
        }
    }

    fn group_size(self) -> usize {
        match self {
            Self::Binary => 1,
            Self::Octal => 3,
            Self::Decimal => 1,
            Self::Hex => 4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntegerValue {
    width: usize,
    signed: bool,
    base: Base,
    bits: Vec<LogicBit>,
}

impl IntegerValue {
    pub fn canonical(&self) -> String {
        if self.base == Base::Decimal
            && self.signed
            && let Some((negative, digits)) = self.render_signed_decimal_digits()
        {
            let prefix = if negative { "-" } else { "" };
            return format!("{prefix}{}'sd{digits}", self.width);
        }

        let signed = if self.signed { "s" } else { "" };
        format!(
            "{}'{}{}{}",
            self.width,
            signed,
            self.base.char(),
            self.render_digits()
        )
    }

    fn render_digits(&self) -> String {
        match self.base {
            Base::Decimal => self.render_decimal_digits(),
            Base::Binary | Base::Octal | Base::Hex => self.render_grouped_digits(),
        }
    }

    fn render_decimal_digits(&self) -> String {
        if self.bits.iter().all(|bit| *bit == LogicBit::X) {
            return "x".to_string();
        }

        if self.bits.iter().all(|bit| *bit == LogicBit::Z) {
            return "z".to_string();
        }

        if self
            .bits
            .iter()
            .any(|bit| matches!(bit, LogicBit::X | LogicBit::Z))
        {
            return if self.bits.contains(&LogicBit::X) {
                "x".to_string()
            } else {
                "z".to_string()
            };
        }

        bits_to_biguint(&self.bits).to_str_radix(10)
    }

    fn render_signed_decimal_digits(&self) -> Option<(bool, String)> {
        if self.bits.iter().all(|bit| *bit == LogicBit::X) {
            return Some((false, "x".to_string()));
        }

        if self.bits.iter().all(|bit| *bit == LogicBit::Z) {
            return Some((false, "z".to_string()));
        }

        if self.has_unknown_bits() {
            return Some((
                false,
                if self.bits.contains(&LogicBit::X) {
                    "x".to_string()
                } else {
                    "z".to_string()
                },
            ));
        }

        let value = bits_to_signed_bigint(&self.bits);
        let negative = value.sign() == Sign::Minus;
        let digits = if negative {
            (-value).to_str_radix(10)
        } else {
            value.to_str_radix(10)
        };

        Some((negative, digits))
    }

    fn render_grouped_digits(&self) -> String {
        let group_size = self.base.group_size();
        let digit_count = self.width.div_ceil(group_size);
        let mut output = String::with_capacity(digit_count);

        for digit_index in (0..digit_count).rev() {
            let mut group_bits = Vec::with_capacity(group_size);

            for offset in 0..group_size {
                let bit_index = digit_index * group_size + offset;
                group_bits.push(self.bits.get(bit_index).copied().unwrap_or(LogicBit::Zero));
            }

            output.push(render_group_digit(&group_bits, self.base));
        }

        output
    }

    fn has_unknown_bits(&self) -> bool {
        self.bits
            .iter()
            .any(|bit| matches!(bit, LogicBit::X | LogicBit::Z))
    }

    fn resized_to_context(&self, width: usize, context_signed: bool) -> Self {
        if width == self.width {
            return self.clone();
        }

        let mut bits = self.bits.clone();

        if bits.len() < width {
            bits.resize(width, self.context_extension_bit(context_signed));
        } else {
            bits.truncate(width);
        }

        Self {
            width,
            signed: context_signed,
            base: self.base,
            bits,
        }
    }

    fn context_extension_bit(&self, context_signed: bool) -> LogicBit {
        match self.bits.last().copied().unwrap_or(LogicBit::Zero) {
            LogicBit::X => LogicBit::X,
            LogicBit::Z => LogicBit::Z,
            LogicBit::One if context_signed => LogicBit::One,
            _ => LogicBit::Zero,
        }
    }

    fn as_bigint(&self, signed: bool) -> BigInt {
        if signed {
            bits_to_signed_bigint(&self.bits)
        } else {
            BigInt::from(bits_to_biguint(&self.bits))
        }
    }

    fn from_bigint(value: BigInt, width: usize, signed: bool, base: Base) -> Self {
        Self {
            width,
            signed,
            base,
            bits: bigint_to_bits_with_width(&value, width),
        }
    }

    fn all_x(width: usize, signed: bool, base: Base) -> Self {
        Self {
            width,
            signed,
            base,
            bits: vec![LogicBit::X; width],
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Evaluation {
    pub output: String,
    pub should_exit: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Expr {
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnaryOp {
    Plus,
    Minus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BinaryOp {
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    IntegerLiteral(String),
    LParen,
    RParen,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Power,
    Less,
    Greater,
    LessEqual,
    GreaterEqual,
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ExprMeta {
    width: usize,
    signed: bool,
    // Inferred display base — leftmost operand wins for binary ops.
    // Used when constructing arithmetic results; ignored when ExprMeta is
    // passed downward as context (literals keep their own base).
    base: Base,
}

enum ParsedLine {
    Value(IntegerValue),
    Exit,
}

pub fn evaluate_input(input: &str) -> Result<Evaluation, String> {
    match parse_line(input)? {
        ParsedLine::Value(value) => Ok(Evaluation {
            output: value.canonical(),
            should_exit: false,
        }),
        ParsedLine::Exit => Ok(Evaluation {
            output: String::new(),
            should_exit: true,
        }),
    }
}

pub fn run_repl<R: BufRead, W: Write>(reader: &mut R, writer: &mut W) -> io::Result<()> {
    let mut index = 0usize;
    let mut line = String::new();

    loop {
        write!(writer, "In[{index}]: ")?;
        writer.flush()?;

        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }

        match evaluate_input(&line) {
            Ok(result) => {
                writeln!(writer, "Out[{index}]: {}", result.output)?;
                if result.should_exit {
                    break;
                }
            }
            Err(message) => {
                writeln!(writer, "Out[{index}]: ")?;
                writeln!(writer, "Error: {message}")?;
            }
        }

        index += 1;
    }

    Ok(())
}

pub fn run_interactive() -> io::Result<()> {
    use rustyline::DefaultEditor;
    use rustyline::error::ReadlineError;

    let mut editor = DefaultEditor::new().map_err(io::Error::other)?;
    let mut index = 0usize;

    loop {
        let line = match editor.readline(&format!("In[{index}]: ")) {
            Ok(line) => line,
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(err) => return Err(io::Error::other(err)),
        };

        if !line.trim().is_empty() {
            let _ = editor.add_history_entry(line.as_str());
        }

        match evaluate_input(&line) {
            Ok(result) => {
                println!("Out[{index}]: {}", result.output);
                if result.should_exit {
                    break;
                }
            }
            Err(message) => {
                println!("Out[{index}]: ");
                println!("Error: {message}");
            }
        }

        index += 1;
    }

    Ok(())
}

fn parse_line(input: &str) -> Result<ParsedLine, String> {
    let input = strip_statement_terminators(input);

    if input.is_empty() {
        return Err("empty input".to_string());
    }

    if let Some(command) = parse_system_task(input)? {
        return Ok(command);
    }

    let expression = parse_expression(input)?;
    evaluate_expr(&expression).map(ParsedLine::Value)
}

fn parse_expression(input: &str) -> Result<Expr, String> {
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

fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.char_indices().peekable();

    while let Some((_, ch)) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }

        match ch {
            '(' => tokens.push(Token::LParen),
            ')' => tokens.push(Token::RParen),
            '+' => tokens.push(Token::Plus),
            '-' => tokens.push(Token::Minus),
            '/' => tokens.push(Token::Slash),
            '%' => tokens.push(Token::Percent),
            '*' => {
                if matches!(chars.peek(), Some((_, '*'))) {
                    chars.next();
                    tokens.push(Token::Power);
                } else {
                    tokens.push(Token::Star);
                }
            }
            '<' => {
                if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    tokens.push(Token::LessEqual);
                } else {
                    tokens.push(Token::Less);
                }
            }
            '>' => {
                if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    tokens.push(Token::GreaterEqual);
                } else {
                    tokens.push(Token::Greater);
                }
            }
            '\'' => {
                tokens.push(Token::IntegerLiteral(read_based_literal_after_apostrophe(
                    &mut chars,
                )?));
            }
            _ => {
                tokens.push(Token::IntegerLiteral(read_integer_literal(ch, &mut chars)?));
            }
        }
    }

    Ok(tokens)
}

fn read_integer_literal<I>(
    first_ch: char,
    chars: &mut std::iter::Peekable<I>,
) -> Result<String, String>
where
    I: Iterator<Item = (usize, char)> + Clone,
{
    let mut literal = String::new();
    literal.push(first_ch);

    while let Some((_, next_ch)) = chars.peek().copied() {
        if next_ch.is_whitespace() || is_expression_delimiter(next_ch) || next_ch == '\'' {
            break;
        }

        chars.next();
        literal.push(next_ch);
    }

    let mut cursor = chars.clone();
    skip_whitespace(&mut cursor);

    if matches!(cursor.peek(), Some((_, '\''))) {
        *chars = cursor;
        chars.next();
        literal.push('\'');

        let (_, base_ch) = chars
            .next()
            .ok_or_else(|| "missing base after apostrophe".to_string())?;
        if base_ch.is_whitespace() {
            return Err("missing base after apostrophe".to_string());
        }
        literal.push(base_ch);

        if matches!(base_ch, 's' | 'S') {
            let (_, signed_base_ch) = chars
                .next()
                .ok_or_else(|| "missing base after signed marker".to_string())?;
            if signed_base_ch.is_whitespace() {
                return Err("missing base after signed marker".to_string());
            }
            literal.push(signed_base_ch);
        }

        let mut saw_digit = false;
        while let Some((_, next_ch)) = chars.peek().copied() {
            if is_expression_delimiter(next_ch) {
                break;
            }

            chars.next();
            if next_ch.is_whitespace() {
                continue;
            }

            literal.push(next_ch);
            saw_digit = true;
        }

        if !saw_digit {
            return Err("missing digits in integer literal".to_string());
        }
    }

    Ok(literal)
}

fn read_based_literal_after_apostrophe<I>(
    chars: &mut std::iter::Peekable<I>,
) -> Result<String, String>
where
    I: Iterator<Item = (usize, char)> + Clone,
{
    let mut literal = String::from("'");

    let (_, base_ch) = chars
        .next()
        .ok_or_else(|| "missing base after apostrophe".to_string())?;
    if base_ch.is_whitespace() {
        return Err("missing base after apostrophe".to_string());
    }
    literal.push(base_ch);

    if matches!(base_ch, 's' | 'S') {
        let (_, signed_base_ch) = chars
            .next()
            .ok_or_else(|| "missing base after signed marker".to_string())?;
        if signed_base_ch.is_whitespace() {
            return Err("missing base after signed marker".to_string());
        }
        literal.push(signed_base_ch);
    }

    let mut saw_digit = false;
    while let Some((_, next_ch)) = chars.peek().copied() {
        if is_expression_delimiter(next_ch) {
            break;
        }

        chars.next();
        if next_ch.is_whitespace() {
            continue;
        }

        literal.push(next_ch);
        saw_digit = true;
    }

    if !saw_digit {
        return Err("missing digits in integer literal".to_string());
    }

    Ok(literal)
}

fn skip_whitespace<I>(chars: &mut std::iter::Peekable<I>)
where
    I: Iterator<Item = (usize, char)>,
{
    while matches!(chars.peek(), Some((_, ch)) if ch.is_whitespace()) {
        chars.next();
    }
}

fn is_expression_delimiter(ch: char) -> bool {
    matches!(ch, '(' | ')' | '+' | '-' | '*' | '/' | '%' | '<' | '>')
}

impl Parser {
    fn parse_expression(&mut self) -> Result<Expr, String> {
        self.parse_relational()
    }

    fn parse_relational(&mut self) -> Result<Expr, String> {
        let mut expression = self.parse_additive()?;

        loop {
            let op = match self.peek() {
                Some(Token::Less) => BinaryOp::LessThan,
                Some(Token::Greater) => BinaryOp::GreaterThan,
                Some(Token::LessEqual) => BinaryOp::LessThanOrEqual,
                Some(Token::GreaterEqual) => BinaryOp::GreaterThanOrEqual,
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
        let op = match self.peek() {
            Some(Token::Plus) => Some(UnaryOp::Plus),
            Some(Token::Minus) => Some(UnaryOp::Minus),
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
            Some(Token::LParen) => {
                let expr = self.parse_expression()?;
                match self.next() {
                    Some(Token::RParen) => Ok(Expr::Grouped(Box::new(expr))),
                    _ => Err("missing closing parenthesis".to_string()),
                }
            }
            Some(Token::RParen) => Err("unexpected closing parenthesis".to_string()),
            Some(Token::Plus) | Some(Token::Minus) | Some(Token::Star) | Some(Token::Slash)
            | Some(Token::Percent) | Some(Token::Power) | Some(Token::Less)
            | Some(Token::Greater) | Some(Token::LessEqual) | Some(Token::GreaterEqual) => {
                Err("expected expression operand".to_string())
            }
            None => Err("unexpected end of expression".to_string()),
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

fn evaluate_expr(expr: &Expr) -> Result<IntegerValue, String> {
    evaluate_expr_in_context(expr, None)
}

fn evaluate_expr_in_context(
    expr: &Expr,
    context: Option<ExprMeta>,
) -> Result<IntegerValue, String> {
    match expr {
        Expr::Literal(value) => Ok(match context {
            Some(context) => value.resized_to_context(context.width, context.signed),
            None => value.clone(),
        }),
        Expr::Grouped(expr) => evaluate_expr_in_context(expr, context),
        Expr::Unary { op, expr } => evaluate_unary_expr(*op, expr, context),
        Expr::Binary { op, lhs, rhs } => evaluate_binary_expr(*op, lhs, rhs, context),
    }
}

fn infer_expr_meta(expr: &Expr) -> Result<ExprMeta, String> {
    match expr {
        Expr::Literal(value) => Ok(ExprMeta {
            width: value.width,
            signed: value.signed,
            base: value.base,
        }),
        Expr::Grouped(expr) => infer_expr_meta(expr),
        Expr::Unary { op, expr } => match op {
            UnaryOp::Plus | UnaryOp::Minus => infer_expr_meta(expr),
        },
        Expr::Binary { op, lhs, rhs } => {
            let lhs_meta = infer_expr_meta(lhs)?;
            let rhs_meta = infer_expr_meta(rhs)?;
            Ok(combine_binary_meta(*op, lhs_meta, rhs_meta))
        }
    }
}

fn combine_binary_meta(op: BinaryOp, lhs_meta: ExprMeta, rhs_meta: ExprMeta) -> ExprMeta {
    match op {
        BinaryOp::Add
        | BinaryOp::Subtract
        | BinaryOp::Multiply
        | BinaryOp::Divide
        | BinaryOp::Modulus => ExprMeta {
            width: usize::max(lhs_meta.width, rhs_meta.width),
            signed: lhs_meta.signed && rhs_meta.signed,
            base: lhs_meta.base,
        },
        BinaryOp::Power => ExprMeta {
            width: lhs_meta.width,
            signed: lhs_meta.signed,
            base: lhs_meta.base,
        },
        BinaryOp::LessThan
        | BinaryOp::GreaterThan
        | BinaryOp::LessThanOrEqual
        | BinaryOp::GreaterThanOrEqual => ExprMeta {
            width: 1,
            signed: false,
            base: Base::Binary,
        },
    }
}

fn evaluate_unary_expr(
    op: UnaryOp,
    expr: &Expr,
    context: Option<ExprMeta>,
) -> Result<IntegerValue, String> {
    let meta = infer_expr_meta(expr)?;
    let effective_meta = ExprMeta {
        width: context.map_or(meta.width, |ctx| usize::max(ctx.width, meta.width)),
        signed: meta.signed,
        base: meta.base,
    };
    let operand = evaluate_expr_in_context(expr, Some(effective_meta))?;

    if op == UnaryOp::Plus {
        return Ok(operand);
    }

    if operand.has_unknown_bits() {
        return Ok(IntegerValue::all_x(
            effective_meta.width,
            meta.signed,
            meta.base,
        ));
    }

    let value = operand.as_bigint(meta.signed);
    let result = match op {
        UnaryOp::Minus => -value,
        UnaryOp::Plus => unreachable!("handled before arithmetic evaluation"),
    };

    Ok(IntegerValue::from_bigint(
        result,
        effective_meta.width,
        meta.signed,
        meta.base,
    ))
}

fn evaluate_binary_expr(
    op: BinaryOp,
    lhs: &Expr,
    rhs: &Expr,
    context: Option<ExprMeta>,
) -> Result<IntegerValue, String> {
    let lhs_meta = infer_expr_meta(lhs)?;
    let rhs_meta = infer_expr_meta(rhs)?;

    if matches!(
        op,
        BinaryOp::LessThan
            | BinaryOp::GreaterThan
            | BinaryOp::LessThanOrEqual
            | BinaryOp::GreaterThanOrEqual
    ) {
        return evaluate_relational_expr(op, lhs, rhs, lhs_meta, rhs_meta, context);
    }

    let meta = combine_binary_meta(op, lhs_meta, rhs_meta);
    let effective_meta = ExprMeta {
        width: context.map_or(meta.width, |ctx| usize::max(ctx.width, meta.width)),
        signed: meta.signed,
        base: meta.base,
    };

    match op {
        BinaryOp::Add
        | BinaryOp::Subtract
        | BinaryOp::Multiply
        | BinaryOp::Divide
        | BinaryOp::Modulus => {
            let lhs_value = evaluate_expr_in_context(lhs, Some(effective_meta))?;
            let rhs_value = evaluate_expr_in_context(rhs, Some(effective_meta))?;

            if lhs_value.has_unknown_bits() || rhs_value.has_unknown_bits() {
                return Ok(IntegerValue::all_x(
                    effective_meta.width,
                    meta.signed,
                    meta.base,
                ));
            }

            let lhs_int = lhs_value.as_bigint(meta.signed);
            let rhs_int = rhs_value.as_bigint(meta.signed);
            let result = match op {
                BinaryOp::Add => lhs_int + rhs_int,
                BinaryOp::Subtract => lhs_int - rhs_int,
                BinaryOp::Multiply => lhs_int * rhs_int,
                BinaryOp::Divide => {
                    if rhs_int.is_zero() {
                        return Ok(IntegerValue::all_x(
                            effective_meta.width,
                            meta.signed,
                            meta.base,
                        ));
                    }
                    lhs_int / rhs_int
                }
                BinaryOp::Modulus => {
                    if rhs_int.is_zero() {
                        return Ok(IntegerValue::all_x(
                            effective_meta.width,
                            meta.signed,
                            meta.base,
                        ));
                    }
                    lhs_int % rhs_int
                }
                _ => unreachable!("handled by outer match"),
            };

            Ok(IntegerValue::from_bigint(
                result,
                effective_meta.width,
                meta.signed,
                meta.base,
            ))
        }
        BinaryOp::Power => {
            let lhs_context = ExprMeta {
                width: effective_meta.width,
                signed: lhs_meta.signed,
                base: lhs_meta.base,
            };
            let lhs_value = evaluate_expr_in_context(lhs, Some(lhs_context))?;
            let rhs_value = evaluate_expr_in_context(rhs, Some(rhs_meta))?;

            if lhs_value.has_unknown_bits() || rhs_value.has_unknown_bits() {
                return Ok(IntegerValue::all_x(
                    effective_meta.width,
                    lhs_meta.signed,
                    lhs_meta.base,
                ));
            }

            let base_value = lhs_value.as_bigint(lhs_meta.signed);
            let exponent_value = evaluate_expr_as_math_bigint(rhs)?;
            let result = match evaluate_power(base_value, exponent_value) {
                Ok(result) => result,
                Err(_) => {
                    return Ok(IntegerValue::all_x(
                        effective_meta.width,
                        lhs_meta.signed,
                        lhs_meta.base,
                    ));
                }
            };

            Ok(IntegerValue::from_bigint(
                result,
                effective_meta.width,
                lhs_meta.signed,
                lhs_meta.base,
            ))
        }
        BinaryOp::LessThan
        | BinaryOp::GreaterThan
        | BinaryOp::LessThanOrEqual
        | BinaryOp::GreaterThanOrEqual => {
            unreachable!("relational ops dispatched to evaluate_relational_expr")
        }
    }
}

fn evaluate_relational_expr(
    op: BinaryOp,
    lhs: &Expr,
    rhs: &Expr,
    lhs_meta: ExprMeta,
    rhs_meta: ExprMeta,
    context: Option<ExprMeta>,
) -> Result<IntegerValue, String> {
    let operand_width = usize::max(lhs_meta.width, rhs_meta.width);
    let comparison_signed = lhs_meta.signed && rhs_meta.signed;

    // Each operand widens using its OWN signedness — sign-extension preserves
    // the signed value at the wider width before comparison-time
    // reinterpretation as unsigned (LRM 5.5.1).
    let lhs_context = ExprMeta {
        width: operand_width,
        signed: lhs_meta.signed,
        base: lhs_meta.base,
    };
    let rhs_context = ExprMeta {
        width: operand_width,
        signed: rhs_meta.signed,
        base: rhs_meta.base,
    };

    let lhs_value = evaluate_expr_in_context(lhs, Some(lhs_context))?;
    let rhs_value = evaluate_expr_in_context(rhs, Some(rhs_context))?;

    if lhs_value.has_unknown_bits() || rhs_value.has_unknown_bits() {
        return Ok(widen_relational_result(
            IntegerValue::all_x(1, false, Base::Binary),
            context,
        ));
    }

    let lhs_int = lhs_value.as_bigint(comparison_signed);
    let rhs_int = rhs_value.as_bigint(comparison_signed);

    let comparison_result = match op {
        BinaryOp::LessThan => lhs_int < rhs_int,
        BinaryOp::GreaterThan => lhs_int > rhs_int,
        BinaryOp::LessThanOrEqual => lhs_int <= rhs_int,
        BinaryOp::GreaterThanOrEqual => lhs_int >= rhs_int,
        _ => unreachable!("non-relational op in evaluate_relational_expr"),
    };

    let bit = if comparison_result {
        LogicBit::One
    } else {
        LogicBit::Zero
    };
    let result = IntegerValue {
        width: 1,
        signed: false,
        base: Base::Binary,
        bits: vec![bit],
    };

    Ok(widen_relational_result(result, context))
}

fn widen_relational_result(result: IntegerValue, context: Option<ExprMeta>) -> IntegerValue {
    match context {
        Some(ctx) if ctx.width > 1 => result.resized_to_context(ctx.width, false),
        _ => result,
    }
}

fn evaluate_expr_as_math_bigint(expr: &Expr) -> Result<BigInt, String> {
    match expr {
        Expr::Literal(value) => {
            if value.has_unknown_bits() {
                return Err("expression contains unknown bits".to_string());
            }

            Ok(value.as_bigint(value.signed))
        }
        Expr::Grouped(expr) => evaluate_expr_as_math_bigint(expr),
        Expr::Unary { op, expr } => {
            let value = evaluate_expr_as_math_bigint(expr)?;
            Ok(match op {
                UnaryOp::Plus => value,
                UnaryOp::Minus => -value,
            })
        }
        Expr::Binary { op, lhs, rhs } => {
            if matches!(
                op,
                BinaryOp::LessThan
                    | BinaryOp::GreaterThan
                    | BinaryOp::LessThanOrEqual
                    | BinaryOp::GreaterThanOrEqual
            ) {
                let lhs_meta = infer_expr_meta(lhs)?;
                let rhs_meta = infer_expr_meta(rhs)?;
                let value = evaluate_relational_expr(*op, lhs, rhs, lhs_meta, rhs_meta, None)?;
                if value.has_unknown_bits() {
                    return Err("expression contains unknown bits".to_string());
                }
                return Ok(value.as_bigint(false));
            }

            let lhs_value = evaluate_expr_as_math_bigint(lhs)?;
            let rhs_value = evaluate_expr_as_math_bigint(rhs)?;

            match op {
                BinaryOp::Add => Ok(lhs_value + rhs_value),
                BinaryOp::Subtract => Ok(lhs_value - rhs_value),
                BinaryOp::Multiply => Ok(lhs_value * rhs_value),
                BinaryOp::Divide => {
                    if rhs_value.is_zero() {
                        Err("expression division by zero".to_string())
                    } else {
                        Ok(lhs_value / rhs_value)
                    }
                }
                BinaryOp::Modulus => {
                    if rhs_value.is_zero() {
                        Err("expression modulus by zero".to_string())
                    } else {
                        Ok(lhs_value % rhs_value)
                    }
                }
                BinaryOp::Power => evaluate_power(lhs_value, rhs_value),
                BinaryOp::LessThan
                | BinaryOp::GreaterThan
                | BinaryOp::LessThanOrEqual
                | BinaryOp::GreaterThanOrEqual => unreachable!("handled above"),
            }
        }
    }
}

fn evaluate_power(base: BigInt, exponent: BigInt) -> Result<BigInt, String> {
    if exponent.is_zero() {
        return Ok(BigInt::one());
    }

    if exponent.sign() == Sign::Minus {
        if base.is_zero() {
            return Err("power result is undefined".to_string());
        }

        if base == BigInt::one() {
            return Ok(BigInt::one());
        }

        if base == BigInt::from(-1) {
            let is_odd = (&(-exponent.clone()) & BigInt::one()) == BigInt::one();
            return Ok(if is_odd {
                BigInt::from(-1)
            } else {
                BigInt::one()
            });
        }

        return Ok(BigInt::zero());
    }

    let exponent = exponent
        .to_biguint()
        .expect("non-negative exponent should convert to BigUint");

    let mut result = BigInt::one();
    let mut factor = base;
    let mut remaining = exponent;

    while !remaining.is_zero() {
        if (&remaining & BigUint::one()) == BigUint::one() {
            result *= &factor;
        }

        remaining >>= 1;
        if !remaining.is_zero() {
            factor = &factor * &factor;
        }
    }

    Ok(result)
}

fn strip_statement_terminators(input: &str) -> &str {
    let mut trimmed = input.trim();

    while let Some(stripped) = trimmed.strip_suffix(';') {
        trimmed = stripped.trim_end();
    }

    trimmed
}

fn parse_system_task(input: &str) -> Result<Option<ParsedLine>, String> {
    for name in ["$finish", "$stop"] {
        if let Some(rest) = input.strip_prefix(name) {
            let rest = rest.trim();
            if rest.is_empty() || rest == "()" {
                return Ok(Some(ParsedLine::Exit));
            }

            return Err(format!("unsupported system task syntax: {input}"));
        }
    }

    if input.starts_with('$') {
        return Err(format!("unsupported system task: {input}"));
    }

    Ok(None)
}

fn parse_integer(input: &str) -> Result<IntegerValue, String> {
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

    if digits.chars().all(is_x_digit) {
        let width = width_hint.unwrap_or(32);
        return Ok(IntegerValue {
            width,
            signed,
            base: Base::Decimal,
            bits: vec![LogicBit::X; width],
        });
    }

    if digits.chars().all(is_z_digit) {
        let width = width_hint.unwrap_or(32);
        return Ok(IntegerValue {
            width,
            signed,
            base: Base::Decimal,
            bits: vec![LogicBit::Z; width],
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

fn render_group_digit(bits: &[LogicBit], base: Base) -> char {
    if bits.contains(&LogicBit::X) {
        return 'x';
    }

    if bits.contains(&LogicBit::Z) {
        return 'z';
    }

    let value = bits.iter().enumerate().fold(0u8, |acc, (index, bit)| {
        if *bit == LogicBit::One {
            acc | (1 << index)
        } else {
            acc
        }
    });

    match base {
        Base::Binary => {
            if value == 0 {
                '0'
            } else {
                '1'
            }
        }
        Base::Octal => char::from(b'0' + value),
        Base::Hex => {
            const DIGITS: &[u8; 16] = b"0123456789abcdef";
            DIGITS[value as usize] as char
        }
        Base::Decimal => unreachable!("decimal output uses dedicated rendering"),
    }
}

fn biguint_bit_len(value: &BigUint) -> usize {
    if value.is_zero() {
        0
    } else {
        value.bits() as usize
    }
}

fn signed_decimal_bit_len(value: &BigUint) -> usize {
    if value.is_zero() {
        1
    } else {
        biguint_bit_len(value) + 1
    }
}

fn biguint_to_bits_with_width(value: &BigUint, width: usize) -> Vec<LogicBit> {
    let one = BigUint::one();

    (0..width)
        .map(|shift| {
            if ((value >> shift) & &one).is_zero() {
                LogicBit::Zero
            } else {
                LogicBit::One
            }
        })
        .collect()
}

fn bigint_to_bits_with_width(value: &BigInt, width: usize) -> Vec<LogicBit> {
    let modulus = BigInt::one() << width;
    let normalized = ((value % &modulus) + &modulus) % &modulus;
    let unsigned = normalized
        .to_biguint()
        .expect("normalized modulo value should be non-negative");
    biguint_to_bits_with_width(&unsigned, width)
}

fn bits_to_biguint(bits: &[LogicBit]) -> BigUint {
    bits.iter()
        .enumerate()
        .fold(BigUint::zero(), |acc, (index, bit)| match bit {
            LogicBit::One => acc | (BigUint::one() << index),
            LogicBit::Zero | LogicBit::X | LogicBit::Z => acc,
        })
}

fn bits_to_signed_bigint(bits: &[LogicBit]) -> BigInt {
    let unsigned = bits_to_biguint(bits);

    if !matches!(bits.last(), Some(LogicBit::One)) {
        return BigInt::from(unsigned);
    }

    BigInt::from_biguint(Sign::Plus, unsigned) - (BigInt::one() << bits.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn evaluates_unsized_decimal() {
        let evaluation = evaluate_input("42").expect("decimal literal should parse");
        assert_eq!(evaluation.output, "32'sd42");
        assert!(!evaluation.should_exit);
    }

    #[test]
    fn evaluates_unsized_hex_with_32_bit_width() {
        let evaluation = evaluate_input("'hFF").expect("hex literal should parse");
        assert_eq!(evaluation.output, "32'h000000ff");
    }

    #[test]
    fn evaluates_sized_signed_decimal() {
        let evaluation = evaluate_input("8'Sd255;").expect("signed decimal should parse");
        assert_eq!(evaluation.output, "-8'sd1");
    }

    #[test]
    fn formats_signed_decimal_and_non_decimal_outputs_differently() {
        let simple_decimal = evaluate_input("1").expect("simple decimal should parse");
        let simple_negative = evaluate_input("-1").expect("simple negative should evaluate");
        let signed_positive = evaluate_input("4'sd1").expect("signed decimal should parse");
        let signed_negative =
            evaluate_input("-4'sd1").expect("signed decimal negation should evaluate");
        let signed_hex = evaluate_input("4'shF").expect("signed hex should parse");

        assert_eq!(simple_decimal.output, "32'sd1");
        assert_eq!(simple_negative.output, "-32'sd1");
        assert_eq!(signed_positive.output, "4'sd1");
        assert_eq!(signed_negative.output, "-4'sd1");
        assert_eq!(signed_hex.output, "4'shf");
    }

    #[test]
    fn accepts_spaces_inside_based_integer_literals_in_expressions() {
        let literal = evaluate_input("8 'd 6").expect("spaced based literal should parse");
        let unary = evaluate_input("- 8 'd 6").expect("spaced unary minus literal should parse");
        let expr =
            evaluate_input("8 'd 6 + 1").expect("spaced based literal expression should parse");

        assert_eq!(literal.output, "8'd6");
        assert_eq!(unary.output, "8'd250");
        assert_eq!(expr.output, "32'd7");
    }

    #[test]
    fn rejects_spaces_inside_base_token() {
        let missing_base =
            evaluate_input("8 ' d 6").expect_err("space after apostrophe should be rejected");
        let split_signed = evaluate_input("8 ' s d 6")
            .expect_err("spaces inside signed base token should be rejected");
        let split_signed_base =
            evaluate_input("8 's d 6").expect_err("space between s and base should be rejected");

        assert_eq!(missing_base, "missing base after apostrophe");
        assert_eq!(split_signed, "missing base after apostrophe");
        assert_eq!(split_signed_base, "missing base after signed marker");
    }

    #[test]
    fn accepts_apostrophe_led_based_literals_with_spaced_digits() {
        let hex = evaluate_input("'h 837FF").expect("apostrophe-led hex literal should parse");
        let signed_hex =
            evaluate_input("'sh f").expect("apostrophe-led signed hex literal should parse");

        assert_eq!(hex.output, "32'h000837ff");
        assert_eq!(signed_hex.output, "32'sh0000000f");
    }

    #[test]
    fn accepts_underscores_in_size_and_digits() {
        let decimal = evaluate_input("1_6'd1_0").expect("underscored decimal should parse");
        let hex = evaluate_input("'hff_ff").expect("underscored hex should parse");

        assert_eq!(decimal.output, "16'd10");
        assert_eq!(hex.output, "32'h0000ffff");
    }

    #[test]
    fn evaluates_based_literal_with_unknown_digits() {
        let evaluation = evaluate_input("4'b10x?").expect("binary literal should parse");
        assert_eq!(evaluation.output, "4'b10xz");
    }

    #[test]
    fn extends_sized_literals_from_their_leftmost_digit_kind() {
        let zero_extended = evaluate_input("4'b1").expect("binary literal should parse");
        let x_extended = evaluate_input("4'bx").expect("x literal should parse");
        let z_extended = evaluate_input("4'b?").expect("z literal should parse");
        let hex_extended = evaluate_input("8'hf").expect("hex literal should parse");

        assert_eq!(zero_extended.output, "4'b0001");
        assert_eq!(x_extended.output, "4'bxxxx");
        assert_eq!(z_extended.output, "4'bzzzz");
        assert_eq!(hex_extended.output, "8'h0f");
    }

    #[test]
    fn keeps_unsized_literals_wider_than_32_bits_when_needed() {
        let evaluation =
            evaluate_input("4294967296").expect("wide unsized decimal literal should parse");
        assert_eq!(evaluation.output, "34'sd4294967296");
    }

    #[test]
    fn parses_parenthesized_literal_expression() {
        let evaluation = evaluate_input("(42)").expect("parenthesized literal should parse");
        assert_eq!(evaluation.output, "32'sd42");
    }

    #[test]
    fn parses_binary_operator_precedence_into_ast() {
        let expression = parse_expression("1 + 2 * 3").expect("expression should parse");

        assert_eq!(
            expression,
            Expr::Binary {
                op: BinaryOp::Add,
                lhs: Box::new(Expr::Literal(
                    parse_integer("1").expect("literal should parse")
                )),
                rhs: Box::new(Expr::Binary {
                    op: BinaryOp::Multiply,
                    lhs: Box::new(Expr::Literal(
                        parse_integer("2").expect("literal should parse")
                    )),
                    rhs: Box::new(Expr::Literal(
                        parse_integer("3").expect("literal should parse")
                    )),
                }),
            }
        );
    }

    #[test]
    fn parses_unary_and_power_operators_into_ast() {
        let expression = parse_expression("-2 ** 3").expect("expression should parse");

        assert_eq!(
            expression,
            Expr::Binary {
                op: BinaryOp::Power,
                lhs: Box::new(Expr::Unary {
                    op: UnaryOp::Minus,
                    expr: Box::new(Expr::Literal(
                        parse_integer("2").expect("literal should parse")
                    )),
                }),
                rhs: Box::new(Expr::Literal(
                    parse_integer("3").expect("literal should parse")
                )),
            }
        );
    }

    #[test]
    fn unary_minus_binds_tighter_than_power() {
        let even_exp = evaluate_input("-2 ** 2").expect("even exponent should evaluate");
        let odd_exp = evaluate_input("-2 ** 3").expect("odd exponent should evaluate");

        assert_eq!(even_exp.output, "32'sd4");
        assert_eq!(odd_exp.output, "-32'sd8");
    }

    #[test]
    fn parses_power_operator_left_associatively() {
        let expression = parse_expression("3 ** 3 ** 3").expect("expression should parse");

        assert_eq!(
            expression,
            Expr::Binary {
                op: BinaryOp::Power,
                lhs: Box::new(Expr::Binary {
                    op: BinaryOp::Power,
                    lhs: Box::new(Expr::Literal(
                        parse_integer("3").expect("literal should parse")
                    )),
                    rhs: Box::new(Expr::Literal(
                        parse_integer("3").expect("literal should parse")
                    )),
                }),
                rhs: Box::new(Expr::Literal(
                    parse_integer("3").expect("literal should parse")
                )),
            }
        );
    }

    #[test]
    fn evaluates_chained_power_left_to_right() {
        let evaluation = evaluate_input("3 ** 3 ** 3").expect("chained power should evaluate");
        assert_eq!(evaluation.output, "32'sd19683");
    }

    #[test]
    fn rejects_missing_closing_parenthesis() {
        let error = parse_expression("(1 + 2").expect_err("expression should be rejected");
        assert_eq!(error, "missing closing parenthesis");
    }

    #[test]
    fn evaluates_unary_and_binary_additive_operators() {
        let unary_plus = evaluate_input("+5").expect("unary plus should evaluate");
        let unary_minus = evaluate_input("-5").expect("unary minus should evaluate");
        let addition = evaluate_input("4'd15 + 4'd1").expect("addition should evaluate");
        let subtraction = evaluate_input("4'd0 - 4'd1").expect("subtraction should evaluate");

        assert_eq!(unary_plus.output, "32'sd5");
        assert_eq!(unary_minus.output, "-32'sd5");
        assert_eq!(addition.output, "4'd0");
        assert_eq!(subtraction.output, "4'd15");
    }

    #[test]
    fn unary_plus_preserves_operand_bits_including_unknowns() {
        let binary = evaluate_input("+4'b01xz").expect("unary plus should preserve bits");
        let decimal = evaluate_input("+1").expect("unary plus on simple decimal should evaluate");

        assert_eq!(binary.output, "4'b01xz");
        assert_eq!(decimal.output, "32'sd1");
    }

    #[test]
    fn widens_nested_addition_from_parent_context() {
        let evaluation = evaluate_input("4'd15 + 4'd1 + 0").expect("addition should evaluate");
        assert_eq!(evaluation.output, "32'd16");
    }

    #[test]
    fn returns_all_x_when_additive_operand_contains_unknown_bits() {
        let addition = evaluate_input("4'bx + 1").expect("x addition should evaluate");
        let unary = evaluate_input("-4'bz").expect("z unary minus should evaluate");

        // Result base inherits from leftmost operand (binary), so the all-x result
        // is rendered in binary, one digit per bit.
        assert_eq!(addition.output, "32'bxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");
        assert_eq!(unary.output, "4'bxxxx");
    }

    #[test]
    fn evaluates_multiplicative_operators() {
        let multiply = evaluate_input("4'd3 * 4'd5").expect("multiply should evaluate");
        let divide = evaluate_input("8'd21 / 8'd4").expect("divide should evaluate");
        let modulus = evaluate_input("8'd21 % 8'd4").expect("modulus should evaluate");

        assert_eq!(multiply.output, "4'd15");
        assert_eq!(divide.output, "8'd5");
        assert_eq!(modulus.output, "8'd1");
    }

    #[test]
    fn applies_width_rules_to_multiplicative_expressions() {
        let truncated = evaluate_input("4'd8 * 4'd4").expect("multiply should evaluate");
        let widened =
            evaluate_input("4'd8 * 4'd4 + 0").expect("context-widened multiply should evaluate");

        assert_eq!(truncated.output, "4'd0");
        assert_eq!(widened.output, "32'd32");
    }

    #[test]
    fn returns_all_x_for_multiplicative_unknowns_and_zero_division() {
        let unknown = evaluate_input("4'bx * 2").expect("unknown multiply should evaluate");
        let divide_by_zero =
            evaluate_input("8'd3 / 8'd0").expect("divide by zero should evaluate to x");
        let modulus_by_zero =
            evaluate_input("8'd3 % 8'd0").expect("modulus by zero should evaluate to x");

        assert_eq!(unknown.output, "32'bxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");
        assert_eq!(divide_by_zero.output, "8'dx");
        assert_eq!(modulus_by_zero.output, "8'dx");
    }

    #[test]
    fn evaluates_power_operator() {
        let square = evaluate_input("4'd3 ** 2").expect("power should evaluate");
        let zero_exp = evaluate_input("4'd2 ** 0").expect("zero exponent should evaluate");
        let negative_exp = evaluate_input("4'd2 ** -1").expect("negative exponent should evaluate");

        assert_eq!(square.output, "4'd9");
        assert_eq!(zero_exp.output, "4'd1");
        assert_eq!(negative_exp.output, "4'd0");
    }

    #[test]
    fn applies_lhs_width_rule_to_power_operator() {
        let self_determined = evaluate_input("4'd3 ** 4'd3").expect("power should evaluate");
        let context_widened =
            evaluate_input("4'd3 ** 4'd3 + 0").expect("power should widen in context");

        assert_eq!(self_determined.output, "4'd11");
        assert_eq!(context_widened.output, "32'd27");
    }

    #[test]
    fn returns_all_x_for_power_unknowns_and_undefined_zero_negative_exponent() {
        let unknown = evaluate_input("4'bx ** 2").expect("unknown power should evaluate");
        let undefined = evaluate_input("0 ** -1").expect("undefined integer power should yield x");

        assert_eq!(unknown.output, "4'bxxxx");
        assert_eq!(undefined.output, "32'sdx");
    }

    #[test]
    fn zero_extends_signed_operands_in_mixed_unsigned_expressions() {
        let addition = evaluate_input("4'sd15 + 4'd1").expect("mixed add should evaluate");
        let division = evaluate_input("4'sd8 / 4'd2").expect("mixed divide should evaluate");

        assert_eq!(addition.output, "4'd0");
        assert_eq!(division.output, "4'd4");
    }

    #[test]
    fn preserves_signed_results_when_all_operands_are_signed() {
        let addition = evaluate_input("4'sd15 + 4'sd1").expect("signed add should evaluate");
        let division = evaluate_input("4'sd8 / 4'sd2").expect("signed divide should evaluate");
        let modulus = evaluate_input("4'sd15 % 4'sd2").expect("signed modulus should evaluate");

        assert_eq!(addition.output, "4'sd0");
        assert_eq!(division.output, "-4'sd4");
        assert_eq!(modulus.output, "-4'sd1");
    }

    #[test]
    fn handles_signed_negative_values_in_arithmetic() {
        let addition = evaluate_input("-4'sd1 + 4'sd1").expect("signed add should evaluate");
        let division = evaluate_input("-4'sd8 / 4'sd2").expect("signed divide should evaluate");
        let modulus = evaluate_input("-4'sd8 % 4'sd3").expect("signed modulus should evaluate");

        assert_eq!(addition.output, "4'sd0");
        assert_eq!(division.output, "-4'sd4");
        assert_eq!(modulus.output, "-4'sd2");
    }

    #[test]
    fn widens_signed_subexpressions_before_truncation() {
        let evaluation =
            evaluate_input("(-4'sd1 + -4'sd1) + 0").expect("signed expression should evaluate");
        assert_eq!(evaluation.output, "-32'sd2");
    }

    #[test]
    fn evaluates_negative_base_power_cases_from_lrm_examples() {
        let odd = evaluate_input("(-4'sd1) ** 3").expect("odd negative-base power should evaluate");
        let even =
            evaluate_input("(-4'sd1) ** 2").expect("even negative-base power should evaluate");
        let reciprocal =
            evaluate_input("(-4'sd1) ** -3").expect("negative exponent should evaluate");

        assert_eq!(odd.output, "-4'sd1");
        assert_eq!(even.output, "4'sd1");
        assert_eq!(reciprocal.output, "-4'sd1");
    }

    #[test]
    fn accepts_finish_and_stop_with_optional_parens() {
        let finish = evaluate_input("$finish()").expect("$finish() should parse");
        let stop = evaluate_input("$stop();").expect("$stop() should parse");

        assert_eq!(finish.output, "");
        assert!(finish.should_exit);
        assert_eq!(stop.output, "");
        assert!(stop.should_exit);
    }

    #[test]
    fn runs_repl_until_exit_command() {
        let mut input = Cursor::new("42\n$finish\nignored\n");
        let mut output = Vec::new();

        run_repl(&mut input, &mut output).expect("REPL should run");

        let output = String::from_utf8(output).expect("output should be valid UTF-8");
        assert_eq!(output, "In[0]: Out[0]: 32'sd42\nIn[1]: Out[1]: \n");
    }

    #[test]
    fn binary_arithmetic_preserves_shared_operand_base() {
        let binary_add = evaluate_input("4'b0111 + 4'b1001").expect("binary add should evaluate");
        let hex_add = evaluate_input("8'h0a + 8'h05").expect("hex add should evaluate");
        let hex_mul = evaluate_input("8'h0a * 8'h02").expect("hex multiply should evaluate");
        let hex_power = evaluate_input("4'h2 ** 2").expect("hex power should evaluate");

        assert_eq!(binary_add.output, "4'b0000");
        assert_eq!(hex_add.output, "8'h0f");
        assert_eq!(hex_mul.output, "8'h14");
        assert_eq!(hex_power.output, "4'h4");
    }

    #[test]
    fn binary_arithmetic_takes_leftmost_base_when_operands_differ() {
        let hex_then_binary = evaluate_input("8'h0a + 8'b1").expect("hex+binary should evaluate");
        let binary_then_hex =
            evaluate_input("8'b00001010 + 8'h05").expect("binary+hex should evaluate");

        assert_eq!(hex_then_binary.output, "8'h0b");
        assert_eq!(binary_then_hex.output, "8'b00001111");
    }

    #[test]
    fn unary_minus_preserves_operand_base() {
        let binary = evaluate_input("-4'b1").expect("binary unary minus should evaluate");
        let hex = evaluate_input("-8'h01").expect("hex unary minus should evaluate");

        assert_eq!(binary.output, "4'b1111");
        assert_eq!(hex.output, "8'hff");
    }

    #[test]
    fn tokenizes_le_and_ge_as_single_tokens() {
        let le = tokenize("4 <= 5").expect("<= should tokenize");
        let ge = tokenize("4 >= 5").expect(">= should tokenize");
        let lt = tokenize("4 < 5").expect("< should tokenize");
        let gt = tokenize("4 > 5").expect("> should tokenize");

        assert_eq!(le.len(), 3);
        assert_eq!(le[1], Token::LessEqual);
        assert_eq!(ge.len(), 3);
        assert_eq!(ge[1], Token::GreaterEqual);
        assert_eq!(lt[1], Token::Less);
        assert_eq!(gt[1], Token::Greater);
    }

    #[test]
    fn relational_binds_looser_than_additive() {
        // 1 + 2 < 4 parses as (1 + 2) < 4 → 3 < 4 → true
        let expr = parse_expression("1 + 2 < 4").expect("parse");
        match expr {
            Expr::Binary {
                op: BinaryOp::LessThan,
                lhs,
                ..
            } => assert!(matches!(*lhs, Expr::Binary { op: BinaryOp::Add, .. })),
            other => panic!("expected top-level <, got {other:?}"),
        }

        let result = evaluate_input("1 + 2 < 4").expect("precedence");
        assert_eq!(result.output, "1'b1");
    }

    #[test]
    fn relational_is_left_associative() {
        // 4 < 5 < 1 parses as (4 < 5) < 1 → 1 < 1 → false
        let expr = parse_expression("4 < 5 < 1").expect("parse");
        match expr {
            Expr::Binary {
                op: BinaryOp::LessThan,
                lhs,
                ..
            } => assert!(matches!(
                *lhs,
                Expr::Binary {
                    op: BinaryOp::LessThan,
                    ..
                }
            )),
            other => panic!("expected top-level <, got {other:?}"),
        }

        let result = evaluate_input("4 < 5 < 1").expect("assoc");
        assert_eq!(result.output, "1'b0");
    }

    #[test]
    fn evaluates_basic_unsigned_relational_operators() {
        let lt = evaluate_input("4'd3 < 4'd5").expect("lt");
        let gt = evaluate_input("4'd5 > 4'd3").expect("gt");
        let le_eq = evaluate_input("4'd3 <= 4'd3").expect("le eq");
        let ge_eq = evaluate_input("4'd3 >= 4'd3").expect("ge eq");
        let le_false = evaluate_input("4'd4 <= 4'd3").expect("le false");
        let ge_false = evaluate_input("4'd2 >= 4'd3").expect("ge false");

        assert_eq!(lt.output, "1'b1");
        assert_eq!(gt.output, "1'b1");
        assert_eq!(le_eq.output, "1'b1");
        assert_eq!(ge_eq.output, "1'b1");
        assert_eq!(le_false.output, "1'b0");
        assert_eq!(ge_false.output, "1'b0");
    }

    #[test]
    fn signed_relational_uses_real_world_signed_comparison() {
        let three_lt_five = evaluate_input("4'sd3 < 4'sd5").expect("signed lt");
        let neg_lt = evaluate_input("-4'sd1 < 4'sd2").expect("signed neg lt");
        let neg_gt_neg = evaluate_input("-4'sd1 > -4'sd2").expect("signed neg/neg");

        assert_eq!(three_lt_five.output, "1'b1");
        assert_eq!(neg_lt.output, "1'b1");
        assert_eq!(neg_gt_neg.output, "1'b1");
    }

    #[test]
    fn mixed_signedness_uses_unsigned_comparison() {
        // -4'sd1 has bits 1111 → reinterpreted as unsigned 15; 15 > 0
        let neg_one_gt_zero = evaluate_input("-4'sd1 > 4'd0").expect("neg vs unsigned");
        // -4'sd1 sign-extends to 11111111 (= 255 unsigned); 255 > 0
        let neg_one_gt_zero_widened = evaluate_input("-4'sd1 > 8'd0").expect("widened");
        // 4'sd2 sign-extends to 00000010 (= 2 unsigned); 2 > 5 is false
        let two_not_gt_five = evaluate_input("4'sd2 > 8'd5").expect("not gt");

        assert_eq!(neg_one_gt_zero.output, "1'b1");
        assert_eq!(neg_one_gt_zero_widened.output, "1'b1");
        assert_eq!(two_not_gt_five.output, "1'b0");
    }

    #[test]
    fn mixed_signedness_extends_before_reinterpreting_as_unsigned() {
        // LRM ordering trap: width extension uses each operand's OWN signedness
        // FIRST, then both bit patterns are reinterpreted as unsigned for the
        // comparison. Reversed ordering (reinterpret first, then zero-extend)
        // would give a different answer here.
        //
        // -4'sd1 sign-extends to 8 bits as `11111111` (= unsigned 255).
        // 255 > 16 → TRUE. Reversed-order would zero-extend `1111` to
        // `00001111` = 15, and 15 > 16 → FALSE (LRM-incorrect).
        let gt = evaluate_input("-4'sd1 > 8'd16").expect("ordering > case");
        let lt = evaluate_input("-4'sd1 < 8'd16").expect("ordering < case");

        assert_eq!(gt.output, "1'b1");
        assert_eq!(lt.output, "1'b0");
    }

    #[test]
    fn unsigned_relational_zero_extends_smaller_operand() {
        // 4'd1 zero-extends to 8 bits = 8'd1; 16 > 1 → true
        let result = evaluate_input("8'd16 > 4'd1").expect("widen unsigned");
        assert_eq!(result.output, "1'b1");
    }

    #[test]
    fn relational_propagates_unknown_bits_as_one_bit_x() {
        let with_x_lhs = evaluate_input("4'bx < 4'd1").expect("x lhs");
        let with_z_rhs = evaluate_input("4'd0 < 4'bz").expect("z rhs");
        let with_partial_x = evaluate_input("4'b01x0 > 4'd1").expect("partial x");

        assert_eq!(with_x_lhs.output, "1'bx");
        assert_eq!(with_z_rhs.output, "1'bx");
        assert_eq!(with_partial_x.output, "1'bx");
    }

    #[test]
    fn relational_result_widens_to_outer_arithmetic_context() {
        // (4'd3 < 4'd5) → 1'b1; outer + widens result to 4 bits.
        // Leftmost-base wins: relational's Binary; outer + is unsigned (mixed).
        let result = evaluate_input("(4'd3 < 4'd5) + 4'd0").expect("widened");
        assert_eq!(result.output, "4'b0001");
    }

    #[test]
    fn relational_result_renders_in_binary_regardless_of_operand_base() {
        // Both operands hex but the 1-bit relational result is binary.
        let hex_compare = evaluate_input("8'h0a < 8'h0f").expect("hex compare");
        assert_eq!(hex_compare.output, "1'b1");
    }
}
