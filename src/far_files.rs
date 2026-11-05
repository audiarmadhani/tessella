//! Files on another machine, through the daemon there. Its file verbs answer later,
//! so every request says what its answer is for, and each machine's own policy
//! decides what this client may do there.

use super::*;
use tessella::files::{self, Clip, Place};

/// How often the folder on show is read again. Each read crosses the link.
const FOLDER_EVERY: Duration = Duration::from_secs(5);
/// How long a read may stay unanswered before another one is sent.
const FOLDER_PATIENCE: Duration = Duration::from_secs(15);

/// What an answer from another machine's file verbs is for.
#[derive(Debug, Clone)]
pub(super) enum FileReply {
    /// A folder's rows. A newer request or a change to the files makes it stale.
    Listing { asked: u64, want: Place },
    /// A change to the files, and what to say when it worked.
    Changed { message: String, clears_cut: bool },
    Search { folder: Place },
    /// When a Markdown file last changed, for a pane.
    DocStamp { window: String, place: Place },
    DocText { window: String, stamp: i64 },
}

/// A failed search in words that mean something to the person using it.
fn plain_search_error(error: &str) -> String {
    if error.contains("larger folder") { "This folder is too large to search. Search a folder inside it.".to_string() } else { files::plain_error(error) }
}

impl Tessella {
    /// The machine a pane's files are on, empty for this one.
    pub(super) fn files_host_of(&self, win: &Win) -> String {
        // A Markdown pane runs here whatever machine its file is on.
        if let Some(doc) = win.markdown_place().filter(|doc| !doc.is_here()) {
            return doc.host;
        }
        if win.host.is_empty() { self.target_host.clone().unwrap_or_default() } else { win.host.clone() }
    }

    fn ask_far(&mut self, host: &str, verb: &str, params: Value, reply: FileReply) {
        self.next_req += 1;
        self.replies.insert(self.next_req, Reply::Files(reply));
        self.send(json!({"cmd": "verb", "verb": verb, "req": self.next_req, "host": host, "params": params}));
    }

    /// Keeps the rows on a folder on another machine.
    pub(super) fn refresh_far_files(&mut self, want: Place) {
        if self.files_want.as_ref() != Some(&want) {
            // The rows are another machine's now, so none may stay to be acted on.
            self.files.clear();
            self.files_shown = None;
            self.files_read = None;
            self.files_error = None;
            self.files_want = Some(want.clone());
            self.forget_far_request();
        }
        let fresh = self.files_read.is_some_and(|t| t.elapsed() < FOLDER_EVERY);
        let waiting = self.files_asked.is_some_and(|t| t.elapsed() < FOLDER_PATIENCE);
        if fresh || waiting {
            return;
        }
        self.files_asked = Some(Instant::now());
        let dir = want.path.to_string_lossy().into_owned();
        let host = want.host.clone();
        self.ask_far(&host, "file-list", json!({"dir": dir, "limit": 5000}), FileReply::Listing { asked: self.files_request, want });
    }

    /// Makes any answer on its way stale, and lets the next read go at once.
    pub(super) fn forget_far_request(&mut self) {
        self.files_request += 1;
        self.files_asked = None;
    }

    pub(super) fn file_answered(&mut self, reply: FileReply, result: &Value) {
        match reply {
            FileReply::Listing { asked, want } => {
                if asked != self.files_request {
                    return;
                }
                let (dir, rows, more) = rail::listing_rows(&want.host, result);
                if more && self.files_shown.as_ref().map(|p| &p.path) != Some(&dir) {
                    self.status = Some(format!("Showing the first {} files of this folder.", rows.len()));
                }
                self.files = rows;
                self.files_shown = Some(Place::new(&want.host, dir));
                self.files_read = Some(Instant::now());
                self.files_asked = None;
                self.files_error = None;
            }
            FileReply::Changed { message, clears_cut } => {
                if clears_cut {
                    self.file_clip = None;
                }
                self.files_changed(Ok(message));
            }
            FileReply::Search { folder } => self.show_far_search(folder, result),
            FileReply::DocStamp { window, place } => self.doc_stamp_answered(window, place, result),
            FileReply::DocText { window, stamp } => self.doc_text_answered(&window, stamp, result),
        }
    }

