//! The Work tab: git worktrees, one session each, and a prompt fanned out across
//! several. Also opens a pane's changes in Review, turn by turn.

use super::agents_view::{AgentsPage, AgentsReply, AgentsTab, Review, Write};
use super::*;
use tessella::agents::{self, Worktree};

/// How often an open review is read again, since an agent may be changing the files.
const REVIEW_EVERY: Duration = Duration::from_secs(4);
/// The same for a far pane, where each read is an ssh connection.
const FAR_REVIEW_EVERY: Duration = Duration::from_secs(12);

/// A read-only look at a far folder's changes. A repository's config can name programs
/// for git to run, so those are turned off, and the size is capped.
fn far_diff_script(folder: &str) -> String {
    let git = "git -c core.fsmonitor=false -c core.hooksPath=/dev/null";
    // ssh hands this to the login shell, which can be any shell, so sh runs it.
    let script = format!(
        "cd {folder} && root=$(git rev-parse --show-toplevel) && cd \"$root\" && {{ {git} diff --no-color --no-ext-diff --no-textconv HEAD -- ; printf '\\n{mark}\\n'; {git} ls-files --others --exclude-standard; }} | head -c 8000000",
        folder = tessella::files::shell_quote(folder),
        mark = agents::UNTRACKED_MARK,
    );
    format!("sh -c {}", tessella::files::shell_quote(&script))
}

/// Names a review's pane. A worktree is known by its session, and tuios takes its
/// focused pane.
fn pane_params(review: &Review) -> Value {
    let mut params = json!({"session": review.session});
    if !review.window.is_empty() {
        params["window"] = json!(review.window);
    }
    params
}

impl Tessella {
    fn agents_page(&mut self) -> Option<&mut AgentsPage> {
        if let Overlay::Agents(page) = &mut self.overlay { Some(page) } else { None }
    }

    fn ask_review(&mut self, host: &str, verb: &str, params: Value, reply: AgentsReply) {
        self.next_req += 1;
        self.replies.insert(self.next_req, Reply::Agents(reply));
        let mut command = json!({"cmd": "verb", "verb": verb, "req": self.next_req, "params": params});
        if !host.is_empty() {
            command["host"] = json!(host);
        }
        self.send(command);
    }

    // ----- changes -----

    /// Opens what the focused pane's agent changed.
    pub(super) fn open_changes(&mut self) {
        let Some(win) = self.focused_win().cloned() else { return };
        self.open_agents(AgentsTab::Review);
        // tuios keeps a pane it runs on another machine out of its review.
        if !win.host.is_empty() && !win.cwd.is_empty() {
            let addr = self.machines.iter().find(|m| m.host.as_deref() == Some(win.host.as_str())).map_or(win.host.clone(), |m| m.addr.clone());
            if let Some(page) = self.agents_page() {
                page.tab = AgentsTab::Review;
                page.review = Some(Review { name: win.label(), over_ssh: Some((addr, win.cwd.clone())), ..Default::default() });
            }
            return self.load_review();
        }
        let session = self.state.session.clone();
        self.open_review_on(String::new(), session, win.id.clone(), win.label());
    }

    pub(super) fn open_review_on(&mut self, host: String, session: String, window: String, name: String) {
        if let Some(page) = self.agents_page() {
            page.tab = AgentsTab::Review;
            page.scroll = 0;
            page.review = Some(Review { host, session, window, name, ..Default::default() });
        }
        self.load_review();
        self.load_turns();
    }

    /// Reads the changes on show: everything, or one turn.
    pub(super) fn load_review(&mut self) {
        let Some(review) = self.agents_page().and_then(|p| p.review.as_mut()) else { return };
        review.loaded = Some(Instant::now());
        if let Some((addr, folder)) = review.over_ssh.clone() {
            return self.read_diff_over_ssh(addr, folder);
        }
        let (host, turn) = (review.host.clone(), review.turn);
        let mut params = pane_params(review);
        match turn {
            Some(n) => {
                params["n"] = json!(n);
                self.ask_review(&host, "checkpoint-diff", params, AgentsReply::Diff { turn });
            }
            None => self.ask_review(&host, "review-diff", params, AgentsReply::Diff { turn }),
        }
    }

    fn load_turns(&mut self) {
        let Some(review) = self.agents_page().and_then(|p| p.review.as_ref()).filter(|r| r.over_ssh.is_none()) else { return };
        let host = review.host.clone();
        let params = pane_params(review);
        self.ask_review(&host, "list-checkpoints", params, AgentsReply::Turns);
    }

