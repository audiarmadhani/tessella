//! Tessella: a native window on a tuios session. See src/bridge.rs for the wire.

use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use futures::StreamExt;
mod agents_view;
mod far_files;
mod file_actions;
mod md_view;
mod notices;
mod pane_links;
mod settings_view;
mod stage_mouse;
mod views;
mod work_view;

use gpui::{
    ContentMask, Subscription, canvas, div, fill, font, point, prelude::*, px, quad, rgb, size, App, Application, BorderStyle, Bounds,
    ClipboardItem, Context, FocusHandle, Font, FontStyle, FontWeight, Hsla, KeyDownEvent, Keystroke,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollHandle, ScrollWheelEvent, SharedString, TextRun,
    UnderlineStyle, Window, WindowBackgroundAppearance, WindowBounds, WindowOptions,
};
use serde_json::{json, Value};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime};
use tessella::bridge::{self, Frame, Grid, Target, Writer};
use tessella::copymode::{CopyMode, Effect};
use tessella::keymap::{self, Keymap, Step};
use tessella::keys::{self, Mods};
use tessella::machines::{self, Machine, MachinePicker};
use tessella::model::{self, Resolved, State, Win};
use tessella::picker::{favourites_first, Item, Picker};
use tessella::predict;
use tessella::rail::{self, Entry, Fleet, Filter, Group, Row};
use tessella::settings::{self, Settings};
use tessella::terminal::{Pane, Screen, SelectionType, Side};
use tessella::theme::{self, Theme};
use tessella::window_state::WindowState;

/// Fonts to try with no family set. One font is used everywhere. These are Nerd
/// Fonts first, for the icons that tuios, yazi and the like draw.
const PREFERRED_FONTS: [&str; 4] = [
    "JetBrainsMono Nerd Font Mono",
    "MesloLGS NF",
    "FiraCode Nerd Font Mono",
    "Hack Nerd Font Mono",
];
const SF_MONO_FILE: &str = "/System/Library/Fonts/SFNSMono.ttf";

const SIDEBAR_MIN: f32 = 190.0;
const SIDEBAR_MAX: f32 = 420.0;
const AGENTS_MIN: f32 = 40.0;
const AGENTS_MAX: f32 = 700.0;
/// The agents section's header and padding, around its list.
const AGENTS_CHROME: f32 = 34.0;
/// The dock across the top, which the traffic lights sit in.
const TOPBAR: f32 = 40.0;
const FONT_MIN: f32 = 9.0;
const FONT_MAX: f32 = 22.0;
/// The scope of tuios's keys for a file row.
const FILE_SCOPE: &str = "sidebar.files";
/// Runs the person's editor on `$0`, at line `$1` if there is one and it can.
const EDIT_SCRIPT: &str = r#"e=${EDITOR:-vi}; p=${e%% *}; case "${p##*/}" in vi|vim|nvim|nano|emacs|hx|kak|micro) [ -n "$1" ] && exec $e "+$1" "$0";; esac; exec $e "$0""#;
/// Marks a palette row that tuios runs by name.
const PALETTE_ROW: &str = "palette:";
/// How long to wait for tuios to answer a change of size.
const SIZE_WAIT: Duration = Duration::from_millis(500);
/// How long a new window size has to hold before panes resize for it, so that dragging
/// an edge resizes them once and not on every step.
const GRID_SETTLE: Duration = Duration::from_millis(120);
/// How long the panes' saved state waits, so that a drag writes the file once.
const STATE_SAVE_WAIT: Duration = Duration::from_millis(500);
/// How long panes just brought on screen get to reach their size. A pane on another
/// machine answers over the network.
const FIT_HOLD: Duration = Duration::from_millis(700);
/// How long the panes' grids have to agree before they are believed.
const BORDER_SETTLE: Duration = Duration::from_millis(400);
/// Room around all the panes, and around each pane inside its slot.
const STAGE_PAD: f32 = 5.0;
const MARGIN: f32 = 4.0;
const HEADER: f32 = 28.0;
const PAD_X: f32 = 12.0;
const PAD_BOTTOM: f32 = 8.0;
const CARD_RADIUS: f32 = 10.0;
const BASE_FONT_SIZE: f32 = 13.0;

/// The zoom of everything drawn: the interface follows the text size.
static UI_SCALE: AtomicU32 = AtomicU32::new(1.0f32.to_bits());

fn ui(v: f32) -> f32 {
    v * f32::from_bits(UI_SCALE.load(Ordering::Relaxed))
}

fn set_ui_scale(font_size: f32) {
    UI_SCALE.store((font_size / BASE_FONT_SIZE).to_bits(), Ordering::Relaxed);
}

/// Fast at the start and settling at the end.
fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// The font to use: the one the settings name, a Nerd Font, then what macOS ships.
/// SF Mono is in the system folder and not the font list, so it loads from there.
fn app_font(wanted: &str, cx: &App) -> SharedString {
    let text = cx.text_system();
    let names = text.all_font_names();
    if !wanted.is_empty() && names.iter().any(|n| n == wanted) {
        return wanted.to_string().into();
    }
    if let Some(found) = PREFERRED_FONTS.iter().find(|f| names.iter().any(|n| n == *f)) {
        return (*found).into();
    }
    let family = |names: Vec<String>| names.into_iter().find(|n| n == "SF Mono" || n == "SF NS Mono");
    if let Some(name) = family(names) {
        return name.into();
    }
    if let Ok(bytes) = std::fs::read(SF_MONO_FILE) {
        let _ = text.add_fonts(vec![Cow::Owned(bytes)]);
        if let Some(name) = family(text.all_font_names()) {
            return name.into();
        }
    }
    "Menlo".into()
}

#[derive(Clone, Copy)]
struct Metrics {
    cell_w: Pixels,
    cell_h: Pixels,
}

/// The stage's slot grid: pixels per cell, stretched so the slots fill it.
#[derive(Clone, Copy)]
struct Slots {
    w: f32,
    h: f32,
    /// Where the layout box starts, in cells.
    x: i32,
    y: i32,
    /// The cells between neighbouring panes.
    gap: i32,
}

/// The fonts a pane's text is drawn with.
#[derive(Clone)]
struct Look {
    font: SharedString,
    size: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ChooserKind {
    Theme,
    Font,
    Palette,
    Session,
    Workspace,
    Pane,
    Launcher,
    Layout,
    FileSearch,
    SendTo,
}

#[derive(Debug, Clone)]
enum Confirm {
    ClosePane(String),
    CloseWorkspace(i32),
    KillSession(String),
    DeleteLayout(String),
    Trash(PathBuf),
    DeleteForever(tessella::files::Place),
    RestoreCheckpoint { host: String, session: String, window: String, n: i64 },
    RemoveWorktree { host: String, session: String },
}

#[derive(Debug, Clone)]
enum PromptKind {
    RenameWindow(String),
    RenameWorkspace(i32),
    RenameSession,
    NewSession,
    SaveLayout,
    NewFile(tessella::files::Place),
    RenameFile(tessella::files::Place),
    Confirm(Confirm),
}

#[derive(Debug, Clone)]
struct Prompt {
    kind: PromptKind,
    title: String,
    text: String,
    /// The text is what the prompt opened with: the first key replaces it.
    fresh: bool,
}

/// At most one thing sits over the panes and takes the keys.
#[derive(Default)]
enum Overlay {
    #[default]
    None,
    Chooser(ChooserKind, Picker),
    Machines(MachinePicker),
    Prompt(Prompt),
    Help,
    Settings(SettingsPage),
    /// A number on each pane: its key goes there.
    PaneLabels,
    /// A menu at the pointer.
    Menu(file_actions::ContextMenu),
    /// The Inbox, the agents, the mail and a review.
    Agents(Box<agents_view::AgentsPage>),
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum Section {
    #[default]
    General,
    Appearance,
    Sidebar,
    Keyboard,
    About,
}

/// A key being rebound: the next key pressed takes this place.
#[derive(Debug, Clone)]
enum Capture {
    Leader,
    Prefix { key: String, action: String },
}

#[derive(Debug, Clone, Default)]
struct SettingsPage {
    section: Section,
    capture: Option<Capture>,
}

/// One pane, copied out of the model so the canvas can own what it draws.
struct PaneDraw {
    win: Win,
    label: String,
    focused: bool,
    screen: Screen,
    /// How far along the stage it is drawn, while workspaces slide.
    dx: f32,
    /// Its rectangle in cells this frame, while the layout changes.
    rect: [f32; 4],
    /// Its scrollbar thumb: top and height, as fractions of the track.
    scrollbar: Option<(f32, f32)>,
    /// The Markdown file it shows in place of a terminal.
    doc: Option<md_view::DocDraw>,
    /// The number on it while the pane picker labels the panes.
    badge: Option<usize>,
    /// The scale its text is drawn at, so that it fits its card.
    fit: f32,
    /// The cells of the link under the pointer, to underline.
    link: Vec<(usize, usize, usize)>,
}

/// A workspace change on its way across the stage.
#[derive(Clone, Copy)]
struct Slide {
    from: i32,
    started: Instant,
}

/// What a verb's answer is for.
#[derive(Debug, Clone)]
enum Reply {
    PanePreview(String),
    Agents(agents_view::AgentsReply),
    Files(far_files::FileReply),
    /// A pane opened on another machine, to go beside this one.
    HostSplit { target: String, side: &'static str },
}

/// Panes moving to new places when the layout changes: a zoom, a split, a closed pane.
/// Each starts from its rectangle in cells, by window id.
struct LayoutMove {
    from: HashMap<String, [f32; 4]>,
    started: Instant,
}

fn rect_of(w: &Win) -> [f32; 4] {
    [w.x as f32, w.y as f32, w.w as f32, w.h as f32]
}

/// The sidebar opening or closing.
#[derive(Clone, Copy)]
struct RailMove {
    from: f32,
    started: Instant,
}

struct Tessella {
    writer: Writer,
    // Holding the child keeps its stdin open. The bridge exits when this process does.
    _child: std::process::Child,
    panes: HashMap<String, Pane>,
    state: State,
    theme: Theme,
    theme_defaulted: bool,
    overlay: Overlay,
    fleet: Fleet,
    palette_rows: Vec<Item>,
    asked_for_pane: bool,
    grid: (u16, u16),
    scale: f32,
    /// What tuios keeps out of the box for chrome this client does not draw.
    unused_chrome: (u16, u16),
    /// The cells tuios keeps between panes, which sets how much room they need.
    layout_gap: i32,
    /// Where a chooser opened from the settings goes back to.
    settings_return: bool,
    agents_height: f32,
    /// Dragging the line above the agents section.
    agents_drag: bool,
    /// A pane opened on another machine, its neighbour, and the side it goes on.
    host_split: Option<(String, String, &'static str)>,
    /// A window size waiting to hold still, and since when.
    grid_pending: Option<((u16, u16), Instant)>,
    /// What was last saved for the next launch, and a save waiting to happen.
    state_saved: Option<WindowState>,
    state_save_due: Option<Instant>,
    /// Panes just brought on screen may not have their size yet. Until this time their
    /// text stays its size, rather than shrinking for a moment and growing back.
    fit_hold: Option<Instant>,
    /// A redraw for when the hold ends has yet to be asked for.
    fit_wake: bool,
    /// The agents on other machines, and how each machine last answered.
    remote_agents: Vec<rail::Agent>,
    remote_hosts: Vec<tessella::agents::HostState>,
    /// When other machines were last asked for their agents.
    remote_asked: Option<Instant>,
    /// The Markdown files on show, by the pane that shows each.
    docs: HashMap<String, md_view::MdDoc>,
    /// A check for changed files is on its way.
    doc_timer: bool,
    /// The window's size in points, for what is placed against its edges.
    viewport: (f32, f32),
    file_clip: Option<tessella::files::Clip>,
    /// The row a file menu was opened on.
    menu_entry: Option<Entry>,
    stage_drag: Option<stage_mouse::StageDrag>,
    stage_cursor: gpui::CursorStyle,
    link_hover: Option<pane_links::LinkHover>,
    notices: notices::Notices,
    review_wake: Option<Instant>,
    diff_side_by_side: bool,
    /// A far pane's changes, once read beside the window.
    far_diff: std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>>,
    far_diff_running: bool,
    pointer: Option<Point<Pixels>>,
    /// The cell a program following the mouse was last told about.
    mouse_seen: Option<(usize, usize)>,
    /// When the open prefix was pressed, for the which-key popup.
    prefix_at: Option<Instant>,
    /// What verb answers are for, by request.
    replies: HashMap<i64, Reply>,
    next_req: i64,
    /// The text of the pane the pane picker is on, by its value.
    pane_preview: Option<(String, Vec<String>)>,
    /// Panes on their way to new places.
    layout_move: Option<LayoutMove>,
    /// A press on a pane until the button comes up: the pane, where, and the focus before.
    click_reveal: Option<(String, Point<Pixels>, String)>,
    /// tuios's appearance.selection.osc52_write.
    osc52_write: String,
    /// The pane a mouse press went to, and the cell it was last told about.
    mouse_down: Option<(String, (usize, usize))>,
    device_scale: f32,
    /// The size last asked of tuios, and when.
    size_sent: Option<((u16, u16), Instant)>,
    /// The cells last asked to be kept out of each pane's slot.
    reserved_sent: (i32, i32),
    /// The cells tuios takes from each pane for itself.
    pane_border: (i32, i32),
    border_seen: Option<((i32, i32), Instant)>,
    /// Whether the first state has been checked against the size asked for.
    size_checked: bool,
    slide: Option<Slide>,
    rail_move: Option<RailMove>,
    reduce_motion: bool,
    _activation: Subscription,
    look: Look,
    focus: FocusHandle,
    status: Option<String>,
    keymap: Keymap,
    /// tuios's keys as last sent, to rebuild from when the settings change.
    keys_event: Value,
    settings: Settings,
    settings_changed: Option<SystemTime>,
    started: Instant,
    machines: Vec<Machine>,
    last_session: Option<String>,
    wheel: f32,
    /// Copy mode, and the pane it is on.
    copy: Option<(String, CopyMode)>,
    /// The pane a mouse selection is being dragged in.
    drag: Option<String>,
    // The sidebar.
    rail_open: bool,
    rail_width: f32,
    rail_focused: bool,
    rail_cursor: usize,
    rail_filter: Filter,
    rail_collapsed: Vec<rail::Section>,
    rail_scroll: ScrollHandle,
    agents_scroll: ScrollHandle,
    // The folder the files section shows, and for which pane it was chosen.
    files: Vec<Entry>,
    files_dir: Option<PathBuf>,
    files_shown: Option<tessella::files::Place>,
    files_read: Option<Instant>,
    files_pane: String,
    /// The folder asked for on another machine, which may be `~` until it answers.
    files_want: Option<tessella::files::Place>,
    files_asked: Option<Instant>,
    /// Counts the far requests that have been superseded, to drop their answers.
    files_request: u64,
    files_error: Option<String>,
    search_host: String,
    /// The machine this whole window is attached to, with `--host`.
    target_host: Option<String>,
}

impl Tessella {
    #[allow(clippy::too_many_arguments)]
    fn new(target: Target, settings: Settings, status: Option<String>, look: Look, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let m = metrics(&look, window);
        let scale = window.scale_factor();
        let rail_space = if settings.sidebar.visible { ui(settings.sidebar.width) } else { 0.0 };
        // What the last run learned, so that the first size asked for is the right one.
        let saved = WindowState::load();
        let (gap, chrome, border) = saved.as_ref().map_or((1, (0, 0), (0, 0)), |s| (s.gap, s.chrome, s.border));
        let grid = grid_for(m, window, rail_space, gap);
        let reserve = reserved(m, gap, border);
        let b = match bridge::spawn(&target, bridge_grid((grid.0 + chrome.0, grid.1 + chrome.1), m, scale, reserve)) {
            Ok(b) => b,
            Err(err) => {
                eprintln!("tessella: {err}");
                if tessella::notify::is_bundled() {
                    tessella::notify::alert(&err.to_string());
                }
                std::process::exit(1);
            }
        };
        let mut reader = b.reader;
        let (tx, rx) = unbounded();
        std::thread::spawn(move || loop {
            match bridge::read_frame(&mut reader) {
                Ok(frame) => {
                    if tx.unbounded_send(frame).is_err() {
                        return;
                    }
                }
                Err(err) => {
                    eprintln!("tessella: bridge closed: {err}");
                    return;
                }
            }
        });
        Self::pump(rx, cx);
        Self::watch_settings(cx);
        // The session follows the window in use, so coming back here claims it.
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.scale = 0.0;
                cx.notify();
            }
        });

