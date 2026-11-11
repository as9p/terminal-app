use std::ffi::OsString;
use std::future::Future;
use std::process::ExitCode;

use term_host::{auth, hookcmd, paths, server, service};

const USAGE: &str = "usage: term-host [hook <event> | service install|uninstall|status]";

/// What the arguments after the program's name ask for.
#[derive(Debug, PartialEq)]
enum Command {
    /// No arguments: run the host.
    Host,
    /// `term-host hook <event>`.
    Hook(OsString),
    /// `term-host service ...`, with the arguments after `service`.
    Service(Vec<OsString>),
    /// Anything else, `--help` included. Starts nothing and touches no file.
    Usage,
}

fn command(mut args: impl Iterator<Item = OsString>) -> Command {
    match args.next() {
        None => Command::Host,
        Some(arg) if arg == "hook" => Command::Hook(args.next().unwrap_or_default()),
        Some(arg) if arg == "service" => Command::Service(args.collect()),
        Some(_) => Command::Usage,
    }
}

fn main() -> ExitCode {
    match command(std::env::args_os().skip(1)) {
        Command::Host => host_main(),
        // Claude runs `term-host hook <event>` for each hook. Checked before the runtime
        // starts, so the call stays quick, and it always exits 0 so Claude never shows an error.
        Command::Hook(event) => {
            hookcmd::run(&event.to_string_lossy());
            std::process::exit(0);
        }
        // `term-host service install|uninstall|status`, which needs no runtime either.
        Command::Service(args) => service::main(&args),
        Command::Usage => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[tokio::main]
async fn host_main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("term-host: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let port = paths::port()?;
    // Before anything on disk, so a second host on this port exits without touching a file.
    let listener = server::bind(port).await?;
    let data_dir = paths::data_dir()?;
    let token = auth::load_or_create_token(&data_dir)?;
    let shutdown = shutdown_signal()?;
    server::serve(listener, token, data_dir, shutdown).await;
    // Not a return: dropping the runtime would wait for any blocking task still running, such
    // as an attach stuck on a slow snapshot. The programs hang up as the host exits.
    std::process::exit(0);
}

/// Completes on SIGTERM, which a shutdown or reboot sends, or SIGHUP, which a closed terminal
/// sends the host itself.
#[cfg(unix)]
fn shutdown_signal() -> std::io::Result<impl Future<Output = ()>> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    Ok(async move {
        tokio::select! {
            _ = terminate.recv() => {}
            _ = hangup.recv() => {}
        }
    })
}

/// Completes on Ctrl+C. Never completes if that cannot be watched.
#[cfg(not(unix))]
fn shutdown_signal() -> std::io::Result<impl Future<Output = ()>> {
    Ok(async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Command {
        command(args.iter().map(OsString::from))
    }

    #[test]
    fn only_no_arguments_runs_the_host() {
        assert_eq!(parse(&[]), Command::Host);
        for wrong in [
            &["--help"][..],
            &["-h"],
            &["help"],
            &["--version"],
            &[""],
            &["Hook"],
        ] {
            assert_eq!(parse(wrong), Command::Usage, "{wrong:?}");
        }
    }

    #[test]
    fn hook_and_service_keep_their_arguments() {
        assert_eq!(parse(&["hook", "Stop"]), Command::Hook("Stop".into()));
        assert_eq!(parse(&["hook"]), Command::Hook(OsString::new()));
        assert_eq!(
            parse(&["service", "install", "extra"]),
            Command::Service(vec!["install".into(), "extra".into()])
        );
    }
}
