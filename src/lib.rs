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

use parser::{Expr, Stmt};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RegRange {
    pub(crate) msb: BigInt,
    pub(crate) lsb: BigInt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RegValue {
    pub(crate) range: Option<RegRange>,
    pub(crate) value: IntegerValue,
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

    let statements = parser::parse_statements(input)?;

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
            for (name, init) in names {
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
                staged.insert(
                    name.clone(),
                    RegValue {
                        range: range.clone(),
                        value: IntegerValue {
                            width,
                            signed: *signed,
                            base: Base::Binary,
                            bits,
                            unsized_literal: false,
                        },
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
    let _ = range.width()?;
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
        return Err(format!("reg range {role} cannot be real"));
    }
    let value = eval::evaluate_constant_expr(expr, session)?;
    if value.has_unknown_bits() {
        return Err(format!("reg range {role} contains unknown bits"));
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
