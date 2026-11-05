//! The sidebar's file manager: the same actions by key, by menu and by button.

use super::*;
use tessella::files::{self, Clip};

/// One row of a menu.
#[derive(Debug, Clone)]
pub(super) struct MenuRow {
    pub label: &'static str,
    pub key: String,
    pub action: &'static str,
    /// It starts a new group, with a line above it.
    pub group: bool,
    pub danger: bool,
}

/// A menu at the pointer, like any Mac app's.
#[derive(Debug, Clone)]
pub(super) struct ContextMenu {
    pub at: Point<Pixels>,
    pub rows: Vec<MenuRow>,
    pub selected: Option<usize>,
}

/// The rows of a file's menu: label, the action tuios calls it, and whether it
/// starts a group.
const MENU: [(&str, &str, bool); 12] = [
    ("Open", "file_open", false),
    ("Edit in editor", "file_edit", false),
    ("Rename", "file_rename", true),
    ("Copy", "file_copy", false),
    ("Cut", "file_cut", false),
    ("Paste here", "file_paste", false),
    ("Copy path", "file_copy_path", false),
    ("New file or folder", "file_create", true),
    ("Search below this folder", "file_search", false),
    ("Send to a machine", "file_send", false),
    ("Move to Trash", "file_delete", true),
    ("Delete for good", "file_delete_forever", false),
];

impl Tessella {
    /// The key for a file action, from the person's tuios keys.
    fn file_key(&self, action: &str) -> String {
        self.keymap.key_for(FILE_SCOPE, action).unwrap_or_default()
    }

    /// Opens a row's menu at a point. With no row it acts on the folder on show.
    pub(super) fn open_file_menu(&mut self, entry: Option<Entry>, at: Point<Pixels>) {
        let real = entry.as_ref().is_some_and(|e| e.name != "..");
        let folder = entry.as_ref().is_some_and(|e| e.dir);
        let mut rows: Vec<MenuRow> = MENU
            .iter()
            .filter(|(_, action, _)| real || matches!(*action, "file_paste" | "file_create" | "file_search"))
            .filter(|(_, action, _)| *action != "file_paste" || self.file_clip.is_some())
            .filter(|(_, action, _)| *action != "file_edit" || !folder)
            .map(|(label, action, group)| MenuRow {
                label,
                key: self.file_key(action),
                action,
                group: *group,
                danger: action.starts_with("file_delete"),
            })
            .collect();
        // A group with nothing before it has no line to draw.
        if let Some(first) = rows.first_mut() {
            first.group = false;
        }
        self.menu_entry = entry;
        self.overlay = Overlay::Menu(ContextMenu { at, rows, selected: None });
    }

    pub(super) fn menu_choose(&mut self, action: &'static str, cx: &mut Context<Self>) {
        self.overlay = Overlay::None;
        let entry = self.menu_entry.take();
        self.file_action(action, entry, cx);
    }

    pub(super) fn menu_key(&mut self, k: &Keystroke, cx: &mut Context<Self>) {
        let Overlay::Menu(menu) = &mut self.overlay else { return };
        let count = menu.rows.len();
        match k.key.as_str() {
            "escape" => self.overlay = Overlay::None,
            "down" | "j" => menu.selected = Some(menu.selected.map_or(0, |i| (i + 1) % count.max(1))),
            "up" | "k" => menu.selected = Some(menu.selected.map_or(count.saturating_sub(1), |i| (i + count - 1) % count.max(1))),
            "enter" => {
                if let Some(action) = menu.selected.and_then(|i| menu.rows.get(i)).map(|r| r.action) {
                    self.menu_choose(action, cx);
                }
            }
            _ => {}
        }
    }

