//! Pure operator, metadata, truth-table, and numeric-conversion helpers.

use num_bigint::{BigInt, BigUint, Sign};
use num_traits::{FromPrimitive, One, ToPrimitive, Zero};

use super::ir::ExprMeta;

use crate::parser::{BinaryOp, MathFunctionKind, UnaryOp};
use crate::value::{
    Base, DisplayStyle, IntegerValue, LogicBit, bitwise_and_bits, bitwise_not_bit, bitwise_or_bits,
    bitwise_xor_bits,
};

// LRM §5.1.7: when one operand of a relational / equality is real, "the
// other operand shall be converted to an equivalent real value". Same
// principle applies to arithmetic with mixed real-int operands and to
// conditional branches where one side is real. LRM §3.5.3 specifies that
// x/z bits "shall be treated as zero upon conversion" — `bits_to_biguint`
// already does this, so the conversion runs unconditionally.
//
// `BigInt::to_f64` always returns `Some` (rounding huge magnitudes to ±∞),
// so the unwrap is total.
pub(super) fn integer_value_to_f64(value: &IntegerValue) -> f64 {
    value
        .as_bigint(value.signed)
        .to_f64()
        .expect("BigInt::to_f64 is total")
}

// LRM §3.5.3: implicit real→integer conversion rounds to nearest with
// ties-away-from-zero. Rust's f64::round implements that exactly. NaN
// and ±∞ have no integer image, surfaced as `None` so callers can apply
// whatever "no integer" handling fits their context (e.g. $itor's chain
// into x→0 below; an integer assignment lvalue would surface 32 bits of x
// the way $rtoi does).
pub(super) fn real_to_integer_bigint(value: f64) -> Option<BigInt> {
    if value.is_nan() || value.is_infinite() {
        return None;
    }
    let rounded = value.round();
    Some(BigInt::from_f64(rounded).expect("finite f64 rounds to a representable BigInt"))
}

// Reduce a real to its 1-bit logical value for !, &&, ||, and ?: cond.
// Verilog has no defined behavior for NaN, so it folds into x. Zero
// (including -0.0) is logical 0; every other finite value is logical 1.
pub(super) fn logical_value_of_real(value: f64) -> LogicBit {
    if value.is_nan() {
        LogicBit::X
    } else if value == 0.0 {
        LogicBit::Zero
    } else {
        LogicBit::One
    }
}

// Apply a real-result math function to `arity` operands popped from
// `real_vals`. Order on the stack matches push order (lhs at the bottom
// of the popped slice, rhs at the top).
pub(super) fn apply_real_math_function(kind: MathFunctionKind, args: &[f64]) -> f64 {
    if args.len() == 1 {
        let x = args[0];
        return match kind {
            MathFunctionKind::Ln => x.ln(),
            MathFunctionKind::Log10 => x.log10(),
            MathFunctionKind::Exp => x.exp(),
            MathFunctionKind::Sqrt => x.sqrt(),
            MathFunctionKind::Floor => x.floor(),
            MathFunctionKind::Ceil => x.ceil(),
            MathFunctionKind::Sin => x.sin(),
            MathFunctionKind::Cos => x.cos(),
            MathFunctionKind::Tan => x.tan(),
            MathFunctionKind::Asin => x.asin(),
            MathFunctionKind::Acos => x.acos(),
            MathFunctionKind::Atan => x.atan(),
            MathFunctionKind::Sinh => x.sinh(),
            MathFunctionKind::Cosh => x.cosh(),
            MathFunctionKind::Tanh => x.tanh(),
            MathFunctionKind::Asinh => x.asinh(),
            MathFunctionKind::Acosh => x.acosh(),
            MathFunctionKind::Atanh => x.atanh(),
            MathFunctionKind::Clog2
            | MathFunctionKind::Pow
            | MathFunctionKind::Atan2
            | MathFunctionKind::Hypot => unreachable!("kind handled by other arity branch"),
        };
    }
    let x = args[0];
    let y = args[1];
    match kind {
        // LRM 17.11 + README "Real numbers": $pow shares f64::powf with
        // the `**` operator on reals, so corner-case results
        // (0.0**0.0=1.0, negative**non-integral=NaN, 0.0**neg=±∞) match.
        MathFunctionKind::Pow => x.powf(y),
        MathFunctionKind::Atan2 => x.atan2(y),
        MathFunctionKind::Hypot => x.hypot(y),
        _ => unreachable!("kind handled by other arity branch"),
    }
}

