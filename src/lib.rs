use std::io::{self, BufRead, Write};

mod eval;
mod lexer;
mod parser;
mod value;

#[cfg(test)]
mod tests;

pub use value::{Base, IntegerValue, LogicBit};

#[derive(Debug, PartialEq, Eq)]
pub struct Evaluation {
    pub output: String,
    pub should_exit: bool,
}

enum ParsedLine {
    Value(IntegerValue),
    Exit,
}

pub fn evaluate_input(input: &str) -> Result<Evaluation, String> {
    match parse_line(input)? {
        ParsedLine::Value(value) => Ok(Evaluation {
            output: value.canonical(),
            should_exit: false,
        }),
        ParsedLine::Exit => Ok(Evaluation {
            output: String::new(),
            should_exit: true,
        }),
    }
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

fn parse_line(input: &str) -> Result<ParsedLine, String> {
    let input = strip_statement_terminators(input);

    if input.is_empty() {
        return Err("empty input".to_string());
    }

    if let Some(command) = parse_system_task(input)? {
        return Ok(command);
    }

    let expression = parser::parse_expression(input)?;
    eval::evaluate_expr(&expression).map(ParsedLine::Value)
}

fn strip_statement_terminators(input: &str) -> &str {
    let mut trimmed = input.trim();

    while let Some(stripped) = trimmed.strip_suffix(';') {
        trimmed = stripped.trim_end();
    }

    trimmed
}

fn parse_system_task(input: &str) -> Result<Option<ParsedLine>, String> {
    for name in ["$finish", "$stop"] {
        if let Some(rest) = input.strip_prefix(name) {
            let rest = rest.trim();
            if rest.is_empty() || rest == "()" {
                return Ok(Some(ParsedLine::Exit));
            }

            return Err(format!("unsupported system task syntax: {input}"));
        }
    }

    // Anything else starting with `$` is either a system function like
    // `$signed`/`$unsigned` (handled by the expression parser) or an unknown
    // identifier the parser will reject with its own diagnostic.
    Ok(None)
}
