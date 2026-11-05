//! The agents view: the Inbox, the agents, the mail, and a review of a pane's changes.
//! Keys come from the person's tuios config, and every action is also a button.

use super::*;
use tessella::agents::{self, Answer, DiffFile, DiffOp, InboxItem, Mail, Note};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(super) enum AgentsTab {
    #[default]
    Inbox,
    Agents,
    Mail,
    Review,
}

/// What the page is typing, and where it goes.
#[derive(Debug, Clone)]
pub(super) enum Write {
    /// A message for an agent: typed as its prompt when it next rests.
    Prompt { session: String, window: String, name: String },
    /// A text answer to the prompt on a pane's screen.
    ScreenAnswer { session: String, window: String, prompt_id: String },
    /// The reason an approval or plan is refused.
    Deny { request_id: String },
    /// A reply in a thread, on the machine the thread is on.
    MailReply { host: String, session: String, to: String, reply_to: i64 },
    /// A new message to an agent, on any machine.
    MailBody { agent: rail::Agent },
    Note { path: String, line: i64, side: &'static str, hunk: String },
}

/// The prompt or approval the Inbox cursor is on, read whole.
#[derive(Debug, Clone, Default)]
pub(super) struct Detail {
    pub lines: Vec<String>,
    pub options: Vec<String>,
    pub prompt_id: String,
    pub risks: Vec<String>,
    pub plan_sha: String,
}

/// The review on show.
#[derive(Debug, Clone, Default)]
pub(super) struct Review {
    pub session: String,
    pub window: String,
    pub name: String,
    pub files: Vec<DiffFile>,
    pub notes: Vec<Note>,
    pub file: usize,
    pub line: usize,
}

#[derive(Debug, Clone, Default)]
pub(super) struct AgentsPage {
    pub tab: AgentsTab,
    pub cursor: usize,
    /// Show only what waits on the person.
    pub waiting_only: bool,
    pub detail: Option<(String, Detail)>,
    /// A risky approval was pressed once, and the next press means it.
    pub risk_armed: bool,
    pub write: Option<(Write, String)>,
    pub mail: Vec<Mail>,
    pub thread: Option<i64>,
    pub review: Option<Review>,
    /// How many rows the detail or diff is scrolled.
    pub scroll: usize,
    /// Choosing who a new message goes to, from these agents.
    pub compose: Option<(Picker, Vec<rail::Agent>)>,
}

/// The diff rows drawn at once.
const DIFF_ROWS: usize = 40;
/// How often other machines are asked for their agents while they are on show.
const REMOTE_EVERY: Duration = Duration::from_secs(5);

/// What a verb's answer goes to in the agents view.
#[derive(Debug, Clone)]
pub(super) enum AgentsReply {
    Approval(String),
    Peek(String),
    Mail,
    Diff,
    Notes,
    Done(String),
    HostAgents,
    MailSent(rail::Agent),
}

impl Tessella {
    fn page(&mut self) -> Option<&mut AgentsPage> {
        if let Overlay::Agents(page) = &mut self.overlay { Some(page) } else { None }
    }

    pub(super) fn open_agents(&mut self, tab: AgentsTab) {
        self.overlay = Overlay::Agents(Box::new(AgentsPage { tab, ..Default::default() }));
        self.tab_opened(tab);
    }

    fn tab_opened(&mut self, tab: AgentsTab) {
        match tab {
            AgentsTab::Inbox => self.load_detail(),
            AgentsTab::Mail => self.load_mail(),
            AgentsTab::Agents | AgentsTab::Review => {}
        }
        // Ask again at once, rather than show what a while ago looked like.
        self.remote_asked = None;
    }

    /// Asks other machines for their agents every few seconds, only while they show.
    pub(super) fn refresh_remote_agents(&mut self, cx: &mut Context<Self>) {
        let showing = matches!(&self.overlay, Overlay::Agents(p) if matches!(p.tab, AgentsTab::Agents | AgentsTab::Mail));
        if !showing || self.remote_asked.is_some_and(|at| at.elapsed() < REMOTE_EVERY) {
            return;
        }
        self.remote_asked = Some(Instant::now());
        self.ask_agents("list-host-agents", json!({}), AgentsReply::HostAgents);
        Self::wake_after(REMOTE_EVERY + Duration::from_millis(20), cx);
    }

    /// Calls a verb on the machine an item is on.
    fn ask_on(&mut self, host: &str, verb: &str, params: Value, reply: AgentsReply) {
        if host.is_empty() {
            return self.ask_agents(verb, params, reply);
        }
        self.next_req += 1;
        self.replies.insert(self.next_req, Reply::Agents(reply));
        self.send(json!({"cmd": "verb", "verb": verb, "req": self.next_req, "host": host, "params": params}));
    }

    fn set_tab(&mut self, tab: AgentsTab) {
        if let Some(page) = self.page() {
            page.tab = tab;
            page.cursor = 0;
            page.scroll = 0;
            page.risk_armed = false;
            page.thread = None;
        }
        self.tab_opened(tab);
    }

    fn ask_agents(&mut self, verb: &str, params: Value, reply: AgentsReply) {
        self.ask(verb, params, Reply::Agents(reply));
    }

    /// Calls a verb and says this when it is done.
    fn act(&mut self, verb: &str, params: Value, done: &str) {
        self.ask_agents(verb, params, AgentsReply::Done(done.to_string()));
    }

    pub(super) fn agents_answered(&mut self, reply: AgentsReply, result: &Value) {
        match reply {
            AgentsReply::HostAgents => {
                let (agents, hosts) = agents::parse_host_agents(result);
                self.remote_agents = agents;
                self.remote_hosts = hosts;
            }
            AgentsReply::MailSent(agent) => self.status = Some(agents::mail_result(&agent, result)),
            AgentsReply::Done(message) => {
                // A later answer than the first one does not count, and says so.
                self.status = if result["applied"] == false {
                    Some(result["reason"].as_str().map_or("Someone answered first.".to_string(), |r| format!("Not applied: {r}.")))
                } else {
                    Some(message)
                };
                self.load_detail();
            }
            AgentsReply::Approval(id) => {
                let text = |k: &str| result[k].as_str().unwrap_or_default().to_string();
                let mut lines: Vec<String> = [text("tool"), text("target"), text("summary")].into_iter().filter(|l| !l.is_empty()).collect();
                lines.extend(text("plan").lines().map(str::to_string));
                let risks = result["risk"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| r["rule"].as_str().map(str::to_string).or_else(|| r.as_str().map(str::to_string)))
                    .collect();
                let detail = Detail { lines, risks, plan_sha: text("plan_sha"), ..Default::default() };
                if let Some(page) = self.page() {
                    page.detail = Some((id, detail));
                }
            }
            AgentsReply::Peek(id) => {
                let strings = |k: &str| -> Vec<String> { result[k].as_array().into_iter().flatten().filter_map(|l| l.as_str().map(str::to_string)).collect() };
                let options = result["options"].as_array().into_iter().flatten().filter_map(|o| Some(format!("{}. {}", o["n"].as_i64()?, o["label"].as_str()?))).collect();
                let detail = Detail {
                    lines: strings("lines"),
                    options,
                    prompt_id: result["prompt_id"].as_str().unwrap_or_default().to_string(),
                    ..Default::default()
                };
                if let Some(page) = self.page() {
                    page.detail = Some((id, detail));
                }
            }
            AgentsReply::Mail => {
                let mail = agents::parse_mail(result);
                if let Some(page) = self.page() {
                    page.mail = mail;
                }
            }
            AgentsReply::Diff => {
                let (files, notes) = (agents::parse_diff(result), agents::parse_notes(result));
                if let Some(review) = self.page().and_then(|p| p.review.as_mut()) {
                    review.files = files;
                    review.notes = notes;
                    review.file = review.file.min(review.files.len().saturating_sub(1));
                }
            }
            AgentsReply::Notes => {
                let (session, window) = self
                    .page()
                    .and_then(|p| p.review.as_ref())
                    .map(|r| (r.session.clone(), r.window.clone()))
                    .unwrap_or_default();
                self.load_diff(session, window);
            }
        }
    }

