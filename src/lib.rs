use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive};

mod color;
mod eval;
mod highlight;
mod lexer;
mod parser;
mod system_call;
mod value;

#[cfg(test)]
mod tests;

use value::DisplayStyle;
pub use value::{Base, IntegerValue, LogicBit, Value};

use parser::{DeclKind, DeclName, Expr, LValue, SelectKind, Stmt};
use system_call::SystemCallKind;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RegRange {
    pub(crate) msb: BigInt,
    pub(crate) lsb: BigInt,
}

// A reg's payload. `Vector` is the existing scalar/vector reg; `Array`
// is the 1-D unpacked-array of vectors (`reg [3:0] a [0:15]` or
// `integer a [0:3]` — both end up here, since an `integer` element is
// just a 32-bit signed decimal vector); `Real` is the LRM 4.8 IEEE 754
// binary64 form introduced by `real r`; `RealArray` is the 1-D
// unpacked-array of reals (`real r [0:3]`). The packed range (the
// `[3:0]` part of a vector array) still lives in `RegValue::range`;
// `Array.dim` / `RealArray.dim` hold the *unpacked* dimension
// (`[0:15]`). Vector-array elements are IntegerValues all-x at decl
// time; real-array elements are f64 zeros at decl time (LRM 4.8:
// reals init to 0). `Eq` can't be derived once `Real` / `RealArray`
// are in the mix because `f64` has no equivalence relation
// (NaN ≠ NaN); `PartialEq` is all the surrounding code needs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RegStorage {
    Vector(IntegerValue),
    Array {
        dim: RegRange,
        elements: Vec<IntegerValue>,
    },
    Real(f64),
    RealArray {
        dim: RegRange,
        elements: Vec<f64>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RegValue {
    pub(crate) range: Option<RegRange>,
    pub(crate) storage: RegStorage,
}

impl RegValue {
    // True for the vector-array form (`reg [3:0] a [0:15]`,
    // `integer a [0:3]`). Distinct from a vector reg because reading
    // the bare name is illegal for an array and array-element access
    // is illegal for a vector.
    pub(crate) fn is_array(&self) -> bool {
        matches!(self.storage, RegStorage::Array { .. })
    }

    // True for the real-array form (`real r [0:3]`). Mirrors
    // `is_array` but for the f64-element path — the two array storages
    // share no read/write code, so eval dispatch needs to tell them
    // apart up front.
    pub(crate) fn is_real_array(&self) -> bool {
        matches!(self.storage, RegStorage::RealArray { .. })
    }

    // True for the real form (`real r`). Distinct from a vector reg
    // because every integer-pipeline accessor — `vector`, the validator's
    // identifier arm, `lvalue_meta` for a bare-name LHS — has to peel off
    // and route real-typed identifiers through the f64 path. Eval uses
    // it to drive the result-type decision in `expression_is_real`.
    pub(crate) fn is_real(&self) -> bool {
        matches!(self.storage, RegStorage::Real(_))
    }

    // Vector-only accessor. Errors with the canonical "array name
    // cannot be used as a value" / "real `r` cannot be used as an
    // integer value" diagnostics when the reg is non-vector. Used
    // everywhere a vector-only path resolves an identifier — it keeps
    // the rejection uniform without duplicating the error strings at
    // each callsite.
    pub(crate) fn require_vector(&self, name: &str) -> Result<&IntegerValue, String> {
        match &self.storage {
            RegStorage::Vector(value) => Ok(value),
            RegStorage::Array { .. } | RegStorage::RealArray { .. } => {
                Err(format!("array `{name}` cannot be used as a value"))
            }
            RegStorage::Real(_) => Err(format!("real `{name}` cannot be used as an integer value")),
        }
    }

    pub(crate) fn vector_mut(&mut self) -> Option<&mut IntegerValue> {
        match &mut self.storage {
            RegStorage::Vector(value) => Some(value),
            RegStorage::Array { .. } | RegStorage::Real(_) | RegStorage::RealArray { .. } => None,
        }
    }

    // Vector-array accessor. Mirrors `vector()`: returns the unpacked
    // dimension and the element slice for a vector array, `None`
    // otherwise. Keeps `RegStorage` private to lib.rs while letting
    // eval dispatch on the storage kind via `is_array()` + `array()`.
    pub(crate) fn array(&self) -> Option<(&RegRange, &[IntegerValue])> {
        match &self.storage {
            RegStorage::Array { dim, elements } => Some((dim, elements.as_slice())),
            RegStorage::Vector(_) | RegStorage::Real(_) | RegStorage::RealArray { .. } => None,
        }
    }

    // Mutable variant of `array()`. Used by the array-element-write
    // path so the element at the chosen index can be replaced in-place
    // on a staged variable map clone, without exposing `RegStorage` to
    // eval.rs.
    pub(crate) fn array_mut(&mut self) -> Option<(&RegRange, &mut [IntegerValue])> {
        match &mut self.storage {
            RegStorage::Array { dim, elements } => Some((dim, elements.as_mut_slice())),
            RegStorage::Vector(_) | RegStorage::Real(_) | RegStorage::RealArray { .. } => None,
        }
    }

    // Real-array accessor — sibling of `array()` for the f64-element
    // form. Element selects on a `real r [0:3]` go through this so the
    // f64 payload stays untouched by the integer pipeline.
    pub(crate) fn real_array(&self) -> Option<(&RegRange, &[f64])> {
        match &self.storage {
            RegStorage::RealArray { dim, elements } => Some((dim, elements.as_slice())),
            RegStorage::Vector(_) | RegStorage::Array { .. } | RegStorage::Real(_) => None,
        }
    }

    // Mutable variant of `real_array()`. Used by the real-array
    // element-write path so the chosen element's f64 can be overwritten
    // in place on a staged map clone.
    pub(crate) fn real_array_mut(&mut self) -> Option<(&RegRange, &mut [f64])> {
        match &mut self.storage {
            RegStorage::RealArray { dim, elements } => Some((dim, elements.as_mut_slice())),
            RegStorage::Vector(_) | RegStorage::Array { .. } | RegStorage::Real(_) => None,
        }
    }

    // Real-only accessor. Returns the f64 payload for a real reg,
    // `None` for a vector or array. Used by the eval path's
    // `expression_is_real` (to detect when an identifier feeds the real
    // pipeline) and `evaluate_expr_as_real` (to load the value).
    pub(crate) fn real(&self) -> Option<f64> {
        match &self.storage {
            RegStorage::Real(value) => Some(*value),
            RegStorage::Vector(_) | RegStorage::Array { .. } | RegStorage::RealArray { .. } => None,
        }
    }

    // Mutable variant of `real()`. Used by the real-LHS assignment path
    // in `apply_stmt` so the staged-variable map's f64 can be overwritten
    // in place without exposing `RegStorage` to eval.rs.
    pub(crate) fn real_mut(&mut self) -> Option<&mut f64> {
        match &mut self.storage {
            RegStorage::Real(value) => Some(value),
            RegStorage::Vector(_) | RegStorage::Array { .. } | RegStorage::RealArray { .. } => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Evaluation {
    pub task_output: Vec<u8>,
    pub output: String,
    pub should_exit: bool,
}

// Persistent REPL state: the variable map that survives across `eval` calls.
// A fresh session has no variables; `evaluate_input` keeps the old stateless
// shape by spinning up a throwaway session.
#[derive(Debug, Default)]
pub struct Session {
    variables: HashMap<String, RegValue>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn lookup(&self, name: &str) -> Option<&RegValue> {
        self.variables.get(name)
    }

    #[cfg(test)]
    pub(crate) fn lookup_reg_range(&self, name: &str) -> Option<(&BigInt, &BigInt)> {
        self.lookup(name)
            .and_then(|reg| reg.range.as_ref().map(|range| (&range.msb, &range.lsb)))
    }

    // Test helper: returns (msb, lsb, element_count) for an array reg, or
    // `None` for a vector reg / undeclared name. Keeps the array storage
    // shape behind a single read accessor so tests don't have to import
    // `RegStorage`.
    #[cfg(test)]
    pub(crate) fn lookup_reg_array(&self, name: &str) -> Option<(BigInt, BigInt, usize)> {
        self.lookup(name).and_then(|reg| match &reg.storage {
            RegStorage::Array { dim, elements } => {
                Some((dim.msb.clone(), dim.lsb.clone(), elements.len()))
            }
            RegStorage::Vector(_) | RegStorage::Real(_) | RegStorage::RealArray { .. } => None,
        })
    }

    // Test helper: returns (msb, lsb, element_count) for a real array,
    // mirroring `lookup_reg_array` for the f64-element form.
    #[cfg(test)]
    pub(crate) fn lookup_reg_real_array(&self, name: &str) -> Option<(BigInt, BigInt, usize)> {
        self.lookup(name).and_then(|reg| match &reg.storage {
            RegStorage::RealArray { dim, elements } => {
                Some((dim.msb.clone(), dim.lsb.clone(), elements.len()))
            }
            RegStorage::Vector(_) | RegStorage::Array { .. } | RegStorage::Real(_) => None,
        })
    }

    // Test helper: returns the f64 payload for a `real` reg, `None` for
    // any other storage shape. Mirrors `lookup_reg_range` / `lookup_reg_array`
    // so tests can assert the real-reg pipeline keeps a value through
    // assignment / arithmetic without importing `RegStorage`.
    #[cfg(test)]
    pub(crate) fn lookup_reg_real(&self, name: &str) -> Option<f64> {
        self.lookup(name).and_then(|reg| reg.real())
    }

    pub fn eval(&mut self, input: &str) -> Result<Evaluation, String> {
        evaluate_input_with_session(self, input)
    }
}

pub fn evaluate_input(input: &str) -> Result<Evaluation, String> {
    let mut session = Session::new();
    session.eval(input)
}

// Default depth at which `parse_input` truncates the AST before
// `{:#?}` rendering. The auto-derived `Debug` impl recurses one stack
// frame per `Box<Expr>` level, so without truncation a 10^4-deep input
// — even though the parser builds it iteratively — would crash during
// formatting. 64 is deeper than any plausible human-written expression,
// shallow enough that the bounded-depth render can't overflow on a
// default thread stack.
pub const DEFAULT_DISPLAY_DEPTH: usize = 64;

// Debug entry point: run the parser only and return the AST as a
// pretty-printed Debug string. Skips validate / evaluate so callers
// can inspect what the parser actually built — useful for diagnosing
// parser-stage issues (deep paren nests, weird precedence, etc.) in
// isolation from the eval pipeline. Uses `DEFAULT_DISPLAY_DEPTH` for
// truncation; see `parse_input_with_depth` to pick a different cap.
pub fn parse_input(input: &str) -> Result<String, String> {
    parse_input_with_depth(input, DEFAULT_DISPLAY_DEPTH)
}

// Same as `parse_input` but lets the caller choose the truncation depth.
// Higher caps preserve more of the AST at the cost of recursion in the
// `{:#?}` formatter — picking a value much above 10⁵ may overflow the
// default thread stack on a deep input. `0` truncates immediately
// (every expression becomes the `…` placeholder); useful only as a
// "did this parse?" probe.
pub fn parse_input_with_depth(input: &str, max_depth: usize) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(String::new());
    }
    let (mut statements, _trailing_semicolon) =
        parser::parse_statements(input).map_err(|e| format!("Syntax error: {e}"))?;
    // Parse-only is a permissive "did it parse?" probe — undeclared
    // identifiers, unknown system calls, and out-of-range literal widths
    // all pass through. Anything that would be a Semantic error in eval
    // mode lives in the validator, not here; this entry point is purely
    // syntactic. The lazy `Expr::Literal(LiteralSpec)` shape lets even
    // `9999999999999'd1` produce a well-formed AST without allocating.
    for stmt in &mut statements {
        parser::truncate_stmt_for_display(stmt, max_depth);
    }
    Ok(format!("{statements:#?}"))
}

fn evaluate_input_with_session(session: &mut Session, input: &str) -> Result<Evaluation, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(Evaluation {
            task_output: Vec::new(),
            output: String::new(),
            should_exit: false,
        });
    }

    let (statements, trailing_semicolon) =
        parser::parse_statements(input).map_err(|e| format!("Syntax error: {e}"))?;

    // IPython-style suppression applies only to value output: the last
    // expression value is visible only if the input did not end with a `;`.
    // System-task output is a side effect and accumulates independently.
    let mut task_output = Vec::new();
    let mut last_output = String::new();
    let mut last_was_expr = false;
    for stmt in &statements {
        let (stmt_task_output, output, should_exit) = apply_stmt(session, stmt)?;
        task_output.extend(stmt_task_output);
        if should_exit {
            return Ok(Evaluation {
                task_output,
                output: String::new(),
                should_exit: true,
            });
        }
        last_output = output;
        last_was_expr = matches!(stmt, Stmt::Expr(_));
    }

    let output = if trailing_semicolon || !last_was_expr {
        String::new()
    } else {
        last_output
    };

    Ok(Evaluation {
        task_output,
        output,
        should_exit: false,
    })
}

