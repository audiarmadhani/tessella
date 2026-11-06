//! What the daemon says about agents, in the shapes the agents view shows: the Inbox,
//! mail, and the diff of a pane's work. All the text here was written by agents and
//! programs, so it is shown and never acted on.

use crate::rail::Agent;
use serde::Deserialize;
use serde_json::Value;

/// One thing waiting for the person.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct InboxItem {
    pub id: String,
    pub kind: String,
    pub host: String,
    pub session: String,
    pub window: String,
    pub harness: String,
    pub name: String,
    pub summary: String,
    pub options: Vec<String>,
    pub request_id: String,
    pub always_scope: String,
    /// Unix nanoseconds.
    pub since: i64,
    pub thread: i64,
    pub count: i64,
    pub stale: bool,
    pub deny_message: bool,
    pub plan_sha: String,
}

/// How an item gets answered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Answer {
    /// A question from ask-human, by one of its options.
    Ask,
    /// A prompt a hook holds, by reply-approval.
    Held,
    /// A prompt on a pane's screen, by respond.
    Screen,
    Mail,
    /// News: there is nothing to answer.
    Notice,
}

impl InboxItem {
    pub fn answer(&self) -> Answer {
        match self.kind.as_str() {
            "ask" => Answer::Ask,
            "approval" | "plan" if !self.request_id.is_empty() => Answer::Held,
            "approval" | "plan" | "question" => Answer::Screen,
            "mail" => Answer::Mail,
            _ => Answer::Notice,
        }
    }

    /// Whether it waits on the person, rather than telling them something.
    pub fn waits(&self) -> bool {
        !matches!(self.kind.as_str(), "finished" | "outbox" | "resume")
    }

    pub fn kind_label(&self) -> &'static str {
        match self.kind.as_str() {
            "approval" => "Approval",
            "plan" => "Plan",
            "ask" => "Question",
            "question" => "Waiting",
            "mail" => "Mail",
            "errored" => "Error",
            "resume" => "Can resume",
            "finished" => "Finished",
            "outbox" => "Sent",
            _ => "Item",
        }
    }
}

/// Reads the items of an attention event.
pub fn parse_items(attention: &Value) -> Vec<InboxItem> {
    attention["items"].as_array().into_iter().flatten().filter_map(|i| serde_json::from_value(i.clone()).ok()).collect()
}

/// One agent turn that changed files, which tuios saved as a checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    pub n: i64,
    pub label: String,
    /// Unix nanoseconds.
    pub at: i64,
}

/// The checkpoints of a pane, newest first. A safety one, which a restore makes, is a
/// place to go back to as well.
pub fn parse_turns(answer: &Value) -> Vec<Turn> {
    let mut turns: Vec<Turn> = answer["checkpoints"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let n = c["n"].as_i64()?;
            let label = c["label"].as_str().filter(|l| !l.is_empty()).or(c["kind"].as_str().filter(|k| *k == "safety")).unwrap_or_default();
            Some(Turn { n, label: label.to_string(), at: c["at"].as_i64().unwrap_or(0) })
        })
        .collect();
    turns.sort_by_key(|t| std::cmp::Reverse(t.n));
    turns
}

/// The turn next to the one on show. None is everything, which comes before the newest.
pub fn next_turn(turns: &[Turn], on_show: Option<i64>, older: bool) -> Option<i64> {
    let at = on_show.and_then(|n| turns.iter().position(|t| t.n == n));
    match (at, older) {
        (None, true) => turns.first().map(|t| t.n),
        (None, false) => None,
        (Some(0), false) => None,
        (Some(i), false) => Some(turns[i - 1].n),
        (Some(i), true) => Some(turns.get(i + 1).map_or(turns[i].n, |t| t.n)),
    }
}

/// A git worktree with its own session, on a machine.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Worktree {
    pub host: String,
    pub session: String,
    pub repo: String,
    pub branch: String,
    pub group: String,
    pub state: String,
    pub uncommitted: Option<i64>,
    pub ahead: Option<i64>,
    pub merged: bool,
}

pub fn parse_worktrees(host: &str, answer: &Value) -> Vec<Worktree> {
    let text = |v: &Value, k: &str| v[k].as_str().unwrap_or_default().to_string();
    let rows = answer["worktrees"].as_array().into_iter().flatten();
    // A main checkout is listed too, but no worktree action applies to it.
    rows.filter(|w| w["gone"] != true && w["main"] != true)
        .map(|w| Worktree {
            host: host.to_string(),
            session: text(w, "session"),
            repo: text(w, "repo"),
            branch: text(w, "branch"),
            group: text(w, "group"),
            state: text(w, "state"),
            uncommitted: w["changes"].as_i64(),
            ahead: w["ahead"].as_i64(),
            merged: w["merged"] == true,
        })
        .collect()
}

