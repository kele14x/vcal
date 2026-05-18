use num_bigint::{BigInt, BigUint, Sign};
use num_traits::{One, ToPrimitive, Zero};
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
    // True for literals parsed without an explicit size (LRM 3.5.1 default
    // width). Drives Table 5-22 footnote a's MSB-fill extension when the
    // propagated context is wider than the default. Always false for sized
    // literals and for any value produced by an operator.
    unsized_literal: bool,
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
        // LRM Table 5-22 footnote a: unsized constants in an expression wider
        // than 32 bits extend per the literal itself, not per the propagated
        // context. The MSB-fill case (x/z) and the literal's own signedness
        // both differ from §5.5.4, so we have to carve out a separate path
        // here rather than reuse `context_extension_bit`.
        if self.unsized_literal && width > self.width {
            return self.extend_unsized_to(width);
        }

        if width == self.width {
            return Self {
                unsized_literal: false,
                ..self.clone()
            };
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
            unsized_literal: false,
        }
    }

    // LRM Table 5-22 footnote a / §3.5.1 literal-fill rule: unsized constants
    // extend by their own MSB (x/z) or by their own declared signedness
    // (sign-extend if signed, zero-extend if unsigned). This is independent of
    // the propagated context signedness, which is why iverilog
    //   'bx        | 64'sb0  → 64'bxxxx...x  (MSB-fill ignores propagated sign)
    //   'shFFFFFFFF| 64'b0   → 64'hFFFFFFFFFFFFFFFF (own-signed sign-extend
    //                          even though propagated context is unsigned)
    // both diverge from §5.5.4. For sized literals §5.5.4 still applies.
    fn extend_unsized_to(&self, width: usize) -> Self {
        let msb = self.bits.last().copied().unwrap_or(LogicBit::Zero);
        let fill = match msb {
            LogicBit::X => LogicBit::X,
            LogicBit::Z => LogicBit::Z,
            _ if self.signed => msb,
            _ => LogicBit::Zero,
        };
        let mut bits = self.bits.clone();
        bits.resize(width, fill);
        Self {
            width,
            signed: self.signed,
            base: self.base,
            bits,
            unsized_literal: false,
        }
    }

    fn context_extension_bit(&self, context_signed: bool) -> LogicBit {
        // LRM §5.5.4 for sized operands: signed propagated context sign-extends
        // (propagating x/z if the MSB is x/z), unsigned propagated context
        // zero-extends. Unsized literals follow Table 5-22 footnote a instead;
        // see `extend_unsized_to`.
        if !context_signed {
            return LogicBit::Zero;
        }
        match self.bits.last().copied().unwrap_or(LogicBit::Zero) {
            LogicBit::X => LogicBit::X,
            LogicBit::Z => LogicBit::Z,
            LogicBit::One => LogicBit::One,
            LogicBit::Zero => LogicBit::Zero,
        }
    }

    fn as_bigint(&self, signed: bool) -> BigInt {
        if signed {
            bits_to_signed_bigint(&self.bits)
        } else {
            BigInt::from(bits_to_biguint(&self.bits))
        }
    }

    // Constructor for any value produced by an operator. Forces
    // `unsized_literal: false` so the leaf-extension carve-out (Table 5-22
    // footnote a) only fires for parser-produced unsized literals; computed
    // values must extend per §5.5.4 even if their MSB happens to be x/z.
    fn computed(width: usize, signed: bool, base: Base, bits: Vec<LogicBit>) -> Self {
        Self {
            width,
            signed,
            base,
            bits,
            unsized_literal: false,
        }
    }

    fn from_bigint(value: BigInt, width: usize, signed: bool, base: Base) -> Self {
        Self {
            width,
            signed,
            base,
            bits: bigint_to_bits_with_width(&value, width),
            unsized_literal: false,
        }
    }

    fn all_x(width: usize, signed: bool, base: Base) -> Self {
        Self {
            width,
            signed,
            base,
            bits: vec![LogicBit::X; width],
            unsized_literal: false,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnaryOp {
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
    EqualEqual,
    NotEqual,
    CaseEqual,
    CaseNotEqual,
    Bang,
    LogicalAnd,
    LogicalOr,
    Tilde,
    BitwiseAnd,
    BitwiseOr,
    BitwiseXor,
    BitwiseXnor,
    BitwiseNand,
    BitwiseNor,
    LogicalShiftLeft,
    LogicalShiftRight,
    ArithmeticShiftLeft,
    ArithmeticShiftRight,
    Question,
    Colon,
    LBrace,
    RBrace,
    Comma,
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
                // Greedy: `<<<` (arithmetic left shift) > `<<` (logical left
                // shift) > `<=` > `<`. Longest-prefix wins, mirroring how the
                // existing `==`/`===` and `~^`/`~&` paths disambiguate.
                if matches!(chars.peek(), Some((_, '<'))) {
                    chars.next();
                    if matches!(chars.peek(), Some((_, '<'))) {
                        chars.next();
                        tokens.push(Token::ArithmeticShiftLeft);
                    } else {
                        tokens.push(Token::LogicalShiftLeft);
                    }
                } else if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    tokens.push(Token::LessEqual);
                } else {
                    tokens.push(Token::Less);
                }
            }
            '>' => {
                if matches!(chars.peek(), Some((_, '>'))) {
                    chars.next();
                    if matches!(chars.peek(), Some((_, '>'))) {
                        chars.next();
                        tokens.push(Token::ArithmeticShiftRight);
                    } else {
                        tokens.push(Token::LogicalShiftRight);
                    }
                } else if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    tokens.push(Token::GreaterEqual);
                } else {
                    tokens.push(Token::Greater);
                }
            }
            '=' => {
                if !matches!(chars.peek(), Some((_, '='))) {
                    return Err("expected `==` or `===`".to_string());
                }
                chars.next();
                if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    tokens.push(Token::CaseEqual);
                } else {
                    tokens.push(Token::EqualEqual);
                }
            }
            '!' => {
                if matches!(chars.peek(), Some((_, '='))) {
                    chars.next();
                    if matches!(chars.peek(), Some((_, '='))) {
                        chars.next();
                        tokens.push(Token::CaseNotEqual);
                    } else {
                        tokens.push(Token::NotEqual);
                    }
                } else {
                    tokens.push(Token::Bang);
                }
            }
            '&' => {
                if matches!(chars.peek(), Some((_, '&'))) {
                    chars.next();
                    tokens.push(Token::LogicalAnd);
                } else {
                    tokens.push(Token::BitwiseAnd);
                }
            }
            '|' => {
                if matches!(chars.peek(), Some((_, '|'))) {
                    chars.next();
                    tokens.push(Token::LogicalOr);
                } else {
                    tokens.push(Token::BitwiseOr);
                }
            }
            '^' => {
                // ^~ is the alternate spelling of the bitwise equivalence
                // operator ~^ (LRM 5.1.10). Lex the two-char form greedily so
                // both spellings collapse onto the same token.
                if matches!(chars.peek(), Some((_, '~'))) {
                    chars.next();
                    tokens.push(Token::BitwiseXnor);
                } else {
                    tokens.push(Token::BitwiseXor);
                }
            }
            '~' => {
                // ~^ is the bitwise equivalence operator (LRM 5.1.10); ~& and
                // ~| are the unary-only NAND/NOR reduction operators
                // (LRM 5.1.11 + A.8.6). All three are lexed greedily so a
                // bare `~` only appears in a position where it must be the
                // per-bit unary NOT.
                match chars.peek() {
                    Some((_, '^')) => {
                        chars.next();
                        tokens.push(Token::BitwiseXnor);
                    }
                    Some((_, '&')) => {
                        chars.next();
                        tokens.push(Token::BitwiseNand);
                    }
                    Some((_, '|')) => {
                        chars.next();
                        tokens.push(Token::BitwiseNor);
                    }
                    _ => tokens.push(Token::Tilde),
                }
            }
            '?' => tokens.push(Token::Question),
            ':' => tokens.push(Token::Colon),
            '{' => tokens.push(Token::LBrace),
            '}' => tokens.push(Token::RBrace),
            ',' => tokens.push(Token::Comma),
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

            // Whitespace before the first digit is OK (e.g. `8'd 6`); once
            // we've started reading digits it terminates the literal so a
            // following `?` (or any other char) tokenises separately.
            if next_ch.is_whitespace() {
                if saw_digit {
                    break;
                }
                chars.next();
                continue;
            }

            chars.next();
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

        if next_ch.is_whitespace() {
            if saw_digit {
                break;
            }
            chars.next();
            continue;
        }

        chars.next();
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
    // Note: `?` is intentionally NOT a delimiter even though it tokenises
    // as the conditional operator's `?` — inside a based literal it is the
    // alias for `z` (LRM 3.5), and `read_integer_literal`'s pre-apostrophe
    // loop already exits on any non-digit, so `1?2` still tokenises as
    // `1`, `?`, `2`.
    matches!(
        ch,
        '(' | ')'
            | '+'
            | '-'
            | '*'
            | '/'
            | '%'
            | '<'
            | '>'
            | '='
            | '!'
            | '&'
            | '|'
            | '^'
            | '~'
            | ':'
            | '{'
            | '}'
            | ','
    )
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
        Expr::Conditional {
            cond,
            then_expr,
            else_expr,
        } => evaluate_conditional_expr(cond, then_expr, else_expr, context),
        Expr::Concatenation { items } => evaluate_concatenation_expr(items, context),
        Expr::Replication { count, items } => evaluate_replication_expr(count, items, context),
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
            UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitwiseNot => infer_expr_meta(expr),
            UnaryOp::LogicalNot
            | UnaryOp::ReductionAnd
            | UnaryOp::ReductionNand
            | UnaryOp::ReductionOr
            | UnaryOp::ReductionNor
            | UnaryOp::ReductionXor
            | UnaryOp::ReductionXnor => Ok(ExprMeta {
                width: 1,
                signed: false,
                base: Base::Binary,
            }),
        },
        Expr::Binary { op, lhs, rhs } => {
            let lhs_meta = infer_expr_meta(lhs)?;
            let rhs_meta = infer_expr_meta(rhs)?;
            Ok(combine_binary_meta(*op, lhs_meta, rhs_meta))
        }
        // LRM 5.1.13: cond is self-determined and contributes nothing to
        // the result meta; then/else are context-determined and unify
        // width (max) and signedness (any unsigned → unsigned, §5.5.1).
        Expr::Conditional {
            cond: _,
            then_expr,
            else_expr,
        } => {
            let then_meta = infer_expr_meta(then_expr)?;
            let else_meta = infer_expr_meta(else_expr)?;
            Ok(ExprMeta {
                width: usize::max(then_meta.width, else_meta.width),
                signed: then_meta.signed && else_meta.signed,
                base: then_meta.base,
            })
        }
        // LRM 5.1.14: width = sum of operand widths, always unsigned. Base
        // follows leftmost-wins (consistent with arithmetic/bitwise/shift).
        Expr::Concatenation { items } => {
            let mut total_width = 0usize;
            let mut leftmost_base = Base::Binary;
            for (idx, item) in items.iter().enumerate() {
                let item_meta = infer_expr_meta(item)?;
                total_width = total_width.saturating_add(item_meta.width);
                if idx == 0 {
                    leftmost_base = item_meta.base;
                }
            }
            Ok(ExprMeta {
                width: total_width,
                signed: false,
                base: leftmost_base,
            })
        }
        // Replication width depends on the constant count value, so we
        // evaluate it eagerly. We use the lenient count helper here — a
        // zero-replication is structurally valid (it just yields width 0)
        // and the per-position constraint is enforced at evaluation time
        // by `evaluate_replication_expr` (top-level) or
        // `collect_concatenation_bits` (the surrounding-list check).
        Expr::Replication { count, items } => {
            let count = evaluate_replication_count_allow_zero(count)?;
            let mut inner_width = 0usize;
            let mut leftmost_base = Base::Binary;
            for (idx, item) in items.iter().enumerate() {
                let item_meta = infer_expr_meta(item)?;
                inner_width = inner_width.saturating_add(item_meta.width);
                if idx == 0 {
                    leftmost_base = item_meta.base;
                }
            }
            Ok(ExprMeta {
                width: inner_width.saturating_mul(count),
                signed: false,
                base: leftmost_base,
            })
        }
    }
}