pub(super) fn unary_op_name(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Plus => "+",
        UnaryOp::Minus => "-",
        UnaryOp::LogicalNot => "!",
        UnaryOp::BitwiseNot => "~",
        UnaryOp::ReductionAnd => "&",
        UnaryOp::ReductionNand => "~&",
        UnaryOp::ReductionOr => "|",
        UnaryOp::ReductionNor => "~|",
        UnaryOp::ReductionXor => "^",
        UnaryOp::ReductionXnor => "~^",
    }
}

pub(super) fn binary_op_name(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Subtract => "-",
        BinaryOp::Multiply => "*",
        BinaryOp::Divide => "/",
        BinaryOp::Modulus => "%",
        BinaryOp::Power => "**",
        BinaryOp::LessThan => "<",
        BinaryOp::GreaterThan => ">",
        BinaryOp::LessThanOrEqual => "<=",
        BinaryOp::GreaterThanOrEqual => ">=",
        BinaryOp::Equal => "==",
        BinaryOp::NotEqual => "!=",
        BinaryOp::CaseEqual => "===",
        BinaryOp::CaseNotEqual => "!==",
        BinaryOp::LogicalAnd => "&&",
        BinaryOp::LogicalOr => "||",
        BinaryOp::BitwiseAnd => "&",
        BinaryOp::BitwiseOr => "|",
        BinaryOp::BitwiseXor => "^",
        BinaryOp::BitwiseXnor => "~^",
        BinaryOp::LogicalShiftLeft => "<<",
        BinaryOp::LogicalShiftRight => ">>",
        BinaryOp::ArithmeticShiftLeft => "<<<",
        BinaryOp::ArithmeticShiftRight => ">>>",
    }
}

// Pure value-level relational comparison; mirrors `evaluate_relational_expr`'s
// bit-comparison logic but on already-evaluated, already-extended values.
pub(super) fn compute_relational_from_values(
    op: BinaryOp,
    lhs_value: &IntegerValue,
    rhs_value: &IntegerValue,
    signed: bool,
    context: Option<ExprMeta>,
) -> IntegerValue {
    if lhs_value.has_unknown_bits() || rhs_value.has_unknown_bits() {
        return widen_relational_result(IntegerValue::all_x(1, false, Base::Binary), context);
    }
    let lhs_int = lhs_value.as_bigint(signed);
    let rhs_int = rhs_value.as_bigint(signed);
    let comparison_result = match op {
        BinaryOp::LessThan => lhs_int < rhs_int,
        BinaryOp::GreaterThan => lhs_int > rhs_int,
        BinaryOp::LessThanOrEqual => lhs_int <= rhs_int,
        BinaryOp::GreaterThanOrEqual => lhs_int >= rhs_int,
        _ => unreachable!("non-relational op in compute_relational_from_values"),
    };
    let bit = if comparison_result {
        LogicBit::One
    } else {
        LogicBit::Zero
    };
    widen_relational_result(comparison_result_value(bit), context)
}

// Pure value-level equality comparison; mirrors `evaluate_equality_expr`.
pub(super) fn compute_equality_from_values(
    op: BinaryOp,
    lhs_value: &IntegerValue,
    rhs_value: &IntegerValue,
    context: Option<ExprMeta>,
) -> IntegerValue {
    let bit = match op {
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
                return widen_relational_result(
                    IntegerValue::all_x(1, false, Base::Binary),
                    context,
                );
            }
            let equal = !definite_mismatch;
            let result = if matches!(op, BinaryOp::Equal) {
                equal
            } else {
                !equal
            };
            if result {
                LogicBit::One
            } else {
                LogicBit::Zero
            }
        }
        BinaryOp::CaseEqual | BinaryOp::CaseNotEqual => {
            let equal = lhs_value.bits == rhs_value.bits;
            let result = if matches!(op, BinaryOp::CaseEqual) {
                equal
            } else {
                !equal
            };
            if result {
                LogicBit::One
            } else {
                LogicBit::Zero
            }
        }
        _ => unreachable!("non-equality op in compute_equality_from_values"),
    };
    widen_relational_result(comparison_result_value(bit), context)
}

