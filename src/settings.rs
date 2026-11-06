//! Tessella's own settings, from `config.toml`.
//!
//! Keys come from the person's tuios config first, so a tuios setup works here
//! with nothing to configure. This file holds what tuios has no setting for, and
//! optional changes to keys that apply in Tessella only.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const TEMPLATE: &str = r#"# Tessella settings. Every line is optional: delete what you do not need.
# Saving this file applies it. Your prefix and keybindings come from your tuios
# config (run `tuios keybinds list`), so change them there to change them in both.

[font]
# family = "JetBrainsMono Nerd Font Mono"   # Empty picks a Nerd Font if you have one.
# size = 13

[sidebar]
# visible = true
# width = 250
# agents_height = 190

[panes]
# confirm_close = true
# predict_remote_typing = true

[animation]
# enabled = true
# duration_ms = 140

[theme]
# Used only when your tuios config names no theme. Choose one with the theme picker.
# fallback = "tokyonight"
# Listed first in the theme picker. Star them there.
# favourites = ["tokyonight", "dracula"]

[notifications]
# enabled = true
# done = true   # Also when an agent finishes.

[keys]
# Changes the prefix in Tessella only.
# leader = "ctrl+a"
# How long the prefix waits before it shows its keys. 0 never shows them.
# which_key_ms = 500

# Keys after the prefix, in Tessella only. "none" removes a key.
# Actions are the names `tuios keybinds list` shows.
[keys.prefix]
# "n" = "new_window"
# "z" = "none"

# Keys with no prefix, in Tessella only.
# Tessella adds tessella_new_pane, tessella_new_workspace, tessella_themes and
# tessella_settings to the tuios actions.
[keys.app]
# "cmd+n" = "tessella_new_pane"
# "cmd+shift+t" = "tessella_themes"
# "cmd+," = "tessella_settings"
"#;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FontSettings {
    pub family: String,
    pub size: f32,
}