// Drives a single top-level Stmt. Decls mutate the session and emit no value
// output. Assignments mutate the session and emit the reg's new canonical
// form. Expression statements just evaluate — except that an `Expr::SystemCall`
// whose name classifies as a task is hoisted here, since tasks have no
// expression value. The hoist walks through `Grouped` layers via the iterative
// `unwrap_grouped` so `((($display("x"))))` still runs as a top-level task.
fn apply_stmt(session: &mut Session, stmt: &Stmt) -> Result<(Vec<u8>, String, bool), String> {
    match stmt {
        Stmt::Expr(expr) => {
            if let Expr::SystemCall { name, args } = eval::unwrap_grouped(expr)
                && let Ok(SystemCallKind::Task(task)) = system_call::classify_system_call(name)
            {
                let result = system_call::execute_task(task, args, session)?;
                return Ok((result.output, String::new(), result.should_exit));
            }
            let value = eval::evaluate_expr(expr, session)?;
            Ok((Vec::new(), value.canonical(), false))
        }
        Stmt::Decl {
            kind,
            signed,
            range,
            names,
        } => {
            let (output, should_exit) = apply_decl(session, *kind, *signed, range.as_ref(), names)?;
            Ok((Vec::new(), output, should_exit))
        }
        Stmt::Assign { lvalue, rhs } => {
            let (output, should_exit) = apply_assign(session, lvalue, rhs)?;
            Ok((Vec::new(), output, should_exit))
        }
    }
}