/// `3 claude,codex` as how many agents, and which, cycled across them.
pub fn parse_fan(text: &str) -> Option<(i64, Vec<String>)> {
    let (count, names) = text.trim().split_once(char::is_whitespace)?;
    let count: i64 = count.parse().ok().filter(|n| (1..=16).contains(n))?;
    let names: Vec<String> = names.split(',').map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).collect();
    (!names.is_empty()).then_some((count, names))
}

/// `seconds` as a short age: 5s, 12m, 3h, 2d.
pub fn age(seconds: i64) -> String {
    match seconds.max(0) {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

/// How a machine answered when asked for its agents.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HostState {
    pub host: String,
    /// It answered this time. If not, its agents are the last ones it gave.
    pub up: bool,
    pub reason: String,
}

/// The agents on other machines, from a list-host-agents answer, and how each
/// machine answered. This machine's are left out: the fleet event has them.
pub fn parse_host_agents(result: &Value) -> (Vec<Agent>, Vec<HostState>) {
    let text = |v: &Value, k: &str| v[k].as_str().unwrap_or_default().to_string();
    let mut agents = Vec::new();
    let mut hosts = Vec::new();
    for entry in result["hosts"].as_array().into_iter().flatten() {
        let host = text(entry, "host");
        if host.is_empty() || host == "local" {
            continue;
        }
        let up = entry["status"].as_str() == Some("up") && entry["stale"].as_bool() != Some(true);
        let reason = [text(entry, "error"), text(entry, "reason")].into_iter().find(|r| !r.is_empty()).unwrap_or_default();
        hosts.push(HostState { host: host.clone(), up, reason });
        // A row has its session, and an entry names one when they all share it.
        let shared = text(entry, "session");
        for a in entry["agents"].as_array().into_iter().flatten() {
            let session = Some(text(a, "session")).filter(|s| !s.is_empty()).unwrap_or_else(|| shared.clone());
            agents.push(Agent {
                host: host.clone(),
                window: text(a, "window_id"),
                name: text(a, "name"),
                state: text(a, "state"),
                message: text(a, "message"),
                session,
                needs_you: a["needs_you"].as_bool().unwrap_or(false),
                unread: a["unread"].as_i64().unwrap_or(0),
            });
        }
    }
    (agents, hosts)
}

/// Where an agent is, for a row: its session, and its machine if that is another.
pub fn place(agent: &Agent) -> String {
    if agent.host.is_empty() { agent.session.clone() } else { format!("{} · {}", agent.host, agent.session) }
}

/// The parameters that send mail from the person to an agent, on any machine.
pub fn mail_params(agent: &Agent, text: &str) -> Value {
    let mut params = serde_json::json!({"session": agent.session, "to": agent.window, "from": "human", "text": text});
    if !agent.host.is_empty() {
        params["host"] = serde_json::json!(agent.host);
    }
    params
}

/// What to tell the person about a send, from its answer.
pub fn mail_result(agent: &Agent, result: &Value) -> String {
    let flag = |k: &str| result[k].as_bool() == Some(true);
    if flag("queued") {
        format!("Waiting to send to {}: {} is not answering. It goes when it does.", agent.name, agent.host)
    } else if flag("held") {
        format!("{} holds mail from other machines, so it is waiting for you there.", agent.host)
    } else if agent.host.is_empty() {
        format!("Sent to {}.", agent.name)
    } else {
        format!("Sent to {} on {}.", agent.name, agent.host)
    }
}

/// One message from an agent's or the person's mailbox.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Mail {
    pub id: i64,
    pub from: String,
    pub from_label: String,
    pub to: String,
    pub to_label: String,
    pub subject: String,
    pub text: String,
    pub thread_id: i64,
}

impl Mail {
    pub fn sender(&self) -> &str {
        if self.from_label.is_empty() { &self.from } else { &self.from_label }
    }
}

pub fn parse_mail(result: &Value) -> Vec<Mail> {
    result["messages"].as_array().into_iter().flatten().filter_map(|m| serde_json::from_value(m.clone()).ok()).collect()
}