    // ----- inbox -----

    pub(super) fn inbox_rows(&self, page: &AgentsPage) -> Vec<InboxItem> {
        self.fleet.inbox.iter().filter(|i| !page.waiting_only || i.waits()).cloned().collect()
    }

    fn inbox_current(&self) -> Option<InboxItem> {
        let Overlay::Agents(page) = &self.overlay else { return None };
        let rows = self.inbox_rows(page);
        rows.get(page.cursor.min(rows.len().saturating_sub(1))).cloned()
    }

    /// Reads the prompt or approval under the cursor, when it is not at hand.
    fn load_detail(&mut self) {
        let Some(item) = self.inbox_current() else { return };
        let on_tab = matches!(&self.overlay, Overlay::Agents(p) if p.tab == AgentsTab::Inbox);
        if !on_tab {
            return;
        }
        if let Some(page) = self.page() {
            page.detail = None;
            page.scroll = 0;
            page.risk_armed = false;
        }
        match item.answer() {
            Answer::Held => self.ask_agents("get-approval", json!({"request_id": item.request_id}), AgentsReply::Approval(item.id)),
            Answer::Screen => self.ask_on(&item.host, "peek-prompt", json!({"session": item.session, "window": item.window}), AgentsReply::Peek(item.id)),
            Answer::Mail => {
                self.ask_on(&item.host, "read-agent-messages", json!({"session": item.session, "to": "human", "peek": true, "thread": item.thread}), AgentsReply::Mail);
            }
            Answer::Ask | Answer::Notice => {}
        }
    }

    /// Answers an ask-human question.
    pub(super) fn answer_ask(&mut self, item: &InboxItem, answer: &str) {
        self.act(
            "answer-ask",
            json!({"request_id": item.request_id, "answer": answer, "question": item.summary}),
            &format!("Answered: {answer}."),
        );
    }

    /// Approves or refuses what the cursor is on. A risky approval takes a second press.
    fn answer_item(&mut self, item: &InboxItem, decision: &str) {
        // Tuios takes an answer for the person only on their own machine.
        if !item.host.is_empty() {
            self.status = Some(format!("This one waits on {}, so answer it there.", item.host));
            return;
        }
        let detail = self.page().and_then(|p| p.detail.as_ref().filter(|(id, _)| *id == item.id).map(|(_, d)| d.clone())).unwrap_or_default();
        match item.answer() {
            Answer::Held => {
                let mut params = json!({"request_id": item.request_id, "decision": decision, "summary": item.summary});
                if decision != "deny" && !detail.risks.is_empty() {
                    let armed = self.page().is_some_and(|p| p.risk_armed);
                    if !armed {
                        if let Some(page) = self.page() {
                            page.risk_armed = true;
                        }
                        self.status = Some(format!("Risky: {}. Press again to allow it.", detail.risks.join(", ")));
                        return;
                    }
                    params["risk_ack"] = json!(detail.risks);
                }
                if !detail.plan_sha.is_empty() {
                    params["plan_sha"] = json!(detail.plan_sha);
                }
                self.act("reply-approval", params, &format!("Answered {}.", item.name));
            }
            Answer::Screen => {
                let action = match decision {
                    "once" => "approve",
                    "always" => "approve_always",
                    _ => "deny",
                };
                self.act(
                    "respond",
                    json!({"session": item.session, "window": item.window, "action": action, "prompt_id": detail.prompt_id, "timeout": 2000}),
                    &format!("Answered {}.", item.name),
                );
            }
            _ => self.status = Some("This one takes no approval.".into()),
        }
    }

    /// Runs one of the Inbox's actions on the item under the cursor.
    pub(super) fn inbox_action(&mut self, action: &str, window: &mut Option<&mut Window>) {
        let _ = window;
        let count = match &self.overlay {
            Overlay::Agents(page) => self.inbox_rows(page).len(),
            _ => 0,
        };
        match action {
            "inbox_down" => self.move_cursor(1, count),
            "inbox_up" => self.move_cursor(-1, count),
            "inbox_first" => self.move_cursor(-(count as i32), count),
            "inbox_last" => self.move_cursor(count as i32, count),
            "inbox_page_down" => self.move_cursor(10, count),
            "inbox_page_up" => self.move_cursor(-10, count),
            "inbox_close" => self.overlay = Overlay::None,
            "inbox_filter" => {
                if let Some(page) = self.page() {
                    page.waiting_only = !page.waiting_only;
                    page.cursor = 0;
                }
                self.load_detail();
            }
            "inbox_mailbox" => self.set_tab(AgentsTab::Mail),
            "inbox_tab_next" | "inbox_tab_prev" => self.step_tab(action == "inbox_tab_next"),
            "inbox_detail_down" | "inbox_detail_up" => {
                let down = action == "inbox_detail_down";
                if let Some(page) = self.page() {
                    page.scroll = if down { page.scroll + 5 } else { page.scroll.saturating_sub(5) };
                }
            }
            _ => {
                let Some(item) = self.inbox_current() else { return };
                self.inbox_item_action(action, &item);
            }
        }
    }

