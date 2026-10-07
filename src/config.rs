use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::SystemTime};

/// Text size the file and Git panels are laid out at; other sizes scale them.
pub const PANEL_FONT_SIZE: f32 = 12.;

/// Where the file panel sits: over the terminal, or beside it with the
/// terminal made narrower.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelMode {
    #[default]
    Overlay,
    Dock,
}

/// The window edge the toolbelt sits on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolbeltSide {
    Left,
    #[default]
    Right,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub shell: Option<String>,
    pub font_family: String,
    /// Terminal text size.
    pub font_size: f32,
    /// Text size of the file panel: tree, tabs, preview, editor and Review.
    pub files_font_size: f32,
    /// Text size of the source control panel.
    pub git_font_size: f32,
    pub panel_mode: PanelMode,
    pub scrollback: usize,
    pub reduced_motion: bool,
    pub notifications: bool,
    pub restore_workspace: bool,
    pub task_history_days: u64,
    pub task_history_limit: usize,
    /// Fetch every few minutes while the Git panel is open.
    pub git_autofetch: bool,
    /// Look for a new Vyber release now and then and download it in the
    /// background, ready to install from the title bar.
    pub check_for_updates: bool,
    /// Show the toolbelt: the focused terminal's jobs and every agent's status.
    pub toolbelt: bool,
    pub toolbelt_side: ToolbeltSide,
    /// Toolbelt width in pixels.
    pub toolbelt_width: f32,
    /// The share of the toolbelt's height that Jobs takes.
    pub toolbelt_split: f64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            shell: None,
            font_family: crate::theme::mono_font().into(),
            font_size: 14.,
            files_font_size: PANEL_FONT_SIZE,
            git_font_size: PANEL_FONT_SIZE,
            panel_mode: PanelMode::Overlay,
            scrollback: 20_000,
            reduced_motion: false,
            notifications: true,
            restore_workspace: true,
            task_history_days: 14,
            task_history_limit: 200,
            git_autofetch: true,
            check_for_updates: true,
            toolbelt: true,
            toolbelt_side: ToolbeltSide::Right,
            toolbelt_width: 300.,
            toolbelt_split: 0.45,
        }
    }
}
/// The settings in use, reloaded when config.toml changes.
impl gpui::Global for Config {}
impl Config {
    pub fn path() -> PathBuf {
        crate::workspace::data_dir().join("config.toml")
    }
    /// When config.toml last changed.
    pub fn modified() -> Option<SystemTime> {
        std::fs::metadata(Self::path())
            .and_then(|m| m.modified())
            .ok()
    }
    /// The settings in config.toml, or `None` while it is missing or does not
    /// parse, say half-way through an edit.
    pub fn read() -> Option<Self> {
        let text = std::fs::read_to_string(Self::path()).ok()?;
        let mut config = toml::from_str::<Self>(&text).ok()?;
        config.clamp();
        Some(config)
    }
    pub fn load() -> Self {
        let path = Self::path();
        let text = std::fs::read_to_string(&path).ok();
        let mut config = text
            .as_deref()
            .and_then(|s| toml::from_str::<Self>(s).ok())
            .unwrap_or_default();
        config.clamp();
        match text {
            None => {
                let _ = std::fs::create_dir_all(path.parent().unwrap());
                if let Ok(s) = toml::to_string_pretty(&config) {
                    let _ = std::fs::write(path, s);
                }
            }
            // Settings added since the file was written show up with their defaults.
            Some(text) => {
                if let Ok(written) = toml::from_str::<toml::Table>(&text) {
                    let table = config.table();
                    let mut edited = text.clone();
                    // In the order of the struct, which `table` does not keep.
                    let order = toml::to_string(&config).unwrap_or_default();
                    for key in order
                        .lines()
                        .filter_map(|l| l.split_once(" = ").map(|(k, _)| k))
                    {
                        if let Some(value) = table.get(key).filter(|_| !written.contains_key(key)) {
                            edited = set_key(&edited, key, value);
                        }
                    }
                    if edited != text {
                        let _ = std::fs::write(path, edited);
                    }
                }
            }
        }
        config
    }
    /// Changes settings in use and writes the changed keys to config.toml,
    /// leaving the rest of the file, comments included, as it is.
    pub fn update(cx: &mut gpui::App, change: impl FnOnce(&mut Config)) {
        let old = cx.global::<Config>().clone();
        let mut new = old.clone();
        change(&mut new);
        new.clamp();
        if new == old {
            return;
        }
        let path = Self::path();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let before = old.table();
        let mut edited = text.clone();
        for (key, value) in new.table() {
            if before.get(&key) != Some(&value) {
                edited = set_key(&edited, &key, &value);
            }
        }
        if edited != text {
            let _ = std::fs::create_dir_all(path.parent().unwrap());
            if let Err(e) = std::fs::write(&path, edited) {
                log::warn!("Save settings: {e}");
            }
        }
        cx.set_global(new);
    }
    fn clamp(&mut self) {
        self.font_size = self.font_size.clamp(8., 32.);
        self.files_font_size = self.files_font_size.clamp(8., 24.);
        self.git_font_size = self.git_font_size.clamp(8., 24.);
        self.scrollback = self.scrollback.clamp(1000, 100_000);
        self.toolbelt_width = if self.toolbelt_width.is_finite() {
            self.toolbelt_width.clamp(200., 640.).round()
        } else {
            300.
        };
        self.toolbelt_split = if self.toolbelt_split.is_finite() {
            (self.toolbelt_split.clamp(0.15, 0.85) * 100.).round() / 100.
        } else {
            0.45
        };
    }
    fn table(&self) -> toml::Table {
        toml::Table::try_from(self).unwrap_or_default()
    }
}

