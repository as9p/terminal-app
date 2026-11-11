//! Snowflake, the plain terminal: arguments, then the windowless mode or the window.

mod args;
mod backlog;
mod headless;
mod input;
mod session;
mod shell;
mod window;

use std::process::ExitCode;

use args::{Parsed, USAGE};

/// The exit code for a command line that cannot be run, as most Unix tools use.
const USAGE_ERROR: u8 = 2;

fn main() -> ExitCode {
    let args = match args::parse(std::env::args_os().skip(1)) {
        Ok(Parsed::Run(args)) => args,
        Ok(Parsed::Help) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Parsed::Version) => {
            println!("snowflake {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(message) => return usage_error(&message),
    };
    let Some(grid) = args.headless else {
        let code = window::run(window::Options {
            record: args.record,
            cwd: args.cwd,
            command: args.command,
        });
        // The window, the view and the shell are all dropped by now.
        std::process::exit(code)
    };
    // A bad keys file is found before the command starts, so nothing is left to clean up.
    let keys = match &args.keys {
        Some(path) => match std::fs::read(path) {
            Ok(file) => match headless::parse_keys(&file) {
                Ok(keys) => keys,
                Err(message) => return usage_error(&message),
            },
            Err(e) => return usage_error(&format!("cannot read {}: {e}", path.display())),
        },
        None => Vec::new(),
    };
    let code = headless::run(headless::Options {
        grid,
        keys,
        timeout: args.timeout,
        record: args.record,
        cwd: args.cwd,
        command: args.command,
    });
    // Every value that owns the shell is dropped by now, so exiting here leaves nothing behind.
    std::process::exit(code)
}

fn usage_error(message: &str) -> ExitCode {
    eprintln!("snowflake: {message}\n\n{USAGE}");
    ExitCode::from(USAGE_ERROR)
}
