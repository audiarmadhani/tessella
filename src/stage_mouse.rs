//! The mouse on the stage: what a press means depends on where it lands. A split line
//! drags, a header moves its pane, the scrollbar scrolls, and in the text a program
//! that asked for the mouse gets it, or else the press starts a selection.

use super::*;
use gpui::CursorStyle;
use tessella::model::{split_at, Rect, SplitHit};

/// A press being dragged.
#[derive(Debug, Clone)]
pub(super) enum StageDrag {
    Split { id: u64, along_x: bool, rect: Rect },
    /// A tiled pane by its header, and where it would land.
    Pane { id: String, from: (f32, f32), moved: bool, drop: Option<(String, DropSide)> },
    /// A floating pane by its header, with the cells from its corner to the pointer.
    Float { id: String, grab: (f32, f32), rect: Rect },
    Scrollbar { pty: String },
}

/// Where a dragged pane lands on another: beside one edge, or swapped with it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum DropSide {
    Left,
    Right,
    Top,
    Bottom,
    Swap,
}

impl DropSide {
    /// The part of a pane a point falls in, as fractions of its size.
    fn at(x: f32, y: f32) -> Self {
        if (0.3..0.7).contains(&x) && (0.3..0.7).contains(&y) {
            return Self::Swap;
        }
        let edges = [(x, Self::Left), (1.0 - x, Self::Right), (y, Self::Top), (1.0 - y, Self::Bottom)];
        edges.into_iter().min_by(|a, b| a.0.total_cmp(&b.0)).map_or(Self::Swap, |e| e.1)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Swap => "swap",
        }
    }

    /// The part of a rectangle `[x, y, w, h]` this side covers.
    pub(super) fn cover(self, [x, y, w, h]: [f32; 4]) -> [f32; 4] {
        match self {
            Self::Left => [x, y, w / 2.0, h],
            Self::Right => [x + w / 2.0, y, w / 2.0, h],
            Self::Top => [x, y, w, h / 2.0],
            Self::Bottom => [x, y + h / 2.0, w, h / 2.0],
            Self::Swap => [x, y, w, h],
        }
    }
}

/// How many points from a split line or the card's edge still count as on it.
const REACH: f32 = 5.0;
/// The width of the strip at a card's right edge that is the scrollbar.
const SCROLLBAR: f32 = 9.0;

impl Tessella {
    /// A window point in the cells of the layout.
    fn cell_point(&self, position: Point<Pixels>, window: &Window) -> (f32, f32) {
        let s = self.slots(window);
        let col = (f32::from(position.x) - ui(STAGE_PAD)) / s.w + s.x as f32;
        let row = (f32::from(position.y) - ui(TOPBAR) - ui(STAGE_PAD)) / s.h + s.y as f32;
        (col, row)
    }

    /// The split line under a point, unless a floating pane covers it.
    fn split_under(&self, position: Point<Pixels>, window: &Window) -> Option<SplitHit> {
        if self.window_under(position, window).is_some_and(|w| w.floating || w.popup || w.zoomed) {
            return None;
        }
        let s = self.slots(window);
        let (col, row) = self.cell_point(position, window);
        let root = self.state.trees.get(&self.state.workspace.to_string())?.root.as_ref()?;
        split_at(root, (col, row), (ui(REACH) / s.w, ui(REACH) / s.h), s.gap)
    }

    /// Whether a point is on the scrollbar of a pane that has one.
    fn on_scrollbar(&self, win: &Win, position: Point<Pixels>, window: &Window) -> bool {
        let s = self.slots(window);
        let right = ui(STAGE_PAD) + ((win.x - s.x) + win.w + s.gap) as f32 * s.w - ui(MARGIN);
        let x = f32::from(position.x);
        x >= right - ui(SCROLLBAR) && x <= right && self.panes.get(&win.pty).is_some_and(|p| p.scrollbar().is_some() && !p.wants_mouse())
    }

    /// Scrolls a pane to where the pointer is along its card.
    fn scroll_to_pointer(&mut self, win: &Win, position: Point<Pixels>, window: &Window) {
        let s = self.slots(window);
        let top = ui(TOPBAR) + ui(STAGE_PAD) + (win.y - s.y) as f32 * s.h + ui(MARGIN) + ui(HEADER);
        let height = (win.h + s.gap) as f32 * s.h - ui(MARGIN) * 2.0 - ui(HEADER);
        let fraction = (f32::from(position.y) - top) / height.max(1.0);
        if let Some(pane) = self.panes.get_mut(&win.pty) {
            pane.scroll_to_fraction(fraction);
        }
    }

