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