        let focus = cx.focus_handle();
        focus.focus(window);
        if let Ok(path) = std::env::var("TESSELLA_TEST_INPUT") {
            Self::test_input(path, window, cx);
        }
        Self {
            writer: b.writer,
            _child: b.child,
            panes: HashMap::new(),
            state: State::default(),
            theme: Theme::default(),
            theme_defaulted: false,
            overlay: Overlay::None,
            fleet: Fleet::default(),
            palette_rows: Vec::new(),
            asked_for_pane: false,
            grid,
            scale,
            unused_chrome: chrome,
            layout_gap: gap,
            settings_return: false,
            agents_height: settings.sidebar.agents_height,
            agents_drag: false,
            host_split: None,
            grid_pending: None,
            state_saved: None,
            state_save_due: None,
            fit_hold: None,
            fit_wake: false,
            remote_agents: Vec::new(),
            remote_hosts: Vec::new(),
            remote_asked: None,
            docs: HashMap::new(),
            doc_timer: false,
            viewport: (1180.0, 760.0),
            file_clip: None,
            menu_entry: None,
            stage_drag: None,
            stage_cursor: gpui::CursorStyle::Arrow,
            link_hover: None,
            notices: notices::Notices::default(),
            review_wake: None,
            diff_side_by_side: saved.as_ref().is_some_and(|s| s.diff_side_by_side),
            far_diff: Default::default(),
            far_diff_running: false,
            pointer: None,
            mouse_seen: None,
            prefix_at: None,
            replies: HashMap::new(),
            next_req: 100,
            pane_preview: None,
            layout_move: None,
            click_reveal: None,
            osc52_write: "focused".to_string(),
            mouse_down: None,
            device_scale: 1.0,
            size_sent: None,
            reserved_sent: reserve,
            pane_border: border,
            border_seen: None,
            size_checked: false,
            slide: None,
            rail_move: None,
            reduce_motion: tessella::native_input::reduce_motion(),
            _activation: activation,
            look,
            focus,
            status,
            keymap: Keymap::default(),
            keys_event: Value::Null,
            settings_changed: settings_file_time(),
            started: Instant::now(),
            machines: machines::from_event(&Value::Null),
            last_session: None,
            wheel: 0.0,
            copy: None,
            drag: None,
            rail_open: settings.sidebar.visible,
            rail_width: settings.sidebar.width,
            settings,
            rail_focused: false,
            rail_cursor: 0,
            rail_filter: Filter::All,
            rail_collapsed: Vec::new(),
            rail_scroll: ScrollHandle::new(),
            agents_scroll: ScrollHandle::new(),
            files: Vec::new(),
            files_dir: None,
            files_shown: None,
            files_read: None,
            files_pane: String::new(),
            files_want: None,
            files_asked: None,
            files_request: 0,
            files_error: None,
            search_host: String::new(),
            target_host: target.host.clone(),
        }
    }

    /// Feeds input for checks, from a named pipe, one input per word: a key as gpui
    /// writes it (`cmd-t`, `down`), or real AppKit events, `click:X,Y` in window
    /// points, `nskey:CODE:TEXT` by macOS key code, and `wheel:X,Y,LINES`.
    fn test_input(path: String, window: &Window, cx: &mut Context<Self>) {
        let Some(view) = tessella::native_input::view_address(window) else { return };
        let (tx, mut rx) = unbounded::<String>();
        std::thread::spawn(move || loop {
            let Ok(text) = std::fs::read_to_string(&path) else { return };
            for word in text.split_whitespace() {
                if tx.unbounded_send(word.to_string()).is_err() {
                    return;
                }
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            while let Some(word) = rx.next().await {
                if let Some(at) = word.strip_prefix("frame:") {
                    let n: Vec<f64> = at.split(',').filter_map(|v| v.parse().ok()).collect();
                    if let [w, h] = n[..] {
                        // SAFETY: as below.
                        unsafe { tessella::native_input::resize_window(view, w, h) };
                    }
                } else if let Some(at) = word.strip_prefix("rclick:") {
                    let n: Vec<f64> = at.split(',').filter_map(|v| v.parse().ok()).collect();
                    if let [x, y] = n[..] {
                        // SAFETY: as below.
                        unsafe { tessella::native_input::right_click(view, x, y) };
                    }
                } else if let Some(at) = word.strip_prefix("wheel:") {
                    // X,Y in window points, then lines, up when positive.
                    let n: Vec<f32> = at.split(',').filter_map(|v| v.parse().ok()).collect();
                    if let [x, y, lines] = n[..] {
                        let event = ScrollWheelEvent {
                            position: point(px(x), px(y)),
                            delta: gpui::ScrollDelta::Lines(point(0.0, lines)),
                            ..Default::default()
                        };
                        let _ = this.update_in(cx, |app, window, cx| app.on_stage_wheel(&event, window, cx));
                    }
                } else if let Some(at) = word.strip_prefix("drag:") {
                    let n: Vec<f64> = at.split(',').filter_map(|v| v.parse().ok()).collect();
                    if let [x1, y1, x2, y2] = n[..] {
                        // SAFETY: the main thread, outside any update, and this
                        // task ends with the window.
                        unsafe { tessella::native_input::drag(view, (x1, y1), (x2, y2)) };
                    }
                } else if word == "raise" {
                    // SAFETY: the main thread, outside any update, and this task
                    // ends with the window.
                    unsafe { tessella::native_input::raise(view) };
                } else if let Some(at) = word.strip_prefix("cmdclick:") {
                    let n: Vec<f64> = at.split(',').filter_map(|v| v.parse().ok()).collect();
                    if let [x, y] = n[..] {
                        // SAFETY: as above.
                        unsafe { tessella::native_input::cmd_click(view, x, y) };
                    }
                } else if let Some(at) = word.strip_prefix("cmdhover:") {
                    let n: Vec<f64> = at.split(',').filter_map(|v| v.parse().ok()).collect();
                    if let [x, y] = n[..] {
                        // SAFETY: as above.
                        unsafe { tessella::native_input::cmd_move(view, x, y) };
                    }
                } else if let Some(path) = word.strip_prefix("drop:") {
                    let paths = vec![PathBuf::from(path)];
                    let _ = this.update_in(cx, |app, window, cx| {
                        app.drop_paths(paths, window);
                        cx.notify();
                    });
                } else if let Some(at) = word.strip_prefix("click:") {
                    let n: Vec<f64> = at.split(',').filter_map(|v| v.parse().ok()).collect();
                    if let [x, y, rest @ ..] = n.as_slice() {
                        let (x, y) = (*x, *y);
                        let clicks = rest.first().copied().unwrap_or(1.0) as isize;
                        // SAFETY: as above.
                        unsafe { tessella::native_input::click(view, x, y, clicks) };
                    }
                } else if let Some(spec) = word.strip_prefix("nskey:") {
                    // CODE:TEXT, then optionally :shift, :cmd or :shift+cmd.
                    let mut parts = spec.splitn(3, ':');
                    if let (Some(Ok(code)), Some(text)) = (parts.next().map(str::parse), parts.next()) {
                        let mods = parts.next().unwrap_or_default();
                        let flags = if code >= 123 { 1usize << 23 } else { 0 } | if mods.contains("ctrl") { 1usize << 18 } else { 0 } | if mods.contains("shift") { 1usize << 17 } else { 0 } | if mods.contains("cmd") { 1usize << 20 } else { 0 };
                        // SAFETY: as above.
                        unsafe { tessella::native_input::key(view, code, text, flags) };
                    }
                } else if let Ok(key) = Keystroke::parse(&word) {
                    let _ = cx.update(|window, cx| window.dispatch_keystroke(key, cx));
                } else {
                    eprintln!("tessella: test input: bad word {word}");
                }
                cx.background_executor().timer(Duration::from_millis(25)).await;
            }
        })
        .detach();
    }

    /// Applies the settings file whenever it is saved.
    fn watch_settings(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let alive = this.update(cx, |app, cx| {
                let time = settings_file_time();
                if time != app.settings_changed {
                    app.settings_changed = time;
                    app.reload_settings(cx);
                    cx.notify();
                }
            });
            if alive.is_err() {
                return;
            }
        })
        .detach();
    }

    fn reload_settings(&mut self, cx: &mut Context<Self>) {
        self.apply_settings_file(cx, true);
    }

    fn apply_settings_file(&mut self, cx: &mut Context<Self>, announce: bool) {
        match Settings::load() {
            Ok(new) => {
                let old = std::mem::replace(&mut self.settings, new);
                let new = self.settings.clone();
                if new.font != old.font {
                    self.look.font = app_font(&new.font.family, cx);
                    // Changing the family keeps the zoom.
                    self.set_font_size(if new.font.size != old.font.size { new.font.size } else { self.look.size });
                }
                if new.sidebar != old.sidebar {
                    self.rail_width = new.sidebar.width;
                    self.agents_height = new.sidebar.agents_height;
                    if new.sidebar.visible != self.rail_open {
                        self.toggle_rail();
                    }
                }
                self.keymap = Keymap::from_event(&self.keys_event).with_settings(&self.settings.keys);
                if announce {
                    self.status = Some("Settings applied.".into());
                }
            }
            // The last good settings stay, as tuios does with a bad config.
            Err(e) => self.status = Some(format!("Settings not applied: {e}")),
        }
    }

