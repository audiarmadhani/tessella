//! A person's tuios keys: the leader, the prefix tables, and direct chords.
//!
//! tuios sends these in its "keybinds" event and leaves the key handling to the
//! renderer. Each key ends as a tuios action name for the bridge, or goes to the
//! pane. A renderer that drew no palette or menus would swallow a chord like
//! ctrl+p for nothing, so direct chords are limited to the ones that act alone.

use crate::settings::KeySettings;
use serde_json::Value;
use std::collections::HashMap;

/// What one key press does.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// The key belongs to the pane.
    Forward,
    /// The key was used up, such as a leader or an unbound key after one.
    Consumed,
    /// Run this tuios action.
    Action(String),
    /// The leader pressed twice: type it into the pane.
    SendLeader,
}

#[derive(Debug, Clone, PartialEq)]
enum Mode {
    Idle,
    Prefix(String),
    /// A repeatable key just ran: the same scope stays live until this time.
    Repeat(String, u64),
}

/// Puts a key name in one spelling: tuios writes `opt+esc` and `super+v`.
pub fn canonical(name: &str) -> String {
    if name == "+" || !name.contains('+') {
        return name.to_string();
    }
    let (mods, key) = match name.rsplit_once('+') {
        Some(("", key)) | Some((_, key)) if key.is_empty() => return name.to_string(),
        Some((mods, key)) => (mods, key),
        None => return name.to_string(),
    };
    let has = |names: &[&str]| mods.split('+').any(|m| names.contains(&m));
    let mut out = String::new();
    for (names, spelled) in [
        (&["ctrl"][..], "ctrl+"),
        (&["alt", "opt"][..], "alt+"),
        (&["shift"][..], "shift+"),
        (&["cmd", "super"][..], "cmd+"),
    ] {
        if has(names) {
            out.push_str(spelled);
        }
    }
    out.push_str(key);
    out
}

/// The name of a key press, in the spelling `canonical` gives. A printable key
/// is its character, so shift+\ is `|`, as tuios writes it.
pub fn key_name(key: &str, key_char: Option<&str>, ctrl: bool, alt: bool, shift: bool, cmd: bool) -> String {
    let plain = !ctrl && !alt && !cmd;
    let named = match key {
        "escape" => "esc",
        "pageup" => "pgup",
        "pagedown" => "pgdown",
        other => other,
    };
    if plain && named != "space" && named.chars().count() == 1 {
        if let Some(text) = key_char.filter(|t| !t.is_empty()) {
            return text.to_string();
        }
        return if shift { named.to_uppercase() } else { named.to_string() };
    }
    if plain {
        if let Some(text) = key_char.filter(|t| t.chars().count() == 1 && *t != " " && *t != "\r" && *t != "\t") {
            if !text.chars().next().is_some_and(char::is_control) {
                return text.to_string();
            }
        }
    }
    let mut out = String::new();
    for (on, spelled) in [(ctrl, "ctrl+"), (alt, "alt+"), (shift, "shift+"), (cmd, "cmd+")] {
        if on {
            out.push_str(spelled);
        }
    }
    out.push_str(named);
    out
}

#[derive(Debug, Default)]
pub struct Keymap {
    leader: String,
    repeat_ms: u64,
    /// Scope, then key, then action.
    scopes: HashMap<String, HashMap<String, String>>,
    /// An action that opens a sub-prefix, and the scope its keys are read from.
    prefixes: HashMap<String, String>,
    /// An action's own description, for a message about it.
    descriptions: HashMap<String, String>,
    /// The groups of the leader's menu, as (title, [(key, description)]).
    help: Vec<(String, Vec<(String, String)>)>,
    mode: Option<Mode>,
}