/// The threads in some mail, each by its id, newest first, with its messages in order.
pub fn threads(mail: &[Mail]) -> Vec<(i64, Vec<Mail>)> {
    let mut out: Vec<(i64, Vec<Mail>)> = Vec::new();
    for m in mail {
        let id = if m.thread_id != 0 { m.thread_id } else { m.id };
        match out.iter_mut().find(|(t, _)| *t == id) {
            Some((_, list)) => list.push(m.clone()),
            None => out.push((id, vec![m.clone()])),
        }
    }
    out.sort_by_key(|(_, list)| std::cmp::Reverse(list.iter().map(|m| m.id).max().unwrap_or(0)));
    out
}

/// A line of a diff, or the header of a hunk.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffLine {
    pub op: DiffOp,
    pub old: i64,
    pub new: i64,
    pub text: String,
    /// The header of the hunk it is in, which a note on it names.
    pub hunk: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DiffOp {
    Hunk,
    Context,
    Add,
    Delete,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiffFile {
    pub path: String,
    pub status: String,
    pub added: i64,
    pub removed: i64,
    pub binary: bool,
    pub lines: Vec<DiffLine>,
}

/// Reads the files of a review-diff answer into flat lines.
pub fn parse_diff(result: &Value) -> Vec<DiffFile> {
    let int = |v: &Value, k: &str| v[k].as_i64().unwrap_or(0);
    let text = |v: &Value, k: &str| v[k].as_str().unwrap_or_default().to_string();
    result["files"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| {
            let mut lines = Vec::new();
            for hunk in f["hunks"].as_array().into_iter().flatten() {
                let header = text(hunk, "header");
                lines.push(DiffLine { op: DiffOp::Hunk, old: 0, new: 0, text: header.clone(), hunk: header.clone() });
                for l in hunk["lines"].as_array().into_iter().flatten() {
                    let op = match l["op"].as_str() {
                        Some("add") => DiffOp::Add,
                        Some("delete") => DiffOp::Delete,
                        _ => DiffOp::Context,
                    };
                    lines.push(DiffLine { op, old: int(l, "old"), new: int(l, "new"), text: text(l, "text"), hunk: header.clone() });
                }
            }
            DiffFile {
                path: text(f, "path"),
                status: text(f, "status"),
                added: int(f, "added"),
                removed: int(f, "removed"),
                binary: f["binary"].as_bool().unwrap_or(false),
                lines,
            }
        })
        .collect()
}

/// One row of a side-by-side diff. Each number is a place in the file's lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SideRow {
    Hunk(usize),
    Pair { old: Option<usize>, new: Option<usize> },
}

impl SideRow {
    pub fn holds(self, line: usize) -> bool {
        match self {
            Self::Hunk(at) => at == line,
            Self::Pair { old, new } => old == Some(line) || new == Some(line),
        }
    }
}

/// Lines as rows for two columns. Removed lines sit beside the lines that replaced them,
/// in order, and any left over face a gap.
pub fn side_rows(lines: &[DiffLine]) -> Vec<SideRow> {
    let mut rows = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        match lines[at].op {
            DiffOp::Hunk => {
                rows.push(SideRow::Hunk(at));
                at += 1;
            }
            DiffOp::Context => {
                rows.push(SideRow::Pair { old: Some(at), new: Some(at) });
                at += 1;
            }
            DiffOp::Delete | DiffOp::Add => {
                let run = |from: usize, op: DiffOp| from + lines[from..].iter().take_while(|l| l.op == op).count();
                let adds = run(at, DiffOp::Delete);
                let end = run(adds, DiffOp::Add);
                let (removed, added) = (adds - at, end - adds);
                for i in 0..removed.max(added) {
                    rows.push(SideRow::Pair { old: (i < removed).then_some(at + i), new: (i < added).then_some(adds + i) });
                }
                at = end;
            }
        }
    }
    rows
}

/// Marks where the list of new files starts, after the diff, in what a far read prints.
pub const UNTRACKED_MARK: &str = "@@tessella-untracked@@";

/// The numbers in `@@ -1,2 +3,4 @@`.
fn hunk_start(header: &str) -> Option<(i64, i64)> {
    let mut parts = header.split_whitespace().skip(1);
    let number = |part: &str| part[1..].split(',').next()?.parse().ok();
    Some((number(parts.next()?)?, number(parts.next()?)?))
}

