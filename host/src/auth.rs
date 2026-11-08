use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::Path;

use anyhow::{Context, Result, anyhow};

const TOKEN_FILE: &str = "host-token";
const TOKEN_BYTES: usize = 32;

/// Exact match only: a web page chooses its own host name, so a prefix or suffix test could
/// be satisfied by a page on another site. `null` is what a sandboxed page sends.
pub fn origin_allowed(origin: &str) -> bool {
    matches!(origin, "tauri://localhost" | "http://tauri.localhost")
        || (cfg!(debug_assertions) && origin == "http://localhost:1420")
}

/// Constant time over the bytes, so response timing does not reveal how much of a guess was right.
pub fn tokens_match(given: &str, expected: &str) -> bool {
    let (given, expected) = (given.as_bytes(), expected.as_bytes());
    given.len() == expected.len()
        && given
            .iter()
            .zip(expected)
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

/// Returns the token in `dir`, creating the folder and the token if they are missing.
///
/// On Windows no permissions are set: the files rely on the user's profile folder, which
/// other users cannot read.
pub fn load_or_create_token(dir: &Path) -> Result<String> {
    create_private_dir(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let path = dir.join(TOKEN_FILE);
    let token = match read_existing_token(&path) {
        Ok(Some(token)) => Ok(token),
        Ok(None) => create_token(&path),
        Err(error) => Err(error.into()),
    };
    token.with_context(|| format!("cannot set up the token in {}", path.display()))
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Returns `None` if there is no usable token. A crash while writing can leave a short file,
/// which is removed here rather than trusted as a guessable token.
fn read_existing_token(path: &Path) -> io::Result<Option<String>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    tighten_permissions(path)?;
    match String::from_utf8(bytes) {
        Ok(token) if is_whole_token(&token) => Ok(Some(token)),
        _ => {
            fs::remove_file(path)?;
            Ok(None)
        }
    }
}

fn is_whole_token(token: &str) -> bool {
    token.len() == TOKEN_BYTES * 2 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(unix)]
fn tighten_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = fs::metadata(path)?.permissions().mode();
    if mode & 0o077 != 0 {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn tighten_permissions(_path: &Path) -> io::Result<()> {
    Ok(())
}

fn create_token(path: &Path) -> Result<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| anyhow!("no random bytes: {error}"))?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();

    // Written beside it and renamed, so a client reading the file never sees a partial token.
    // Only the host holding the port gets here, so nothing else is writing this temp file.
    let temp = path.with_extension("tmp");
    match fs::remove_file(&temp) {
        Err(error) if error.kind() != ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    file.write_all(token.as_bytes())?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    Ok(token)
}