// LRM A.2.1.3 variable declarations. `apply_decl` is the shared driver
// for `reg`, `integer`, and `real` decls — each `DeclKind` decides the
// per-name storage shape but the staging / atomic-commit / per-name
// init / self-reference semantics are identical across all three. Per
// LRM 4.8 an `integer` reg is fixed at signed 32-bit with decimal
// display base; a `real` reg is IEEE 754 binary64 with no width or
// base. Redeclaration replaces the previous binding outright (vcal is a
// single-scope REPL), and the whole decl is committed all-or-nothing so
// `integer i = 1, j = nope` leaves `i` untouched on failure (mirroring
// `reg`'s prior behaviour).
fn apply_decl(
    session: &mut Session,
    kind: DeclKind,
    signed: bool,
    range: Option<&(Expr, Expr)>,
    names: &[DeclName],
) -> Result<(String, bool), String> {
    // `reg` evaluates its packed range up front so a malformed range
    // aborts the whole decl before any name is committed. `integer`
    // bakes a constant `[31:0]` range so bit-selects against the
    // integer reg work identically to a `reg signed [31:0]` decl — the
    // value is conceptually 32 bits. `real` has no bit range.
    let (resolved_range, element_width, element_signed, element_base, element_base_locked) =
        match kind {
            DeclKind::Reg => {
                let resolved = match range {
                    Some((msb_expr, lsb_expr)) => {
                        Some(evaluate_reg_range(msb_expr, lsb_expr, session)?)
                    }
                    None => None,
                };
                let width = match &resolved {
                    Some(range) => range.width()?,
                    None => 1,
                };
                (resolved, width, signed, Base::Binary, false)
            }
            DeclKind::Integer => {
                // LRM 4.8: `integer` is signed 32-bit. Decimal display base
                // matches `integer i; i = 0;` round-tripping through the
                // canonical printer as `32'sd0` — same as `$signed(0)` or
                // `reg signed [31:0] i` would render once stored in decimal.
                let range = RegRange {
                    msb: BigInt::from(31),
                    lsb: BigInt::from(0),
                };
                (Some(range), 32usize, true, Base::Decimal, true)
            }
            DeclKind::Real => (None, 0usize, false, Base::Binary, true),
        };

    // Staging area mirrors the prior reg-only path: every init runs
    // against a `Session` view of `staged` so a self-reference reads
    // the prior binding (`reg [1:0] a = 2'b11; reg a = a` narrows to
    // 1 bit) and within the same statement each name sees the bindings
    // of names earlier in the list (`integer i = 1, j = i + 1`). To
    // avoid cloning on every iteration the map is moved into a
    // throwaway `Session` for the eval call, then moved back out.
    let mut staged = session.variables.clone();
    for DeclName { name, init, dim } in names {
        // Build per-kind storage. `apply_init_for` walks the same
        // staged / view-Session dance the previous code used so init
        // evaluation rules (width/sign/base context for integer-typed
        // inits, §3.5.3 real→integer conversion, x-fill default) stay
        // a single path.
        let storage = match (kind, dim) {
            // Vector array: `reg [3:0] a [0:15]` and `integer a [0:3]`
            // share this path — the element template's
            // (width, signed, base) comes from the per-kind context
            // above. The parser rejects init on any array decl, so
            // `init.is_some()` here is defended against AST construction
            // from a hypothetical non-parser path.
            (DeclKind::Reg | DeclKind::Integer, Some((dim_msb_expr, dim_lsb_expr))) => {
                if init.is_some() {
                    return Err(format!(
                        "array variable `{name}` cannot have an init expression"
                    ));
                }
                let dim_range = with_staged_session(&mut staged, |view| {
                    evaluate_reg_range(dim_msb_expr, dim_lsb_expr, view)
                })?;
                let count = dim_range.width()?;
                ensure_array_total_bits(element_width, count)?;
                let element_template = IntegerValue {
                    width: element_width,
                    signed: element_signed,
                    base: element_base,
                    base_locked: element_base_locked,
                    display_style: DisplayStyle::Base,
                    bits: vec![LogicBit::X; element_width],
                    unsized_literal: false,
                };
                RegStorage::Array {
                    dim: dim_range,
                    elements: vec![element_template; count],
                }
            }
            // Real array: `real r [0:3]`. Same parser-rejects-init
            // rule, element default is 0.0 per LRM 4.8.
            (DeclKind::Real, Some((dim_msb_expr, dim_lsb_expr))) => {
                if init.is_some() {
                    return Err(format!(
                        "array variable `{name}` cannot have an init expression"
                    ));
                }
                let dim_range = with_staged_session(&mut staged, |view| {
                    evaluate_reg_range(dim_msb_expr, dim_lsb_expr, view)
                })?;
                let count = dim_range.width()?;
                RegStorage::RealArray {
                    dim: dim_range,
                    elements: vec![0.0; count],
                }
            }
            // Scalar / vector (non-array) reg or integer: integer-pipeline
            // init evaluation with the per-kind context.
            (DeclKind::Reg | DeclKind::Integer, None) => {
                let value = eval_init_value(
                    init.as_ref(),
                    element_width,
                    element_signed,
                    element_base,
                    element_base_locked,
                    &mut staged,
                )?;
                RegStorage::Vector(value)
            }
            // Scalar real: LRM 4.8 zero-init, optional real-pipeline
            // init (`evaluate_real_value` handles the §5.1.7 / §3.5.3
            // promotion for integer-typed inits).
            (DeclKind::Real, None) => {
                let value = match init {
                    Some(init_expr) => with_staged_session(&mut staged, |view| {
                        evaluate_real_value(init_expr, view)
                    })?,
                    None => 0.0,
                };
                RegStorage::Real(value)
            }
        };
        staged.insert(
            name.clone(),
            RegValue {
                range: resolved_range.clone(),
                storage,
            },
        );
    }
    session.variables = staged;
    Ok((String::new(), false))
}