    /// Runs a file action on a row, or on the folder on show.
    pub(super) fn file_action(&mut self, action: &str, entry: Option<Entry>, cx: &mut Context<Self>) {
        let entry = entry.filter(|e| e.name != "..");
        let dir = self.files_shown.clone();
        match (action, entry, dir) {
            ("file_create", _, Some(dir)) => {
                self.prompt(PromptKind::NewFile(dir), "New file, or a folder with a trailing /", String::new());
            }
            ("file_paste", _, Some(dir)) => self.paste_into(dir),
            ("file_search", _, Some(dir)) => self.open_file_search(dir),
            ("file_edit", Some(e), _) if !e.dir => self.edit_file(e.path),
            ("file_open" | "file_edit", Some(e), _) => self.activate_row(Row::File(e), cx),
            ("file_rename", Some(e), _) => {
                let title = format!("Rename {}", e.name);
                self.prompt(PromptKind::RenameFile(e.path), &title, e.name);
            }
            ("file_copy" | "file_cut", Some(e), _) => {
                let cut = action == "file_cut";
                self.status = Some(format!("{} {}. Paste it in another folder.", if cut { "Cut" } else { "Copied" }, e.name));
                self.file_clip = Some(Clip { path: e.path, cut });
            }
            ("file_copy_path", Some(e), _) => {
                self.write_clipboard(e.path.to_string_lossy().into_owned(), cx);
                self.status = Some(format!("Copied the path of {}.", e.name));
            }
            ("file_delete", Some(e), _) => {
                let title = format!("Move {} to the Trash?", e.name);
                self.prompt(PromptKind::Confirm(Confirm::Trash(e.path)), &title, String::new());
            }
            ("file_delete_forever", Some(e), _) => {
                let title = format!("Delete {} for good? This cannot be undone.", e.name);
                self.prompt(PromptKind::Confirm(Confirm::DeleteForever(e.path)), &title, String::new());
            }
            ("file_send", Some(e), _) => self.open_send_to(e.path),
            _ => {}
        }
    }

    pub(super) fn files_changed(&mut self, result: std::io::Result<String>) {
        match result {
            Ok(message) => self.status = Some(message),
            Err(err) => self.status = Some(err.to_string()),
        }
        self.files_read = None;
    }

    fn paste_into(&mut self, dir: PathBuf) {
        let Some(clip) = self.file_clip.clone() else {
            self.status = Some("Nothing is copied yet.".into());
            return;
        };
        let result = files::paste(&clip, &dir);
        if result.is_ok() && clip.cut {
            self.file_clip = None;
        }
        self.files_changed(result.map(|to| format!("Pasted {}.", to.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()))));
    }

    fn open_file_search(&mut self, dir: PathBuf) {
        let items = files::walk(&dir).into_iter().map(|rel| {
            let path = dir.join(&rel).to_string_lossy().into_owned();
            Item::new(rel, "", path)
        });
        self.overlay = Overlay::Chooser(ChooserKind::FileSearch, Picker::new(items.collect(), String::new()));
    }

    fn open_send_to(&mut self, path: PathBuf) {
        let items: Vec<Item> = self
            .machines
            .iter()
            .filter(|m| m.ready)
            .filter_map(|m| m.host.clone())
            .map(|host| Item::new(host.clone(), "", host))
            .collect();
        if items.is_empty() {
            self.status = Some("No other machine is connected.".into());
            return;
        }
        self.menu_entry = Some(Entry { name: String::new(), dir: path.is_dir(), path });
        self.overlay = Overlay::Chooser(ChooserKind::SendTo, Picker::new(items, String::new()));
    }

    /// Copies a path to a host's receive folder, with tuios's own copy.
    pub(super) fn send_to(&mut self, path: PathBuf, host: String, cx: &mut Context<Self>) {
        let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        self.status = Some(format!("Sending {name} to {host}…"));
        let bin = std::env::var("TESSELLA_TUIOS_BIN").unwrap_or_else(|_| "tuios".to_string());
        cx.spawn(async move |this, cx| {
            let dest = format!("{host}:");
            let run = cx
                .background_executor()
                .spawn(async move { std::process::Command::new(bin).arg("cp").arg(&path).arg(&dest).stdin(std::process::Stdio::null()).output() })
                .await;
            let message = match run {
                Ok(out) if out.status.success() => format!("Sent {name} to {host}."),
                Ok(out) => String::from_utf8_lossy(&out.stderr).trim().to_string(),
                Err(err) => err.to_string(),
            };
            let _ = this.update(cx, |this, cx| {
                this.status = Some(message);
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens what a search found: a file in a pane, a folder in the sidebar.
    pub(super) fn open_found(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let dir = path.is_dir();
        let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
        self.activate_row(Row::File(Entry { path, name, dir }), cx);
    }
}
