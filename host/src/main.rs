use std::future::Future;
use std::process::ExitCode;

use term_host::{auth, hookcmd, paths, server, service};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    match args.next() {
        // Claude runs `term-host hook <event>` for each hook. Checked before the runtime
        // starts, so the call stays quick, and it always exits 0 so Claude never shows an error.
        Some(arg) if arg == "hook" => {
            let event = args.next().unwrap_or_default();
            hookcmd::run(&event.to_string_lossy());
            std::process::exit(0);
        }
        // `term-host service install|uninstall|status`, which needs no runtime either.
        Some(arg) if arg == "service" => service::main(args.next()),
        _ => host_main(),
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