fn ensure_array_total_bits(element_width: usize, count: usize) -> Result<(), String> {
    let total = element_width.checked_mul(count).ok_or_else(|| {
        format!(
            "Semantic error: array total width exceeds limit {}",
            value::MAX_BIT_WIDTH
        )
    })?;
    value::ensure_bit_width(total, "array total").map_err(|e| format!("Semantic error: {e}"))
}

// Wraps an evaluator call that needs a `&Session` view over the staged
// variable map. The map is moved out, lent to the closure, then moved
// back in so the per-init loop doesn't have to clone on every
// iteration. Mirrors the inlined pattern the previous reg-only path
// used; lifting it out keeps the per-kind storage branches readable.
fn with_staged_session<T, F>(staged: &mut HashMap<String, RegValue>, f: F) -> Result<T, String>
where
    F: FnOnce(&Session) -> Result<T, String>,
{
    let view = Session {
        variables: std::mem::take(staged),
    };
    let outcome = f(&view);
    *staged = view.variables;
    outcome
}

// Evaluates an optional init expression to a `Vec<LogicBit>` of the
// given (width, signed, base) context. With no init, returns x bits at
// the target width (LRM 4.8 default for reg / integer). With an init,
// runs through `evaluate_assignment_rhs` so the same width / sign /
// base context and real→integer §3.5.3 conversion semantics apply that
// `name = expr` would use after the decl.
fn eval_init_value(
    init: Option<&Expr>,
    width: usize,
    signed: bool,
    base: Base,
    base_locked: bool,
    staged: &mut HashMap<String, RegValue>,
) -> Result<IntegerValue, String> {
    match init {
        Some(init_expr) => {
            let result = with_staged_session(staged, |view| {
                eval::evaluate_assignment_rhs(init_expr, width, signed, base, view)
            })?;
            let sized = result.resized_to_context(width, signed);
            let (stored_base, stored_base_locked) = if base_locked {
                (base, true)
            } else if result.base_locked {
                (result.base, true)
            } else {
                (base, false)
            };
            Ok(IntegerValue {
                width,
                signed,
                base: stored_base,
                base_locked: stored_base_locked,
                display_style: DisplayStyle::Base,
                bits: sized.bits,
                unsized_literal: false,
            })
        }
        None => Ok(IntegerValue {
            width,
            signed,
            base,
            base_locked,
            display_style: DisplayStyle::Base,
            bits: vec![LogicBit::X; width],
            unsized_literal: false,
        }),
    }
}

