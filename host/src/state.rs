//! What the host saves so its sessions come back after a restart or a reboot.
//!
//! `state.json` in the data folder holds the sessions worth bringing back and the tree of groups
//! and tabs. It is written to a temporary file that is then renamed over it, so a crash mid-write
//! leaves the previous file whole. A file that does not parse is renamed aside, not lost.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::tree::Tree;

const FILE: &str = "state.json";
const TEMP_FILE: &str = "state.json.tmp";
const MOVED_ASIDE: &str = "state.json.bad";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub sessions: Vec<SavedSession>,
    /// Missing from a file saved before the tree, which also has a `layout` key that is now
    /// ignored. Such a file loads with an empty tree, and its sessions get tabs on restore.
    #[serde(default)]
    pub tree: Tree,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSession {
    /// As the app sent it in `create`. The app finds its tabs again by this, since session
    /// numbers start over with each host.
    pub uuid: String,
    /// `shell` or `claude`, as in `create`.
    pub kind: String,
    /// The folder the program was last seen in.
    pub cwd: PathBuf,
    pub title: Option<String>,
    /// Set for a Claude session.
    pub chat: Option<Uuid>,
}

/// The saved state, or an empty one if there is none. A file that does not parse is renamed
/// aside so the host still starts and the file is there to look at.
pub fn load(dir: &Path) -> State {
    let path = dir.join(FILE);
    // Bytes, not a string, so a file that is not UTF-8 is moved aside like any other bad file.
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return State::default(),
        Err(error) => {
            eprintln!("term-host: cannot read {}: {error}", path.display());
            return State::default();
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(state) => state,
        Err(error) => {
            let millis = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_millis());
            let aside = dir.join(format!("{MOVED_ASIDE}-{millis}"));
            match fs::rename(&path, &aside) {
                Ok(()) => eprintln!(
                    "term-host: {} did not parse ({error}), moved it to {}",
                    path.display(),
                    aside.display()
                ),
                Err(rename) => eprintln!(
                    "term-host: {} did not parse ({error}) and could not be moved: {rename}",
                    path.display()
                ),
            }
            State::default()
        }
    }
}

pub fn save(dir: &Path, state: &State) -> io::Result<()> {
    let text = serde_json::to_vec_pretty(state)?;
    fs::create_dir_all(dir)?;
    let temp = dir.join(TEMP_FILE);
    let mut file = File::create(&temp)?;
    file.write_all(&text)?;
    // On disk before the rename, so a power cut cannot leave an empty file in its place.
    file.sync_all()?;
    drop(file);
    fs::rename(&temp, dir.join(FILE))
}

/// Whether Claude has a transcript for this chat, under `projects/*/<chat>.jsonl` in its
/// config folder. A chat that never got a first message has none, and `--resume` would fail
/// on it.
pub(crate) fn transcript_exists(chat: Uuid) -> bool {
    let Some(config) = claude_config_dir() else {
        return false;
    };
    let Ok(projects) = fs::read_dir(config.join("projects")) else {
        return false;
    };
    let name = format!("{chat}.jsonl");
    projects
        .flatten()
        .any(|project| project.path().join(&name).is_file())
}

fn claude_config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|dir| !dir.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    Some(dirs::home_dir()?.join(".claude"))
}

/// The folder a running program is in now. Skips a folder whose name is not UTF-8, since
/// the state file could not hold it.
#[cfg(target_os = "macos")]
pub(crate) fn current_dir(pid: u32) -> Option<PathBuf> {
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
    let filled = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    if filled != size {
        return None;
    }
    let path = &info.pvi_cdir.vip_path;
    let bytes =
        unsafe { std::slice::from_raw_parts(path.as_ptr().cast::<u8>(), size_of_val(path)) };
    let end = bytes.iter().position(|&byte| byte == 0)?;
    let path = std::str::from_utf8(&bytes[..end]).ok()?;
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// The folder a running program is in now. Skips a folder whose name is not UTF-8, since
/// the state file could not hold it.
#[cfg(target_os = "linux")]
pub(crate) fn current_dir(pid: u32) -> Option<PathBuf> {
    let path = fs::read_link(format!("/proc/{pid}/cwd")).ok()?;
    path.to_str().is_some().then_some(path)
}

/// Not known on Windows for now, so a session keeps the folder it started in.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn current_dir(_pid: u32) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("term-host-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn saved_state_loads_back() {
        let dir = scratch("state-round-trip");
        let state = State {
            sessions: vec![SavedSession {
                uuid: "a".to_string(),
                kind: "shell".to_string(),
                cwd: PathBuf::from("/tmp"),
                title: Some("vim".to_string()),
                chat: None,
            }],
            tree: Tree::default(),
        };
        save(&dir, &state).expect("save");
        assert_eq!(load(&dir), state);
        assert!(!dir.join(TEMP_FILE).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_layout_file_loads_with_an_empty_tree() {
        let dir = scratch("state-old-layout");
        fs::create_dir_all(&dir).expect("make the folder");
        let old = r#"{"sessions":[{"uuid":"a","kind":"shell","cwd":"/tmp","title":null,"chat":null}],"layout":"{}"}"#;
        fs::write(dir.join(FILE), old).expect("write an old file");
        let state = load(&dir);
        assert_eq!(state.sessions.len(), 1);
        assert_eq!(state.sessions[0].uuid, "a");
        assert_eq!(state.tree, Tree::default());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_file_is_an_empty_state() {
        assert_eq!(load(&scratch("state-missing")), State::default());
    }

    #[cfg(unix)]
    #[test]
    fn current_dir_finds_this_process() {
        let here = std::env::current_dir().expect("current folder");
        let found = current_dir(std::process::id()).expect("a folder");
        assert_eq!(
            found.canonicalize().expect("resolve"),
            here.canonicalize().expect("resolve")
        );
    }
}