    fn inbox_item_action(&mut self, action: &str, item: &InboxItem) {
        match action {
            "inbox_go" => {
                if item.answer() == Answer::Mail {
                    let thread = item.thread;
                    self.set_tab(AgentsTab::Mail);
                    if let Some(page) = self.page() {
                        page.thread = Some(thread);
                    }
                } else {
                    self.overlay = Overlay::None;
                    self.focus_anywhere(&item.session.clone(), &item.window.clone());
                }
            }
            "inbox_allow" => self.answer_item(item, "once"),
            "inbox_allow_always" => self.answer_item(item, "always"),
            "inbox_deny_reason" => {
                if item.answer() == Answer::Held && item.deny_message {
                    let id = item.request_id.clone();
                    if let Some(page) = self.page() {
                        page.write = Some((Write::Deny { request_id: id }, String::new()));
                    }
                } else {
                    self.answer_item(item, "deny");
                }
            }
            "inbox_reply" | "inbox_write" => self.start_reply(item),
            "inbox_dismiss" => self.act("dismiss-attention", json!({"id": item.id}), "Dismissed."),
            "inbox_undo" => self.act("mark-attention", json!({"id": item.id, "action": "restore"}), "Put it back."),
            "inbox_snooze" => self.act("mark-attention", json!({"id": item.id, "action": "snooze", "for_ms": 3_600_000}), "Snoozed for an hour."),
            "inbox_resume" => self.act("resume-agent", json!({"session": item.session, "window": item.window}), "Resumed."),
            "inbox_review" => self.open_review(item.session.clone(), item.window.clone(), item.name.clone()),
            _ => {}
        }
    }

    fn start_reply(&mut self, item: &InboxItem) {
        let write = match item.answer() {
            Answer::Screen => {
                let prompt_id = self.page().and_then(|p| p.detail.as_ref().map(|(_, d)| d.prompt_id.clone())).unwrap_or_default();
                Write::ScreenAnswer { session: item.session.clone(), window: item.window.clone(), prompt_id }
            }
            Answer::Mail => {
                let last = self.page().and_then(|p| p.mail.last().cloned());
                let Some(last) = last else {
                    self.status = Some("The mail has not loaded yet.".into());
                    return;
                };
                if last.from.is_empty() {
                    self.status = Some("This mail does not say who sent it.".into());
                    return;
                }
                Write::MailReply { host: item.host.clone(), session: item.session.clone(), to: last.from.clone(), reply_to: last.id }
            }
            _ if !item.host.is_empty() => {
                self.status = Some(format!("This pane is on {}. Mail its agent from the Agents tab.", item.host));
                return;
            }
            Answer::Ask => {
                self.status = Some("Choose one of the answers.".into());
                return;
            }
            _ => Write::Prompt { session: item.session.clone(), window: item.window.clone(), name: item.name.clone() },
        };
        if let Some(page) = self.page() {
            page.write = Some((write, String::new()));
        }
    }

    fn move_cursor(&mut self, by: i32, count: usize) {
        if let Some(page) = self.page() {
            page.cursor = (page.cursor as i32 + by).clamp(0, count.saturating_sub(1) as i32) as usize;
            page.risk_armed = false;
        }
        self.load_detail();
    }

    fn step_tab(&mut self, next: bool) {
        const ORDER: [AgentsTab; 4] = [AgentsTab::Inbox, AgentsTab::Agents, AgentsTab::Mail, AgentsTab::Review];
        let Some(page) = self.page() else { return };
        let at = ORDER.iter().position(|t| *t == page.tab).unwrap_or(0);
        let to = (at + if next { 1 } else { 3 }) % 4;
        self.set_tab(ORDER[to]);
    }

    // ----- agents -----

    /// The agents, without the plain terminals the daemon lists with them.
    fn agent_rows(&self) -> Vec<rail::Agent> {
        self.fleet.agents.iter().chain(&self.remote_agents).filter(|a| !a.state.is_empty() && a.state != "none").cloned().collect()
    }

    /// Starts a new message: first, who it goes to.
    fn open_compose(&mut self) {
        let agents = self.agent_rows();
        let items = agents.iter().enumerate().map(|(i, a)| Item::new(a.name.clone(), agents::place(a), i.to_string())).collect();
        if let Some(page) = self.page() {
            page.compose = Some((Picker::new(items, String::new()), agents));
        }
    }

    fn write_mail_to(&mut self, agent: rail::Agent) {
        if let Some(page) = self.page() {
            page.compose = None;
            page.write = Some((Write::MailBody { agent }, String::new()));
        }
    }

    fn compose_key(&mut self, k: &Keystroke) {
        let Some((picker, agents)) = self.page().and_then(|p| p.compose.as_mut()) else { return };
        match k.key.as_str() {
            "escape" => {
                if let Some(page) = self.page() {
                    page.compose = None;
                }
            }
            "enter" => {
                let chosen = picker.current().and_then(|i| i.value.parse::<usize>().ok()).and_then(|i| agents.get(i).cloned());
                if let Some(agent) = chosen {
                    self.write_mail_to(agent);
                }
            }
            "up" => picker.step(-1),
            "down" => picker.step(1),
            "backspace" => picker.backspace(),
            "space" => picker.push(" "),
            _ => {
                if let Some(typed) = k.key_char.as_deref().filter(|_| !k.modifiers.control && !k.modifiers.platform) {
                    picker.push(typed);
                }
            }
        }
    }

    fn current_agent(&self) -> Option<rail::Agent> {
        let Overlay::Agents(page) = &self.overlay else { return None };
        let rows = self.agent_rows();
        rows.get(page.cursor.min(rows.len().saturating_sub(1))).cloned()
    }

    /// Runs an action on an agent, from the Agents tab or the sidebar.
    pub(super) fn agent_action(&mut self, action: &str, agent: &rail::Agent) {
        let (session, window) = (agent.session.clone(), agent.window.clone());
        // Tuios takes the person's other actions only on their own machine, so mail is
        // what goes to an agent elsewhere.
        if !agent.host.is_empty() && !matches!(action, "agent_mail" | "inbox_mailbox" | "agent_reply" | "inbox_reply" | "inbox_write") {
            self.status = Some(format!("{} is on {}. From here you can send it mail.", agent.name, agent.host));
            return;
        }
        if !agent.host.is_empty() || matches!(action, "agent_mail" | "inbox_mailbox") {
            return self.write_mail_to(agent.clone());
        }
        match action {
            "agent_reply" | "inbox_reply" | "inbox_write" => {
                if !matches!(self.overlay, Overlay::Agents(_)) {
                    self.open_agents(AgentsTab::Agents);
                }
                if let Some(page) = self.page() {
                    page.write = Some((Write::Prompt { session, window, name: agent.name.clone() }, String::new()));
                }
            }
            "agent_review" | "inbox_review" => {
                if !matches!(self.overlay, Overlay::Agents(_)) {
                    self.open_agents(AgentsTab::Review);
                }
                self.open_review(session, window, agent.name.clone());
            }
            "agent_snooze" | "inbox_snooze" => {
                self.act("mark-attention", json!({"session": session, "window": window, "kind": "finished", "action": "snooze", "for_ms": 3_600_000}), "Snoozed for an hour.");
            }
            "agent_unread" | "inbox_undo" => {
                self.act("mark-attention", json!({"session": session, "window": window, "kind": "finished", "action": "unread"}), "Marked unread.");
            }
            "agent_cancel_queued" | "inbox_mark" => {
                self.act("cancel-queued", json!({"session": session, "window": window, "all": true}), "Dropped its queued messages.");
            }
            "inbox_resume" => self.act("resume-agent", json!({"session": session, "window": window}), "Resumed."),
            "inbox_go" => {
                self.overlay = Overlay::None;
                self.focus_anywhere(&session, &window);
            }
            _ => {}
        }
    }

