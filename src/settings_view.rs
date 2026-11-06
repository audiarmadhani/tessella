//! The settings window: a page of controls over `config.toml`.

use super::*;
use gpui::rgba;
use tessella::settings;
use toml_edit::value;

/// Points, at the current zoom.
fn px(v: f32) -> gpui::Pixels {
    gpui::px(super::ui(v))
}

type Action = std::rc::Rc<dyn Fn(&mut Tessella, &mut Context<Tessella>)>;

const SPEEDS: [(&str, u64); 3] = [("Fast", 90), ("Normal", 140), ("Slow", 240)];

impl Section {
    const ALL: [Section; 5] = [Section::General, Section::Appearance, Section::Sidebar, Section::Keyboard, Section::About];

    fn title(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Appearance => "Appearance",
            Section::Sidebar => "Sidebar",
            Section::Keyboard => "Keyboard",
            Section::About => "About",
        }
    }

    fn summary(self) -> &'static str {
        match self {
            Section::General => "How Tessella behaves.",
            Section::Appearance => "Colours, type and size.",
            Section::Sidebar => "What the sidebar shows, and how much room it takes.",
            Section::Keyboard => "Your keys come from tuios. Changes here apply in Tessella only.",
            Section::About => "Version and files.",
        }
    }
}

impl Tessella {
    /// Writes a change to the file, and applies it right away.
    pub(super) fn change_setting(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut toml_edit::DocumentMut)) {
        match settings::save(edit) {
            Ok(()) => {
                self.apply_settings_file(cx, false);
                self.settings_changed = settings_file_time();
            }
            Err(e) => self.status = Some(format!("Could not save the settings: {e}")),
        }
    }

    pub(super) fn settings_key(&mut self, k: &Keystroke, cx: &mut Context<Self>) {
        let Overlay::Settings(page) = &mut self.overlay else { return };
        let m = &k.modifiers;
        let Some(capture) = page.capture.clone() else {
            match k.key.as_str() {
                "escape" => self.overlay = Overlay::None,
                "tab" => {
                    let at = Section::ALL.iter().position(|s| *s == page.section).unwrap_or(0) as i32;
                    let by = if m.shift { -1 } else { 1 };
                    page.section = Section::ALL[(at + by).rem_euclid(Section::ALL.len() as i32) as usize];
                }
                _ => {}
            }
            return;
        };
        page.capture = None;
        if k.key == "escape" {
            return;
        }
        let name = keymap::key_name(&k.key, k.key_char.as_deref(), m.control, m.alt, m.shift, m.platform);
        match capture {
            Capture::Leader => self.change_setting(cx, |doc| doc["keys"]["leader"] = value(name)),
            Capture::Prefix { key, action } => self.change_setting(cx, |doc| {
                let table = &mut doc["keys"]["prefix"];
                table[&key] = value("none");
                table[&name] = value(action);
            }),
        }
    }

    fn listen(&self, f: Action, cx: &mut Context<Self>) -> Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App)> {
        Box::new(cx.listener(move |this, _: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            f(this, cx);
            cx.notify();
        }))
    }

    fn toggle(&self, on: bool, f: Action, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let track = if on { t.accent } else { theme::mix(t.palette.bg, t.text, 0.22) };
        let knob = if on { t.on_accent } else { t.text };
        div()
            .w(px(36.0))
            .h(px(20.0))
            .flex_none()
            .flex()
            .items_center()
            .px(px(3.0))
            .rounded(px(6.0))
            .bg(rgb(track))
            .cursor_pointer()
            .when(on, |d| d.justify_end())
            .on_mouse_down(MouseButton::Left, self.listen(f, cx))
            .child(div().size(px(14.0)).rounded(px(4.0)).bg(rgb(knob)))
    }

    fn stepper(&self, text: String, down: Action, up: Action, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let line = theme::mix(t.palette.bg, t.text, 0.16);
        let hover = rgb(theme::mix(t.palette.bg, t.text, 0.1));
        let step = |label: &'static str, f: Action, cx: &mut Context<Self>| {
            div()
                .w(px(28.0))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(t.text))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(label)
                .on_mouse_down(MouseButton::Left, self.listen(f, cx))
        };
        div()
            .h(px(28.0))
            .flex_none()
            .flex()
            .items_center()
            .rounded(px(7.0))
            .border_1()
            .border_color(rgb(line))
            .overflow_hidden()
            .text_size(px(12.5))
            .child(step("−", down, cx))
            .child(
                div()
                    .min_w(px(52.0))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_l_1()
                    .border_r_1()
                    .border_color(rgb(line))
                    .text_color(rgb(t.text))
                    .child(text),
            )
            .child(step("+", up, cx))
    }

    fn select(&self, text: String, f: Action, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let line = theme::mix(t.palette.bg, t.text, 0.16);
        let hover = rgb(theme::mix(t.palette.bg, t.text, 0.08));
        div()
            .h(px(28.0))
            .max_w(px(260.0))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(10.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(rgb(line))
            .text_size(px(12.5))
            .text_color(rgb(t.text))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_mouse_down(MouseButton::Left, self.listen(f, cx))
            .child(div().min_w_0().truncate().child(text))
            .child(div().flex_none().text_color(rgb(t.muted)).child("▾"))
    }

    fn row(&self, title: &str, detail: &str, control: impl IntoElement) -> impl IntoElement {
        let t = &self.theme;
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(20.0))
            .px(px(14.0))
            .py(px(10.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.0))
                    .child(div().text_size(px(13.0)).text_color(rgb(t.text)).child(title.to_string()))
                    .when(!detail.is_empty(), |d| {
                        d.child(div().text_size(px(11.5)).text_color(rgb(t.muted)).child(detail.to_string()))
                    }),
            )
            .child(control)
    }

    /// A group of rows on one surface, with a hairline between them.
    fn card(&self, rows: Vec<gpui::AnyElement>) -> impl IntoElement {
        let t = &self.theme;
        let line = theme::mix(t.palette.bg, t.text, 0.1);
        let count = rows.len();
        div()
            .flex()
            .flex_col()
            .rounded(px(10.0))
            .border_1()
            .border_color(rgb(line))
            .bg(rgb(theme::mix(t.palette.bg, t.text, 0.025)))
            .children(rows.into_iter().enumerate().map(|(i, row)| {
                div().when(i + 1 < count, |d| d.border_b_1().border_color(rgb(line))).child(row)
            }))
    }

    fn general_page(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let t = &self.theme;
        let s = &self.settings;
        let confirm = s.panes.confirm_close;
        let predict = s.panes.predict_remote_typing;
        let animated = s.animation.enabled;
        let current = s.animation.duration_ms;
        let segments: Vec<_> = SPEEDS
            .iter()
            .map(|(label, ms)| {
                let (label, ms) = (*label, *ms);
                let on = current == ms;
                let f: Action = std::rc::Rc::new(move |this, cx| {
                    this.change_setting(cx, |doc| doc["animation"]["duration_ms"] = value(ms as i64));
                });
                div()
                    .h_full()
                    .px(px(11.0))
                    .flex()
                    .items_center()
                    .rounded(px(5.0))
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .when(on, |d| d.bg(rgb(t.accent)).text_color(rgb(t.on_accent)))
                    .when(!on, |d| d.text_color(rgb(t.muted)))
                    .child(label)
                    .on_mouse_down(MouseButton::Left, self.listen(f, cx))
            })
            .collect();
        let speed = div()
            .h(px(28.0))
            .flex_none()
            .flex()
            .p(px(2.0))
            .rounded(px(7.0))
            .bg(rgb(theme::mix(t.palette.bg, t.text, 0.09)))
            .children(segments);
        vec![
            self.card(vec![
                self.row(
                    "Confirm before closing a pane",
                    "Ask first when a key would close a pane and whatever runs in it.",
                    self.toggle(
                        confirm,
                        std::rc::Rc::new(move |this, cx| {
                            this.change_setting(cx, |doc| doc["panes"]["confirm_close"] = value(!confirm));
                        }),
                        cx,
                    ),
                )
                .into_any_element(),
                self.row(
                    "Predict typing on other machines",
                    "Show letters as you type, before the other machine echoes them. Passwords stay hidden.",
                    self.toggle(
                        predict,
                        std::rc::Rc::new(move |this, cx| {
                            this.change_setting(cx, |doc| doc["panes"]["predict_remote_typing"] = value(!predict));
                        }),
                        cx,
                    ),
                )
                .into_any_element(),
            ])
            .into_any_element(),
            self.card(vec![
                self.row(
                    "Animations",
                    "Slide workspaces, and ease the sidebar and dialogs.",
                    self.toggle(
                        animated,
                        std::rc::Rc::new(move |this, cx| {
                            this.change_setting(cx, |doc| doc["animation"]["enabled"] = value(!animated));
                        }),
                        cx,
                    ),
                )
                .into_any_element(),
                self.row("Speed", "", speed).into_any_element(),
            ])
            .into_any_element(),
        ]
    }

    fn appearance_page(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let t = &self.theme;
        let swatches: Vec<_> = t.palette.ansi[1..7].iter().map(|c| div().size(px(12.0)).rounded(px(3.0)).bg(rgb(*c))).collect();
        let size = self.look.size;
        let set_size = move |by: f32| -> Action {
            std::rc::Rc::new(move |this: &mut Tessella, cx: &mut Context<Tessella>| {
                let new = (size + by).clamp(FONT_MIN, FONT_MAX);
                this.change_setting(cx, |doc| doc["font"]["size"] = value(new as f64));
                this.set_font_size(new);
            })
        };
        vec![self
            .card(vec![
                self.row(
                    "Theme",
                    "",
                    div().flex().items_center().gap(px(10.0)).child(div().flex().gap(px(3.0)).children(swatches)).child(self.select(
                        theme::pretty(&t.name),
                        std::rc::Rc::new(|this, _| {
                            this.settings_return = true;
                            this.open_themes();
                        }),
                        cx,
                    )),
                )
                .into_any_element(),
                self.row(
                    "Font",
                    "One font is used everywhere. A Nerd Font draws icons.",
                    self.select(
                        self.look.font.to_string(),
                        std::rc::Rc::new(|this, cx| {
                            this.settings_return = true;
                            this.open_fonts(cx);
                        }),
                        cx,
                    ),
                )
                .into_any_element(),
                self.row(
                    "Text size",
                    "Zooms the interface with it. ⌘+ and ⌘− change it for now.",
                    self.stepper(format!("{size:.0} pt"), set_size(-1.0), set_size(1.0), cx),
                )
                .into_any_element(),
            ])
            .into_any_element()]
    }

    fn sidebar_page(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let open = self.rail_open;
        let width = self.rail_width;
        let agents = self.agents_height;
        let set_width = move |by: f32| -> Action {
            std::rc::Rc::new(move |this: &mut Tessella, cx: &mut Context<Tessella>| {
                let new = (width + by).clamp(SIDEBAR_MIN, SIDEBAR_MAX);
                this.change_setting(cx, |doc| doc["sidebar"]["width"] = value(new as f64));
            })
        };
        let set_agents = move |by: f32| -> Action {
            std::rc::Rc::new(move |this: &mut Tessella, cx: &mut Context<Tessella>| {
                let new = (agents + by).clamp(AGENTS_MIN, AGENTS_MAX);
                this.change_setting(cx, |doc| doc["sidebar"]["agents_height"] = value(new as f64));
            })
        };
        vec![self
            .card(vec![
                self.row(
                    "Show the sidebar",
                    "",
                    self.toggle(
                        open,
                        std::rc::Rc::new(move |this, cx| {
                            this.change_setting(cx, |doc| doc["sidebar"]["visible"] = value(!open));
                        }),
                        cx,
                    ),
                )
                .into_any_element(),
                self.row("Width", "", self.stepper(format!("{width:.0}"), set_width(-10.0), set_width(10.0), cx)).into_any_element(),
                self.row(
                    "Agents section height",
                    "Or drag the line above it.",
                    self.stepper(format!("{agents:.0}"), set_agents(-20.0), set_agents(20.0), cx),
                )
                .into_any_element(),
            ])
            .into_any_element()]
    }

    fn key_chip(&self, text: String, capturing: bool, f: Action, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let line = theme::mix(t.palette.bg, t.text, 0.16);
        div()
            .h(px(24.0))
            .min_w(px(36.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .px(px(8.0))
            .rounded(px(6.0))
            .border_1()
            .text_size(px(12.0))
            .cursor_pointer()
            .when(capturing, |d| d.border_color(rgb(t.accent)).text_color(rgb(t.accent_text)))
            .when(!capturing, |d| d.border_color(rgb(line)).text_color(rgb(t.text)))
            .child(if capturing { "Press a key…".to_string() } else { text })
            .on_mouse_down(MouseButton::Left, self.listen(f, cx))
    }

    fn keyboard_page(&self, page: &SettingsPage, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let t = &self.theme;
        let leader = self.keymap.leader().to_string();
        let capturing_leader = matches!(page.capture, Some(Capture::Leader));
        let customised = !self.settings.keys.leader.is_empty() || !self.settings.keys.prefix.is_empty();
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        for (section, keys) in self.keymap.prefix_sections() {
            rows.push(
                div()
                    .flex_none()
                    .px(px(14.0))
                    .pt(px(if rows.is_empty() { 6.0 } else { 14.0 }))
                    .pb(px(4.0))
                    .text_size(px(11.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(t.muted))
                    .child(section)
                    .into_any_element(),
            );
            for (key, action, description) in keys {
                let capturing = matches!(&page.capture, Some(Capture::Prefix { key: k, .. }) if *k == key);
                let (k, a) = (key.clone(), action.clone());
                let f: Action = std::rc::Rc::new(move |this, _| {
                    if let Overlay::Settings(p) = &mut this.overlay {
                        p.capture = Some(Capture::Prefix { key: k.clone(), action: a.clone() });
                    }
                });
                rows.push(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_between()
                        .gap(px(12.0))
                        .px(px(14.0))
                        .py(px(3.0))
                        .child(div().flex_1().min_w_0().truncate().text_size(px(12.5)).text_color(rgb(t.text)).child(description))
                        .child(self.key_chip(key, capturing, f, cx))
                        .into_any_element(),
                );
            }
        }
        let line = theme::mix(t.palette.bg, t.text, 0.1);
        vec![
            self.card(vec![self
                .row(
                    "Prefix key",
                    "Press this, then one of the keys below.",
                    self.key_chip(
                        leader,
                        capturing_leader,
                        std::rc::Rc::new(|this, _| {
                            if let Overlay::Settings(p) = &mut this.overlay {
                                p.capture = Some(Capture::Leader);
                            }
                        }),
                        cx,
                    ),
                )
                .into_any_element()])
            .into_any_element(),
            div()
                .id("settings-keys")
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .pb(px(8.0))
                .overflow_y_scroll()
                .rounded(px(10.0))
                .border_1()
                .border_color(rgb(line))
                .children(rows)
                .into_any_element(),
            div()
                .flex()
                .justify_end()
                .when(customised, |d| {
                    let f: Action = std::rc::Rc::new(|this, cx| {
                        this.change_setting(cx, |doc| {
                            doc.remove("keys");
                        });
                    });
                    d.child(self.select("Use my tuios keys".to_string(), f, cx))
                })
                .into_any_element(),
        ]
    }

    fn about_page(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
        let path = settings::path().map(|p| p.display().to_string()).unwrap_or_default();
        vec![self
            .card(vec![
                self.row("Tessella", "A native GUI for tuios. MIT licence.", {
                    let t = &self.theme;
                    div().text_size(px(12.5)).text_color(rgb(t.muted)).child(concat!("v", env!("CARGO_PKG_VERSION")))
                })
                .into_any_element(),
                self.row(
                    "Settings file",
                    &path,
                    self.select(
                        "Open".to_string(),
                        std::rc::Rc::new(|this, _| {
                            this.overlay = Overlay::None;
                            this.open_settings_file();
                        }),
                        cx,
                    ),
                )
                .into_any_element(),
            ])
            .into_any_element()]
    }

    pub(super) fn settings_view(&self, page: &SettingsPage, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let line = theme::mix(t.palette.bg, t.text, 0.1);
        let hover = rgb(theme::mix(t.sidebar, t.text, 0.07));
        let nav: Vec<_> = Section::ALL
            .iter()
            .map(|s| {
                let s = *s;
                let on = s == page.section;
                let f: Action = std::rc::Rc::new(move |this, _| {
                    if let Overlay::Settings(p) = &mut this.overlay {
                        p.section = s;
                        p.capture = None;
                    }
                });
                div()
                    .flex()
                    .items_center()
                    .h(px(30.0))
                    .px(px(10.0))
                    .rounded(px(7.0))
                    .text_size(px(13.0))
                    .cursor_pointer()
                    .when(on, |d| d.bg(rgba((t.accent << 8) | 0x2e)).text_color(rgb(t.text)).font_weight(FontWeight::SEMIBOLD))
                    .when(!on, |d| d.text_color(rgb(t.muted)).hover(move |x| x.bg(hover)))
                    .child(s.title())
                    .on_mouse_down(MouseButton::Left, self.listen(f, cx))
            })
            .collect();
        let body = match page.section {
            Section::General => self.general_page(cx),
            Section::Appearance => self.appearance_page(cx),
            Section::Sidebar => self.sidebar_page(cx),
            Section::Keyboard => self.keyboard_page(page, cx),
            Section::About => self.about_page(cx),
        };
        let close: Action = std::rc::Rc::new(|this, _| this.overlay = Overlay::None);
        div()
            .h(px(540.0))
            .flex()
            .rounded(px(14.0))
            .overflow_hidden()
            .child(
                div()
                    .w(px(172.0))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .p(px(10.0))
                    .bg(rgb(t.sidebar))
                    .border_r_1()
                    .border_color(rgb(line))
                    .child(
                        div()
                            .px(px(10.0))
                            .pt(px(8.0))
                            .pb(px(12.0))
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(t.text))
                            .child("Settings"),
                    )
                    .children(nav),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_start()
                            .justify_between()
                            .gap(px(12.0))
                            .px(px(24.0))
                            .pt(px(20.0))
                            .pb(px(14.0))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.0))
                                    .child(
                                        div()
                                            .text_size(px(17.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(t.text))
                                            .child(page.section.title()),
                                    )
                                    .child(div().text_size(px(12.0)).text_color(rgb(t.muted)).child(page.section.summary())),
                            )
                            .child(
                                div()
                                    .size(px(26.0))
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(7.0))
                                    .text_size(px(15.0))
                                    .text_color(rgb(t.muted))
                                    .cursor_pointer()
                                    .hover(move |x| x.bg(hover))
                                    .child("×")
                                    .on_mouse_down(MouseButton::Left, self.listen(close, cx)),
                            ),
                    )
                    .child(div().flex_1().min_h_0().flex().flex_col().gap(px(12.0)).px(px(24.0)).pb(px(20.0)).children(body)),
            )
    }
}
