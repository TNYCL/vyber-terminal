use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub shell: Option<String>,
    pub font_family: String,
    pub font_size: f32,
    pub scrollback: usize,
    pub reduced_motion: bool,
    pub notifications: bool,
    pub restore_workspace: bool,
    pub task_history_days: u64,
    pub task_history_limit: usize,
    /// Fetch every few minutes while the Git panel is open.
    pub git_autofetch: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            shell: None,
            font_family: if cfg!(windows) { "Consolas" } else { "Menlo" }.into(),
            font_size: 14.,
            scrollback: 20_000,
            reduced_motion: false,
            notifications: true,
            restore_workspace: true,
            task_history_days: 14,
            task_history_limit: 200,
            git_autofetch: true,
        }
    }
}
impl Config {
    pub fn path() -> PathBuf {
        crate::workspace::data_dir().join("config.toml")
    }
    pub fn load() -> Self {
        let path = Self::path();
        let mut config = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| toml::from_str::<Self>(&s).ok())
            .unwrap_or_default();
        config.font_size = config.font_size.clamp(9., 32.);
        config.scrollback = config.scrollback.clamp(1000, 100_000);
        if !path.exists() {
            let _ = std::fs::create_dir_all(path.parent().unwrap());
            if let Ok(s) = toml::to_string_pretty(&config) {
                let _ = std::fs::write(path, s);
            }
        }
        config
    }
}