    // ----- mail -----

    fn load_mail(&mut self) {
        let session = self.state.session.clone();
        self.ask_agents("read-agent-messages", json!({"session": session, "to": "human", "peek": true, "limit": 200}), AgentsReply::Mail);
    }

    fn mail_action(&mut self, action: &str) {
        let page_threads = match &self.overlay {
            Overlay::Agents(page) => agents::threads(&page.mail),
            _ => return,
        };
        let Some(page) = self.page() else { return };
        let count = page_threads.len();
        match action {
            "mail_down" => page.cursor = (page.cursor + 1).min(count.saturating_sub(1)),
            "mail_up" => page.cursor = page.cursor.saturating_sub(1),
            "mail_page_down" => page.cursor = (page.cursor + 10).min(count.saturating_sub(1)),
            "mail_page_up" => page.cursor = page.cursor.saturating_sub(10),
            "mail_back" => {
                if page.thread.take().is_none() {
                    self.overlay = Overlay::None;
                }
            }
            "mail_open" => {
                let at = page.cursor;
                page.thread = page_threads.get(at).map(|t| t.0);
            }
            "mail_new" => self.open_compose(),
            "mail_reply" => {
                let thread = page.thread.or_else(|| page_threads.get(page.cursor).map(|t| t.0));
                let last = page_threads.iter().find(|t| Some(t.0) == thread).and_then(|t| t.1.last()).cloned();
                match last.filter(|l| !l.from.is_empty()) {
                    Some(last) => {
                        let session = self.state.session.clone();
                        if let Some(page) = self.page() {
                            page.write = Some((Write::MailReply { host: String::new(), session, to: last.from, reply_to: last.id }, String::new()));
                        }
                    }
                    None => self.status = Some("This mail does not say who sent it.".into()),
                }
            }
            _ => {}
        }
    }

    // ----- review -----

    pub(super) fn open_review(&mut self, session: String, window: String, name: String) {
        if let Some(page) = self.page() {
            page.tab = AgentsTab::Review;
            page.scroll = 0;
            page.review = Some(Review { session: session.clone(), window: window.clone(), name, ..Default::default() });
        }
        self.load_diff(session, window);
    }

    fn load_diff(&mut self, session: String, window: String) {
        self.ask_agents("review-diff", json!({"session": session, "window": window}), AgentsReply::Diff);
    }

    fn review_key(&mut self, name: &str) {
        let Some(page) = self.page() else { return };
        let Some(review) = page.review.as_mut() else {
            if matches!(name, "esc" | "escape" | "q") {
                self.overlay = Overlay::None;
            }
            return;
        };
        let count = review.files.get(review.file).map_or(0, |f| f.lines.len());
        let files = review.files.len();
        match name {
            "esc" | "escape" | "q" => self.overlay = Overlay::None,
            "j" | "down" => review.line = (review.line + 1).min(count.saturating_sub(1)),
            "k" | "up" => review.line = review.line.saturating_sub(1),
            "pagedown" | "J" => review.line = (review.line + 20).min(count.saturating_sub(1)),
            "pageup" | "K" => review.line = review.line.saturating_sub(20),
            "tab" | "l" | "right" => {
                review.file = (review.file + 1).min(files.saturating_sub(1));
                review.line = 0;
            }
            "shift+tab" | "h" | "left" => {
                review.file = review.file.saturating_sub(1);
                review.line = 0;
            }
            "n" => {
                let file = review.files.get(review.file);
                let line = file.and_then(|f| f.lines.get(review.line));
                if let (Some(file), Some(line)) = (file, line) {
                    let path = file.path.clone();
                    let write = match line.op {
                        DiffOp::Hunk => Write::Note { path, line: 0, side: "new", hunk: line.hunk.clone() },
                        DiffOp::Delete => Write::Note { path, line: line.old, side: "old", hunk: String::new() },
                        _ => Write::Note { path, line: line.new, side: "new", hunk: String::new() },
                    };
                    page.write = Some((write, String::new()));
                }
            }
            "S" => {
                let (session, window) = (review.session.clone(), review.window.clone());
                self.ask_agents("send-review", json!({"session": session, "window": window}), AgentsReply::Notes);
                self.status = Some("Sent your notes to the agent.".into());
            }
            _ => {}
        }
    }

    // ----- keys -----

    pub(super) fn agents_key(&mut self, k: &Keystroke, name: &str) {
        if self.page().is_some_and(|p| p.write.is_some()) {
            return self.write_key(k);
        }
        if self.page().is_some_and(|p| p.compose.is_some()) {
            return self.compose_key(k);
        }
        let tab = self.page().map_or(AgentsTab::Inbox, |p| p.tab);
        if matches!(name, "]" | "[") {
            return self.step_tab(name == "]");
        }
        match tab {
            AgentsTab::Review => self.review_key(name),
            AgentsTab::Mail => {
                if let Some(action) = self.keymap.scope_action("mail", name).map(str::to_string) {
                    self.mail_action(&action);
                }
            }
            AgentsTab::Inbox => {
                // An ask-human question is answered by the number of its answer.
                if let (Ok(n), Some(item)) = (name.parse::<usize>(), self.inbox_current()) {
                    if item.answer() == Answer::Ask {
                        if let Some(answer) = n.checked_sub(1).and_then(|i| item.options.get(i)).cloned() {
                            self.answer_ask(&item, &answer);
                        }
                        return;
                    }
                }
                if let Some(action) = self.keymap.scope_action("inbox", name).map(str::to_string) {
                    self.inbox_action(&action, &mut None);
                }
            }
            AgentsTab::Agents => {
                let Some(action) = self.keymap.scope_action("inbox", name).map(str::to_string) else { return };
                let count = self.agent_rows().len();
                match action.as_str() {
                    "inbox_down" | "inbox_up" | "inbox_first" | "inbox_last" | "inbox_page_down" | "inbox_page_up" => {
                        let by = match action.as_str() {
                            "inbox_down" => 1,
                            "inbox_up" => -1,
                            "inbox_first" => -(count as i32),
                            "inbox_last" => count as i32,
                            "inbox_page_down" => 10,
                            _ => -10,
                        };
                        self.move_cursor(by, count);
                    }
                    "inbox_close" => self.overlay = Overlay::None,
                    "inbox_tab_next" | "inbox_tab_prev" => self.step_tab(action == "inbox_tab_next"),
                    _ => {
                        if let Some(agent) = self.current_agent() {
                            self.agent_action(&action, &agent);
                        }
                    }
                }
            }
        }
    }

