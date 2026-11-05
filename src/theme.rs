//! A tuios theme, and the window colours this client derives from it.
//!
//! Panes use the theme's terminal colours as they are. The chrome is mixed from
//! those same colours, so all 345 themes stay coherent, the light ones included.

use crate::terminal::Palette;
use serde_json::Value;

fn hex(v: &Value) -> Option<u32> {
    u32::from_str_radix(v.as_str()?.strip_prefix('#')?, 16).ok()
}

pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let channel = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xff) as f32, ((b >> shift) & 0xff) as f32);
        ((x + (y - x) * t).round() as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}

fn luminance(c: u32) -> f32 {
    let linear = |shift: u32| {
        let v = ((c >> shift) & 0xff) as f32 / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * linear(16) + 0.7152 * linear(8) + 0.0722 * linear(0)
}

pub fn contrast(a: u32, b: u32) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Black or white, whichever reads better on this colour.
pub fn on(background: u32) -> u32 {
    if contrast(background, 0x000000) >= contrast(background, 0xffffff) { 0x000000 } else { 0xffffff }
}

/// `color`, moved toward black or white only as far as it takes to reach `min`
/// contrast on `background`. Black or white always reaches at least 4.58 to 1,
/// so a `min` up to 4.5 is guaranteed. Many themes have a dim foreground.
pub fn readable(color: u32, background: u32, min: f32) -> u32 {
    let target = on(background);
    (0..=20)
        .map(|step| mix(color, target, step as f32 / 20.0))
        .find(|c| contrast(*c, background) >= min)
        .unwrap_or(target)
}

/// `tokyo_night` reads as `Tokyo Night`.
pub fn pretty(name: &str) -> String {
    name.split(['_', '-'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars.next().map_or(String::new(), |c| c.to_uppercase().chain(chars).collect())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub names: Vec<String>,
    pub light: bool,
    pub palette: Palette,
    pub cursor: u32,
    pub accent: u32,
    /// The accent, adjusted to read as text on the sidebar.
    pub accent_text: u32,
    pub on_accent: u32,
    pub stage: u32,
    pub sidebar: u32,
    pub rule: u32,
    pub text: u32,
    pub muted: u32,
    pub faint: u32,
    pub hover: u32,
    pub selected: u32,
    needs_input: u32,
    working: u32,
    done: u32,
    errored: u32,
    idle: u32,
}

struct Agents {
    needs_input: u32,
    working: u32,
    done: u32,
    errored: u32,
    idle: u32,
}

impl Theme {
    fn derive(name: String, names: Vec<String>, light: bool, palette: Palette, cursor: u32, accent: u32, a: Agents) -> Self {
        let (fg, bg) = (palette.fg, palette.bg);
        let stage = mix(bg, 0x000000, if light { 0.07 } else { 0.28 });
        let sidebar = if light { mix(bg, 0xffffff, 0.35) } else { mix(bg, 0x000000, 0.12) };
        let text = readable(fg, sidebar, 4.5);
        Self {
            name,
            names,
            light,
            palette,
            cursor,
            accent,
            accent_text: readable(accent, sidebar, 4.5),
            on_accent: on(accent),
            stage,
            sidebar,
            rule: mix(bg, fg, 0.14),
            text,
            muted: readable(mix(text, sidebar, 0.3), sidebar, 4.5),
            faint: readable(mix(text, sidebar, 0.5), sidebar, 3.0),
            hover: mix(sidebar, fg, 0.08),
            selected: mix(sidebar, accent, 0.22),
            needs_input: a.needs_input,
            working: a.working,
            done: a.done,
            errored: a.errored,
            idle: a.idle,
        }
    }

    /// Reads a bridge "theme" event's `theme` object.
    pub fn from_event(t: &Value) -> Option<Self> {
        let terminal = &t["terminal"];
        let fallback = Palette::default();
        let mut ansi = fallback.ansi;
        for (i, slot) in ansi.iter_mut().enumerate() {
            if let Some(c) = hex(&terminal["ansi"][i]) {
                *slot = c;
            }
        }
        let palette = Palette { fg: hex(&terminal["fg"])?, bg: hex(&terminal["bg"])?, ansi };
        let agent = |key: &str, or: u32| hex(&t["agent"][key]).unwrap_or(or);
        let agents = Agents {
            needs_input: agent("needs_input", ansi[3]),
            working: agent("working", ansi[4]),
            done: agent("done", ansi[2]),
            errored: agent("errored", ansi[1]),
            idle: agent("idle", mix(palette.fg, palette.bg, 0.5)),
        };
        Some(Self::derive(
            t["name"].as_str().unwrap_or_default().to_string(),
            t["names"]
                .as_array()
                .map(|names| names.iter().filter_map(|n| n.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            t["light"].as_bool().unwrap_or(false),
            palette,
            hex(&terminal["cursor"]).unwrap_or(palette.fg),
            hex(&t["ground"]["Accent"]).unwrap_or(ansi[4]),
            agents,
        ))
    }

    /// The colour of an agent's state, or None for a pane with no agent.
    pub fn agent(&self, state: &str) -> Option<u32> {
        match state {
            "needs_input" => Some(self.needs_input),
            "working" => Some(self.working),
            "done" => Some(self.done),
            "errored" => Some(self.errored),
            "idle" => Some(self.idle),
            _ => None,
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        let palette = Palette::default();
        let a = palette.ansi;
        let agents = Agents { needs_input: a[3], working: a[4], done: a[2], errored: a[1], idle: a[8] };
        Self::derive(String::new(), Vec::new(), false, palette, palette.fg, a[4], agents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dracula() -> Value {
        json!({
            "name": "dracula", "names": ["dracula", "nord"], "light": false,
            "terminal": {"fg": "#f8f8f2", "bg": "#1e1f28", "cursor": "#bbbbbb",
                "ansi": ["#000000", "#ff5555", "#50fa7b", "#f1fa8c", "#bd93f9", "#ff79c6", "#8be9fd", "#bbbbbb",
                         "#555555", "#ff5555", "#50fa7b", "#f1fa8c", "#bd93f9", "#ff79c6", "#8be9fd", "#ffffff"]},
            "ground": {"Accent": "#bd93f9"},
            "agent": {"done": "#50fa7b", "errored": "#ff5555", "idle": "#858392", "needs_input": "#f1fa8c", "working": "#bd93f9"}
        })
    }

    #[test]
    fn a_real_event_sets_the_terminal_colours_and_the_accent() {
        let t = Theme::from_event(&dracula()).unwrap();
        assert_eq!((t.palette.fg, t.palette.bg, t.palette.ansi[1]), (0xf8f8f2, 0x1e1f28, 0xff5555));
        assert_eq!((t.cursor, t.accent), (0xbbbbbb, 0xbd93f9));
        assert_eq!(t.agent("needs_input"), Some(0xf1fa8c));
        assert_eq!(t.names, ["dracula", "nord"]);
    }

    #[test]
    fn the_default_theme_sends_empty_ansi_and_keeps_the_built_in_palette() {
        // The bridge reports this before any theme is chosen.
        let t = Theme::from_event(&json!({"name": "", "terminal": {"fg": "#e5e5e5", "bg": "#000000", "ansi": ["", ""]}})).unwrap();
        assert_eq!(t.palette.bg, 0x000000);
        assert_eq!(t.palette.ansi, Palette::default().ansi);
    }

    #[test]
    fn a_pane_with_no_agent_has_no_status_colour() {
        let t = Theme::default();
        assert_eq!(t.agent(""), None);
        assert_eq!(t.agent("none"), None);
    }

    #[test]
    fn chrome_stays_readable_on_light_and_dark_grounds() {
        for (bg, fg, light) in [(0x1e1f28, 0xf8f8f2, false), (0xfafafa, 0x383a42, true)] {
            let palette = Palette { fg, bg, ansi: Palette::default().ansi };
            let a = palette.ansi;
            let agents = Agents { needs_input: a[3], working: a[4], done: a[2], errored: a[1], idle: a[8] };
            let t = Theme::derive(String::new(), Vec::new(), light, palette, fg, a[4], agents);
            assert!(contrast(t.text, t.sidebar) >= 4.5, "text on {bg:06x}");
            assert!(contrast(t.muted, t.sidebar) >= 3.0, "muted on {bg:06x}");
        }
    }

    #[test]
    fn chrome_text_meets_its_contrast_for_any_foreground_and_background() {
        let shades = (0..=255u32).step_by(15).map(|v| v << 16 | v << 8 | v);
        let tints = [0xff0000, 0x00ff00, 0x0000ff, 0xffff00, 0x808000, 0x008080, 0x787c99];
        let colors: Vec<u32> = shades.chain(tints).collect();
        for &bg in &colors {
            for &fg in &colors {
                let palette = Palette { fg, bg, ansi: Palette::default().ansi };
                let a = palette.ansi;
                let agents = Agents { needs_input: a[3], working: a[4], done: a[2], errored: a[1], idle: a[8] };
                let light = contrast(bg, 0x000000) > contrast(bg, 0xffffff);
                let t = Theme::derive(String::new(), Vec::new(), light, palette, fg, a[4], agents);
                assert!(contrast(t.text, t.sidebar) >= 4.5, "text {fg:06x} on {bg:06x}");
                assert!(contrast(t.muted, t.sidebar) >= 4.5, "muted {fg:06x} on {bg:06x}");
                assert!(contrast(t.faint, t.sidebar) >= 3.0, "faint {fg:06x} on {bg:06x}");
            }
        }
    }

    #[test]
    fn a_readable_colour_is_left_exactly_as_the_theme_gave_it() {
        assert_eq!(readable(0xffffff, 0x000000, 4.5), 0xffffff);
    }

    #[test]
    fn text_on_an_accent_picks_the_readable_colour() {
        assert_eq!(on(0xfff27a), 0x000000);
        assert_eq!(on(0x1e1f28), 0xffffff);
    }

    #[test]
    fn names_read_like_titles() {
        assert_eq!(pretty("tokyo_night_light"), "Tokyo Night Light");
        assert_eq!(pretty("3024_day"), "3024 Day");
    }
}
