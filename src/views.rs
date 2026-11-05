//! Everything drawn outside the panes: the top dock, the sidebar, the overlays.

use super::*;
use gpui::{ease_out_quint, relative, rgba, Animation, AnimationExt};
use tessella::picker::VISIBLE;
use tessella::rail::Section as RailSection;

/// Points, at the current zoom. Everything in this file is drawn through this.
fn px(v: f32) -> gpui::Pixels {
    gpui::px(super::ui(v))
}

/// tuios's marker for an agent's state.
fn marker(state: &str) -> &'static str {
    match state {
        "needs_input" | "errored" => "▲",
        "working" | "done" => "●",
        "idle" => "○",
        _ => "",
    }
}

fn is_row_of(section: RailSection, row: &Row) -> bool {
    matches!(
        (section, row),
        (RailSection::Sessions, Row::Session { .. })
            | (RailSection::Terminals, Row::Terminal { .. })
            | (RailSection::Files, Row::File(_))
            | (RailSection::Agents, Row::Agent { .. })
    )
}

impl Tessella {
    // ----- the top dock -----

    pub(super) fn topbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let hover = rgb(t.hover);
        let current = self.state.workspace;
        let occupied = self.state.occupied.clone().unwrap_or_default();
        let mut shown: Vec<i32> = occupied;
        if !shown.contains(&current) && current > 0 {
            shown.push(current);
            shown.sort_unstable();
        }
        let chips: Vec<_> = shown
            .into_iter()
            .map(|n| {
                let on = n == current;
                let name = self.state.workspace_names.get(&n).filter(|s| !s.is_empty());
                let label = name.map_or(n.to_string(), |s| format!("{n} {s}"));
                div()
                    .h(px(24.0))
                    .min_w(px(24.0))
                    .px(px(7.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .when(on, |d| d.bg(rgb(t.accent)).text_color(rgb(t.on_accent)).font_weight(FontWeight::SEMIBOLD))
                    .when(!on, |d| d.text_color(rgb(t.text)).hover(move |s| s.bg(hover)))
                    .child(label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.send(json!({"cmd": "workspace", "n": n}));
                            cx.notify();
                        }),
                    )
            })
            .collect();
        let panes_here = self.state.windows.iter().filter(|w| w.workspace == current && !w.popup).count();
        let waiting = self.fleet.waiting();
        let more = waiting.len().saturating_sub(1);
        let alert = t.agent("needs_input").unwrap_or(t.accent);
        let inbox_waiting = self.fleet.inbox.iter().filter(|i| i.waits()).count();
        // The prefix hint and any message take the ticker's place while they apply.
        let middle = if let Some((_, mode)) = &self.copy {
            Some((mode.label(), t.accent_text, None))
        } else if self.keymap.in_prefix() {
            Some((format!("{} …", self.keymap.leader()), t.accent_text, None))
        } else if let Some(s) = &self.status {
            Some((s.clone(), t.text, None))
        } else {
            waiting.first().map(|a| {
                let text = if a.summary.is_empty() { a.name.clone() } else { format!("{}: {}", a.name, a.summary) };
                (text, alert, Some((a.session.clone(), a.window.clone())))
            })
        };
        div()
            .h(px(TOPBAR))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.0))
            // The traffic lights do not zoom, so their room does not either.
            .pl(gpui::px(84.0))
            .pr(px(10.0))
            .bg(rgb(t.sidebar))
            .border_b_1()
            .border_color(rgb(t.rule))
            // The bar stands in for the title bar, so the window drags by it.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .children(chips)
            .child(
                div()
                    .size(px(24.0))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .text_size(px(14.0))
                    .text_color(rgb(t.muted))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child("+")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.run_action("tessella_new_workspace".into(), cx);
                            cx.notify();
                        }),
                    ),
            )
            .child(div().flex_none().pl(px(4.0)).text_size(px(11.5)).text_color(rgb(t.muted)).child(format!("{current}:{panes_here}")))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(8.0))
                    .px(px(12.0))
                    .when_some(middle, |d, (text, color, target)| {
                        let is_alert = target.is_some();
                        d.child(
                            div()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .text_size(px(12.0))
                                .text_color(rgb(color))
                                .when(is_alert, |d| d.flex_none().child("▲"))
                                .child(div().min_w_0().truncate().when(is_alert, |d| d.text_color(rgb(t.text))).child(text))
                                .when_some(target, |d, (session, window)| {
                                    d.cursor_pointer().on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.focus_anywhere(&session, &window);
                                            cx.notify();
                                        }),
                                    )
                                }),
                        )
                        .when(more > 0 && is_alert, |d| {
                            d.child(div().flex_none().text_size(px(11.5)).text_color(rgb(t.muted)).child(format!("more +{more}")))
                        })
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(8.0))
                    .h(px(24.0))
                    .flex()
                    .items_center()
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .text_color(rgb(if inbox_waiting > 0 { alert } else { t.text }))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(if inbox_waiting > 0 { SharedString::from(format!("Inbox {inbox_waiting}")) } else { SharedString::from("Inbox") })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.open_agents(super::agents_view::AgentsTab::Inbox);
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(8.0))
                    .h(px(24.0))
                    .flex()
                    .items_center()
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .text_color(rgb(t.text))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child("+ Pane")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.new_pane();
                            this.focus.focus(window);
                            cx.notify();
                        }),
                    ),
            )
            .child({
                // A Nerd Font has a proper gear. Anything else gets the symbol.
                let nerd = self.look.font.contains("Nerd") || self.look.font.ends_with(" NF");
                div()
                    .flex_none()
                    .size(px(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .text_size(px(14.0))
                    .text_color(rgb(t.muted))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(if nerd { "\u{f013}" } else { "⚙" })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.overlay = Overlay::Settings(SettingsPage::default());
                            cx.notify();
                        }),
                    )
            })
            .child(
                div()
                    .flex_none()
                    .size(px(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .text_size(px(14.0))
                    .text_color(rgb(t.muted))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .child(if self.rail_open { "»" } else { "«" })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_rail();
                            cx.notify();
                        }),
                    ),
            )
    }

    // ----- the sidebar -----

    fn section_header(&self, section: RailSection, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let collapsed = self.rail_collapsed.contains(&section);
        let hover = rgb(t.hover);
        let small_button = |label: &'static str| {
            div()
                .flex_none()
                .px(px(5.0))
                .rounded(px(4.0))
                .text_size(px(11.5))
                .text_color(rgb(t.muted))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(label)
        };
        let controls = match section {
            RailSection::Sessions => div().flex().items_center().child(small_button("+").on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.prompt(PromptKind::NewSession, "New session name", String::new());
                    cx.notify();
                }),
            )),
            RailSection::Terminals => div().flex().items_center().child(small_button("+").on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.new_pane();
                    cx.notify();
                }),
            )),
            RailSection::Files => {
                let path = self.files_shown.as_ref().map(|shown| {
                    let path = rail::short_path(&shown.path);
                    if shown.is_here() { path } else { format!("{}:{path}", shown.host) }
                });
                let path = path.unwrap_or_default();
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(div().min_w_0().text_size(px(11.0)).text_color(rgb(t.muted)).truncate().child(path))
                    .child(small_button("⌕").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.file_action("file_search", None, cx);
                            cx.notify();
                        }),
                    ))
                    .child(small_button("+").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.file_action("file_create", None, cx);
                            cx.notify();
                        }),
                    ))
                    .child(small_button("cd").on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.cd_here();
                            cx.notify();
                        }),
                    ))
            }
            RailSection::Agents => {
                let filter = self.rail_filter;
                let option = |label: &'static str, on: bool, cx: &mut Context<Self>, set: Filter| {
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(if on { t.text } else { t.muted }))
                        .when(on, |d| d.font_weight(FontWeight::SEMIBOLD))
                        .cursor_pointer()
                        .child(label)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.rail_filter = set;
                                cx.notify();
                            }),
                        )
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .child(option("all", filter == Filter::All, cx, Filter::All))
                    .child(option("you", filter == Filter::You, cx, Filter::You))
                    .child(option("@", filter == Filter::Mail, cx, Filter::Mail))
            }
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .h(px(22.0))
            .px(px(12.0))
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(t.muted))
                    .cursor_pointer()
                    .child(section.title())
                    .when(collapsed, |d| d.child(" ›"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            if this.rail_collapsed.contains(&section) {
                                this.rail_collapsed.retain(|s| *s != section);
                            } else {
                                this.rail_collapsed.push(section);
                            }
                            cx.notify();
                        }),
                    ),
            )
            .child(controls)
    }

    fn rail_row(&self, row: &Row, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let cursor = self.rail_focused && index == self.rail_cursor;
        let hover = rgb(t.hover);
        let (state, active) = match row {
            Row::Session { state, current, .. } => (state.as_str(), *current),
            Row::Terminal { focused, .. } => ("", *focused),
            Row::Agent { state, focused, .. } => (state.as_str(), *focused),
            Row::File(_) => ("", false),
        };
        let color = t.agent(state).unwrap_or(t.muted);
        let mark = marker(state);
        // tuios's bar at the row's edge: this one is current, or it wants you.
        let bar = match row {
            _ if state == "needs_input" => Some(color),
            Row::Agent { .. } => None,
            _ if active => Some(t.accent),
            _ => None,
        };
        let clicked = row.clone();
        let file_entry = if let Row::File(e) = row { Some(e.clone()) } else { None };
        let line = match row {
            Row::Session { name, current, panes, branch, .. } => div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(t.text))
                        .when(*current, |d| d.font_weight(FontWeight::SEMIBOLD))
                        .child(SharedString::from(name.clone())),
                )
                .when(!branch.is_empty(), |d| {
                    d.child(div().flex_none().text_size(px(11.5)).text_color(rgb(t.muted)).child(SharedString::from(branch.clone())))
                })
                .child(div().flex_1())
                .child(div().flex_none().text_size(px(11.5)).text_color(rgb(t.muted)).child(panes.to_string())),
            Row::Terminal { label, host, .. } => div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(px(6.0))
                .child(div().min_w_0().truncate().text_color(rgb(t.text)).child(SharedString::from(label.clone())))
                .when(!host.is_empty(), |d| {
                    d.child(
                        div()
                            .flex_none()
                            .px(px(5.0))
                            .rounded(px(4.0))
                            .bg(rgba((t.text << 8) | 0x24))
                            .text_size(px(10.0))
                            .text_color(rgb(t.text))
                            .child(SharedString::from(host.clone())),
                    )
                }),
            Row::File(e) => {
                let color = if e.dir { t.accent_text } else { t.text };
                let name = if e.dir && e.name != ".." { format!("{}/", e.name) } else { e.name.clone() };
                div().flex_1().min_w_0().truncate().text_color(rgb(color)).child(name)
            }
            Row::Agent { name, note, .. } => div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_color(rgb(t.text))
                        .when(active || state == "needs_input", |d| d.font_weight(FontWeight::SEMIBOLD))
                        .child(SharedString::from(name.clone())),
                )
                .when(!note.is_empty(), |d| {
                    d.child(div().truncate().text_size(px(11.5)).text_color(rgb(t.muted)).child(SharedString::from(note.clone())))
                }),
        };
        div()
            .flex()
            .flex_none()
            .items_start()
            .gap(px(6.0))
            .pr(px(10.0))
            .py(px(1.0))
            .text_size(px(12.5))
            .cursor_pointer()
            .when(cursor, |d| d.bg(rgb(t.selected)))
            .when(!cursor, |d| d.hover(move |s| s.bg(hover)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.rail_cursor = index;
                    this.activate_row(clicked.clone(), cx);
                    this.focus.focus(window);
                }),
            )
            .when_some(file_entry, |d, entry| {
                d.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        this.rail_cursor = index;
                        this.open_file_menu(Some(entry.clone()), event.position);
                        this.focus.focus(window);
                        cx.notify();
                    }),
                )
            })
            .child(div().w(px(2.0)).h(px(15.0)).flex_none().when_some(bar, |d, c| d.bg(rgb(c))))
            .child(div().w(px(12.0)).flex_none().text_size(px(11.0)).text_color(rgb(color)).child(mark))
            .child(line)
    }

    pub(super) fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let groups = self.rail_groups();
        let mut upper: Vec<gpui::AnyElement> = Vec::new();
        let mut agents: Vec<gpui::AnyElement> = Vec::new();
        let mut index = 0;
        for g in &groups {
            let section = g.section;
            if section != RailSection::Agents {
                upper.push(self.section_header(section, cx).into_any_element());
            }
            for row in &g.rows {
                debug_assert!(is_row_of(section, row));
                let element = self.rail_row(row, index, cx).into_any_element();
                if section == RailSection::Agents {
                    agents.push(element);
                } else {
                    upper.push(element);
                }
                index += 1;
            }
        }
        let empty_agents = agents.is_empty();
        let line_color = t.rule;
        let agents_open = !self.rail_collapsed.contains(&RailSection::Agents);
        let panel = div()
            .w(px(self.rail_width))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .line_height(relative(1.3))
            .bg(rgb(t.sidebar))
            .border_l_1()
            .border_color(if self.rail_focused { rgb(t.accent) } else { rgb(t.rule) })
            .child(
                div()
                    .id("rail-upper")
                    .flex_1()
                    .min_h_0()
                    .pt(px(6.0))
                    .overflow_y_scroll()
                    .track_scroll(&self.rail_scroll)
                    .flex()
                    .flex_col()
                    .children(upper),
            )
            // The line above the agents section is a handle: drag it to resize them.
            .child(
                div()
                    .h(px(7.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .cursor_row_resize()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.agents_drag = true;
                            cx.notify();
                        }),
                    )
                    .child(div().w_full().h(px(1.0)).bg(rgb(line_color))),
            )
            .child(self.section_header(RailSection::Agents, cx))
            .child(
                div()
                    .id("rail-agents")
                    .flex_none()
                    .when(agents_open, |d| d.h(px(self.agents_height)))
                    .pb(px(5.0))
                    .overflow_y_scroll()
                    .track_scroll(&self.agents_scroll)
                    .flex()
                    .flex_col()
                    .children(agents)
                    .when(empty_agents, |d| {
                        d.child(div().px(px(14.0)).py(px(2.0)).text_size(px(11.5)).text_color(rgb(t.muted)).child("No agents"))
                    }),
            );
        // The panel keeps its width, and this reveals it as the sidebar slides.
        div().w(px(self.rail_drawn())).h_full().flex_none().overflow_hidden().child(panel)
    }

    // ----- overlays -----

    /// A menu at the pointer, kept inside the window.
    fn menu_view(&self, menu: &super::file_actions::ContextMenu, cx: &mut Context<Self>) -> gpui::AnyElement {
        const WIDTH: f32 = 236.0;
        const ROW: f32 = 25.0;
        let t = &self.theme;
        let line = theme::mix(t.palette.bg, t.text, 0.16);
        let hover = t.accent;
        let danger = t.agent("errored").unwrap_or(t.text);
        let groups = menu.rows.iter().filter(|r| r.group).count() as f32;
        let height = menu.rows.len() as f32 * ROW + groups * 9.0 + 10.0;
        let (vw, vh) = self.viewport;
        let x = f32::from(menu.at.x).min(vw - WIDTH - 8.0).max(8.0);
        let y = f32::from(menu.at.y).min(vh - height - 8.0).max(8.0);
        let rows = menu.rows.iter().enumerate().map(|(i, row)| {
            let on = menu.selected == Some(i);
            let action = row.action;
            let color = if row.danger { danger } else { t.text };
            div()
                .flex_none()
                .when(row.group, |d| d.mt(px(4.0)).pt(px(4.0)).border_t_1().border_color(rgb(line)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .h(px(ROW))
                        .mx(px(5.0))
                        .px(px(9.0))
                        .rounded(px(5.0))
                        .text_size(px(12.5))
                        .cursor_pointer()
                        .when(on, |d| d.bg(rgb(hover)).text_color(rgb(t.on_accent)))
                        .when(!on, |d| d.text_color(rgb(color)).hover(move |s| s.bg(rgb(hover)).text_color(rgb(t.on_accent))))
                        .child(row.label)
                        .child(div().text_size(px(11.0)).opacity(0.65).child(SharedString::from(row.key.clone())))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.menu_choose(action, cx);
                                cx.notify();
                            }),
                        ),
                )
        });
        div()
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.overlay = Overlay::None;
                    cx.notify();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, _, cx| {
                    this.overlay = Overlay::None;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(WIDTH))
                    .py(px(5.0))
                    .flex()
                    .flex_col()
                    .rounded(px(9.0))
                    .bg(rgb(t.palette.bg))
                    .border_1()
                    .border_color(rgb(line))
                    .shadow_lg()
                    .children(rows),
            )
            .into_any_element()
    }

    pub(super) fn overlay_view(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if let Overlay::Menu(menu) = &self.overlay {
            return Some(self.menu_view(menu, cx));
        }
        let (sheet, width) = match &self.overlay {
            Overlay::None | Overlay::PaneLabels | Overlay::Menu(_) => return None,
            Overlay::Agents(page) => (self.agents_view(page, cx).into_any_element(), 1040.0),
            Overlay::Chooser(ChooserKind::Pane, picker) => (self.pane_chooser_view(picker, cx).into_any_element(), 920.0),
            Overlay::Chooser(kind, picker) => (self.chooser_view(*kind, picker, cx).into_any_element(), 460.0),
            Overlay::Machines(picker) => (self.machine_view(picker, cx).into_any_element(), 380.0),
            Overlay::Prompt(prompt) => (self.prompt_view(prompt, cx).into_any_element(), 400.0),
            Overlay::Settings(page) => (self.settings_view(page, cx).into_any_element(), 780.0),
            Overlay::Help => (self.help_view().into_any_element(), 960.0),
        };
        let t = &self.theme;
        let line = theme::mix(t.palette.bg, t.text, 0.14);
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(match self.overlay {
                    Overlay::Help => 52.0,
                    Overlay::Settings(_) | Overlay::Agents(_) => 54.0,
                    _ => 90.0,
                }))
                .bg(rgba(0x00000040))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.cancel_overlay();
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .w(px(width))
                        .flex()
                        .flex_col()
                        .rounded(px(14.0))
                        .bg(rgb(t.palette.bg))
                        .border_1()
                        .border_color(rgb(line))
                        .shadow_lg()
                        .occlude()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(sheet),
                )
                .with_animation(
                    "overlay",
                    Animation::new(self.animation().unwrap_or(Duration::from_millis(1))).with_easing(ease_out_quint()),
                    |overlay, t| overlay.opacity(t),
                )
                .into_any_element(),
        )
    }

    /// The pane picker: the list, and beside it what the chosen pane shows.
    fn pane_chooser_view(&self, p: &Picker, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let line = theme::mix(t.palette.bg, t.text, 0.14);
        let current = p.current().map(|i| i.value);
        let preview = self.pane_preview.as_ref().filter(|(v, _)| Some(v) == current.as_ref()).map(|(_, l)| l.as_slice()).unwrap_or(&[]);
        // The end of a pane is what it is doing now.
        let tail = preview.iter().skip(preview.len().saturating_sub(24)).cloned().collect::<Vec<_>>();
        div()
            .flex()
            .child(div().w(px(460.0)).flex_none().border_r_1().border_color(rgb(line)).child(self.chooser_view(ChooserKind::Pane, p, cx)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(VISIBLE as f32 * 31.0 + 102.0))
                    .overflow_hidden()
                    .p(px(14.0))
                    .text_size(px(11.0))
                    .text_color(rgb(t.muted))
                    .children(tail.into_iter().map(|l| div().h(px(15.0)).whitespace_nowrap().child(SharedString::from(l)))),
            )
    }

    /// The keys of the open prefix, shown when it waits for one.
    pub(super) fn which_key_view(&self) -> Option<gpui::AnyElement> {
        if !self.which_key_due() {
            return None;
        }
        let rows = self.keymap.open_prefix_rows();
        if rows.is_empty() {
            return None;
        }
        let t = &self.theme;
        let line = theme::mix(t.palette.bg, t.text, 0.14);
        Some(
            div()
                .absolute()
                .bottom(px(14.0))
                .left(px(14.0))
                .right(px(14.0))
                .flex()
                .justify_center()
                .child(
                    div()
                        .max_w(px(1150.0))
                        .overflow_hidden()
                        .flex()
                        .flex_wrap()
                        .gap_x(px(18.0))
                        .px(px(16.0))
                        .py(px(10.0))
                        .rounded(px(12.0))
                        .bg(rgb(t.palette.bg))
                        .border_1()
                        .border_color(rgb(line))
                        .shadow_lg()
                        .children(rows.into_iter().map(|(key, text)| {
                            div()
                                .w(px(215.0))
                                .h(px(18.0))
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .text_size(px(11.0))
                                .child(div().flex_none().min_w(px(30.0)).text_color(rgb(t.accent)).font_weight(FontWeight::SEMIBOLD).child(SharedString::from(key)))
                                .child(div().min_w_0().truncate().text_color(rgb(t.text)).child(SharedString::from(text)))
                        })),
                )
                .into_any_element(),
        )
    }

    fn chooser_view(&self, kind: ChooserKind, p: &Picker, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let items = p.filtered();
        let first = p.first_visible(items.len());
        let surface = t.palette.bg;
        let line = theme::mix(surface, t.text, 0.14);
        let rows: Vec<_> = items
            .iter()
            .enumerate()
            .skip(first)
            .take(VISIBLE)
            .map(|(i, item)| {
                let on = i == p.selected;
                let current = !p.original.is_empty() && item.value == p.original;
                let chosen = (*item).clone();
                let hover = theme::mix(surface, t.text, 0.08);
                let sub = if on { t.on_accent } else { t.muted };
                let is_theme = kind == ChooserKind::Theme;
                let favourite = item.favourite;
                // The last favourite closes their group with a line.
                let last_favourite = favourite && items.get(i + 1).is_none_or(|next| !next.favourite);
                let star_for = item.value.clone();
                let star = is_theme.then(|| {
                    let color = if on { t.on_accent } else if favourite { t.agent("needs_input").unwrap_or(t.accent) } else { t.faint };
                    div()
                        .w(px(18.0))
                        .h_full()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .text_size(px(13.0))
                        .text_color(rgb(color))
                        .child(if favourite { "★" } else { "☆" })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_favourite(&star_for, cx);
                                cx.notify();
                            }),
                        )
                });
                let row = div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.0))
                    .mx(px(8.0))
                    .px(px(10.0))
                    .h(px(30.0))
                    .rounded(px(7.0))
                    .cursor_pointer()
                    .text_size(px(13.0))
                    .when(on, |d| d.bg(rgb(t.accent)).text_color(rgb(t.on_accent)))
                    .when(!on, |d| d.text_color(rgb(t.text)).hover(move |s| s.bg(rgb(hover))))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.accept_chooser(kind, chosen.clone(), cx);
                            cx.notify();
                        }),
                    )
                    .children(star)
                    .child(div().flex_1().min_w_0().truncate().child(SharedString::from(item.label.clone())))
                    .when(!item.detail.is_empty(), |d| {
                        d.child(div().flex_none().max_w(px(170.0)).truncate().text_size(px(11.5)).text_color(rgb(sub)).child(SharedString::from(item.detail.clone())))
                    })
                    .when(current, |d| d.child(div().flex_none().text_size(px(12.0)).child("✓")));
                div()
                    .flex_none()
                    .when(last_favourite && p.query.is_empty(), |d| d.pb(px(3.0)).mb(px(3.0)).border_b_1().border_color(rgb(line)))
                    .child(row)
            })
            .collect();
        let empty = rows.is_empty();
        let placeholder = match kind {
            ChooserKind::Theme => "Search themes",
            ChooserKind::Font => "Search fonts",
            ChooserKind::Palette => "Run a command",
            ChooserKind::Session => "Switch session",
            ChooserKind::Workspace => "Switch workspace",
            ChooserKind::Pane => "Go to a pane",
            ChooserKind::Launcher => "Run a program",
            ChooserKind::Layout => "Load a layout",
            ChooserKind::FileSearch => "Search files below this folder",
            ChooserKind::SendTo => "Send to which machine?",
        };
        let swatches: Vec<_> = if kind == ChooserKind::Theme {
            t.palette.ansi[..8]
                .iter()
                .map(|c| div().size(px(13.0)).rounded_full().bg(rgb(*c)).border_1().border_color(rgb(t.rule)))
                .collect()
        } else {
            Vec::new()
        };
        let hint = match kind {
            ChooserKind::Theme => "↑↓ preview  ·  tab star  ·  ⏎ apply  ·  esc",
            ChooserKind::Layout => "↑↓ choose  ·  tab delete  ·  ⏎ load  ·  esc",
            _ => "↑↓ choose  ·  ⏎ open  ·  esc",
        };
        let query = p.query.clone();
        let count = items.len();
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(1.0))
                    .h(px(46.0))
                    .px(px(18.0))
                    .text_size(px(15.0))
                    .border_b_1()
                    .border_color(rgb(line))
                    .when(query.is_empty(), |d| {
                        d.child(div().w(px(2.0)).h(px(18.0)).bg(rgb(t.accent)))
                            .child(div().pl(px(4.0)).text_color(rgb(t.faint)).child(placeholder))
                    })
                    .when(!query.is_empty(), |d| {
                        d.child(div().text_color(rgb(t.text)).child(SharedString::from(query.clone())))
                            .child(div().w(px(2.0)).h(px(18.0)).bg(rgb(t.accent)))
                    }),
            )
            .child(
                div()
                    .h(px((VISIBLE as f32) * 31.0 + 14.0))
                    .pt(px(7.0))
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .children(rows)
                    .when(empty, |d| {
                        d.child(div().px(px(18.0)).pt(px(14.0)).text_size(px(13.0)).text_color(rgb(t.muted)).child("Nothing matches"))
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(42.0))
                    .px(px(18.0))
                    .border_t_1()
                    .border_color(rgb(line))
                    .child(div().flex().gap(px(4.0)).children(swatches))
                    .child(div().text_size(px(11.0)).text_color(rgb(t.faint)).child(if kind == ChooserKind::Theme { hint.to_string() } else { format!("{count} shown  ·  {hint}") })),
            )
    }

    fn machine_view(&self, p: &MachinePicker, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let surface = t.palette.bg;
        let line = theme::mix(surface, t.text, 0.14);
        let rows: Vec<_> = self
            .machines
            .iter()
            .enumerate()
            .map(|(i, machine)| {
                let on = i == p.selected && machine.ready;
                let host = machine.host.clone();
                let ready = machine.ready;
                let hover = theme::mix(surface, t.text, 0.08);
                let dot = match (machine.ready, machine.health.as_str()) {
                    (false, _) => t.agent("errored"),
                    (true, "slow") | (true, "stalled") => t.agent("needs_input"),
                    (true, _) => t.agent("done"),
                }
                .unwrap_or(t.accent);
                let main = if on { t.on_accent } else if ready { t.text } else { t.faint };
                let sub = if on { t.on_accent } else { t.muted };
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .mx(px(8.0))
                    .px(px(10.0))
                    .py(px(7.0))
                    .rounded(px(7.0))
                    .when(on, |d| d.bg(rgb(t.accent)))
                    .when(ready, |d| d.cursor_pointer())
                    .when(ready && !on, |d| d.hover(move |s| s.bg(rgb(hover))))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            if ready {
                                this.start_pane_on(host.clone());
                                cx.notify();
                            }
                        }),
                    )
                    .child(div().size(px(8.0)).flex_none().rounded_full().bg(rgb(dot)))
                    .child(div().flex_1().min_w_0().text_size(px(13.0)).text_color(rgb(main)).truncate().child(SharedString::from(machine.label.clone())))
                    .child(div().flex_none().max_w(px(180.0)).text_size(px(11.5)).text_color(rgb(sub)).truncate().child(SharedString::from(machine.detail.clone())))
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(44.0))
                    .px(px(18.0))
                    .flex()
                    .items_center()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(t.text))
                    .border_b_1()
                    .border_color(rgb(line))
                    .child("New pane on…"),
            )
            .child(div().py(px(7.0)).flex().flex_col().gap(px(1.0)).children(rows))
            .child(
                div()
                    .h(px(34.0))
                    .px(px(18.0))
                    .flex()
                    .items_center()
                    .text_size(px(11.0))
                    .text_color(rgb(t.faint))
                    .border_t_1()
                    .border_color(rgb(line))
                    .child("↑↓ choose  ·  ⏎ open  ·  esc"),
            )
    }

    fn prompt_view(&self, p: &Prompt, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let line = theme::mix(t.palette.bg, t.text, 0.14);
        let confirm = matches!(p.kind, PromptKind::Confirm(_));
        // What the button does, and whether a program stops because of it.
        let (go, stops_programs) = match &p.kind {
            PromptKind::Confirm(Confirm::Trash(_)) => ("Move  ⏎", false),
            PromptKind::Confirm(Confirm::DeleteForever(_) | Confirm::DeleteLayout(_)) => ("Delete  ⏎", false),
            _ => ("Close  ⏎", true),
        };
        let danger = t.agent("errored").unwrap_or(t.accent);
        let hover = rgb(theme::mix(t.palette.bg, t.text, 0.08));
        let button = |label: &'static str| {
            div()
                .h(px(30.0))
                .px(px(14.0))
                .flex()
                .items_center()
                .rounded(px(7.0))
                .text_size(px(12.5))
                .cursor_pointer()
                .child(label)
        };
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(18.0))
                    .pt(px(16.0))
                    .pb(px(if confirm { 4.0 } else { 12.0 }))
                    .text_size(px(14.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(t.text))
                    .child(SharedString::from(p.title.clone())),
            )
            .when(confirm && stops_programs, |d| {
                d.child(
                    div()
                        .px(px(18.0))
                        .pb(px(14.0))
                        .text_size(px(12.0))
                        .text_color(rgb(t.muted))
                        .child("Whatever is running in it stops."),
                )
            })
            .when(!confirm, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(1.0))
                        .mx(px(18.0))
                        .mb(px(12.0))
                        .px(px(10.0))
                        .h(px(32.0))
                        .rounded(px(7.0))
                        .border_1()
                        .border_color(rgb(line))
                        .text_size(px(13.0))
                        .text_color(rgb(t.text))
                        // Marked while the first key would replace it.
                        .child(div().when(p.fresh, |d| d.rounded(px(3.0)).bg(rgba((t.accent << 8) | 0x55))).child(SharedString::from(p.text.clone())))
                        .child(div().w(px(2.0)).h(px(16.0)).bg(rgb(t.accent))),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(8.0))
                    .px(px(14.0))
                    .py(px(12.0))
                    .border_t_1()
                    .border_color(rgb(line))
                    .when(confirm, |d| {
                        let what = match &p.kind {
                            PromptKind::Confirm(w) => Some(w.clone()),
                            _ => None,
                        };
                        d.child(
                            button("Cancel  esc")
                                .text_color(rgb(t.text))
                                .hover(move |s| s.bg(hover))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.overlay = Overlay::None;
                                        cx.notify();
                                    }),
                                ),
                        )
                        .child(
                            button(go).bg(rgb(danger)).text_color(rgb(theme::on(danger))).on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.overlay = Overlay::None;
                                    if let Some(what) = what.clone() {
                                        this.confirmed(what);
                                    }
                                    cx.notify();
                                }),
                            ),
                        )
                    })
                    .when(!confirm, |d| d.child(div().text_size(px(11.0)).text_color(rgb(t.faint)).child("⏎ save  ·  esc"))),
            )
    }

    /// The leader's menu, in tuios's own groups.
    fn help_view(&self) -> impl IntoElement {
        let t = &self.theme;
        let line = theme::mix(t.palette.bg, t.text, 0.14);
        let leader = self.keymap.leader().to_string();
        let groups: Vec<_> = self
            .keymap
            .help()
            .iter()
            .map(|(title, rows)| {
                div()
                    .w(px(290.0))
                    .pb(px(10.0))
                    .child(
                        div()
                            .pb(px(3.0))
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(t.muted))
                            .child(SharedString::from(title.clone())),
                    )
                    .children(rows.iter().map(|(key, description)| {
                        div()
                            .flex()
                            .gap(px(10.0))
                            .text_size(px(12.0))
                            .child(div().w(px(62.0)).flex_none().text_color(rgb(t.accent_text)).truncate().child(SharedString::from(key.clone())))
                            .child(div().flex_1().min_w_0().truncate().text_color(rgb(t.text)).child(SharedString::from(description.clone())))
                    }))
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(44.0))
                    .px(px(18.0))
                    .flex()
                    .items_center()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(t.text))
                    .border_b_1()
                    .border_color(rgb(line))
                    .child(format!("Keys after {leader}")),
            )
            .child(div().p(px(18.0)).flex().flex_wrap().gap(px(10.0)).children(groups))
    }
}
