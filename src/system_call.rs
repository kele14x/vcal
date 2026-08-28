use num_traits::ToPrimitive;

use crate::Session;
use crate::eval;
use crate::parser::{MathFunctionKind, RealConversionKind, SystemArg, SystemTask};
use crate::value::{self, Base, DisplayStyle, IntegerValue, LogicBit, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SystemCallKind {
    Function(SystemFunction),
    Task(SystemTask),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SystemFunction {
    Math(MathFunctionKind),
    RealConversion(RealConversionKind),
    SignCast { signed: bool },
    BaseCast(Base),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SystemTaskResult {
    pub(crate) output: Vec<u8>,
    pub(crate) should_exit: bool,
}

pub(crate) fn classify_system_call(name: &str) -> Result<SystemCallKind, String> {
    if let Some(task) = SystemTask::from_name(name) {
        return Ok(SystemCallKind::Task(task));
    }
    Ok(match name {
        "$signed" => SystemCallKind::Function(SystemFunction::SignCast { signed: true }),
        "$unsigned" => SystemCallKind::Function(SystemFunction::SignCast { signed: false }),
        "$bin" => SystemCallKind::Function(SystemFunction::BaseCast(Base::Binary)),
        "$oct" => SystemCallKind::Function(SystemFunction::BaseCast(Base::Octal)),
        "$dec" => SystemCallKind::Function(SystemFunction::BaseCast(Base::Decimal)),
        "$hex" => SystemCallKind::Function(SystemFunction::BaseCast(Base::Hex)),
        "$rtoi" => SystemCallKind::Function(SystemFunction::RealConversion(
            RealConversionKind::RealToInteger,
        )),
        "$itor" => SystemCallKind::Function(SystemFunction::RealConversion(
            RealConversionKind::IntegerToReal,
        )),
        "$realtobits" => SystemCallKind::Function(SystemFunction::RealConversion(
            RealConversionKind::RealToBits,
        )),
        "$bitstoreal" => SystemCallKind::Function(SystemFunction::RealConversion(
            RealConversionKind::BitsToReal,
        )),
        _ => match MathFunctionKind::from_name(name) {
            Some(math_kind) => SystemCallKind::Function(SystemFunction::Math(math_kind)),
            None => return Err(format!("unknown system identifier: {name}")),
        },
    })
}

pub(crate) fn task_in_expression_error(name: &str) -> String {
    format!("{name}() is a system task, it cannot be called as a function.")
}

pub(crate) fn execute_task(
    task: SystemTask,
    args: &[SystemArg],
    session: &Session,
) -> Result<SystemTaskResult, String> {
    match task {
        SystemTask::Finish | SystemTask::Stop => Ok(SystemTaskResult {
            output: Vec::new(),
            should_exit: true,
        }),
        SystemTask::Display
        | SystemTask::DisplayB
        | SystemTask::DisplayO
        | SystemTask::DisplayH
        | SystemTask::Write
        | SystemTask::WriteB
        | SystemTask::WriteO
        | SystemTask::WriteH => Ok(SystemTaskResult {
            output: format_display_args(
                args,
                session,
                task.appends_newline(),
                task.default_base(),
            )?,
            should_exit: false,
        }),
    }
}

fn format_display_args(
    args: &[SystemArg],
    session: &Session,
    append_newline: bool,
    default_base: Base,
) -> Result<Vec<u8>, String> {
    let values = evaluate_display_args(args, session)?;

    let mut output = if let Some((first, rest)) = values.split_first() {
        if let Some(format_bytes) = format_arg_string_bytes(first) {
            format_with_controls(&format_bytes, rest, UnformattedStyle::Base(default_base))?
        } else {
            join_default_values(&values, default_base)
        }
    } else {
        Vec::new()
    };

    if append_newline {
        output.push(b'\n');
    }
    Ok(output)
}

// REPL calculator output is a unified one-or-more display-argument list.
// A leading string in a multi-argument list uses the same control walker as
// `$display`, but arguments left unconsumed by controls keep vcal's canonical
// width / signed / base rendering. Otherwise every argument is canonical too,
// extending the traditional single-expression echo to `a, b` without losing
// Verilog value metadata.
// Requiring another argument preserves the established bare-string echo:
// `"hello"` remains the canonical escaped string rather than silently becoming
// a no-argument format string.
pub(crate) fn format_repl_echo_args(
    args: &[SystemArg],
    session: &Session,
) -> Result<Vec<u8>, String> {
    let values = evaluate_display_args(args, session)?;
    let Some((first, rest)) = values.split_first() else {
        return Ok(Vec::new());
    };

    if !rest.is_empty()
        && let Some(format_bytes) = format_arg_string_bytes(first)
    {
        format_with_controls(&format_bytes, rest, UnformattedStyle::Canonical)
    } else {
        Ok(join_canonical_values(&values))
    }
}

fn evaluate_display_args(args: &[SystemArg], session: &Session) -> Result<Vec<DisplayArg>, String> {
    args.iter()
        .map(|arg| match arg {
            SystemArg::Expr(expr) => eval::evaluate_expr(expr, session).map(DisplayArg::Value),
            SystemArg::Null => Ok(DisplayArg::Null),
        })
        .collect()
}

enum DisplayArg {
    Value(Value),
    Null,
}

#[derive(Clone, Copy)]
enum UnformattedStyle {
    Base(Base),
    Canonical,
}

fn format_with_controls(
    format_bytes: &[u8],
    args: &[DisplayArg],
    unformatted_style: UnformattedStyle,
) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut arg_index = 0usize;
    let mut index = 0usize;

    while index < format_bytes.len() {
        let byte = format_bytes[index];
        if byte != b'%' {
            output.push(byte);
            index += 1;
            continue;
        }

        index += 1;
        if index == format_bytes.len() {
            return Err("display format control `%` is missing a specifier".to_string());
        }

        let specifier = format_bytes[index] as char;
        index += 1;

        if specifier == '%' {
            output.push(b'%');
            continue;
        }

        let Some(value) = args.get(arg_index) else {
            return Err(format!(
                "display format %{specifier} expects an argument, got {}",
                args.len()
            ));
        };
        arg_index += 1;

        if let DisplayArg::Null = value {
            output.push(b' ');
            continue;
        }

        let DisplayArg::Value(value) = value else {
            unreachable!("null display argument handled above");
        };

        match specifier {
            'b' | 'B' => push_text(&mut output, &format_integer_base(value, Base::Binary)),
            'o' | 'O' => push_text(&mut output, &format_integer_base(value, Base::Octal)),
            'd' | 'D' => push_text(&mut output, &format_integer_base(value, Base::Decimal)),
            'h' | 'H' | 'x' | 'X' => push_text(&mut output, &format_integer_base(value, Base::Hex)),
            'c' | 'C' => output.extend(format_char_value(value)),
            's' | 'S' => output.extend(format_string_value(value)),
            'f' | 'F' | 'e' | 'E' | 'g' | 'G' => {
                push_text(&mut output, &format_real_value(value, specifier));
            }
            _ => return Err(format!("unsupported display format control `%{specifier}`")),
        }
    }

    for value in &args[arg_index..] {
        match value {
            DisplayArg::Value(_) => {
                if !output.is_empty()
                    && !output.last().is_some_and(|byte| byte.is_ascii_whitespace())
                {
                    output.push(b' ');
                }
            }
            DisplayArg::Null => {}
        }
        output.extend(format_unformatted_arg(value, unformatted_style));
    }

    Ok(output)
}

fn format_unformatted_arg(value: &DisplayArg, style: UnformattedStyle) -> Vec<u8> {
    match style {
        UnformattedStyle::Base(base) => format_default_arg(value, base),
        UnformattedStyle::Canonical => match value {
            DisplayArg::Value(value) => value.canonical().into_bytes(),
            DisplayArg::Null => vec![b' '],
        },
    }
}

fn join_default_values(values: &[DisplayArg], default_base: Base) -> Vec<u8> {
    let mut output = Vec::new();
    let mut previous_was_value = false;
    for (index, value) in values.iter().enumerate() {
        let current_is_value = matches!(value, DisplayArg::Value(_));
        if index > 0 && previous_was_value && current_is_value {
            output.push(b' ');
        }
        output.extend(format_default_arg(value, default_base));
        previous_was_value = current_is_value;
    }
    output
}

fn join_canonical_values(values: &[DisplayArg]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut previous_was_value = false;
    for (index, value) in values.iter().enumerate() {
        let current_is_value = matches!(value, DisplayArg::Value(_));
        if index > 0 && previous_was_value && current_is_value {
            output.push(b' ');
        }
        match value {
            DisplayArg::Value(value) => output.extend(value.canonical().bytes()),
            DisplayArg::Null => output.push(b' '),
        }
        previous_was_value = current_is_value;
    }
    output
}

fn format_default_arg(value: &DisplayArg, default_base: Base) -> Vec<u8> {
    match value {
        DisplayArg::Value(value) => format_default_value(value, default_base),
        DisplayArg::Null => vec![b' '],
    }
}

fn format_arg_string_bytes(value: &DisplayArg) -> Option<Vec<u8>> {
    match value {
        DisplayArg::Value(value) => format_string_bytes(value),
        DisplayArg::Null => None,
    }
}

fn format_string_bytes(value: &Value) -> Option<Vec<u8>> {
    match value {
        Value::Integer(integer) if integer.display_style == DisplayStyle::String => {
            integer.render_string_bytes()
        }
        Value::Integer(_) | Value::Real(_) => None,
    }
}

fn format_default_value(value: &Value, default_base: Base) -> Vec<u8> {
    match format_string_bytes(value) {
        Some(bytes) => bytes,
        None => match value {
            Value::Integer(integer) => integer.format_digits_in_base(default_base).into_bytes(),
            Value::Real(_) => value.canonical().into_bytes(),
        },
    }
}

fn format_string_value(value: &Value) -> Vec<u8> {
    match value {
        Value::Integer(integer) => format_string_control_bytes(integer),
        Value::Real(_) => value.canonical().into_bytes(),
    }
}

fn format_string_control_bytes(integer: &IntegerValue) -> Vec<u8> {
    let padded_width = integer.width.div_ceil(8) * 8;
    let byte_count = padded_width / 8;
    let mut bytes = Vec::with_capacity(byte_count);

    for byte_index in (0..byte_count).rev() {
        bytes.push(byte_from_integer_bits_zeroing_unknowns(
            integer,
            byte_index * 8,
        ));
    }

    bytes
}

fn format_char_value(value: &Value) -> Vec<u8> {
    match value {
        Value::Integer(integer) => vec![byte_from_integer_bits_zeroing_unknowns(integer, 0)],
        Value::Real(_) => value.canonical().into_bytes(),
    }
}

fn byte_from_integer_bits_zeroing_unknowns(integer: &IntegerValue, start: usize) -> u8 {
    let mut byte = 0u8;
    for bit_index in 0..8 {
        let absolute_bit = start + bit_index;
        let bit = if absolute_bit < integer.width {
            integer
                .bits
                .get(absolute_bit)
                .copied()
                .unwrap_or(LogicBit::Zero)
        } else {
            LogicBit::Zero
        };

        match bit {
            LogicBit::Zero => {}
            LogicBit::One => byte |= 1 << bit_index,
            LogicBit::X | LogicBit::Z => {}
        }
    }
    byte
}

fn push_text(output: &mut Vec<u8>, text: &str) {
    output.extend_from_slice(text.as_bytes());
}

fn format_integer_base(value: &Value, base: Base) -> String {
    match value {
        Value::Integer(integer) => integer.format_digits_in_base(base),
        Value::Real(real) => value::format_real(*real),
    }
}

fn format_real_value(value: &Value, specifier: char) -> String {
    let real = match value {
        Value::Real(real) => *real,
        Value::Integer(integer) => integer
            .as_bigint(integer.signed)
            .to_f64()
            .expect("BigInt::to_f64 is total"),
    };

    match specifier {
        'e' => format!("{real:e}"),
        'E' => format!("{real:E}"),
        'f' | 'F' => format_fixed_real(real),
        'g' => value::format_real(real),
        'G' => uppercase_exponent(&value::format_real(real)),
        _ => unreachable!("caller only passes real display controls"),
    }
}

fn format_fixed_real(real: f64) -> String {
    if real.is_nan() || real.is_infinite() {
        return value::format_real(real);
    }

    let formatted = format!("{real}");
    if formatted.contains('.') || formatted.contains('e') || formatted.contains('E') {
        formatted
    } else {
        format!("{formatted}.0")
    }
}

fn uppercase_exponent(text: &str) -> String {
    text.replace('e', "E")
}
