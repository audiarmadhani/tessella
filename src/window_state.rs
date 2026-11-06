//! What Tessella remembers between launches: the window, the zoom, and what it learned
//! about how tuios lays out a session. With these it can ask for the right size at once,
//! rather than resize every pane in the session to a default and back.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    /// The screen it was on, and where on that screen.
    pub display: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub maximized: bool,
    pub fullscreen: bool,
    /// The zoom, and the size in the settings it was zoomed from. The zoom counts only
    /// while the settings still say that size.
    pub font_size: f32,
    pub settings_font_size: f32,
    pub gap: i32,
    pub chrome: (u16, u16),
    pub border: (i32, i32),
    /// Diffs in two columns rather than one.
    pub diff_side_by_side: bool,
}

fn path() -> Option<PathBuf> {
    Some(crate::settings::path()?.with_file_name("state.json"))
}

impl WindowState {
    /// Whether a window this size is one to open. A hand-edited or broken file is not.
    pub fn usable(&self) -> bool {
        self.w >= 300.0 && self.h >= 200.0 && self.w.is_finite() && self.h.is_finite() && self.x.is_finite() && self.y.is_finite()
    }

    /// The zoom to open at, given the size the settings say now.
    pub fn zoom(&self, settings_font_size: f32) -> Option<f32> {
        (self.settings_font_size == settings_font_size && self.font_size > 0.0).then_some(self.font_size)
    }

    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_str::<Self>(text).ok().filter(Self::usable)
    }

    pub fn load() -> Option<Self> {
        Self::parse(&std::fs::read_to_string(path()?).ok()?)
    }

    pub fn save(&self) {
        let (Some(path), Ok(text)) = (path(), serde_json::to_string_pretty(self)) else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Written whole and moved into place, so that a crash cannot leave half a file.
        let temp = path.with_extension("json.tmp");
        if std::fs::write(&temp, text).is_ok() {
            let _ = std::fs::rename(temp, path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> WindowState {
        WindowState { display: 2, x: 10.0, y: 20.0, w: 1710.0, h: 1073.0, font_size: 11.0, settings_font_size: 12.0, gap: 0, chrome: (39, 0), border: (2, 2), ..Default::default() }
    }

    #[test]
    fn it_reads_back_what_it_wrote() {
        let text = serde_json::to_string(&state()).unwrap();
        assert_eq!(WindowState::parse(&text), Some(state()));
    }

    #[test]
    fn a_broken_or_tiny_window_is_not_opened() {
        assert_eq!(WindowState::parse("not json"), None);
        assert_eq!(WindowState::parse(r#"{"w": 10, "h": 10}"#), None);
        assert_eq!(WindowState::parse("{}"), None);
    }

    #[test]
    fn a_new_field_or_a_missing_one_does_not_lose_the_window() {
        let s = WindowState::parse(r#"{"w": 800, "h": 600, "from_the_future": 1}"#).unwrap();
        assert_eq!((s.w, s.gap), (800.0, 0));
    }

    #[test]
    fn the_zoom_lasts_until_the_settings_change_the_size() {
        assert_eq!(state().zoom(12.0), Some(11.0));
        assert_eq!(state().zoom(13.0), None);
    }
}