    /// Reads a far folder's changes, beside the window, into `far_diff`.
    fn read_diff_over_ssh(&mut self, addr: String, folder: String) {
        if std::mem::replace(&mut self.far_diff_running, true) {
            return;
        }
        let slot = self.far_diff.clone();
        std::thread::spawn(move || {
            let read = std::process::Command::new("ssh")
                .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-T", &addr, &far_diff_script(&folder)])
                .stdin(std::process::Stdio::null())
                .output();
            let answer = match read {
                Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
                Ok(out) => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
                Err(err) => Err(err.to_string()),
            };
            if let Ok(mut slot) = slot.lock() {
                *slot = Some(answer);
            }
        });
    }

    /// Takes in a far read, if one has finished.
    fn take_far_diff(&mut self) {
        let Some(answer) = self.far_diff.lock().ok().and_then(|mut slot| slot.take()) else { return };
        self.far_diff_running = false;
        let Some(review) = self.agents_page().and_then(|p| p.review.as_mut()).filter(|r| r.over_ssh.is_some()) else { return };
        match answer {
            Ok(text) => {
                review.problem = None;
                review.files = agents::parse_unified(&text);
                review.file = review.file.min(review.files.len().saturating_sub(1));
                let lines = review.files.get(review.file).map_or(0, |f| f.lines.len());
                review.line = review.line.min(lines.saturating_sub(1));
            }
            Err(error) => {
                review.files.clear();
                review.problem = Some(tessella::files::plain_error(&error));
            }
        }
    }

    /// Keeps an open review current while an agent works. An idle window draws nothing,
    /// so a wake is always waiting while one is on show.
    pub(super) fn refresh_review(&mut self, cx: &mut Context<Self>) {
        self.take_far_diff();
        let Overlay::Agents(page) = &self.overlay else { return };
        if page.tab != AgentsTab::Review || page.write.is_some() {
            return;
        }
        // One turn never changes, but everything an agent has done does.
        let Some(review) = page.review.as_ref().filter(|r| r.turn.is_none()) else { return };
        let far = review.over_ssh.is_some();
        let every = if far { FAR_REVIEW_EVERY } else { REVIEW_EVERY };
        if review.loaded.is_none_or(|t| t.elapsed() >= every) {
            self.load_review();
            self.load_turns();
        }
        // A far read finishes on its own, so look for it sooner than the next read.
        let wake = if far && self.far_diff_running { Duration::from_millis(250) } else { REVIEW_EVERY };
        if self.review_wake.is_none_or(|t| t.elapsed() >= wake) {
            self.review_wake = Some(Instant::now());
            Self::wake_after(wake + Duration::from_millis(20), cx);
        }
    }

    pub(super) fn turns_answered(&mut self, result: &Value) {
        let turns = agents::parse_turns(result);
        if let Some(review) = self.agents_page().and_then(|p| p.review.as_mut()) {
            review.turns = turns;
        }
    }

    pub(super) fn review_failed(&mut self, error: &str) {
        let message = tessella::files::plain_error(error);
        if let Some(review) = self.agents_page().and_then(|p| p.review.as_mut()) {
            review.files.clear();
            review.problem = Some(message);
        }
    }

    /// Shows one turn, or with None everything the agent changed.
    pub(super) fn show_turn(&mut self, turn: Option<i64>) {
        let Some(review) = self.agents_page().and_then(|p| p.review.as_mut()) else { return };
        review.turn = turn;
        review.file = 0;
        review.line = 0;
        self.load_review();
    }

    /// Moves to the turn before or after the one on show. Turns are newest first.
    pub(super) fn step_turn(&mut self, older: bool) {
        let Some(review) = self.agents_page().and_then(|p| p.review.as_ref()) else { return };
        let turn = agents::next_turn(&review.turns, review.turn, older);
        self.show_turn(turn);
    }

    /// Asks to put the files back to the turn on show.
    pub(super) fn ask_restore(&mut self) {
        let Some(review) = self.agents_page().and_then(|p| p.review.as_ref()) else { return };
        let Some(n) = review.turn else {
            self.status = Some("Choose a turn to go back to first.".into());
            return;
        };
        let what = Confirm::RestoreCheckpoint { host: review.host.clone(), session: review.session.clone(), window: review.window.clone(), n };
        self.prompt(PromptKind::Confirm(what), &format!("Put the files back to turn {n}? tuios saves them first, so this can be undone."), String::new());
    }