// LRM A.6.2 blocking assignment. For an integer LHS (vector reg,
// integer reg, array element, bit/part-select, concatenation) the
// existing `evaluate_lvalue_assignment` path is exact — width /
// signed / base context is read off the LHS, RHS evaluates through
// `evaluate_assignment_rhs` (real → integer per §3.5.3, x bits on
// NaN/±∞), and the staged map swaps in atomically. A bare-name LHS
// that resolves to a `real` reg routes through `apply_real_assign`
// instead: real has no width / base / bits, so the LHS context that
// `evaluate_lvalue_assignment` would otherwise build doesn't apply
// here.
fn apply_assign(
    session: &mut Session,
    lvalue: &LValue,
    rhs: &Expr,
) -> Result<(String, bool), String> {
    if let LValue::Name(name) = lvalue
        && let Some(reg) = session.lookup(name)
        && reg.is_real()
    {
        return apply_real_assign(session, name, rhs);
    }
    if let LValue::Select {
        name,
        kind: SelectKind::Bit { index },
        inner: None,
    } = lvalue
        && let Some(reg) = session.lookup(name)
        && reg.is_real_array()
    {
        return apply_real_array_element_assign(session, name, index, rhs);
    }
    let (staged, displayed) = eval::evaluate_lvalue_assignment(lvalue, rhs, session)?;
    session.variables = staged;
    Ok((displayed.canonical(), false))
}