/// Plain `git diff` output as files, with new files named after a mark. A pane on
/// another machine has no tuios review, so its changes come back as this.
pub fn parse_unified(text: &str) -> Vec<DiffFile> {
    let (diff, untracked) = text.split_once(UNTRACKED_MARK).unwrap_or((text, ""));
    // The line break the mark is printed after is not part of the diff.
    let diff = diff.strip_suffix('\n').unwrap_or(diff);
    let mut files: Vec<DiffFile> = Vec::new();
    let (mut hunk, mut old, mut new) = (String::new(), 0, 0);
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            // `a/x b/x`, whose two halves match, which tells them apart with spaces in it.
            let half = (rest.len().saturating_sub(1)) / 2;
            let path = rest.get(half + 1..).unwrap_or(rest);
            files.push(DiffFile { path: path.strip_prefix("b/").unwrap_or(path).to_string(), status: "modified".to_string(), ..Default::default() });
            hunk.clear();
            continue;
        }
        let Some(file) = files.last_mut() else { continue };
        if line.starts_with("@@") {
            hunk = line.to_string();
            (old, new) = hunk_start(line).unwrap_or((0, 0));
            file.lines.push(DiffLine { op: DiffOp::Hunk, old: 0, new: 0, text: line.to_string(), hunk: hunk.clone() });
        } else if hunk.is_empty() {
            if line.starts_with("Binary files") {
                file.binary = true;
            } else if line.starts_with("new file") {
                file.status = "added".to_string();
            } else if line.starts_with("deleted file") {
                file.status = "deleted".to_string();
            }
        } else if let Some(text) = line.strip_prefix('+') {
            file.added += 1;
            file.lines.push(DiffLine { op: DiffOp::Add, old: 0, new, text: text.to_string(), hunk: hunk.clone() });
            new += 1;
        } else if let Some(text) = line.strip_prefix('-') {
            file.removed += 1;
            file.lines.push(DiffLine { op: DiffOp::Delete, old, new: 0, text: text.to_string(), hunk: hunk.clone() });
            old += 1;
        } else if !line.starts_with('\\') {
            file.lines.push(DiffLine { op: DiffOp::Context, old, new, text: line.strip_prefix(' ').unwrap_or(line).to_string(), hunk: hunk.clone() });
            old += 1;
            new += 1;
        }
    }
    files.extend(untracked.lines().filter(|l| !l.is_empty()).map(|path| DiffFile {
        path: path.to_string(),
        status: "added".to_string(),
        lines: vec![DiffLine { op: DiffOp::Hunk, old: 0, new: 0, text: "A new file, not added to git yet".to_string(), hunk: String::new() }],
        ..Default::default()
    }));
    files
}

/// A review note, as review-diff lists them.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Note {
    pub id: String,
    pub path: String,
    pub line: i64,
    pub text: String,
    pub sent_at: Value,
}

impl Note {
    pub fn sent(&self) -> bool {
        match &self.sent_at {
            Value::Null => false,
            Value::Number(n) => n.as_i64() != Some(0),
            Value::String(s) => !s.is_empty(),
            _ => true,
        }
    }
}

pub fn parse_notes(result: &Value) -> Vec<Note> {
    result["notes"].as_array().into_iter().flatten().filter_map(|n| serde_json::from_value(n.clone()).ok()).collect()
}

#[cfg(test)]
mod work_tests {
    use super::*;
    use serde_json::json;

    fn turns() -> Vec<Turn> {
        parse_turns(&json!({"checkpoints": [
            {"n": 1, "kind": "turn", "label": "first", "at": 5},
            {"n": 2, "kind": "safety", "at": 6},
            {"n": 3, "kind": "turn", "at": 7},
        ]}))
    }

    #[test]
    fn side_by_side_puts_a_change_beside_what_it_replaced() {
        let line = |op| DiffLine { op, old: 0, new: 0, text: String::new(), hunk: String::new() };
        let lines: Vec<DiffLine> = [DiffOp::Hunk, DiffOp::Context, DiffOp::Delete, DiffOp::Delete, DiffOp::Add, DiffOp::Context, DiffOp::Add, DiffOp::Delete].map(line).into();
        let pair = |old, new| SideRow::Pair { old, new };
        let rows = side_rows(&lines);
        assert_eq!(
            rows,
            [
                SideRow::Hunk(0),
                pair(Some(1), Some(1)),
                pair(Some(2), Some(4)),
                pair(Some(3), None),
                pair(Some(5), Some(5)),
                pair(None, Some(6)),
                pair(Some(7), None),
            ]
        );
        // Every line has one row to be found in, so the cursor never gets lost.
        for at in 0..lines.len() {
            assert!(rows.iter().any(|r| r.holds(at)), "line {at}");
        }
        assert!(side_rows(&[]).is_empty());
    }

