use num_bigint::{BigInt, BigUint, Sign};
use num_traits::{One, Zero};

// LRM §3.5.2 / §4.8: real numbers are IEEE 754 double-precision and live
// alongside integers as a separate value kind. Operators that are legal on
// reals (Table 5-2) operate in f64; operators that are illegal (Table 5-3)
// reject before evaluating. Reals never carry width, signedness, base, or
// x/z bits — once a sub-expression is real, the integer leaf-extension and
// context-propagation machinery does not apply.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Integer(IntegerValue),
    Real(f64),
}

impl Value {
    pub fn canonical(&self) -> String {
        match self {
            Self::Integer(value) => value.canonical(),
            Self::Real(value) => format_real(*value),
        }
    }
}

// Render an f64 in a Verilog-friendly form. Goals: always produce a token
// the lexer can read back (so a decimal point or exponent is always
// present), keep ordinary magnitudes readable as "1.0" / "2.5", and switch
// to scientific notation for magnitudes outside [1e-4, 1e10) where the
// non-scientific form would be either lossy or unwieldy. NaN/±∞ go through
// even though the LRM doesn't enumerate them — they only arise from the
// "unspecified" real-power corners (§5.1.5), and emitting the literal Rust
// names keeps them visible to the user instead of silently masking them.
fn format_real(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-inf".to_string()
        } else {
            "inf".to_string()
        };
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0".to_string()
        } else {
            "0.0".to_string()
        };
    }

    // [1e-4, 1e10) is the "human-readable fixed-point" window. Outside it
    // we switch to scientific so very large / very small magnitudes don't
    // become illegible runs of zeros — 1.2E12 stays as `1.2e+12` rather
    // than `1200000000000.0`.
    let abs = value.abs();
    if !(1e-4..1e10).contains(&abs) {
        let formatted = format!("{value:e}");
        return ensure_exponent_has_sign(&formatted);
    }

    let formatted = format!("{value}");
    if formatted.contains('.') || formatted.contains('e') || formatted.contains('E') {
        formatted
    } else {
        format!("{formatted}.0")
    }
}

