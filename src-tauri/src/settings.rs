/// App-wide user settings (theme, font size, default shell, prefix key),
/// persisted as a single JSON blob in SQLite. Distinct from `workspace.rs`'s
/// `vmux_meta` table — this is app-level config, not per-workspace state.
use serde::{Deserialize, Serialize};
use rusqlite::{params, Connection};
use anyhow::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// "tokyo_night" | "catppuccin_mocha"
    pub theme_name: String,
    pub default_shell_id: Option<String>,
    pub font_size: f32,
    /// Single lowercase letter used as the tmux-style prefix key (Ctrl+<letter>).
    pub prefix_key: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme_name: "tokyo_night".to_string(),
            default_shell_id: None,
            font_size: 14.0,
            prefix_key: "a".to_string(),
        }
    }
}

const SETTINGS_KEY: &str = "settings";

fn ensure_table(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);"
    )?;
    Ok(())
}

pub fn load(db_path: &str) -> Result<Settings> {
    let db = Connection::open(db_path)?;
    ensure_table(&db)?;
    let value: Option<String> = db.query_row(
        "SELECT value FROM app_settings WHERE key = ?1",
        [SETTINGS_KEY],
        |row| row.get(0),
    ).ok();

    Ok(value
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default())
}

pub fn save(db_path: &str, settings: &Settings) -> Result<()> {
    let db = Connection::open(db_path)?;
    ensure_table(&db)?;
    let json = serde_json::to_string(settings)?;
    db.execute(
        "INSERT OR REPLACE INTO app_settings (key, value) VALUES (?1, ?2)",
        params![SETTINGS_KEY, json],
    )?;
    Ok(())
}

pub fn theme_from_name(name: &str) -> crate::theme::Theme {
    match name {
        "catppuccin_mocha" => crate::theme::Theme::catppuccin_mocha(),
        _ => crate::theme::Theme::tokyo_night(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDb(std::path::PathBuf);
    impl TempDb {
        fn new() -> Self {
            TempDb(std::env::temp_dir().join(format!("vmux_settings_test_{}.db", uuid::Uuid::new_v4())))
        }
        fn path(&self) -> &str { self.0.to_str().unwrap() }
    }
    impl Drop for TempDb {
        fn drop(&mut self) { let _ = std::fs::remove_file(&self.0); }
    }

    #[test]
    fn load_returns_defaults_when_unset() {
        let db = TempDb::new();
        let settings = load(db.path()).unwrap();
        assert_eq!(settings.theme_name, "tokyo_night");
        assert_eq!(settings.prefix_key, "a");
        assert_eq!(settings.font_size, 14.0);
    }

    #[test]
    fn save_and_load_round_trip() {
        let db = TempDb::new();
        let custom = Settings {
            theme_name: "catppuccin_mocha".to_string(),
            default_shell_id: Some("powershell".to_string()),
            font_size: 16.5,
            prefix_key: "g".to_string(),
        };
        save(db.path(), &custom).unwrap();

        let loaded = load(db.path()).unwrap();
        assert_eq!(loaded.theme_name, "catppuccin_mocha");
        assert_eq!(loaded.default_shell_id.as_deref(), Some("powershell"));
        assert_eq!(loaded.font_size, 16.5);
        assert_eq!(loaded.prefix_key, "g");
    }
}