fn combine_binary_meta(op: BinaryOp, lhs_meta: ExprMeta, rhs_meta: ExprMeta) -> ExprMeta {
    match op {
        BinaryOp::Add
        | BinaryOp::Subtract
        | BinaryOp::Multiply
        | BinaryOp::Divide
        | BinaryOp::Modulus
        | BinaryOp::BitwiseAnd
        | BinaryOp::BitwiseOr
        | BinaryOp::BitwiseXor
        | BinaryOp::BitwiseXnor => ExprMeta {
            width: usize::max(lhs_meta.width, rhs_meta.width),
            signed: lhs_meta.signed && rhs_meta.signed,
            base: lhs_meta.base,
        },
        BinaryOp::Power => ExprMeta {
            width: lhs_meta.width,
            signed: lhs_meta.signed,
            base: lhs_meta.base,
        },
        // LRM 5.1.12: result width and signedness derive from the LHS only;
        // the RHS is self-determined and treated as unsigned, so it cannot
        // widen the result or flip its signedness.
        BinaryOp::LogicalShiftLeft
        | BinaryOp::LogicalShiftRight
        | BinaryOp::ArithmeticShiftLeft
        | BinaryOp::ArithmeticShiftRight => ExprMeta {
            width: lhs_meta.width,
            signed: lhs_meta.signed,
            base: lhs_meta.base,
        },
        BinaryOp::LessThan
        | BinaryOp::GreaterThan
        | BinaryOp::LessThanOrEqual
        | BinaryOp::GreaterThanOrEqual
        | BinaryOp::Equal
        | BinaryOp::NotEqual
        | BinaryOp::CaseEqual
        | BinaryOp::CaseNotEqual
        | BinaryOp::LogicalAnd
        | BinaryOp::LogicalOr => ExprMeta {
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
    if op == UnaryOp::LogicalNot {
        // LRM 5.4: logical operands are self-determined — evaluate without
        // pushing a context down, reduce to the operand's logical value, then
        // apply the !-truth table from §5.1.9.
        let operand = evaluate_expr_in_context(expr, None)?;
        let bit = match logical_value(&operand) {
            LogicBit::One => LogicBit::Zero,
            LogicBit::Zero => LogicBit::One,
            LogicBit::X | LogicBit::Z => LogicBit::X,
        };
        return Ok(widen_relational_result(
            comparison_result_value(bit),
            context,
        ));
    }

    if is_reduction_op(op) {
        // LRM 5.1.11: reduction operands are self-determined (LRM Table 5-22)
        // and the result is always 1-bit unsigned. Same outer-context
        // widening shape as `!`/`&&`/`||`/relational/equality.
        let operand = evaluate_expr_in_context(expr, None)?;
        let bit = reduce_bits(op, &operand.bits);
        return Ok(widen_relational_result(
            comparison_result_value(bit),
            context,
        ));
    }

    let meta = infer_expr_meta(expr)?;
    // LRM 5.5.2: unary +/-/~ is context-determined — propagated size AND
    // signedness must reach the inner primary. Falling back to the operand's
    // own signedness here would sign-extend a signed leaf even when the
    // surrounding comparison/arithmetic unified to unsigned, mis-encoding
    // the value before negation.
    let effective_meta = ExprMeta {
        width: context.map_or(meta.width, |ctx| usize::max(ctx.width, meta.width)),
        signed: context.map_or(meta.signed, |ctx| ctx.signed),
        base: meta.base,
    };
    let operand = evaluate_expr_in_context(expr, Some(effective_meta))?;

    if op == UnaryOp::Plus {
        return Ok(operand);
    }

    if op == UnaryOp::BitwiseNot {
        // Per-bit flip: x and z both fold to x; no all-x short-circuit since
        // bitwise ops mix known and unknown bits per position.
        let bits: Vec<LogicBit> = operand.bits.iter().copied().map(bitwise_not_bit).collect();
        return Ok(IntegerValue::computed(
            effective_meta.width,
            effective_meta.signed,
            meta.base,
            bits,
        ));
    }

    if operand.has_unknown_bits() {
        return Ok(IntegerValue::all_x(
            effective_meta.width,
            effective_meta.signed,
            meta.base,
        ));
    }

    let value = operand.as_bigint(effective_meta.signed);
    let result = match op {
        UnaryOp::Minus => -value,
        UnaryOp::Plus => unreachable!("handled before arithmetic evaluation"),
        UnaryOp::LogicalNot => unreachable!("handled by early-return path"),
        UnaryOp::BitwiseNot => unreachable!("handled by early-return path"),
        UnaryOp::ReductionAnd
        | UnaryOp::ReductionNand
        | UnaryOp::ReductionOr
        | UnaryOp::ReductionNor
        | UnaryOp::ReductionXor
        | UnaryOp::ReductionXnor => unreachable!("handled by early-return path"),
    };

    Ok(IntegerValue::from_bigint(
        result,
        effective_meta.width,
        effective_meta.signed,
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

    if matches!(
        op,
        BinaryOp::Equal | BinaryOp::NotEqual | BinaryOp::CaseEqual | BinaryOp::CaseNotEqual
    ) {
        return evaluate_equality_expr(op, lhs, rhs, lhs_meta, rhs_meta, context);
    }

    if matches!(op, BinaryOp::LogicalAnd | BinaryOp::LogicalOr) {
        return evaluate_logical_expr(op, lhs, rhs, context);
    }

    if matches!(
        op,
        BinaryOp::LogicalShiftLeft
            | BinaryOp::LogicalShiftRight
            | BinaryOp::ArithmeticShiftLeft
            | BinaryOp::ArithmeticShiftRight
    ) {
        return evaluate_shift_expr(op, lhs, rhs, lhs_meta, context);
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
        BinaryOp::BitwiseAnd | BinaryOp::BitwiseOr | BinaryOp::BitwiseXor | BinaryOp::BitwiseXnor => {
            // Both operands inherit the unified width/sign context, so each
            // side's leaf primary extends consistently before we zip bits.
            let lhs_value = evaluate_expr_in_context(lhs, Some(effective_meta))?;
            let rhs_value = evaluate_expr_in_context(rhs, Some(effective_meta))?;

            let combine = match op {
                BinaryOp::BitwiseAnd => bitwise_and_bits,
                BinaryOp::BitwiseOr => bitwise_or_bits,
                BinaryOp::BitwiseXor => bitwise_xor_bits,
                BinaryOp::BitwiseXnor => bitwise_xnor_bits,
                _ => unreachable!("guarded by outer match"),
            };

            let bits: Vec<LogicBit> = lhs_value
                .bits
                .iter()
                .zip(rhs_value.bits.iter())
                .map(|(l, r)| combine(*l, *r))
                .collect();

            Ok(IntegerValue::computed(
                effective_meta.width,
                meta.signed,
                meta.base,
                bits,
            ))
        }
        BinaryOp::LessThan
        | BinaryOp::GreaterThan
        | BinaryOp::LessThanOrEqual
        | BinaryOp::GreaterThanOrEqual => {
            unreachable!("relational ops dispatched to evaluate_relational_expr")
        }
        BinaryOp::Equal | BinaryOp::NotEqual | BinaryOp::CaseEqual | BinaryOp::CaseNotEqual => {
            unreachable!("equality ops dispatched to evaluate_equality_expr")
        }
        BinaryOp::LogicalAnd | BinaryOp::LogicalOr => {
            unreachable!("logical ops dispatched to evaluate_logical_expr")
        }
        BinaryOp::LogicalShiftLeft
        | BinaryOp::LogicalShiftRight
        | BinaryOp::ArithmeticShiftLeft
        | BinaryOp::ArithmeticShiftRight => {
            unreachable!("shift ops dispatched to evaluate_shift_expr")
        }
    }
}

// LRM 5.1.12: the LHS is context-determined like arithmetic — its width
// widens to max(L(lhs), L(context)) and the propagated signedness drives
// extension at its leaf primary. The RHS is self-determined (LRM Table 5-22)
// and "always treated as an unsigned number ... has no effect on the
// signedness of the result", so we pass it `None` for the context and read
// its bits as unsigned regardless of the operand's declared signedness.
//
// `>>>` (arithmetic right shift) fills vacated MSB positions with the LHS
// sign bit only when the propagated context is signed. Under an unsigned
// outer context the same operator zero-fills, matching iverilog. The other
// three shift forms always zero-fill.
fn evaluate_shift_expr(
    op: BinaryOp,
    lhs: &Expr,
    rhs: &Expr,
    lhs_meta: ExprMeta,
    context: Option<ExprMeta>,
) -> Result<IntegerValue, String> {
    let meta = ExprMeta {
        width: lhs_meta.width,
        signed: lhs_meta.signed,
        base: lhs_meta.base,
    };
    let effective_meta = ExprMeta {
        width: context.map_or(meta.width, |ctx| usize::max(ctx.width, meta.width)),
        signed: context.map_or(meta.signed, |ctx| ctx.signed),
        base: meta.base,
    };

    let lhs_value = evaluate_expr_in_context(lhs, Some(effective_meta))?;
    // RHS is self-determined: do NOT push effective_meta; let it evaluate at
    // its own width, then reinterpret its bits as unsigned for the count.
    let rhs_value = evaluate_expr_in_context(rhs, None)?;

    if rhs_value.has_unknown_bits() {
        return Ok(IntegerValue::all_x(
            effective_meta.width,
            effective_meta.signed,
            meta.base,
        ));
    }

    let shift_count = bits_to_biguint(&rhs_value.bits);
    // BigUint shift counts can dwarf usize; clamp to the result width since
    // any larger count produces the same all-fill output.
    let max_shift = BigUint::from(effective_meta.width);
    let clamped_shift = if shift_count >= max_shift {
        effective_meta.width
    } else {
        shift_count
            .to_usize()
            .expect("shift count smaller than width fits in usize")
    };

    let result_bits = match op {
        BinaryOp::LogicalShiftLeft | BinaryOp::ArithmeticShiftLeft => {
            shift_bits_left(&lhs_value.bits, clamped_shift)
        }
        BinaryOp::LogicalShiftRight => {
            shift_bits_right(&lhs_value.bits, clamped_shift, LogicBit::Zero)
        }
        BinaryOp::ArithmeticShiftRight => {
            let fill = if effective_meta.signed {
                lhs_value.bits.last().copied().unwrap_or(LogicBit::Zero)
            } else {
                LogicBit::Zero
            };
            shift_bits_right(&lhs_value.bits, clamped_shift, fill)
        }
        _ => unreachable!("evaluate_shift_expr called with non-shift op"),
    };

    Ok(IntegerValue::computed(
        effective_meta.width,
        effective_meta.signed,
        meta.base,
        result_bits,
    ))
}

fn shift_bits_left(bits: &[LogicBit], shift: usize) -> Vec<LogicBit> {
    let width = bits.len();
    (0..width)
        .map(|i| {
            if i < shift {
                LogicBit::Zero
            } else {
                bits[i - shift]
            }
        })
        .collect()
}

fn shift_bits_right(bits: &[LogicBit], shift: usize, fill: LogicBit) -> Vec<LogicBit> {
    let width = bits.len();
    (0..width)
        .map(|i| match i.checked_add(shift) {
            Some(src) if src < width => bits[src],
            _ => fill,
        })
        .collect()
}

// LRM 5.5.2: relational/equality operands form a shared context — size =
// max(L(i), L(j)), signed iff both signed. The propagated type drives extension
// at the leaf primary (sign-extend only when propagated type is signed), so the
// unified `comparison_signed` is what each operand sees.
fn unify_comparison_operands(
    lhs: &Expr,
    rhs: &Expr,
    lhs_meta: ExprMeta,
    rhs_meta: ExprMeta,
) -> Result<(IntegerValue, IntegerValue, bool), String> {
    let operand_width = usize::max(lhs_meta.width, rhs_meta.width);
    let comparison_signed = lhs_meta.signed && rhs_meta.signed;

    let lhs_context = ExprMeta {
        width: operand_width,
        signed: comparison_signed,
        base: lhs_meta.base,
    };
    let rhs_context = ExprMeta {
        width: operand_width,
        signed: comparison_signed,
        base: rhs_meta.base,
    };

    let lhs_value = evaluate_expr_in_context(lhs, Some(lhs_context))?;
    let rhs_value = evaluate_expr_in_context(rhs, Some(rhs_context))?;

    Ok((lhs_value, rhs_value, comparison_signed))
}

fn comparison_result_value(bit: LogicBit) -> IntegerValue {
    IntegerValue::computed(1, false, Base::Binary, vec![bit])
}

// LRM 5.1.9: an operand reduces to its logical value before the
// !/&&/|| truth table applies. Any 1 bit makes the operand definitely
// true; all-zero is definitely false; otherwise (any x/z, no 1) the
// operand is ambiguous and reduces to x.
fn logical_value(value: &IntegerValue) -> LogicBit {
    if value.bits.iter().any(|bit| *bit == LogicBit::One) {
        LogicBit::One
    } else if value.bits.iter().all(|bit| *bit == LogicBit::Zero) {
        LogicBit::Zero
    } else {
        LogicBit::X
    }
}

fn evaluate_logical_expr(
    op: BinaryOp,
    lhs: &Expr,
    rhs: &Expr,
    context: Option<ExprMeta>,
) -> Result<IntegerValue, String> {
    // LRM 5.4: each operand is self-determined, so we evaluate them in
    // isolation rather than unifying widths the way relational/equality do.
    let lhs_logical = logical_value(&evaluate_expr_in_context(lhs, None)?);
    let rhs_logical = logical_value(&evaluate_expr_in_context(rhs, None)?);

    // LRM 5.1.9 Table 5-7. Iverilog-confirmed via doc/four_value_ops_output.txt:
    // a definite false defeats x in &&, a definite true defeats x in ||.
    let bit = match op {
        BinaryOp::LogicalAnd => match (lhs_logical, rhs_logical) {
            (LogicBit::Zero, _) | (_, LogicBit::Zero) => LogicBit::Zero,
            (LogicBit::One, LogicBit::One) => LogicBit::One,
            _ => LogicBit::X,
        },
        BinaryOp::LogicalOr => match (lhs_logical, rhs_logical) {
            (LogicBit::One, _) | (_, LogicBit::One) => LogicBit::One,
            (LogicBit::Zero, LogicBit::Zero) => LogicBit::Zero,
            _ => LogicBit::X,
        },
        _ => unreachable!("non-logical op in evaluate_logical_expr"),
    };

    Ok(widen_relational_result(comparison_result_value(bit), context))
}

fn evaluate_relational_expr(
    op: BinaryOp,
    lhs: &Expr,
    rhs: &Expr,
    lhs_meta: ExprMeta,
    rhs_meta: ExprMeta,
    context: Option<ExprMeta>,
) -> Result<IntegerValue, String> {
    let (lhs_value, rhs_value, comparison_signed) =
        unify_comparison_operands(lhs, rhs, lhs_meta, rhs_meta)?;

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
    Ok(widen_relational_result(comparison_result_value(bit), context))
}

fn evaluate_equality_expr(
    op: BinaryOp,
    lhs: &Expr,
    rhs: &Expr,
    lhs_meta: ExprMeta,
    rhs_meta: ExprMeta,
    context: Option<ExprMeta>,
) -> Result<IntegerValue, String> {
    // Bit-level comparison; the unified signedness only matters for operand
    // extension (already done inside `unify_comparison_operands`), not for the
    // comparison itself.
    let (lhs_value, rhs_value, _comparison_signed) =
        unify_comparison_operands(lhs, rhs, lhs_meta, rhs_meta)?;

    let bit = match op {
        // LRM 5.1.8: ==/!= are 1-bit x only when the relation is *ambiguous*.
        // A single definite bit mismatch (0 vs 1) makes the operands unequal
        // regardless of any x/z elsewhere; only when no bit definitively
        // mismatches AND at least one bit involves x or z is the result x.
        BinaryOp::Equal | BinaryOp::NotEqual => {
            let mut definite_mismatch = false;
            let mut has_unknown = false;
            for (lb, rb) in lhs_value.bits.iter().zip(rhs_value.bits.iter()) {
                match (lb, rb) {
                    (LogicBit::Zero, LogicBit::One) | (LogicBit::One, LogicBit::Zero) => {
                        definite_mismatch = true;
                    }
                    (LogicBit::X | LogicBit::Z, _) | (_, LogicBit::X | LogicBit::Z) => {
                        has_unknown = true;
                    }
                    _ => {}
                }
            }
            if !definite_mismatch && has_unknown {
                return Ok(widen_relational_result(
                    IntegerValue::all_x(1, false, Base::Binary),
                    context,
                ));
            }
            let equal = !definite_mismatch;
            let result = if matches!(op, BinaryOp::Equal) {
                equal
            } else {
                !equal
            };
            if result { LogicBit::One } else { LogicBit::Zero }
        }
        // LRM 5.1.8: ===/!== compare bit-for-bit including x and z; the result
        // is always a known 0 or 1, never x. Operands are already the same
        // length after unification.
        BinaryOp::CaseEqual | BinaryOp::CaseNotEqual => {
            let equal = lhs_value.bits == rhs_value.bits;
            let result = if matches!(op, BinaryOp::CaseEqual) {
                equal
            } else {
                !equal
            };
            if result { LogicBit::One } else { LogicBit::Zero }
        }
        _ => unreachable!("non-equality op in evaluate_equality_expr"),
    };

    Ok(widen_relational_result(comparison_result_value(bit), context))
}

// LRM 5.1.13: cond is self-determined and reduced to a 1-bit logical the
// way `&&`/`||`/`!` reduce their operands. then/else are context-determined
// and unify width/sign with each other AND with the propagated outer
// context. As with the shift path, signedness must consult the propagated
// context — if the surrounding expression is unsigned (§5.5.1), the leaf
// primaries of then/else must zero-fill rather than sign-fill.
//
// When cond is x or z, evaluate both branches and merge per bit: agreeing
// bits stay, disagreeing bits become x. This preserves x/z agreement
// (e.g. x ∩ x → x) and reduces any disagreement (including 0 vs x) to x.
fn evaluate_conditional_expr(
    cond: &Expr,
    then_expr: &Expr,
    else_expr: &Expr,
    context: Option<ExprMeta>,
) -> Result<IntegerValue, String> {
    let then_meta = infer_expr_meta(then_expr)?;
    let else_meta = infer_expr_meta(else_expr)?;
    let meta = ExprMeta {
        width: usize::max(then_meta.width, else_meta.width),
        signed: then_meta.signed && else_meta.signed,
        base: then_meta.base,
    };
    let effective_meta = ExprMeta {
        width: context.map_or(meta.width, |ctx| usize::max(ctx.width, meta.width)),
        signed: context.map_or(meta.signed, |ctx| ctx.signed),
        base: meta.base,
    };

    let cond_value = evaluate_expr_in_context(cond, None)?;
    let cond_logical = logical_value(&cond_value);

    let then_value = evaluate_expr_in_context(then_expr, Some(effective_meta))?;
    let else_value = evaluate_expr_in_context(else_expr, Some(effective_meta))?;

    let bits = match cond_logical {
        LogicBit::One => then_value.bits,
        LogicBit::Zero => else_value.bits,
        LogicBit::X | LogicBit::Z => then_value
            .bits
            .iter()
            .zip(else_value.bits.iter())
            .map(|(t, e)| if t == e { *t } else { LogicBit::X })
            .collect(),
    };

    Ok(IntegerValue::computed(
        effective_meta.width,
        effective_meta.signed,
        meta.base,
        bits,
    ))
}

// LRM 5.1.14: every concatenation operand "shall be sized" — an operand
// with indefinite width (i.e. one whose self-determined width comes from an
// unsized literal) is rejected. The flag propagates through context-determined
// operators that take width from their operands (arithmetic/bitwise/power,
// shift LHS, conditional branches, unary +/-/~), but stops at any operator
// with a definite 1-bit result (relational/equality/logical/reduction) and at
// concatenation/replication themselves (their result widths are summed/
// multiplied integers, never indefinite). Matches iverilog's "indefinite
// width" rejection (e.g. `{4'd1 + 1, 4'd2}` → error because the `1` is
// unsized).
fn is_indefinite_width(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(value) => value.unsized_literal,
        Expr::Grouped(inner) => is_indefinite_width(inner),
        Expr::Unary { op, expr } => match op {
            UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitwiseNot => is_indefinite_width(expr),
            UnaryOp::LogicalNot
            | UnaryOp::ReductionAnd
            | UnaryOp::ReductionNand
            | UnaryOp::ReductionOr
            | UnaryOp::ReductionNor
            | UnaryOp::ReductionXor
            | UnaryOp::ReductionXnor => false,
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
            | BinaryOp::BitwiseXnor => is_indefinite_width(lhs) || is_indefinite_width(rhs),
            BinaryOp::Power => is_indefinite_width(lhs),
            BinaryOp::LogicalShiftLeft
            | BinaryOp::LogicalShiftRight
            | BinaryOp::ArithmeticShiftLeft
            | BinaryOp::ArithmeticShiftRight => is_indefinite_width(lhs),
            BinaryOp::LessThan
            | BinaryOp::GreaterThan
            | BinaryOp::LessThanOrEqual
            | BinaryOp::GreaterThanOrEqual
            | BinaryOp::Equal
            | BinaryOp::NotEqual
            | BinaryOp::CaseEqual
            | BinaryOp::CaseNotEqual
            | BinaryOp::LogicalAnd
            | BinaryOp::LogicalOr => false,
        },
        Expr::Conditional {
            cond: _,
            then_expr,
            else_expr,
        } => is_indefinite_width(then_expr) || is_indefinite_width(else_expr),
        Expr::Concatenation { .. } | Expr::Replication { .. } => false,
    }
}

// Replication count must be a constant, non-negative, non-x, non-z value
// (LRM 5.1.14). `to_usize` doubles as the "fits in addressable space" check;
// vcal uses `usize` for widths, so an oversized count surfaces as a clean
// error rather than overflowing. Zero is allowed at parse-meta level — the
// position-sensitive rule (zero is valid only inside a concatenation whose
// other operands sum to positive width) is enforced separately in
// `evaluate_replication_count` (top-level) and `collect_concatenation_bits`
// (the surrounding-list check).
fn evaluate_replication_count_allow_zero(count_expr: &Expr) -> Result<usize, String> {
    let value = evaluate_expr_in_context(count_expr, None)?;
    if value.has_unknown_bits() {
        return Err("replication count contains unknown bits".to_string());
    }
    let count = value.as_bigint(value.signed);
    if count.sign() == Sign::Minus {
        return Err("replication count must be non-negative".to_string());
    }
    count
        .to_usize()
        .ok_or_else(|| "replication count too large".to_string())
}

// Strict variant: a top-level replication (one whose result is the whole
// expression, or whose only consumers are non-concatenation operators) needs
// a positive count, since it would otherwise produce a zero-width
// `IntegerValue` in a position where vcal can't represent it. iverilog
// rejects the same case ("Concatenation repeat may not be zero in this
// context").
fn evaluate_replication_count(count_expr: &Expr) -> Result<usize, String> {
    let count = evaluate_replication_count_allow_zero(count_expr)?;
    if count == 0 {
        return Err("replication count must be positive in this context".to_string());
    }
    Ok(count)
}

// Walk through `Grouped` wrappers without evaluating. Used so that
// `({0{1'b1}})` is treated the same as `{0{1'b1}}` when the parent is
// looking for a Replication child to allow zero replication on
// (matching iverilog).
fn unwrap_grouped(expr: &Expr) -> &Expr {
    match expr {
        Expr::Grouped(inner) => unwrap_grouped(inner),
        other => other,
    }
}

// LRM 5.1.14: each operand is self-determined (no context propagated down)
// and must have a definite width — bare unsized literals (and any expression
// whose width derives from one) are rejected. Bits are joined MSB-first: the
// leftmost item ends up in the high bits of the result. Result is always
// unsigned; outer context can only widen the joined value (zero-extending),
// never reach into the operands.
fn evaluate_concatenation_expr(
    items: &[Expr],
    context: Option<ExprMeta>,
) -> Result<IntegerValue, String> {
    let bits = collect_concatenation_bits(items)?;
    let leftmost_base = infer_expr_meta(&items[0])?.base;
    let natural_width = bits.len();
    let result = IntegerValue::computed(natural_width, false, leftmost_base, bits);
    Ok(extend_to_outer_context(result, context))
}

fn evaluate_replication_expr(
    count_expr: &Expr,
    items: &[Expr],
    context: Option<ExprMeta>,
) -> Result<IntegerValue, String> {
    let count = evaluate_replication_count(count_expr)?;
    let inner_bits = collect_concatenation_bits(items)?;
    let leftmost_base = infer_expr_meta(&items[0])?.base;

    let mut bits = Vec::with_capacity(inner_bits.len().saturating_mul(count));
    for _ in 0..count {
        bits.extend(inner_bits.iter().copied());
    }
    let natural_width = bits.len();
    let result = IntegerValue::computed(natural_width, false, leftmost_base, bits);
    Ok(extend_to_outer_context(result, context))
}

// Joins the bit patterns of every item in a concatenation list (used both
// for plain `{a, b, ...}` and for the inner list of `{N{a, b, ...}}`).
//
// LRM 5.1.14 lets a replication's count be zero when it sits directly inside
// a concatenation — the zero-rep contributes no bits, but the surrounding
// list must still have at least one operand of positive size. So we
// special-case Replication items here (looking through `Grouped`) to permit
// a zero count, then verify the joined width is non-zero. This rejects
// `{ {0{1'b1}} }` and `{N{ {0{1'b1}} }}` (no positive-size sibling) while
// accepting `{ {0{1'b1}}, 1'b1 }` and `{N{ {0{1'b1}}, 1'b1 }}` — matching
// iverilog.
fn collect_concatenation_bits(items: &[Expr]) -> Result<Vec<LogicBit>, String> {
    if items.is_empty() {
        return Err("concatenation requires at least one operand".to_string());
    }
    for item in items {
        if is_indefinite_width(item) {
            return Err("concatenation operand has indefinite width".to_string());
        }
    }
    // Items are in source order (leftmost first → MSB-side). Our bit vectors
    // are LSB-first, so we feed bits starting from the rightmost item.
    let mut bits = Vec::new();
    for item in items.iter().rev() {
        bits.extend(evaluate_concatenation_item_bits(item)?);
    }
    if bits.is_empty() {
        // Every operand collapsed to zero width — the concatenation has no
        // positive-size operand, which is the case LRM 5.1.14 forbids.
        return Err(
            "concatenation must have at least one operand with positive size".to_string(),
        );
    }
    Ok(bits)
}

fn evaluate_concatenation_item_bits(item: &Expr) -> Result<Vec<LogicBit>, String> {
    if let Expr::Replication { count, items } = unwrap_grouped(item) {
        let count = evaluate_replication_count_allow_zero(count)?;
        if count == 0 {
            return Ok(Vec::new());
        }
        let inner_bits = collect_concatenation_bits(items)?;
        let mut bits = Vec::with_capacity(inner_bits.len().saturating_mul(count));
        for _ in 0..count {
            bits.extend(inner_bits.iter().copied());
        }
        return Ok(bits);
    }
    let value = evaluate_expr_in_context(item, None)?;
    Ok(value.bits)
}

// Concatenation/replication results are unsigned; if an outer context is
// wider, zero-extend (reusing the existing §5.5.4 path via
// `resized_to_context` with context_signed = false). If the outer context is
// narrower or absent we keep the natural width — concatenation is
// self-determined, so the joined width never shrinks below itself.
fn extend_to_outer_context(value: IntegerValue, context: Option<ExprMeta>) -> IntegerValue {
    match context {
        Some(ctx) if ctx.width > value.width => value.resized_to_context(ctx.width, false),
        _ => value,
    }
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
            if matches!(op, UnaryOp::LogicalNot) || is_reduction_op(*op) {
                // Both produce a 1-bit unsigned result via the same
                // early-return shape, so a single bigint conversion works.
                let value = evaluate_unary_expr(*op, expr, None)?;
                if value.has_unknown_bits() {
                    return Err("expression contains unknown bits".to_string());
                }
                return Ok(value.as_bigint(false));
            }
            if matches!(op, UnaryOp::BitwiseNot) {
                // Bitwise NOT depends on operand width, so go through
                // evaluate_unary_expr rather than negating in bigint.
                let value = evaluate_unary_expr(*op, expr, None)?;
                if value.has_unknown_bits() {
                    return Err("expression contains unknown bits".to_string());
                }
                return Ok(value.as_bigint(value.signed));
            }
            let value = evaluate_expr_as_math_bigint(expr)?;
            Ok(match op {
                UnaryOp::Plus => value,
                UnaryOp::Minus => -value,
                UnaryOp::LogicalNot => unreachable!("handled above"),
                UnaryOp::BitwiseNot => unreachable!("handled above"),
                UnaryOp::ReductionAnd
                | UnaryOp::ReductionNand
                | UnaryOp::ReductionOr
                | UnaryOp::ReductionNor
                | UnaryOp::ReductionXor
                | UnaryOp::ReductionXnor => unreachable!("handled above"),
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

            if matches!(
                op,
                BinaryOp::Equal
                    | BinaryOp::NotEqual
                    | BinaryOp::CaseEqual
                    | BinaryOp::CaseNotEqual
            ) {
                let lhs_meta = infer_expr_meta(lhs)?;
                let rhs_meta = infer_expr_meta(rhs)?;
                let value = evaluate_equality_expr(*op, lhs, rhs, lhs_meta, rhs_meta, None)?;
                if value.has_unknown_bits() {
                    return Err("expression contains unknown bits".to_string());
                }
                return Ok(value.as_bigint(false));
            }

            if matches!(op, BinaryOp::LogicalAnd | BinaryOp::LogicalOr) {
                let value = evaluate_logical_expr(*op, lhs, rhs, None)?;
                if value.has_unknown_bits() {
                    return Err("expression contains unknown bits".to_string());
                }
                return Ok(value.as_bigint(false));
            }

            if matches!(
                op,
                BinaryOp::BitwiseAnd
                    | BinaryOp::BitwiseOr
                    | BinaryOp::BitwiseXor
                    | BinaryOp::BitwiseXnor
            ) {
                // Bitwise binaries depend on the unified operand width, so
                // evaluate them through the normal pipeline rather than
                // applying bigint operators directly.
                let value = evaluate_binary_expr(*op, lhs, rhs, None)?;
                if value.has_unknown_bits() {
                    return Err("expression contains unknown bits".to_string());
                }
                return Ok(value.as_bigint(value.signed));
            }

            if matches!(
                op,
                BinaryOp::LogicalShiftLeft
                    | BinaryOp::LogicalShiftRight
                    | BinaryOp::ArithmeticShiftLeft
                    | BinaryOp::ArithmeticShiftRight
            ) {
                // Same reasoning as bitwise: shifts depend on the LHS width
                // and signedness, so route through the standard pipeline
                // rather than reaching into bigint shift operators directly.
                let value = evaluate_binary_expr(*op, lhs, rhs, None)?;
                if value.has_unknown_bits() {
                    return Err("expression contains unknown bits".to_string());
                }
                return Ok(value.as_bigint(value.signed));
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
                | BinaryOp::GreaterThanOrEqual
                | BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::CaseEqual
                | BinaryOp::CaseNotEqual
                | BinaryOp::LogicalAnd
                | BinaryOp::LogicalOr
                | BinaryOp::BitwiseAnd
                | BinaryOp::BitwiseOr
                | BinaryOp::BitwiseXor
                | BinaryOp::BitwiseXnor
                | BinaryOp::LogicalShiftLeft
                | BinaryOp::LogicalShiftRight
                | BinaryOp::ArithmeticShiftLeft
                | BinaryOp::ArithmeticShiftRight => unreachable!("handled above"),
            }
        }
        // The result depends on width-aware extension of then/else and on
        // a per-bit merge under an x/z cond, so route through the standard
        // pipeline rather than reaching into bigint directly.
        Expr::Conditional { .. } => {
            let value = evaluate_expr_in_context(expr, None)?;
            if value.has_unknown_bits() {
                return Err("expression contains unknown bits".to_string());
            }
            Ok(value.as_bigint(value.signed))
        }
        // Concatenation/replication results are bit-pattern values — read
        // them as unsigned integers (LRM 5.1.14: "The result of a
        // concatenation is treated as an unsigned vector").
        Expr::Concatenation { .. } | Expr::Replication { .. } => {
            let value = evaluate_expr_in_context(expr, None)?;
            if value.has_unknown_bits() {
                return Err("expression contains unknown bits".to_string());
            }
            Ok(value.as_bigint(false))
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

// LRM 5.1.10 4-state truth tables. Verified against
// `doc/four_value_ops_output.txt` (iverilog).

fn bitwise_not_bit(a: LogicBit) -> LogicBit {
    match a {
        LogicBit::Zero => LogicBit::One,
        LogicBit::One => LogicBit::Zero,
        LogicBit::X | LogicBit::Z => LogicBit::X,
    }
}

fn bitwise_and_bits(a: LogicBit, b: LogicBit) -> LogicBit {
    // A definite 0 dominates, even against x/z. Otherwise any unknown poisons
    // the bit; only 1 & 1 yields 1.
    match (a, b) {
        (LogicBit::Zero, _) | (_, LogicBit::Zero) => LogicBit::Zero,
        (LogicBit::One, LogicBit::One) => LogicBit::One,
        _ => LogicBit::X,
    }
}

fn bitwise_or_bits(a: LogicBit, b: LogicBit) -> LogicBit {
    // Symmetric to AND with 1 dominating. 0 | 0 is the only definite-0 case.
    match (a, b) {
        (LogicBit::One, _) | (_, LogicBit::One) => LogicBit::One,
        (LogicBit::Zero, LogicBit::Zero) => LogicBit::Zero,
        _ => LogicBit::X,
    }
}

fn bitwise_xor_bits(a: LogicBit, b: LogicBit) -> LogicBit {
    // XOR has no dominator: any x/z makes the bit ambiguous.
    match (a, b) {
        (LogicBit::X | LogicBit::Z, _) | (_, LogicBit::X | LogicBit::Z) => LogicBit::X,
        (LogicBit::Zero, LogicBit::Zero) | (LogicBit::One, LogicBit::One) => LogicBit::Zero,
        _ => LogicBit::One,
    }
}

fn bitwise_xnor_bits(a: LogicBit, b: LogicBit) -> LogicBit {
    bitwise_not_bit(bitwise_xor_bits(a, b))
}

// LRM 5.1.11 reduction: fold the binary operator across all operand bits.
// Identity element matches the operator (AND uses 1; OR and XOR use 0);
// the negated forms NAND/NOR/XNOR invert the fold result. Reusing the
// binary truth tables from Phase 6a keeps x/z propagation identical: e.g.
// AND-reduction still gives 0 when any bit is 0 (even with x/z elsewhere),
// because `bitwise_and_bits(0, x)` returns 0.
fn reduce_bits(op: UnaryOp, bits: &[LogicBit]) -> LogicBit {
    let folded = match op {
        UnaryOp::ReductionAnd | UnaryOp::ReductionNand => bits
            .iter()
            .copied()
            .fold(LogicBit::One, bitwise_and_bits),
        UnaryOp::ReductionOr | UnaryOp::ReductionNor => bits
            .iter()
            .copied()
            .fold(LogicBit::Zero, bitwise_or_bits),
        UnaryOp::ReductionXor | UnaryOp::ReductionXnor => bits
            .iter()
            .copied()
            .fold(LogicBit::Zero, bitwise_xor_bits),
        _ => unreachable!("reduce_bits called with non-reduction op"),
    };
    match op {
        UnaryOp::ReductionNand | UnaryOp::ReductionNor | UnaryOp::ReductionXnor => {
            bitwise_not_bit(folded)
        }
        _ => folded,
    }
}

fn is_reduction_op(op: UnaryOp) -> bool {
    matches!(
        op,
        UnaryOp::ReductionAnd
            | UnaryOp::ReductionNand
            | UnaryOp::ReductionOr
            | UnaryOp::ReductionNor
            | UnaryOp::ReductionXor
            | UnaryOp::ReductionXnor
    )
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

    // LRM Table 5-22 footnote a (iverilog-confirmed): an unsized x/z-leading
    // constant in an expression wider than 32 bits extends by the MSB
    // regardless of the propagated context signedness. Sized x/z operands
    // still follow §5.5.4 (zero-fill in unsigned propagated context).
    #[test]
    fn unsized_x_literal_msb_extends_in_wider_unsigned_context() {
        let bitwise = evaluate_input("'bx | 64'b0").expect("expression should evaluate");
        assert_eq!(
            bitwise.output,
            "64'bxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
        );

        let case_eq = evaluate_input("'bx === 64'bx").expect("expression should evaluate");
        assert_eq!(case_eq.output, "1'b1");
    }

    #[test]
    fn unsized_x_literal_msb_extends_regardless_of_mixed_signedness() {
        let unsigned_unsized_signed_sized =
            evaluate_input("'bx | 64'sb0").expect("expression should evaluate");
        assert_eq!(
            unsigned_unsized_signed_sized.output,
            "64'bxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
        );

        let signed_unsized_unsigned_sized =
            evaluate_input("'sbx | 64'b0").expect("expression should evaluate");
        assert_eq!(
            signed_unsized_unsigned_sized.output,
            "64'bxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
        );
    }

    #[test]
    fn unsized_signed_literal_sign_extends_per_own_signedness_in_unsigned_context() {
        // 'shFFFFFFFF is signed with MSB=1 at the 32-bit default. Per footnote
        // a's "Otherwise" branch, the own (signed) signedness drives a sign-
        // extend even though the propagated context is unsigned. §5.5.4 would
        // instead zero-extend and yield 64'h00000000FFFFFFFF.
        let evaluation =
            evaluate_input("'shFFFFFFFF | 64'b0").expect("expression should evaluate");
        assert_eq!(evaluation.output, "64'hffffffffffffffff");
    }

    #[test]
    fn outer_context_propagates_to_unsized_leaf_through_inner_expression() {
        let nested = evaluate_input("('bx | 4'b0) | 64'b0").expect("expression should evaluate");
        assert_eq!(
            nested.output,
            "64'bxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
        );

        // Without the outer 64-bit context the inner expression stays at its
        // own self-determined max(32, 4) = 32 bits.
        let alone = evaluate_input("('bx | 4'b0)").expect("expression should evaluate");
        assert_eq!(alone.output, "32'bxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");
    }

    #[test]
    fn sized_operands_still_follow_propagated_context_extension() {
        // 32'sbx is sized signed with MSB=x. Mixed with 34'b0 (unsigned) the
        // propagated context is unsigned, so §5.5.4 zero-fills the two extra
        // MSB positions even though the operand's MSB is x.
        let mixed = evaluate_input("32'sbx | 34'b0").expect("expression should evaluate");
        assert_eq!(mixed.output, "34'b00xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");

        // Both signed → propagated signed → MSB-fill carries x up.
        let both_signed = evaluate_input("32'sbx | 34'sb0").expect("expression should evaluate");
        assert_eq!(
            both_signed.output,
            "34'sbxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
        );
    }

    #[test]
    fn unsized_value_literals_unchanged_in_wider_context() {
        // Sanity: value (non-x/z) literals should produce the same bits
        // whether we eager-size at 32 + §5.5.4 extend, or footnote-a extend.
        let unsigned_hex =
            evaluate_input("'h7FFFFFFF | 64'b0").expect("expression should evaluate");
        assert_eq!(unsigned_hex.output, "64'h000000007fffffff");

        let signed_decimal = evaluate_input("42 + 64'sb0").expect("expression should evaluate");
        assert_eq!(signed_decimal.output, "64'sd42");
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
        // -4'sd1 propagates 8-bit unsigned context to the inner 4'sd1, which
        // zero-extends to 0000_0001; negation at 8-bit unsigned yields 255.
        let neg_one_gt_zero_widened = evaluate_input("-4'sd1 > 8'd0").expect("widened");
        // 4'sd2 zero-extends (unsigned context) to 0000_0010 = 2; 2 > 5 false.
        let two_not_gt_five = evaluate_input("4'sd2 > 8'd5").expect("not gt");

        assert_eq!(neg_one_gt_zero.output, "1'b1");
        assert_eq!(neg_one_gt_zero_widened.output, "1'b1");
        assert_eq!(two_not_gt_five.output, "1'b0");
    }

    #[test]
    fn mixed_signedness_zero_extends_signed_primary_per_lrm_5_5_2() {
        // LRM §5.1.7 + §5.5.2: when one operand is unsigned the propagated
        // type is unsigned, so the narrower signed primary is ZERO-extended,
        // not sign-extended-then-reinterpreted. The buggy "extend with own
        // signedness, then reinterpret as unsigned" model would flip these
        // answers. Verified against iverilog.
        //
        //   4'sb1111 < 8'd255  →  zero-ext 1111 → 0000_1111 = 15;  15 < 255 → 1
        //   4'sb1000 > 8'd7    →  zero-ext 1000 → 0000_1000 = 8;    8 >  7  → 1
        //   4'sb1000 < 8'd9    →  zero-ext 1000 → 0000_1000 = 8;    8 <  9  → 1
        //   4'sb1111 < 8'd16   →  zero-ext 1111 → 0000_1111 = 15;  15 < 16  → 1
        let lt_big = evaluate_input("4'sb1111 < 8'd255").expect("lt_big");
        let gt_small = evaluate_input("4'sb1000 > 8'd7").expect("gt_small");
        let lt_small = evaluate_input("4'sb1000 < 8'd9").expect("lt_small");
        let lt_sixteen = evaluate_input("4'sb1111 < 8'd16").expect("lt_sixteen");

        assert_eq!(lt_big.output, "1'b1");
        assert_eq!(gt_small.output, "1'b1");
        assert_eq!(lt_small.output, "1'b1");
        assert_eq!(lt_sixteen.output, "1'b1");
    }

    #[test]
    fn unary_minus_propagates_unsigned_context_through_to_primary() {
        // LRM §5.5.2: propagation passes through context-determined unary `-`
        // down to the leaf primary. For `-4'sb1000 < 8'd9` the inner 4'sb1000
        // zero-extends to 0000_1000 = 8 (unsigned context); negation at 8-bit
        // unsigned wraps to 256-8 = 248; 248 < 9 → 0.
        //
        // The "evaluate -4'sb1000 self-determined first then sign-extend"
        // model would give 8 < 9 → 1, which iverilog disagrees with.
        let lt = evaluate_input("-4'sb1000 < 8'd9").expect("unary lt");
        let gt = evaluate_input("-4'sb1000 > 8'd9").expect("unary gt");
        let lt_close = evaluate_input("-4'sb1000 < 8'd249").expect("248 < 249");

        assert_eq!(lt.output, "1'b0");
        assert_eq!(gt.output, "1'b1");
        assert_eq!(lt_close.output, "1'b1");
    }

    #[test]
    fn mixed_signedness_relational_compatible_with_iverilog_neg_one_widened() {
        // Both models agree here (the buggy "sign-extend then reinterpret"
        // path happens to coincide with the LRM-correct "propagate down,
        // negate at wider width" path because 4'sd1 has MSB=0). Kept as a
        // regression guard against future propagation changes.
        let gt = evaluate_input("-4'sd1 > 8'd16").expect("> case");
        let lt = evaluate_input("-4'sd1 < 8'd16").expect("< case");

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

    // ---------- Equality operators (==, !=, ===, !==) ----------
    //
    // All expected values in this section were verified against iverilog
    // (Icarus Verilog) and match the LRM 1364-2005 §5.1.8 + §5.5.2 rules:
    //   * Operand unification follows the same shared-context model as
    //     relational ops (max width; signed iff both signed; extension at the
    //     leaf primary uses the propagated signedness).
    //   * `==`/`!=` return 1'bx only when the relation is *ambiguous* — a
    //     definite bit mismatch (0 vs 1) makes operands unequal regardless
    //     of x/z elsewhere.
    //   * `===`/`!==` compare bit-for-bit including x and z; result is always
    //     a known 0 or 1, never x.

    #[test]
    fn evaluates_basic_equality_operators() {
        let eq_true = evaluate_input("4'd3 == 4'd3").expect("eq true");
        let eq_false = evaluate_input("4'd3 == 4'd5").expect("eq false");
        let ne_true = evaluate_input("4'd3 != 4'd5").expect("ne true");
        let ne_false = evaluate_input("4'd3 != 4'd3").expect("ne false");
        let case_eq = evaluate_input("4'd3 === 4'd3").expect("case eq");
        let case_ne = evaluate_input("4'd3 !== 4'd5").expect("case ne");

        assert_eq!(eq_true.output, "1'b1");
        assert_eq!(eq_false.output, "1'b0");
        assert_eq!(ne_true.output, "1'b1");
        assert_eq!(ne_false.output, "1'b0");
        assert_eq!(case_eq.output, "1'b1");
        assert_eq!(case_ne.output, "1'b1");
    }

    #[test]
    fn equality_zero_extends_signed_primary_in_mixed_context() {
        // 4'sb1111 zero-extends (unsigned context) to 0000_1111 = 15;
        // RHS 8'hFF = 255; not equal → == 0, != 1.
        let eq = evaluate_input("4'sb1111 == 8'hFF").expect("eq");
        let ne = evaluate_input("4'sb1111 != 8'hFF").expect("ne");
        // 4'sb1000 zero-extends to 0000_1000 = 8; RHS 8'hF8 = 248 → not equal
        let eq8 = evaluate_input("4'sb1000 == 8'hF8").expect("eq8");
        // Positive signed primary: 4'sb0001 zero-extends to 1; equals 8'd1.
        let eq_pos = evaluate_input("4'sb0001 == 8'd1").expect("eq_pos");

        assert_eq!(eq.output, "1'b0");
        assert_eq!(ne.output, "1'b1");
        assert_eq!(eq8.output, "1'b0");
        assert_eq!(eq_pos.output, "1'b1");
    }

    #[test]
    fn equality_unary_minus_changes_extension_outcome_with_same_bits() {
        // Three cases that look almost identical but expose the LRM §5.5.2
        // propagation rule clearly. The bit pattern `1111` shows up in all
        // three, but the surrounding context decides whether it ends up as
        // 15 or as 255 in an 8-bit comparison. Iverilog-confirmed.
        //
        //   -4'sh1 == 4'shF : both 4-bit signed, no extension; bits
        //                     `1111` == `1111`             → 1
        //   -4'sh1 == 8'hFF : mixed → 8-bit unsigned context. Propagation
        //                     passes through unary `-` to the primary
        //                     `4'sh1` = `0001`; zero-extends to
        //                     `0000_0001` = 1; negate at 8-bit unsigned
        //                     → `1111_1111` = 255. 255 == 255 → 1
        //   4'shF  == 8'hFF : same mixed context, but no unary `-`, so
        //                     the primary `4'shF` = `1111` zero-extends
        //                     directly to `0000_1111` = 15. 15 ≠ 255 → 0
        let neg_same_width = evaluate_input("-4'sh1 == 4'shF").expect("neg same width");
        let neg_widened = evaluate_input("-4'sh1 == 8'hFF").expect("neg widened");
        let no_neg_widened = evaluate_input("4'shF == 8'hFF").expect("no neg widened");

        assert_eq!(neg_same_width.output, "1'b1");
        assert_eq!(neg_widened.output, "1'b1");
        assert_eq!(no_neg_widened.output, "1'b0");
    }

    #[test]
    fn equality_unary_minus_propagates_unsigned_context_to_primary() {
        // -4'sb1000 in 8-bit unsigned context: inner 4'sb1000 zero-extends to
        // 0000_1000 = 8; negate at 8-bit unsigned → 256-8 = 248. 248 == 248 → 1.
        let eq = evaluate_input("-4'sb1000 == 8'd248").expect("neg eq");
        // Same mechanism: -4'sd1 in 8-bit unsigned context becomes 255.
        let neg_one = evaluate_input("-4'sd1 == 8'hFF").expect("neg one eq");

        assert_eq!(eq.output, "1'b1");
        assert_eq!(neg_one.output, "1'b1");
    }

    #[test]
    fn equality_both_signed_uses_sign_extension() {
        // Both signed → context signed → narrower side sign-extends.
        // -4'sd1 sign-extends to 8-bit -1 = 1111_1111; -8'sd1 same bits → equal.
        let neg_neg = evaluate_input("-4'sd1 == -8'sd1").expect("neg neg");
        // === on identical signed bit patterns → 1.
        let case_neg = evaluate_input("4'sb1111 === 4'sb1111").expect("case neg");

        assert_eq!(neg_neg.output, "1'b1");
        assert_eq!(case_neg.output, "1'b1");
    }

    #[test]
    fn logical_equality_returns_x_only_when_ambiguous() {
        // All-x: nothing definite to mismatch on → ambiguous → x.
        let all_x = evaluate_input("4'bx == 4'd1").expect("all x");
        // RHS all-z: same reasoning → x.
        let all_z = evaluate_input("4'd0 == 4'bz").expect("all z");
        // Identical bit pattern with one x: also ambiguous → x.
        let same_x = evaluate_input("4'b01x0 == 4'b01x0").expect("same with x");
        // Definite mismatch elsewhere (bit[2]: 1 vs 0) makes operands
        // unequal regardless of the x bit → != is 1, not x. iverilog confirmed.
        let definite_mismatch_eq = evaluate_input("4'b01x0 == 4'd1").expect("definite mismatch ==");
        let definite_mismatch_ne = evaluate_input("4'b01x0 != 4'd1").expect("definite mismatch !=");
        // No definite mismatch, only an x at bit[0] → ambiguous → x.
        let ambiguous_eq = evaluate_input("4'b101x == 4'b1010").expect("ambiguous ==");
        let ambiguous_ne = evaluate_input("4'b101x != 4'b1010").expect("ambiguous !=");

        assert_eq!(all_x.output, "1'bx");
        assert_eq!(all_z.output, "1'bx");
        assert_eq!(same_x.output, "1'bx");
        assert_eq!(definite_mismatch_eq.output, "1'b0");
        assert_eq!(definite_mismatch_ne.output, "1'b1");
        assert_eq!(ambiguous_eq.output, "1'bx");
        assert_eq!(ambiguous_ne.output, "1'bx");
    }

    #[test]
    fn case_equality_matches_x_and_z_literally() {
        // === requires bit-for-bit identity including x and z; result never x.
        let xxxx_eq = evaluate_input("4'bxxxx === 4'bxxxx").expect("xxxx eq");
        let mixed_eq = evaluate_input("4'bx101 === 4'bx101").expect("mixed eq");
        let mixed_ne_diff = evaluate_input("4'bx101 !== 4'bx100").expect("mixed ne diff");
        let xxxx_vs_zero = evaluate_input("4'bxxxx === 4'd0").expect("xxxx vs zero");
        let x_vs_one = evaluate_input("4'bx101 === 4'b1101").expect("x vs one");
        let zzzz_eq = evaluate_input("4'bzzzz === 4'bzzzz").expect("zzzz eq");
        let xz_pattern = evaluate_input("4'bxzxz === 4'bxzxz").expect("xz pattern");
        let same_ne = evaluate_input("4'bxxxx !== 4'bxxxx").expect("same !==");

        assert_eq!(xxxx_eq.output, "1'b1");
        assert_eq!(mixed_eq.output, "1'b1");
        assert_eq!(mixed_ne_diff.output, "1'b1");
        assert_eq!(xxxx_vs_zero.output, "1'b0");
        assert_eq!(x_vs_one.output, "1'b0");
        assert_eq!(zzzz_eq.output, "1'b1");
        assert_eq!(xz_pattern.output, "1'b1");
        assert_eq!(same_ne.output, "1'b0");
    }

    #[test]
    fn case_equality_extends_unsigned_with_zero_not_x() {
        // LRM 5.5.4: x/z fill on extension applies only to SIGNED resize.
        // For mixed signedness (unsigned context) the narrower side
        // zero-extends regardless of MSB, so 4'bx101 becomes 0000_x101.
        let zero_filled = evaluate_input("4'bx101 === 8'b0000x101").expect("zero filled");
        let not_x_filled = evaluate_input("4'bx101 === 8'bxxxxx101").expect("not x filled");
        // Same for z.
        let z_zero_filled = evaluate_input("4'bz101 === 8'b0000z101").expect("z zero filled");
        let z_not_z_filled = evaluate_input("4'bz101 === 8'bzzzzz101").expect("z not z filled");

        assert_eq!(zero_filled.output, "1'b1");
        assert_eq!(not_x_filled.output, "1'b0");
        assert_eq!(z_zero_filled.output, "1'b1");
        assert_eq!(z_not_z_filled.output, "1'b0");
    }

    #[test]
    fn case_equality_signed_extends_msb_with_x_or_z() {
        // LRM 5.5.4: when BOTH operands are signed (context signed), an x or
        // z MSB does propagate into the upper bits.
        let signed_x_fill =
            evaluate_input("4'sbx000 === 8'sbxxxxx000").expect("signed x fills");
        let signed_zero_fill_wrong =
            evaluate_input("4'sbx000 === 8'sb0000x000").expect("signed zero would be wrong");

        assert_eq!(signed_x_fill.output, "1'b1");
        assert_eq!(signed_zero_fill_wrong.output, "1'b0");
    }

    #[test]
    fn equality_lower_precedence_than_relational() {
        // LRM 5.1.8: equality is lower precedence than relational. So
        // `4'd1 < 4'd2 == 4'd1` parses as `(4'd1 < 4'd2) == 4'd1`, which is
        // 1'b1 == 4'd1 → 1 == 1 → 1. The other grouping would yield 0.
        let result = evaluate_input("4'd1 < 4'd2 == 4'd1").expect("precedence");
        assert_eq!(result.output, "1'b1");
    }

    #[test]
    fn equality_is_left_associative() {
        // 4'd1 == 4'd1 == 4'd1 → (1 == 1) == 4'd1 → 1'b1 == 4'd1 → 1 == 1 → 1
        let result = evaluate_input("4'd1 == 4'd1 == 4'd1").expect("assoc");
        let expr = parse_expression("4'd1 == 4'd1 == 4'd1").expect("parse assoc");
        match expr {
            Expr::Binary {
                op: BinaryOp::Equal,
                lhs,
                ..
            } => assert!(matches!(*lhs, Expr::Binary { op: BinaryOp::Equal, .. })),
            other => panic!("expected top-level ==, got {other:?}"),
        }
        assert_eq!(result.output, "1'b1");
    }

    #[test]
    fn equality_result_widens_to_outer_arithmetic_context() {
        // (4'd3 == 4'd3) → 1'b1; outer + widens result to 4 bits.
        let result = evaluate_input("(4'd3 == 4'd3) + 4'd0").expect("widened");
        assert_eq!(result.output, "4'b0001");
    }

    // ---------- Logical operators (!, &&, ||) ----------
    //
    // All expected values were checked against the iverilog-generated truth
    // tables in `doc/four_value_ops_output.txt`, which encode LRM 1364-2005
    // §5.1.9 Table 5-7. Operands of !, &&, || are self-determined (LRM §5.4
    // Table 5-22) — each operand is reduced to a 1-bit logical value before
    // the truth table applies, so width unification is irrelevant.

    #[test]
    fn tokenizes_logical_operators_as_single_tokens() {
        let and = tokenize("4'd1 && 4'd0").expect("&& should tokenize");
        let or = tokenize("4'd1 || 4'd0").expect("|| should tokenize");
        let bang = tokenize("!4'd0").expect("! should tokenize");

        assert_eq!(and[1], Token::LogicalAnd);
        assert_eq!(or[1], Token::LogicalOr);
        assert_eq!(bang[0], Token::Bang);
    }

    #[test]
    fn evaluates_logical_not_truth_table() {
        let not_zero = evaluate_input("!1'b0").expect("!0");
        let not_one = evaluate_input("!1'b1").expect("!1");
        let not_x = evaluate_input("!1'bx").expect("!x");
        let not_z = evaluate_input("!1'bz").expect("!z");

        assert_eq!(not_zero.output, "1'b1");
        assert_eq!(not_one.output, "1'b0");
        assert_eq!(not_x.output, "1'bx");
        assert_eq!(not_z.output, "1'bx");
    }

    #[test]
    fn logical_not_reduces_across_operand_width() {
        // Any 1 bit makes the operand definitely true; all-zero is false; an x
        // or z with no 1 bit is ambiguous → x. A 1 bit defeats x in the
        // reduction, so 4'b01x0 → false, not x.
        let not_five = evaluate_input("!4'd5").expect("!5");
        let not_zero8 = evaluate_input("!8'd0").expect("!8'd0");
        let not_x_only = evaluate_input("!4'b00x0").expect("!00x0");
        let not_one_with_x = evaluate_input("!4'b01x0").expect("!01x0");

        assert_eq!(not_five.output, "1'b0");
        assert_eq!(not_zero8.output, "1'b1");
        assert_eq!(not_x_only.output, "1'bx");
        assert_eq!(not_one_with_x.output, "1'b0");
    }

    #[test]
    fn evaluates_logical_and_truth_table() {
        // Table 5-7 cases including the "0 dominates x" and "1 && 1 = 1" rows.
        let true_and_true = evaluate_input("4'd1 && 4'd1").expect("1&&1");
        let true_and_false = evaluate_input("4'd5 && 4'd0").expect("5&&0");
        let false_and_true = evaluate_input("4'd0 && 4'd5").expect("0&&5");
        let false_and_x = evaluate_input("4'd0 && 4'bx").expect("0&&x");
        let x_and_false = evaluate_input("4'bx && 4'd0").expect("x&&0");
        let x_and_true = evaluate_input("4'bx && 4'd1").expect("x&&1");
        let x_and_x = evaluate_input("4'bx && 4'bx").expect("x&&x");

        assert_eq!(true_and_true.output, "1'b1");
        assert_eq!(true_and_false.output, "1'b0");
        assert_eq!(false_and_true.output, "1'b0");
        assert_eq!(false_and_x.output, "1'b0");
        assert_eq!(x_and_false.output, "1'b0");
        assert_eq!(x_and_true.output, "1'bx");
        assert_eq!(x_and_x.output, "1'bx");
    }

    #[test]
    fn evaluates_logical_or_truth_table() {
        let true_or_false = evaluate_input("4'd1 || 4'd0").expect("1||0");
        let false_or_false = evaluate_input("4'd0 || 4'd0").expect("0||0");
        let false_or_true = evaluate_input("4'd0 || 4'd5").expect("0||5");
        let true_or_x = evaluate_input("4'd1 || 4'bx").expect("1||x");
        let x_or_true = evaluate_input("4'bx || 4'd1").expect("x||1");
        let x_or_false = evaluate_input("4'bx || 4'd0").expect("x||0");
        let x_or_x = evaluate_input("4'bx || 4'bx").expect("x||x");

        assert_eq!(true_or_false.output, "1'b1");
        assert_eq!(false_or_false.output, "1'b0");
        assert_eq!(false_or_true.output, "1'b1");
        assert_eq!(true_or_x.output, "1'b1");
        assert_eq!(x_or_true.output, "1'b1");
        assert_eq!(x_or_false.output, "1'bx");
        assert_eq!(x_or_x.output, "1'bx");
    }

    #[test]
    fn logical_result_renders_in_binary_regardless_of_operand_base() {
        // Operands hex but the 1-bit logical result is binary, like
        // relational/equality.
        let hex_and = evaluate_input("8'h0a && 8'h0f").expect("hex &&");
        let hex_or = evaluate_input("8'h00 || 8'h0f").expect("hex ||");
        let hex_not = evaluate_input("!8'h0a").expect("hex !");

        assert_eq!(hex_and.output, "1'b1");
        assert_eq!(hex_or.output, "1'b1");
        assert_eq!(hex_not.output, "1'b0");
    }

    #[test]
    fn logical_not_binds_tighter_than_power() {
        // LRM Table 5-4: unary operators (including !) are higher precedence
        // than **. So `!4'd0 ** 4'd2` parses as `(!4'd0) ** 4'd2` → 1**2 → 1.
        let expr = parse_expression("!4'd0 ** 4'd2").expect("parse");
        match expr {
            Expr::Binary {
                op: BinaryOp::Power,
                lhs,
                ..
            } => assert!(matches!(
                *lhs,
                Expr::Unary {
                    op: UnaryOp::LogicalNot,
                    ..
                }
            )),
            other => panic!("expected top-level **, got {other:?}"),
        }
        let result = evaluate_input("!4'd0 ** 4'd2").expect("eval");
        assert_eq!(result.output, "1'b1");
    }

    #[test]
    fn logical_and_lower_precedence_than_equality() {
        // `4'd0 == 4'd0 && 4'd1` parses as `(4'd0 == 4'd0) && 4'd1`.
        let expr = parse_expression("4'd0 == 4'd0 && 4'd1").expect("parse");
        match expr {
            Expr::Binary {
                op: BinaryOp::LogicalAnd,
                lhs,
                ..
            } => assert!(matches!(*lhs, Expr::Binary { op: BinaryOp::Equal, .. })),
            other => panic!("expected top-level &&, got {other:?}"),
        }
        let result = evaluate_input("4'd0 == 4'd0 && 4'd1").expect("eval");
        assert_eq!(result.output, "1'b1");
    }

    #[test]
    fn logical_or_lower_precedence_than_logical_and() {
        // `4'd1 || 4'd0 && 4'd0` parses as `4'd1 || (4'd0 && 4'd0)` → 1.
        let expr = parse_expression("4'd1 || 4'd0 && 4'd0").expect("parse");
        match expr {
            Expr::Binary {
                op: BinaryOp::LogicalOr,
                rhs,
                ..
            } => assert!(matches!(
                *rhs,
                Expr::Binary {
                    op: BinaryOp::LogicalAnd,
                    ..
                }
            )),
            other => panic!("expected top-level ||, got {other:?}"),
        }
        let result = evaluate_input("4'd1 || 4'd0 && 4'd0").expect("eval");
        assert_eq!(result.output, "1'b1");
    }

    #[test]
    fn logical_not_chains_recursively() {
        // !! parses as `!(!x)` because `!` is right-associative through
        // the recursive parse_unary; it also lets us test that the inner
        // 1'b0 from `!4'd5` is correctly fed back into `!`.
        let result = evaluate_input("!!4'd5").expect("!!5");
        let zero = evaluate_input("!!4'd0").expect("!!0");

        assert_eq!(result.output, "1'b1");
        assert_eq!(zero.output, "1'b0");
    }

    #[test]
    fn logical_and_is_left_associative() {
        // a && b && c parses as (a && b) && c; same shape check as the
        // existing equality_is_left_associative test.
        let expr = parse_expression("4'd1 && 4'd1 && 4'd1").expect("parse");
        match expr {
            Expr::Binary {
                op: BinaryOp::LogicalAnd,
                lhs,
                ..
            } => assert!(matches!(
                *lhs,
                Expr::Binary {
                    op: BinaryOp::LogicalAnd,
                    ..
                }
            )),
            other => panic!("expected top-level &&, got {other:?}"),
        }
        let result = evaluate_input("4'd1 && 4'd1 && 4'd0").expect("eval");
        assert_eq!(result.output, "1'b0");
    }

    #[test]
    fn logical_or_is_left_associative() {
        let expr = parse_expression("4'd0 || 4'd0 || 4'd1").expect("parse");
        match expr {
            Expr::Binary {
                op: BinaryOp::LogicalOr,
                lhs,
                ..
            } => assert!(matches!(
                *lhs,
                Expr::Binary {
                    op: BinaryOp::LogicalOr,
                    ..
                }
            )),
            other => panic!("expected top-level ||, got {other:?}"),
        }
        let result = evaluate_input("4'd0 || 4'd0 || 4'd1").expect("eval");
        assert_eq!(result.output, "1'b1");
    }

    #[test]
    fn logical_result_widens_to_outer_arithmetic_context() {
        // (4'd1 && 4'd1) → 1'b1; outer + widens to 4 bits and inherits the
        // leftmost operand's binary base (the && result's base).
        let result = evaluate_input("(4'd1 && 4'd1) + 4'd0").expect("widened &&");
        let or_widened = evaluate_input("(4'd0 || 4'd0) + 4'd0").expect("widened ||");
        let not_widened = evaluate_input("(!4'd0) + 4'd0").expect("widened !");

        assert_eq!(result.output, "4'b0001");
        assert_eq!(or_widened.output, "4'b0000");
        assert_eq!(not_widened.output, "4'b0001");
    }

    // ---------- Bitwise operators (~, &, |, ^, ~^/^~) ----------
    //
    // All expected values are checked against the iverilog-generated truth
    // tables in `doc/four_value_ops_output.txt`, which encode LRM 1364-2005
    // §5.1.10 Tables 5-9..5-12. Per LRM Table 5-22, per-bit `~` and the
    // binary forms are context-determined like arithmetic: width =
    // max(L(lhs), L(rhs)), signed iff both operands signed.

    #[test]
    fn tokenizes_bitwise_single_char_operators() {
        // Bare & and | are no longer rejected: they tokenize as their
        // bitwise forms when not followed by a second &/|.
        let amp = tokenize("4'd1 & 4'd0").expect("& should tokenize");
        let pipe = tokenize("4'd1 | 4'd0").expect("| should tokenize");
        let xor = tokenize("4'd1 ^ 4'd0").expect("^ should tokenize");
        let tilde = tokenize("~4'd0").expect("~ should tokenize");

        assert_eq!(amp[1], Token::BitwiseAnd);
        assert_eq!(pipe[1], Token::BitwiseOr);
        assert_eq!(xor[1], Token::BitwiseXor);
        assert_eq!(tilde[0], Token::Tilde);
    }

    #[test]
    fn tokenizes_xnor_with_either_spelling() {
        // LRM 5.1.10: `^~` and `~^` denote the same operator; both lex to a
        // single BitwiseXnor token so downstream code does not branch on
        // spelling.
        let tilde_caret = tokenize("4'd1 ~^ 4'd0").expect("~^ should tokenize");
        let caret_tilde = tokenize("4'd1 ^~ 4'd0").expect("^~ should tokenize");

        assert_eq!(tilde_caret[1], Token::BitwiseXnor);
        assert_eq!(caret_tilde[1], Token::BitwiseXnor);
    }

    #[test]
    fn double_amp_and_pipe_still_lex_as_logical() {
        // Greedy two-char matching must win over the bare bitwise tokens;
        // otherwise && would silently become two & tokens.
        let and = tokenize("4'd1 && 4'd0").expect("&& should tokenize");
        let or = tokenize("4'd1 || 4'd0").expect("|| should tokenize");

        assert_eq!(and[1], Token::LogicalAnd);
        assert_eq!(or[1], Token::LogicalOr);
    }

    #[test]
    fn evaluates_bitwise_not_truth_table() {
        let zero = evaluate_input("~1'b0").expect("~0");
        let one = evaluate_input("~1'b1").expect("~1");
        let x = evaluate_input("~1'bx").expect("~x");
        let z = evaluate_input("~1'bz").expect("~z");

        assert_eq!(zero.output, "1'b1");
        assert_eq!(one.output, "1'b0");
        assert_eq!(x.output, "1'bx");
        assert_eq!(z.output, "1'bx");
    }

    #[test]
    fn bitwise_not_flips_each_bit_independently() {
        // Per-bit operation: x and z fold to x; other bits flip. Crucially
        // there is no all-x short-circuit (unlike arithmetic), so known and
        // unknown bits coexist in the result.
        let mixed = evaluate_input("~4'b01xz").expect("~01xz");
        let all_zeros = evaluate_input("~4'b0000").expect("~0000");

        assert_eq!(mixed.output, "4'b10xx");
        assert_eq!(all_zeros.output, "4'b1111");
    }

    #[test]
    fn bitwise_not_preserves_operand_base() {
        let binary = evaluate_input("~4'b0001").expect("binary ~");
        let hex = evaluate_input("~8'h0a").expect("hex ~");

        assert_eq!(binary.output, "4'b1110");
        assert_eq!(hex.output, "8'hf5");
    }

    #[test]
    fn bitwise_not_chains() {
        // parse_unary recurses, so ~~ parses as ~(~x).
        let result = evaluate_input("~~4'b0101").expect("~~0101");
        assert_eq!(result.output, "4'b0101");
    }

    #[test]
    fn bitwise_not_widens_through_outer_arithmetic_context() {
        // Self-determined: ~4'b0001 = 4'b1110. With outer + 0 (32-bit signed
        // 0 makes the shared context 32-bit unsigned), the operand widens to
        // 32 bits BEFORE the negation runs, so we get 32 ones except the
        // LSB. Leftmost-base wins, so result is binary.
        let widened = evaluate_input("~4'b0001 + 0").expect("widened ~");
        assert_eq!(widened.output, "32'b11111111111111111111111111111110");
    }

    #[test]
    fn bitwise_not_binds_tighter_than_power() {
        // LRM Table 5-4: unary ~ is tighter than **, so `~4'd1 ** 2` parses
        // as `(~4'd1) ** 2`. ~4'd1 self-determined at 4-bit unsigned is
        // 4'b1110 = 14; 14**2 = 196; 196 mod 16 = 4. Result base inherits
        // from the lhs (decimal), so 4'd4.
        let expr = parse_expression("~4'd1 ** 2").expect("parse");
        match expr {
            Expr::Binary {
                op: BinaryOp::Power,
                lhs,
                ..
            } => assert!(matches!(
                *lhs,
                Expr::Unary {
                    op: UnaryOp::BitwiseNot,
                    ..
                }
            )),
            other => panic!("expected top-level **, got {other:?}"),
        }
        let result = evaluate_input("~4'd1 ** 2").expect("eval");
        assert_eq!(result.output, "4'd4");
    }

    #[test]
    fn evaluates_bitwise_and_truth_table() {
        // doc/four_value_ops_output.txt: 0 dominates AND, x/z elsewhere → x,
        // only 1&1 yields 1.
        let zero_zero = evaluate_input("1'b0 & 1'b0").expect("0&0");
        let one_one = evaluate_input("1'b1 & 1'b1").expect("1&1");
        let zero_x = evaluate_input("1'b0 & 1'bx").expect("0&x");
        let one_x = evaluate_input("1'b1 & 1'bx").expect("1&x");
        let one_z = evaluate_input("1'b1 & 1'bz").expect("1&z");
        let x_z = evaluate_input("1'bx & 1'bz").expect("x&z");

        assert_eq!(zero_zero.output, "1'b0");
        assert_eq!(one_one.output, "1'b1");
        assert_eq!(zero_x.output, "1'b0");
        assert_eq!(one_x.output, "1'bx");
        assert_eq!(one_z.output, "1'bx");
        assert_eq!(x_z.output, "1'bx");
    }

    #[test]
    fn evaluates_bitwise_or_truth_table() {
        // Symmetric to AND with 1 dominating.
        let zero_zero = evaluate_input("1'b0 | 1'b0").expect("0|0");
        let one_zero = evaluate_input("1'b1 | 1'b0").expect("1|0");
        let one_x = evaluate_input("1'b1 | 1'bx").expect("1|x");
        let zero_x = evaluate_input("1'b0 | 1'bx").expect("0|x");
        let zero_z = evaluate_input("1'b0 | 1'bz").expect("0|z");
        let x_z = evaluate_input("1'bx | 1'bz").expect("x|z");

        assert_eq!(zero_zero.output, "1'b0");
        assert_eq!(one_zero.output, "1'b1");
        assert_eq!(one_x.output, "1'b1");
        assert_eq!(zero_x.output, "1'bx");
        assert_eq!(zero_z.output, "1'bx");
        assert_eq!(x_z.output, "1'bx");
    }

    #[test]
    fn evaluates_bitwise_xor_truth_table() {
        // XOR has no dominator: any x/z anywhere → x. Otherwise standard XOR.
        let zero_one = evaluate_input("1'b0 ^ 1'b1").expect("0^1");
        let one_one = evaluate_input("1'b1 ^ 1'b1").expect("1^1");
        let zero_zero = evaluate_input("1'b0 ^ 1'b0").expect("0^0");
        let one_x = evaluate_input("1'b1 ^ 1'bx").expect("1^x");
        let zero_z = evaluate_input("1'b0 ^ 1'bz").expect("0^z");

        assert_eq!(zero_one.output, "1'b1");
        assert_eq!(one_one.output, "1'b0");
        assert_eq!(zero_zero.output, "1'b0");
        assert_eq!(one_x.output, "1'bx");
        assert_eq!(zero_z.output, "1'bx");
    }

    #[test]
    fn evaluates_bitwise_xnor_truth_table_with_either_spelling() {
        // ^~ and ~^ are the same operator (NOT-of-XOR semantics).
        let tilde_caret_eq = evaluate_input("1'b0 ~^ 1'b0").expect("0~^0");
        let caret_tilde_eq = evaluate_input("1'b0 ^~ 1'b0").expect("0^~0");
        let one_one = evaluate_input("1'b1 ^~ 1'b1").expect("1^~1");
        let mixed = evaluate_input("1'b1 ~^ 1'b0").expect("1~^0");
        let one_x = evaluate_input("1'b1 ~^ 1'bx").expect("1~^x");

        assert_eq!(tilde_caret_eq.output, "1'b1");
        assert_eq!(caret_tilde_eq.output, "1'b1");
        assert_eq!(one_one.output, "1'b1");
        assert_eq!(mixed.output, "1'b0");
        assert_eq!(one_x.output, "1'bx");
    }

    #[test]
    fn bitwise_binary_zips_known_and_unknown_bits_per_position() {
        // The arithmetic all-x short-circuit does NOT apply: bitwise ops mix
        // known and unknown bits per position. Worked examples (bit 0 = LSB):
        //
        //   4'b1100 & 4'b10x1 → bits: 0&1=0, 0&x=0 (0 dominates), 1&0=0, 1&1=1 → 4'b1000
        //   4'b1100 | 4'b00x1 → bits: 0|1=1, 0|x=x, 1|0=1, 1|0=1            → 4'b11x1
        //   4'b1100 ^ 4'b00x1 → bits: 0^1=1, 0^x=x, 1^0=1, 1^0=1            → 4'b11x1
        let and = evaluate_input("4'b1100 & 4'b10x1").expect("mixed &");
        let or = evaluate_input("4'b1100 | 4'b00x1").expect("mixed |");
        let xor = evaluate_input("4'b1100 ^ 4'b00x1").expect("mixed ^");

        assert_eq!(and.output, "4'b1000");
        assert_eq!(or.output, "4'b11x1");
        assert_eq!(xor.output, "4'b11x1");
    }

    #[test]
    fn bitwise_binary_uses_max_width_of_operands() {
        // Same width: trivially preserved.
        let same = evaluate_input("4'b1100 & 4'b1010").expect("4&4");
        // Mixed width (both unsigned → unsigned context): narrower operand
        // zero-extends to the wider width before zipping.
        //   8'hff = 8'b11111111; 4'b1010 zero-extends to 8'b00001010;
        //   AND → 8'b00001010.
        let mixed = evaluate_input("8'hff & 4'b1010").expect("8&4");

        assert_eq!(same.output, "4'b1000");
        assert_eq!(mixed.output, "8'h0a");
    }

    #[test]
    fn bitwise_binary_signed_only_when_both_signed() {
        // Both signed → context signed → narrower side sign-extends.
        //   4'sb1111 sign-extends to 8'sb11111111;
        //   & 8'sb01010101 → 8'sb01010101.
        let both_signed = evaluate_input("4'sb1111 & 8'sb01010101").expect("both signed");
        // Mixed → context unsigned → narrower zero-extends.
        //   4'sb1111 zero-extends to 8'b00001111;
        //   & 8'b01010101 → 8'b00000101.
        let mixed = evaluate_input("4'sb1111 & 8'b01010101").expect("mixed");

        assert_eq!(both_signed.output, "8'sb01010101");
        assert_eq!(mixed.output, "8'b00000101");
    }

    #[test]
    fn bitwise_extends_per_5_5_2_not_per_5_1_10() {
        // LRM §5.1.10 says the shorter operand "shall be zero-filled in the
        // most significant bit positions", but §5.5.2 says signed-signed
        // operands unify under a signed propagated context and the narrower
        // side sign-extends. The two rules disagree for `4'shF | 8'sh0`.
        // vcal follows §5.5.2 (matching iverilog, VCS, Xcelium, and the
        // SystemVerilog clarification that drops the §5.1.10 sentence):
        //   - both signed → sign-extend the narrower operand.
        //   - any unsigned → unsigned context → zero-extend.
        let both_signed = evaluate_input("4'shF | 8'sh0").expect("both signed");
        let mixed_unsigned = evaluate_input("4'shF | 8'h0").expect("mixed");

        assert_eq!(both_signed.output, "8'shff");
        assert_eq!(mixed_unsigned.output, "8'h0f");
    }

    #[test]
    fn bitwise_binary_widens_through_outer_arithmetic_context() {
        // Without context-widening these would be 4-bit. With outer + 0
        // (32-bit signed 0 produces 32-bit unsigned shared context), the
        // bitwise operands widen to 32 bits BEFORE zipping. Leftmost-base
        // for the outer + is the bitwise op's binary base.
        let widened_and = evaluate_input("(4'b1100 & 4'b1010) + 0").expect("widened &");
        let widened_or = evaluate_input("(4'b0100 | 4'b1010) + 0").expect("widened |");
        let widened_xor = evaluate_input("(4'b0110 ^ 4'b1010) + 0").expect("widened ^");

        assert_eq!(widened_and.output, "32'b00000000000000000000000000001000");
        assert_eq!(widened_or.output, "32'b00000000000000000000000000001110");
        assert_eq!(widened_xor.output, "32'b00000000000000000000000000001100");
    }

    #[test]
    fn bitwise_band_precedence_below_equality() {
        // `4'd1 == 4'd1 & 4'd1` parses as `(4'd1 == 4'd1) & 4'd1`. The 1-bit
        // 1'b1 zero-extends to 4'b0001 under the unified 4-bit context, then
        // & 4'b0001 → 4'b0001.
        let expr = parse_expression("4'd1 == 4'd1 & 4'd1").expect("parse");
        match expr {
            Expr::Binary {
                op: BinaryOp::BitwiseAnd,
                lhs,
                ..
            } => assert!(matches!(*lhs, Expr::Binary { op: BinaryOp::Equal, .. })),
            other => panic!("expected top-level &, got {other:?}"),
        }
        let result = evaluate_input("4'd1 == 4'd1 & 4'd1").expect("eval");
        assert_eq!(result.output, "4'b0001");
    }

    #[test]
    fn bitwise_band_precedence_above_logical_and() {
        // `4'd1 & 4'd1 && 4'd0` parses as `(4'd1 & 4'd1) && 4'd0`.
        let expr = parse_expression("4'd1 & 4'd1 && 4'd0").expect("parse");
        match expr {
            Expr::Binary {
                op: BinaryOp::LogicalAnd,
                lhs,
                ..
            } => assert!(matches!(
                *lhs,
                Expr::Binary {
                    op: BinaryOp::BitwiseAnd,
                    ..
                }
            )),
            other => panic!("expected top-level &&, got {other:?}"),
        }
        let result = evaluate_input("4'd1 & 4'd1 && 4'd0").expect("eval");
        assert_eq!(result.output, "1'b0");
    }

    #[test]
    fn bitwise_internal_precedence_and_tightest_or_loosest() {
        // & > ^ > | per LRM Table 5-4.
        //
        //   4'b0110 ^ 4'b0011 & 4'b1100  →  4'b0110 ^ (4'b0011 & 4'b1100)
        //                                = 4'b0110 ^ 4'b0000 = 4'b0110
        //   4'b1000 | 4'b0001 ^ 4'b1010  →  4'b1000 | (4'b0001 ^ 4'b1010)
        //                                = 4'b1000 | 4'b1011 = 4'b1011
        let and_under_xor = evaluate_input("4'b0110 ^ 4'b0011 & 4'b1100").expect("eval");
        let xor_under_or = evaluate_input("4'b1000 | 4'b0001 ^ 4'b1010").expect("eval");

        assert_eq!(and_under_xor.output, "4'b0110");
        assert_eq!(xor_under_or.output, "4'b1011");
    }

    #[test]
    fn bitwise_binary_is_left_associative() {
        // Same shape check used elsewhere: a OP b OP c parses as (a OP b) OP c.
        let expr = parse_expression("4'd1 & 4'd2 & 4'd3").expect("parse");
        match expr {
            Expr::Binary {
                op: BinaryOp::BitwiseAnd,
                lhs,
                ..
            } => assert!(matches!(
                *lhs,
                Expr::Binary {
                    op: BinaryOp::BitwiseAnd,
                    ..
                }
            )),
            other => panic!("expected top-level &, got {other:?}"),
        }

        // Cross-check XOR chain: (1^2)^3 = 3^3 = 0. Leftmost-base is
        // decimal (all operands `'d`), so the result renders as `4'd0`.
        let xor_chain = evaluate_input("4'd1 ^ 4'd2 ^ 4'd3").expect("eval");
        assert_eq!(xor_chain.output, "4'd0");
    }

    #[test]
    fn bitwise_binary_inherits_leftmost_base() {
        // Same leftmost-wins rule as arithmetic.
        let hex_then_binary = evaluate_input("8'h0a & 8'b00001111").expect("hex&binary");
        let binary_then_hex = evaluate_input("8'b00001111 & 8'h0a").expect("binary&hex");

        assert_eq!(hex_then_binary.output, "8'h0a");
        assert_eq!(binary_then_hex.output, "8'b00001010");
    }

    // ---------- Reduction unary operators (& ~& | ~| ^ ~^/^~) ----------
    //
    // All expected values are checked against the iverilog-generated truth
    // tables in `doc/four_value_ops_output.txt`, which encode LRM 1364-2005
    // §5.1.11. Per LRM Table 5-22, reduction operands are self-determined
    // and the result is always 1-bit unsigned that widens through outer
    // arithmetic context like `!`, `&&`, `||`, relational, equality.

    #[test]
    fn tokenizes_nand_and_nor_as_single_tokens() {
        // ~& and ~| are unary-only operators (LRM A.8.6: binary_operator does
        // not list them). They must lex greedily as one token so the parser
        // can claim them at unary position without re-splitting.
        let nand = tokenize("~&4'b1111").expect("~& should tokenize");
        let nor = tokenize("~|4'b0000").expect("~| should tokenize");

        assert_eq!(nand[0], Token::BitwiseNand);
        assert_eq!(nor[0], Token::BitwiseNor);
    }

    #[test]
    fn bare_tilde_unaffected_by_reduction_lexing() {
        // After the ~&/~|/~^ greedy paths, a bare ~ followed by anything else
        // (whitespace, digit-start, paren) must still produce a Tilde token.
        let spaced = tokenize("~ 4'd1").expect("~ + space");
        let parened = tokenize("~(4'd1)").expect("~(...)");

        assert_eq!(spaced[0], Token::Tilde);
        assert_eq!(parened[0], Token::Tilde);
    }

    #[test]
    fn reduction_nand_nor_rejected_as_binary() {
        // No parse_bitwise_* level consumes BitwiseNand/BitwiseNor, so
        // `a ~& b` cleanly fails after the lhs is reduced to a primary.
        let nand = evaluate_input("4'd1 ~& 4'd1").expect_err("binary ~& rejected");
        let nor = evaluate_input("4'd0 ~| 4'd0").expect_err("binary ~| rejected");

        assert_eq!(nand, "unexpected token after end of expression");
        assert_eq!(nor, "unexpected token after end of expression");
    }

    #[test]
    fn evaluates_reduction_and_single_bit_truth_table() {
        // Single-bit reduction degenerates to identity for known values and
        // x for x/z, matching the &-unary row in four_value_ops_output.txt.
        let zero = evaluate_input("&1'b0").expect("&0");
        let one = evaluate_input("&1'b1").expect("&1");
        let x = evaluate_input("&1'bx").expect("&x");
        let z = evaluate_input("&1'bz").expect("&z");

        assert_eq!(zero.output, "1'b0");
        assert_eq!(one.output, "1'b1");
        assert_eq!(x.output, "1'bx");
        assert_eq!(z.output, "1'bx");
    }

    #[test]
    fn evaluates_reduction_or_single_bit_truth_table() {
        let zero = evaluate_input("|1'b0").expect("|0");
        let one = evaluate_input("|1'b1").expect("|1");
        let x = evaluate_input("|1'bx").expect("|x");
        let z = evaluate_input("|1'bz").expect("|z");

        assert_eq!(zero.output, "1'b0");
        assert_eq!(one.output, "1'b1");
        assert_eq!(x.output, "1'bx");
        assert_eq!(z.output, "1'bx");
    }

    #[test]
    fn evaluates_reduction_xor_single_bit_truth_table() {
        let zero = evaluate_input("^1'b0").expect("^0");
        let one = evaluate_input("^1'b1").expect("^1");
        let x = evaluate_input("^1'bx").expect("^x");
        let z = evaluate_input("^1'bz").expect("^z");

        assert_eq!(zero.output, "1'b0");
        assert_eq!(one.output, "1'b1");
        assert_eq!(x.output, "1'bx");
        assert_eq!(z.output, "1'bx");
    }

    #[test]
    fn evaluates_negated_reduction_single_bit_truth_tables() {
        // The negated forms are NOT-of-positive (per LRM 5.1.11 last
        // sentence). Single-bit cases match the ~&/~|/~^ unary rows in
        // four_value_ops_output.txt: known → flipped, x/z → x.
        let nand_one = evaluate_input("~&1'b1").expect("~&1");
        let nand_zero = evaluate_input("~&1'b0").expect("~&0");
        let nand_x = evaluate_input("~&1'bx").expect("~&x");
        let nor_one = evaluate_input("~|1'b1").expect("~|1");
        let nor_zero = evaluate_input("~|1'b0").expect("~|0");
        let nor_z = evaluate_input("~|1'bz").expect("~|z");
        let xnor_one = evaluate_input("~^1'b1").expect("~^1");
        let xnor_zero = evaluate_input("~^1'b0").expect("~^0");
        let xnor_x = evaluate_input("~^1'bx").expect("~^x");

        assert_eq!(nand_one.output, "1'b0");
        assert_eq!(nand_zero.output, "1'b1");
        assert_eq!(nand_x.output, "1'bx");
        assert_eq!(nor_one.output, "1'b0");
        assert_eq!(nor_zero.output, "1'b1");
        assert_eq!(nor_z.output, "1'bx");
        assert_eq!(xnor_one.output, "1'b0");
        assert_eq!(xnor_zero.output, "1'b1");
        assert_eq!(xnor_x.output, "1'bx");
    }

    #[test]
    fn xnor_reduction_accepts_either_spelling() {
        // ^~ and ~^ are the same operator at unary position too.
        let tilde_caret = evaluate_input("~^4'b1100").expect("~^");
        let caret_tilde = evaluate_input("^~4'b1100").expect("^~");

        // 4'b1100 has two 1s → XOR parity = 0 → XNOR = 1.
        assert_eq!(tilde_caret.output, "1'b1");
        assert_eq!(caret_tilde.output, "1'b1");
    }

    #[test]
    fn reduction_and_folds_multi_bit_operand() {
        // 0 dominates AND-reduction even against x/z (because
        // bitwise_and_bits(0, x) = 0). Otherwise: any x/z → x; all-1 → 1.
        let all_ones = evaluate_input("&4'b1111").expect("&1111");
        let has_zero = evaluate_input("&4'b1101").expect("&1101");
        let zero_dominates_over_x = evaluate_input("&4'b110x").expect("&110x");
        let unknown_no_zero = evaluate_input("&4'b111x").expect("&111x");
        let unknown_no_zero_z = evaluate_input("&4'b111z").expect("&111z");
        let unknown_mixed = evaluate_input("&4'b1x1z").expect("&1x1z");

        assert_eq!(all_ones.output, "1'b1");
        assert_eq!(has_zero.output, "1'b0");
        assert_eq!(zero_dominates_over_x.output, "1'b0");
        assert_eq!(unknown_no_zero.output, "1'bx");
        assert_eq!(unknown_no_zero_z.output, "1'bx");
        assert_eq!(unknown_mixed.output, "1'bx");
    }

    #[test]
    fn reduction_or_folds_multi_bit_operand() {
        // Symmetric: 1 dominates OR-reduction. all-0 → 0; any 1 → 1;
        // otherwise any x/z → x.
        let all_zeros = evaluate_input("|4'b0000").expect("|0000");
        let has_one = evaluate_input("|4'b0010").expect("|0010");
        let one_dominates_over_x = evaluate_input("|4'b001x").expect("|001x");
        let unknown_no_one = evaluate_input("|4'b000x").expect("|000x");
        let unknown_no_one_z = evaluate_input("|4'b000z").expect("|000z");

        assert_eq!(all_zeros.output, "1'b0");
        assert_eq!(has_one.output, "1'b1");
        assert_eq!(one_dominates_over_x.output, "1'b1");
        assert_eq!(unknown_no_one.output, "1'bx");
        assert_eq!(unknown_no_one_z.output, "1'bx");
    }

    #[test]
    fn reduction_xor_folds_to_parity_and_x_on_unknowns() {
        // XOR has no dominator: any x/z anywhere → x. Otherwise standard
        // odd-parity.
        let even_parity = evaluate_input("^4'b1111").expect("^1111");
        let odd_parity = evaluate_input("^4'b1110").expect("^1110");
        let zero = evaluate_input("^4'b0000").expect("^0000");
        let unknown = evaluate_input("^4'b111x").expect("^111x");
        let unknown_with_zero = evaluate_input("^4'b110x").expect("^110x");
        let unknown_z = evaluate_input("^4'b00z0").expect("^00z0");

        assert_eq!(even_parity.output, "1'b0");
        assert_eq!(odd_parity.output, "1'b1");
        assert_eq!(zero.output, "1'b0");
        assert_eq!(unknown.output, "1'bx");
        assert_eq!(unknown_with_zero.output, "1'bx");
        assert_eq!(unknown_z.output, "1'bx");
    }

    #[test]
    fn negated_reductions_fold_then_invert() {
        // Spot-check that NAND/NOR/XNOR are exactly NOT-of-positive across
        // multi-bit operands too.
        let nand_all_ones = evaluate_input("~&4'b1111").expect("~&1111");
        let nand_has_zero = evaluate_input("~&4'b1101").expect("~&1101");
        let nand_unknown = evaluate_input("~&4'b111x").expect("~&111x");
        let nor_all_zeros = evaluate_input("~|4'b0000").expect("~|0000");
        let nor_has_one = evaluate_input("~|4'b0010").expect("~|0010");
        let xnor_even = evaluate_input("~^4'b1111").expect("~^1111");
        let xnor_odd = evaluate_input("~^4'b1110").expect("~^1110");
        let xnor_unknown = evaluate_input("~^4'b111x").expect("~^111x");

        assert_eq!(nand_all_ones.output, "1'b0");
        assert_eq!(nand_has_zero.output, "1'b1");
        assert_eq!(nand_unknown.output, "1'bx");
        assert_eq!(nor_all_zeros.output, "1'b1");
        assert_eq!(nor_has_one.output, "1'b0");
        assert_eq!(xnor_even.output, "1'b1");
        assert_eq!(xnor_odd.output, "1'b0");
        assert_eq!(xnor_unknown.output, "1'bx");
    }

    #[test]
    fn reduction_result_renders_in_binary_regardless_of_operand_base() {
        // Operand bases vary but the 1-bit reduction result is always
        // binary, like `!`/relational/equality.
        let hex_and = evaluate_input("&8'hff").expect("&hex");
        let hex_xor = evaluate_input("^8'h05").expect("^hex");
        let dec_or = evaluate_input("|4'd0").expect("|dec");

        assert_eq!(hex_and.output, "1'b1");
        assert_eq!(hex_xor.output, "1'b0");
        assert_eq!(dec_or.output, "1'b0");
    }

    #[test]
    fn reduction_widens_through_outer_arithmetic_context() {
        // (&4'b1111) → 1'b1; outer + widens to the parent's 32-bit decimal
        // context. The reduction result's binary base wins the leftmost-base
        // rule, so the parent + renders in binary.
        let widened = evaluate_input("(&4'b1111) + 0").expect("widened &");
        let widened_xnor = evaluate_input("(~^4'b1110) + 0").expect("widened ~^");

        assert_eq!(widened.output, "32'b00000000000000000000000000000001");
        assert_eq!(widened_xnor.output, "32'b00000000000000000000000000000000");
    }

    #[test]
    fn reduction_position_disambiguates_unary_from_binary_and() {
        // `&4'b1111` — pure unary reduction (1).
        // `4'd1 & &4'b1111` — binary AND with rhs = unary reduction (1 & 1 = 1).
        // `4'd1 & 4'd2` — pure binary AND (0).
        let pure_unary = evaluate_input("&4'b1111").expect("pure unary");
        let mixed = evaluate_input("4'd1 & &4'b1111").expect("binary + unary");
        let pure_binary = evaluate_input("4'd1 & 4'd2").expect("pure binary");

        assert_eq!(pure_unary.output, "1'b1");
        // 4'd1 (4 bits) & (reduced 1'b1, zero-extended to 4 bits = 4'b0001) = 4'b0001.
        // Leftmost-base is decimal (4'd1).
        assert_eq!(mixed.output, "4'd1");
        assert_eq!(pure_binary.output, "4'd0");
    }

    #[test]
    fn reduction_chains_through_recursive_parse_unary() {
        // !!, ~~, and reduction stacks all flow through parse_unary
        // recursively. `&|4'b0110` parses as `&(|4'b0110)` → &(1'b1) → 1.
        // `~~&4'b1111` parses as `~(~(&4'b1111))` → ~(~1) → ~0 → 1.
        let nested_reductions = evaluate_input("&|4'b0110").expect("&|0110");
        let not_chains_into_reduction = evaluate_input("~~&4'b1111").expect("~~&1111");

        assert_eq!(nested_reductions.output, "1'b1");
        assert_eq!(not_chains_into_reduction.output, "1'b1");
    }

    // ---------- Shift operators (<< >> <<< >>>) ----------
    //
    // LRM 1364-2005 §5.1.12: the LHS is context-determined; the RHS is
    // self-determined and "always treated as an unsigned number ... has no
    // effect on the signedness of the result". `<<` and `<<<` zero-fill
    // vacated positions; `>>` always zero-fills; `>>>` fills with the LHS
    // sign bit when the result type is signed and zero-fills otherwise. If
    // the RHS contains x or z, the entire result is unknown.
    //
    // Single-bit truth tables in `doc/four_value_ops_output.txt` only cover
    // 1-bit operands (where multi-bit shift dynamics collapse), so the
    // interesting multi-bit cases here were cross-checked against iverilog.

    #[test]
    fn tokenizes_shift_operators_as_single_tokens() {
        // Greedy lex: `<<<`/`>>>` win over `<<`/`>>`, which win over the
        // single-character `<`/`>` (and over `<=`/`>=` which still need the
        // `=`-specific path). A regression where `<<<` collapsed to two
        // tokens would silently become `<<` followed by `<`.
        let shl = tokenize("4'd1 << 1").expect("<< should tokenize");
        let shr = tokenize("4'd1 >> 1").expect(">> should tokenize");
        let ashl = tokenize("4'd1 <<< 1").expect("<<< should tokenize");
        let ashr = tokenize("4'd1 >>> 1").expect(">>> should tokenize");

        assert_eq!(shl[1], Token::LogicalShiftLeft);
        assert_eq!(shr[1], Token::LogicalShiftRight);
        assert_eq!(ashl[1], Token::ArithmeticShiftLeft);
        assert_eq!(ashr[1], Token::ArithmeticShiftRight);
    }

    #[test]
    fn shift_lex_does_not_swallow_relational_or_le_ge() {
        // Adding `<<`/`<<<` paths must not regress `<=`, `>=`, or bare `<`/`>`.
        let le = tokenize("4'd1 <= 4'd2").expect("<=");
        let ge = tokenize("4'd1 >= 4'd2").expect(">=");
        let lt = tokenize("4'd1 < 4'd2").expect("<");
        let gt = tokenize("4'd1 > 4'd2").expect(">");

        assert_eq!(le[1], Token::LessEqual);
        assert_eq!(ge[1], Token::GreaterEqual);
        assert_eq!(lt[1], Token::Less);
        assert_eq!(gt[1], Token::Greater);
    }

    #[test]
    fn evaluates_basic_logical_shift_left() {
        let shifted = evaluate_input("4'b0001 << 1").expect("<< 1");
        let by_two = evaluate_input("4'b0001 << 4'd2").expect("<< 2");
        // Top bit shifts out at the 4-bit self-determined width.
        let overflow = evaluate_input("4'b1000 << 1").expect("<< 1 overflow");
        let by_zero = evaluate_input("4'b0101 << 0").expect("<< 0 noop");

        assert_eq!(shifted.output, "4'b0010");
        assert_eq!(by_two.output, "4'b0100");
        assert_eq!(overflow.output, "4'b0000");
        assert_eq!(by_zero.output, "4'b0101");
    }

    #[test]
    fn evaluates_basic_logical_shift_right() {
        let shifted = evaluate_input("4'b1000 >> 1").expect(">> 1");
        let by_two = evaluate_input("4'b1100 >> 4'd2").expect(">> 2");
        let by_zero = evaluate_input("4'b0101 >> 0").expect(">> 0 noop");
        // Logical right shift always zero-fills, even when the LHS is signed.
        let signed_zero_fill = evaluate_input("4'sb1000 >> 1").expect("signed >> 1");

        assert_eq!(shifted.output, "4'b0100");
        assert_eq!(by_two.output, "4'b0011");
        assert_eq!(by_zero.output, "4'b0101");
        assert_eq!(signed_zero_fill.output, "4'sb0100");
    }

    #[test]
    fn arithmetic_left_shift_matches_logical_left_shift() {
        // LRM 5.1.12: `<<<` is exactly `<<` — both zero-fill the LSBs.
        let logical = evaluate_input("4'b0011 << 1").expect("<<");
        let arithmetic = evaluate_input("4'b0011 <<< 1").expect("<<<");
        let signed_logical = evaluate_input("4'sb1010 << 1").expect("signed <<");
        let signed_arith = evaluate_input("4'sb1010 <<< 1").expect("signed <<<");

        assert_eq!(logical.output, arithmetic.output);
        assert_eq!(signed_logical.output, signed_arith.output);
    }

    #[test]
    fn arithmetic_right_shift_sign_fills_when_signed() {
        // Signed self-determined: vacated MSB takes the LHS sign bit.
        //   4'sb1000 = -8;  >>> 1 → 4'sb1100 = -4
        //   4'sb1110 = -2;  >>> 1 → 4'sb1111 = -1
        //   4'sb1000 >>> 4'd3 → all four MSBs vacated, all filled with 1
        let neg_eight = evaluate_input("4'sb1000 >>> 1").expect("signed >>> 1");
        let neg_two = evaluate_input("4'sb1110 >>> 1").expect("signed >>> 1");
        let saturated = evaluate_input("4'sb1000 >>> 4'd3").expect("signed >>> 3");

        assert_eq!(neg_eight.output, "4'sb1100");
        assert_eq!(neg_two.output, "4'sb1111");
        assert_eq!(saturated.output, "4'sb1111");
    }

    #[test]
    fn arithmetic_right_shift_zero_fills_when_unsigned() {
        // Unsigned LHS (self-determined unsigned context) → `>>>` is just `>>`.
        let unsigned = evaluate_input("4'b1000 >>> 1").expect("unsigned >>> 1");
        let unsigned_full = evaluate_input("4'b1111 >>> 4'd3").expect("unsigned >>> 3");

        assert_eq!(unsigned.output, "4'b0100");
        assert_eq!(unsigned_full.output, "4'b0001");
    }

    #[test]
    fn arithmetic_right_shift_propagates_x_or_z_when_msb_is_unknown() {
        // The fill bit IS the LHS MSB (LRM 5.1.12). When the MSB is x, the
        // vacated positions become x; when it is z, the same z value is used.
        let x_fill = evaluate_input("4'sbx000 >>> 1").expect("x msb");
        let z_fill = evaluate_input("4'sbz000 >>> 1").expect("z msb");

        assert_eq!(x_fill.output, "4'sbxx00");
        assert_eq!(z_fill.output, "4'sbzz00");
    }

    #[test]
    fn shift_with_unknown_rhs_returns_all_x() {
        // LRM 5.1.12: "If the right operand has an x or z value, then the
        // result shall be unknown." This dominates the LHS bit pattern — even
        // a fully-known LHS yields all-x.
        let x_rhs_left = evaluate_input("4'd5 << 4'bx").expect("x rhs <<");
        let z_rhs_right = evaluate_input("4'd5 >> 4'bz").expect("z rhs >>");
        let x_rhs_arith = evaluate_input("4'sb1000 >>> 4'bx").expect("x rhs >>>");
        // Even one x bit in the RHS poisons the entire result.
        let partial_x = evaluate_input("4'd5 << 4'b00x0").expect("partial x rhs");
        // Result inherits LHS base for rendering (decimal here → 4'dx).
        assert_eq!(x_rhs_left.output, "4'dx");
        assert_eq!(z_rhs_right.output, "4'dx");
        assert_eq!(x_rhs_arith.output, "4'sbxxxx");
        assert_eq!(partial_x.output, "4'dx");
    }

    #[test]
    fn shift_preserves_lhs_bit_values_including_x_and_z() {
        // The shift moves bits into new positions without altering them; only
        // the vacated edge takes the fill value. So an x/z in the middle of
        // the LHS just slides one position over.
        let x_in_middle = evaluate_input("4'b01x0 << 1").expect("x in middle <<");
        let z_in_middle = evaluate_input("4'b1z00 >> 1").expect("z in middle >>");
        // Left shift by 1 of 4'sb1xx0: the MSB 1 shifts out (lost); xx slides
        // left to bits 3,2; the original LSB 0 slides to bit 1; bit 0 is the
        // zero-filled vacated LSB. → 4'sbxx00.
        let signed_xx = evaluate_input("4'sb1xx0 << 1").expect("signed x's");

        assert_eq!(x_in_middle.output, "4'b1x00");
        assert_eq!(z_in_middle.output, "4'b01z0");
        assert_eq!(signed_xx.output, "4'sbxx00");
    }

    #[test]
    fn shift_clamps_oversized_count_to_lhs_width() {
        // LRM 5.1.12 doesn't bound the RHS, so we treat any count >= width
        // as an all-fill case. Useful both for huge constants and (next test)
        // for negative RHS values that bit-encode as huge unsigned numbers.
        let exactly_width = evaluate_input("4'd5 << 4'd4").expect("exactly width");
        let beyond_width = evaluate_input("4'b0101 << 4'd5").expect("beyond width");
        let beyond_width_right = evaluate_input("4'b1111 >> 4'd9").expect(">> beyond");
        let signed_beyond = evaluate_input("4'sb1000 >>> 4'd9").expect(">>> beyond");

        assert_eq!(exactly_width.output, "4'd0");
        assert_eq!(beyond_width.output, "4'b0000");
        assert_eq!(beyond_width_right.output, "4'b0000");
        // signed >>> with beyond-width count saturates to the sign bit.
        assert_eq!(signed_beyond.output, "4'sb1111");
    }

    #[test]
    fn shift_treats_negative_rhs_as_large_unsigned() {
        // LRM 5.1.12: the RHS is "always treated as an unsigned number".
        // -1 has bits 1...1, which read unsigned is 2^N-1 — well past any
        // reasonable LHS width — so the shift saturates to all-fill.
        let neg_one_left = evaluate_input("4'd5 << -4'sd1").expect("<< -1");
        let neg_one_signed_arith = evaluate_input("4'sb1000 >>> -4'sd1").expect(">>> -1");

        assert_eq!(neg_one_left.output, "4'd0");
        assert_eq!(neg_one_signed_arith.output, "4'sb1111");
    }

    #[test]
    fn shift_widens_lhs_through_outer_arithmetic_context() {
        // Self-determined the high bit of `4'd8 << 4'd1` shifts out → 4'd0.
        // Inside a 32-bit context the LHS first widens to 32 bits before the
        // shift, so the bit survives and the answer is 16, not 0. Same shape
        // as the existing `applies_width_rules_to_multiplicative_expressions`
        // test for arithmetic.
        let truncated = evaluate_input("4'd8 << 4'd1").expect("truncated");
        let widened = evaluate_input("(4'd8 << 4'd1) + 0").expect("widened");

        assert_eq!(truncated.output, "4'd0");
        assert_eq!(widened.output, "32'd16");
    }

    #[test]
    fn arithmetic_right_shift_fill_follows_propagated_signedness() {
        // Same shift, three different propagated contexts. `>>>` flips
        // between sign-fill and zero-fill based on whether the result type
        // ends up signed. iverilog-confirmed.
        //
        //   Self-determined signed: signed → sign-fill → -4 → 4'sb1100.
        //   Mixed unsigned context (8'd0): unsigned → zero-fill, but the
        //     LHS first zero-extends to 8 bits = 8 → 8 >>> 1 = 4 → 8'b0...100.
        //   All-signed context (signed `0`): signed → sign-extend LHS to 32
        //     bits = -8 → -8 >>> 1 = -4 → 32'sb1...1100.
        let self_determined = evaluate_input("4'sb1000 >>> 1").expect("self");
        let mixed_unsigned = evaluate_input("(4'sb1000 >>> 1) + 8'd0").expect("mixed");
        let all_signed = evaluate_input("(4'sb1000 >>> 1) + 0").expect("all signed");

        assert_eq!(self_determined.output, "4'sb1100");
        // Result base inherits from the leftmost operand (Binary), and the
        // shared 8-bit unsigned context makes the outer result unsigned.
        assert_eq!(mixed_unsigned.output, "8'b00000100");
        assert_eq!(
            all_signed.output,
            "32'sb11111111111111111111111111111100"
        );
    }

    #[test]
    fn shift_inherits_leftmost_base_like_arithmetic() {
        // Result base is the LHS base, mirroring the existing leftmost-wins
        // rule for arithmetic and bitwise binaries.
        let hex = evaluate_input("8'h0a << 4'd1").expect("hex base");
        let binary = evaluate_input("8'b00001010 << 4'd1").expect("binary base");
        let decimal = evaluate_input("8'd10 << 4'd1").expect("decimal base");

        assert_eq!(hex.output, "8'h14");
        assert_eq!(binary.output, "8'b00010100");
        assert_eq!(decimal.output, "8'd20");
    }

    #[test]
    fn shift_rhs_is_self_determined_and_does_not_widen_lhs() {
        // RHS at LRM Table 5-22 is self-determined, so a wide RHS must NOT
        // pull the LHS up to its width. Without the self-determined rule the
        // 4-bit `4'd8` would widen to 32 bits and `<< 1` would yield 16
        // instead of the truncated 4'd0.
        let wide_rhs = evaluate_input("4'd8 << 32'd1").expect("wide rhs");
        assert_eq!(wide_rhs.output, "4'd0");
    }

    #[test]
    fn shift_rhs_signedness_does_not_flip_result_signedness() {
        // LRM 5.1.12: the RHS "has no effect on the signedness of the
        // result". A signed RHS therefore keeps the LHS-driven signedness.
        let signed_lhs_signed_rhs = evaluate_input("4'sd2 << 4'sd1").expect("ss");
        let unsigned_lhs_signed_rhs = evaluate_input("4'd2 << 4'sd1").expect("us");

        assert_eq!(signed_lhs_signed_rhs.output, "4'sd4");
        assert_eq!(unsigned_lhs_signed_rhs.output, "4'd4");
    }

    #[test]
    fn shift_precedence_below_additive_above_relational() {
        // LRM Table 5-4: `+`/`-` > `<<`/`>>` > `<`/`>`.
        //
        //   `4'd1 + 4'd2 << 4'd1` parses as `(4'd1 + 4'd2) << 4'd1`
        //                              = 3 << 1 = 6.
        //   `4'd2 << 4'd1 < 4'd5` parses as `(4'd2 << 4'd1) < 4'd5`
        //                              = 4 < 5 = 1.
        let add_then_shift_expr = parse_expression("4'd1 + 4'd2 << 4'd1").expect("parse");
        match add_then_shift_expr {
            Expr::Binary {
                op: BinaryOp::LogicalShiftLeft,
                lhs,
                ..
            } => assert!(matches!(*lhs, Expr::Binary { op: BinaryOp::Add, .. })),
            other => panic!("expected top-level <<, got {other:?}"),
        }
        let add_then_shift = evaluate_input("4'd1 + 4'd2 << 4'd1").expect("eval");
        assert_eq!(add_then_shift.output, "4'd6");

        let shift_then_relational_expr =
            parse_expression("4'd2 << 4'd1 < 4'd5").expect("parse");
        match shift_then_relational_expr {
            Expr::Binary {
                op: BinaryOp::LessThan,
                lhs,
                ..
            } => assert!(matches!(
                *lhs,
                Expr::Binary {
                    op: BinaryOp::LogicalShiftLeft,
                    ..
                }
            )),
            other => panic!("expected top-level <, got {other:?}"),
        }
        let shift_then_relational = evaluate_input("4'd2 << 4'd1 < 4'd5").expect("eval");
        assert_eq!(shift_then_relational.output, "1'b1");
    }

    #[test]
    fn shift_is_left_associative() {
        // `a << b << c` parses as `(a << b) << c`. Same shape check used for
        // the other binary levels.
        let expr = parse_expression("4'd1 << 4'd1 << 4'd1").expect("parse");
        match expr {
            Expr::Binary {
                op: BinaryOp::LogicalShiftLeft,
                lhs,
                ..
            } => assert!(matches!(
                *lhs,
                Expr::Binary {
                    op: BinaryOp::LogicalShiftLeft,
                    ..
                }
            )),
            other => panic!("expected top-level <<, got {other:?}"),
        }
        // (1 << 1) << 1 = 2 << 1 = 4 at 4-bit width.
        let result = evaluate_input("4'd1 << 4'd1 << 4'd1").expect("eval");
        assert_eq!(result.output, "4'd4");
    }

    #[test]
    fn shift_at_primary_position_is_rejected() {
        // No shift operator is unary, so a leading shift token has no
        // operand to the left. parse_primary's catchall must turn this into
        // the standard "expected expression operand" error rather than
        // silently consuming the operator as something else.
        let lead_shl = evaluate_input("<< 4'd1").expect_err("leading <<");
        let lead_shr = evaluate_input(">> 4'd1").expect_err("leading >>");

        assert_eq!(lead_shl, "expected expression operand");
        assert_eq!(lead_shr, "expected expression operand");
    }

    #[test]
    fn reduction_binds_tighter_than_power() {
        // LRM Table 5-4: unary reductions are at the unary level, tighter
        // than **. So `&4'b1111 ** 2` parses as `(&4'b1111) ** 2` = 1**2 = 1.
        let expr = parse_expression("&4'b1111 ** 2").expect("parse");
        match expr {
            Expr::Binary {
                op: BinaryOp::Power,
                lhs,
                ..
            } => assert!(matches!(
                *lhs,
                Expr::Unary {
                    op: UnaryOp::ReductionAnd,
                    ..
                }
            )),
            other => panic!("expected top-level **, got {other:?}"),
        }
        let result = evaluate_input("&4'b1111 ** 2").expect("eval");
        assert_eq!(result.output, "1'b1");
    }

    #[test]
    fn conditional_selects_then_when_cond_true() {
        // LRM 5.1.13: a definite-true cond returns expression2, in the
        // unified width of then/else (4 bits here).
        let result = evaluate_input("1 ? 4'd5 : 4'd9").expect("eval");
        assert_eq!(result.output, "4'd5");
    }

    #[test]
    fn conditional_selects_else_when_cond_false() {
        let result = evaluate_input("1'b0 ? 4'd5 : 4'd9").expect("eval");
        assert_eq!(result.output, "4'd9");
    }

    #[test]
    fn conditional_reduces_wide_cond_to_logical() {
        // LRM 5.1.13: cond is self-determined and reduced to a 1-bit
        // logical (any 1 → true, all 0 → false).
        let any_one = evaluate_input("4'b1000 ? 4'd5 : 4'd9").expect("any 1");
        let all_zero = evaluate_input("4'b0000 ? 4'd5 : 4'd9").expect("all 0");

        assert_eq!(any_one.output, "4'd5");
        assert_eq!(all_zero.output, "4'd9");
    }

    #[test]
    fn conditional_ambiguous_cond_merges_when_branches_agree() {
        // LRM 5.1.13: when cond is x, evaluate both branches and merge per
        // bit. Identical branches collapse to the shared value.
        let result = evaluate_input("1'bx ? 4'b1100 : 4'b1100").expect("eval");
        assert_eq!(result.output, "4'b1100");
    }

    #[test]
    fn conditional_ambiguous_cond_merges_per_bit_with_disagreement() {
        // 1'bx ? 4'b1100 : 4'b1010 → bits are (1,1)=1, (1,0)=x, (0,1)=x,
        // (0,0)=0. With LSB-first storage rendered MSB-first as `1xx0`.
        let result = evaluate_input("1'bx ? 4'b1100 : 4'b1010").expect("eval");
        assert_eq!(result.output, "4'b1xx0");
    }

    #[test]
    fn conditional_ambiguous_cond_handles_xz_bits() {
        // x agrees with x (stays x); z agrees with z (stays z, since the
        // merge keeps the shared bit verbatim); x vs z disagrees → x.
        let xx = evaluate_input("1'bx ? 1'bx : 1'bx").expect("xx");
        let zz = evaluate_input("1'bx ? 1'bz : 1'bz").expect("zz");
        let xz = evaluate_input("1'bx ? 1'bx : 1'bz").expect("xz");

        assert_eq!(xx.output, "1'bx");
        assert_eq!(zz.output, "1'bz");
        assert_eq!(xz.output, "1'bx");
    }

    #[test]
    fn conditional_unifies_then_else_widths() {
        // Result width = max(L(then), L(else)). Selecting the narrower
        // branch zero-extends to the unified width.
        let result = evaluate_input("1 ? 4'd5 : 8'd1").expect("eval");
        assert_eq!(result.output, "8'd5");
    }

    #[test]
    fn conditional_signedness_propagates_per_5_5_1() {
        // LRM 5.5.1: any unsigned operand → unsigned result. Pairing a
        // signed and an unsigned branch yields an unsigned conditional.
        let mixed = evaluate_input("1 ? 4'sd1 : 4'd1").expect("mixed");
        let both_signed = evaluate_input("1 ? 4'sd1 : 4'sd1").expect("both signed");

        assert_eq!(mixed.output, "4'd1");
        assert_eq!(both_signed.output, "4'sd1");
    }

    #[test]
    fn conditional_extends_per_5_5_2_not_per_5_1_13() {
        // LRM §5.1.13 last paragraph says the shorter branch is zero-filled
        // from the left, but §5.5.2 says signed-signed unifies under a
        // signed propagated context and the narrower side sign-extends.
        // The two rules disagree for `1 ? 4'shF : 8'sh0`. vcal follows
        // §5.5.2 (matching iverilog and the bitwise path):
        //   - both signed → sign-extend the narrower branch.
        //   - any unsigned → unsigned context → zero-extend.
        let both_signed = evaluate_input("1 ? 4'shF : 8'sh0").expect("both signed");
        let mixed_unsigned = evaluate_input("1 ? 4'shF : 8'h0").expect("mixed");

        assert_eq!(both_signed.output, "8'shff");
        assert_eq!(mixed_unsigned.output, "8'h0f");
    }

    #[test]
    fn conditional_outer_arithmetic_context_widens_branches() {
        // Self-determined `1 ? 4'd8 : 4'd0` is 4'd8. Inside a 32-bit
        // context the branches first widen to 32 bits before selection,
        // matching the shape of the existing shift-widening test.
        let self_determined = evaluate_input("1 ? 4'd8 : 4'd0").expect("self");
        let widened = evaluate_input("(1 ? 4'd8 : 4'd0) + 0").expect("widened");

        assert_eq!(self_determined.output, "4'd8");
        assert_eq!(widened.output, "32'd8");
    }

    #[test]
    fn conditional_outer_unsigned_context_zero_fills_signed_branch() {
        // Mirror of the shift `>>>` propagation test. Same conditional, two
        // outer contexts:
        //   Self-determined: signed → 4'sb1000 sign-extends nowhere
        //     (already 4 bits).
        //   Mixed unsigned (8'd0): result type is unsigned → 4'sb1000
        //     zero-extends to 8'b00001000.
        //   All-signed (signed `0`): result type is signed → 4'sb1000
        //     sign-extends to 32'sb1...11000.
        let self_determined = evaluate_input("1 ? 4'sb1000 : 4'sb1000").expect("self");
        let mixed = evaluate_input("(1 ? 4'sb1000 : 4'sb1000) + 8'd0").expect("mixed");
        let all_signed = evaluate_input("(1 ? 4'sb1000 : 4'sb1000) + 0").expect("all signed");

        assert_eq!(self_determined.output, "4'sb1000");
        assert_eq!(mixed.output, "8'b00001000");
        assert_eq!(
            all_signed.output,
            "32'sb11111111111111111111111111111000"
        );
    }

    #[test]
    fn conditional_is_right_associative() {
        // `1'b0 ? 1 : 1'b1 ? 2 : 3` parses as `1'b0 ? 1 : (1'b1 ? 2 : 3)`.
        // Cond is false, so the else branch runs, picking 2.
        let expr = parse_expression("1'b0 ? 1 : 1'b1 ? 2 : 3").expect("parse");
        match expr {
            Expr::Conditional { else_expr, .. } => {
                assert!(matches!(*else_expr, Expr::Conditional { .. }));
            }
            other => panic!("expected top-level conditional, got {other:?}"),
        }
        let result = evaluate_input("1'b0 ? 1 : 1'b1 ? 2 : 3").expect("eval");
        // Unsized integer literals are 32-bit signed (LRM 3.5.1), so all
        // three branches are signed and the result keeps signedness.
        assert_eq!(result.output, "32'sd2");
    }

    #[test]
    fn conditional_lower_precedence_than_logical_or() {
        // LRM Table 5-4: `?:` sits below `||`. `1 || 0 ? 1 : 2` parses as
        // `(1 || 0) ? 1 : 2`, picking the then branch.
        let expr = parse_expression("1 || 0 ? 1 : 2").expect("parse");
        match expr {
            Expr::Conditional { cond, .. } => {
                assert!(matches!(
                    *cond,
                    Expr::Binary {
                        op: BinaryOp::LogicalOr,
                        ..
                    }
                ));
            }
            other => panic!("expected top-level conditional, got {other:?}"),
        }
        let result = evaluate_input("1 || 0 ? 1 : 2").expect("eval");
        assert_eq!(result.output, "32'sd1");
    }

    #[test]
    fn conditional_lower_precedence_than_relational_and_arithmetic() {
        // `2 > 1 ? 5 : 6` parses as `(2 > 1) ? 5 : 6` and `1 + 1 ? 3 : 4`
        // parses as `(1 + 1) ? 3 : 4` — both lower-precedence operators
        // bind into the cond.
        let relational = evaluate_input("2 > 1 ? 5 : 6").expect("relational cond");
        let arithmetic = evaluate_input("1 + 1 ? 3 : 4").expect("arithmetic cond");

        assert_eq!(relational.output, "32'sd5");
        assert_eq!(arithmetic.output, "32'sd3");
    }

    #[test]
    fn conditional_inherits_then_branch_base() {
        // Result base follows the then branch (the leftmost bit-pattern
        // operand after the cond), mirroring leftmost-wins for binaries.
        let hex_then = evaluate_input("1 ? 8'h0a : 8'd5").expect("hex then");
        let dec_then = evaluate_input("1 ? 8'd10 : 8'h05").expect("dec then");

        assert_eq!(hex_then.output, "8'h0a");
        assert_eq!(dec_then.output, "8'd10");
    }

    #[test]
    fn conditional_missing_colon_is_parse_error() {
        // A `?` without `:` should not silently parse as something else.
        let err = evaluate_input("1 ? 2").expect_err("missing colon");
        assert_eq!(err, "expected `:` in conditional expression");
    }

    #[test]
    fn conditional_chained_in_else_position() {
        // `0 ? 1 : 0 ? 2 : 3` is right-associative so it evaluates as
        // `0 ? 1 : (0 ? 2 : 3)` = `0 ? 1 : 3` = 3.
        let result = evaluate_input("1'b0 ? 4'd1 : 1'b0 ? 4'd2 : 4'd3").expect("eval");
        assert_eq!(result.output, "4'd3");
    }

    // -------- Concatenation / replication (LRM 5.1.14) --------

    #[test]
    fn concatenation_joins_operands_msb_first() {
        // Leftmost operand occupies the high bits. iverilog reference:
        //   $display("%b", {2'b10, 2'b01}); // → 1001
        let result = evaluate_input("{2'b10, 2'b01}").expect("eval");
        assert_eq!(result.output, "4'b1001");
    }

    #[test]
    fn concatenation_supports_more_than_two_operands() {
        let result = evaluate_input("{1'b1, 2'b00, 1'b1}").expect("eval");
        assert_eq!(result.output, "4'b1001");
    }

    #[test]
    fn concatenation_inherits_leftmost_operand_base() {
        // Same leftmost-wins rule as arithmetic/bitwise/shift (vcal display
        // convention; LRM doesn't prescribe one). iverilog confirms the bit
        // pattern (`8'h12`); the base choice is ours.
        let hex_first = evaluate_input("{4'h1, 4'b10}").expect("eval");
        let bin_first = evaluate_input("{4'b10, 4'h1}").expect("eval");
        assert_eq!(hex_first.output, "8'h12");
        assert_eq!(bin_first.output, "8'b00100001");
    }

    #[test]
    fn concatenation_preserves_x_and_z_bits() {
        // Concatenation never reduces unknown bits — each position is copied
        // through. iverilog confirms `xz01`.
        let result = evaluate_input("{2'bxz, 2'b01}").expect("eval");
        assert_eq!(result.output, "4'bxz01");
    }

    #[test]
    fn concatenation_result_is_unsigned_even_when_operands_signed() {
        // LRM 5.5.1 last paragraph + 5.1.14: result is unsigned regardless
        // of operand signedness. iverilog confirms the bit pattern.
        let result = evaluate_input("{4'sb1000, 4'sb0001}").expect("eval");
        assert_eq!(result.output, "8'b10000001");
    }

    #[test]
    fn single_element_concatenation_is_identity_on_bits() {
        // `{x}` is legal LRM syntax and produces the operand's bit pattern
        // re-flagged as unsigned. iverilog accepts it as an identity.
        let result = evaluate_input("{4'b1010}").expect("eval");
        assert_eq!(result.output, "4'b1010");
    }

    #[test]
    fn concatenation_widens_through_outer_arithmetic_context() {
        // The joined value (4 bits) zero-extends to the outer context width
        // (8 bits) before the addition runs — concatenation is unsigned, so
        // §5.5.4 zero-fills regardless of operand signedness.
        let result = evaluate_input("{4'b1010} + 8'd0").expect("eval");
        assert_eq!(result.output, "8'b00001010");
    }

    #[test]
    fn replication_repeats_inner_concatenation() {
        // {N{...}} — N copies of the inner concatenation joined back to back.
        let single = evaluate_input("{4{1'b1}}").expect("eval");
        let multi = evaluate_input("{2{2'b01, 2'b10}}").expect("eval");
        assert_eq!(single.output, "4'b1111");
        assert_eq!(multi.output, "8'b01100110");
    }

    #[test]
    fn replication_count_can_be_a_constant_expression() {
        // The count is any constant expression (LRM 5.1.14). vcal has no
        // variables, so any well-formed expression qualifies.
        let result = evaluate_input("{(1+3){1'b1}}").expect("eval");
        assert_eq!(result.output, "4'b1111");
    }

    #[test]
    fn replication_can_nest_when_inner_is_braced() {
        // `{2{ {2{1'b1}} }}` — outer rep of an inner replication. Note the
        // inner `{2{1'b1}}` must itself be a brace primary; iverilog also
        // rejects `{2{2{1'b1}}}` as a syntax error since `2{1'b1}` is not a
        // standalone primary.
        let result = evaluate_input("{2{ {2{1'b1}} }}").expect("eval");
        assert_eq!(result.output, "4'b1111");
    }

    #[test]
    fn concatenation_rejects_bare_unsized_literal_operand() {
        // LRM 5.1.14: "Unsized constant numbers shall not be allowed in
        // concatenations." iverilog rejects this with "indefinite width".
        let err = evaluate_input("{1, 4'd2}").expect_err("indefinite");
        assert_eq!(err, "concatenation operand has indefinite width");
    }

    #[test]
    fn concatenation_rejects_arithmetic_with_unsized_operand() {
        // The indefinite-width flag propagates through context-determined
        // arithmetic: `4'd1 + 1` is indefinite because the `1` is unsized.
        // iverilog rejects with "Concatenation operand ... has indefinite
        // width."
        let err = evaluate_input("{4'd1 + 1, 4'd2}").expect_err("indefinite");
        assert_eq!(err, "concatenation operand has indefinite width");
    }

    #[test]
    fn concatenation_accepts_arithmetic_when_all_operands_sized() {
        // `4'd1 + 4'd1` is sized (both operands sized → result is 4-bit), so
        // the operand has a definite width and concatenation succeeds.
        // iverilog: `00100010` = 8'd34.
        let result = evaluate_input("{4'd1 + 4'd1, 4'd2}").expect("eval");
        assert_eq!(result.output, "8'd34");
    }

    #[test]
    fn concatenation_accepts_one_bit_results_with_unsized_subexpressions() {
        // Relational/equality/logical/reduction always produce 1-bit results
        // — they have a definite width even when their operands are unsized.
        // iverilog: `{1==2, 4'd2}` → `00010` = 5 bits.
        let result = evaluate_input("{1==2, 4'd2}").expect("eval");
        assert_eq!(result.output, "5'b00010");
    }

    #[test]
    fn concatenation_rejects_shift_with_unsized_lhs() {
        // Shifts take their result width from the LHS only (LRM 5.1.12), so
        // an unsized LHS makes the whole expression indefinite. iverilog
        // also rejects `{1 << 1, 4'd2}` as indefinite.
        let err = evaluate_input("{1 << 1, 4'd2}").expect_err("indefinite");
        assert_eq!(err, "concatenation operand has indefinite width");
    }

    #[test]
    fn concatenation_rejects_conditional_with_unsized_branch() {
        // Conditional width is max(then, else) (LRM 5.1.13), so an unsized
        // branch makes the whole conditional indefinite. iverilog rejects
        // `{1'b1 ? 1 : 4'd2, 4'd2}` as indefinite.
        let err = evaluate_input("{1'b1 ? 1 : 4'd2, 4'd2}").expect_err("indefinite");
        assert_eq!(err, "concatenation operand has indefinite width");
    }

    #[test]
    fn top_level_replication_rejects_zero_count() {
        // LRM 5.1.14 only permits zero replication when it sits inside a
        // concatenation with at least one positive-size operand; a top-level
        // `{0{...}}` (no enclosing concat) is rejected. iverilog produces
        // "Concatenation repeat may not be zero in this context."
        let err = evaluate_input("{0{1'b1}}").expect_err("zero count");
        assert_eq!(err, "replication count must be positive in this context");
    }

    #[test]
    fn zero_replication_inside_concatenation_contributes_no_bits() {
        // LRM 5.1.14: a replication may have a zero count when it is one of
        // the operands of a concatenation whose other operands sum to a
        // positive width. The zero-rep simply contributes nothing.
        // iverilog: `{{0{1'b1}}, 1'b1}` → `1`.
        let prefix = evaluate_input("{ {0{1'b1}}, 1'b1 }").expect("zero rep prefix");
        let suffix = evaluate_input("{ 4'b1010, {0{1'b1}} }").expect("zero rep suffix");
        let middle = evaluate_input("{ 1'b1, {0{1'b1}}, 1'b0 }").expect("zero rep middle");
        let multiple = evaluate_input("{ {0{1'b1}}, {0{1'b1}}, 1'b1 }").expect("zero rep many");

        assert_eq!(prefix.output, "1'b1");
        assert_eq!(suffix.output, "4'b1010");
        assert_eq!(middle.output, "2'b10");
        assert_eq!(multiple.output, "1'b1");
    }

    #[test]
    fn zero_replication_through_grouped_is_treated_the_same() {
        // `({0{1'b1}})` is `Grouped(Replication{0, ...})`. iverilog accepts
        // it inside a concatenation; vcal looks through `Grouped` to find
        // the underlying Replication node when applying the zero-permission.
        let result = evaluate_input("{ ({0{1'b1}}), 1'b1 }").expect("grouped zero rep");
        assert_eq!(result.output, "1'b1");
    }

    #[test]
    fn zero_replication_inside_nested_replication_inner_list() {
        // The zero-permission also applies to a replication's *inner*
        // concatenation list, since that list is itself a concatenation.
        // iverilog: `{2{ {0{1'b1}}, 1'b1 }}` → `11`.
        let result = evaluate_input("{2{ {0{1'b1}}, 1'b1 }}").expect("nested zero rep");
        assert_eq!(result.output, "2'b11");
    }

    #[test]
    fn concatenation_of_only_zero_replication_is_rejected() {
        // `{ {0{1'b1}} }` — one operand, and it has zero size. No
        // positive-size sibling, so the surrounding concatenation has no
        // positive-size operand. iverilog: "Concatenation/replication may
        // not have zero width in this context."
        let solo =
            evaluate_input("{ {0{1'b1}} }").expect_err("solo zero rep in concat");
        let pair = evaluate_input("{ {0{1'b1}}, {0{1'b1}} }")
            .expect_err("two zero reps no positive sibling");
        let nested =
            evaluate_input("{2{ {0{1'b1}} }}").expect_err("outer rep over zero-only inner");
        assert_eq!(
            solo,
            "concatenation must have at least one operand with positive size"
        );
        assert_eq!(
            pair,
            "concatenation must have at least one operand with positive size"
        );
        assert_eq!(
            nested,
            "concatenation must have at least one operand with positive size"
        );
    }

    #[test]
    fn replication_rejects_negative_count() {
        // `-1` is signed-negative — read as a math integer, sign() = Minus.
        let err = evaluate_input("{-1{1'b1}}").expect_err("negative count");
        assert_eq!(err, "replication count must be non-negative");
    }

    #[test]
    fn replication_rejects_unknown_count() {
        // A count with any x or z bit is rejected — same self-determined
        // count check that iverilog applies (the count must be "a constant
        // expression that is non-negative, non-x, non-z").
        let err = evaluate_input("{1'bx{1'b1}}").expect_err("unknown count");
        assert_eq!(err, "replication count contains unknown bits");
    }

    #[test]
    fn empty_braces_is_a_parse_error() {
        // `{}` — no expressions inside; LRM grammar requires at least one.
        let err = evaluate_input("{}").expect_err("empty");
        assert_eq!(err, "expected expression operand");
    }

    #[test]
    fn unclosed_concatenation_is_a_parse_error() {
        let err = evaluate_input("{4'd1, 4'd2").expect_err("unclosed");
        assert_eq!(err, "missing closing brace in concatenation");
    }

    #[test]
    fn tokenizes_braces_and_comma_as_separate_tokens() {
        // Braces and comma must split adjacent literals — `1,2'b10` should
        // tokenize as `1`, `,`, `2'b10`, not be swallowed into a single
        // integer literal.
        let tokens = tokenize("{1'd1,2'b10}").expect("tokens");
        assert_eq!(
            tokens,
            vec![
                Token::LBrace,
                Token::IntegerLiteral("1'd1".to_string()),
                Token::Comma,
                Token::IntegerLiteral("2'b10".to_string()),
                Token::RBrace,
            ]
        );
    }

    #[test]
    fn replication_widens_through_outer_arithmetic_context() {
        // Same self-determined-then-extend shape as plain concatenation: the
        // 4-bit replication result zero-extends to the 8-bit outer context.
        let result = evaluate_input("{4{1'b1}} + 8'd0").expect("eval");
        assert_eq!(result.output, "8'b00001111");
    }
}