// Rust's {:e} omits the '+' on positive exponents (e.g. "1e10"). Verilog's
// LRM examples and most simulators print with an explicit sign on the
// exponent, and round-tripping through our own lexer is easier when the
// sign is always present.
fn ensure_exponent_has_sign(formatted: &str) -> String {
    if let Some(index) = formatted.find('e') {
        let (mantissa, exponent) = formatted.split_at(index);
        let exponent = &exponent[1..];
        let mantissa = if mantissa.contains('.') {
            mantissa.to_string()
        } else {
            format!("{mantissa}.0")
        };
        if exponent.starts_with('+') || exponent.starts_with('-') {
            format!("{mantissa}e{exponent}")
        } else {
            format!("{mantissa}e+{exponent}")
        }
    } else {
        formatted.to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicBit {
    Zero,
    One,
    X,
    Z,
}

// LogicBit is a `Copy` enum laid out as 1 byte per Verilog bit, so a
// `Vec<LogicBit>` of width N costs N bytes. Without a cap, a tiny input
// like `9999999999999'd1` asks for ~10 TB and the kernel hangs committing
// pages. Cap at 16 Mbit (16 MB per vector) — comfortably above any
// realistic calculator use, comfortably below "the box freezes".
pub(crate) const MAX_BIT_WIDTH: usize = 1 << 24;

pub(crate) fn ensure_bit_width(width: usize, kind: &str) -> Result<(), String> {
    if width > MAX_BIT_WIDTH {
        Err(format!(
            "{kind} width {width} exceeds limit {MAX_BIT_WIDTH}"
        ))
    } else {
        Ok(())
    }
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

    pub(crate) fn group_size(self) -> usize {
        match self {
            Self::Binary => 1,
            Self::Octal => 3,
            Self::Decimal => 1,
            Self::Hex => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DisplayStyle {
    Base,
    String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntegerValue {
    pub(crate) width: usize,
    pub(crate) signed: bool,
    pub(crate) base: Base,
    pub(crate) display_style: DisplayStyle,
    pub(crate) bits: Vec<LogicBit>,
    // True for literals parsed without an explicit size (LRM 3.5.1 default
    // width). Drives Table 5-22 footnote a's MSB-fill extension when the
    // propagated context is wider than the default. Always false for sized
    // literals and for any value produced by an operator.
    pub(crate) unsized_literal: bool,
}

impl IntegerValue {
    pub fn canonical(&self) -> String {
        if self.display_style == DisplayStyle::String
            && let Some(text) = self.render_string_literal()
        {
            return text;
        }

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

    pub(crate) fn has_unknown_bits(&self) -> bool {
        self.bits
            .iter()
            .any(|bit| matches!(bit, LogicBit::X | LogicBit::Z))
    }

    pub(crate) fn resized_to_context(&self, width: usize, context_signed: bool) -> Self {
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
            display_style: DisplayStyle::Base,
            bits,
            unsized_literal: false,
        }
    }

    // LRM Table 5-22 footnote a / §3.5.1 literal-fill rule: unsized constants
    // extend by their own MSB (x/z) or by their own declared signedness
    // (sign-extend if signed, zero-extend if unsigned). This is independent of
    // the propagated context signedness, so e.g.:
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
            display_style: DisplayStyle::Base,
            bits,
            unsized_literal: false,
        }
    }

    fn context_extension_bit(&self, context_signed: bool) -> LogicBit {
        if !context_signed {
            return LogicBit::Zero;
        }
        self.bits.last().copied().unwrap_or(LogicBit::Zero)
    }

    pub(crate) fn as_bigint(&self, signed: bool) -> BigInt {
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
    pub(crate) fn computed(width: usize, signed: bool, base: Base, bits: Vec<LogicBit>) -> Self {
        Self {
            width,
            signed,
            base,
            display_style: DisplayStyle::Base,
            bits,
            unsized_literal: false,
        }
    }

    pub(crate) fn from_bigint(value: BigInt, width: usize, signed: bool, base: Base) -> Self {
        Self {
            width,
            signed,
            base,
            display_style: DisplayStyle::Base,
            bits: bigint_to_bits_with_width(&value, width),
            unsized_literal: false,
        }
    }

    pub(crate) fn all_x(width: usize, signed: bool, base: Base) -> Self {
        Self {
            width,
            signed,
            base,
            display_style: DisplayStyle::Base,
            bits: vec![LogicBit::X; width],
            unsized_literal: false,
        }
    }

    pub(crate) fn with_display_style(mut self, display_style: DisplayStyle) -> Self {
        self.display_style = display_style;
        self
    }

    fn render_string_literal(&self) -> Option<String> {
        if self.width != self.bits.len() || !self.width.is_multiple_of(8) {
            return None;
        }

        let byte_count = self.width / 8;
        let mut output = String::with_capacity(byte_count + 2);
        output.push('"');
        for byte_index in (0..byte_count).rev() {
            let byte = byte_from_bits(&self.bits[byte_index * 8..byte_index * 8 + 8])?;
            push_escaped_byte(byte, &mut output);
        }
        output.push('"');
        Some(output)
    }
}

fn byte_from_bits(bits: &[LogicBit]) -> Option<u8> {
    let mut byte = 0u8;
    for (index, bit) in bits.iter().enumerate() {
        match bit {
            LogicBit::Zero => {}
            LogicBit::One => byte |= 1 << index,
            LogicBit::X | LogicBit::Z => return None,
        }
    }
    Some(byte)
}

fn push_escaped_byte(byte: u8, output: &mut String) {
    match byte {
        b'\n' => output.push_str("\\n"),
        b'\t' => output.push_str("\\t"),
        b'\\' => output.push_str("\\\\"),
        b'"' => output.push_str("\\\""),
        0x20..=0x7e => output.push(byte as char),
        _ => output.push_str(&format!("\\{byte:03o}")),
    }
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

pub(crate) fn biguint_bit_len(value: &BigUint) -> usize {
    if value.is_zero() {
        0
    } else {
        value.bits() as usize
    }
}

pub(crate) fn signed_decimal_bit_len(value: &BigUint) -> usize {
    if value.is_zero() {
        1
    } else {
        biguint_bit_len(value) + 1
    }
}

pub(crate) fn biguint_to_bits_with_width(value: &BigUint, width: usize) -> Vec<LogicBit> {
    // Walk u32 limbs once and read each output bit by index/limb — O(width),
    // versus the natural `(value >> shift) & 1` which re-shifts the entire
    // BigUint per bit and degrades to O(width²) on wide operands.
    let digits = value.to_u32_digits();
    (0..width)
        .map(|index| {
            let limb = digits.get(index / 32).copied().unwrap_or(0);
            if (limb >> (index % 32)) & 1 == 1 {
                LogicBit::One
            } else {
                LogicBit::Zero
            }
        })
        .collect()
}

pub(crate) fn bigint_to_bits_with_width(value: &BigInt, width: usize) -> Vec<LogicBit> {
    let (sign, magnitude) = value.to_u32_digits();
    if sign != Sign::Minus {
        return biguint_to_bits_with_width(&BigUint::new(magnitude), width);
    }
    // Two's complement for negative values: invert all bits and add 1,
    // operating on the raw u32 limbs in O(width) rather than the O(width²)
    // BigInt modulus that was here before.
    let mut bits = Vec::with_capacity(width);
    let mut carry = 1u32;
    for i in 0..width {
        let limb_idx = i / 32;
        let bit_idx = i % 32;
        let raw_bit = if limb_idx < magnitude.len() {
            (magnitude[limb_idx] >> bit_idx) & 1
        } else {
            0
        };
        // Invert the magnitude bit, then add carry for two's complement.
        let inverted = raw_bit ^ 1;
        let sum = inverted + carry;
        carry = sum >> 1;
        bits.push(if sum & 1 == 1 {
            LogicBit::One
        } else {
            LogicBit::Zero
        });
    }
    bits
}

pub(crate) fn bits_to_biguint(bits: &[LogicBit]) -> BigUint {
    // Pack the bit vector into u32 limbs once and hand them to BigUint, rather
    // than the natural `acc | (one << index)` fold which reallocates and shifts
    // the entire BigUint per bit and degrades to O(width²) on wide operands.
    let mut digits = vec![0u32; bits.len().div_ceil(32)];
    for (index, bit) in bits.iter().enumerate() {
        if *bit == LogicBit::One {
            digits[index / 32] |= 1u32 << (index % 32);
        }
    }
    BigUint::new(digits)
}

pub(crate) fn bits_to_signed_bigint(bits: &[LogicBit]) -> BigInt {
    let unsigned = bits_to_biguint(bits);

    if !matches!(bits.last(), Some(LogicBit::One)) {
        return BigInt::from(unsigned);
    }

    BigInt::from_biguint(Sign::Plus, unsigned) - (BigInt::one() << bits.len())
}

// LRM 5.1.10 4-state truth tables.

pub(crate) fn bitwise_not_bit(a: LogicBit) -> LogicBit {
    match a {
        LogicBit::Zero => LogicBit::One,
        LogicBit::One => LogicBit::Zero,
        LogicBit::X | LogicBit::Z => LogicBit::X,
    }
}

pub(crate) fn bitwise_and_bits(a: LogicBit, b: LogicBit) -> LogicBit {
    // A definite 0 dominates, even against x/z. Otherwise any unknown poisons
    // the bit; only 1 & 1 yields 1.
    match (a, b) {
        (LogicBit::Zero, _) | (_, LogicBit::Zero) => LogicBit::Zero,
        (LogicBit::One, LogicBit::One) => LogicBit::One,
        _ => LogicBit::X,
    }
}

pub(crate) fn bitwise_or_bits(a: LogicBit, b: LogicBit) -> LogicBit {
    // Symmetric to AND with 1 dominating. 0 | 0 is the only definite-0 case.
    match (a, b) {
        (LogicBit::One, _) | (_, LogicBit::One) => LogicBit::One,
        (LogicBit::Zero, LogicBit::Zero) => LogicBit::Zero,
        _ => LogicBit::X,
    }
}

pub(crate) fn bitwise_xor_bits(a: LogicBit, b: LogicBit) -> LogicBit {
    // XOR has no dominator: any x/z makes the bit ambiguous.
    match (a, b) {
        (LogicBit::X | LogicBit::Z, _) | (_, LogicBit::X | LogicBit::Z) => LogicBit::X,
        (LogicBit::Zero, LogicBit::Zero) | (LogicBit::One, LogicBit::One) => LogicBit::Zero,
        _ => LogicBit::One,
    }
}

pub(crate) fn bitwise_xnor_bits(a: LogicBit, b: LogicBit) -> LogicBit {
    bitwise_not_bit(bitwise_xor_bits(a, b))
}
