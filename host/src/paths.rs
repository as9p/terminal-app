use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};

const DEFAULT_PORT: u16 = 47211;
const APP_FOLDER: &str = "terminal-app";

pub fn port() -> Result<u16> {
    let Ok(value) = std::env::var("TERM_HOST_PORT") else {
        return Ok(DEFAULT_PORT);
    };
    let port: u16 = value
        .parse()
        .with_context(|| format!("TERM_HOST_PORT is not a port: {value:?}"))?;
    if port == 0 {
        bail!("TERM_HOST_PORT is 0, and a client could not find a port the system picked");
    }
    Ok(port)
}

pub fn data_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("TERM_APP_DATA").filter(|dir| !dir.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let base = dirs::data_dir().ok_or_else(|| anyhow!("this computer has no data folder"))?;
    Ok(base.join(APP_FOLDER))
}
