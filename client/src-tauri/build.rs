use serde_json::{Value, json};

fn main() {
    // tauri-build insists the sidecar file exists and copies it over target/<profile>/term-host.
    // Only a release bundle wants that. In a debug build it would make clippy fail on a fresh
    // checkout, and would replace the debug host, the only one that accepts the dev server's
    // origin, with the release one.
    if std::env::var("PROFILE").as_deref() != Ok("release") {
        let mut config: Value = std::env::var("TAURI_CONFIG")
            .ok()
            .and_then(|config| serde_json::from_str(&config).ok())
            .unwrap_or_else(|| json!({}));
        config["bundle"]["externalBin"] = Value::Null;
        // SAFETY: a build script's main has no other threads yet.
        unsafe { std::env::set_var("TAURI_CONFIG", config.to_string()) };
    }
    tauri_build::build()
}
