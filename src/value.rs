use num_bigint::{BigInt, BigUint, Sign};
use num_traits::{One, Zero};

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

    pub(crate) fn group_size(self) -> usize {
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
    pub(crate) width: usize,
    pub(crate) signed: bool,
    pub(crate) base: Base,
    pub(crate) bits: Vec<LogicBit>,
    // True for literals parsed without an explicit size (LRM 3.5.1 default
    // width). Drives Table 5-22 footnote a's MSB-fill extension when the
    // propagated context is wider than the default. Always false for sized
    // literals and for any value produced by an operator.
    pub(crate) unsized_literal: bool,
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
            bits,
            unsized_literal: false,
        }
    }

    pub(crate) fn from_bigint(value: BigInt, width: usize, signed: bool, base: Base) -> Self {
        Self {
            width,
            signed,
            base,
            bits: bigint_to_bits_with_width(&value, width),
            unsized_literal: false,
        }
    }

    pub(crate) fn all_x(width: usize, signed: bool, base: Base) -> Self {
        Self {
            width,
            signed,
            base,
            bits: vec![LogicBit::X; width],
            unsized_literal: false,
        }
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

pub(crate) fn bigint_to_bits_with_width(value: &BigInt, width: usize) -> Vec<LogicBit> {
    let modulus = BigInt::one() << width;
    let normalized = ((value % &modulus) + &modulus) % &modulus;
    let unsigned = normalized
        .to_biguint()
        .expect("normalized modulo value should be non-negative");
    biguint_to_bits_with_width(&unsigned, width)
}

pub(crate) fn bits_to_biguint(bits: &[LogicBit]) -> BigUint {
    bits.iter()
        .enumerate()
        .fold(BigUint::zero(), |acc, (index, bit)| match bit {
            LogicBit::One => acc | (BigUint::one() << index),
            LogicBit::Zero | LogicBit::X | LogicBit::Z => acc,
        })
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