pub(super) fn combine_binary_meta(
    op: BinaryOp,
    lhs_meta: ExprMeta,
    rhs_meta: ExprMeta,
) -> ExprMeta {
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

pub(super) fn shift_bits_left(bits: &[LogicBit], shift: usize) -> Vec<LogicBit> {
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

pub(super) fn shift_bits_right(bits: &[LogicBit], shift: usize, fill: LogicBit) -> Vec<LogicBit> {
    let width = bits.len();
    (0..width)
        .map(|i| match i.checked_add(shift) {
            Some(src) if src < width => bits[src],
            _ => fill,
        })
        .collect()
}

pub(super) fn comparison_result_value(bit: LogicBit) -> IntegerValue {
    IntegerValue::computed(1, false, Base::Binary, vec![bit])
}

pub(super) fn concatenated_display_style(items: &[IntegerValue]) -> DisplayStyle {
    let mut saw_string_bytes = false;
    for item in items {
        if item.width == 0 {
            continue;
        }
        if item.display_style != DisplayStyle::String || item.width % 8 != 0 {
            return DisplayStyle::Base;
        }
        saw_string_bytes = true;
    }
    if saw_string_bytes {
        DisplayStyle::String
    } else {
        DisplayStyle::Base
    }
}

// LRM 5.1.9: an operand reduces to its logical value before the
// !/&&/|| truth table applies. Any 1 bit makes the operand definitely
// true; all-zero is definitely false; otherwise (any x/z, no 1) the
// operand is ambiguous and reduces to x.
pub(super) fn logical_value(value: &IntegerValue) -> LogicBit {
    if value.bits.contains(&LogicBit::One) {
        LogicBit::One
    } else if value.bits.iter().all(|bit| *bit == LogicBit::Zero) {
        LogicBit::Zero
    } else {
        LogicBit::X
    }
}

// LRM 5.1.9 Tables 5-14 / 5-15: the `&&` / `||` truth table over two
// operands that have each already been reduced to a single logical bit by
// `logical_value` (integer) or `logical_value_of_real` (real). A definite 0
// dominates `&&` and a definite 1 dominates `||`; anything else with an
// unknown operand stays x.
pub(super) fn apply_logical_truth_table(op: BinaryOp, lhs: LogicBit, rhs: LogicBit) -> LogicBit {
    match op {
        BinaryOp::LogicalAnd => match (lhs, rhs) {
            (LogicBit::Zero, _) | (_, LogicBit::Zero) => LogicBit::Zero,
            (LogicBit::One, LogicBit::One) => LogicBit::One,
            _ => LogicBit::X,
        },
        BinaryOp::LogicalOr => match (lhs, rhs) {
            (LogicBit::One, _) | (_, LogicBit::One) => LogicBit::One,
            (LogicBit::Zero, LogicBit::Zero) => LogicBit::Zero,
            _ => LogicBit::X,
        },
        _ => unreachable!("apply_logical_truth_table got non-logical op"),
    }
}

// Concatenation/replication results are unsigned; if an outer context is
// wider, zero-extend (reusing the existing §5.5.4 path via
// `resized_to_context` with context_signed = false). If the outer context is
// narrower or absent we keep the natural width — concatenation is
// self-determined, so the joined width never shrinks below itself.
pub(super) fn extend_to_outer_context(
    value: IntegerValue,
    context: Option<ExprMeta>,
) -> IntegerValue {
    match context {
        Some(ctx) if ctx.width > value.width => value.resized_to_context(ctx.width, false),
        _ => value,
    }
}

// Outer-context widening for cast / conversion results ($signed, $unsigned,
// $rtoi, $realtobits, $clog2). Differs from `extend_to_outer_context` in that
// extension follows the *propagated* context's signedness (§5.5.2) rather than
// forcing zero-extension — the cast's own type already lives in the result.
pub(super) fn extend_cast_to_outer_context(
    value: IntegerValue,
    context: Option<ExprMeta>,
) -> IntegerValue {
    match context {
        Some(ctx) if ctx.width > value.width => value.resized_to_context(ctx.width, ctx.signed),
        _ => value,
    }
}

pub(super) fn real_to_integer_value(value: f64) -> IntegerValue {
    if value.is_nan() || value.is_infinite() {
        return IntegerValue::all_x(32, true, Base::Decimal);
    }
    let truncated = value.trunc();
    let bigint =
        BigInt::from_f64(truncated).expect("finite f64 truncates to a representable BigInt");
    IntegerValue::from_bigint(bigint, 32, true, Base::Decimal)
}

pub(super) fn clog2_result_value(value: BigUint) -> IntegerValue {
    let result = if value.is_zero() {
        BigUint::zero()
    } else {
        BigUint::from((value - BigUint::one()).bits())
    };
    IntegerValue::from_bigint(BigInt::from(result), 32, true, Base::Decimal)
}

// LRM 17.8: $bitstoreal reinterprets a 64-bit operand as an IEEE 754
// double. Width is enforced to be exactly 64 by the caller, so the loop
// just packs the 64 LogicBits into a u64. x/z bits map to 0, mirroring
// §3.5.3's integer-to-real conversion rule — `$bitstoreal` is a sibling
// conversion in the same clause and the LRM doesn't carve out a different
// rule for it.
//
// Must stay a pure u64-pack-then-`f64::from_bits` transmute: any NaN
// canonicalization here would break the
// `$realtobits($bitstoreal(x)) == x` round-trip on non-finite payloads
// (matching iverilog), which the test suite pins down.
pub(super) fn bits_value_to_real(value: &IntegerValue) -> f64 {
    let mut bits = 0u64;
    for (index, bit) in value.bits.iter().enumerate() {
        if *bit == LogicBit::One {
            bits |= 1u64 << index;
        }
    }
    f64::from_bits(bits)
}

pub(super) fn widen_relational_result(
    result: IntegerValue,
    context: Option<ExprMeta>,
) -> IntegerValue {
    match context {
        Some(ctx) if ctx.width > 1 => result.resized_to_context(ctx.width, false),
        _ => result,
    }
}

// Integer `**` per LRM Table 5-3. `width` is the result width (L(base));
// the LRM guarantees the result is only `width` bits wide, so we compute
// `base ** exponent mod 2^width` via modular exponentiation instead of
// materialising the full-precision power and truncating afterwards. This
// keeps every intermediate bounded by `width` bits, matching iverilog /
// Verilator and avoiding the pathological blow-up on huge exponents
// (e.g. `3 ** 32'd200000000`). The special small-magnitude results
// (0 / 1 / -1) are returned directly and truncated to `width` by the
// caller's `IntegerValue::from_bigint`.
pub(super) fn evaluate_power(
    base: BigInt,
    exponent: BigInt,
    width: usize,
) -> Result<BigInt, String> {
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
            // Parity is sign-invariant under num-bigint's two's-complement
            // BitAnd (e.g. -3 & 1 == 3 & 1 == 1), so we read the low bit
            // of the exponent directly instead of allocating its absolute
            // value first. Consumes `exponent`, which is fine — we return
            // immediately on every branch below.
            let is_odd = (exponent & BigInt::one()) == BigInt::one();
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

    // Modulus is 2^width. Reduce the (possibly negative) base into
    // [0, 2^width) first — `(a mod m)^e mod m == a^e mod m`, and folding a
    // negative base up by the modulus yields its two's-complement residue,
    // which is exactly what the caller reinterprets under the result's
    // signedness. A zero-width result collapses to the empty modulus 1.
    let modulus = BigUint::one() << width;
    let mut base_res = base % BigInt::from(modulus.clone());
    if base_res.sign() == Sign::Minus {
        base_res += BigInt::from(modulus.clone());
    }
    let base_res = base_res
        .to_biguint()
        .expect("residue folded into [0, 2^width) is non-negative");
    let result = base_res.modpow(&exponent, &modulus);
    Ok(BigInt::from(result))
}

// LRM 5.1.11 reduction: fold the binary operator across all operand bits.
// Identity element matches the operator (AND uses 1; OR and XOR use 0);
// the negated forms NAND/NOR/XNOR invert the fold result. Reusing the
// binary truth tables from Phase 6a keeps x/z propagation identical: e.g.
// AND-reduction still gives 0 when any bit is 0 (even with x/z elsewhere),
// because `bitwise_and_bits(0, x)` returns 0.
pub(super) fn reduce_bits(op: UnaryOp, bits: &[LogicBit]) -> LogicBit {
    let folded = match op {
        UnaryOp::ReductionAnd | UnaryOp::ReductionNand => {
            bits.iter().copied().fold(LogicBit::One, bitwise_and_bits)
        }
        UnaryOp::ReductionOr | UnaryOp::ReductionNor => {
            bits.iter().copied().fold(LogicBit::Zero, bitwise_or_bits)
        }
        UnaryOp::ReductionXor | UnaryOp::ReductionXnor => {
            bits.iter().copied().fold(LogicBit::Zero, bitwise_xor_bits)
        }
        _ => unreachable!("reduce_bits called with non-reduction op"),
    };
    match op {
        UnaryOp::ReductionNand | UnaryOp::ReductionNor | UnaryOp::ReductionXnor => {
            bitwise_not_bit(folded)
        }
        _ => folded,
    }
}
