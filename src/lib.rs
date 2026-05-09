use num_bigint::BigUint;
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
            return if self.bits.iter().any(|bit| *bit == LogicBit::X) {
                "x".to_string()
            } else {
                "z".to_string()
            };
        }

        bits_to_biguint(&self.bits).to_str_radix(10)
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
}

#[derive(Debug, PartialEq, Eq)]
pub struct Evaluation {
    pub output: String,
    pub should_exit: bool,
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
        write!(writer, "In[{index}]:")?;
        writer.flush()?;

        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }

        match evaluate_input(&line) {
            Ok(result) => {
                writeln!(writer, "Out[{index}]:{}", result.output)?;
                if result.should_exit {
                    break;
                }
            }
            Err(message) => {
                writeln!(writer, "Out[{index}]:")?;
                writeln!(writer, "Error: {message}")?;
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

    parse_integer(input).map(ParsedLine::Value)
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
    let width = usize::max(biguint_bit_len(&value), 32);

    Ok(IntegerValue {
        width,
        signed: false,
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
        Base::Binary | Base::Octal | Base::Hex => {
            parse_based_radix(width, signed, base, &digits)
        }
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
    if bits.iter().any(|bit| *bit == LogicBit::X) {
        return 'x';
    }

    if bits.iter().any(|bit| *bit == LogicBit::Z) {
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
            if value == 0 { '0' } else { '1' }
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

fn bits_to_biguint(bits: &[LogicBit]) -> BigUint {
    bits.iter()
        .enumerate()
        .fold(BigUint::zero(), |acc, (index, bit)| match bit {
            LogicBit::One => acc | (BigUint::one() << index),
            LogicBit::Zero | LogicBit::X | LogicBit::Z => acc,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn evaluates_unsized_decimal() {
        let evaluation = evaluate_input("42").expect("decimal literal should parse");
        assert_eq!(evaluation.output, "32'd42");
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
        assert_eq!(evaluation.output, "8'sd255");
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
        assert_eq!(output, "In[0]:Out[0]:32'd42\nIn[1]:Out[1]:\n");
    }
}
