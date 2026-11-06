//! Cmd-click on a link or a file path in a pane. A file opens on the pane's machine.

use super::*;
use gpui::{Modifiers, ModifiersChangedEvent};
use tessella::files::Place;
use tessella::links::Link;

/// The link under the pointer while Cmd is held.
pub(super) struct LinkHover {
    pub pty: String,
    pub cells: Vec<(usize, usize, usize)>,
    pub target: String,
}

impl Tessella {
    /// Marks the link under a point, if Cmd is held and a pane's text is there.
    pub(super) fn hover_link(&mut self, position: Point<Pixels>, modifiers: Modifiers, window: &Window, cx: &mut Context<Self>) -> bool {
        let found = modifiers.platform.then(|| self.window_under(position, window).cloned()).flatten().and_then(|win| {
            if win.markdown_place().is_some() {
                return None;
            }
            let (col, row, _, past) = self.cell_at(&win, position, window);
            let found = (past == 0).then(|| self.panes.get(&win.pty)?.link_under(col, row)).flatten()?;
            Some(LinkHover { pty: win.pty, cells: found.cells, target: found.link.target() })
        });
        let on_link = found.is_some();
        if on_link || self.link_hover.is_some() {
            self.link_hover = found;
            cx.notify();
        }
        on_link
    }

    /// Cmd pressed or let go with the pointer still.
    pub(super) fn on_modifiers(&mut self, event: &ModifiersChangedEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(position) = self.pointer {
            let on_link = self.hover_link(position, event.modifiers, window, cx);
            let cursor = if on_link { gpui::CursorStyle::PointingHand } else { gpui::CursorStyle::Arrow };
            if cursor != self.stage_cursor {
                self.stage_cursor = cursor;
                cx.notify();
            }
        }
    }

    pub(super) fn open_link(&mut self, win: &Win, link: Link, cx: &mut Context<Self>) {
        match link {
            Link::Url(url) => {
                // Tests must not open the person's browser.
                if std::env::var_os("TESSELLA_TEST_INPUT").is_some() {
                    eprintln!("tessella: would open {url}");
                } else {
                    cx.open_url(&url);
                }
            }
            Link::File { path, line } => self.open_file_link(win, &path, line),
        }
    }

    fn open_file_link(&mut self, win: &Win, path: &str, line: Option<u32>) {
        let host = self.files_host_of(win);
        // The pane's own folder is on the pane's machine, and `~` is that machine's home.
        let target = if path.starts_with('/') || (!host.is_empty() && path.starts_with('~')) {
            PathBuf::from(path)
        } else if let Some(rest) = path.strip_prefix("~/").filter(|_| host.is_empty()) {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest)
        } else if win.cwd.is_empty() {
            PathBuf::from(if host.is_empty() { "." } else { "~" }).join(path)
        } else {
            PathBuf::from(&win.cwd).join(path)
        };
        let place = Place::new(&host, target);
        if !place.is_here() {
            return self.ask_link_stat(place, line);
        }
        match std::fs::metadata(&place.path) {
            Ok(meta) => self.open_link_place(place, meta.is_dir(), line),
            Err(_) => self.status = Some(format!("No file at {}.", place.path.display())),
        }
    }

    /// Opens a file or folder a link was found to name.
    pub(super) fn open_link_place(&mut self, place: Place, dir: bool, line: Option<u32>) {
        if dir {
            self.files_dir = Some(place.path);
            self.files_read = None;
        } else if tessella::markdown::is_markdown(&place.path) {
            self.open_file(place);
        } else {
            self.edit_file_at(place, line);
        }
    }
}