    /// Applies every frame that has landed, then draws once.
    fn pump(mut rx: UnboundedReceiver<Frame>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                let mut batch = vec![first];
                while let Ok(Some(next)) = rx.try_next() {
                    batch.push(next);
                }
                let alive = this.update(cx, |app, cx| {
                    for frame in batch {
                        app.apply(frame, cx);
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    return;
                }
            }
            let _ = this.update(cx, |app, cx| {
                app.status = Some("tuios gui-bridge stopped. Tessella needs tuios 0.10 or newer.".into());
                cx.notify();
            });
        })
        .detach();
    }

    fn apply(&mut self, frame: Frame, cx: &mut Context<Self>) {
        match frame {
            Frame::Event(event) => self.apply_event(event),
            Frame::Output { pty, bytes } => {
                let pane = self.panes.entry(pty.clone()).or_insert_with(|| Pane::new(80, 24));
                pane.feed(&bytes);
                for text in pane.take_copies() {
                    self.pane_copied(&pty, text, cx);
                }
            }
            // A snapshot rebuilds a reset emulator. When the daemon has no state
            // for the pane, it sends 0x0 with no bytes: nothing to rebuild from.
            Frame::Snapshot { cols: 0, .. } | Frame::Snapshot { rows: 0, .. } => {}
            Frame::Snapshot { pty, cols, rows, bytes } => {
                let mut pane = Pane::new(cols, rows);
                pane.feed(&bytes);
                // A copy in here is one from before, not one being made now.
                pane.take_copies();
                self.panes.insert(pty, pane);
            }
            Frame::Resize { pty, cols, rows } => {
                self.panes.entry(pty).or_insert_with(|| Pane::new(cols, rows)).resize(cols, rows);
            }
            Frame::Unknown(_) => {}
        }
    }

    fn apply_event(&mut self, event: Value) {
        match event["type"].as_str() {
            Some("state") => {
                if let Ok(state) = serde_json::from_value::<State>(event["state"].clone()) {
                    if !self.state.session.is_empty() && self.state.session != state.session {
                        self.last_session = Some(self.state.session.clone());
                    }
                    let same_session = self.state.session == state.session;
                    if same_session && self.state.workspace != state.workspace && self.state.workspace > 0 && self.animation().is_some() {
                        self.slide = Some(Slide { from: self.state.workspace, started: Instant::now() });
                        self.fit_hold = Some(Instant::now() + FIT_HOLD);
                        self.fit_wake = true;
                    }
                    if same_session {
                        self.start_layout_move(&state);
                    } else {
                        self.layout_move = None;
                    }
                    if self.state.session.is_empty() {
                        self.send(json!({"cmd": "options"}));
                    }
                    let new_pane = state.focus_moved_to_new_pane(&self.state);
                    let previous = std::mem::take(&mut self.state.focused);
                    self.state = state;
                    self.place_host_split();
                    if new_pane {
                        let focused = self.state.focused.clone();
                        self.reveal_if_cut_off(&focused, &previous);
                    }
                    self.sync_session_size();
                    let live: Vec<&str> = self.state.windows.iter().map(|w| w.pty.as_str()).collect();
                    self.panes.retain(|pty, _| live.contains(&pty.as_str()));
                    if self.copy.as_ref().is_some_and(|(pty, _)| !self.panes.contains_key(pty)) {
                        self.copy = None;
                    }
                    if self.state.windows.is_empty() {
                        if !self.asked_for_pane {
                            self.asked_for_pane = true;
                            self.send(json!({"cmd": "action", "name": "new_window"}));
                        }
                    } else {
                        self.asked_for_pane = false;
                    }
                }
            }
            Some("options") => {
                let rows = event["options"]["options"].as_array().into_iter().flatten();
                if let Some(mode) = rows.filter(|o| o["path"] == "appearance.selection.osc52_write").find_map(|o| o["value"].as_str()) {
                    self.osc52_write = mode.to_string();
                }
            }
            Some("theme") => {
                if let Some(theme) = Theme::from_event(&event["theme"]) {
                    let unset = theme.name.is_empty();
                    self.theme = theme;
                    // With no theme in the config, tuios has plain terminal colours.
                    let fallback = self.settings.theme.fallback.clone();
                    if unset && !self.theme_defaulted && !fallback.is_empty() {
                        self.theme_defaulted = true;
                        self.send(json!({"cmd": "theme", "theme": fallback}));
                    }
                }
            }
            Some("attention") => self.fleet.apply_attention(&event["attention"]),
            Some("fleet") => self.fleet.apply_fleet(&event["fleet"]),
            Some("keybinds") => {
                self.keys_event = event["keybinds"].clone();
                self.keymap = Keymap::from_event(&self.keys_event).with_settings(&self.settings.keys);
            }
            Some("hosts") => self.machines = machines::from_event(&event["hosts"]),
            Some("palette") => {
                self.palette_rows = event["palette"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| {
                        let name = r["name"].as_str()?;
                        // A row with no action runs by its name in tuios, as the layouts do.
                        let action = r["action"].as_str().filter(|a| !a.is_empty()).map_or_else(|| format!("{PALETTE_ROW}{name}"), str::to_string);
                        Some(Item::new(name, r["shortcut"].as_str().unwrap_or_default(), action))
                    })
                    .collect();
            }
            Some("launcher") => {
                let items = event["launcher"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| {
                        let path = e["path"].as_str()?;
                        let name = e["display"].as_str().filter(|d| !d.is_empty()).or(e["name"].as_str()).unwrap_or(path);
                        Some(Item::new(name, e["detail"].as_str().unwrap_or_default(), path))
                    })
                    .collect();
                self.overlay = Overlay::Chooser(ChooserKind::Launcher, Picker::new(items, String::new()));
            }
            Some("result") if event["result"]["ok"] == false => {
                let result = &event["result"];
                let error = result["error"].as_str().unwrap_or_default();
                // tuios draws these itself, and this client has not built them yet.
                self.status = if error.contains("only a screen can show") || error.contains("not an action") {
                    let name = result["name"].as_str().unwrap_or("That");
                    Some(format!("{} isn't in Tessella yet.", self.keymap.description(name).unwrap_or(name)))
                } else {
                    Some(error.to_string())
                };
            }
            Some("verb_result") => {
                let result = &event["verb_result"];
                let reply = result["req"].as_i64().and_then(|req| self.replies.remove(&req));
                if result["ok"] == false {
                    let error = result["error"].as_str().unwrap_or("That did not work.");
                    match reply {
                        Some(Reply::Files(reply)) => self.file_failed(reply, error, result["code"].as_str().unwrap_or_default()),
                        Some(Reply::Agents(agents_view::AgentsReply::Diff { .. })) => self.review_failed(error),
                        // A pane outside a git repository has no turns, and Review says why.
                        Some(Reply::Agents(agents_view::AgentsReply::Turns | agents_view::AgentsReply::Worktrees(_))) => {}
                        _ => self.status = Some(error.to_string()),
                    }
                } else if let Some(reply) = reply {
                    self.verb_answered(reply, &result["result"]);
                }
            }
            Some("layouts") => {
                let items = event["layouts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|l| {
                        let name = l["name"].as_str()?;
                        Some(Item::new(name, format!("{} panes", l["panes"].as_i64().unwrap_or(0)), name))
                    })
                    .collect::<Vec<_>>();
                if items.is_empty() {
                    self.status = Some("No saved layouts yet.".into());
                } else {
                    self.overlay = Overlay::Chooser(ChooserKind::Layout, Picker::new(items, String::new()));
                }
            }
            Some("detached") => self.status = Some("Detached from the session.".into()),
            _ => {}
        }
    }

    /// Checks the session against this window. The size is settled once, after
    /// the attach. After that a session follows the client in use, so another
    /// client taking it is not something to take back.
    fn sync_session_size(&mut self) {
        if let Some(gap) = self.state.layout_gap() {
            if gap != self.layout_gap {
                self.layout_gap = gap;
                self.scale = 0.0;
            }
        }
        if let Some(chrome) = self.state.unused_chrome() {
            if chrome != self.unused_chrome {
                self.unused_chrome = chrome;
                self.scale = 0.0;
            }
        }
        if !self.size_checked && self.state.session_size.cols > 0 {
            self.size_checked = true;
            self.scale = 0.0;
        }
    }

    /// How long animations take, or None when they are off.
    fn animation(&self) -> Option<Duration> {
        let a = &self.settings.animation;
        (a.enabled && a.duration_ms > 0 && !self.reduce_motion).then(|| Duration::from_millis(a.duration_ms))
    }

    /// How far along an animation is, from 0 to 1, eased.
    fn progress(&self, started: Instant) -> f32 {
        let total = self.animation().map_or(0.0, |d| d.as_secs_f32());
        if total <= 0.0 { 1.0 } else { ease_out(started.elapsed().as_secs_f32() / total) }
    }

    fn set_font_size(&mut self, size: f32) {
        self.look.size = size.clamp(FONT_MIN, FONT_MAX);
        set_ui_scale(self.look.size);
        // The cell and the padding changed, even if the grid comes out the same.
        self.scale = 0.0;
    }

    fn toggle_rail(&mut self) {
        let from = self.rail_drawn();
        self.rail_open = !self.rail_open;
        self.rail_focused = false;
        self.rail_move = self.animation().map(|_| RailMove { from, started: Instant::now() });
    }

    /// The width the sidebar is drawn at this frame, in unzoomed points.
    fn rail_drawn(&self) -> f32 {
        let target = if self.rail_open { self.rail_width } else { 0.0 };
        match self.rail_move {
            Some(m) => m.from + (target - m.from) * self.progress(m.started),
            None => target,
        }
    }

    fn send(&mut self, command: Value) {
        if let Err(err) = self.writer.command(&command) {
            self.status = Some(format!("Could not reach the bridge: {err}"));
        }
    }

    fn focused_pty(&self) -> Option<String> {
        let id = &self.state.focused;
        self.state.windows.iter().find(|w| &w.id == id).map(|w| w.pty.clone())
    }

    fn focused_win(&self) -> Option<&Win> {
        self.state.windows.iter().find(|w| w.id == self.state.focused)
    }

    /// Calls a verb whose answer something is waiting for.
    fn ask(&mut self, verb: &str, params: Value, reply: Reply) {
        self.next_req += 1;
        self.replies.insert(self.next_req, reply);
        self.send(json!({"cmd": "verb", "verb": verb, "req": self.next_req, "params": params}));
    }

    fn verb_answered(&mut self, reply: Reply, result: &Value) {
        match reply {
            Reply::PanePreview(value) => {
                let lines = result["content"].as_str().unwrap_or_default().lines().map(str::to_string).collect();
                self.pane_preview = Some((value, lines));
            }
            Reply::Agents(reply) => self.agents_answered(reply, result),
            Reply::Files(reply) => self.file_answered(reply, result),
            Reply::HostSplit { target, side } => {
                if let Some(id) = result["window_id"].as_str() {
                    self.host_split = Some((id.to_string(), target, side));
                    self.place_host_split();
                }
            }
        }
    }

    fn verb(&mut self, verb: &str, params: Value) {
        self.send(json!({"cmd": "verb", "verb": verb, "req": 2, "params": params}));
    }

    // ----- choosers -----

    fn open_themes(&mut self) {
        let mut picker = Picker::new(self.theme_items(), self.theme.name.clone());
        // With favourites, open at the top so that they are in view.
        if self.settings.theme.favourites.iter().any(|f| self.theme.names.contains(f)) {
            let favourite_in_force = self.settings.theme.favourites.contains(&self.theme.name);
            if !favourite_in_force {
                picker.select_first();
            }
        }
        self.overlay = Overlay::Chooser(ChooserKind::Theme, picker);
    }

    fn theme_items(&self) -> Vec<Item> {
        let items = self.theme.names.iter().map(|n| Item::new(theme::pretty(n), "", n.clone())).collect();
        favourites_first(items, &self.settings.theme.favourites)
    }

    /// Stars or unstars a theme, and saves it. The list reorders in place.
    fn toggle_favourite(&mut self, name: &str, cx: &mut Context<Self>) {
        let mut favourites = self.settings.theme.favourites.clone();
        match favourites.iter().position(|f| f == name) {
            Some(at) => {
                favourites.remove(at);
            }
            None => favourites.push(name.to_string()),
        }
        let mut list = toml_edit::Array::new();
        for f in &favourites {
            list.push(f.as_str());
        }
        self.change_setting(cx, |doc| doc["theme"]["favourites"] = toml_edit::value(list));
        let items = self.theme_items();
        if let Overlay::Chooser(ChooserKind::Theme, picker) = &mut self.overlay {
            picker.set_items(items);
        }
    }

    fn open_fonts(&mut self, cx: &mut Context<Self>) {
        let mut names = cx.text_system().all_font_names();
        names.retain(|n| !n.starts_with('.'));
        names.sort();
        names.dedup();
        // Terminal text sits on a grid, so only fonts with one width can draw it well.
        let text = cx.text_system().clone();
        names.retain(|n| {
            let id = text.resolve_font(&font(n.clone()));
            let width = |c| text.advance(id, px(13.0), c).ok().map(|s| s.width);
            width('i').is_some() && width('i') == width('m') && width('i') == width('.')
        });
        let items = names.into_iter().map(|n| Item::new(n.clone(), "", n)).collect();
        self.overlay = Overlay::Chooser(ChooserKind::Font, Picker::new(items, self.look.font.to_string()));
    }

    fn open_palette(&mut self) {
        let mut items = vec![
            Item::new("Changes in this pane", "⇧⌘D", "tessella_review"),
            Item::new("Worktrees", "", "tessella_worktrees"),
            Item::new("New worktree", "", "tessella_new_worktree"),
            Item::new("Fan out a prompt", "", "tessella_fan"),
            Item::new("Change theme", "⇧⌘T", "tessella_themes"),
            Item::new("Open settings", "⌘,", "tessella_settings"),
        ];
        items.extend(self.palette_rows.iter().cloned());
        self.overlay = Overlay::Chooser(ChooserKind::Palette, Picker::new(items, String::new()));
    }

    fn open_sessions(&mut self) {
        let items = self
            .fleet
            .sessions
            .iter()
            .map(|s| Item::new(s.name.clone(), format!("{} panes", s.panes), s.name.clone()))
            .collect();
        self.overlay = Overlay::Chooser(ChooserKind::Session, Picker::new(items, self.state.session.clone()));
    }

    fn open_workspaces(&mut self) {
        let occupied = self.state.occupied.clone().unwrap_or_default();
        let items = occupied
            .iter()
            .map(|n| {
                let name = self.state.workspace_names.get(n).cloned().unwrap_or_default();
                let panes = self.state.windows.iter().filter(|w| w.workspace == *n && !w.popup).count();
                let label = if name.is_empty() { format!("Workspace {n}") } else { format!("{n}  {name}") };
                Item::new(label, format!("{panes} panes"), n.to_string())
            })
            .collect();
        self.overlay = Overlay::Chooser(ChooserKind::Workspace, Picker::new(items, self.state.workspace.to_string()));
    }

    /// Every pane of this session, then the agents of the others.
    fn open_panes(&mut self) {
        let here = self.state.session.clone();
        let mut items: Vec<Item> = self
            .state
            .windows
            .iter()
            .filter(|w| !w.popup)
            .map(|w| Item::new(w.label(), format!("workspace {}", w.workspace), format!("{here}\t{}", w.id)))
            .collect();
        items.extend(
            self.fleet
                .agents
                .iter()
                .filter(|a| a.session != here)
                .map(|a| Item::new(a.name.clone(), a.session.clone(), format!("{}\t{}", a.session, a.window))),
        );
        self.overlay = Overlay::Chooser(ChooserKind::Pane, Picker::new(items, String::new()));
        self.pane_preview = None;
        self.preview();
    }

    /// Shows the pane the picker is on. The panes of this session are at hand, and
    /// the daemon has the others.
    fn preview_pane(&mut self, value: String) {
        if self.pane_preview.as_ref().is_some_and(|(v, _)| *v == value) {
            return;
        }
        let Some((session, window)) = value.split_once('\t') else { return };
        let (session, window) = (session.to_string(), window.to_string());
        let local = (session == self.state.session)
            .then(|| self.state.windows.iter().find(|w| w.id == window))
            .flatten()
            .and_then(|w| self.panes.get(&w.pty));
        if let Some(pane) = local {
            self.pane_preview = Some((value, pane.text_lines()));
        } else {
            self.ask("capture-pane", json!({"session": session, "window": window, "lines": 60}), Reply::PanePreview(value));
        }
    }

    fn preview(&mut self) {
        let current = match &self.overlay {
            Overlay::Chooser(ChooserKind::Theme, p) => p.current(),
            _ => None,
        };
        if let Some(item) = current {
            self.send(json!({"cmd": "theme", "theme": item.value}));
        }
        let pane = match &self.overlay {
            Overlay::Chooser(ChooserKind::Pane, p) => p.current(),
            _ => None,
        };
        if let Some(item) = pane {
            self.preview_pane(item.value);
        }
    }

    fn cancel_overlay(&mut self) {
        if let Overlay::Chooser(ChooserKind::Theme, p) = std::mem::take(&mut self.overlay) {
            self.send(json!({"cmd": "theme", "theme": p.original}));
        }
        self.back_to_settings();
    }

    /// A chooser opened from the settings page returns to it.
    fn back_to_settings(&mut self) {
        if std::mem::take(&mut self.settings_return) {
            self.overlay = Overlay::Settings(SettingsPage { section: Section::Appearance, capture: None });
        }
    }

    fn accept_chooser(&mut self, kind: ChooserKind, item: Item, cx: &mut Context<Self>) {
        self.overlay = Overlay::None;
        match kind {
            // Keeps the theme on screen, and writes it to the config for next time.
            ChooserKind::Theme => self.send(json!({"cmd": "theme", "theme": item.value, "persist": true})),
            ChooserKind::Font => {
                let family = item.value;
                self.change_setting(cx, |doc| doc["font"]["family"] = toml_edit::value(family));
            }
            ChooserKind::Palette => match item.value.strip_prefix(PALETTE_ROW) {
                Some(name) => self.send(json!({"cmd": "palette", "name": name})),
                None => self.run_action(item.value, cx),
            },
            ChooserKind::Session => self.switch_session(&item.value),
            ChooserKind::Workspace => {
                if let Ok(n) = item.value.parse::<i32>() {
                    self.send(json!({"cmd": "workspace", "n": n}));
                }
            }
            ChooserKind::Pane => {
                if let Some((session, window)) = item.value.split_once('\t') {
                    self.focus_anywhere(session, window);
                }
            }
            ChooserKind::Launcher => self.send(json!({"cmd": "launch", "path": item.value})),
            ChooserKind::Layout => self.send(json!({"cmd": "layout-load", "name": item.value})),
            ChooserKind::FileSearch => self.open_found(item.value, cx),
            ChooserKind::SendTo => {
                if let Some(entry) = self.menu_entry.take() {
                    self.send_to(entry.place(), item.value, cx);
                }
            }
        }
        self.back_to_settings();
    }

    fn switch_session(&mut self, name: &str) {
        if name != self.state.session {
            self.send(json!({"cmd": "switch-session", "name": name}));
        }
    }

    /// Focuses a pane, going to its session first when it is in another one.
    fn focus_anywhere(&mut self, session: &str, window: &str) {
        if session == self.state.session {
            let previous = self.state.focused.clone();
            self.send(json!({"cmd": "focus", "window": window}));
            self.reveal_if_cut_off(window, &previous);
        } else {
            self.verb("focus-window", json!({"session": session, "window": window}));
            self.switch_session(session);
        }
    }

    fn chooser_key(&mut self, k: &Keystroke, cx: &mut Context<Self>) {
        let Overlay::Chooser(kind, picker) = &mut self.overlay else { return };
        let kind = *kind;
        match k.key.as_str() {
            "escape" => return self.cancel_overlay(),
            "enter" => {
                if let Some(item) = picker.current() {
                    self.accept_chooser(kind, item, cx);
                }
                return;
            }
            "tab" => {
                if let Some(item) = picker.current() {
                    match kind {
                        ChooserKind::Theme => self.toggle_favourite(&item.value, cx),
                        ChooserKind::Layout => {
                            let title = format!("Delete layout {}?", item.value);
                            self.prompt(PromptKind::Confirm(Confirm::DeleteLayout(item.value)), &title, String::new());
                        }
                        _ => {}
                    }
                }
                return;
            }
            "up" => picker.step(-1),
            "down" => picker.step(1),
            "backspace" => picker.backspace(),
            "space" => picker.push(" "),
            _ => match k.key_char.as_deref() {
                Some(text) if !k.modifiers.control && !k.modifiers.platform => picker.push(text),
                _ => return,
            },
        }
        self.preview();
    }

    // ----- prompts -----

    fn prompt(&mut self, kind: PromptKind, title: &str, text: String) {
        let fresh = !text.is_empty();
        self.overlay = Overlay::Prompt(Prompt { kind, title: title.to_string(), text, fresh });
    }

    fn prompt_key(&mut self, k: &Keystroke, cx: &mut Context<Self>) {
        let Overlay::Prompt(p) = &mut self.overlay else { return };
        if let PromptKind::Confirm(what) = &p.kind {
            let what = what.clone();
            match k.key.as_str() {
                "y" | "enter" => {
                    self.overlay = Overlay::None;
                    self.confirmed(what);
                }
                "n" | "escape" => self.overlay = Overlay::None,
                _ => {}
            }
            return;
        }
        match k.key.as_str() {
            "escape" => self.overlay = Overlay::None,
            "enter" => {
                let Overlay::Prompt(p) = std::mem::take(&mut self.overlay) else { return };
                self.answered(p.kind, p.text.trim().to_string(), cx);
            }
            "backspace" => {
                if std::mem::take(&mut p.fresh) {
                    p.text.clear();
                } else {
                    p.text.pop();
                }
            }
            "space" => {
                if std::mem::take(&mut p.fresh) {
                    p.text.clear();
                }
                p.text.push(' ');
            }
            _ => {
                if let Some(text) = k.key_char.as_deref().filter(|_| !k.modifiers.control && !k.modifiers.platform) {
                    if std::mem::take(&mut p.fresh) {
                        p.text.clear();
                    }
                    p.text.push_str(text);
                }
            }
        }
    }

    fn confirmed(&mut self, what: Confirm) {
        let session = self.state.session.clone();
        match what {
            Confirm::ClosePane(window) => self.verb("close-window", json!({"session": session, "window": window})),
            Confirm::CloseWorkspace(n) => self.verb("close-workspace", json!({"session": session, "workspace": n})),
            Confirm::KillSession(name) => self.verb("kill-session", json!({"session": name})),
            Confirm::DeleteLayout(name) => self.send(json!({"cmd": "layout-delete", "name": name})),
            Confirm::Trash(path) => {
                let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
                let result = if tessella::native_input::trash(&path) {
                    Ok(format!("Moved {name} to the Trash."))
                } else {
                    Err(std::io::Error::other(format!("Could not move {name} to the Trash.")))
                };
                self.files_changed(result);
            }
            Confirm::RestoreCheckpoint { host, session, window, n } => self.restore_turn(&host, &session, &window, n),
            Confirm::RemoveWorktree { host, session } => self.remove_worktree(&host, &session),
            Confirm::DeleteForever(what) if !what.is_here() => self.far_delete(&what),
            Confirm::DeleteForever(what) => {
                let name = what.name();
                self.files_changed(tessella::files::delete_forever(&what.path).map(|()| format!("Deleted {name}.")));
            }
        }
    }

    fn answered(&mut self, kind: PromptKind, text: String, cx: &mut Context<Self>) {
        let session = self.state.session.clone();
        match kind {
            PromptKind::RenameWindow(window) => self.verb("set-window", json!({"session": session, "window": window, "name": text})),
            PromptKind::RenameWorkspace(n) => self.verb("set-workspace-name", json!({"session": session, "workspace": n, "name": text})),
            PromptKind::RenameSession if !text.is_empty() => self.verb("rename-session", json!({"session": session, "name": text})),
            PromptKind::NewFile(dir) if !dir.is_here() => self.far_new(&dir, &text, cx),
            PromptKind::NewFile(dir) => {
                let made = tessella::files::create(&dir.path, &text);
                self.files_changed(made.map(|p| format!("Made {}.", p.display())));
            }
            PromptKind::RenameFile(from) if !from.is_here() => self.far_rename(&from, &text),
            PromptKind::RenameFile(from) => {
                let done = tessella::files::rename(&from.path, &text);
                self.files_changed(done.map(|p| format!("Renamed to {}.", p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()))));
            }
            PromptKind::SaveLayout if !text.is_empty() => {
                self.send(json!({"cmd": "layout-save", "name": text}));
                self.status = Some(format!("Saved layout {text}."));
            }
            PromptKind::NewSession => {
                // The bridge can create what it switches to, so a named one opens at once.
                if text.is_empty() {
                    self.verb("new-session", json!({}));
                } else {
                    self.send(json!({"cmd": "switch-session", "name": text, "create": true}));
                }
            }
            _ => {}
        }
    }

    // ----- new pane -----

    /// With other machines known, asks where the pane runs. With none, there is
    /// nothing to ask.
    fn new_pane(&mut self) {
        if self.machines.len() > 1 {
            self.overlay = Overlay::Machines(MachinePicker::default());
        } else {
            self.send(json!({"cmd": "action", "name": "new_window"}));
        }
    }

    /// Opens a pane on the focused pane's machine. tuios's split opens one here, and
    /// new-window takes a machine but not a place, so the new pane moves beside the
    /// old one once it is in the layout. If the machine cannot open it, that is said,
    /// and no pane opens here by mistake.
    fn split_on_host(&mut self, win: Win, side: &'static str) {
        let mut params = json!({"session": self.state.session, "host": win.host, "workspace": win.workspace, "focus": true});
        if !win.cwd.is_empty() {
            params["cwd"] = json!(win.cwd);
        }
        self.ask("new-window", params, Reply::HostSplit { target: win.id, side });
    }

    /// Brings a pane just focused all the way on screen, if the scrolling layout has part
    /// of it off. Asking for a pane is asking to see it. The bridge has no command that
    /// only does this, but tuios does it after a focus key, and going to the last pane
    /// is such a key. A step to another pane on the strip makes this one the last pane,
    /// whatever tuios remembered: that pane may have closed, or be on another workspace.
    fn reveal_if_cut_off(&mut self, id: &str, previous: &str) {
        let b = self.state.layout_box();
        if !self.state.is_cut_off(id, b.x, self.layout_cells().0) {
            return;
        }
        if let Some(step) = self.state.reveal_step(id, previous) {
            self.send(json!({"cmd": "focus", "window": step}));
            self.send(json!({"cmd": "action", "name": "last_pane"}));
        }
    }

    /// Moves a split's new pane beside its old one, once the layout holds it.
    fn place_host_split(&mut self) {
        let Some((id, target, side)) = self.host_split.clone() else { return };
        let Some(win) = self.state.windows.iter().find(|w| w.id == id) else { return };
        self.host_split = None;
        // With no split tree, as in the scrolling layout, the layout places it.
        if self.state.trees.contains_key(&win.workspace.to_string()) {
            self.send(json!({"cmd": "layout", "op": "move", "id": id, "target": target, "side": side}));
        }
    }

    fn start_pane_on(&mut self, host: Option<String>) {
        self.overlay = Overlay::None;
        match host {
            None => self.send(json!({"cmd": "action", "name": "new_window"})),
            Some(host) => {
                let session = self.state.session.clone();
                let workspace = self.state.workspace;
                self.verb("new-window", json!({"session": session, "host": host, "workspace": workspace}));
            }
        }
    }

    fn machine_key(&mut self, k: &Keystroke) {
        let machines = self.machines.clone();
        let Overlay::Machines(p) = &mut self.overlay else { return };
        match k.key.as_str() {
            "escape" => self.overlay = Overlay::None,
            "up" => p.step(&machines, -1),
            "down" => p.step(&machines, 1),
            "enter" => {
                if let Some(machine) = p.current(&machines) {
                    let host = machine.host.clone();
                    self.start_pane_on(host);
                }
            }
            _ => {}
        }
    }

    // ----- actions -----

    /// Carries out a tuios action. A real client does some of these itself, so
    /// the bridge refuses them: those are done here, as near as they can be.
    fn run_action(&mut self, action: String, cx: &mut Context<Self>) {
        self.status = None;
        // A split of a pane on another machine opens on that machine too.
        if let Some(side) = model::split_side(&action) {
            if let Some(win) = self.focused_win().filter(|w| !w.host.is_empty()).cloned() {
                return self.split_on_host(win, side);
            }
        }
        match action.as_str() {
            "new_window" | "prefix_new_window" | "window_prefix_new" => self.new_pane(),
            "tessella_themes" => self.open_themes(),
            "tessella_review" => self.open_changes(),
            "tessella_worktrees" => self.open_agents(agents_view::AgentsTab::Work),
            "tessella_new_worktree" => self.start_work(false),
            "tessella_fan" => self.start_work(true),
            "tessella_new_pane" => self.new_pane(),
            "tessella_new_workspace" | "new_workspace" => self.new_workspace(),
            "tessella_settings" | "prefix_settings" => self.overlay = Overlay::Settings(SettingsPage::default()),
            "close_window" | "prefix_close_window" | "window_prefix_close" if self.settings.panes.confirm_close => {
                if let Some(w) = self.focused_win().cloned() {
                    self.prompt(PromptKind::Confirm(Confirm::ClosePane(w.id.clone())), &format!("Close {}?", w.label()), String::new());
                }
            }
            "command_palette" | "prefix_command_palette" => self.open_palette(),
            "launcher" | "gui_launcher" => self.send(json!({"cmd": "launcher"})),
            "prefix_help" | "prefix_keybinds" | "gui_keys" => self.overlay = Overlay::Help,
            "prefix_toggle_sidebar" | "gui_toggle_sidebar" => {
                self.toggle_rail();
            }
            "prefix_explore" => self.focus_rail(rail::Section::Files),
            "prefix_inbox" | "gui_inbox" => self.open_agents(agents_view::AgentsTab::Inbox),
            "prefix_mail" => self.open_agents(agents_view::AgentsTab::Mail),
            "prefix_session_switcher" => self.open_sessions(),
            "prefix_workspace_switcher" => self.open_workspaces(),
            "choose_tree" | "goto_picker" => self.open_panes(),
            "display_panes" | "gui_pane_picker" => self.overlay = Overlay::PaneLabels,
            "gui_save_layout" | "layout_prefix_save" => self.prompt(PromptKind::SaveLayout, "Save layout as", String::new()),
            "gui_load_layout" | "layout_prefix_load" => self.send(json!({"cmd": "layouts"})),
            "prefix_rename_window" | "window_prefix_rename" | "gui_rename" => {
                if let Some(w) = self.focused_win() {
                    let (id, name) = (w.id.clone(), w.name.clone());
                    self.prompt(PromptKind::RenameWindow(id), "Rename pane", name);
                }
            }
            "rename_workspace" | "workspace_prefix_rename" => {
                let n = self.state.workspace;
                let name = self.state.workspace_names.get(&n).cloned().unwrap_or_default();
                self.prompt(PromptKind::RenameWorkspace(n), "Rename workspace", name);
            }
            "rename_session" => {
                let name = self.state.session.clone();
                self.prompt(PromptKind::RenameSession, "Rename session", name);
            }
            "close_workspace" => {
                let n = self.state.workspace;
                self.prompt(PromptKind::Confirm(Confirm::CloseWorkspace(n)), &format!("Close workspace {n} and its panes?"), String::new());
            }
            "prefix_close_session" => {
                let name = self.state.session.clone();
                self.prompt(PromptKind::Confirm(Confirm::KillSession(name.clone())), &format!("End session {name} and its panes?"), String::new());
            }
            "prefix_detach" | "prefix_quit" | "gui_quit" => cx.quit(),
            "reload_config" => self.verb("apply-config", json!({})),
            "paste_buffer" | "gui_paste" | "terminal_paste_host" => self.paste(cx),
            "prefix_selection" | "prefix_scrollback" | "gui_copy_mode" => self.enter_copy(CopyMode::default()),
            "gui_find" => self.enter_copy(CopyMode::searching()),
            "gui_copy" => self.copy_selection(cx),
            "terminal_scroll_up" => self.scroll_focused(1),
            "terminal_scroll_down" => self.scroll_focused(-1),
            "terminal_scroll_page_up" => self.scroll_focused_pages(true),
            "terminal_scroll_page_down" => self.scroll_focused_pages(false),
            "gui_font_bigger" => self.zoom(1.0, cx),
            "gui_font_smaller" => self.zoom(-1.0, cx),
            "gui_font_reset" => self.set_font_size(self.settings.font.size),
            "gui_resize_left" => self.run_action("resize_pane_left_5".into(), cx),
            "gui_resize_right" => self.run_action("resize_pane_right_5".into(), cx),
            "gui_resize_up" => self.run_action("resize_pane_up_5".into(), cx),
            "gui_resize_down" => self.run_action("resize_pane_down_5".into(), cx),
            "gui_switcher" | "gui_switcher_back" => self.send(json!({"cmd": "action", "name": "last_pane"})),
            "gui_toggle_float" => self.send(json!({"cmd": "action", "name": "window_prefix_float"})),
            "last_session" => {
                if let Some(name) = self.last_session.clone() {
                    self.switch_session(&name);
                }
            }
            "break_pane" => self.break_pane(),
            a if a.ends_with("_cancel") || a == "prefix_exit_mode" => {}
            other => match self.state.resolve(other) {
                Resolved::Action(name) => self.send(json!({"cmd": "action", "name": name})),
                Resolved::Workspace(n) => self.send(json!({"cmd": "workspace", "n": n})),
                Resolved::Order(order) => {
                    let session = self.state.session.clone();
                    self.verb("set-workspace-order", json!({"session": session, "order": order}));
                }
                Resolved::Nothing => {}
            },
        }
    }

    /// Opens a pane on the first empty workspace, and goes there.
    fn new_workspace(&mut self) {
        let occupied = self.state.occupied.clone().unwrap_or_default();
        let Some(n) = (1..=self.state.num_workspaces).find(|n| !occupied.contains(n)) else {
            self.status = Some("Every workspace has panes.".into());
            return;
        };
        let session = self.state.session.clone();
        // An empty workspace keeps the name of what it last held, which is not this.
        if self.state.workspace_names.get(&n).is_some_and(|name| !name.is_empty()) {
            self.verb("set-workspace-name", json!({"session": session, "workspace": n, "name": ""}));
        }
        self.verb("new-window", json!({"session": session, "workspace": n}));
        self.send(json!({"cmd": "workspace", "n": n}));
    }

    /// Opens the settings file in a new pane, making it first if needed.
    fn open_settings_file(&mut self) {
        match settings::ensure_file() {
            Ok(path) => self.open_file(tessella::files::Place::new("", path)),
            Err(e) => self.status = Some(format!("Could not open the settings: {e}")),
        }
    }

    /// Moves the focused pane to the first empty workspace, and follows it.
    fn break_pane(&mut self) {
        let occupied = self.state.occupied.clone().unwrap_or_default();
        let empty = (1..=self.state.num_workspaces).find(|n| !occupied.contains(n));
        let window = self.state.focused.clone();
        let session = self.state.session.clone();
        match empty {
            Some(n) if !window.is_empty() => {
                self.verb("move-window", json!({"session": session, "window": window, "workspace": n}));
                self.send(json!({"cmd": "workspace", "n": n}));
            }
            _ => self.status = Some("No empty workspace to move this pane to.".into()),
        }
    }

    // ----- clipboard -----

    /// A check names a file with this, so that it never touches the real clipboard.
    fn test_clipboard() -> Option<PathBuf> {
        std::env::var_os("TESSELLA_TEST_CLIPBOARD").map(PathBuf::from)
    }

    fn write_clipboard(&mut self, text: String, cx: &mut Context<Self>) {
        self.status = Some(format!("Copied {} characters.", text.chars().count()));
        match Self::test_clipboard() {
            Some(path) => {
                let _ = std::fs::write(path, text);
            }
            None => cx.write_to_clipboard(ClipboardItem::new_string(text)),
        }
    }

    /// A program in a pane asked to set the clipboard (OSC 52), which is how one on
    /// another machine reaches this Mac's clipboard. Anything a pane prints can carry
    /// this, so it follows tuios's appearance.selection.osc52_write, with "focused" as
    /// the default. Tessella has no way to ask, so a pane that would be asked is refused.
    fn pane_copied(&mut self, pty: &str, text: String, cx: &mut Context<Self>) {
        let Some(win) = self.state.windows.iter().find(|w| w.pty == pty) else { return };
        let (name, focused) = (win.label(), win.id == self.state.focused);
        if text.is_empty() || self.osc52_write == "off" {
            return;
        }
        if self.osc52_write == "on" || (self.osc52_write == "focused" && focused) {
            let count = text.chars().count();
            self.write_clipboard(text, cx);
            self.status = Some(format!("{name} copied {count} characters."));
        } else {
            self.status = Some(format!("{name} tried to copy to the clipboard. Only the focused pane can."));
        }
    }

    fn read_clipboard(&self, cx: &mut Context<Self>) -> Option<String> {
        match Self::test_clipboard() {
            Some(path) => std::fs::read_to_string(path).ok(),
            None => cx.read_from_clipboard().and_then(|c: ClipboardItem| c.text()),
        }
    }

    fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let pty = self.copy.as_ref().map(|(pty, _)| pty.clone()).or_else(|| self.focused_pty());
        let text = pty.and_then(|p| self.panes.get(&p)).and_then(Pane::selected_text);
        if let Some(text) = text {
            self.write_clipboard(text, cx);
        }
    }

    /// An image on the clipboard with no text, which a text paste could not carry.
    fn clipboard_image(&self, cx: &mut Context<Self>) -> Option<(Vec<u8>, &'static str)> {
        if let Some(path) = std::env::var_os("TESSELLA_TEST_CLIPBOARD_IMAGE") {
            return std::fs::read(path).ok().map(|bytes| (bytes, "png"));
        }
        if Self::test_clipboard().is_some() {
            return None;
        }
        cx.read_from_clipboard()?.entries().iter().find_map(|entry| match entry {
            gpui::ClipboardEntry::Image(image) => {
                let extension = match image.format {
                    gpui::ImageFormat::Png => "png",
                    gpui::ImageFormat::Jpeg => "jpg",
                    gpui::ImageFormat::Gif => "gif",
                    gpui::ImageFormat::Webp => "webp",
                    gpui::ImageFormat::Bmp => "bmp",
                    gpui::ImageFormat::Tiff => "tiff",
                    gpui::ImageFormat::Svg => "svg",
                };
                Some((image.bytes.clone(), extension))
            }
            _ => None,
        })
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.read_clipboard(cx) else {
            if let Some((bytes, extension)) = self.clipboard_image(cx) {
                self.paste_image(&bytes, extension);
            }
            return;
        };
        let Some(pty) = self.focused_pty() else { return };
        self.paste_text(&pty, &text);
    }

    /// Types text into a pane as a paste.
    fn paste_text(&mut self, pty: &str, text: &str) {
        let bracketed = self.panes.get(pty).is_some_and(Pane::bracketed_paste);
        // A pasted newline is a carriage return to a terminal program.
        let body = text.replace("\r\n", "\r").replace('\n', "\r");
        let bytes = if bracketed { format!("\x1b[200~{body}\x1b[201~") } else { body };
        if let Some(pane) = self.panes.get_mut(pty) {
            pane.predict_reset();
        }
        if let Err(err) = self.writer.input(pty, bytes.as_bytes()) {
            self.status = Some(format!("Could not paste: {err}"));
        }
    }

    // ----- copy mode -----

    fn enter_copy(&mut self, mode: CopyMode) {
        let Some(pty) = self.focused_pty() else { return };
        self.leave_copy();
        if let Some(pane) = self.panes.get_mut(&pty) {
            pane.enter_copy();
            self.copy = Some((pty, mode));
        }
    }

    fn leave_copy(&mut self) {
        if let Some((pty, _)) = self.copy.take() {
            if let Some(pane) = self.panes.get_mut(&pty) {
                pane.exit_copy();
            }
        }
    }

    fn copy_key(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some((pty, mode)) = self.copy.as_mut() else { return };
        let Some(pane) = self.panes.get_mut(pty.as_str()) else { return };
        match mode.key(pane, name) {
            Effect::Stay => self.status = None,
            Effect::Note(note) => self.status = Some(note),
            Effect::Exit => {
                self.status = None;
                self.leave_copy();
            }
            Effect::Yank(text) => {
                self.leave_copy();
                self.write_clipboard(text, cx);
            }
        }
    }

    // ----- mouse selection -----

    /// The cell of a pane under a point, and which half of it. A point outside
    /// the pane gives the nearest cell, and how far past its top or bottom it is.
    fn cell_at(&self, win: &Win, position: Point<Pixels>, window: &Window) -> (usize, usize, Side, i32) {
        let m = metrics(&self.look, window);
        let s = self.slots(window);
        let fit = self.fit_for(win, s, m);
        let m = Metrics { cell_w: m.cell_w * fit, cell_h: m.cell_h * fit };
        let left = ui(STAGE_PAD) + (win.x - s.x) as f32 * s.w + ui(MARGIN) + ui(PAD_X);
        let top = ui(TOPBAR) + ui(STAGE_PAD) + (win.y - s.y) as f32 * s.h + ui(MARGIN) + ui(HEADER);
        let col = (f32::from(position.x) - left) / f32::from(m.cell_w);
        let row = ((f32::from(position.y) - top) / f32::from(m.cell_h)).floor() as i32;
        let rows = self.panes.get(&win.pty).map_or(1, |p| p.size().1 as i32);
        let past = if row < 0 { row } else { (row - rows + 1).max(0) };
        let side = if col.fract() < 0.5 { Side::Left } else { Side::Right };
        (col.max(0.0) as usize, row.clamp(0, rows - 1) as usize, side, past)
    }

    fn scroll_focused(&mut self, lines: i32) {
        if let Some(pane) = self.focused_pty().and_then(|p| self.panes.get_mut(&p)) {
            pane.scroll(lines);
        }
    }

    fn scroll_focused_pages(&mut self, up: bool) {
        if let Some(pane) = self.focused_pty().and_then(|p| self.panes.get_mut(&p)) {
            pane.scroll_pages(up);
        }
    }

    // ----- sidebar -----

    fn rail_groups(&self) -> Vec<Group> {
        rail::build(&rail::Inputs {
            state: &self.state,
            fleet: &self.fleet,
            files: &self.files,
            filter: self.rail_filter,
            collapsed: &self.rail_collapsed,
        })
    }

    fn focus_rail(&mut self, section: rail::Section) {
        if self.rail_focused {
            self.rail_focused = false;
            return;
        }
        self.rail_open = true;
        self.rail_focused = true;
        self.rail_collapsed.retain(|s| *s != section);
        if let Some(at) = rail::section_start(&self.rail_groups(), section) {
            self.set_rail_cursor(at);
        }
    }

    /// Moves the cursor, and scrolls its row into view.
    fn set_rail_cursor(&mut self, at: usize) {
        self.rail_cursor = at;
        let groups = self.rail_groups();
        let mut top = 0;
        let mut left = at;
        for g in &groups {
            let is_agents = g.section == rail::Section::Agents;
            if left < g.rows.len() {
                if is_agents {
                    self.agents_scroll.scroll_to_item(left);
                } else {
                    // Each section above the agents is a header and its rows.
                    self.rail_scroll.scroll_to_item(top + 1 + left);
                }
                return;
            }
            left -= g.rows.len();
            top += 1 + g.rows.len();
        }
    }

    fn activate_row(&mut self, row: Row, cx: &mut Context<Self>) {
        match row {
            Row::Session { name, .. } => {
                self.switch_session(&name);
                self.rail_focused = false;
            }
            Row::Terminal { id, .. } => {
                let previous = self.state.focused.clone();
                self.send(json!({"cmd": "focus", "window": id}));
                self.reveal_if_cut_off(&id, &previous);
                self.rail_focused = false;
            }
            Row::Agent { window, session, .. } => {
                self.focus_anywhere(&session, &window);
                self.rail_focused = false;
            }
            Row::File(e) if e.dir => {
                self.files_dir = Some(e.path);
                self.files_read = None;
                self.refresh_files();
                self.rail_cursor = rail::section_start(&self.rail_groups(), rail::Section::Files).unwrap_or(0);
            }
            Row::File(e) => self.open_file(e.place()),
        }
        cx.notify();
    }

    /// Opens a file in a new pane: Markdown drawn, and anything else in the editor.
    /// A Markdown pane is drawn here, wherever its file is.
    fn open_file(&mut self, place: tessella::files::Place) {
        if !tessella::markdown::is_markdown(&place.path) {
            return self.edit_file(place);
        }
        let session = self.state.session.clone();
        let workspace = self.state.workspace;
        let name = place.name();
        let uri = tessella::markdown::file_uri(&place);
        self.verb("new-window", json!({"session": session, "name": name, "workspace": workspace, "focus": true, "kind": "view", "uri": uri}));
        self.rail_focused = false;
    }

    /// Opens a file in a new pane, in the person's editor on the file's machine.
    fn edit_file(&mut self, place: tessella::files::Place) {
        self.edit_file_at(place, None);
    }

    /// As `edit_file`, at a line for the editors known to take `+N`.
    fn edit_file_at(&mut self, place: tessella::files::Place, line: Option<u32>) {
        let session = self.state.session.clone();
        let name = place.name();
        let path = place.path.to_string_lossy().into_owned();
        let workspace = self.state.workspace;
        let mut params = json!({"session": session, "name": name, "workspace": workspace, "focus": true,
            "command": ["sh", "-c", EDIT_SCRIPT, path, line.map_or(String::new(), |n| n.to_string())]});
        if !place.is_here() {
            params["host"] = json!(place.host);
            // This machine's folders need not exist there.
            params["cwd"] = json!(place.path.parent().unwrap_or(Path::new("~")).to_string_lossy());
        }
        self.verb("new-window", params);
        self.rail_focused = false;
    }

    /// Types a cd into the focused pane, when it sits at a shell prompt.
    fn cd_here(&mut self) {
        let Some(dir) = self.files_shown.clone() else { return };
        let Some(w) = self.focused_win().cloned() else { return };
        if dir.host != self.files_host_of(&w) {
            return;
        }
        let dir = dir.path;
        if !w.foreground.is_empty() {
            self.status = Some(format!("{} is running in this pane, so it would get the cd as input.", w.foreground));
            return;
        }
        let quoted = dir.to_string_lossy().replace('\'', "'\\''");
        let _ = self.writer.input(&w.pty, format!("cd '{quoted}'\r").as_bytes());
    }

    fn rail_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let groups = self.rail_groups();
        let count = rail::row_count(&groups);
        let at = self.rail_cursor.min(count.saturating_sub(1));
        let current = rail::row_at(&groups, at).map(|(s, r)| (s, r.clone()));
        // A file row has keys of its own, and they come first.
        if let Some((_, Row::File(entry))) = &current {
            let file_action = self.keymap.scope_action(FILE_SCOPE, key).map(str::to_string);
            if let Some(action) = file_action {
                return self.file_action(&action, Some(entry.clone()), cx);
            }
        }
        if let Some((_, Row::Agent { window, session, name, .. })) = &current {
            let agent_action = self.keymap.scope_action("sidebar.agents", key).map(str::to_string);
            if let Some(action) = agent_action {
                let agent = rail::Agent { window: window.clone(), session: session.clone(), name: name.clone(), ..Default::default() };
                return self.agent_action(&action, &agent);
            }
        }
        let Some(action) = self.keymap.scope_action("sidebar", key).map(str::to_string) else {
            return;
        };
        match action.as_str() {
            "exit" => self.rail_focused = false,
            "cursor_down" if count > 0 => self.set_rail_cursor((at + 1).min(count - 1)),
            "cursor_up" => self.set_rail_cursor(at.saturating_sub(1)),
            "first" => self.set_rail_cursor(0),
            "last" => self.set_rail_cursor(count.saturating_sub(1)),
            "section" => self.set_rail_cursor(rail::next_section(&groups, at, 1)),
            "file_search" => self.file_action("file_search", None, cx),
            "menu" => {
                if let Some((_, Row::File(entry))) = current {
                    let (w, _) = self.viewport;
                    self.open_file_menu(Some(entry), point(px(w - ui(self.rail_width) - 250.0), px(ui(TOPBAR) + 60.0)));
                }
            }
            "activate" => {
                if let Some((_, row)) = current {
                    self.activate_row(row, cx);
                }
            }
            "collapse" | "expand" => {
                if let Some((section, _)) = current {
                    if action == "collapse" && !self.rail_collapsed.contains(&section) {
                        self.rail_collapsed.push(section);
                    } else if action == "expand" {
                        self.rail_collapsed.retain(|s| *s != section);
                    }
                    let groups = self.rail_groups();
                    self.rail_cursor = self.rail_cursor.min(rail::row_count(&groups).saturating_sub(1));
                }
            }
            "agents_filter" => self.rail_filter = self.rail_filter.next(),
            "narrow" => self.rail_width = (self.rail_width - 20.0).max(SIDEBAR_MIN),
            "widen" => self.rail_width = (self.rail_width + 20.0).min(SIDEBAR_MAX),
            "new_session" => self.prompt(PromptKind::NewSession, "New session name", String::new()),
            "new_window" => self.new_pane(),
            "palette" => self.open_palette(),
            "help" => self.overlay = Overlay::Help,
            "rename" => match current.map(|(_, r)| r) {
                Some(Row::Session { name, current: true, .. }) => self.prompt(PromptKind::RenameSession, "Rename session", name),
                Some(Row::Terminal { id, label, .. }) => self.prompt(PromptKind::RenameWindow(id), "Rename pane", label),
                Some(Row::Agent { window, name, .. }) => self.prompt(PromptKind::RenameWindow(window), "Rename pane", name),
                _ => {}
            },
            "kill" => match current.map(|(_, r)| r) {
                Some(Row::Session { name, .. }) => {
                    self.prompt(PromptKind::Confirm(Confirm::KillSession(name.clone())), &format!("End session {name} and its panes?"), String::new())
                }
                Some(Row::Terminal { id, label, .. }) => {
                    self.prompt(PromptKind::Confirm(Confirm::ClosePane(id)), &format!("Close {label}?"), String::new())
                }
                Some(Row::Agent { window, name, .. }) => {
                    self.prompt(PromptKind::Confirm(Confirm::ClosePane(window)), &format!("Close {name}?"), String::new())
                }
                _ => {}
            },
            a => {
                if let Some(n) = a.strip_prefix("jump_").and_then(|n| n.parse().ok()) {
                    self.set_rail_cursor(rail::jump(&groups, at, n));
                } else {
                    let name = self.keymap.description(a).unwrap_or(a).to_string();
                    self.status = Some(format!("{name} isn't in Tessella yet."));
                }
            }
        }
    }

    /// Keeps the files section on the focused pane's folder.
    fn refresh_files(&mut self) {
        let Some(w) = self.focused_win().cloned() else { return };
        if w.id != self.files_pane {
            self.files_pane = w.id.clone();
            self.files_dir = None;
            self.files_read = None;
        }
        let host = self.files_host_of(&w);
        if !host.is_empty() {
            // A pane that has not said where it is starts at its machine's home folder.
            let doc_folder = w.markdown_place().filter(|doc| !doc.is_here()).and_then(|doc| doc.path.parent().map(Path::to_path_buf));
            let dir = self.files_dir.clone().or(doc_folder).unwrap_or_else(|| PathBuf::from(if w.cwd.is_empty() { "~" } else { &w.cwd }));
            return self.refresh_far_files(tessella::files::Place::new(&host, dir));
        }
        let Some(dir) = self.files_dir.clone().or_else(|| (!w.cwd.is_empty()).then(|| PathBuf::from(&w.cwd))) else {
            return;
        };
        let place = tessella::files::Place::new("", dir);
        let fresh = self.files_read.is_some_and(|t| t.elapsed() < Duration::from_secs(3));
        if self.files_shown.as_ref() == Some(&place) && fresh {
            return;
        }
        self.files_want = None;
        self.files = rail::read_dir(&place.path);
        self.files_shown = Some(place);
        self.files_read = Some(Instant::now());
    }

    // ----- input -----

    fn on_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let k = event.keystroke.clone();
        let m = &k.modifiers;
        match self.overlay {
            Overlay::None => {}
            Overlay::Chooser(..) => {
                self.chooser_key(&k, cx);
                return cx.notify();
            }
            Overlay::Machines(_) => {
                self.machine_key(&k);
                return cx.notify();
            }
            Overlay::Prompt(_) => {
                self.prompt_key(&k, cx);
                return cx.notify();
            }
            Overlay::Settings(_) => {
                self.settings_key(&k, cx);
                return cx.notify();
            }
            Overlay::Help => {
                if k.key == "escape" || k.key == "q" || k.key_char.as_deref() == Some("?") {
                    self.overlay = Overlay::None;
                }
                return cx.notify();
            }
            Overlay::Menu(_) => {
                self.menu_key(&k, cx);
                return cx.notify();
            }
            Overlay::Agents(_) => {
                let name = keymap::key_name(&k.key, k.key_char.as_deref(), m.control, m.alt, m.shift, m.platform);
                self.agents_key(&k, &name);
                return cx.notify();
            }
            Overlay::PaneLabels => {
                self.overlay = Overlay::None;
                if let Some(n) = k.key.parse::<usize>().ok().filter(|n| *n > 0) {
                    let target = self.labelled_panes().get(n - 1).map(|w| w.id.clone());
                    if let Some(id) = target {
                        self.send(json!({"cmd": "focus", "window": id}));
                    }
                }
                return cx.notify();
            }
        }
        let name = keymap::key_name(&k.key, k.key_char.as_deref(), m.control, m.alt, m.shift, m.platform);
        // Copy mode takes every key but the app's own chords, as in tuios.
        if self.copy.is_some() && !m.platform {
            self.copy_key(&name, cx);
            return cx.notify();
        }
        if self.rail_focused && !self.keymap.in_prefix() && name != self.keymap.leader() {
            self.rail_key(&name, cx);
            return cx.notify();
        }
        let now = self.started.elapsed().as_millis() as u64;
        match self.keymap.press(&name, now) {
            Step::Consumed => {
                self.prefix_at = Some(Instant::now());
                let wait = self.settings.keys.which_key_ms;
                if wait > 0 {
                    Self::wake_after(Duration::from_millis(wait + 10), cx);
                }
                return cx.notify();
            }
            Step::Action(action) => {
                // Handled: AppKit otherwise offers a key equivalent again.
                cx.stop_propagation();
                self.run_action(action, cx);
                return cx.notify();
            }
            Step::SendLeader => {
                let leader = keys::encode_name(self.keymap.leader());
                if let (Some(pty), Some(bytes)) = (self.focused_pty(), leader) {
                    let _ = self.writer.input(&pty, &bytes);
                }
                return cx.notify();
            }
            Step::Forward => {}
        }
        if m.platform && (m.control || m.alt || m.shift) {
            return;
        }
        if !m.platform && self.doc_key(&name) {
            return cx.notify();
        }
        let Some(pty) = self.focused_pty() else { return };
        let app_cursor = self.panes.get(&pty).is_some_and(Pane::app_cursor);
        let kitty = self.panes.get(&pty).is_some_and(Pane::kitty_keyboard);
        let mods = Mods { ctrl: m.control, alt: m.alt, shift: m.shift };
        let bytes = if m.platform { keys::cmd_chord(&k.key) } else { keys::encode(&k.key, k.key_char.as_deref(), mods, app_cursor, kitty) };
        if let Some(bytes) = bytes {
            if let Some(pane) = self.panes.get_mut(&pty) {
                pane.scroll_to_bottom();
                pane.select_clear();
            }
            if let Err(err) = self.writer.input(&pty, &bytes) {
                self.status = Some(format!("Could not type: {err}"));
            }
            if self.predicts_typing() {
                if let Some(pane) = self.panes.get_mut(&pty) {
                    if m.platform {
                        pane.predict_reset();
                    } else {
                        pane.predict_typed(&bytes);
                    }
                }
                // A guess with no echo has to leave the screen with no output to prompt it.
                Self::wake_after(predict::PATIENCE + Duration::from_millis(20), cx);
            }
            cx.notify();
        }
    }

    /// Whether the focused pane runs on another machine, where an echo takes a while.
    fn predicts_typing(&self) -> bool {
        self.settings.panes.predict_remote_typing && (self.target_host.is_some() || self.focused_win().is_some_and(|w| !w.host.is_empty()))
    }

    /// The scale a pane's text is drawn at. Tiled panes on a workspace share the
    /// smallest, so that text stays one size across them. A popup has its own.
    fn fit_for(&self, win: &Win, s: Slots, m: Metrics) -> f32 {
        if self.grid_pending.is_some() || self.size_pending() || self.layout_move.is_some() || self.fit_hold.is_some_and(|until| Instant::now() < until) {
            return 1.0;
        }
        let fit_of = |w: &Win| {
            let (cols, rows) = self.panes.get(&w.pty).map_or((0, 0), |p| p.size());
            text_fit(w, cols, rows, s, m)
        };
        if win.popup {
            return fit_of(win);
        }
        self.state
            .windows
            .iter()
            .filter(|w| w.workspace == win.workspace && !w.minimized && !w.popup)
            .map(fit_of)
            .fold(1.0, f32::min)
    }

    /// The pane under a point of the window.
    fn window_under(&self, position: Point<Pixels>, window: &Window) -> Option<&Win> {
        let s = self.slots(window);
        let col = ((f32::from(position.x) - ui(STAGE_PAD)) / s.w).floor() as i32 + s.x;
        let row = ((f32::from(position.y) - ui(TOPBAR) - ui(STAGE_PAD)) / s.h).floor() as i32 + s.y;
        model::window_at(&self.state.windows, self.state.workspace, col, row, s.gap)
    }

    fn on_root_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.agents_drag || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let height = f32::from(window.viewport_size().height);
        let below = height - f32::from(event.position.y);
        // The line, the section's header and its bottom padding sit below the pointer.
        let list = below / ui(1.0) - AGENTS_CHROME;
        let most = (height / ui(1.0) - 180.0).min(AGENTS_MAX);
        self.agents_height = list.clamp(AGENTS_MIN, most.max(AGENTS_MIN));
        cx.notify();
    }

    fn on_root_release(&mut self, _event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.agents_drag) {
            let height = self.agents_height.round();
            self.change_setting(cx, |doc| doc["sidebar"]["agents_height"] = toml_edit::value(height as f64));
            cx.notify();
        }
    }

    /// Scrolls the history. A full-screen program gets arrows, as a terminal sends.
    fn on_stage_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = metrics(&self.look, window);
        let doc = self.window_under(event.position, window).filter(|w| w.markdown_place().is_some()).map(|w| w.id.clone());
        if let Some(id) = doc {
            self.scroll_doc(&id, -f32::from(event.delta.pixel_delta(m.cell_h).y));
            return cx.notify();
        }
        self.wheel += f32::from(event.delta.pixel_delta(m.cell_h).y) / f32::from(m.cell_h);
        let lines = self.wheel.trunc() as i32;
        if lines == 0 {
            return;
        }
        self.wheel -= lines as f32;
        let Some(win) = self.window_under(event.position, window).cloned() else { return };
        let (col, row, ..) = self.cell_at(&win, event.position, window);
        let pty = win.pty;
        let Some(pane) = self.panes.get_mut(&pty) else { return };
        // A program that asked for the mouse scrolls itself, as under tuios.
        if let Some(report) = pane.wheel_report(lines > 0, col, row) {
            let _ = self.writer.input(&pty, &report.repeat(lines.unsigned_abs() as usize));
        } else if pane.alt_screen() {
            let arrow: &[u8] = match (lines > 0, pane.app_cursor()) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            let _ = self.writer.input(&pty, &arrow.repeat(lines.unsigned_abs() as usize));
        } else {
            pane.scroll(lines);
        }
        cx.notify();
    }

    // ----- geometry -----

    /// Slots follow the size the daemon gives the session, which is what the
    /// panes are laid out in, and stretch so the cards always fill the stage.
    /// The columns and rows of the layout that the stage shows.
    fn layout_cells(&self) -> (i32, i32) {
        let b = self.state.layout_box();
        let (cols, rows) = if b.w > 0 && b.h > 0 { (b.w, b.h) } else { (self.grid.0 as i32, self.grid.1 as i32) };
        // With no tree anywhere, the box is the whole session: chrome tuios keeps out
        // of it is still kept out, and panes are laid out without it.
        if self.state.trees.is_empty() {
            ((cols - self.unused_chrome.0 as i32).max(1), (rows - self.unused_chrome.1 as i32).max(1))
        } else {
            (cols, rows)
        }
    }

    fn slots(&self, window: &Window) -> Slots {
        let (w, h) = stage_size(window, ui(self.rail_drawn()));
        let b = self.state.layout_box();
        let (cols, rows) = self.layout_cells();
        let gap = self.layout_gap;
        Slots { w: w / (cols.max(1) + gap) as f32, h: h / (rows.max(1) + gap) as f32, x: b.x, y: b.y, gap }
    }

    /// The room the sidebar will take once any animation ends. The panes are sized
    /// for this at once, so that they reflow once and not on every frame.
    fn rail_space(&self) -> f32 {
        if self.rail_open { ui(self.rail_width) } else { 0.0 }
    }

    /// The rectangle a pane is drawn in this frame.
    fn drawn_rect(&self, win: &Win) -> [f32; 4] {
        let to = rect_of(win);
        let Some(m) = &self.layout_move else { return to };
        let Some(from) = m.from.get(&win.id) else { return to };
        let p = self.progress(m.started);
        std::array::from_fn(|i| from[i] + (to[i] - from[i]) * p)
    }

    /// Starts panes moving if this state puts any somewhere new. A change of the
    /// session's size moves everything by a little, so that one just happens.
    fn start_layout_move(&mut self, next: &State) {
        let old = &self.state;
        if self.animation().is_none() || old.workspace != next.workspace || old.session_size != next.session_size {
            self.layout_move = None;
            return;
        }
        let same_workspace = |w: &&Win| w.workspace == next.workspace && !w.minimized;
        let before = |w: &Win| old.windows.iter().filter(same_workspace).find(|o| o.id == w.id);
        let tiles: Vec<&Win> = next.windows.iter().filter(same_workspace).collect();
        if !tiles.iter().any(|w| before(w).is_some_and(|o| rect_of(o) != rect_of(w))) {
            return;
        }
        let from = tiles
            .iter()
            .filter_map(|w| Some((w.id.clone(), self.drawn_rect(before(w)?))))
            .collect();
        self.layout_move = Some(LayoutMove { from, started: Instant::now() });
    }

    /// One step of zoom, in or out.
    fn zoom(&mut self, step: f32, cx: &App) {
        let text = cx.text_system();
        let id = text.resolve_font(&font(self.look.font.clone()));
        let advance = text.advance(id, px(100.0), 'm').map_or(0.6, |s| f32::from(s.width) / 100.0);
        self.set_font_size(model::zoom_size(self.look.size, advance, self.device_scale, step));
    }

    /// The panes on this workspace, in the order their labels number them.
    fn labelled_panes(&self) -> Vec<&Win> {
        let mut panes: Vec<&Win> =
            self.state.windows.iter().filter(|w| w.workspace == self.state.workspace && !w.minimized && !w.popup).collect();
        panes.sort_by_key(|w| (w.y, w.x));
        panes.truncate(9);
        panes
    }

    /// Whether the prefix has been open long enough to show its keys.
    fn which_key_due(&self) -> bool {
        let wait = self.settings.keys.which_key_ms;
        wait > 0
            && self.keymap.in_prefix()
            && matches!(self.overlay, Overlay::None)
            && self.prefix_at.is_some_and(|at| at.elapsed() >= Duration::from_millis(wait))
    }

    /// Saves the window and what was learned once they have stopped changing.
    fn save_window_state(&mut self, window: &Window, cx: &mut Context<Self>) {
        let (bounds, maximized, fullscreen) = match window.window_bounds() {
            WindowBounds::Windowed(b) => (b, false, false),
            WindowBounds::Maximized(b) => (b, true, false),
            WindowBounds::Fullscreen(b) => (b, false, true),
        };
        let state = WindowState {
            display: window.display(cx).map_or(0, |d| u32::from(d.id())),
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            w: f32::from(bounds.size.width),
            h: f32::from(bounds.size.height),
            maximized,
            fullscreen,
            font_size: self.look.size,
            settings_font_size: self.settings.font.size,
            gap: self.layout_gap,
            chrome: self.unused_chrome,
            border: self.pane_border,
            diff_side_by_side: self.diff_side_by_side,
        };
        if self.state_saved.as_ref() == Some(&state) {
            self.state_save_due = None;
            return;
        }
        match self.state_save_due {
            Some(due) if Instant::now() >= due => {
                state.save();
                self.state_saved = Some(state);
                self.state_save_due = None;
            }
            Some(_) => {}
            None => {
                self.state_save_due = Some(Instant::now() + STATE_SAVE_WAIT);
                Self::wake_after(STATE_SAVE_WAIT + Duration::from_millis(10), cx);
            }
        }
    }

    fn wake_after(wait: Duration, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    /// Whether tuios has yet to answer a change of size. Panes still have their old
    /// grid until it does, which is no reason to scale their text.
    fn size_pending(&self) -> bool {
        self.size_sent.is_some_and(|((cols, rows), at)| {
            let size = self.state.session_size;
            (size.cols, size.rows) != (cols, rows) && at.elapsed() < SIZE_WAIT
        })
    }

    /// tuios can take cells from each pane for a border this client does not draw,
    /// depending on how the session was made. It shows in the panes' grids, and once
    /// they agree for a moment the insets give those cells back.
    fn measure_border(&mut self, cx: &mut Context<Self>) {
        let holding = self.fit_hold.is_some_and(|until| Instant::now() < until);
        if holding || self.grid_pending.is_some() || self.size_pending() || self.reserved_sent == (0, 0) || self.state.session_size != self.state.client_size {
            return;
        }
        let tiles = self.state.windows.iter().filter(|w| {
            w.workspace == self.state.workspace && !w.minimized && !w.popup && w.host.is_empty()
        });
        let grids = tiles.filter_map(|w| {
            let (cols, rows) = self.panes.get(&w.pty)?.size();
            (cols > 0).then_some(((w.w, w.h), (cols as i32, rows as i32)))
        });
        let Some(border) = model::agreed_border(grids, self.reserved_sent) else {
            self.border_seen = None;
            return;
        };
        if border == self.pane_border {
            self.border_seen = None;
            return;
        }
        match self.border_seen {
            Some((seen, at)) if seen == border => {
                if at.elapsed() >= BORDER_SETTLE {
                    self.pane_border = border;
                    self.border_seen = None;
                    self.scale = 0.0;
                }
            }
            _ => {
                self.border_seen = Some((border, Instant::now()));
                Self::wake_after(BORDER_SETTLE + Duration::from_millis(20), cx);
            }
        }
    }

    /// Tells the bridge when the window holds a different grid.
    fn sync_grid(&mut self, m: Metrics, window: &Window, cx: &mut Context<Self>) {
        let grid = grid_for(m, window, self.rail_space(), self.layout_gap);
        let scale = window.scale_factor();
        if grid == self.grid && scale == self.scale {
            self.grid_pending = None;
            return;
        }
        // Only a change of window size waits. A zoom or a new screen goes at once.
        if scale == self.scale {
            match self.grid_pending {
                Some((waiting, since)) if waiting == grid && since.elapsed() >= GRID_SETTLE => {}
                Some((waiting, _)) if waiting == grid => return,
                _ => {
                    self.grid_pending = Some((grid, Instant::now()));
                    Self::wake_after(GRID_SETTLE + Duration::from_millis(10), cx);
                    return;
                }
            }
        }
        self.grid_pending = None;
        self.grid = grid;
        self.scale = scale;
        // Ask for the box plus what tuios keeps out of it, so the panes fill the stage.
        let (extra_cols, extra_rows) = self.unused_chrome;
        let reserve = reserved(m, self.layout_gap, self.pane_border);
        self.reserved_sent = reserve;
        let g = bridge_grid((grid.0 + extra_cols, grid.1 + extra_rows), m, scale, reserve);
        self.size_sent = Some(((g.cols, g.rows), Instant::now()));
        Self::wake_after(SIZE_WAIT, cx);
        self.send(json!({
            "cmd": "resize",
            "cols": g.cols,
            "rows": g.rows,
            "cell_width": g.cell_width,
            "cell_height": g.cell_height,
            "insets": g.insets,
        }));
    }

    // ----- panes -----

    #[allow(clippy::too_many_arguments)]
    fn paint_pane(p: &PaneDraw, origin: Point<Pixels>, s: Slots, m: Metrics, t: &Theme, look: &Look, window: &mut Window, cx: &mut App) {
        let w = &p.win;
        let [x, y, cells_w, cells_h] = p.rect;
        let slot_x = f32::from(origin.x) + (x - s.x as f32) * s.w + p.dx;
        let slot_y = f32::from(origin.y) + (y - s.y as f32) * s.h;
        // Edges and text start on whole device pixels, to stay sharp at 1x.
        let scale = window.scale_factor();
        let snap = |v: f32| (v * scale).round() / scale;
        let (card_x, card_y) = (snap(slot_x + ui(MARGIN)), snap(slot_y + ui(MARGIN)));
        let card = Bounds::new(
            point(px(card_x), px(card_y)),
            size(
                px(snap(slot_x + (cells_w + s.gap as f32) * s.w - ui(MARGIN)) - card_x),
                px(snap(slot_y + (cells_h + s.gap as f32) * s.h - ui(MARGIN)) - card_y),
            ),
        );
        let waiting = w.agent == "needs_input";
        let (edge, weight) = match (waiting, p.focused) {
            (true, _) => (t.agent("needs_input").unwrap_or(t.accent), 1.5),
            (false, true) => (t.accent, 1.5),
            (false, false) => (t.rule, 1.0),
        };
        let edge_color: Hsla = rgb(edge).into();
        window.paint_quad(quad(card, px(CARD_RADIUS), rgb(t.palette.bg), px(weight), edge_color, BorderStyle::Solid));

        // The header: a status dot, the label, and on the right the branch.
        let mut title_x = f32::from(card.origin.x) + ui(PAD_X);
        let header_y = f32::from(card.origin.y);
        if let Some(c) = t.agent(&w.agent) {
            let dot = Bounds::new(point(px(title_x), px(header_y + ui(HEADER) / 2.0 - ui(3.5))), size(px(ui(7.0)), px(ui(7.0))));
            window.paint_quad(fill(dot, rgb(c)).corner_radii(px(ui(3.5))));
            title_x += ui(14.0);
        }
        let mut weight_font: Font = font(look.font.clone());
        weight_font.weight = if p.focused { FontWeight::SEMIBOLD } else { FontWeight::NORMAL };
        let label_color: Hsla = rgb(if p.focused { t.text } else { t.muted }).into();
        let label = window.text_system().shape_line(
            p.label.clone().into(),
            px(ui(12.0)),
            &[plain_run(p.label.len(), label_color, weight_font)],
            None,
        );
        let _ = label.paint(point(px(title_x), px(header_y + 1.0)), px(HEADER - 2.0), window, cx);
        let place = match (w.host.is_empty(), w.branch.is_empty()) {
            (true, _) => w.branch.clone(),
            (false, true) => w.host.clone(),
            (false, false) => format!("{}  ·  {}", w.host, w.branch),
        };
        if !place.is_empty() {
            let color: Hsla = rgb(t.faint).into();
            let branch = window.text_system().shape_line(
                place.clone().into(),
                px(ui(11.0)),
                &[plain_run(place.len(), color, font(look.font.clone()))],
                None,
            );
            let right = f32::from(card.origin.x + card.size.width) - ui(PAD_X) - f32::from(branch.width);
            if right > title_x + f32::from(label.width) + 12.0 {
                let _ = branch.paint(point(px(right), px(header_y + 1.0)), px(HEADER - 2.0), window, cx);
            }
        }

        // The text, scaled down if the program's grid is larger than the card.
        let fit = p.fit;
        let m = Metrics { cell_w: m.cell_w * fit, cell_h: m.cell_h * fit };
        let left = snap(f32::from(card.origin.x) + ui(PAD_X));
        let top = snap(header_y + ui(HEADER));
        let at = |col: usize, row: usize| {
            point(px(left) + m.cell_w * col as f32, px(top) + m.cell_h * row as f32)
        };
        if let Some(doc) = &p.doc {
            let inset = ui(PAD_X);
            let area = Bounds::new(
                point(px(left), px(top)),
                size(card.size.width - px(inset * 2.0), card.size.height - px(ui(HEADER) + ui(PAD_BOTTOM))),
            );
            let clip = Bounds::new(point(card.origin.x, px(top)), size(card.size.width, area.size.height));
            window.with_content_mask(Some(ContentMask { bounds: clip }), |window| Self::paint_doc(doc, area, t, look, window, cx));
        }
        window.with_content_mask(Some(ContentMask { bounds: card }), |window| {
        for (y, runs) in p.screen.rows.iter().enumerate().filter(|_| p.doc.is_none()) {
            for r in runs {
                let pos = at(r.col, y);
                let count = r.text.chars().count() as f32;
                if let Some(bg) = r.bg {
                    window.paint_quad(fill(Bounds::new(pos, size(m.cell_w * count, m.cell_h)), rgb(bg)));
                }
                let mut f: Font = font(look.font.clone());
                f.weight = if r.bold { FontWeight::BOLD } else { body_weight(scale) };
                if r.italic {
                    f.style = FontStyle::Italic;
                }
                let mut run = plain_run(r.text.len(), rgb(r.fg).into(), f);
                if r.underline {
                    run.underline = Some(UnderlineStyle { thickness: px(1.0), ..Default::default() });
                }
                let line = window.text_system().shape_line(r.text.clone().into(), px(look.size * fit), &[run], Some(m.cell_w));
                let _ = line.paint(pos, m.cell_h, window, cx);
            }
        }
        for (row, first, end) in &p.link {
            let pos = at(*first, *row);
            let bar = Bounds::new(point(pos.x, pos.y + m.cell_h - px(1.5)), size(m.cell_w * (end - first) as f32, px(1.0)));
            window.paint_quad(fill(bar, rgb(t.accent_text)));
        }
        });

        if let Some((thumb_top, thumb_height)) = p.scrollbar {
            let track = f32::from(card.size.height) - ui(HEADER) - ui(PAD_BOTTOM);
            let width = ui(4.0);
            let at = point(px(f32::from(card.origin.x + card.size.width) - width - ui(2.0)), px(top + thumb_top * track));
            let thumb = Bounds::new(at, size(px(width), px((thumb_height * track).max(ui(16.0)))));
            window.paint_quad(fill(thumb, gpui::rgba((t.muted << 8) | 0x66)).corner_radii(px(width / 2.0)));
        }

        if let Some(n) = p.badge {
            let side = ui(72.0);
            let at = point(card.center().x - px(side / 2.0), card.center().y - px(side / 2.0));
            window.paint_quad(fill(Bounds::new(at, size(px(side), px(side))), rgb(t.accent)).corner_radii(px(ui(14.0))));
            let text: SharedString = n.to_string().into();
            let run = plain_run(text.len(), rgb(t.on_accent).into(), font(look.font.clone()));
            let line = window.text_system().shape_line(text, px(ui(40.0)), &[run], None);
            let origin = point(card.center().x - line.width / 2.0, at.y);
            let _ = line.paint(origin, px(side), window, cx);
        }

        if let (true, Some((col, row)), None) = (p.focused, p.screen.cursor, &p.doc) {
            if p.screen.block_cursor {
                // Copy mode marks the whole cell, and the letter shows through.
                let cell = Bounds::new(at(col, row), size(m.cell_w, m.cell_h));
                window.paint_quad(fill(cell, gpui::rgba((t.cursor << 8) | 0x80)));
            } else {
                let bar = Bounds::new(at(col, row), size(px(2.0), m.cell_h));
                window.paint_quad(fill(bar, rgb(t.cursor)).corner_radii(px(1.0)));
            }
        }
    }
}

