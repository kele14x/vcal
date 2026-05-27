use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive};

mod eval;
mod lexer;
mod parser;
mod value;

#[cfg(test)]
mod tests;

pub use value::{Base, IntegerValue, LogicBit, Value};

use parser::{DeclName, Expr, Stmt};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RegRange {
    pub(crate) msb: BigInt,
    pub(crate) lsb: BigInt,
}

// A reg's payload. `Vector` is the existing scalar/vector reg; `Array` is
// the new 1-D unpacked-array form (`reg [3:0] a [0:15]`). The packed
// range (the `[3:0]` part) still lives in `RegValue::range`; `Array.dim`
// holds the *unpacked* dimension (`[0:15]`). Each element is an
// IntegerValue with the same width / signedness / base as a freshly
// declared vector reg of that packed range, all-x at decl time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RegStorage {
    Vector(IntegerValue),
    Array {
        dim: RegRange,
        elements: Vec<IntegerValue>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RegValue {
    pub(crate) range: Option<RegRange>,
    pub(crate) storage: RegStorage,
}

impl RegValue {
    // True for the array form (`reg [3:0] a [0:15]`). Distinct from a
    // vector reg because reading the bare name is illegal for an array
    // and array-element access is illegal for a vector.
    pub(crate) fn is_array(&self) -> bool {
        matches!(self.storage, RegStorage::Array { .. })
    }

    // Vector-only accessor used by every non-array codepath. Arrays
    // surface as an error from the caller before this is reached, so
    // panicking here would mask a missed dispatch in eval.
    pub(crate) fn vector(&self) -> Option<&IntegerValue> {
        match &self.storage {
            RegStorage::Vector(value) => Some(value),
            RegStorage::Array { .. } => None,
        }
    }

    // Same as `vector()` but errors with the canonical
    // "array name cannot be used as a value" diagnostic when the reg is
    // an array. Used everywhere a vector-only path resolves an
    // identifier — it makes the array-name-as-primary rejection
    // uniform without duplicating the error string at each callsite.
    pub(crate) fn require_vector(&self, name: &str) -> Result<&IntegerValue, String> {
        self.vector()
            .ok_or_else(|| format!("array `{name}` cannot be used as a value"))
    }

    pub(crate) fn vector_mut(&mut self) -> Option<&mut IntegerValue> {
        match &mut self.storage {
            RegStorage::Vector(value) => Some(value),
            RegStorage::Array { .. } => None,
        }
    }

    // Array-only accessor. Mirrors `vector()`: returns the unpacked
    // dimension and the element slice for an array, `None` for a
    // vector. Keeps `RegStorage` private to lib.rs while letting eval
    // dispatch on the storage kind via `is_array()` + `array()`.
    pub(crate) fn array(&self) -> Option<(&RegRange, &[IntegerValue])> {
        match &self.storage {
            RegStorage::Array { dim, elements } => Some((dim, elements.as_slice())),
            RegStorage::Vector(_) => None,
        }
    }

    // Mutable variant of `array()`. Used by the array-element-write
    // path so the element at the chosen index can be replaced in-place
    // on a staged variable map clone, without exposing `RegStorage` to
    // eval.rs.
    pub(crate) fn array_mut(&mut self) -> Option<(&RegRange, &mut [IntegerValue])> {
        match &mut self.storage {
            RegStorage::Array { dim, elements } => Some((dim, elements.as_mut_slice())),
            RegStorage::Vector(_) => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Evaluation {
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
    pub(crate) fn lookup_reg_array(
        &self,
        name: &str,
    ) -> Option<(BigInt, BigInt, usize)> {
        self.lookup(name).and_then(|reg| match &reg.storage {
            RegStorage::Array { dim, elements } => {
                Some((dim.msb.clone(), dim.lsb.clone(), elements.len()))
            }
            RegStorage::Vector(_) => None,
        })
    }

    pub fn eval(&mut self, input: &str) -> Result<Evaluation, String> {
        evaluate_input_with_session(self, input)
    }
}

pub fn evaluate_input(input: &str) -> Result<Evaluation, String> {
    let mut session = Session::new();
    session.eval(input)
}

fn evaluate_input_with_session(
    session: &mut Session,
    input: &str,
) -> Result<Evaluation, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(Evaluation {
            output: String::new(),
            should_exit: false,
        });
    }

    let statements = parser::parse_statements(input).map_err(|e| format!("Syntax error: {e}"))?;

    let mut outputs = Vec::new();
    for stmt in &statements {
        let (output, should_exit) = apply_stmt(session, stmt)?;
        if !output.is_empty() {
            outputs.push(output);
        }
        if should_exit {
            return Ok(Evaluation {
                output: outputs.join("\n"),
                should_exit: true,
            });
        }
    }

    Ok(Evaluation {
        output: outputs.join("\n"),
        should_exit: false,
    })
}

// Drives a single top-level Stmt. Decls mutate the session and emit no Out
// text (mirroring how `$finish`/`$stop` show an empty Out line). Assignments
// mutate the session and emit the reg's new canonical form. Expression
// statements just evaluate.
fn apply_stmt(session: &mut Session, stmt: &Stmt) -> Result<(String, bool), String> {
    match stmt {
        Stmt::Expr(expr) => {
            let value = eval::evaluate_expr(expr, session)?;
            Ok((value.canonical(), false))
        }
        Stmt::Task(_) => Ok((String::new(), true)),
        Stmt::Decl {
            signed,
            range,
            names,
        } => {
            let range = match range {
                Some((msb_expr, lsb_expr)) => Some(evaluate_reg_range(msb_expr, lsb_expr, session)?),
                None => None,
            };
            let width = match &range {
                Some(range) => range.width()?,
                None => 1,
            };
            // Redeclaration replaces the previous binding outright. The
            // calculator REPL is single-scope and a user iterating on a
            // throwaway calculation expects `reg [3:0] a` to override an
            // earlier `reg [7:0] a` rather than need a separate "drop"
            // command. The new decl's width / signed / base / x-init all
            // wipe the old reg's state.
            //
            // The whole decl is committed all-or-nothing: every init runs
            // against a `staged` clone of the live variable map, and the
            // live session only adopts the result if *all* names finish
            // without error. This stops `reg [3:0] a = 1, b = nope` from
            // silently binding `a` even though the statement errored.
            //
            // Within the staging area, names are processed left-to-right
            // and each commit is visible to the next name's init, so
            // `reg [3:0] a = 1, b = a + 1` still resolves `b` against the
            // freshly-applied `a` (matching the textual order implied by
            // LRM A.2.3 list_of_variable_identifiers).
            //
            // Each init expression evaluates *before* its own binding
            // replaces the corresponding prior entry in the staging map,
            // so a self-reference reads the prior value: with
            // `reg [1:0] a = 2'b11` already in place, `reg a = a` sees the
            // old 2-bit `a`, narrows it to the new 1-bit width via the
            // assignment-RHS context, and stores `1'b1`. Names without an
            // init still install x bits, so `reg a` (no init) wipes the
            // prior binding cleanly.
            //
            // Reusing `evaluate_assignment_rhs` keeps real→integer
            // conversion (LRM §3.5.3, NaN/±∞ → x bits) and width / sign /
            // base context propagation identical to `name = expr`. To
            // avoid cloning `staged` on every iteration, we
            // `std::mem::take` the map into a throwaway `Session` view for
            // the duration of the eval call, then move it back out.
            let mut staged = session.variables.clone();
            for DeclName { name, init, dim } in names {
                // Build the element prototype: every reg (whether
                // bare-name, vector, or array-of-vector) carries the
                // same (width, signed, base = Binary) shape on each
                // element. For an array we also evaluate the unpacked
                // dimension here so a malformed dim aborts the whole
                // decl before any name is committed.
                let storage = if let Some((dim_msb_expr, dim_lsb_expr)) = dim {
                    if init.is_some() {
                        // Should already be caught by the parser, but
                        // keep the runtime check tight in case the AST
                        // is constructed by some other path later.
                        return Err(format!(
                            "array variable `{name}` cannot have an init expression"
                        ));
                    }
                    let dim_range = {
                        let view = Session {
                            variables: std::mem::take(&mut staged),
                        };
                        let outcome = evaluate_reg_range(dim_msb_expr, dim_lsb_expr, &view);
                        staged = view.variables;
                        outcome?
                    };
                    let count = dim_range.width()?;
                    let element_template = IntegerValue {
                        width,
                        signed: *signed,
                        base: Base::Binary,
                        bits: vec![LogicBit::X; width],
                        unsized_literal: false,
                    };
                    RegStorage::Array {
                        dim: dim_range,
                        elements: vec![element_template; count],
                    }
                } else {
                    let bits = match init {
                        Some(init_expr) => {
                            let view = Session {
                                variables: std::mem::take(&mut staged),
                            };
                            let outcome = eval::evaluate_assignment_rhs(
                                init_expr,
                                width,
                                *signed,
                                Base::Binary,
                                &view,
                            );
                            staged = view.variables;
                            outcome?.bits
                        }
                        None => vec![LogicBit::X; width],
                    };
                    RegStorage::Vector(IntegerValue {
                        width,
                        signed: *signed,
                        base: Base::Binary,
                        bits,
                        unsized_literal: false,
                    })
                };
                staged.insert(
                    name.clone(),
                    RegValue {
                        range: range.clone(),
                        storage,
                    },
                );
            }
            session.variables = staged;
            Ok((String::new(), false))
        }
        Stmt::Assign { lvalue, rhs } => {
            // LRM A.6.2 blocking assignment with the full A.8.5
            // variable_lvalue form. All structural validation, RHS
            // evaluation, and bit distribution happen inside
            // `evaluate_lvalue_assignment`, which returns a staged copy
            // of the variable map on success — we swap it in atomically
            // so a multi-leaf concat LHS that fails partway leaves the
            // live session untouched (mirroring `Stmt::Decl`'s
            // all-or-nothing commit). The displayed value is the RHS
            // evaluated in the total-LHS context, so a bare-name LHS
            // prints bit-identically to the pre-lvalue behavior.
            let (staged, displayed) = eval::evaluate_lvalue_assignment(lvalue, rhs, session)?;
            session.variables = staged;
            Ok((displayed.canonical(), false))
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
    let _ = range.width().map_err(|e| format!("Semantic error: {e}"))?;
    Ok(range)
}

impl RegRange {
    fn width(&self) -> Result<usize, String> {
        let width = (&self.msb - &self.lsb).abs() + BigInt::from(1u8);
        width
            .to_usize()
            .ok_or_else(|| "reg range width too large".to_string())
    }
}

fn evaluate_range_endpoint(
    expr: &Expr,
    session: &Session,
    role: &str,
) -> Result<BigInt, String> {
    if eval::expression_is_real(expr) {
        return Err(format!("Semantic error: reg range {role} cannot be real"));
    }
    // `evaluate_constant_expr` runs its own semantic_check and prefixes
    // structural errors itself; the constant-must-not-contain-x check below
    // is also a static-semantic rule, so it carries the same prefix.
    let value = eval::evaluate_constant_expr(expr, session)?;
    if value.has_unknown_bits() {
        return Err(format!("Semantic error: reg range {role} contains unknown bits"));
    }
    Ok(value.as_bigint(value.signed))
}

pub fn run_repl<R: BufRead, W: Write>(reader: &mut R, writer: &mut W) -> io::Result<()> {
    let mut session = Session::new();
    let mut index = 0usize;
    let mut line = String::new();

    loop {
        write!(writer, "In[{index}]: ")?;
        writer.flush()?;

        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }

        match session.eval(&line) {
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
    let mut session = Session::new();
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

        match session.eval(&line) {
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