    pub(super) fn on_stage_click(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_copy();
        for pane in self.panes.values_mut() {
            pane.select_clear();
        }
        self.rail_focused = false;
        self.focus.focus(window);
        if let Some(hit) = self.split_under(event.position, window) {
            self.stage_drag = Some(StageDrag::Split { id: hit.id, along_x: hit.along_x, rect: hit.rect });
            return cx.notify();
        }
        if let Some(win) = self.window_under(event.position, window).cloned() {
            self.send(json!({"cmd": "focus", "window": win.id}));
            let (col, row, side, past) = self.cell_at(&win, event.position, window);
            if past < 0 {
                self.press_header(&win, event, window);
                return cx.notify();
            }
            if win.markdown_place().is_some() {
                self.click_doc(&win.id, event.position, cx);
                return cx.notify();
            }
            if self.on_scrollbar(&win, event.position, window) {
                self.scroll_to_pointer(&win, event.position, window);
                self.stage_drag = Some(StageDrag::Scrollbar { pty: win.pty.clone() });
                return cx.notify();
            }
            // A program that asked for the mouse gets the click, as under tuios.
            // Shift keeps selecting text, as in other terminals.
            if !event.modifiers.shift && self.panes.get(&win.pty).is_some_and(Pane::wants_mouse) {
                if past == 0 {
                    if let Some(report) = self.panes.get(&win.pty).and_then(|p| p.mouse_report(0, true, col, row)) {
                        let _ = self.writer.input(&win.pty, &report);
                        self.mouse_down = Some((win.pty.clone(), (col, row)));
                    }
                }
                return cx.notify();
            }
            let kind = match event.click_count {
                1 => SelectionType::Simple,
                2 => SelectionType::Semantic,
                _ => SelectionType::Lines,
            };
            if let Some(pane) = self.panes.get_mut(&win.pty) {
                let point = pane.point_at(col, row);
                pane.select_start(kind, point, side);
                self.drag = Some(win.pty.clone());
            }
        }
        cx.notify();
    }

    /// A press on a pane's header: twice renames it, and a drag moves it.
    fn press_header(&mut self, win: &Win, event: &MouseDownEvent, window: &Window) {
        if event.click_count >= 2 {
            self.stage_drag = None;
            self.prompt(PromptKind::RenameWindow(win.id.clone()), "Rename pane", win.label());
            return;
        }
        let from = self.cell_point(event.position, window);
        self.stage_drag = Some(if win.floating || win.popup {
            let rect = Rect { x: win.x, y: win.y, w: win.w, h: win.h };
            StageDrag::Float { id: win.id.clone(), grab: (from.0 - win.x as f32, from.1 - win.y as f32), rect }
        } else {
            StageDrag::Pane { id: win.id.clone(), from, moved: false, drop: None }
        });
    }

    /// A press of another button goes to a program that asked for the mouse.
    pub(super) fn on_stage_other_press(&mut self, button: u8, position: Point<Pixels>, pressed: bool, window: &Window) {
        let Some(win) = self.window_under(position, window).cloned() else { return };
        let (col, row, _, past) = self.cell_at(&win, position, window);
        if past == 0 {
            if let Some(report) = self.panes.get(&win.pty).and_then(|p| p.mouse_report(button, pressed, col, row)) {
                let _ = self.writer.input(&win.pty, &report);
            }
        }
    }

