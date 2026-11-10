//! The Claude Code settings file the app passes with `--settings`. It only adds hooks that run
//! `term-host hook <event>`, so Alex's own `~/.claude` settings and hooks are never touched
//! and keep running beside these (docs/risky-tests.md, ruling d).

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

pub(crate) const FILE: &str = "claude-hooks.json";
const TEMP_FILE: &str = "claude-hooks.json.tmp";
/// The flags every `claude` in a tab gets, one per line.
pub(crate) const ARGS_FILE: &str = "claude-args";
const ARGS_TEMP_FILE: &str = "claude-args.tmp";
/// The app's settings, which the client writes. The host only reads the `claude_flags` keys.
const SETTINGS_FILE: &str = "settings.json";
const DEFAULT_FLAGS: [&str; 1] = ["--dangerously-skip-permissions"];

/// Every hook the session states need, with its matcher. `PreToolUse` gives edited files and
/// the call that starts a helper (`Agent`). `PostToolUse` has no matcher so blocked clears as
/// soon as an approved tool finishes, and `PermissionRequest` sets blocked as the prompt
/// appears (docs/risky-tests.md, rulings c and f).
const HOOKS: [(&str, Option<&str>); 9] = [
    ("SessionStart", None),
    ("UserPromptSubmit", None),
    ("PreToolUse", Some("Edit|Write|Agent")),
    ("PostToolUse", None),
    ("PermissionRequest", None),
    ("Notification", None),
    ("Stop", None),
    ("SubagentStop", None),
    ("SessionEnd", None),
];

/// Writes `<data_dir>/claude-hooks.json` for `host_exe` and returns its path.
pub fn write(data_dir: &Path, host_exe: &Path) -> io::Result<PathBuf> {
    let exe = quote(&host_exe.to_string_lossy());
    let mut hooks = Map::new();
    for (event, matcher) in HOOKS {
        let mut entry = Map::new();
        if let Some(matcher) = matcher {
            entry.insert("matcher".to_string(), Value::from(matcher));
        }
        let command = format!("{exe} hook {event}");
        entry.insert(
            "hooks".to_string(),
            json!([{ "type": "command", "command": command }]),
        );
        hooks.insert(event.to_string(), json!([entry]));
    }
    let text = serde_json::to_vec_pretty(&json!({ "hooks": hooks }))?;
    replace(data_dir, FILE, TEMP_FILE, &text)
}

/// Writes `<data_dir>/claude-args` from the `claude_flags_enabled` (default true) and
/// `claude_flags` (default bypass) keys of `settings.json`: one flag per line, or nothing when
/// the flags are off. Left alone when it already holds that, so calling it often costs a read.
pub fn write_args(data_dir: &Path) -> io::Result<()> {
    let text = args_text(data_dir);
    if fs::read(data_dir.join(ARGS_FILE)).is_ok_and(|on_disk| on_disk == text.as_bytes()) {
        return Ok(());
    }
    replace(data_dir, ARGS_FILE, ARGS_TEMP_FILE, text.as_bytes()).map(|_| ())
}

/// A missing or broken `settings.json` reads as all defaults.
fn args_text(data_dir: &Path) -> String {
    let settings: Value = fs::read(data_dir.join(SETTINGS_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null);
    if settings["claude_flags_enabled"].as_bool() == Some(false) {
        return String::new();
    }
    let flags: Vec<String> = match settings["claude_flags"].as_array() {
        Some(flags) => flags
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect(),
        None => DEFAULT_FLAGS.map(String::from).to_vec(),
    };
    // A line is one flag, so a flag that is empty or holds a line break cannot be passed.
    flags
        .iter()
        .filter(|flag| !flag.is_empty() && !flag.contains(['\n', '\r']))
        .map(|flag| format!("{flag}\n"))
        .collect()
}

/// Writes the file beside its final name and renames it, so a `claude` starting at that moment
/// never reads half a file.
fn replace(data_dir: &Path, name: &str, temp_name: &str, text: &[u8]) -> io::Result<PathBuf> {
    fs::create_dir_all(data_dir)?;
    let temp = data_dir.join(temp_name);
    let mut file = File::create(&temp)?;
    file.write_all(text)?;
    file.sync_all()?;
    drop(file);
    let path = data_dir.join(name);
    fs::rename(&temp, &path)?;
    Ok(path)
}

/// The path quoted for the shell Claude Code runs hook commands in: single quotes for a Unix
/// shell, where nothing inside is expanded, and double quotes on Windows.
fn quote(path: &str) -> String {
    if cfg!(windows) {
        format!("\"{path}\"")
    } else {
        format!("'{}'", path.replace('\'', r"'\''"))
    }
}
