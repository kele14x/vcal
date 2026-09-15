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
    let prepared = prepare_display_args(args, session, false)?;

    let mut output = match &prepared.format_bytes {
        Some(format_bytes) => format_with_controls(
            format_bytes,
            prepared.consumable_args(),
            UnformattedStyle::Base(default_base),
        )?,
        None => join_default_values(&prepared.values, default_base),
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
    let prepared = prepare_display_args(args, session, true)?;

    Ok(match &prepared.format_bytes {
        Some(format_bytes) => format_with_controls(
            format_bytes,
            prepared.consumable_args(),
            UnformattedStyle::Canonical,
        )?,
        None => join_canonical_values(&prepared.values),
    })
}

fn evaluate_display_args(args: &[SystemArg], session: &Session) -> Result<Vec<DisplayArg>, String> {
    args.iter()
        .map(|arg| match arg {
            SystemArg::Expr(expr) => eval::evaluate_expr(expr, session).map(DisplayArg::Value),
            SystemArg::Null => Ok(DisplayArg::Null),
        })
        .collect()
}

// An evaluated display-argument list, plus the format string its first
// argument folded to when the list runs in format mode at all.
struct PreparedDisplay {
    values: Vec<DisplayArg>,
    format_bytes: Option<Vec<u8>>,
}

impl PreparedDisplay {
    // The arguments format controls may consume: everything after the format
    // string. Empty for a zero-argument call, where `format_bytes` is `None`.
    fn consumable_args(&self) -> &[DisplayArg] {
        self.values.split_first().map_or(&[], |(_, rest)| rest)
    }
}

// Static-semantics check for one display-argument list, shaped like
// `eval::semantic_check`: evaluate every argument in source order — so an
// identifier or type error still wins over anything about the format string,
// exactly as it did when this check lived in the renderer — then validate the
// controls against the number of arguments available to consume them. The
// evaluated arguments come back out, so nothing is computed twice.
//
// A format string is always literal-derived, which makes this a static
// property of the input rather than a runtime condition: `DisplayStyle::String`
// originates only at `Expr::StringLiteral` and survives only all-string
// concatenation / replication, while a packed `reg` stays an ordinary numeric
// vector even when assigned a string literal (doc/non-standard.md → "Top-level
// input"). iverilog agrees — `$display(s, x)` on `reg [63:0] s = "v=%d"`
// prints two numbers, never a formatted string.
//
// Format rejections pick up the "Semantic error: " stage prefix here rather
// than in the message text, so the prefix stays injected at a stage boundary
// the way `parse_statements`'s and `semantic_check`'s call sites do it.
// Argument errors arrive already prefixed from `eval::evaluate_expr`.
fn prepare_display_args(
    args: &[SystemArg],
    session: &Session,
    require_extra_arg: bool,
) -> Result<PreparedDisplay, String> {
    let values = evaluate_display_args(args, session)?;
    let format_bytes = format_mode_bytes(&values, require_extra_arg);

    if let Some(format_bytes) = &format_bytes {
        // `format_bytes` is `Some` only when `split_first` succeeded, so there
        // is at least one value and the count cannot underflow.
        let available = values.len() - 1;
        check_format_controls(format_bytes, available)
            .map_err(|e| format!("Semantic error: {e}"))?;
    }

    Ok(PreparedDisplay {
        values,
        format_bytes,
    })
}

// Decides format mode and folds the format string in one step, so the
// validation pass and the renderer cannot disagree about which mode an
// argument list is in. `require_extra_arg` is the echo-list gate: a lone
// `"hello"` stays a canonical string echo instead of becoming a zero-argument
// format string, whereas `$display("value: %")` has no such gate and still
// reports its trailing `%`.
fn format_mode_bytes(values: &[DisplayArg], require_extra_arg: bool) -> Option<Vec<u8>> {
    let (first, rest) = values.split_first()?;
    if require_extra_arg && rest.is_empty() {
        return None;
    }
    format_arg_string_bytes(first)
}