    pub(super) fn on_stage_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.pressed_button != Some(MouseButton::Left) {
            self.hover_stage(event, window, cx);
            return;
        }
        if let Some(drag) = self.stage_drag.clone() {
            self.drag_stage(drag, event, window);
            return cx.notify();
        }
        if let Some((pty, last)) = self.mouse_down.clone() {
            let Some(win) = self.state.windows.iter().find(|w| w.pty == pty).cloned() else { return };
            let (col, row, ..) = self.cell_at(&win, event.position, window);
            if (col, row) != last {
                if let Some(report) = self.panes.get(&pty).and_then(|p| p.drag_report(col, row)) {
                    let _ = self.writer.input(&pty, &report);
                }
                self.mouse_down = Some((pty, (col, row)));
            }
            return;
        }
        let Some(pty) = self.drag.clone() else { return };
        let Some(win) = self.state.windows.iter().find(|w| w.pty == pty).cloned() else { return };
        let (col, row, side, past) = self.cell_at(&win, event.position, window);
        let Some(pane) = self.panes.get_mut(&pty) else { return };
        // Dragged past an edge, the history scrolls under the selection.
        if past != 0 {
            pane.scroll(if past < 0 { 1 } else { -1 });
        }
        let point = pane.point_at(col, row);
        pane.select_update(point, side);
        cx.notify();
    }

    /// With no button down: the pointer's shape, and motion to a program that follows it.
    fn hover_stage(&mut self, event: &MouseMoveEvent, window: &Window, cx: &mut Context<Self>) {
        let cursor = match self.split_under(event.position, window) {
            Some(hit) if hit.along_x => CursorStyle::ResizeLeftRight,
            Some(_) => CursorStyle::ResizeUpDown,
            None => CursorStyle::Arrow,
        };
        if cursor != self.stage_cursor {
            self.stage_cursor = cursor;
            cx.notify();
        }
        let Some(win) = self.window_under(event.position, window).cloned() else { return };
        let (col, row, _, past) = self.cell_at(&win, event.position, window);
        if past == 0 && self.mouse_seen != Some((col, row)) {
            self.mouse_seen = Some((col, row));
            if let Some(report) = self.panes.get(&win.pty).and_then(|p| p.motion_report(col, row)) {
                let _ = self.writer.input(&win.pty, &report);
            }
        }
    }

    fn drag_stage(&mut self, drag: StageDrag, event: &MouseMoveEvent, window: &Window) {
        let (col, row) = self.cell_point(event.position, window);
        match drag {
            StageDrag::Split { id, along_x, rect } => {
                let ratio = if along_x { (col - rect.x as f32) / rect.w.max(1) as f32 } else { (row - rect.y as f32) / rect.h.max(1) as f32 };
                let ratio = ratio.clamp(0.05, 0.95);
                self.send(json!({"cmd": "layout", "op": "set-ratio", "split": id, "ratio": ratio}));
            }
            StageDrag::Pane { id, from, moved, .. } => {
                let moved = moved || (col - from.0).abs() > 2.0 || (row - from.1).abs() > 1.0;
                let target = self.window_under(event.position, window).filter(|w| w.id != id && !w.popup);
                let drop = target.filter(|_| moved).map(|w| {
                    let side = DropSide::at((col - w.x as f32) / w.w.max(1) as f32, (row - w.y as f32) / w.h.max(1) as f32);
                    (w.id.clone(), side)
                });
                self.stage_drag = Some(StageDrag::Pane { id, from, moved, drop });
            }
            StageDrag::Float { id, grab, rect } => {
                let (x, y) = ((col - grab.0).round() as i32, (row - grab.1).round() as i32);
                let moved = Rect { x, y, ..rect };
                self.send(json!({"cmd": "layout", "op": "float-geometry", "id": id, "rect": {"x": x, "y": y, "w": rect.w, "h": rect.h}}));
                self.stage_drag = Some(StageDrag::Float { id, grab, rect: moved });
            }
            StageDrag::Scrollbar { pty } => {
                if let Some(win) = self.state.windows.iter().find(|w| w.pty == pty).cloned() {
                    self.scroll_to_pointer(&win, event.position, window);
                }
            }
        }
    }

    pub(super) fn on_stage_release(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(StageDrag::Pane { id, drop: Some((target, side)), .. }) = self.stage_drag.take() {
            if side == DropSide::Swap {
                self.send(json!({"cmd": "layout", "op": "swap", "a": id, "b": target}));
            } else {
                self.send(json!({"cmd": "layout", "op": "move", "id": id, "target": target, "side": side.name()}));
            }
            return cx.notify();
        }
        if let Some((pty, _)) = self.mouse_down.take() {
            if let Some(win) = self.state.windows.iter().find(|w| w.pty == pty).cloned() {
                let (col, row, ..) = self.cell_at(&win, event.position, window);
                if let Some(report) = self.panes.get(&pty).and_then(|p| p.mouse_report(0, false, col, row)) {
                    let _ = self.writer.input(&pty, &report);
                }
            }
            return;
        }
        if let Some(pane) = self.drag.take().and_then(|pty| self.panes.get_mut(&pty)) {
            pane.select_finish();
        }
        cx.notify();
    }

    /// Where a dragged pane would land, in cells, to mark on the stage.
    pub(super) fn drop_mark(&self) -> Option<[f32; 4]> {
        let Some(StageDrag::Pane { drop: Some((target, side)), .. }) = &self.stage_drag else { return None };
        let win = self.state.windows.iter().find(|w| w.id == *target)?;
        Some(side.cover(rect_of(win)))
    }
}

#[cfg(test)]
mod tests {
    use super::DropSide;

    #[test]
    fn a_pane_lands_by_the_nearest_edge_or_swaps_in_the_middle() {
        assert_eq!(DropSide::at(0.5, 0.5), DropSide::Swap);
        assert_eq!(DropSide::at(0.1, 0.5), DropSide::Left);
        assert_eq!(DropSide::at(0.9, 0.5), DropSide::Right);
        assert_eq!(DropSide::at(0.5, 0.1), DropSide::Top);
        assert_eq!(DropSide::at(0.5, 0.9), DropSide::Bottom);
    }

    #[test]
    fn the_mark_covers_the_half_it_lands_in() {
        assert_eq!(DropSide::Right.cover([10.0, 0.0, 20.0, 8.0]), [20.0, 0.0, 10.0, 8.0]);
        assert_eq!(DropSide::Swap.cover([10.0, 0.0, 20.0, 8.0]), [10.0, 0.0, 20.0, 8.0]);
    }
}
