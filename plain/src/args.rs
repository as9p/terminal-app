//! The command line, parsed by hand: an argument crate would cost start-up time for six options.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use term_engine::GridSize;

pub const USAGE: &str =
    "usage: paw [--headless --cols N --rows N [--keys FILE] [--timeout SECONDS]]
           [--record FILE] [--cwd DIR] [-- COMMAND ARGS...]

  --headless         no window: run the command, then print its final screen
  --cols N, --rows N the grid size in windowless mode
  --keys FILE        lines of \"<milliseconds since start> <text>\" typed at their time;
                     the text may hold \\r \\n \\t \\\\ and \\xNN
  --timeout SECONDS  kill the command after this long and exit 124
  --record FILE      write what the command printed, with times, to FILE
  --cwd DIR          start the command in DIR
  --help, --version
";

pub enum Parsed {
    Run(Args),
    Help,
    Version,
}

#[derive(Default)]
pub struct Args {
    /// Set only in windowless mode, which needs both sizes.
    pub headless: Option<GridSize>,
    pub keys: Option<PathBuf>,
    pub timeout: Option<Duration>,
    pub record: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub command: Option<Vec<String>>,
}

/// An error is a message for stderr, shown above the usage text.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Parsed, String> {
    let mut args = args.into_iter();
    let mut parsed = Args::default();
    let (mut headless, mut cols, mut rows) = (false, None, None);
    while let Some(arg) = args.next() {
        let arg = arg
            .into_string()
            .map_err(|arg| format!("unknown option {arg:?}"))?;
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--help" | "-h" => return Ok(Parsed::Help),
            "--version" | "-V" => return Ok(Parsed::Version),
            "--headless" => headless = true,
            "--cols" => cols = Some(size(&arg, value(&arg)?)?),
            "--rows" => rows = Some(size(&arg, value(&arg)?)?),
            "--keys" => parsed.keys = Some(value(&arg)?.into()),
            "--timeout" => parsed.timeout = Some(seconds(value(&arg)?)?),
            "--record" => parsed.record = Some(value(&arg)?.into()),
            "--cwd" => parsed.cwd = Some(value(&arg)?.into()),
            "--" => {
                let command = args
                    .by_ref()
                    .map(|part| {
                        part.into_string()
                            .map_err(|part| format!("command part {part:?} is not UTF-8"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                parsed.command = (!command.is_empty()).then_some(command);
            }
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    match (headless, cols, rows) {
        (true, Some(cols), Some(rows)) => parsed.headless = Some(GridSize { cols, rows }),
        (true, _, _) => return Err("--headless needs both --cols and --rows".into()),
        (false, _, _) if parsed.keys.is_some() || parsed.timeout.is_some() => {
            return Err("--keys and --timeout need --headless".into());
        }
        (false, _, _) => {}
    }
    Ok(Parsed::Run(parsed))
}

fn size(name: &str, value: OsString) -> Result<u16, String> {
    value
        .to_str()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|n| *n > 0)
        .ok_or_else(|| format!("{name} needs a whole number from 1 to 65535"))
}

fn seconds(value: OsString) -> Result<Duration, String> {
    value
        .to_str()
        .and_then(|value| value.parse::<f64>().ok())
        .and_then(|secs| Duration::try_from_secs_f64(secs).ok())
        .filter(|secs| !secs.is_zero())
        .ok_or_else(|| "--timeout needs a number of seconds above zero".to_string())
}