impl Default for FontSettings {
    fn default() -> Self {
        Self { family: String::new(), size: 13.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SidebarSettings {
    pub visible: bool,
    pub width: f32,
    pub agents_height: f32,
}

impl Default for SidebarSettings {
    fn default() -> Self {
        Self { visible: true, width: 250.0, agents_height: 190.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeSettings {
    pub fallback: String,
    pub favourites: Vec<String>,
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self { fallback: "tokyonight".to_string(), favourites: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PaneSettings {
    pub confirm_close: bool,
    /// Draw typed letters before a pane on another machine echoes them.
    pub predict_remote_typing: bool,
}

impl Default for PaneSettings {
    fn default() -> Self {
        Self { confirm_close: true, predict_remote_typing: true }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnimationSettings {
    pub enabled: bool,
    pub duration_ms: u64,
}

impl Default for AnimationSettings {
    fn default() -> Self {
        Self { enabled: true, duration_ms: 140 }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KeySettings {
    pub leader: String,
    pub prefix: BTreeMap<String, String>,
    pub app: BTreeMap<String, String>,
    /// How long the prefix waits before it shows its keys. 0 never shows them.
    pub which_key_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub font: FontSettings,
    pub sidebar: SidebarSettings,
    pub panes: PaneSettings,
    pub animation: AnimationSettings,
    pub theme: ThemeSettings,
    pub keys: KeySettings,
    pub notifications: NotificationSettings,
}

/// Mac notifications when an agent needs the person.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationSettings {
    pub enabled: bool,
    /// Also when an agent finishes, not only when it needs an answer or fails.
    pub done: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self { enabled: true, done: true }
    }
}

impl Default for KeySettings {
    fn default() -> Self {
        Self { leader: String::new(), prefix: BTreeMap::new(), app: BTreeMap::new(), which_key_ms: 500 }
    }
}

/// `$XDG_CONFIG_HOME/tessella/config.toml`, or under `~/.config`.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("tessella").join("config.toml"))
}

impl Settings {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut s: Self = toml::from_str(text).map_err(|e| e.message().to_string())?;
        s.font.size = s.font.size.clamp(6.0, 48.0);
        s.sidebar.width = s.sidebar.width.clamp(150.0, 600.0);
        s.animation.duration_ms = s.animation.duration_ms.min(2000);
        Ok(s)
    }

    /// The defaults when there is no file. An error says what is wrong with one.
    pub fn load() -> Result<Self, String> {
        let Some(path) = path() else { return Ok(Self::default()) };
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// Applies a change to the file's text, keeping its comments and layout.
pub fn edit_text(text: &str, edit: impl FnOnce(&mut toml_edit::DocumentMut)) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| e.message().to_string())?;
    edit(&mut doc);
    Ok(doc.to_string())
}

/// Changes the settings file, making it first if needed.
pub fn save(edit: impl FnOnce(&mut toml_edit::DocumentMut)) -> Result<(), String> {
    let path = ensure_file()?;
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    std::fs::write(&path, edit_text(&text, edit)?).map_err(|e| e.to_string())
}

/// Writes the commented template if there is no file yet, and gives its path.
pub fn ensure_file() -> Result<PathBuf, String> {
    let path = path().ok_or("HOME is not set")?;
    if !path.exists() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, TEMPLATE).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_parses_and_changes_nothing() {
        assert_eq!(Settings::parse(TEMPLATE).unwrap(), Settings::default());
    }

    #[test]
    fn every_template_example_is_valid_once_uncommented() {
        let live: String = TEMPLATE
            .lines()
            .map(|l| match l.strip_prefix("# ") {
                Some(rest) if rest.contains(" = ") => rest.split("   #").next().unwrap_or(rest),
                _ => l,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let s = Settings::parse(&live).unwrap();
        assert_eq!(s.keys.leader, "ctrl+a");
        assert_eq!(s.keys.prefix.get("z").map(String::as_str), Some("none"));
        assert_eq!(s.keys.app.get("cmd+n").map(String::as_str), Some("tessella_new_pane"));
        assert_eq!(s.font.size, 13.0);
    }

    #[test]
    fn an_edit_keeps_the_comments_and_reads_back() {
        let out = edit_text(TEMPLATE, |doc| {
            doc["font"]["size"] = toml_edit::value(15.0);
            doc["panes"]["confirm_close"] = toml_edit::value(false);
            doc["keys"]["prefix"]["n"] = toml_edit::value("new_window");
        })
        .unwrap();
        assert!(out.contains("# Tessella settings."));
        let s = Settings::parse(&out).unwrap();
        assert_eq!(s.font.size, 15.0);
        assert!(!s.panes.confirm_close);
        assert_eq!(s.keys.prefix.get("n").map(String::as_str), Some("new_window"));
    }

    #[test]
    fn favourite_themes_write_and_read_back_in_order() {
        let mut list = toml_edit::Array::new();
        list.push("nord");
        list.push("dracula");
        let out = edit_text(TEMPLATE, |doc| doc["theme"]["favourites"] = toml_edit::value(list)).unwrap();
        assert_eq!(Settings::parse(&out).unwrap().theme.favourites, ["nord", "dracula"]);
    }

    #[test]
    fn an_edit_can_create_a_table_the_file_does_not_have() {
        let out = edit_text("", |doc| doc["sidebar"]["width"] = toml_edit::value(300.0)).unwrap();
        assert_eq!(Settings::parse(&out).unwrap().sidebar.width, 300.0);
    }

    #[test]
    fn a_partial_file_keeps_the_other_defaults() {
        let s = Settings::parse("[font]\nsize = 15\n").unwrap();
        assert_eq!(s.font.size, 15.0);
        assert_eq!(s.sidebar, SidebarSettings::default());
    }

    #[test]
    fn a_misspelt_setting_is_an_error_not_silently_ignored() {
        assert!(Settings::parse("[font]\nsizee = 15\n").unwrap_err().contains("sizee"));
    }

    #[test]
    fn numbers_that_would_break_the_layout_are_clamped() {
        let s = Settings::parse("[font]\nsize = 1\n[sidebar]\nwidth = 99999\n").unwrap();
        assert_eq!((s.font.size, s.sidebar.width), (6.0, 600.0));
    }
}