// Pure scan of a format string's controls against the number of arguments
// available to consume them. Precedence matches the renderer's historical
// order — arity before specifier support — so `$display("%z")` still reports
// the missing argument instead of naming an unsupported `%z`. Message text is
// pinned by src/tests/display.rs.
fn check_format_controls(format_bytes: &[u8], available: usize) -> Result<(), String> {
    let mut index = 0usize;
    let mut consumed = 0usize;

    while index < format_bytes.len() {
        if format_bytes[index] != b'%' {
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
            continue;
        }

        if consumed == available {
            return Err(format!(
                "display format %{specifier} expects an argument, got {available}"
            ));
        }
        consumed += 1;

        if !is_supported_format_control(specifier) {
            return Err(format!("unsupported display format control `%{specifier}`"));
        }
    }

    Ok(())
}

// The set of format controls vcal implements, as a predicate so the static
// check above cannot drift from the renderer's `match`. The renderer keeps the
// authoritative spelling because it also has to choose a rendering per control;
// its fallthrough arm asserts the two agree.
fn is_supported_format_control(specifier: char) -> bool {
    matches!(
        specifier,
        'b' | 'B'
            | 'o'
            | 'O'
            | 'd'
            | 'D'
            | 'h'
            | 'H'
            | 'x'
            | 'X'
            | 'c'
            | 'C'
            | 's'
            | 'S'
            | 'f'
            | 'F'
            | 'e'
            | 'E'
            | 'g'
            | 'G'
    )
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

// Renders an already-validated format string. `prepare_display_args` runs
// `check_format_controls` over the same bytes before getting here — the same
// "callers validate first" contract `eval::replication_count_value` documents —
// so the three `Err` arms below are a defensive backstop rather than a
// user-visible path, and rejections reach the user as `Semantic error:` from
// the validation pass. They stay `Err` rather than `unreachable!` so the
// renderer remains total if that contract is ever broken.
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

        // This `match` is the single source of truth for which format
        // controls vcal implements, and it runs before the null-argument
        // shortcut inside `push_formatted`. That ordering matters: a null
        // argument renders as a space whatever the specifier asks for, so
        // validating afterwards would let `$display("%q", )` print a space
        // where `$display("%q", 5)` correctly reports the bad specifier.
        match specifier {
            'b' | 'B' => push_formatted(&mut output, value, |value| {
                format_integer_base(value, Base::Binary).into_bytes()
            }),
            'o' | 'O' => push_formatted(&mut output, value, |value| {
                format_integer_base(value, Base::Octal).into_bytes()
            }),
            'd' | 'D' => push_formatted(&mut output, value, |value| {
                format_integer_base(value, Base::Decimal).into_bytes()
            }),
            'h' | 'H' | 'x' | 'X' => push_formatted(&mut output, value, |value| {
                format_integer_base(value, Base::Hex).into_bytes()
            }),
            'c' | 'C' => push_formatted(&mut output, value, format_char_value),
            's' | 'S' => push_formatted(&mut output, value, format_string_value),
            'f' | 'F' | 'e' | 'E' | 'g' | 'G' => push_formatted(&mut output, value, |value| {
                format_real_value(value, specifier).into_bytes()
            }),
            _ => {
                debug_assert!(
                    !is_supported_format_control(specifier),
                    "`is_supported_format_control` and the render match disagree on `{specifier}`"
                );
                return Err(format!("unsupported display format control `%{specifier}`"));
            }
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

// LRM 17.1.1.4: a null argument renders as a single space whatever the
// format specifier asks for, so only the value case reaches `render`. The
// caller validates the specifier before getting here.
fn push_formatted(output: &mut Vec<u8>, arg: &DisplayArg, render: impl FnOnce(&Value) -> Vec<u8>) {
    match arg {
        DisplayArg::Null => output.push(b' '),
        DisplayArg::Value(value) => output.extend(render(value)),
    }
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