// LRM 5.6 blocking assignment with a real LHS. Evaluates the RHS as a
// real value (integer-typed RHS auto-promotes via §3.5.3, x/z bits → 0
// inside `integer_value_to_f64`), stages a single-name update in a
// clone of the variable map, then commits. Mirrors
// `evaluate_lvalue_assignment`'s atomic-commit contract: the live
// session only adopts the change if RHS evaluation succeeded.
fn apply_real_assign(
    session: &mut Session,
    name: &str,
    rhs: &Expr,
) -> Result<(String, bool), String> {
    let value = evaluate_real_value(rhs, session)?;
    let mut staged = session.variables.clone();
    let reg = staged
        .get_mut(name)
        .expect("caller verified the reg exists in this session");
    let slot = reg
        .real_mut()
        .expect("caller verified the reg is a real reg");
    *slot = value;
    session.variables = staged;
    Ok((Value::Real(value).canonical(), false))
}

// LRM 5.6 blocking assignment for a real-array element (`r[i] = expr`
// where `r` is `real r [0:..]`). Sibling of `apply_real_assign`: the RHS
// flows through the real pipeline (`evaluate_real_value` promotes
// integer RHS via §3.5.3); the index is resolved against the unpacked
// dimension. Per LRM 4.2.1, OOB index / x-z index drop the write — but
// we still echo the RHS as if it had landed, mirroring how the bare
// `apply_real_assign` always reports the RHS value. The index
// expression is structurally validated up front (so e.g. `r[a + b]`
// surfaces an undeclared-identifier error from `a`/`b` before RHS
// evaluation).
fn apply_real_array_element_assign(
    session: &mut Session,
    name: &str,
    index: &Expr,
    rhs: &Expr,
) -> Result<(String, bool), String> {
    eval::semantic_check(index, session)?;
    if eval::expression_is_real(index, session) {
        return Err("Semantic error: array element index cannot be real".to_string());
    }
    let value = evaluate_real_value(rhs, session)?;
    let resolved = eval::resolve_real_array_element_index(name, index, session)?;
    if let Some(internal) = resolved {
        let mut staged = session.variables.clone();
        let reg = staged
            .get_mut(name)
            .expect("caller verified the reg exists in this session");
        let (_, elements) = reg
            .real_array_mut()
            .expect("caller verified the reg is a real array");
        elements[internal] = value;
        session.variables = staged;
    }
    Ok((Value::Real(value).canonical(), false))
}