    pub(super) fn restore_turn(&mut self, host: &str, session: &str, window: &str, n: i64) {
        let params = json!({"session": session, "window": window, "n": n});
        self.ask_review(host, "restore-checkpoint", params, AgentsReply::Done(format!("Put the files back to turn {n}.")));
    }

    // ----- worktrees -----

    /// The worktrees here and on each connected machine.
    pub(super) fn load_worktrees(&mut self) {
        self.ask_review("", "list-worktrees", json!({"changes": true}), AgentsReply::Worktrees(String::new()));
        let hosts: Vec<String> = self.machines.iter().filter(|m| m.ready).filter_map(|m| m.host.clone()).collect();
        for host in hosts {
            self.ask_review(&host, "list-worktrees", json!({"changes": true}), AgentsReply::Worktrees(host.clone()));
        }
    }

    pub(super) fn worktrees_answered(&mut self, host: &str, result: &Value) {
        let rows = agents::parse_worktrees(host, result);
        if let Some(page) = self.agents_page() {
            page.worktrees.retain(|w| w.host != host);
            page.worktrees.extend(rows);
            page.worktrees.sort_by(|a, b| (&a.host, &a.repo, &a.branch).cmp(&(&b.host, &b.repo, &b.branch)));
        }
    }

    fn current_worktree(&mut self) -> Option<Worktree> {
        let page = self.agents_page()?;
        page.worktrees.get(page.cursor.min(page.worktrees.len().saturating_sub(1))).cloned()
    }

    /// The folder and machine of the focused pane, which new work starts from.
    fn work_origin(&self) -> Option<(String, String)> {
        let win = self.focused_win()?;
        let dir = if win.cwd.is_empty() { None } else { Some(win.cwd.clone()) };
        Some((self.files_host_of(win), dir?))
    }

    /// Starts asking for a new worktree's branch, or a prompt to fan out.
    pub(super) fn start_work(&mut self, fan: bool) {
        let Some((host, repo)) = self.work_origin() else {
            self.status = Some("Focus a pane in a git repository first.".into());
            return;
        };
        if self.agents_page().is_none() {
            self.open_agents(AgentsTab::Work);
        }
        let write = if fan { Write::FanPrompt { host, repo } } else { Write::NewWorktree { host, repo } };
        if let Some(page) = self.agents_page() {
            page.tab = AgentsTab::Work;
            page.write = Some((write, String::new()));
        }
    }

    pub(super) fn make_worktree(&mut self, host: &str, repo: &str, branch: &str) {
        let params = json!({"repo": repo, "branch": branch});
        self.status = Some(format!("Making a worktree for {branch}…"));
        self.ask_review(host, "new-worktree", params, AgentsReply::WorktreeMade(host.to_string()));
    }

    /// The prompt is written. Next: how many, and which agents.
    pub(super) fn ask_fan_how(&mut self, host: String, repo: String, prompt: String) {
        if let Some(page) = self.agents_page() {
            page.write = Some((Write::FanHow { host, repo, prompt }, "3 claude".to_string()));
        }
    }

    pub(super) fn start_fan(&mut self, host: &str, repo: &str, prompt: &str, how: &str) {
        let Some((count, names)) = agents::parse_fan(how) else {
            self.status = Some("Write how many and which agents, like: 3 claude,codex".into());
            return;
        };
        self.status = Some(format!("Starting {count} agents…"));
        let params = json!({"count": count, "agents": names, "prompt": prompt, "repo": repo});
        self.ask_review(host, "fan", params, AgentsReply::Fanned(count));
    }

    pub(super) fn work_answered(&mut self, reply: AgentsReply, result: &Value) {
        match reply {
            AgentsReply::WorktreeMade(host) => {
                let session = result["session"].as_str().unwrap_or_default().to_string();
                self.status = Some(format!("Made {session}."));
                if host.is_empty() && !session.is_empty() {
                    self.overlay = Overlay::None;
                    self.switch_session(&session);
                } else {
                    self.load_worktrees();
                }
            }
            AgentsReply::Fanned(count) => {
                self.status = Some(format!("Started {count} agents. Each works in its own worktree."));
                self.load_worktrees();
            }
            _ => {}
        }
    }

    pub(super) fn work_key(&mut self, name: &str) {
        let Some(page) = self.agents_page() else { return };
        let count = page.worktrees.len();
        match name {
            "esc" | "escape" | "q" => self.overlay = Overlay::None,
            "j" | "down" => page.cursor = (page.cursor + 1).min(count.saturating_sub(1)),
            "k" | "up" => page.cursor = page.cursor.saturating_sub(1),
            "n" => self.start_work(false),
            "f" => self.start_work(true),
            "v" => self.review_worktree(),
            "enter" => self.go_to_worktree(),
            "D" => self.ask_remove_worktree(),
            _ => {}
        }
    }

