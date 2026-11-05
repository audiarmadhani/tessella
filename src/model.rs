//! The parts of the bridge's JSON that this client reads.

use serde::Deserialize;

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct State {
    pub session: String,
    pub workspace: i32,
    pub workspace_names: std::collections::HashMap<i32, String>,
    pub num_workspaces: i32,
    pub occupied: Option<Vec<i32>>,
    pub focused: String,
    pub windows: Vec<Win>,
    /// The size the daemon gives the session, which every client shares.
    pub session_size: Size,
    /// This client's own size, as the bridge has it.
    pub client_size: Size,
    /// Each workspace's split tree, by workspace number, while the layout is one.
    pub trees: std::collections::HashMap<String, Tree>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct Tree {
    pub bounds: Rect,
    /// The cells kept between neighbouring panes, which can be none.
    pub gap: i32,
    pub root: Option<Node>,
}

/// One node of a split tree: a split of two others, or a pane.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct Node {
    pub id: u64,
    /// "x" puts `a` left of `b`, "y" puts `a` above `b`. Empty on a pane.
    pub axis: String,
    pub rect: Rect,
    pub a: Option<Box<Node>>,
    pub b: Option<Box<Node>>,
}

/// A split line under a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitHit {
    pub id: u64,
    pub along_x: bool,
    pub rect: Rect,
}

