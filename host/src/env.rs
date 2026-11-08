const STRIPPED: [&str; 4] = ["CLAUDECODE", "CLAUDE_CONFIG_DIR", "TERM", "COLORTERM"];
const STRIPPED_PREFIXES: [&str; 3] = ["CLAUDE_CODE_", "KITTY_", "ORCA_"];

/// The environment a session's program starts with.
pub fn clean_env(from: impl Iterator<Item = (String, String)>) -> Vec<(String, String)> {
    let mut env: Vec<_> = from.filter(|(key, _)| !is_stripped(key)).collect();
    env.push(("TERM".to_string(), "xterm-256color".to_string()));
    env.push(("COLORTERM".to_string(), "truecolor".to_string()));
    env
}

fn is_stripped(key: &str) -> bool {
    // Windows treats environment names as case-insensitive.
    let key = if cfg!(windows) {
        key.to_ascii_uppercase()
    } else {
        key.to_string()
    };
    STRIPPED.contains(&key.as_str())
        || STRIPPED_PREFIXES
            .iter()
            .any(|prefix| key.starts_with(prefix))
}

/// The host's own environment. Skips entries that aren't UTF-8, because
/// `std::env::vars` would panic on them.
pub(crate) fn host_env() -> impl Iterator<Item = (String, String)> {
    std::env::vars_os()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
}