    pub(super) fn file_failed(&mut self, reply: FileReply, error: &str, code: &str) {
        match reply {
            FileReply::Listing { asked, want } => {
                if asked != self.files_request {
                    return;
                }
                self.files_read = Some(Instant::now());
                self.files_asked = None;
                // A pane on another machine can report a folder from this one, which
                // need not exist there. Its home folder is somewhere to start.
                if code == "no_file" && self.files_dir.is_none() {
                    self.files_dir = Some(PathBuf::from("~"));
                    self.files_read = None;
                    return;
                }
                // A link that drops for a moment keeps the rows on show. A folder that
                // never opened keeps a way back up.
                if self.files_shown.as_ref() != Some(&want) {
                    self.files = rail::rows_above(&want.host, &want.path);
                    self.files_shown = Some(want);
                }
                // It is read again every few seconds, and says so once.
                if self.files_error.as_deref() != Some(error) {
                    self.files_error = Some(error.to_string());
                    self.status = Some(files::plain_error(error));
                }
            }
            FileReply::Changed { .. } => self.files_changed(Err(std::io::Error::other(error.to_string()))),
            FileReply::Search { .. } => self.status = Some(plain_search_error(error)),
            FileReply::DocStamp { window, .. } | FileReply::DocText { window, .. } => self.doc_failed(&window, error),
        }
    }

    pub(super) fn far_new(&mut self, dir: &Place, name: &str, cx: &mut Context<Self>) {
        let path = match files::inside(&dir.path, name) {
            Ok(path) => path,
            Err(err) => return self.files_changed(Err(err)),
        };
        let shown = path.display().to_string();
        if name.trim().ends_with('/') {
            let message = format!("Made {shown}.");
            return self.ask_far(&dir.host, "file-mkdir", json!({"path": shown}), FileReply::Changed { message, clears_cut: false });
        }
        // The file verbs have no way to make a file, and a copy of an empty one does.
        let target = Place::new(&dir.host, &path);
        let empty = std::env::temp_dir().join(format!("tessella-new-{}-{}", std::process::id(), self.next_req));
        let source = empty.join(target.name());
        let made = std::fs::create_dir_all(&empty).and_then(|()| std::fs::write(&source, ""));
        if let Err(err) = made {
            return self.files_changed(Err(err));
        }
        let parent = Place::new(&dir.host, path.parent().unwrap_or(&dir.path));
        let args = vec!["--conflict".into(), "skip".into(), Place::new("", &source).spec(), parent.folder_spec()];
        self.run_cp(args, format!("Made {shown}."), false, Some(empty), cx);
    }

    pub(super) fn far_rename(&mut self, from: &Place, name: &str) {
        let Some(dir) = from.path.parent() else { return };
        let to = match files::inside(dir, name) {
            Ok(to) => to,
            Err(err) => return self.files_changed(Err(err)),
        };
        let message = format!("Renamed to {}.", Place::new("", &to).name());
        let params = json!({"from": from.path.to_string_lossy(), "to": to.to_string_lossy()});
        self.ask_far(&from.host, "file-rename", params, FileReply::Changed { message, clears_cut: false });
    }

    pub(super) fn far_delete(&mut self, what: &Place) {
        let message = format!("Deleted {}.", what.name());
        let params = json!({"path": what.path.to_string_lossy(), "recursive": true});
        self.ask_far(&what.host, "file-remove", params, FileReply::Changed { message, clears_cut: false });
    }

