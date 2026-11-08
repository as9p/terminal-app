use std::process::ExitCode;

use term_host::{auth, paths, server};

#[tokio::main]
async fn main() -> ExitCode {
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
    let token = auth::load_or_create_token(&paths::data_dir()?)?;
    server::serve(listener, token).await;
    Ok(())
}