/// A direct chord this client carries out, so it is worth taking from the pane.
fn direct(action: &str) -> bool {
    matches!(
        action,
        "next_workspace"
            | "prev_workspace"
            | "move_workspace_left"
            | "move_workspace_right"
            | "command_palette"
            | "launcher"
            | "terminal_next_window"
            | "terminal_prev_window"
            | "terminal_paste_host"
            | "terminal_scroll_up"
            | "terminal_scroll_down"
            | "terminal_scroll_page_up"
            | "terminal_scroll_page_down"
    ) || action.starts_with("terminal_focus_")
}

/// tuios's GUI chords, as its GUI reads them on macOS: ctrl+shift is cmd. A chord
/// on tab stays, because macOS keeps cmd+tab for itself.
fn gui_chord(key: &str) -> String {
    let key = canonical(key);
    if key.ends_with("tab") {
        return key;
    }
    match key.strip_prefix("ctrl+") {
        Some(rest) if rest.contains("shift+") => {
            let without = rest.replacen("shift+", "", 1);
            match without.strip_prefix("alt+") {
                Some(tail) => format!("alt+cmd+{tail}"),
                None => format!("cmd+{without}"),
            }
        }
        _ => key,
    }
}

/// The sections of the key list, in the order they are shown.
pub const KEY_SECTIONS: [&str; 7] = ["Panes", "Workspaces", "Sessions", "Agents", "Copy and paste", "Interface", "Your commands"];

/// Which section of the key list an action belongs to.
pub fn key_section(action: &str) -> &'static str {
    let has = |words: &[&str]| words.iter().any(|w| action.contains(w));
    if action.starts_with("command:") {
        "Your commands"
    } else if has(&["selection", "paste", "buffer", "scrollback", "copy"]) {
        "Copy and paste"
    } else if has(&["agent", "attention", "finished", "inbox", "mail", "message", "notif", "review"]) {
        "Agents"
    } else if has(&["session", "worktree", "detach", "goto", "choose_tree", "prefix_quit"]) {
        "Sessions"
    } else if has(&["workspace", "break_pane"]) {
        "Workspaces"
    } else if has(&["sidebar", "rail", "spotlight", "explore", "file", "settings", "keybinds", "help", "palette", "launcher", "theme", "transfers", "tape", "screenshot", "debug", "layout", "reload", "font"]) {
        "Interface"
    } else {
        "Panes"
    }
}

/// The bridge does not say which keys repeat. These are the ones tuios defaults to.
fn repeats(action: &str) -> bool {
    action.starts_with("resize_pane_")
}

impl Keymap {
    /// Reads a bridge "keybinds" event's `keybinds` object.
    pub fn from_event(k: &Value) -> Self {
        let mut scopes: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut descriptions = HashMap::new();
        for b in k["bindings"].as_array().into_iter().flatten() {
            let (Some(scope), Some(key), Some(action)) = (b["scope"].as_str(), b["key"].as_str(), b["action"].as_str()) else {
                continue;
            };
            let chord = if scope == "gui" { gui_chord(key) } else { canonical(key) };
            scopes.entry(scope.to_string()).or_default().insert(chord, action.to_string());
            if let Some(text) = b["description"].as_str() {
                descriptions.insert(action.to_string(), text.to_string());
            }
        }
        let help = k["menus"][""]
            .as_array()
            .into_iter()
            .flatten()
            .map(|group| {
                let rows = group["rows"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|r| (r["key"].as_str().unwrap_or_default().to_string(), r["description"].as_str().unwrap_or_default().to_string()))
                    .collect();
                (group["title"].as_str().unwrap_or_default().to_string(), rows)
            })
            .collect();
        let prefixes = k["prefixes"]
            .as_object()
            .map(|p| p.iter().filter_map(|(a, s)| Some((a.clone(), s.as_str()?.to_string()))).collect())
            .unwrap_or_default();
        Self {
            leader: canonical(k["leader"].as_str().unwrap_or_default()),
            repeat_ms: k["repeat_ms"].as_u64().unwrap_or(0),
            scopes,
            prefixes,
            descriptions,
            help,
            mode: Some(Mode::Idle),
        }
    }

    pub fn leader(&self) -> &str {
        &self.leader
    }

