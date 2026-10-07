use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MIN_TIMEOUT_MS: u64 = 50;
pub const MAX_TIMEOUT_MS: u64 = 2000;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub remapping_enabled: bool,
    pub escape_timeout_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            remapping_enabled: true,
            escape_timeout_ms: 300,
        }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        let config: Self =
            serde_json::from_str(text).map_err(|e| format!("Invalid settings JSON: {e}"))?;
        if !(MIN_TIMEOUT_MS..=MAX_TIMEOUT_MS).contains(&config.escape_timeout_ms) {
            return Err(format!(
                "escape_timeout_ms must be {MIN_TIMEOUT_MS}–{MAX_TIMEOUT_MS}"
            ));
        }
        Ok(config)
    }
}

pub fn path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is unavailable; cannot locate settings")?;
    Ok(PathBuf::from(home).join("Library/Application Support/Caps Tap/settings.json"))
}

pub fn load(path: &Path) -> Result<Config, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    Config::parse(&text)
}

pub fn load_or_create(path: &Path, legacy_enabled: bool) -> Result<Config, String> {
    match fs::read_to_string(path) {
        Ok(text) => Config::parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let config = Config {
                remapping_enabled: legacy_enabled,
                ..Config::default()
            };
            save(path, &config)?;
            Ok(config)
        }
        Err(e) => Err(format!("Could not read {}: {e}", path.display())),
    }
}

pub fn save(path: &Path, config: &Config) -> Result<(), String> {
    use std::io::Write;
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    Config::parse(&text)?;
    let parent = path.parent().ok_or("Settings path has no parent")?;
    fs::create_dir_all(parent).map_err(|e| format!("Could not create settings directory: {e}"))?;
    // Same-directory rename prevents a crash during writing from truncating the
    // live config. create_new avoids overwriting another writer's temporary file.
    let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| format!("Could not create temporary settings: {e}"))?;
    let result = (|| {
        writeln!(file, "{text}")?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(|e| format!("Could not save settings: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_defaults_types_and_timeout_bounds() {
        assert_eq!(Config::parse("{}").unwrap(), Config::default());
        for text in [
            "{",
            r#"{"escape_timeout_ms":49}"#,
            r#"{"escape_timeout_ms":2001}"#,
            r#"{"escape_timeout_ms":-1}"#,
            r#"{"escape_timeout_ms":"300"}"#,
            r#"{"escape_timeot_ms":500}"#,
        ] {
            assert!(Config::parse(text).is_err(), "{text}");
        }
        assert_eq!(
            Config::parse(r#"{"escape_timeout_ms":500}"#)
                .unwrap()
                .escape_timeout_ms,
            500
        );
    }

    #[test]
    fn creates_migrates_reloads_and_does_not_overwrite_invalid_json() {
        let dir = std::env::temp_dir().join(format!("caps-tap-config-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let config = load_or_create(&path, false).unwrap();
        assert!(!config.remapping_enabled);
        let updated = Config {
            escape_timeout_ms: 500,
            ..config
        };
        save(&path, &updated).unwrap();
        assert_eq!(load_or_create(&path, true).unwrap(), updated);
        fs::write(&path, "bad JSON").unwrap();
        assert!(load(&path).is_err());
        assert!(load_or_create(&path, true).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "bad JSON");
        fs::remove_dir_all(dir).unwrap();
    }
}
