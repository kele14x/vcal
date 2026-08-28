use std::io::{self, IsTerminal};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut parse_only = false;
    let mut max_depth: Option<usize> = None;
    for arg in &args {
        match arg.as_str() {
            "--parse-only" => parse_only = true,
            "--help" | "-h" => {
                println!("Usage: vcal [--parse-only [--max-depth=N]]");
                println!();
                println!("  --parse-only   Stop after the parser and print the AST.");
                println!("                 Skips validation and evaluation. For");
                println!("                 debugging parser-stage issues.");
                println!();
                println!("  --max-depth=N  AST display depth cap for --parse-only mode.",);
                println!(
                    "                 Sub-trees at depth N or deeper render as `Truncated`. Default {}.",
                    vcal::DEFAULT_DISPLAY_DEPTH
                );
                println!("                 Higher caps preserve more of the AST but spend more");
                println!("                 stack on the {{:#?}} formatter; very large values");
                println!("                 (>10⁵) on a deep input may overflow.");
                return ExitCode::SUCCESS;
            }
            other if other.starts_with("--max-depth=") => {
                let value = &other["--max-depth=".len()..];
                match value.parse::<usize>() {
                    Ok(n) => max_depth = Some(n),
                    Err(_) => {
                        eprintln!(
                            "Error: --max-depth requires a non-negative integer (got `{value}`)"
                        );
                        return ExitCode::from(2);
                    }
                }
            }
            other => {
                eprintln!("Error: unknown argument `{other}` (try --help)");
                return ExitCode::from(2);
            }
        }
    }

    if max_depth.is_some() && !parse_only {
        eprintln!("Error: --max-depth only applies under --parse-only");
        return ExitCode::from(2);
    }
    let depth = max_depth.unwrap_or(vcal::DEFAULT_DISPLAY_DEPTH);

    let result = if io::stdin().is_terminal() {
        if parse_only {
            vcal::run_parse_interactive(depth)
        } else {
            vcal::run_interactive()
        }
    } else {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut reader = stdin.lock();
        if parse_only {
            let mut writer = stdout.lock();
            vcal::run_parse_repl(&mut reader, &mut writer, depth)
        } else {
            let mut writer = vcal::ConsoleSafeWriter::new(stdout.lock());
            vcal::run_repl(&mut reader, &mut writer)
        }
    };

    if let Err(error) = result {
        eprintln!("Error: {error}");
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}