    pub(super) fn review_worktree(&mut self) {
        let Some(w) = self.current_worktree() else { return };
        self.open_review_on(w.host, w.session.clone(), String::new(), w.session);
    }

    pub(super) fn go_to_worktree(&mut self) {
        let Some(w) = self.current_worktree() else { return };
        if w.host.is_empty() {
            self.overlay = Overlay::None;
            self.switch_session(&w.session);
        } else {
            self.status = Some(format!("{} is on {}. Open it with a pane on that machine.", w.session, w.host));
        }
    }

    pub(super) fn ask_remove_worktree(&mut self) {
        let Some(w) = self.current_worktree() else { return };
        let title = format!("Remove the worktree {}? tuios refuses if it has uncommitted changes.", w.session);
        self.prompt(PromptKind::Confirm(Confirm::RemoveWorktree { host: w.host, session: w.session }), &title, String::new());
    }

    pub(super) fn remove_worktree(&mut self, host: &str, session: &str) {
        self.ask_review(host, "remove-worktree", json!({"session": session}), AgentsReply::Done(format!("Removed {session}.")));
    }

    pub(super) fn work_view(&self, page: &AgentsPage, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = &self.theme;
        let line = theme::mix(t.palette.bg, t.text, 0.14);
        let selected = page.cursor.min(page.worktrees.len().saturating_sub(1));
        let rows: Vec<_> = page
            .worktrees
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let on = i == selected;
                let mut details = vec![w.repo.clone()];
                if !w.host.is_empty() {
                    details.push(w.host.clone());
                }
                if !w.group.is_empty() {
                    details.push(format!("fan {}", w.group));
                }
                if let Some(n) = w.uncommitted.filter(|n| *n > 0) {
                    details.push(format!("{n} uncommitted"));
                }
                if let Some(n) = w.ahead.filter(|n| *n > 0) {
                    details.push(format!("{n} ahead"));
                }
                if w.merged {
                    details.push("merged".to_string());
                }
                let dot = t.agent(&w.state).unwrap_or(t.faint);
                div()
                    .flex()
                    .items_center()
                    .gap(px(9.0))
                    .px(px(14.0))
                    .py(px(6.0))
                    .cursor_pointer()
                    .when(on, |d| d.bg(rgb(t.selected)))
                    .child(div().size(px(7.0)).flex_none().rounded(px(3.5)).bg(rgb(dot)))
                    .child(div().flex_none().text_size(px(12.5)).text_color(rgb(t.text)).child(SharedString::from(if w.branch.is_empty() { w.session.clone() } else { w.branch.clone() })))
                    .child(div().flex_1().min_w_0().truncate().text_size(px(11.5)).text_color(rgb(t.muted)).child(SharedString::from(details.join("  ·  "))))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            if let Some(page) = this.agents_page() {
                                page.cursor = i;
                            }
                            cx.notify();
                        }),
                    )
            })
            .collect();
        let action = |label: &str, run: fn(&mut Self), cx: &mut Context<Self>| {
            div()
                .px(px(10.0))
                .py(px(4.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(rgb(line))
                .text_size(px(12.0))
                .text_color(rgb(t.text))
                .cursor_pointer()
                .child(SharedString::from(label.to_string()))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        run(this);
                        cx.notify();
                    }),
                )
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap(px(6.0))
                    .px(px(14.0))
                    .py(px(8.0))
                    .border_b_1()
                    .border_color(rgb(line))
                    .child(action("New worktree", |this| this.start_work(false), cx))
                    .child(action("Fan out a prompt", |this| this.start_work(true), cx))
                    .when(!page.worktrees.is_empty(), |d| {
                        d.child(action("Changes", Self::review_worktree, cx)).child(action("Go", Self::go_to_worktree, cx)).child(action("Remove", Self::ask_remove_worktree, cx))
                    }),
            )
            .child(div().id("worktree-list").flex_1().min_h_0().py(px(5.0)).overflow_y_scroll().children(rows).when(page.worktrees.is_empty(), |d| {
                d.child(div().px(px(14.0)).py(px(10.0)).text_size(px(12.5)).text_color(rgb(t.muted)).child("No worktrees yet. A worktree gives one task its own checkout and session."))
            }))
            .into_any_element()
    }
}