    #[test]
    fn plain_git_diff_reads_as_files() {
        let text = "diff --git a/my file.txt b/my file.txt\nindex 1..2 100644\n--- a/my file.txt\n+++ b/my file.txt\n@@ -2,2 +2,3 @@ fn x\n one\n-two\n+TWO\n+three\ndiff --git a/g.png b/g.png\nBinary files a/g.png and b/g.png differ\n\n@@tessella-untracked@@\nnew.txt\n";
        let files = parse_unified(text);
        assert_eq!(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["my file.txt", "g.png", "new.txt"]);
        let f = &files[0];
        assert_eq!((f.added, f.removed, f.lines.len()), (2, 1, 5));
        assert_eq!((f.lines[1].old, f.lines[1].new), (2, 2));
        assert_eq!((f.lines[2].op, f.lines[2].old), (DiffOp::Delete, 3));
        assert_eq!((f.lines[3].op, f.lines[3].new), (DiffOp::Add, 3));
        assert!(files[1].binary);
        assert_eq!(files[2].status, "added");
        assert!(parse_unified("").is_empty());
    }

    #[test]
    fn the_newest_turn_comes_first() {
        let t = turns();
        assert_eq!(t.iter().map(|t| t.n).collect::<Vec<_>>(), [3, 2, 1]);
        assert_eq!((t[1].label.as_str(), t[2].label.as_str()), ("safety", "first"));
    }

    #[test]
    fn stepping_goes_from_everything_through_each_turn_and_back() {
        let t = turns();
        assert_eq!(next_turn(&t, None, true), Some(3));
        assert_eq!(next_turn(&t, Some(3), true), Some(2));
        assert_eq!(next_turn(&t, Some(1), true), Some(1));
        assert_eq!(next_turn(&t, Some(2), false), Some(3));
        assert_eq!(next_turn(&t, Some(3), false), None);
        assert_eq!(next_turn(&[], None, true), None);
    }

    #[test]
    fn a_fan_is_a_count_and_its_agents() {
        assert_eq!(parse_fan("3 claude"), Some((3, vec!["claude".to_string()])));
        assert_eq!(parse_fan(" 2 claude, codex --model o5 "), Some((2, vec!["claude".to_string(), "codex --model o5".to_string()])));
        assert_eq!(parse_fan("claude"), None);
        assert_eq!(parse_fan("99 claude"), None);
        assert_eq!(parse_fan("3 "), None);
    }

