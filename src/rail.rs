//! The right sidebar, modelled on tuios's rail: sessions, terminals, files, agents.
//!
//! Everything here is plain data, built fresh from what the bridge last sent, so
//! the view stays a function of it and the navigation can be tested without a window.

use crate::model::State;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Sessions,
    Terminals,
    Files,
    Agents,
}

impl Section {
    /// In tuios's order. Agents sit apart, at the bottom of the rail.
    pub const ALL: [Section; 4] = [Section::Sessions, Section::Terminals, Section::Files, Section::Agents];

    pub fn title(self) -> &'static str {
        match self {
            Section::Sessions => "sessions",
            Section::Terminals => "terminals",
            Section::Files => "files",
            Section::Agents => "agents",
        }
    }
}

/// Which agents the agents section lists, as tuios's `all · you · @`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    /// Waiting on the person.
    You,
    /// Holding unread mail.
    Mail,
}

impl Filter {
    pub fn next(self) -> Self {
        match self {
            Filter::All => Filter::You,
            Filter::You => Filter::Mail,
            Filter::Mail => Filter::All,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Agent {
    /// The machine it runs on, by its name in [hosts]. Empty for this one.
    pub host: String,
    pub window: String,
    pub name: String,
    pub state: String,
    pub message: String,
    pub session: String,
    pub needs_you: bool,
    pub unread: i64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SessionRow {
    pub name: String,
    pub panes: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attention {
    pub kind: String,
    pub session: String,
    pub window: String,
    pub name: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub dir: bool,
}

/// What the daemon last said about everything outside this session.
#[derive(Debug, Clone, Default)]
pub struct Fleet {
    pub sessions: Vec<SessionRow>,
    pub agents: Vec<Agent>,
    pub attention: Vec<Attention>,
    /// The Inbox, whole, for the agents view.
    pub inbox: Vec<crate::agents::InboxItem>,
}

fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

impl Fleet {
    pub fn apply_fleet(&mut self, fleet: &Value) {
        self.sessions = fleet["sessions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| SessionRow { name: text(s, "name"), panes: s["window_count"].as_i64().unwrap_or(0) })
            .collect();
        // list-agents answers either a bare list or an object holding one.
        let agents = if fleet["agents"].is_array() { &fleet["agents"] } else { &fleet["agents"]["agents"] };
        self.agents = agents
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| Agent {
                host: String::new(),
                window: text(a, "window_id"),
                name: text(a, "name"),
                state: text(a, "state"),
                message: text(a, "message"),
                session: text(a, "session"),
                needs_you: a["needs_you"].as_bool().unwrap_or(false),
                unread: a["unread"].as_i64().unwrap_or(0),
            })
            .collect();
    }

    pub fn apply_attention(&mut self, attention: &Value) {
        self.inbox = crate::agents::parse_items(attention);
        self.attention = attention["items"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|i| Attention {
                kind: text(i, "kind"),
                session: text(i, "session"),
                window: text(i, "window"),
                name: text(i, "name"),
                summary: text(i, "summary"),
            })
            .collect();
    }

    /// The items waiting on the person. A finished turn is news, not a request.
    pub fn waiting(&self) -> Vec<&Attention> {
        self.attention.iter().filter(|a| a.kind != "finished").collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Session { name: String, current: bool, panes: i64, branch: String, state: String },
    Terminal { id: String, label: String, host: String, focused: bool },
    File(Entry),
    Agent { window: String, session: String, name: String, state: String, note: String, focused: bool },
}

#[derive(Debug, Clone)]
pub struct Group {
    pub section: Section,
    pub rows: Vec<Row>,
}

fn is_active(state: &str) -> bool {
    matches!(state, "working" | "needs_input" | "idle" | "done" | "errored")
}

/// The state that speaks loudest for a whole session.
fn loudest<'a>(states: impl Iterator<Item = &'a str>) -> String {
    let rank = |s: &str| match s {
        "needs_input" => 5,
        "errored" => 4,
        "working" => 3,
        "done" => 2,
        "idle" => 1,
        _ => 0,
    };
    states.max_by_key(|s| rank(s)).filter(|s| rank(s) > 0).unwrap_or_default().to_string()
}

pub struct Inputs<'a> {
    pub state: &'a State,
    pub fleet: &'a Fleet,
    pub files: &'a [Entry],
    pub filter: Filter,
    pub collapsed: &'a [Section],
}

pub fn build(i: &Inputs) -> Vec<Group> {
    let current = &i.state.session;
    let branch = i.state.windows.iter().map(|w| w.branch.as_str()).find(|b| !b.is_empty()).unwrap_or_default();
    let sessions = i
        .fleet
        .sessions
        .iter()
        .map(|s| Row::Session {
            name: s.name.clone(),
            current: &s.name == current,
            panes: s.panes,
            branch: if &s.name == current { branch.to_string() } else { String::new() },
            state: loudest(i.fleet.agents.iter().filter(|a| a.session == s.name).map(|a| a.state.as_str())),
        })
        .collect();
    let terminals = i
        .state
        .windows
        .iter()
        .filter(|w| !w.popup && !w.has_agent())
        .map(|w| Row::Terminal {
            id: w.id.clone(),
            label: w.label(),
            host: w.host.clone(),
            focused: w.id == i.state.focused,
        })
        .collect();
    let files = i.files.iter().cloned().map(Row::File).collect();
    let agents = i
        .fleet
        .agents
        .iter()
        .filter(|a| is_active(&a.state))
        .filter(|a| match i.filter {
            Filter::All => true,
            Filter::You => a.needs_you || a.state == "needs_input",
            Filter::Mail => a.unread > 0,
        })
        .map(|a| {
            let kind = i.fleet.attention.iter().find(|x| x.window == a.window).map(|x| x.kind.as_str());
            let note = match kind {
                Some(k) if !a.message.is_empty() => format!("{k} · {}", a.message),
                Some(k) => k.to_string(),
                None => a.message.clone(),
            };
            Row::Agent {
                window: a.window.clone(),
                session: a.session.clone(),
                name: a.name.clone(),
                state: a.state.clone(),
                note,
                focused: a.window == i.state.focused,
            }
        })
        .collect();
    let rows_of = |section: Section, rows: Vec<Row>| Group {
        section,
        rows: if i.collapsed.contains(&section) { Vec::new() } else { rows },
    };
    vec![
        rows_of(Section::Sessions, sessions),
        rows_of(Section::Terminals, terminals),
        rows_of(Section::Files, files),
        rows_of(Section::Agents, agents),
    ]
}

pub fn row_count(groups: &[Group]) -> usize {
    groups.iter().map(|g| g.rows.len()).sum()
}

pub fn row_at(groups: &[Group], cursor: usize) -> Option<(Section, &Row)> {
    let mut left = cursor;
    for g in groups {
        if left < g.rows.len() {
            return Some((g.section, &g.rows[left]));
        }
        left -= g.rows.len();
    }
    None
}

/// The cursor at the first row of the next section that has rows, wrapping.
pub fn next_section(groups: &[Group], cursor: usize, by: i32) -> usize {
    let mut starts = Vec::new();
    let mut at = 0;
    for g in groups {
        if !g.rows.is_empty() {
            starts.push(at);
        }
        at += g.rows.len();
    }
    if starts.is_empty() {
        return 0;
    }
    let here = starts.iter().rposition(|s| *s <= cursor).unwrap_or(0) as i32;
    starts[(here + by).rem_euclid(starts.len() as i32) as usize]
}

/// The cursor at the first row of this section, if it has rows.
pub fn section_start(groups: &[Group], section: Section) -> Option<usize> {
    let mut at = 0;
    for g in groups {
        if g.section == section {
            return (!g.rows.is_empty()).then_some(at);
        }
        at += g.rows.len();
    }
    None
}

/// The cursor at the Nth row, from 1, of the section the cursor is in.
pub fn jump(groups: &[Group], cursor: usize, n: usize) -> usize {
    let mut at = 0;
    for g in groups {
        if cursor < at + g.rows.len() {
            return if n >= 1 && n <= g.rows.len() { at + n - 1 } else { cursor };
        }
        at += g.rows.len();
    }
    cursor
}

/// A folder's entries: its parent, then folders, then files, each by name.
pub fn read_dir(dir: &Path) -> Vec<Entry> {
    let mut found: Vec<Entry> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| {
            let path = e.path();
            Entry { name: e.file_name().to_string_lossy().into_owned(), dir: path.is_dir(), path }
        })
        .collect();
    found.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    let mut out = Vec::with_capacity(found.len() + 1);
    if let Some(parent) = dir.parent() {
        out.push(Entry { path: parent.to_path_buf(), name: "..".to_string(), dir: true });
    }
    out.extend(found);
    out
}

/// `/a/b/c/d` as `…/c/d`, for a header with little room.
pub fn short_path(path: &Path) -> String {
    let parts: Vec<_> = path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    match parts.len() {
        0..=2 => path.display().to_string(),
        n => format!("…/{}/{}", parts[n - 2], parts[n - 1]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Win;
    use serde_json::json;

    fn fleet() -> Fleet {
        let mut f = Fleet::default();
        f.apply_fleet(&json!({
            "sessions": [{"name": "work", "window_count": 3}, {"name": "other", "window_count": 1}],
            "agents": [
                {"window_id": "a", "name": "review-api", "state": "needs_input", "message": "approve", "session": "work", "needs_you": true},
                {"window_id": "b", "name": "Refactor", "state": "working", "message": "reading", "session": "work"},
                {"window_id": "c", "name": "Shell", "state": "none", "session": "work"},
                {"window_id": "d", "name": "Other", "state": "done", "session": "other", "unread": 2}
            ]
        }));
        f.apply_attention(&json!({"items": [
            {"kind": "approval", "session": "work", "window": "a", "name": "review-api", "summary": "approve"},
            {"kind": "finished", "session": "other", "window": "d", "name": "Other", "summary": ""}
        ]}));
        f
    }

    fn state() -> State {
        let win = |id: &str, agent: &str| Win { id: id.into(), title: id.into(), agent: agent.into(), branch: "main".into(), ..Win::default() };
        State {
            session: "work".into(),
            focused: "c".into(),
            windows: vec![win("a", "needs_input"), win("b", "working"), win("c", "none"), Win { popup: true, ..win("p", "") }],
            ..State::default()
        }
    }

    fn files() -> Vec<Entry> {
        let e = |n: &str, dir| Entry { path: PathBuf::from(n), name: n.into(), dir };
        vec![e("..", true), e("src", true), e("Cargo.toml", false)]
    }

    fn groups(filter: Filter, collapsed: &[Section]) -> Vec<Group> {
        let (s, f, fs) = (state(), fleet(), files());
        build(&Inputs { state: &s, fleet: &f, files: &fs, filter, collapsed })
    }

    #[test]
    fn a_session_shows_its_loudest_agent_and_the_current_one_its_branch() {
        let g = groups(Filter::All, &[]);
        assert_eq!(
            g[0].rows[0],
            Row::Session { name: "work".into(), current: true, panes: 3, branch: "main".into(), state: "needs_input".into() }
        );
        assert_eq!(
            g[0].rows[1],
            Row::Session { name: "other".into(), current: false, panes: 1, branch: String::new(), state: "done".into() }
        );
    }

    #[test]
    fn terminals_are_the_panes_with_no_agent_and_no_popups() {
        let g = groups(Filter::All, &[]);
        assert_eq!(g[1].rows.len(), 1);
        assert!(matches!(&g[1].rows[0], Row::Terminal { id, focused: true, .. } if id == "c"));
    }

    #[test]
    fn agents_span_every_session_and_a_waiting_one_says_why() {
        let g = groups(Filter::All, &[]);
        assert_eq!(g[3].rows.len(), 3);
        let Row::Agent { note, .. } = &g[3].rows[0] else { panic!() };
        assert_eq!(note, "approval · approve");
    }

    #[test]
    fn the_filter_keeps_waiting_agents_or_those_with_mail() {
        assert_eq!(groups(Filter::You, &[])[3].rows.len(), 1);
        let mail = groups(Filter::Mail, &[]);
        assert!(matches!(&mail[3].rows[0], Row::Agent { name, .. } if name == "Other"));
        assert_eq!(Filter::Mail.next(), Filter::All);
    }

    #[test]
    fn a_collapsed_section_has_no_rows() {
        let g = groups(Filter::All, &[Section::Files]);
        assert!(g[2].rows.is_empty());
        assert_eq!(row_count(&g), 2 + 1 + 3);
    }

    #[test]
    fn the_cursor_finds_rows_across_sections() {
        let g = groups(Filter::All, &[]);
        assert_eq!(row_at(&g, 0).unwrap().0, Section::Sessions);
        assert_eq!(row_at(&g, 2).unwrap().0, Section::Terminals);
        assert_eq!(row_at(&g, 3).unwrap().0, Section::Files);
        assert_eq!(row_at(&g, 6).unwrap().0, Section::Agents);
        assert!(row_at(&g, 99).is_none());
    }

    #[test]
    fn tab_moves_between_sections_and_skips_empty_ones() {
        let g = groups(Filter::All, &[Section::Terminals]);
        assert_eq!(next_section(&g, 0, 1), 2);
        assert_eq!(next_section(&g, 2, 1), 5);
        assert_eq!(next_section(&g, 5, 1), 0);
        assert_eq!(next_section(&g, 0, -1), 5);
    }

    #[test]
    fn a_number_jumps_within_the_section_and_stays_put_past_its_end() {
        let g = groups(Filter::All, &[]);
        assert_eq!(jump(&g, 4, 3), 5);
        assert_eq!(jump(&g, 4, 9), 4);
        assert_eq!(section_start(&g, Section::Agents), Some(6));
    }

    #[test]
    fn a_folder_lists_its_parent_then_folders_then_files() {
        let dir = std::env::temp_dir().join(format!("tessella-rail-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("zdir")).unwrap();
        std::fs::write(dir.join("afile"), "").unwrap();
        let names: Vec<_> = read_dir(&dir).into_iter().map(|e| e.name).collect();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(names, ["..", "zdir", "afile"]);
    }

    #[test]
    fn a_long_path_shortens_to_its_last_two_folders() {
        assert_eq!(short_path(Path::new("/Users/a/Developer/tessella")), "…/Developer/tessella");
        assert_eq!(short_path(Path::new("/tmp")), "/tmp");
    }
}