    /// Adds Tessella's own chords, then applies the person's changes. A key mapped
    /// to "none" is removed. Chords with no prefix live with tuios's GUI chords.
    pub fn with_settings(mut self, keys: &KeySettings) -> Self {
        let gui = self.scopes.entry("gui".to_string()).or_default();
        let mut own = vec![
            ("cmd+n".to_string(), "tessella_new_pane".to_string()),
            ("shift+cmd+t".to_string(), "tessella_themes".to_string()),
            ("shift+cmd+d".to_string(), "tessella_review".to_string()),
            ("cmd+,".to_string(), "tessella_settings".to_string()),
        ];
        own.extend((1..=9).map(|n| (format!("cmd+{n}"), format!("switch_workspace_{n}"))));
        for (key, action) in own {
            gui.entry(key).or_insert(action);
        }
        // Zoom is what these mean on a Mac, so they win over tuios's GUI table, which
        // has cmd+= for equalize. That stays on the prefix. A shifted key can arrive
        // named either way.
        for (key, action) in [
            ("cmd+=", "gui_font_bigger"),
            ("shift+cmd+=", "gui_font_bigger"),
            ("cmd++", "gui_font_bigger"),
            ("shift+cmd++", "gui_font_bigger"),
            ("cmd+-", "gui_font_smaller"),
            ("cmd+0", "gui_font_reset"),
        ] {
            gui.insert(key.to_string(), action.to_string());
        }
        // These edit text on a Mac. tuios's actions on them stay on the keys tuios
        // bound them to, ctrl+shift plus the key.
        for key in ["left", "right", "backspace"] {
            if let Some(action) = gui.remove(&format!("cmd+{key}")) {
                gui.entry(format!("ctrl+shift+{key}")).or_insert(action);
            }
        }
        for (scope, table) in [("prefix", &keys.prefix), ("gui", &keys.app)] {
            let scope = self.scopes.entry(scope.to_string()).or_default();
            for (key, action) in table {
                if action == "none" {
                    scope.remove(&canonical(key));
                } else {
                    scope.insert(canonical(key), action.clone());
                }
            }
        }
        if !keys.leader.is_empty() {
            self.leader = canonical(&keys.leader);
        }
        self
    }

    /// The keys after the prefix, as (key, action, description), by description.
    pub fn prefix_rows(&self) -> Vec<(String, String, String)> {
        let mut rows: Vec<_> = self
            .scopes
            .get("prefix")
            .into_iter()
            .flatten()
            .filter(|(_, action)| *action != "send_prefix")
            .map(|(key, action)| {
                let text = self.descriptions.get(action).cloned().unwrap_or_else(|| action.replace('_', " "));
                (key.clone(), action.clone(), text)
            })
            .collect();
        rows.sort_by(|a, b| a.2.to_lowercase().cmp(&b.2.to_lowercase()).then_with(|| a.0.cmp(&b.0)));
        rows
    }

    /// The keys after the prefix in their sections, leaving out empty sections.
    pub fn prefix_sections(&self) -> Vec<(&'static str, Vec<(String, String, String)>)> {
        let rows = self.prefix_rows();
        KEY_SECTIONS
            .iter()
            .map(|s| (*s, rows.iter().filter(|r| key_section(&r.1) == *s).cloned().collect::<Vec<_>>()))
            .filter(|(_, rows)| !rows.is_empty())
            .collect()
    }

    pub fn help(&self) -> &[(String, Vec<(String, String)>)] {
        &self.help
    }

    pub fn description(&self, action: &str) -> Option<&str> {
        self.descriptions.get(action).map(String::as_str)
    }

    /// The action this key runs in a scope that this client reads itself.
    pub fn scope_action(&self, scope: &str, key: &str) -> Option<&str> {
        self.lookup(scope, &canonical(key)).map(String::as_str)
    }