/// When the settings file last changed, or None with no file.
fn settings_file_time() -> Option<SystemTime> {
    std::fs::metadata(settings::path()?).ok()?.modified().ok()
}

const USAGE: &str = "tessella: a native macOS GUI for tuios

Usage: tessella [--session NAME] [--host NAME]

  --session NAME  Attach this session, created if missing. Default: the last active.
  --host NAME     Attach a session on this machine from your tuios [hosts].
  --help          Show this.
  --version       Show the version.

Settings: ~/.config/tessella/config.toml. Keys come from your tuios config.";

fn parse_args() -> Result<Target, String> {
    let mut target = Target::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--version" | "-V" => {
                println!("tessella {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--session" => target.session = Some(args.next().ok_or("--session needs a name")?),
            "--host" => target.host = Some(args.next().ok_or("--host needs a name")?),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(target)
}

/// How much a pane's text has to shrink to fit its card. A program's grid is its
/// own, and can be larger than the card: another client can set the session's size,
/// and a pane on another machine can lag behind. Text is never cut off for that.
fn text_fit(win: &Win, cols: usize, rows: usize, s: Slots, m: Metrics) -> f32 {
    if cols == 0 || rows == 0 {
        return 1.0;
    }
    let card_w = (win.w + s.gap) as f32 * s.w - ui(MARGIN) * 2.0;
    let card_h = (win.h + s.gap) as f32 * s.h - ui(MARGIN) * 2.0;
    let room_w = card_w - ui(PAD_X) * 2.0;
    let room_h = card_h - ui(HEADER) - ui(PAD_BOTTOM);
    // Rounding can take a pixel or two. The padding absorbs that, as scaling for it
    // would blur all of the text.
    let fit_w = (room_w + ui(4.0)) / (cols as f32 * f32::from(m.cell_w));
    let fit_h = (room_h + ui(3.0)) / (rows as f32 * f32::from(m.cell_h));
    fit_w.min(fit_h).clamp(0.25, 1.0)
}

fn metrics(look: &Look, window: &Window) -> Metrics {
    let text = window.text_system();
    let id = text.resolve_font(&font(look.font.clone()));
    let advance = text.advance(id, px(look.size), 'm').map_or(8.0, |s| f32::from(s.width));
    // A cell is a whole number of device pixels. Every letter then lands on a pixel
    // the same way, which is what keeps text sharp on a screen at 1x.
    let scale = window.scale_factor();
    let snap = |v: f32| (v * scale).round().max(1.0) / scale;
    Metrics { cell_w: px(snap(advance)), cell_h: px(snap(look.size * 1.4)) }
}

fn stage_size(window: &Window, rail: f32) -> (f32, f32) {
    let viewport = window.viewport_size();
    (f32::from(viewport.width) - rail - ui(STAGE_PAD) * 2.0, f32::from(viewport.height) - ui(TOPBAR) - ui(STAGE_PAD) * 2.0)
}

/// The grid the stage holds.
fn grid_for(m: Metrics, window: &Window, rail: f32, gap: i32) -> (u16, u16) {
    let (w, h) = stage_size(window, rail);
    model::stage_grid((w, h), (f32::from(m.cell_w), f32::from(m.cell_h)), gap)
}

/// The cells tuios should keep out of a pane's slot: enough that the text fits the
/// card, given the cells it takes for itself.
fn reserved(m: Metrics, gap: i32, border: (i32, i32)) -> (i32, i32) {
    (
        model::reserved_cells(ui(MARGIN + PAD_X) * 2.0, f32::from(m.cell_w), gap, border.0),
        model::reserved_cells(ui(MARGIN) * 2.0 + ui(HEADER) + ui(PAD_BOTTOM), f32::from(m.cell_h), gap, border.1),
    )
}

fn bridge_grid((cols, rows): (u16, u16), m: Metrics, scale: f32, reserve: (i32, i32)) -> Grid {
    let device = |v: f32| (v * scale).round() as i32;
    let (cell_width, cell_height) = (device(f32::from(m.cell_w)), device(f32::from(m.cell_h)));
    let (top, left) = (device(ui(MARGIN) + ui(HEADER)), device(ui(MARGIN) + ui(PAD_X)));
    Grid {
        cols,
        rows,
        cell_width,
        cell_height,
        // Top, left, right, bottom. tuios divides them by the cell, so they add up to
        // whole cells, and the far side takes what the near side leaves.
        insets: [top, left, reserve.0 * cell_width - left, reserve.1 * cell_height - top],
    }
}

/// The weight of ordinary text. A 1x screen has too few pixels for thin stems, and
/// macOS's font smoothing, which thickens them in other apps, does not apply here.
fn body_weight(scale: f32) -> FontWeight {
    if scale < 1.5 { FontWeight::MEDIUM } else { FontWeight::NORMAL }
}

fn plain_run(len: usize, color: Hsla, font: Font) -> TextRun {
    TextRun { len, font, color, background_color: None, underline: None, strikethrough: None }
}

impl Render for Tessella {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let m = metrics(&self.look, window);
        self.device_scale = window.scale_factor();
        self.viewport = (f32::from(window.viewport_size().width), f32::from(window.viewport_size().height));
        if std::mem::take(&mut self.fit_wake) {
            Self::wake_after(FIT_HOLD + Duration::from_millis(20), cx);
        }
        self.save_window_state(window, cx);
        self.refresh_docs(cx);
        self.refresh_remote_agents(cx);
        self.refresh_review(cx);
        self.measure_border(cx);
        self.sync_grid(m, window, cx);
        self.refresh_files();
        self.send_notices(window);
        let slots = self.slots(window);
        if self.slide.is_some_and(|s| self.progress(s.started) >= 1.0) {
            self.slide = None;
        }
        if self.rail_move.is_some_and(|m| self.progress(m.started) >= 1.0) {
            self.rail_move = None;
        }
        if self.layout_move.as_ref().is_some_and(|m| self.progress(m.started) >= 1.0) {
            self.layout_move = None;
        }
        if self.slide.is_some() || self.rail_move.is_some() || self.layout_move.is_some() {
            window.request_animation_frame();
        }
        let travel = stage_size(window, ui(self.rail_drawn())).0 + ui(STAGE_PAD) * 2.0;
        // While workspaces slide, the old one leaves as the new one arrives.
        let (slide_from, new_dx, old_dx) = match self.slide {
            Some(s) => {
                let p = self.progress(s.started);
                let dir = if self.state.workspace > s.from { 1.0 } else { -1.0 };
                (Some(s.from), dir * travel * (1.0 - p), -dir * travel * p)
            }
            None => (None, 0.0, 0.0),
        };

        let focused = self.state.focused.clone();
        let palette = self.theme.palette;
        let labels: Vec<String> = if matches!(self.overlay, Overlay::PaneLabels) {
            self.labelled_panes().iter().map(|w| w.id.clone()).collect()
        } else {
            Vec::new()
        };
        let mut draws: Vec<PaneDraw> = self
            .state
            .windows
            .iter()
            .filter(|w| (w.workspace == self.state.workspace || Some(w.workspace) == slide_from) && !w.minimized)
            .map(|w| PaneDraw {
                win: w.clone(),
                label: w.label(),
                focused: w.id == focused,
                screen: self.panes.get(&w.pty).map(|p| p.screen(&palette)).unwrap_or_default(),
                dx: if w.workspace == self.state.workspace { new_dx } else { old_dx },
                rect: self.drawn_rect(w),
                scrollbar: self.panes.get(&w.pty).and_then(Pane::scrollbar).filter(|_| w.markdown_place().is_none()),
                link: self.link_hover.as_ref().filter(|h| h.pty == w.pty).map_or(Vec::new(), |h| h.cells.clone()),
                doc: self.doc_draw(&w.id),
                badge: labels.iter().position(|id| *id == w.id).map(|i| i + 1),
                fit: self.fit_for(w, slots, m),
            })
            .collect();
        // A pane zooming in or out stays above the panes it covers and uncovers.
        let from = self.layout_move.as_ref().map(|m| &m.from);
        let zooming = |d: &PaneDraw| {
            let area = (d.win.w * d.win.h) as f32;
            from.and_then(|f| f.get(&d.win.id)).is_some_and(|r| {
                let before = r[2] * r[3];
                before > area * 1.2 || area > before * 1.2
            })
        };
        draws.sort_by_key(|d| (zooming(d), d.win.z));
        let theme = self.theme.clone();
        let look = self.look.clone();
        let drop_mark = self.drop_mark();

        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .font_family(self.look.font.clone())
            .font_weight(body_weight(window.scale_factor()))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_modifiers_changed(cx.listener(Self::on_modifiers))
            .on_mouse_move(cx.listener(Self::on_root_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_root_release))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_root_release))
            .child(self.topbar(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .p(px(STAGE_PAD))
                            .bg(rgb(self.theme.stage))
                            .cursor(self.stage_cursor)
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_stage_click))
                            .on_mouse_down(MouseButton::Right, cx.listener(|this, e: &MouseDownEvent, w, _| this.on_stage_other_press(2, e.position, true, w)))
                            .on_mouse_up(MouseButton::Right, cx.listener(|this, e: &MouseUpEvent, w, _| this.on_stage_other_press(2, e.position, false, w)))
                            .on_mouse_down(MouseButton::Middle, cx.listener(|this, e: &MouseDownEvent, w, _| this.on_stage_other_press(1, e.position, true, w)))
                            .on_mouse_up(MouseButton::Middle, cx.listener(|this, e: &MouseUpEvent, w, _| this.on_stage_other_press(1, e.position, false, w)))
                            .on_mouse_move(cx.listener(Self::on_stage_move))
                            .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| {
                                this.drop_paths(paths.paths().to_vec(), window);
                                cx.notify();
                            }))
                            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_stage_release))
                            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_stage_release))
                            .on_scroll_wheel(cx.listener(Self::on_stage_wheel))
                            .child(
                                canvas(
                                    |_, _, _| (),
                                    move |bounds, _, window, cx| {
                                        window.with_content_mask(Some(ContentMask { bounds }), |window| {
                                            for p in &draws {
                                                if p.win.popup {
                                                    window.paint_quad(fill(bounds, gpui::rgba(0x00000059)));
                                                }
                                                Self::paint_pane(p, bounds.origin, slots, m, &theme, &look, window, cx);
                                            }
                                            if let Some([x, y, w, h]) = drop_mark {
                                                let at = point(
                                                    bounds.origin.x + px((x - slots.x as f32) * slots.w),
                                                    bounds.origin.y + px((y - slots.y as f32) * slots.h),
                                                );
                                                let area = Bounds::new(at, size(px((w + slots.gap as f32) * slots.w), px((h + slots.gap as f32) * slots.h)));
                                                window.paint_quad(fill(area, gpui::rgba((theme.accent << 8) | 0x40)).corner_radii(px(ui(CARD_RADIUS))));
                                            }
                                        });
                                    },
                                )
                                .size_full(),
                            ),
                    )
                    .when(self.rail_open || self.rail_move.is_some(), |d| d.child(self.sidebar(cx))),
            )
            .children(self.ask_cards_view(cx))
            .children(self.which_key_view())
            .children(self.overlay_view(cx))
    }
}