    /// Pastes when either end is on another machine. A copy never replaces a file
    /// that differs: it keeps both.
    pub(super) fn far_paste(&mut self, clip: Clip, dir: Place, cx: &mut Context<Self>) {
        let name = clip.from.name();
        let same_machine = clip.from.host == dir.host;
        let same_folder = same_machine && clip.from.path.parent() == Some(dir.path.as_path());
        if same_folder && clip.cut {
            return;
        }
        if same_machine && dir.path.starts_with(&clip.from.path) {
            self.status = Some("A folder cannot go inside itself.".into());
            return;
        }
        if same_machine && clip.cut {
            let to = dir.path.join(&name);
            let params = json!({"from": clip.from.path.to_string_lossy(), "to": to.to_string_lossy()});
            let message = format!("Pasted {name}.");
            return self.ask_far(&dir.host, "file-rename", params, FileReply::Changed { message, clears_cut: true });
        }
        let mut args: Vec<String> = vec!["--conflict".into(), "keep-both".into()];
        if clip.cut {
            args.push("-m".into());
        }
        args.push(clip.from.spec());
        // A copy with the same bytes does nothing, so a copy in its own folder names itself.
        args.push(if same_folder {
            let free = files::free_among(&name, |taken| self.files.iter().any(|row| row.name == taken));
            Place::new(&dir.host, dir.path.join(free)).spec()
        } else {
            dir.folder_spec()
        });
        self.status = Some(format!("Pasting {name}…"));
        self.run_cp(args, format!("Pasted {name}."), clip.cut, None, cx);
    }

    /// Runs `tuios cp` beside the window. Its message or error goes in the status line.
    pub(super) fn run_cp(&mut self, args: Vec<String>, done: String, clears_cut: bool, scratch: Option<PathBuf>, cx: &mut Context<Self>) {
        let bin = std::env::var("TESSELLA_TUIOS_BIN").unwrap_or_else(|_| "tuios".to_string());
        cx.spawn(async move |this, cx| {
            let run = cx
                .background_executor()
                .spawn(async move {
                    let out = std::process::Command::new(bin).arg("cp").args(&args).stdin(std::process::Stdio::null()).output();
                    if let Some(scratch) = scratch {
                        let _ = std::fs::remove_dir_all(scratch);
                    }
                    out
                })
                .await;
            let result = match run {
                Ok(out) if out.status.success() => Ok(done),
                Ok(out) => {
                    let said = String::from_utf8_lossy(&out.stderr);
                    let said = said.trim();
                    Err(std::io::Error::other(if said.is_empty() { "Nothing was copied.".to_string() } else { said.to_string() }))
                }
                Err(err) => Err(err),
            };
            let _ = this.update(cx, |this, cx| {
                if clears_cut && result.is_ok() {
                    this.file_clip = None;
                }
                this.files_changed(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn far_search(&mut self, folder: Place) {
        self.status = Some(format!("Reading {} on {}…", folder.name(), folder.host));
        let host = folder.host.clone();
        let path = folder.path.to_string_lossy().into_owned();
        self.ask_far(&host, "file-walk", json!({"path": path}), FileReply::Search { folder });
    }

    fn show_far_search(&mut self, folder: Place, result: &Value) {
        self.status = None;
        // Someone may have moved on while the folder was read.
        if !matches!(self.overlay, Overlay::None) {
            return;
        }
        let items = files::walk_answer(result).into_iter().map(|(relative, dir)| {
            let path = folder.path.join(&relative).to_string_lossy().into_owned();
            // The chooser holds text, and a far folder cannot be checked later.
            Item::new(relative, "", if dir { path + "/" } else { path })
        });
        self.search_host = folder.host.clone();
        self.overlay = Overlay::Chooser(ChooserKind::FileSearch, Picker::new(items.collect(), String::new()));
    }

    /// Asks when a far Markdown file last changed.
    pub(super) fn ask_doc_stamp(&mut self, window: String, place: Place) {
        let host = place.host.clone();
        let path = place.path.to_string_lossy().into_owned();
        self.ask_far(&host, "file-stat", json!({"path": path}), FileReply::DocStamp { window, place });
    }

    /// Reads a far Markdown file. tuios answers with up to 4 MB in one reply.
    pub(super) fn ask_doc_text(&mut self, window: String, place: &Place, stamp: i64) {
        let path = place.path.to_string_lossy().into_owned();
        self.ask_far(&place.host, "file-read", json!({"path": path}), FileReply::DocText { window, stamp });
    }
}