    /// The keys the open prefix takes, with what each does, for the which-key popup.
    pub fn open_prefix_rows(&self) -> Vec<(String, String)> {
        let Some(Mode::Prefix(scope)) = &self.mode else { return Vec::new() };
        let mut rows: Vec<_> = self
            .scopes
            .get(scope)
            .into_iter()
            .flatten()
            .filter(|(_, action)| *action != "send_prefix")
            .map(|(key, action)| {
                let text = self.descriptions.get(action).cloned().unwrap_or_else(|| action.replace('_', " "));
                (key.clone(), text)
            })
            .collect();
        rows.sort_by(|a, b| (a.0.chars().count(), a.0.to_lowercase(), &a.0).cmp(&(b.0.chars().count(), b.0.to_lowercase(), &b.0)));
        rows
    }

    /// The key that runs an action in a scope, the shortest if there are several.
    pub fn key_for(&self, scope: &str, action: &str) -> Option<String> {
        let table = self.scopes.get(scope)?;
        table.iter().filter(|(_, a)| *a == action).map(|(k, _)| k.clone()).min_by_key(|k| (k.chars().count(), k.clone()))
    }

    pub fn in_prefix(&self) -> bool {
        matches!(self.mode, Some(Mode::Prefix(_)))
    }

    fn lookup(&self, scope: &str, key: &str) -> Option<&String> {
        self.scopes.get(scope)?.get(key)
    }

