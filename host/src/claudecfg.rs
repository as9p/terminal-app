//! The Claude Code settings file the app passes with `--settings`. It only adds hooks that run
//! `term-host hook <event>`, so Alex's own `~/.claude` settings and hooks are never touched
//! and keep running beside these (docs/risky-tests.md, ruling d).

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

const FILE: &str = "claude-hooks.json";
const TEMP_FILE: &str = "claude-hooks.json.tmp";

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

/// Writes `<data_dir>/claude-hooks.json` for `host_exe` and returns its path. Written beside it
/// and renamed, so a `claude` starting at that moment never reads half a file.
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

    fs::create_dir_all(data_dir)?;
    let temp = data_dir.join(TEMP_FILE);
    let mut file = File::create(&temp)?;
    file.write_all(&text)?;
    file.sync_all()?;
    drop(file);
    let path = data_dir.join(FILE);
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
