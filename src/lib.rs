use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use num_bigint::Sign;
use num_traits::ToPrimitive;

mod eval;
mod lexer;
mod parser;
mod value;

#[cfg(test)]
mod tests;

pub use value::{Base, IntegerValue, LogicBit, Value};

use parser::{Expr, Stmt};

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
    variables: HashMap<String, IntegerValue>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn lookup(&self, name: &str) -> Option<&IntegerValue> {
        self.variables.get(name)
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
            let width = match range {
                Some((msb_expr, lsb_expr)) => compute_decl_width(msb_expr, lsb_expr, session)?,
                None => 1,
            };
            // Redeclaration replaces the previous binding outright. The
            // calculator REPL is single-scope and a user iterating on a
            // throwaway calculation expects `reg [3:0] a` to override an
            // earlier `reg [7:0] a` rather than need a separate "drop"
            // command. The new decl's width / signed / base / x-init all
            // wipe the old reg's state.
            for name in names {
                session.variables.insert(
                    name.clone(),
                    IntegerValue {
                        width,
                        signed: *signed,
                        base: Base::Binary,
                        bits: vec![LogicBit::X; width],
                        unsized_literal: false,
                    },
                );
            }
            Ok((String::new(), false))
        }
        Stmt::Assign { name, rhs } => {
            let (width, signed, base) = match session.lookup(name) {
                Some(value) => (value.width, value.signed, value.base),
                None => return Err(format!("undeclared identifier: {name}")),
            };
            // A real RHS triggers an implicit real→integer conversion per
            // LRM §3.5.3 (round to nearest, ties away from zero), handled
            // inside `evaluate_assignment_rhs`. NaN / ±∞ have no integer
            // image and surface as the lvalue filled with x bits.
            let value = eval::evaluate_assignment_rhs(rhs, width, signed, base, session)?;
            let updated = IntegerValue {
                width,
                signed,
                base,
                bits: value.bits,
                unsized_literal: false,
            };
            session.variables.insert(name.clone(), updated.clone());
            Ok((updated.canonical(), false))
        }
    }
}

// Evaluate a reg declaration's `[msb:lsb]` range. Each half is a constant
// integer expression, evaluated in the current session so a prior reg can
// be referenced (and immediately rejected because its bits are x). Negative
// or x/z half values are rejected up-front; the width is |msb - lsb| + 1,
// matching LRM 4.8's reversed-range tolerance. If that width would exceed
// addressable `usize`, surface a normal error instead of overflowing.
fn compute_decl_width(
    msb_expr: &Expr,
    lsb_expr: &Expr,
    session: &Session,
) -> Result<usize, String> {
    let msb = evaluate_range_endpoint(msb_expr, session, "msb")?;
    let lsb = evaluate_range_endpoint(lsb_expr, session, "lsb")?;
    let diff = if msb >= lsb { msb - lsb } else { lsb - msb };
    diff.checked_add(1)
        .ok_or_else(|| "reg range width too large".to_string())
}

fn evaluate_range_endpoint(
    expr: &Expr,
    session: &Session,
    role: &str,
) -> Result<usize, String> {
    if eval::expression_is_real(expr) {
        return Err(format!("reg range {role} cannot be real"));
    }
    let value = eval::evaluate_constant_expr(expr, session)?;
    if value.has_unknown_bits() {
        return Err(format!("reg range {role} contains unknown bits"));
    }
    let bigint = value.as_bigint(value.signed);
    if bigint.sign() == Sign::Minus {
        return Err(format!("reg range {role} must be non-negative"));
    }
    bigint
        .to_usize()
        .ok_or_else(|| format!("reg range {role} too large"))
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