    /// `now` is in milliseconds, from any fixed start.
    pub fn press(&mut self, key: &str, now: u64) -> Step {
        let key = canonical(key);
        let mode = self.mode.clone().unwrap_or(Mode::Idle);
        match mode {
            Mode::Repeat(scope, until) => {
                if now <= until {
                    if let Some(action) = self.lookup(&scope, &key).filter(|a| repeats(a)).cloned() {
                        self.mode = Some(Mode::Repeat(scope, now + self.repeat_ms));
                        return Step::Action(action);
                    }
                }
                self.mode = Some(Mode::Idle);
                self.press(&key, now)
            }
            Mode::Prefix(scope) => {
                self.mode = Some(Mode::Idle);
                let Some(action) = self.lookup(&scope, &key).cloned() else {
                    // The leader twice types it, whichever key it is.
                    return if scope == "prefix" && key == self.leader { Step::SendLeader } else { Step::Consumed };
                };
                if action == "send_prefix" {
                    return Step::SendLeader;
                }
                if let Some(next) = self.prefixes.get(&action).cloned() {
                    self.mode = Some(Mode::Prefix(next));
                    return Step::Consumed;
                }
                if repeats(&action) && self.repeat_ms > 0 {
                    self.mode = Some(Mode::Repeat(scope, now + self.repeat_ms));
                }
                Step::Action(action)
            }
            Mode::Idle => {
                if self.mode.is_some() && !self.leader.is_empty() && key == self.leader {
                    self.mode = Some(Mode::Prefix("prefix".to_string()));
                    return Step::Consumed;
                }
                // tuios's GUI looks these up before the leader's table and the pane.
                if let Some(action) = self.lookup("gui", &key) {
                    return Step::Action(action.clone());
                }
                for scope in ["global", "terminal"] {
                    if let Some(action) = self.lookup(scope, &key).filter(|a| direct(a)) {
                        return Step::Action(action.clone());
                    }
                }
                Step::Forward
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map() -> Keymap {
        Keymap::from_event(&json!({
            "leader": "`", "repeat_ms": 500,
            "prefixes": {"prefix_tuios": "prefix.tuios"},
            "bindings": [
                {"scope": "prefix", "key": "`", "action": "send_prefix"},
                {"scope": "prefix", "key": "b", "action": "command:deploy", "command": true},
                {"scope": "prefix", "key": "|", "action": "prefix_split_vertical"},
                {"scope": "prefix", "key": "h", "action": "resize_pane_left_5"},
                {"scope": "prefix", "key": "ctrl+t", "action": "prefix_tuios"},
                {"scope": "prefix.tuios", "key": "x", "action": "some_tuios_action"},
                {"scope": "global", "key": "shift+left", "action": "prev_workspace"},
                {"scope": "global", "key": "ctrl+p", "action": "command_palette"},
                {"scope": "terminal", "key": "alt+left", "action": "terminal_focus_left"},
                {"scope": "terminal", "key": "shift+up", "action": "terminal_scroll_up"},
                {"scope": "terminal", "key": "opt+tab", "action": "terminal_next_window"},
                {"scope": "gui", "key": "ctrl+shift+w", "action": "close_window"},
                {"scope": "gui", "key": "ctrl+shift+alt+e", "action": "gui_split_root_down"},
                {"scope": "gui", "key": "alt+1", "action": "switch_workspace_1"},
                {"scope": "gui", "key": "ctrl+shift+tab", "action": "gui_switcher_back"},
                {"scope": "gui", "key": "ctrl+=", "action": "gui_font_bigger", "description": "Make the text bigger"},
                {"scope": "sidebar", "key": "j", "action": "cursor_down"}
            ],
            "menus": {"": [{"title": "Panes", "rows": [{"key": "z", "description": "Toggle zoom"}]}]}
        }))
    }

    #[test]
    fn the_leader_then_a_key_runs_its_command() {
        let mut m = map();
        assert_eq!(m.press("`", 0), Step::Consumed);
        assert!(m.in_prefix());
        assert_eq!(m.press("b", 1), Step::Action("command:deploy".into()));
        assert!(!m.in_prefix());
        assert_eq!(m.press("b", 2), Step::Forward);
    }

    #[test]
    fn the_leader_twice_types_it() {
        let mut m = map();
        m.press("`", 0);
        assert_eq!(m.press("`", 1), Step::SendLeader);
    }

    #[test]
    fn an_unbound_key_after_the_leader_is_dropped_and_ends_the_prefix() {
        let mut m = map();
        m.press("`", 0);
        assert_eq!(m.press("9", 1), Step::Consumed);
        assert_eq!(m.press("9", 2), Step::Forward);
    }

    #[test]
    fn a_sub_prefix_reads_its_own_table() {
        let mut m = map();
        m.press("`", 0);
        assert_eq!(m.press("ctrl+t", 1), Step::Consumed);
        assert_eq!(m.press("x", 2), Step::Action("some_tuios_action".into()));
    }

    #[test]
    fn a_resize_key_repeats_until_the_window_passes() {
        let mut m = map();
        m.press("`", 0);
        assert_eq!(m.press("h", 10), Step::Action("resize_pane_left_5".into()));
        assert_eq!(m.press("h", 400), Step::Action("resize_pane_left_5".into()));
        assert_eq!(m.press("h", 2000), Step::Forward);
    }

    #[test]
    fn another_key_during_a_repeat_goes_to_the_pane() {
        let mut m = map();
        m.press("`", 0);
        m.press("h", 10);
        assert_eq!(m.press("x", 20), Step::Forward);
    }

    #[test]
    fn only_chords_that_need_nothing_drawn_leave_the_pane() {
        let mut m = map();
        assert_eq!(m.press("shift+left", 0), Step::Action("prev_workspace".into()));
        assert_eq!(m.press("alt+left", 0), Step::Action("terminal_focus_left".into()));
        assert_eq!(m.press("alt+tab", 0), Step::Action("terminal_next_window".into()));
        assert_eq!(m.press("ctrl+p", 0), Step::Action("command_palette".into()));
        // No scrolling action is bound here to this key, so the pane keeps it.
        assert_eq!(m.press("shift+up", 0), Step::Action("terminal_scroll_up".into()));
        assert_eq!(m.press("shift+down", 0), Step::Forward);
    }

    #[test]
    fn the_open_prefix_lists_its_own_keys_and_not_the_leader_itself() {
        let mut m = map();
        assert!(m.open_prefix_rows().is_empty());
        assert_eq!(m.press("`", 0), Step::Consumed);
        let rows = m.open_prefix_rows();
        assert!(rows.iter().any(|(key, _)| key == "b"));
        assert!(!rows.iter().any(|(key, _)| key == "`"));
    }

    #[test]
    fn cmd_arrows_are_left_for_text_and_the_action_keeps_its_ctrl_shift_key() {
        let event = json!({"bindings": [{"scope": "gui", "key": "ctrl+shift+left", "action": "gui_focus_left"}]});
        let map = Keymap::from_event(&event).with_settings(&KeySettings::default());
        assert!(!map.scopes["gui"].contains_key("cmd+left"));
        assert_eq!(map.scopes["gui"].get("ctrl+shift+left").map(String::as_str), Some("gui_focus_left"));
    }

    #[test]
    fn gui_chords_read_ctrl_shift_as_cmd_as_tuios_does_on_macos() {
        let mut m = map();
        assert_eq!(m.press("cmd+w", 0), Step::Action("close_window".into()));
        assert_eq!(m.press("alt+cmd+e", 0), Step::Action("gui_split_root_down".into()));
        assert_eq!(m.press("alt+1", 0), Step::Action("switch_workspace_1".into()));
        assert_eq!(m.press("ctrl+shift+tab", 0), Step::Action("gui_switcher_back".into()));
        assert_eq!(m.press("ctrl+=", 0), Step::Action("gui_font_bigger".into()));
    }

    #[test]
    fn a_scope_this_client_reads_itself_answers_directly_and_help_comes_through() {
        let m = map();
        assert_eq!(m.scope_action("sidebar", "j"), Some("cursor_down"));
        assert_eq!(m.description("gui_font_bigger"), Some("Make the text bigger"));
        assert_eq!(m.help()[0].1[0], ("z".to_string(), "Toggle zoom".to_string()));
    }

    fn tuios_defaults() -> Keymap {
        Keymap::from_event(&json!({
            "leader": "ctrl+b", "repeat_ms": 0,
            "bindings": [
                {"scope": "prefix", "key": "%", "action": "prefix_split_vertical"},
                {"scope": "prefix", "key": "z", "action": "prefix_fullscreen"},
                {"scope": "gui", "key": "ctrl+shift+t", "action": "new_window"},
                {"scope": "gui", "key": "ctrl+shift+=", "action": "equalize_splits"}
            ]
        }))
    }

    fn overrides(toml_text: &str) -> KeySettings {
        crate::settings::Settings::parse(toml_text).unwrap().keys
    }

    #[test]
    fn a_ctrl_prefix_works_and_pressed_twice_types_it() {
        let mut m = tuios_defaults();
        assert_eq!(m.press("ctrl+b", 0), Step::Consumed);
        assert_eq!(m.press("%", 1), Step::Action("prefix_split_vertical".into()));
        m.press("ctrl+b", 2);
        assert_eq!(m.press("ctrl+b", 3), Step::SendLeader);
    }

    #[test]
    fn settings_can_change_the_prefix_in_tessella_only() {
        let mut m = tuios_defaults().with_settings(&overrides("[keys]\nleader = \"ctrl+a\"\n"));
        assert_eq!(m.leader(), "ctrl+a");
        assert_eq!(m.press("ctrl+b", 0), Step::Forward);
        assert_eq!(m.press("ctrl+a", 1), Step::Consumed);
        assert_eq!(m.press("z", 2), Step::Action("prefix_fullscreen".into()));
    }

    #[test]
    fn settings_can_add_change_and_remove_keys() {
        let mut m = tuios_defaults().with_settings(&overrides(
            "[keys.prefix]\nn = \"new_window\"\nz = \"none\"\n[keys.app]\n\"cmd+shift+t\" = \"tessella_settings\"\n\"cmd+n\" = \"none\"\n",
        ));
        m.press("ctrl+b", 0);
        assert_eq!(m.press("n", 1), Step::Action("new_window".into()));
        m.press("ctrl+b", 2);
        assert_eq!(m.press("z", 3), Step::Consumed);
        assert_eq!(m.press("shift+cmd+t", 4), Step::Action("tessella_settings".into()));
        assert_eq!(m.press("cmd+n", 5), Step::Forward);
    }

    #[test]
    fn tessellas_chords_do_not_take_a_key_tuios_already_binds() {
        let mut m = tuios_defaults().with_settings(&KeySettings::default());
        assert_eq!(m.press("cmd+n", 0), Step::Action("tessella_new_pane".into()));
        assert_eq!(m.press("cmd+3", 0), Step::Action("switch_workspace_3".into()));
        assert_eq!(m.press("cmd+-", 0), Step::Action("gui_font_smaller".into()));
        // Zoom wins over tuios's chord on the same key, in each way it can arrive.
        assert_eq!(m.press("cmd+=", 0), Step::Action("gui_font_bigger".into()));
        assert_eq!(m.press("cmd++", 0), Step::Action("gui_font_bigger".into()));
        assert_eq!(m.press("shift+cmd+=", 0), Step::Action("gui_font_bigger".into()));
        assert_eq!(m.press("cmd+t", 0), Step::Action("new_window".into()));
    }

    #[test]
    fn the_settings_page_lists_prefix_keys_by_description_without_the_leader() {
        let m = map();
        let rows = m.prefix_rows();
        assert!(rows.iter().all(|r| r.1 != "send_prefix"));
        assert!(rows.iter().any(|r| r.0 == "b" && r.1 == "command:deploy"));
        assert!(rows.windows(2).all(|w| w[0].2.to_lowercase() <= w[1].2.to_lowercase()));
    }

    #[test]
    fn keys_sort_into_sections_by_what_they_do() {
        for (action, section) in [
            ("prefix_split_vertical", "Panes"),
            ("terminal_focus_left", "Panes"),
            ("resize_pane_left_5", "Panes"),
            ("next_workspace", "Workspaces"),
            ("close_workspace", "Workspaces"),
            ("break_pane", "Workspaces"),
            ("prefix_session_switcher", "Sessions"),
            ("prefix_close_session", "Sessions"),
            ("prefix_inbox", "Agents"),
            ("prefix_next_finished", "Agents"),
            ("prefix_selection", "Copy and paste"),
            ("choose_buffer", "Copy and paste"),
            ("prefix_toggle_sidebar", "Interface"),
            ("prefix_command_palette", "Interface"),
            ("command:run-tests", "Your commands"),
        ] {
            assert_eq!(key_section(action), section, "{action}");
        }
    }

    #[test]
    fn every_prefix_key_lands_in_exactly_one_listed_section() {
        let m = map();
        let total: usize = m.prefix_sections().iter().map(|(_, r)| r.len()).sum();
        assert_eq!(total, m.prefix_rows().len());
    }

    #[test]
    fn with_no_keys_yet_everything_goes_to_the_pane() {
        assert_eq!(Keymap::default().press("`", 0), Step::Forward);
    }

    #[test]
    fn key_names_match_how_tuios_writes_them() {
        assert_eq!(key_name("\\", Some("|"), false, false, true, false), "|");
        assert_eq!(key_name("r", Some("R"), false, false, true, false), "R");
        assert_eq!(key_name("b", Some("b"), false, false, false, false), "b");
        assert_eq!(key_name("t", None, true, false, false, false), "ctrl+t");
        assert_eq!(key_name("left", None, false, false, true, false), "shift+left");
        assert_eq!(key_name("escape", None, false, false, false, false), "esc");
        assert_eq!(key_name("left", None, true, false, true, false), "ctrl+shift+left");
    }

    #[test]
    fn canonical_spelling_covers_tuios_modifier_names() {
        assert_eq!(canonical("opt+esc"), "alt+esc");
        assert_eq!(canonical("shift+super+v"), "shift+cmd+v");
        assert_eq!(canonical("+"), "+");
        assert_eq!(canonical("b"), "b");
    }
}
