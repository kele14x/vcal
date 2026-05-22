use std::io::{self, BufRead, Write};

mod eval;
mod lexer;
mod parser;
mod value;

#[cfg(test)]
mod tests;

pub use value::{Base, IntegerValue, LogicBit, Value};

#[derive(Debug, PartialEq, Eq)]
pub struct Evaluation {
    pub output: String,
    pub should_exit: bool,
}

pub fn evaluate_input(input: &str) -> Result<Evaluation, String> {
    let input = input.trim();
    if input.is_empty() {
        return Ok(Evaluation {
            output: String::new(),
            should_exit: false,
        });
    }

    let expressions = parser::parse_expressions(input)?;

    let mut outputs = Vec::new();
    for expr in &expressions {
        if is_top_level_system_task(expr) {
            return Ok(Evaluation {
                output: outputs.join("\n"),
                should_exit: true,
            });
        }
        let value = eval::evaluate_expr(expr)?;
        outputs.push(value.canonical());
    }

    Ok(Evaluation {
        output: outputs.join("\n"),
        should_exit: false,
    })
}

pub fn run_repl<R: BufRead, W: Write>(reader: &mut R, writer: &mut W) -> io::Result<()> {
    let mut index = 0usize;
    let mut line = String::new();

    loop {
        write!(writer, "In[{index}]: ")?;
        writer.flush()?;

        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }

        match evaluate_input(&line) {
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

        match evaluate_input(&line) {
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


// LRM 17.4: `$finish` / `$stop` are statements that exit. They are valid
// only at the top of the input — parentheses are tolerated (`($finish)`)
// since `Grouped` carries no semantics here, but any other AST shape means
// the task is nested in an expression and the evaluator will reject it.
fn is_top_level_system_task(expr: &parser::Expr) -> bool {
    match expr {
        parser::Expr::Grouped(inner) => is_top_level_system_task(inner),
        parser::Expr::SystemTask { .. } => true,
        _ => false,
    }
}
