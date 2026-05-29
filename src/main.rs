use std::io::{self, IsTerminal};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut parse_only = false;
    for arg in &args {
        match arg.as_str() {
            "--parse-only" => parse_only = true,
            "--help" | "-h" => {
                println!("Usage: vcal [--parse-only]");
                println!();
                println!("  --parse-only   Stop after the parser and print the AST.");
                println!("                 Skips validation and evaluation. For");
                println!("                 debugging parser-stage issues.");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("Error: unknown argument `{other}` (try --help)");
                return ExitCode::from(2);
            }
        }
    }

    let result = if io::stdin().is_terminal() {
        if parse_only {
            vcal::run_parse_interactive()
        } else {
            vcal::run_interactive()
        }
    } else {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut reader = stdin.lock();
        let mut writer = stdout.lock();
        if parse_only {
            vcal::run_parse_repl(&mut reader, &mut writer)
        } else {
            vcal::run_repl(&mut reader, &mut writer)
        }
    };

    if let Err(error) = result {
        eprintln!("Error: {error}");
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}