/// The split line within `reach` cells of a point, with the innermost split winning.
pub fn split_at(node: &Node, (col, row): (f32, f32), reach: (f32, f32), gap: i32) -> Option<SplitHit> {
    let (a, b) = (node.a.as_deref()?, node.b.as_deref()?);
    if let Some(hit) = split_at(a, (col, row), reach, gap).or_else(|| split_at(b, (col, row), reach, gap)) {
        return Some(hit);
    }
    let r = node.rect;
    let half_gap = gap as f32 / 2.0;
    let along_x = node.axis == "x";
    let near = if along_x {
        let line = (a.rect.x + a.rect.w) as f32 + half_gap;
        (col - line).abs() <= reach.0 + half_gap && row >= r.y as f32 && row < (r.y + r.h) as f32
    } else {
        let line = (a.rect.y + a.rect.h) as f32 + half_gap;
        (row - line).abs() <= reach.1 + half_gap && col >= r.x as f32 && col < (r.x + r.w) as f32
    };
    near.then_some(SplitHit { id: node.id, along_x, rect: r })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct Win {
    pub id: String,
    pub pty: String,
    pub title: String,
    pub name: String,
    pub workspace: i32,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub z: i32,
    /// terminal, or view, web or app for a pane a GUI draws itself.
    pub kind: String,
    pub uri: String,
    pub minimized: bool,
    pub floating: bool,
    pub zoomed: bool,
    pub agent: String,
    pub agent_message: String,
    pub cwd: String,
    pub branch: String,
    pub foreground: String,
    /// The machine the pane's program runs on. Empty for this one.
    pub host: String,
    /// A floating pane `tuios popup` opened. It closes with its program.
    pub popup: bool,
}

impl Win {
    /// The Markdown file this pane shows, if it is such a pane.
    pub fn markdown_path(&self) -> Option<std::path::PathBuf> {
        (self.kind == "view").then(|| crate::markdown::markdown_path(&self.uri)).flatten()
    }

    /// A short name for the pane. tuios's default title is a pane id, which
    /// says nothing, so the running program or the folder stands in for it.
    pub fn label(&self) -> String {
        if !self.name.is_empty() {
            return self.name.clone();
        }
        let generic = self.title.is_empty() || self.title.starts_with("Terminal ");
        if !generic {
            return self.title.clone();
        }
        if !self.foreground.is_empty() {
            return self.foreground.clone();
        }
        let folder = self.cwd.rsplit('/').find(|p| !p.is_empty()).unwrap_or_default();
        if folder.is_empty() { "Shell".to_string() } else { folder.to_string() }
    }

    /// tuios also reports `none` and `unknown`, and the latter is what an agent
    /// that stopped reporting decays to. Neither says anything is running.
    pub fn has_agent(&self) -> bool {
        matches!(self.agent.as_str(), "working" | "needs_input" | "idle" | "done" | "errored")
    }

    /// An agent's last note. A pane whose agent has gone keeps its old message,
    /// which no longer describes anything, so it shows no note.
    pub fn note(&self) -> &str {
        if self.has_agent() { &self.agent_message } else { "" }
    }

}

/// How to carry out a tuios action the bridge may not run itself.
#[derive(Debug, PartialEq)]
pub enum Resolved {
    /// The bridge runs this action, under this name.
    Action(String),
    Workspace(i32),
    /// Renumber the workspaces into this order.
    Order(Vec<i32>),
    /// Valid, with nothing to do here, such as a step with no other workspace.
    Nothing,
}

impl State {
    /// The box the panes are laid out in. tuios can keep part of the session for
    /// chrome, so this can be smaller than the session, and start inside it.
    pub fn layout_box(&self) -> Rect {
        self.trees
            .get(&self.workspace.to_string())
            .map(|t| t.bounds)
            .or_else(|| self.trees.values().next().map(|t| t.bounds))
            .filter(|b| b.w > 0 && b.h > 0)
            .unwrap_or(Rect { x: 0, y: 0, w: self.session_size.cols as i32, h: self.session_size.rows as i32 })
    }

    /// The cells between neighbouring panes on this workspace, if anything on it says.
    /// A workspace with one pane or none does not, and a guess would change as the
    /// person moves between workspaces, and resize every pane each time.
    pub fn layout_gap(&self) -> Option<i32> {
        if let Some(tree) = self.trees.get(&self.workspace.to_string()) {
            return Some(tree.gap.max(0));
        }
        // A layout with no tree, such as the scrolling one, shows its gap in how far
        // neighbouring panes sit apart.
        let panes: Vec<&Win> = self.windows.iter().filter(|w| w.workspace == self.workspace && !w.minimized && !w.popup && !w.floating).collect();
        gap_between(&panes)
    }

    /// The columns and rows tuios keeps out of the box for chrome this client does
    /// not draw. None while another client sets the session's size, since that
    /// client's chrome is its own to ask for, and on a workspace with no split tree,
    /// which has no box to measure: it says nothing about the chrome.
    pub fn unused_chrome(&self) -> Option<(u16, u16)> {
        let (s, c) = (self.session_size, self.client_size);
        if s.cols == 0 || s != c {
            return None;
        }
        let b = self.trees.get(&self.workspace.to_string())?.bounds;
        if b.w <= 0 || b.h <= 0 {
            return None;
        }
        Some(((s.cols as i32 - b.w).max(0) as u16, (s.rows as i32 - b.h).max(0) as u16))
    }

    fn is_occupied(&self, n: i32) -> bool {
        self.occupied.as_ref().is_some_and(|o| o.contains(&n))
    }

    /// As tuios steps: to the next workspace that has panes, wrapping.
    fn step_workspace(&self, step: i32) -> Resolved {
        let n = self.num_workspaces.max(1);
        for i in 1..n {
            let at = (self.workspace - 1 + step * i).rem_euclid(n) + 1;
            if self.is_occupied(at) {
                return Resolved::Workspace(at);
            }
        }
        Resolved::Nothing
    }

    /// As tuios moves: swap with the neighbour, which has to hold panes.
    fn move_workspace(&self, delta: i32) -> Resolved {
        let (from, to) = (self.workspace, self.workspace + delta);
        if to < 1 || to > self.num_workspaces || !self.is_occupied(to) {
            return Resolved::Nothing;
        }
        Resolved::Order(if delta < 0 { vec![from, to] } else { vec![to, from] })
    }

    /// A real client resolves some actions itself, so the bridge refuses them.
    /// These turn into what it does accept. tuios's resize_pane moves a split by
    /// its own step, and the bridge's nearest action moves it by 4 columns.
    pub fn resolve(&self, action: &str) -> Resolved {
        match action {
            "next_workspace" => self.step_workspace(1),
            "prev_workspace" => self.step_workspace(-1),
            "move_workspace_left" => self.move_workspace(-1),
            "move_workspace_right" => self.move_workspace(1),
            a if a.starts_with("resize_pane_left") => Resolved::Action("resize_master_shrink".into()),
            a if a.starts_with("resize_pane_right") => Resolved::Action("resize_master_grow".into()),
            a if a.starts_with("resize_pane_up") => Resolved::Action("resize_height_shrink".into()),
            a if a.starts_with("resize_pane_down") => Resolved::Action("resize_height_grow".into()),
            other => Resolved::Action(other.to_string()),
        }
    }
}

/// The window whose slot holds this cell, topmost first. A slot is the pane's
/// cells plus the gap after it, and slots tile the stage.
pub fn window_at(windows: &[Win], workspace: i32, col: i32, row: i32, gap: i32) -> Option<&Win> {
    windows
        .iter()
        .filter(|w| w.workspace == workspace && !w.minimized)
        .filter(|w| col >= w.x && col < w.x + w.w + gap && row >= w.y && row < w.y + w.h + gap)
        .max_by_key(|w| w.z)
}

/// The font size one zoom step away. A step is one device pixel on the cell's width,
/// and the size is exactly what fills that width: otherwise a step can leave the cell
/// as it was on a 1x screen, so steps feel uneven. `advance` is a cell's width at size 1.
pub fn zoom_size(size: f32, advance: f32, scale: f32, step: f32) -> f32 {
    ((advance * size * scale).round() + step) / (advance * scale)
}

/// The cells between panes that sit side by side, or one above the other.
fn gap_between(panes: &[&Win]) -> Option<i32> {
    let mut gaps = Vec::new();
    for a in panes {
        for b in panes {
            let beside = a.y == b.y && b.x >= a.x + a.w;
            let below = a.x == b.x && b.y >= a.y + a.h;
            if beside {
                gaps.push(b.x - (a.x + a.w));
            }
            if below {
                gaps.push(b.y - (a.y + a.h));
            }
        }
    }
    gaps.into_iter().min()
}

/// Where a split action puts its new pane: right of the focused one, or below it.
pub fn split_side(action: &str) -> Option<&'static str> {
    match action {
        "split_vertical" | "prefix_split_vertical" => Some("right"),
        "split_horizontal" | "prefix_split_horizontal" => Some("bottom"),
        _ => None,
    }
}