    #[test]
    fn a_removed_worktree_is_left_out() {
        let answer = json!({"worktrees": [
            {"session": "a", "repo": "r", "branch": "b", "changes": 2, "ahead": 1, "merged": false},
            {"session": "gone", "gone": true},
            {"session": "checkout", "main": true},
        ]});
        let rows = parse_worktrees("build", &answer);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].host.as_str(), rows[0].uncommitted, rows[0].ahead), ("build", Some(2), Some(1)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(kind: &str, request: &str) -> InboxItem {
        InboxItem { kind: kind.into(), request_id: request.into(), ..Default::default() }
    }

    #[test]
    fn an_item_is_answered_the_way_its_kind_takes() {
        assert_eq!(item("ask", "r").answer(), Answer::Ask);
        assert_eq!(item("approval", "r").answer(), Answer::Held);
        assert_eq!(item("plan", "r").answer(), Answer::Held);
        // A prompt no hook holds is on the pane's screen.
        assert_eq!(item("approval", "").answer(), Answer::Screen);
        assert_eq!(item("question", "").answer(), Answer::Screen);
        assert_eq!(item("mail", "").answer(), Answer::Mail);
        assert_eq!(item("finished", "").answer(), Answer::Notice);
    }

    #[test]
    fn items_read_with_their_answers_and_unknown_fields_are_ignored() {
        let v = json!({"items": [{"id": "1", "kind": "ask", "session": "s", "summary": "Ship?", "options": ["yes", "no"], "request_id": "r", "new_field": 3}]});
        let items = parse_items(&v);
        assert_eq!(items[0].options, ["yes", "no"]);
        assert!(items[0].waits());
        assert!(!item("finished", "").waits());
    }

    #[test]
    fn agents_on_other_machines_keep_their_machine_and_leave_this_one_out() {
        let v = json!({"hosts": [
            {"host": "local", "session": "s", "status": "up", "agents": [{"window_id": "here", "name": "local one", "state": "working"}]},
            {"host": "build", "session": "s", "status": "up", "agents": [{"window_id": "w1", "name": "there", "state": "done", "session": "api"}, {"window_id": "w2", "name": "shared", "state": "idle"}]},
            {"host": "down", "status": "lost", "error": "no route", "stale": true, "agents": [{"window_id": "w3", "name": "old", "state": "idle", "session": "x"}]}
        ]});
        let (agents, hosts) = parse_host_agents(&v);
        assert_eq!(agents.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["there", "shared", "old"]);
        assert_eq!((agents[0].host.as_str(), agents[0].session.as_str()), ("build", "api"));
        // A row with no session of its own has the one its machine names.
        assert_eq!(agents[1].session, "s");
        assert_eq!(hosts[0], HostState { host: "build".into(), up: true, reason: String::new() });
        assert!(!hosts[1].up);
        assert_eq!(hosts[1].reason, "no route");
    }

    fn agent(host: &str) -> Agent {
        Agent { host: host.into(), window: "w".into(), session: "s".into(), name: "n".into(), ..Default::default() }
    }

    #[test]
    fn mail_names_the_machine_only_when_it_is_another() {
        let local = mail_params(&agent(""), "hi");
        assert_eq!(local["to"], "w");
        assert!(local.get("host").is_none());
        let far = mail_params(&agent("build"), "hi");
        assert_eq!((far["host"].as_str(), far["session"].as_str(), far["text"].as_str()), (Some("build"), Some("s"), Some("hi")));
        // The token that proves it is the person is the bridge's to add, never this.
        assert!(far.get("human_nonce").is_none());
    }

    #[test]
    fn the_person_hears_whether_mail_went_or_is_waiting() {
        let far = agent("build");
        assert_eq!(mail_result(&far, &json!({"message_id": 3})), "Sent to n on build.");
        assert!(mail_result(&far, &json!({"queued": true})).contains("Waiting"));
        assert!(mail_result(&far, &json!({"held": true})).contains("holds"));
        assert_eq!(mail_result(&agent(""), &json!({})), "Sent to n.");
        assert_eq!(place(&far), "build · s");
        assert_eq!(place(&agent("")), "s");
    }

    #[test]
    fn ages_read_shortly() {
        assert_eq!(age(5), "5s");
        assert_eq!(age(120), "2m");
        assert_eq!(age(7200), "2h");
        assert_eq!(age(172_800), "2d");
        assert_eq!(age(-4), "0s");
    }

    #[test]
    fn threads_gather_their_messages_with_the_newest_thread_first() {
        let m = |id, thread| Mail { id, thread_id: thread, ..Default::default() };
        let t = threads(&[m(1, 1), m(2, 2), m(3, 1), m(4, 0)]);
        assert_eq!(t.iter().map(|x| x.0).collect::<Vec<_>>(), [4, 1, 2]);
        assert_eq!(t[1].1.len(), 2);
    }

    #[test]
    fn a_diff_flattens_to_lines_under_their_hunk() {
        let v = json!({"files": [{"path": "a.rs", "status": "M", "added": 1, "removed": 1, "hunks": [
            {"header": "@@ -1,2 +1,2 @@", "lines": [
                {"op": "context", "old": 1, "new": 1, "text": "same"},
                {"op": "delete", "old": 2, "text": "old"},
                {"op": "add", "new": 2, "text": "new"}
            ]}
        ]}]});
        let files = parse_diff(&v);
        let ops: Vec<_> = files[0].lines.iter().map(|l| l.op).collect();
        assert_eq!(ops, [DiffOp::Hunk, DiffOp::Context, DiffOp::Delete, DiffOp::Add]);
        assert_eq!(files[0].lines[3].new, 2);
        assert_eq!(files[0].lines[3].hunk, "@@ -1,2 +1,2 @@");
    }

    #[test]
    fn a_note_knows_whether_it_was_sent() {
        let notes = parse_notes(&json!({"notes": [{"id": "a", "path": "x", "line": 3, "text": "t"}, {"id": "b", "sent_at": 5}]}));
        assert!(!notes[0].sent());
        assert!(notes[1].sent());
    }
}