/// `text` with `key` set to `value`: its line rewritten in place, keeping a
/// trailing comment, or a new line added before the first table.
fn set_key(text: &str, key: &str, value: &toml::Value) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
    let tables = lines
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .unwrap_or(lines.len());
    let assignment = format!("{key} = {value}");
    let found = lines[..tables].iter().position(|l| {
        l.trim_start()
            .strip_prefix(key)
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    });
    match found {
        Some(i) => {
            lines[i] = match comment(&lines[i]) {
                Some(comment) => format!("{assignment} {comment}"),
                None => assignment,
            }
        }
        None => {
            // Keep blank lines that end the top-level keys after the new one.
            let at = lines[..tables]
                .iter()
                .rposition(|l| !l.trim().is_empty())
                .map_or(0, |i| i + 1);
            lines.insert(at, assignment);
        }
    }
    let mut out = lines.join(newline);
    out.push_str(newline);
    out
}

/// The `# …` comment ending a line, outside any quoted string.
fn comment(line: &str) -> Option<&str> {
    let mut quote = None;
    for (i, c) in line.char_indices() {
        match (c, quote) {
            ('"' | '\'', None) => quote = Some(c),
            (c, Some(q)) if c == q => quote = None,
            ('#', None) => return Some(&line[i..]),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_key_keeps_the_rest_of_the_file() {
        let text = "# my settings\nshell = 'C:/Git/bin/bash.exe' # bash\nfont_size = 14.0 # terminal\nscrollback = 20000\n";
        let edited = set_key(text, "font_size", &toml::Value::Float(15.));
        assert_eq!(
            edited,
            "# my settings\nshell = 'C:/Git/bin/bash.exe' # bash\nfont_size = 15.0 # terminal\nscrollback = 20000\n"
        );
        let added = set_key(&edited, "panel_mode", &toml::Value::String("dock".into()));
        assert!(added.ends_with("scrollback = 20000\npanel_mode = \"dock\"\n"));
        // A key that only starts like another is left alone.
        let text = "font_size_extra = 1\n";
        assert_eq!(
            set_key(text, "font_size", &toml::Value::Float(9.)),
            "font_size_extra = 1\nfont_size = 9.0\n"
        );
    }

    #[test]
    fn set_key_stays_above_tables_and_keeps_crlf() {
        let text = "font_size = 14.0\r\n\r\n[extra]\r\nfont_size = 1\r\n";
        assert_eq!(
            set_key(text, "git_font_size", &toml::Value::Float(13.)),
            "font_size = 14.0\r\ngit_font_size = 13.0\r\n\r\n[extra]\r\nfont_size = 1\r\n"
        );
    }

    #[test]
    fn comments_inside_strings_are_values() {
        assert_eq!(comment("shell = \"a#b\" # real"), Some("# real"));
        assert_eq!(comment("shell = 'a#b'"), None);
    }

    #[test]
    fn panel_mode_reads_as_a_word() {
        let config: Config = toml::from_str("panel_mode = \"dock\"").unwrap();
        assert_eq!(config.panel_mode, PanelMode::Dock);
        assert_eq!(config.table()["panel_mode"].as_str(), Some("dock"));
    }
}