/// The cells a stage of this size holds. Panes are one gap apart, and the layout
/// counts a gap after the last pane too, so that much comes off.
pub fn stage_grid((w, h): (f32, f32), (cell_w, cell_h): (f32, f32), gap: i32) -> (u16, u16) {
    let gap = gap.max(0) as f32;
    (((w / cell_w).floor().max(20.0) - gap) as u16, ((h / cell_h).floor().max(6.0) - gap) as u16)
}

/// How many cells tuios should keep out of a slot of `n` cells. `pad` points of
/// chrome have to fit, the slot has `gap` cells more than the pane, and tuios gives
/// `n + 1` less what it is told, and less `border` for itself.
pub fn reserved_cells(pad: f32, cell: f32, gap: i32, border: i32) -> i32 {
    (pad / cell).ceil() as i32 + 1 - gap - border
}

/// The cells tuios takes from a pane beyond what it was told to keep, if every pane
/// shows the same. Grids are `((slot width, slot height), (cols, rows))`. Panes that
/// disagree are still settling, or are not laid out alike.
pub fn agreed_border(grids: impl Iterator<Item = ((i32, i32), (i32, i32))>, reserved: (i32, i32)) -> Option<(i32, i32)> {
    let mut seen = None;
    for ((w, h), (cols, rows)) in grids {
        let border = (w + 1 - reserved.0 - cols, h + 1 - reserved.1 - rows);
        if !(0..=3).contains(&border.0) || !(0..=3).contains(&border.1) || seen.is_some_and(|s| s != border) {
            return None;
        }
        seen = Some(border);
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_split_line_is_found_near_it_and_the_inner_one_wins() {
        let leaf = |x, y, w, h| Node { rect: Rect { x, y, w, h }, ..Default::default() };
        let right = Node { id: 2, axis: "y".into(), rect: Rect { x: 51, y: 0, w: 49, h: 40 }, a: Some(Box::new(leaf(51, 0, 49, 19))), b: Some(Box::new(leaf(51, 20, 49, 20))) };
        let root = Node { id: 1, axis: "x".into(), rect: Rect { x: 0, y: 0, w: 100, h: 40 }, a: Some(Box::new(leaf(0, 0, 50, 40))), b: Some(Box::new(right)) };
        let hit = |col, row| split_at(&root, (col, row), (0.5, 0.25), 1).map(|h| (h.id, h.along_x));
        assert_eq!(hit(50.5, 10.0), Some((1, true)));
        assert_eq!(hit(70.0, 19.5), Some((2, false)));
        assert_eq!(hit(20.0, 10.0), None);
        // The inner split spans only its own side.
        assert_eq!(hit(20.0, 19.5), None);
    }

    #[test]
    fn a_layout_with_no_tree_reads_its_gap_from_where_its_panes_sit() {
        let win = |x, w| Win { workspace: 2, x, y: 0, w, h: 30, ..Default::default() };
        let mut state = State { workspace: 2, windows: vec![win(0, 60), win(61, 60), win(122, 60)], ..Default::default() };
        state.trees.insert("1".into(), Tree { gap: 0, ..Default::default() });
        assert_eq!(state.layout_gap(), Some(1), "columns one apart, whatever workspace 1 uses");
        state.windows = vec![win(0, 60), win(60, 60)];
        assert_eq!(state.layout_gap(), Some(0), "columns that touch");
    }

    #[test]
    fn a_workspace_that_says_nothing_about_the_layout_changes_nothing() {
        let win = |workspace| Win { workspace, x: 0, y: 0, w: 60, h: 30, ..Default::default() };
        let size = Size { cols: 242, rows: 59 };
        let tree = Tree { gap: 0, bounds: Rect { x: 0, y: 0, w: 203, h: 59 }, ..Default::default() };
        let mut state = State { workspace: 1, windows: vec![win(1), win(2)], session_size: size, client_size: size, ..Default::default() };
        state.trees.insert("1".into(), tree);
        assert_eq!((state.layout_gap(), state.unused_chrome()), (Some(0), Some((39, 0))));
        // One pane and no tree of its own, as on a new workspace or a scrolling one.
        state.workspace = 2;
        assert_eq!((state.layout_gap(), state.unused_chrome()), (None, None));
    }

    #[test]
    fn a_split_goes_where_its_name_says() {
        assert_eq!(split_side("prefix_split_vertical"), Some("right"));
        assert_eq!(split_side("split_vertical"), Some("right"));
        assert_eq!(split_side("prefix_split_horizontal"), Some("bottom"));
        assert_eq!(split_side("split_horizontal"), Some("bottom"));
        assert_eq!(split_side("rotate_split"), None);
    }

    #[test]
    fn a_zoom_step_moves_one_pixel_and_back_on_any_screen() {
        for scale in [1.0, 2.0] {
            let width = |size: f32| (0.6 * size * scale).round();
            let mut size = 13.0;
            for _ in 0..6 {
                let bigger = zoom_size(size, 0.6, scale, 1.0);
                assert_eq!(width(bigger), width(size) + 1.0);
                let back = zoom_size(bigger, 0.6, scale, -1.0);
                assert_eq!(width(back), width(size));
                size = bigger;
            }
        }
    }

    #[test]
    fn with_no_gap_between_panes_the_grid_gives_up_no_cells_for_one() {
        // A session of no gaps on a 1920 by 1050 window, at 7 by 15 cells.
        assert_eq!(stage_grid((1700.0, 1007.0), (7.0, 15.0), 0), (242, 67));
        assert_eq!(stage_grid((1700.0, 1007.0), (7.0, 15.0), 1), (241, 66));
    }

    #[test]
    fn a_pane_gets_the_cells_that_fit_its_card() {
        // 29.5 points of padding in 7 point cells is 5 cells, and 1 of them is the gap.
        assert_eq!(reserved_cells(29.5, 7.0, 1, 0), 5);
        assert_eq!(reserved_cells(29.5, 7.0, 0, 0), 6);
        // Cells tuios takes itself are not asked for again.
        assert_eq!(reserved_cells(29.5, 7.0, 0, 2), 4);
    }

    #[test]
    fn a_border_is_believed_only_when_panes_agree() {
        // Told to keep 3, a 130 by 30 slot would hold 128 by 28.
        let two = |c, r| ((130, 30), (c, r));
        assert_eq!(agreed_border([two(126, 26), two(126, 26)].into_iter(), (3, 3)), Some((2, 2)));
        assert_eq!(agreed_border([two(128, 28)].into_iter(), (3, 3)), Some((0, 0)));
        assert_eq!(agreed_border([two(126, 26), two(128, 28)].into_iter(), (3, 3)), None);
        // A grid far from its slot is a pane that has not resized yet.
        assert_eq!(agreed_border([two(90, 28)].into_iter(), (3, 3)), None);
        assert_eq!(agreed_border(std::iter::empty(), (3, 3)), None);
    }

    fn win(id: &str, workspace: i32, z: i32) -> Win {
        Win { id: id.into(), workspace, x: 0, y: 0, w: 10, h: 10, z, ..Win::default() }
    }

    #[test]
    fn a_session_with_room_kept_for_a_sidebar_reports_it_only_when_this_client_sets_the_size() {
        let json = r#"{"workspace":1,"session_size":{"cols":116,"rows":38},"client_size":{"cols":116,"rows":38},
            "trees":{"1":{"bounds":{"x":0,"y":2,"w":92,"h":36}}}}"#;
        let mut s: State = serde_json::from_str(json).unwrap();
        assert_eq!(s.layout_box(), Rect { x: 0, y: 2, w: 92, h: 36 });
        assert_eq!(s.unused_chrome(), Some((24, 2)));
        s.client_size = Size { cols: 200, rows: 50 };
        assert_eq!(s.unused_chrome(), None);
    }

    #[test]
    fn with_no_tree_the_box_is_the_whole_session() {
        let s = State { session_size: Size { cols: 80, rows: 24 }, ..State::default() };
        assert_eq!(s.layout_box(), Rect { x: 0, y: 0, w: 80, h: 24 });
    }

    fn state_on(workspace: i32, occupied: &[i32]) -> State {
        State { workspace, num_workspaces: 9, occupied: Some(occupied.to_vec()), ..State::default() }
    }

    #[test]
    fn stepping_skips_empty_workspaces_and_wraps() {
        let s = state_on(3, &[1, 3, 7]);
        assert_eq!(s.resolve("next_workspace"), Resolved::Workspace(7));
        assert_eq!(s.resolve("prev_workspace"), Resolved::Workspace(1));
        assert_eq!(state_on(7, &[1, 3, 7]).resolve("next_workspace"), Resolved::Workspace(1));
    }

    #[test]
    fn stepping_with_no_other_workspace_does_nothing() {
        assert_eq!(state_on(3, &[3]).resolve("next_workspace"), Resolved::Nothing);
    }

    #[test]
    fn moving_swaps_with_an_occupied_neighbour_lower_number_first() {
        assert_eq!(state_on(2, &[1, 2, 3]).resolve("move_workspace_left"), Resolved::Order(vec![2, 1]));
        assert_eq!(state_on(2, &[1, 2, 3]).resolve("move_workspace_right"), Resolved::Order(vec![3, 2]));
        assert_eq!(state_on(2, &[2]).resolve("move_workspace_right"), Resolved::Nothing);
        assert_eq!(state_on(1, &[1]).resolve("move_workspace_left"), Resolved::Nothing);
    }

    #[test]
    fn resizes_map_to_what_the_bridge_accepts_and_commands_pass_through() {
        let s = state_on(1, &[1]);
        assert_eq!(s.resolve("resize_pane_left_5"), Resolved::Action("resize_master_shrink".into()));
        assert_eq!(s.resolve("resize_pane_down_20"), Resolved::Action("resize_height_grow".into()));
        assert_eq!(s.resolve("command:deploy"), Resolved::Action("command:deploy".into()));
    }

    #[test]
    fn a_click_hits_the_topmost_window_on_the_workspace() {
        let windows = [win("low", 1, 1), win("high", 1, 2), win("other", 2, 9)];
        assert_eq!(window_at(&windows, 1, 5, 5, 1).unwrap().id, "high");
        assert!(window_at(&windows, 1, 50, 5, 1).is_none());
    }

    #[test]
    fn slots_tile_with_no_dead_cell_between_neighbours() {
        let left = Win { id: "l".into(), workspace: 1, x: 0, w: 59, h: 20, ..Win::default() };
        let right = Win { id: "r".into(), workspace: 1, x: 60, w: 58, h: 20, ..Win::default() };
        let windows = [left, right];
        assert_eq!(window_at(&windows, 1, 59, 5, 1).unwrap().id, "l");
        assert_eq!(window_at(&windows, 1, 60, 5, 1).unwrap().id, "r");
    }

    #[test]
    fn with_no_gap_a_cell_belongs_to_exactly_one_pane() {
        let left = Win { id: "l".into(), workspace: 1, x: 0, w: 44, h: 20, ..Win::default() };
        let right = Win { id: "r".into(), workspace: 1, x: 44, w: 40, h: 20, ..Win::default() };
        let windows = [left, right];
        assert_eq!(window_at(&windows, 1, 43, 5, 0).unwrap().id, "l");
        assert_eq!(window_at(&windows, 1, 44, 5, 0).unwrap().id, "r");
        assert!(window_at(&windows, 1, 84, 5, 0).is_none());
    }

    #[test]
    fn state_parses_with_fields_this_client_does_not_read() {
        let json = r#"{"session":"s","workspace":1,"occupied":null,"focused":"a","layout":"bsp",
            "windows":[{"id":"a","pty":"p","agent":"working","queued":0,"kind":"terminal"}]}"#;
        let state: State = serde_json::from_str(json).unwrap();
        assert_eq!(state.windows[0].agent, "working");
    }

    #[test]
    fn a_default_title_gives_way_to_the_program_then_the_folder() {
        let mut w = Win { title: "Terminal 77f6169b".into(), cwd: "/a/tessella".into(), ..Win::default() };
        assert_eq!(w.label(), "tessella");
        w.foreground = "vim".into();
        assert_eq!(w.label(), "vim");
        w.title = "review-api".into();
        assert_eq!(w.label(), "review-api");
        w.name = "named".into();
        assert_eq!(w.label(), "named");
        assert_eq!(Win::default().label(), "Shell");
    }

    #[test]
    fn a_pane_whose_agent_left_shows_no_stale_note() {
        let mut w = Win { agent: "working".into(), agent_message: "refactoring".into(), ..Win::default() };
        assert_eq!(w.note(), "refactoring");
        for gone in ["none", "unknown", ""] {
            w.agent = gone.into();
            assert_eq!(w.note(), "", "state {gone:?}");
        }
    }

}
