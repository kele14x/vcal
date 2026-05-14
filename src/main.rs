use std::io::{self, IsTerminal};
use std::process::ExitCode;

fn main() -> ExitCode {
    let result = if io::stdin().is_terminal() {
        vcal::run_interactive()
    } else {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut reader = stdin.lock();
        let mut writer = stdout.lock();
        vcal::run_repl(&mut reader, &mut writer)
    };

    if let Err(error) = result {
        eprintln!("Error: {error}");
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}
