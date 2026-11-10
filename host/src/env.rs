const STRIPPED: [&str; 4] = ["CLAUDECODE", "CLAUDE_CONFIG_DIR", "TERM", "COLORTERM"];
const STRIPPED_PREFIXES: [&str; 3] = ["CLAUDE_CODE_", "KITTY_", "ORCA_"];

/// The environment a session's program starts with.
pub fn clean_env(from: impl Iterator<Item = (String, String)>) -> Vec<(String, String)> {
    let mut env: Vec<_> = from.filter(|(key, _)| !is_stripped(key)).collect();
    env.push(("TERM".to_string(), "xterm-256color".to_string()));
    env.push(("COLORTERM".to_string(), "truecolor".to_string()));
    // An app opened from Finder has no locale, and a shell without one loses UTF-8.
    if cfg!(target_os = "macos") && !has_locale(&env) {
        env.push(("LANG".to_string(), "en_US.UTF-8".to_string()));
    }
    env
}

fn has_locale(env: &[(String, String)]) -> bool {
    env.iter()
        .any(|(key, _)| matches!(key.as_str(), "LANG" | "LC_ALL" | "LC_CTYPE"))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(names: &[&str]) -> Vec<(String, String)> {
        names
            .iter()
            .map(|name| (name.to_string(), "fr_FR.UTF-8".to_string()))
            .collect()
    }

    fn lang(env: &[(String, String)]) -> Vec<&str> {
        env.iter()
            .filter(|(key, _)| key == "LANG")
            .map(|(_, value)| value.as_str())
            .collect()
    }

    #[test]
    fn a_locale_is_added_on_macos_only_when_none_is_set() {
        let env = clean_env(pairs(&["HOME"]).into_iter());
        let expected: &[&str] = if cfg!(target_os = "macos") {
            &["en_US.UTF-8"]
        } else {
            &[]
        };
        assert_eq!(lang(&env), expected);
    }

    #[test]
    fn a_locale_already_set_is_left_alone() {
        let env = clean_env(pairs(&["LANG"]).into_iter());
        assert_eq!(lang(&env), ["fr_FR.UTF-8"]);

        for other in ["LC_ALL", "LC_CTYPE"] {
            let env = clean_env(pairs(&[other]).into_iter());
            assert!(lang(&env).is_empty(), "{other} set: {env:?}");
        }
    }
}