// Evaluates an arbitrary expression as a real value, after the same
// static-semantic pre-pass every public entry point runs. Routes
// through `evaluate_expr` and pulls the f64 out of the resulting
// `Value` — real-result expressions yield `Value::Real(f64)` directly,
// while integer-result expressions yield `Value::Integer(...)` and
// promote to f64 via the IEEE-conversion `to_f64()` (LRM §5.1.7 mixed
// real / int operands; §3.5.3 x/z bits treated as zero). Errors carry
// whichever `Syntax error: ` / `Semantic error: ` / runtime prefix
// `evaluate_expr` produces.
fn evaluate_real_value(expr: &Expr, session: &Session) -> Result<f64, String> {
    let value = eval::evaluate_expr(expr, session)?;
    match value {
        Value::Real(f) => Ok(f),
        Value::Integer(int_val) => {
            use num_traits::ToPrimitive;
            // LRM §5.1.7 + §3.5.3: an integer operand converts to its
            // equivalent real value (x/z bits already folded to zero by
            // `as_bigint`). `BigInt::to_f64` is total (saturates huge
            // magnitudes to ±∞).
            Ok(int_val
                .as_bigint(int_val.signed)
                .to_f64()
                .expect("BigInt::to_f64 is total"))
        }
    }
}

// Evaluate a reg declaration's `[msb:lsb]` range. Each half is a constant
// integer expression, evaluated in the current session so a prior reg can
// be referenced (and immediately rejected because its bits are x). x/z half
// values are rejected up-front; the width is |msb - lsb| + 1, so negative
// endpoints and reversed ranges are both accepted per LRM 4.8. If that
// width would exceed addressable `usize`, surface a normal error instead of
// overflowing.
fn evaluate_reg_range(
    msb_expr: &Expr,
    lsb_expr: &Expr,
    session: &Session,
) -> Result<RegRange, String> {
    let msb = evaluate_range_endpoint(msb_expr, session, "msb")?;
    let lsb = evaluate_range_endpoint(lsb_expr, session, "lsb")?;
    let range = RegRange { msb, lsb };
    let _ = range.width()?;
    Ok(range)
}

impl RegRange {
    fn width(&self) -> Result<usize, String> {
        let width = (&self.msb - &self.lsb).abs() + BigInt::from(1u8);
        let width = width
            .to_usize()
            .ok_or_else(|| "Semantic error: reg range width too large".to_string())?;
        value::ensure_bit_width(width, "reg").map_err(|e| format!("Semantic error: {e}"))?;
        Ok(width)
    }
}

fn evaluate_range_endpoint(expr: &Expr, session: &Session, role: &str) -> Result<BigInt, String> {
    if eval::expression_is_real(expr, session) {
        return Err(format!("Semantic error: reg range {role} cannot be real"));
    }
    // `evaluate_constant_expr` runs its own semantic_check and prefixes
    // structural errors itself; the constant-must-not-contain-x check below
    // is also a static-semantic rule, so it carries the same prefix.
    let value = eval::evaluate_constant_expr(expr, session)?;
    if value.has_unknown_bits() {
        return Err(format!(
            "Semantic error: reg range {role} contains unknown bits"
        ));
    }
    Ok(value.as_bigint(value.signed))
}

