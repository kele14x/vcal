use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();

    if let Err(error) = vcal::run_repl(&mut reader, &mut writer) {
        eprintln!("I/O error: {error}");
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}