fn main() {
    if tessella::notify::is_bundled() {
        tessella::notify::adopt_login_environment();
    }
    let target = match parse_args() {
        Ok(target) => target,
        Err(e) => {
            eprintln!("tessella: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    let (settings, status) = match Settings::load() {
        Ok(s) => (s, None),
        Err(e) => (Settings::default(), Some(format!("Settings not applied: {e}"))),
    };
    // For checks: open without taking the keyboard from whatever is in front.
    let background = std::env::var_os("TESSELLA_BACKGROUND").is_some();
    Application::new().run(move |cx: &mut App| {
        let font = app_font(&settings.font.family, cx);
        if background {
            eprintln!("tessella: font: {font}");
        }
        let saved = WindowState::load();
        let font_size = saved.as_ref().and_then(|s| s.zoom(settings.font.size)).unwrap_or(settings.font.size);
        set_ui_scale(font_size);
        let look = Look { font, size: font_size };
        // The window comes back where it was, if its screen is still there.
        let display = saved.as_ref().and_then(|s| cx.displays().into_iter().find(|d| u32::from(d.id()) == s.display));
        let window_bounds = match (&saved, &display) {
            (Some(s), Some(_)) => {
                let bounds = Bounds::new(point(px(s.x), px(s.y)), size(px(s.w), px(s.h)));
                if s.fullscreen {
                    WindowBounds::Fullscreen(bounds)
                } else if s.maximized {
                    WindowBounds::Maximized(bounds)
                } else {
                    WindowBounds::Windowed(bounds)
                }
            }
            _ => WindowBounds::Windowed(Bounds::centered(None, size(px(1180.0), px(760.0)), cx)),
        };
        let display_id = display.map(|d| d.id());
        let titlebar = gpui::TitlebarOptions {
            title: Some("Tessella".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(14.0), px(13.0))),
        };
        cx.open_window(
            WindowOptions {
                window_bounds: Some(window_bounds),
                display_id,
                titlebar: Some(titlebar),
                window_background: WindowBackgroundAppearance::Blurred,
                focus: !background,
                ..Default::default()
            },
            move |window, cx| cx.new(|cx| Tessella::new(target, settings, status, look, window, cx)),
        )
        .expect("the window opens");
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        if !background {
            cx.activate(true);
        }
    });
}
