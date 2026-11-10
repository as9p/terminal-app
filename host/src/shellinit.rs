//! Loads the shell hook, which reports the folder at each prompt, without editing the user's
//! startup files. The hook files live in `<data>/shell/` and the shell is pointed at them.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const FOLDER: &str = "shell";
const ZSH_FOLDER: &str = "zsh";

/// Each hook file, by its path under `<data>/shell/`.
const FILES: [(&str, &str); 4] = [
    ("zsh/.zshenv", include_str!("../shell/zsh/.zshenv")),
    ("ta.zsh", include_str!("../shell/ta.zsh")),
    ("ta.bash", include_str!("../shell/ta.bash")),
    ("ta.ps1", include_str!("../shell/ta.ps1")),
];

/// Writes the hook files, leaving any that already hold the same text.
pub fn install(data_dir: &Path) -> io::Result<()> {
    let folder = data_dir.join(FOLDER);
    for (name, text) in FILES {
        let path = folder.join(name);
        if fs::read(&path).is_ok_and(|on_disk| on_disk == text.as_bytes()) {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, text)?;
    }
    Ok(())
}

/// The argv for an interactive shell that loads the hook, adding what the hook needs to `env`.
/// An unknown shell, or one whose hook file is missing, gets the plain argv and no change.
pub fn wrap(shell: &Path, data_dir: &Path, env: &mut Vec<(String, String)>) -> Vec<OsString> {
    wrap_for(shell, data_dir, env, cfg!(windows))
}

/// `wrap` as on Windows or not, so both can be tested anywhere.
pub(crate) fn wrap_for(
    shell: &Path,
    data_dir: &Path,
    env: &mut Vec<(String, String)>,
    windows: bool,
) -> Vec<OsString> {
    let folder = data_dir.join(FOLDER);
    let name = shell
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_ascii_lowercase());
    let hooked = match name.as_deref() {
        Some("zsh") => zsh(shell, &folder, env),
        Some("bash") => bash(shell, &folder),
        Some("pwsh" | "powershell") => powershell(shell, &folder),
        _ => None,
    };
    hooked.unwrap_or_else(|| plain(shell, windows))
}

/// What a shell got before the hook.
fn plain(shell: &Path, windows: bool) -> Vec<OsString> {
    if windows {
        vec![shell.into()]
    } else {
        vec![shell.into(), "-l".into()]
    }
}

/// zsh reads its startup files from `ZDOTDIR`, so pointing that at our folder runs our
/// `.zshenv` first, which puts the user's `ZDOTDIR` back before anything else loads.
fn zsh(shell: &Path, folder: &Path, env: &mut Vec<(String, String)>) -> Option<Vec<OsString>> {
    let ours = folder.join(ZSH_FOLDER);
    existing(ours.join(".zshenv"))?;
    env.retain(|(key, _)| key != "TA_USER_ZDOTDIR");
    let theirs = env
        .iter()
        .find(|(key, _)| key == "ZDOTDIR")
        .map(|(_, value)| value.clone());
    if let Some(theirs) = theirs {
        env.push(("TA_USER_ZDOTDIR".to_string(), theirs));
    }
    set_env(env, "ZDOTDIR", ours.to_string_lossy().into_owned());
    Some(vec![shell.into(), "-l".into()])
}

/// The rcfile stands in for the login files, then adds the hook.
fn bash(shell: &Path, folder: &Path) -> Option<Vec<OsString>> {
    let rcfile = existing(folder.join("ta.bash"))?;
    Some(vec![
        shell.into(),
        "--rcfile".into(),
        rcfile.into(),
        "-i".into(),
    ])
}

/// `-Command` runs after the user's profile, so the hook wraps the prompt the profile set.
fn powershell(shell: &Path, folder: &Path) -> Option<Vec<OsString>> {
    let script = existing(folder.join("ta.ps1"))?;
    // Doubled, because a single quote ends a PowerShell string.
    let quoted = script.to_string_lossy().replace('\'', "''");
    Some(vec![
        shell.into(),
        "-NoExit".into(),
        "-Command".into(),
        format!(". '{quoted}'").into(),
    ])
}

fn existing(path: PathBuf) -> Option<PathBuf> {
    path.is_file().then_some(path)
}

/// Replaces `key` in `env`, or adds it.
pub(crate) fn set_env(env: &mut Vec<(String, String)>, key: &str, value: String) {
    match env.iter_mut().find(|(name, _)| name == key) {
        Some((_, old)) => *old = value,
        None => env.push((key.to_string(), value)),
    }
}