pub fn run_repl<R: BufRead, W: Write>(reader: &mut R, writer: &mut W) -> io::Result<()> {
    let mut session = Session::new();
    let mut index = 0usize;
    let mut line = String::new();

    loop {
        write!(writer, "In [{index}]: ")?;
        writer.flush()?;

        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }

        match session.eval(&line) {
            Ok(result) => {
                writer.write_all(&result.task_output)?;
                if result.output.is_empty() {
                    writeln!(writer)?;
                } else {
                    writeln!(writer, "Out[{index}]: {}", result.output)?;
                    writeln!(writer)?;
                }
                if result.should_exit {
                    break;
                }
            }
            Err(message) => {
                writeln!(writer, "{message}")?;
                writeln!(writer)?;
            }
        }

        index += 1;
    }

    Ok(())
}

pub fn run_interactive() -> io::Result<()> {
    use rustyline::Editor;
    use rustyline::error::ReadlineError;
    use rustyline::history::DefaultHistory;

    let use_color = color::should_color();
    let mut editor: Editor<color::PromptHelper, DefaultHistory> =
        Editor::new().map_err(io::Error::other)?;
    editor.set_helper(Some(color::PromptHelper { enabled: use_color }));
    let mut session = Session::new();
    let mut index = 0usize;

    loop {
        let line = match editor.readline(&format!("In [{index}]: ")) {
            Ok(line) => line,
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(err) => return Err(io::Error::other(err)),
        };

        if !line.trim().is_empty() {
            let _ = editor.add_history_entry(line.as_str());
        }

        match session.eval(&line) {
            Ok(result) => {
                let mut stdout = io::stdout();
                stdout.write_all(&result.task_output)?;
                if result.output.is_empty() {
                    println!();
                } else {
                    let prefix = format!("Out[{index}]: ");
                    let prefix = if use_color {
                        color::red(&prefix)
                    } else {
                        prefix
                    };
                    println!("{prefix}{}", result.output);
                    println!();
                }
                if result.should_exit {
                    break;
                }
            }
            Err(message) => {
                println!("{message}");
                println!();
            }
        }

        index += 1;
    }

    Ok(())
}

// Parse-only REPL for the piped / non-TTY path. Mirrors `run_repl` but
// stops after the parser and prints the AST instead of evaluating. Used
// when the binary is invoked with `--parse-only`. `max_depth` controls
// the AST display truncation cap (see `parse_input_with_depth`). No
// `Session` is threaded through — the parser doesn't read or write
// variable state.
pub fn run_parse_repl<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    max_depth: usize,
) -> io::Result<()> {
    let mut index = 0usize;
    let mut line = String::new();

    loop {
        write!(writer, "In [{index}]: ")?;
        writer.flush()?;

        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }

        match parse_input_with_depth(&line, max_depth) {
            Ok(ast) => {
                if ast.is_empty() {
                    writeln!(writer)?;
                } else {
                    writeln!(writer, "Out[{index}]: {ast}")?;
                    writeln!(writer)?;
                }
            }
            Err(message) => {
                writeln!(writer, "{message}")?;
                writeln!(writer)?;
            }
        }

        index += 1;
    }

    Ok(())
}

// Parse-only TTY REPL — rustyline-backed counterpart to run_parse_repl.
pub fn run_parse_interactive(max_depth: usize) -> io::Result<()> {
    use rustyline::Editor;
    use rustyline::error::ReadlineError;
    use rustyline::history::DefaultHistory;

    let use_color = color::should_color();
    let mut editor: Editor<color::PromptHelper, DefaultHistory> =
        Editor::new().map_err(io::Error::other)?;
    editor.set_helper(Some(color::PromptHelper { enabled: use_color }));
    let mut index = 0usize;

    loop {
        let line = match editor.readline(&format!("In [{index}]: ")) {
            Ok(line) => line,
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(err) => return Err(io::Error::other(err)),
        };

        if !line.trim().is_empty() {
            let _ = editor.add_history_entry(line.as_str());
        }

        match parse_input_with_depth(&line, max_depth) {
            Ok(ast) => {
                if ast.is_empty() {
                    println!();
                } else {
                    let prefix = format!("Out[{index}]: ");
                    let prefix = if use_color {
                        color::red(&prefix)
                    } else {
                        prefix
                    };
                    println!("{prefix}{ast}");
                    println!();
                }
            }
            Err(message) => {
                println!("{message}");
                println!();
            }
        }

        index += 1;
    }

    Ok(())
}
