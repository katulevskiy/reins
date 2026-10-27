//! `reins-audit verify|report …`: see [`reins_audit::cli`] (and `reins-audit --help`). This wrapper only prints what
//! the command returns and exits with its code.

use std::io::Write as _;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let o = reins_audit::cli::run(&args, &mut std::io::stdin().lock());
    // A closed pipe (`reins-audit report … | head`) is not worth a panic.
    std::io::stdout().lock().write_all(o.stdout.as_bytes()).ok();
    std::io::stderr().lock().write_all(o.stderr.as_bytes()).ok();
    ExitCode::from(o.code)
}