    fn write_key(&mut self, k: &Keystroke) {
        let Some(page) = self.page() else { return };
        let Some((_, text)) = page.write.as_mut() else { return };
        match k.key.as_str() {
            "escape" => page.write = None,
            "backspace" => {
                text.pop();
            }
            "space" => text.push(' '),
            "enter" if k.modifiers.shift => text.push('\n'),
            "enter" => {
                if let Some((write, text)) = page.write.take() {
                    self.send_write(write, text);
                }
            }
            _ => {
                if let Some(typed) = k.key_char.as_deref().filter(|_| !k.modifiers.control && !k.modifiers.platform) {
                    text.push_str(typed);
                }
            }
        }
    }

    fn send_write(&mut self, write: Write, text: String) {
        let text_empty = text.trim().is_empty();
        match write {
            Write::Deny { request_id } => {
                self.act("reply-approval", json!({"request_id": request_id, "decision": "deny", "message": text}), "Refused.");
            }
            _ if text_empty => {}
            Write::Prompt { session, window, name } => {
                self.act("queue-prompt", json!({"session": session, "window": window, "text": text}), &format!("Queued for {name}."));
            }
            Write::ScreenAnswer { session, window, prompt_id } => {
                self.act("respond", json!({"session": session, "window": window, "action": "text", "value": text, "prompt_id": prompt_id, "timeout": 2000}), "Answered.");
            }
            Write::MailReply { host, session, to, reply_to } => {
                let mut params = json!({"session": session, "to": to, "from": "human", "reply_to": reply_to, "text": text});
                if !host.is_empty() {
                    params["host"] = json!(host);
                }
                self.ask_agents("send-agent-message", params, AgentsReply::Done("Replied.".into()));
                self.load_mail();
            }
            Write::MailBody { agent } => {
                self.ask_agents("send-agent-message", agents::mail_params(&agent, &text), AgentsReply::MailSent(agent));
            }
            Write::Note { path, line, side, hunk } => {
                let review = self.page().and_then(|p| p.review.as_ref().map(|r| (r.session.clone(), r.window.clone())));
                let Some((session, window)) = review else { return };
                let mut params = json!({"action": "add", "session": session, "window": window, "path": path, "text": text});
                if line > 0 {
                    params["line"] = json!(line);
                    params["side"] = json!(side);
                } else {
                    params["hunk"] = json!(hunk);
                }
                self.ask_agents("review-note", params, AgentsReply::Notes);
            }
        }
    }

    // ----- views -----

    pub(super) fn agents_view(&self, page: &AgentsPage, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme.clone();
        let line = theme::mix(t.palette.bg, t.text, 0.14);
        let tab = |label: String, which: AgentsTab, cx: &mut Context<Self>| {
            let on = page.tab == which;
            div()
                .px(px(12.0))
                .py(px(5.0))
                .rounded(px(7.0))
                .text_size(px(12.5))
                .cursor_pointer()
                .when(on, |d| d.bg(rgb(t.accent)).text_color(rgb(t.on_accent)))
                .when(!on, |d| d.text_color(rgb(t.muted)))
                .child(label)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.set_tab(which);
                        cx.notify();
                    }),
                )
        };
        let waiting = self.fleet.inbox.iter().filter(|i| i.waits()).count();
        let body = match page.tab {
            AgentsTab::Inbox => self.inbox_view(page, cx).into_any_element(),
            AgentsTab::Agents => self.agents_list_view(page, cx).into_any_element(),
            AgentsTab::Mail => self.mail_view(page, cx).into_any_element(),
            AgentsTab::Review => self.review_view(page).into_any_element(),
        };
        let hint = match page.tab {
            AgentsTab::Inbox => "a allow  ·  A always  ·  R refuse  ·  r reply  ·  d dismiss  ·  z snooze  ·  u undo  ·  v review  ·  ⏎ go  ·  [ ] tabs  ·  esc",
            AgentsTab::Agents if self.current_agent().is_some_and(|a| !a.host.is_empty()) => "m mail  ·  [ ] tabs  ·  esc",
            AgentsTab::Agents => "m mail  ·  r reply  ·  v review  ·  z snooze  ·  u unread  ·  x drop queued  ·  y resume  ·  ⏎ go  ·  [ ] tabs  ·  esc",
            AgentsTab::Mail if page.compose.is_some() => "type to filter  ·  ↑↓ choose  ·  ⏎ write  ·  esc cancel",
            AgentsTab::Mail => "⏎ open  ·  r reply  ·  n new message  ·  esc back  ·  [ ] tabs",
            AgentsTab::Review => "j k line  ·  h l file  ·  n note  ·  S send notes  ·  [ ] tabs  ·  esc",
        };
        div()
            .flex()
            .flex_col()
            .h(px(640.0))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(4.0))
                    .h(px(46.0))
                    .px(px(12.0))
                    .border_b_1()
                    .border_color(rgb(line))
                    .child(tab(if waiting > 0 { format!("Inbox  {waiting}") } else { "Inbox".into() }, AgentsTab::Inbox, cx))
                    .child(tab("Agents".into(), AgentsTab::Agents, cx))
                    .child(tab("Mail".into(), AgentsTab::Mail, cx))
                    .child(tab("Review".into(), AgentsTab::Review, cx)),
            )
            .child(div().flex_1().min_h_0().child(body))
            .when_some(page.write.as_ref(), |d, (write, text)| {
                let title = match write {
                    Write::Prompt { name, .. } => format!("Message for {name}, typed when it rests"),
                    Write::ScreenAnswer { .. } => "Your answer to the prompt".to_string(),
                    Write::Deny { .. } => "Why it is refused (optional)".to_string(),
                    Write::MailReply { .. } => "Your reply".to_string(),
                    Write::MailBody { agent } if agent.host.is_empty() => format!("Mail for {}", agent.name),
                    Write::MailBody { agent } => format!("Mail for {} on {}", agent.name, agent.host),
                    Write::Note { path, line, .. } => format!("Note on {path}:{line}"),
                };
                d.child(
                    div()
                        .flex_none()
                        .px(px(16.0))
                        .py(px(9.0))
                        .border_t_1()
                        .border_color(rgb(line))
                        .child(div().pb(px(3.0)).text_size(px(11.5)).text_color(rgb(t.muted)).child(title))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .text_size(px(13.0))
                                .text_color(rgb(t.text))
                                .child(SharedString::from(text.clone()))
                                .child(div().w(px(2.0)).h(px(16.0)).bg(rgb(t.accent))),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(36.0))
                    .px(px(16.0))
                    .border_t_1()
                    .border_color(rgb(line))
                    .text_size(px(11.0))
                    .text_color(rgb(t.faint))
                    .child(hint),
            )
    }

    /// A list row, shared by the tabs.
    fn list_row(&self, on: bool, dot: u32, title: String, note: String, side: String, click: impl Fn(&mut Self, &mut Context<Self>) + 'static, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let hover = theme::mix(t.palette.bg, t.text, 0.08);
        let sub = if on { t.on_accent } else { t.muted };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .mx(px(8.0))
            .px(px(9.0))
            .py(px(5.0))
            .rounded(px(7.0))
            .cursor_pointer()
            .when(on, |d| d.bg(rgb(t.accent)))
            .when(!on, |d| d.hover(move |s| s.bg(rgb(hover))))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    click(this, cx);
                    cx.notify();
                }),
            )
            .child(div().size(px(7.0)).flex_none().rounded_full().bg(rgb(if on { t.on_accent } else { dot })))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().text_size(px(12.5)).text_color(rgb(if on { t.on_accent } else { t.text })).child(SharedString::from(title)))
                    .when(!note.is_empty(), |d| d.child(div().truncate().text_size(px(11.0)).text_color(rgb(sub)).child(SharedString::from(note)))),
            )
            .child(div().flex_none().text_size(px(10.5)).text_color(rgb(sub)).child(SharedString::from(side)))
    }

    fn button(&self, label: String, strong: bool, click: impl Fn(&mut Self, &mut Context<Self>) + 'static, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let line = theme::mix(t.palette.bg, t.text, 0.2);
        let hover = theme::mix(t.palette.bg, t.text, 0.08);
        div()
            .px(px(11.0))
            .py(px(5.0))
            .rounded(px(7.0))
            .text_size(px(12.0))
            .cursor_pointer()
            .when(strong, |d| d.bg(rgb(t.accent)).text_color(rgb(t.on_accent)))
            .when(!strong, |d| d.border_1().border_color(rgb(line)).text_color(rgb(t.text)).hover(move |s| s.bg(rgb(hover))))
            .child(SharedString::from(label))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    click(this, cx);
                    cx.notify();
                }),
            )
    }

    fn kind_color(&self, item: &InboxItem) -> u32 {
        let t = &self.theme;
        match item.kind.as_str() {
            "errored" => t.agent("errored"),
            "finished" | "resume" | "outbox" => t.agent("done"),
            _ => t.agent("needs_input"),
        }
        .unwrap_or(t.accent)
    }

    fn inbox_view(&self, page: &AgentsPage, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let line = theme::mix(t.palette.bg, t.text, 0.14);
        let rows = self.inbox_rows(page);
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let at = page.cursor.min(rows.len().saturating_sub(1));
        let list: Vec<_> = rows
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let side = agents::age(now - item.since / 1_000_000_000);
                let title = if item.name.is_empty() { item.kind_label().to_string() } else { format!("{}  ·  {}", item.kind_label(), item.name) };
                self.list_row(
                    i == at,
                    self.kind_color(item),
                    title,
                    item.summary.clone(),
                    side,
                    move |this, _| {
                        if let Some(page) = this.page() {
                            page.cursor = i;
                        }
                        this.load_detail();
                    },
                    cx,
                )
                .into_any_element()
            })
            .collect();
        let empty = list.is_empty();
        let detail = rows.get(at).map(|item| self.item_detail(item, page, cx).into_any_element());
        div()
            .flex()
            .size_full()
            .child(
                div()
                    .id("inbox-list")
                    .w(px(400.0))
                    .flex_none()
                    .py(px(7.0))
                    .overflow_y_scroll()
                    .border_r_1()
                    .border_color(rgb(line))
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .children(list)
                    .when(empty, |d| d.child(div().p(px(18.0)).text_size(px(13.0)).text_color(rgb(t.muted)).child("Nothing is waiting for you."))),
            )
            .child(div().flex_1().min_w_0().children(detail))
    }

    fn item_detail(&self, item: &InboxItem, page: &AgentsPage, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let detail = page.detail.as_ref().filter(|(id, _)| *id == item.id).map(|(_, d)| d);
        let mut buttons: Vec<gpui::AnyElement> = Vec::new();
        let mut add = |this: &Self, label: &str, strong: bool, action: &'static str, cx: &mut Context<Self>| {
            let label = label.to_string();
            let target = item.clone();
            buttons.push(this.button(label, strong, move |this, _| this.inbox_item_action(action, &target), cx).into_any_element());
        };
        match item.answer() {
            Answer::Held | Answer::Screen => {
                add(self, "Allow", true, "inbox_allow", cx);
                add(self, "Always", false, "inbox_allow_always", cx);
                add(self, "Refuse", false, "inbox_deny_reason", cx);
            }
            _ => {}
        }
        if item.answer() != Answer::Ask {
            add(self, if item.answer() == Answer::Mail { "Reply" } else { "Message" }, false, "inbox_reply", cx);
        }
        add(self, "Go to pane", false, "inbox_go", cx);
        add(self, "Review", false, "inbox_review", cx);
        add(self, "Snooze", false, "inbox_snooze", cx);
        add(self, "Dismiss", false, "inbox_dismiss", cx);
        if item.kind == "resume" {
            add(self, "Resume", true, "inbox_resume", cx);
        }
        let answers: Vec<_> = if item.answer() == Answer::Ask {
            item.options
                .iter()
                .enumerate()
                .map(|(i, option)| {
                    let (target, answer) = (item.clone(), option.clone());
                    self.button(format!("{}  {option}", i + 1), i == 0, move |this, _| this.answer_ask(&target, &answer), cx).into_any_element()
                })
                .collect()
        } else {
            Vec::new()
        };
        let mail_lines: Vec<String> = if item.answer() == Answer::Mail {
            page.mail.iter().map(|m| format!("{}: {}", m.sender(), m.text)).collect()
        } else {
            Vec::new()
        };
        let lines: Vec<String> = detail.map(|d| d.lines.clone()).unwrap_or_default();
        // The prompt's own lines usually show its options already.
        let options: Vec<String> = detail.filter(|d| d.lines.is_empty()).map(|d| d.options.clone()).unwrap_or_default();
        let risks: Vec<String> = detail.map(|d| d.risks.clone()).unwrap_or_default();
        let danger = t.agent("errored").unwrap_or(t.accent);
        let place = if item.host.is_empty() { item.session.clone() } else { format!("{} on {}", item.session, item.host) };
        let scroll = page.scroll;
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("inbox-detail")
                    .flex_1()
                    .min_h_0()
                    .p(px(16.0))
                    .overflow_y_scroll()
                    .child(div().text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(t.text)).child(SharedString::from(if item.name.is_empty() { item.kind_label().to_string() } else { item.name.clone() })))
                    .child(div().pb(px(9.0)).text_size(px(11.5)).text_color(rgb(t.muted)).child(SharedString::from(format!("{}  ·  {place}", item.kind_label()))))
                    .child(div().pb(px(10.0)).text_size(px(13.0)).text_color(rgb(t.text)).child(SharedString::from(item.summary.clone())))
                    .when(!risks.is_empty(), |d| d.child(div().pb(px(8.0)).text_size(px(12.0)).text_color(rgb(danger)).child(SharedString::from(format!("Risky: {}", risks.join(", "))))))
                    .children(lines.into_iter().chain(options).chain(mail_lines).skip(scroll).map(|l| div().text_size(px(11.5)).text_color(rgb(t.muted)).child(SharedString::from(l))))
                    .when(!answers.is_empty(), |d| d.child(div().flex().flex_wrap().gap(px(6.0)).pt(px(6.0)).children(answers))),
            )
            .child(div().flex().flex_none().flex_wrap().gap(px(6.0)).px(px(14.0)).pb(px(10.0)).children(buttons))
    }

    fn agents_list_view(&self, page: &AgentsPage, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let line = theme::mix(t.palette.bg, t.text, 0.14);
        let agent_rows = self.agent_rows();
        let at = page.cursor.min(agent_rows.len().saturating_sub(1));
        let list: Vec<_> = agent_rows
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let color = t.agent(&a.state).unwrap_or(t.muted);
                self.list_row(
                    i == at,
                    color,
                    a.name.clone(),
                    a.message.clone(),
                    agents::place(a),
                    move |this, _| {
                        if let Some(page) = this.page() {
                            page.cursor = i;
                        }
                    },
                    cx,
                )
                .into_any_element()
            })
            .collect();
        let empty = list.is_empty();
        let current = agent_rows.get(at).cloned();
        let actions: Vec<_> = current
            .iter()
            .flat_map(|agent| {
                let local = [("Go to pane", "inbox_go"), ("Send mail", "agent_mail"), ("Message", "agent_reply"), ("Review", "agent_review"), ("Snooze", "agent_snooze"), ("Mark unread", "agent_unread"), ("Drop queued", "agent_cancel_queued"), ("Resume", "inbox_resume")];
                let rows: &[(&str, &'static str)] = if agent.host.is_empty() { &local } else { &[("Send mail", "agent_mail")] };
                rows.iter()
                    .map(|(label, action)| {
                        let (agent, action) = (agent.clone(), *action);
                        let strong = action == "inbox_go" || !agent.host.is_empty();
                        self.button(label.to_string(), strong, move |this, _| this.agent_action(action, &agent), cx).into_any_element()
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let down: Vec<_> = self
            .remote_hosts
            .iter()
            .filter(|h| !h.up)
            .map(|h| {
                let text = if h.reason.is_empty() { format!("{} is not answering, so its agents may be out of date.", h.host) } else { format!("{}: {}", h.host, h.reason) };
                div().pb(px(4.0)).text_size(px(11.5)).text_color(rgb(t.agent("needs_input").unwrap_or(t.muted))).child(SharedString::from(text))
            })
            .collect();
        div()
            .flex()
            .size_full()
            .child(
                div()
                    .id("agents-list")
                    .w(px(400.0))
                    .flex_none()
                    .py(px(7.0))
                    .overflow_y_scroll()
                    .border_r_1()
                    .border_color(rgb(line))
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .children(list)
                    .when(empty, |d| d.child(div().p(px(18.0)).text_size(px(13.0)).text_color(rgb(t.muted)).child("No agents are running."))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .p(px(16.0))
                    .children(down)
                    .when_some(current.clone(), |d, a| {
                        let place = agents::place(&a);
                        d.child(div().text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(t.text)).child(SharedString::from(a.name)))
                            .child(div().pb(px(9.0)).text_size(px(11.5)).text_color(rgb(t.muted)).child(SharedString::from(format!("{}  ·  {place}", a.state.replace('_', " ")))))
                            .child(div().pb(px(12.0)).text_size(px(12.5)).text_color(rgb(t.text)).child(SharedString::from(a.message)))
                    })
                    .child(div().flex().flex_wrap().gap(px(6.0)).children(actions)),
            )
    }

    fn mail_view(&self, page: &AgentsPage, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let line = theme::mix(t.palette.bg, t.text, 0.14);
        let threads = agents::threads(&page.mail);
        let at = page.cursor.min(threads.len().saturating_sub(1));
        let list: Vec<_> = threads
            .iter()
            .enumerate()
            .map(|(i, (id, messages))| {
                let first = &messages[0];
                let id = *id;
                let title = if first.subject.is_empty() { first.sender().to_string() } else { first.subject.clone() };
                let note = messages.last().map_or(String::new(), |m| m.text.lines().next().unwrap_or_default().to_string());
                self.list_row(
                    i == at,
                    t.accent,
                    title,
                    note,
                    messages.len().to_string(),
                    move |this, _| {
                        if let Some(page) = this.page() {
                            page.cursor = i;
                            page.thread = Some(id);
                        }
                    },
                    cx,
                )
                .into_any_element()
            })
            .collect();
        let empty = list.is_empty();
        let open = threads.iter().find(|(id, _)| page.thread == Some(*id)).or_else(|| threads.get(at));
        let messages: Vec<_> = open
            .into_iter()
            .flat_map(|(_, list)| list.iter())
            .map(|m| {
                div()
                    .pb(px(12.0))
                    .child(div().text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(t.muted)).child(SharedString::from(format!("{} → {}", m.sender(), if m.to_label.is_empty() { &m.to } else { &m.to_label }))))
                    .child(div().text_size(px(12.5)).text_color(rgb(t.text)).child(SharedString::from(m.text.clone())))
            })
            .collect();
        let recipients = page.compose.as_ref().map(|(picker, agents)| {
            let on = picker.selected;
            let rows: Vec<_> = picker
                .filtered()
                .into_iter()
                .enumerate()
                .map(|(i, item)| {
                    let chosen = item.value.parse::<usize>().ok().and_then(|n| agents.get(n).cloned());
                    let color = chosen.as_ref().and_then(|a| t.agent(&a.state)).unwrap_or(t.muted);
                    self.list_row(
                        i == on,
                        color,
                        item.label.clone(),
                        String::new(),
                        item.detail.clone(),
                        move |this, _| {
                            if let Some(agent) = chosen.clone() {
                                this.write_mail_to(agent);
                            }
                        },
                        cx,
                    )
                    .into_any_element()
                })
                .collect();
            (picker.query.clone(), rows)
        });
        let new_message = self.button("New message".to_string(), true, |this, _| this.open_compose(), cx);
        div()
            .flex()
            .size_full()
            .child(
                div()
                    .id("mail-list")
                    .w(px(360.0))
                    .flex_none()
                    .py(px(7.0))
                    .overflow_y_scroll()
                    .border_r_1()
                    .border_color(rgb(line))
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .children(list)
                    .when(empty, |d| d.child(div().p(px(18.0)).text_size(px(13.0)).text_color(rgb(t.muted)).child("No mail for you."))),
            )
            .child(
                div()
                    .id("mail-thread")
                    .flex_1()
                    .min_w_0()
                    .p(px(16.0))
                    .overflow_y_scroll()
                    .when_some(recipients, |d, (query, rows)| {
                        let none = rows.is_empty();
                        d.child(div().pb(px(3.0)).text_size(px(15.0)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(t.text)).child("Send to which agent?"))
                            .child(div().pb(px(9.0)).text_size(px(12.5)).text_color(rgb(if query.is_empty() { t.faint } else { t.text })).child(SharedString::from(if query.is_empty() { "Type to filter".to_string() } else { query })))
                            .children(rows)
                            .when(none, |d| d.child(div().text_size(px(12.5)).text_color(rgb(t.muted)).child("No agent matches.")))
                    })
                    .when(page.compose.is_none(), |d| d.child(div().flex().justify_end().pb(px(8.0)).child(new_message)).children(messages)),
            )
    }

    fn review_view(&self, page: &AgentsPage) -> impl IntoElement {
        let t = &self.theme;
        let line_color = theme::mix(t.palette.bg, t.text, 0.14);
        let Some(review) = &page.review else {
            return div().p(px(18.0)).text_size(px(13.0)).text_color(rgb(t.muted)).child("Press v on an agent to review what it changed.").into_any_element();
        };
        let (add, del) = (t.agent("done").unwrap_or(t.accent), t.agent("errored").unwrap_or(t.accent));
        let files = review.files.iter().enumerate().map(|(i, f)| {
            let on = i == review.file;
            div()
                .flex()
                .gap(px(6.0))
                .px(px(12.0))
                .py(px(3.0))
                .text_size(px(11.5))
                .when(on, |d| d.bg(rgb(t.accent)).text_color(rgb(t.on_accent)))
                .when(!on, |d| d.text_color(rgb(t.text)))
                .child(div().flex_1().min_w_0().truncate().child(SharedString::from(f.path.clone())))
                .child(div().flex_none().child(SharedString::from(format!("+{} −{}", f.added, f.removed))))
        });
        let file = review.files.get(review.file);
        let first = review.line.saturating_sub(DIFF_ROWS / 2);
        let rows: Vec<_> = file
            .map(|f| {
                let path = f.path.clone();
                f.lines
                    .iter()
                    .enumerate()
                    .skip(first)
                    .take(DIFF_ROWS)
                    .flat_map(|(i, l)| {
                        let (mark, color) = match l.op {
                            DiffOp::Hunk => ("", t.muted),
                            DiffOp::Add => ("+", add),
                            DiffOp::Delete => ("-", del),
                            DiffOp::Context => (" ", t.text),
                        };
                        let number = match l.op {
                            DiffOp::Hunk => String::new(),
                            DiffOp::Delete => l.old.to_string(),
                            _ => l.new.to_string(),
                        };
                        let row = div()
                            .flex()
                            .h(px(16.0))
                            .text_size(px(11.5))
                            .when(i == review.line, |d| d.bg(rgb(t.selected)))
                            .child(div().w(px(38.0)).flex_none().text_color(rgb(t.faint)).child(number))
                            .child(div().text_color(rgb(color)).whitespace_nowrap().child(SharedString::from(format!("{mark}{}", l.text))))
                            .into_any_element();
                        let notes = review
                            .notes
                            .iter()
                            .filter(|n| n.path == path && n.line > 0 && n.line == if l.op == DiffOp::Delete { l.old } else { l.new })
                            .map(|n| {
                                let state = if n.sent() { "sent" } else { "not sent" };
                                div()
                                    .ml(px(38.0))
                                    .px(px(8.0))
                                    .py(px(2.0))
                                    .rounded(px(4.0))
                                    .bg(rgb(theme::mix(t.palette.bg, t.accent, 0.2)))
                                    .text_size(px(11.0))
                                    .text_color(rgb(t.text))
                                    .child(SharedString::from(format!("{} ({state})", n.text)))
                                    .into_any_element()
                            })
                            .collect::<Vec<_>>();
                        std::iter::once(row).chain(notes)
                    })
                    .collect()
            })
            .unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(div().flex_none().px(px(14.0)).py(px(6.0)).text_size(px(11.5)).text_color(rgb(t.muted)).border_b_1().border_color(rgb(line_color)).child(SharedString::from(format!("Changes by {}", review.name))))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(div().id("review-files").w(px(250.0)).flex_none().py(px(5.0)).overflow_y_scroll().border_r_1().border_color(rgb(line_color)).children(files))
                    .child(div().flex_1().min_w_0().p(px(8.0)).overflow_hidden().children(rows)),
            )
            .into_any_element()
    }

    /// The questions from ask-human, over the panes. They answer by click only, so
    /// that a key meant for a pane can never answer one.
    pub(super) fn ask_cards_view(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !matches!(self.overlay, Overlay::None) {
            return None;
        }
        let asks: Vec<InboxItem> = self.fleet.inbox.iter().filter(|i| i.answer() == Answer::Ask).take(3).cloned().collect();
        if asks.is_empty() {
            return None;
        }
        let t = &self.theme;
        let line = theme::mix(t.palette.bg, t.text, 0.2);
        let alert = t.agent("needs_input").unwrap_or(t.accent);
        let cards: Vec<_> = asks
            .iter()
            .map(|item| {
                let answers: Vec<_> = item
                    .options
                    .iter()
                    .enumerate()
                    .map(|(i, option)| {
                        let (target, answer) = (item.clone(), option.clone());
                        self.button(option.clone(), i == 0, move |this, _| this.answer_ask(&target, &answer), cx).into_any_element()
                    })
                    .collect();
                let later = item.clone();
                div()
                    .w(px(360.0))
                    .p(px(12.0))
                    .rounded(px(11.0))
                    .bg(rgb(t.palette.bg))
                    .border_1()
                    .border_color(rgb(alert))
                    .shadow_lg()
                    .child(div().text_size(px(11.0)).text_color(rgb(t.muted)).child(SharedString::from(if item.name.is_empty() || item.name == "ask-human" { "An agent asks".to_string() } else { format!("{} asks", item.name) })))
                    .child(div().pt(px(2.0)).pb(px(9.0)).text_size(px(13.0)).text_color(rgb(t.text)).child(SharedString::from(item.summary.clone())))
                    .child(
                        div().flex().flex_wrap().gap(px(6.0)).children(answers).child(
                            div()
                                .px(px(8.0))
                                .py(px(5.0))
                                .text_size(px(12.0))
                                .cursor_pointer()
                                .border_1()
                                .border_color(rgb(line))
                                .rounded(px(7.0))
                                .text_color(rgb(t.muted))
                                .child("Later")
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.act("mark-attention", json!({"id": later.id, "action": "snooze", "for_ms": 600_000}), "It will ask again in ten minutes.");
                                        cx.notify();
                                    }),
                                ),
                        ),
                    )
            })
            .collect();
        Some(div().absolute().top(px(46.0)).right(px(14.0)).flex().flex_col().gap(px(8.0)).children(cards).into_any_element())
    }
}
