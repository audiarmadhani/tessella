//! Draws a Markdown file in its pane, and scrolls it. The file reloads when it changes.

use super::*;
use gpui::{TextAlign, WrappedLine};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use tessella::markdown::{self, Block, Span};

/// A line of text that holds links, to find them under a click.
pub(super) struct LinkHit {
    bounds: Bounds<Pixels>,
    line: WrappedLine,
    height: Pixels,
    links: Vec<(std::ops::Range<usize>, String)>,
}

/// A Markdown file on show.
pub(super) struct MdDoc {
    path: PathBuf,
    blocks: Rc<Vec<Block>>,
    changed: Option<SystemTime>,
    checked: Instant,
    pub scroll: f32,
    /// The furthest it can scroll, known once it has been drawn.
    max_scroll: Rc<Cell<f32>>,
    links: Rc<RefCell<Vec<LinkHit>>>,
}

/// What one frame needs to draw a file.
#[derive(Clone)]
pub(super) struct DocDraw {
    blocks: Rc<Vec<Block>>,
    scroll: f32,
    max_scroll: Rc<Cell<f32>>,
    links: Rc<RefCell<Vec<LinkHit>>>,
}

fn read(path: &Path) -> (Vec<Block>, Option<SystemTime>) {
    let changed = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    match std::fs::read_to_string(path) {
        Ok(text) => (markdown::parse(&text), changed),
        Err(err) => (vec![Block::Paragraph(vec![Span { text: format!("Could not read {}: {err}", path.display()), ..Default::default() }])], changed),
    }
}

impl Tessella {
    /// Keeps a document for each Markdown pane, and reloads a file that changed.
    pub(super) fn refresh_docs(&mut self, cx: &mut Context<Self>) {
        let wanted: Vec<(String, PathBuf)> = self.state.windows.iter().filter_map(|w| Some((w.id.clone(), w.markdown_path()?))).collect();
        self.docs.retain(|id, _| wanted.iter().any(|(w, _)| w == id));
        for (id, path) in wanted {
            match self.docs.get_mut(&id) {
                Some(doc) if doc.checked.elapsed() >= Duration::from_secs(1) => {
                    doc.checked = Instant::now();
                    if std::fs::metadata(&path).and_then(|m| m.modified()).ok() != doc.changed {
                        let (blocks, changed) = read(&path);
                        doc.blocks = Rc::new(blocks);
                        doc.changed = changed;
                    }
                }
                Some(_) => {}
                None => {
                    let (blocks, changed) = read(&path);
                    let doc = MdDoc { path, blocks: Rc::new(blocks), changed, checked: Instant::now(), scroll: 0.0, max_scroll: Rc::default(), links: Rc::default() };
                    self.docs.insert(id, doc);
                }
            }
        }
        // An idle window does not redraw, and a file can change while it sits there.
        if !self.docs.is_empty() && !self.doc_timer {
            self.doc_timer = true;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let _ = this.update(cx, |this, cx| {
                    this.doc_timer = false;
                    cx.notify();
                });
            })
            .detach();
        }
    }

    pub(super) fn doc_draw(&self, window_id: &str) -> Option<DocDraw> {
        let doc = self.docs.get(window_id)?;
        Some(DocDraw { blocks: doc.blocks.clone(), scroll: doc.scroll, max_scroll: doc.max_scroll.clone(), links: doc.links.clone() })
    }

    /// Scrolls a Markdown pane by points, down when positive.
    pub(super) fn scroll_doc(&mut self, window_id: &str, by: f32) {
        if let Some(doc) = self.docs.get_mut(window_id) {
            doc.scroll = (doc.scroll + by).clamp(0.0, doc.max_scroll.get().max(0.0));
        }
    }

    /// A key on a focused Markdown pane. It says whether the pane took the key.
    pub(super) fn doc_key(&mut self, key: &str) -> bool {
        let Some(id) = self.focused_win().filter(|w| w.markdown_path().is_some()).map(|w| w.id.clone()) else { return false };
        let page = ui(300.0);
        let line = ui(40.0);
        match key {
            "j" | "down" => self.scroll_doc(&id, line),
            "k" | "up" => self.scroll_doc(&id, -line),
            "space" | "pagedown" | "d" => self.scroll_doc(&id, page),
            "pageup" | "u" => self.scroll_doc(&id, -page),
            "g" | "home" => self.scroll_doc(&id, f32::MIN),
            "G" | "end" => self.scroll_doc(&id, f32::MAX),
            "e" => {
                if let Some(path) = self.docs.get(&id).map(|d| d.path.clone()) {
                    self.edit_file(path);
                }
            }
            _ => {}
        }
        // A file has no program behind it to type at, so no key goes to the pane.
        true
    }

    /// A click on a Markdown pane: it follows a link, if one is under it.
    pub(super) fn click_doc(&mut self, window_id: &str, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get(window_id) else { return };
        let target = doc.links.borrow().iter().find_map(|hit| {
            if !hit.bounds.contains(&position) {
                return None;
            }
            let index = hit.line.index_for_position(position - hit.bounds.origin, hit.height).ok()?;
            hit.links.iter().find(|(range, _)| range.contains(&index)).map(|(_, url)| url.clone())
        });
        let Some(target) = target else { return };
        let base = doc.path.parent().map(Path::to_path_buf);
        if target.contains("://") || target.starts_with("mailto:") {
            cx.open_url(&target);
        } else if !target.starts_with('#') {
            // A path beside the file, without any #section after it.
            let relative = target.split('#').next().unwrap_or_default().to_string();
            if let Some(path) = base.map(|b| b.join(relative)) {
                self.open_file(path);
            }
        }
    }

    /// Paints a file inside a pane's card.
    pub(super) fn paint_doc(doc: &DocDraw, area: Bounds<Pixels>, t: &Theme, look: &Look, window: &mut Window, cx: &mut App) {
        let base = look.size;
        let line_height = (base * 1.5).round();
        let left = f32::from(area.origin.x);
        let width = f32::from(area.size.width);
        let mut y = f32::from(area.origin.y) - doc.scroll;
        let start = y;
        let bottom = f32::from(area.origin.y + area.size.height);
        doc.links.borrow_mut().clear();
        let code_bg = theme::mix(t.palette.bg, t.text, 0.08);
        let rule = theme::mix(t.palette.bg, t.text, 0.22);
        let mut after_list = false;
        for block in doc.blocks.iter() {
            // A list ends with room before whatever follows it.
            let in_list = matches!(block, Block::Item { .. });
            if after_list && !in_list {
                y += base * 0.7;
            }
            after_list = in_list;
            match block {
                Block::Heading { level, spans } => {
                    y += base * if *level <= 2 { 1.1 } else { 0.7 };
                    let size = base * match level {
                        1 => 1.7,
                        2 => 1.4,
                        3 => 1.2,
                        _ => 1.05,
                    };
                    let height = (size * 1.35).round();
                    y += Self::paint_spans(doc, spans, (left, y), width, size, height, Some(t.text), true, t, look, window, cx);
                    if *level <= 2 {
                        window.paint_quad(fill(Bounds::new(point(px(left), px(y + 3.0)), size_of(width, 1.0)), rgb(rule)));
                        y += 7.0;
                    }
                    y += base * 0.5;
                }
                Block::Paragraph(spans) => {
                    y += Self::paint_spans(doc, spans, (left, y), width, base, line_height, None, false, t, look, window, cx);
                    y += base * 0.8;
                }
                Block::Item { depth, marker, spans } => {
                    let indent = ui(20.0) * *depth as f32;
                    let gutter = ui(22.0);
                    if !marker.is_empty() {
                        let run = plain_run(marker.len(), rgb(t.accent_text).into(), font(look.font.clone()));
                        let mark = window.text_system().shape_line(marker.clone().into(), px(base), &[run], None);
                        let _ = mark.paint(point(px(left + indent), px(y)), px(line_height), window, cx);
                    }
                    y += Self::paint_spans(doc, spans, (left + indent + gutter, y), width - indent - gutter, base, line_height, None, false, t, look, window, cx);
                    y += base * 0.2;
                }
                Block::Quote(spans) => {
                    let inset = ui(14.0);
                    let top = y;
                    y += Self::paint_spans(doc, spans, (left + inset, y), width - inset, base, line_height, Some(t.muted), false, t, look, window, cx);
                    window.paint_quad(fill(Bounds::new(point(px(left), px(top)), size_of(3.0, y - top)), rgb(t.accent)));
                    y += base * 0.8;
                }
                Block::Code { lines } => {
                    let pad = ui(10.0);
                    let height = lines.len() as f32 * line_height + pad * 2.0;
                    let bounds = Bounds::new(point(px(left), px(y)), size_of(width, height));
                    if y < bottom && y + height > f32::from(area.origin.y) {
                        window.paint_quad(fill(bounds, rgb(code_bg)).corner_radii(px(ui(6.0))));
                        window.with_content_mask(Some(ContentMask { bounds }), |window| {
                            for (i, line) in lines.iter().enumerate() {
                                let run = plain_run(line.len(), rgb(t.text).into(), font(look.font.clone()));
                                let shaped = window.text_system().shape_line(line.clone().into(), px(base * 0.95), &[run], None);
                                let _ = shaped.paint(point(px(left + pad), px(y + pad + i as f32 * line_height)), px(line_height), window, cx);
                            }
                        });
                    }
                    y += height + base * 0.8;
                }
                Block::Rule => {
                    y += base * 0.5;
                    window.paint_quad(fill(Bounds::new(point(px(left), px(y)), size_of(width, 1.0)), rgb(rule)));
                    y += base * 1.2;
                }
                Block::Table(rows) => y += Self::paint_table(rows, (left, y), width, line_height, t, look, window, cx) + base * 0.8,
            }
        }
        let content = y - start;
        doc.max_scroll.set((content - f32::from(area.size.height)).max(0.0));
    }

    /// Paints text with its styles, wrapped, and answers with its height.
    #[allow(clippy::too_many_arguments)]
    fn paint_spans(doc: &DocDraw, spans: &[Span], (x, y): (f32, f32), width: f32, size: f32, height: f32, color: Option<u32>, heading: bool, t: &Theme, look: &Look, window: &mut Window, cx: &mut App) -> f32 {
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
        if text.is_empty() {
            return 0.0;
        }
        let code_bg = theme::mix(t.palette.bg, t.text, 0.12);
        let mut links = Vec::new();
        let mut at = 0;
        let runs: Vec<TextRun> = spans
            .iter()
            .map(|s| {
                let mut f: Font = font(look.font.clone());
                if s.bold || heading {
                    f.weight = FontWeight::BOLD;
                }
                if s.italic {
                    f.style = FontStyle::Italic;
                }
                let linked = s.link.is_some();
                let hue = if linked || s.code { t.accent_text } else { color.unwrap_or(t.text) };
                let mut run = plain_run(s.text.len(), rgb(hue).into(), f);
                if s.code {
                    run.background_color = Some(rgb(code_bg).into());
                }
                if linked {
                    run.underline = Some(UnderlineStyle { thickness: px(1.0), ..Default::default() });
                }
                if s.strike {
                    run.strikethrough = Some(gpui::StrikethroughStyle { thickness: px(1.0), ..Default::default() });
                }
                if let Some(url) = &s.link {
                    links.push((at..at + s.text.len(), url.clone()));
                }
                at += s.text.len();
                run
            })
            .collect();
        let shaped = window.text_system().shape_text(text.into(), px(size), &runs, Some(px(width)), None);
        let mut total = 0.0;
        for line in shaped.into_iter().flatten() {
            let line_size = line.size(px(height));
            let origin = point(px(x), px(y + total));
            let _ = line.paint(origin, px(height), TextAlign::Left, None, window, cx);
            if !links.is_empty() {
                doc.links.borrow_mut().push(LinkHit { bounds: Bounds::new(origin, line_size), line: line.clone(), height: px(height), links: links.clone() });
            }
            total += f32::from(line_size.height);
        }
        total
    }

    /// Paints a table with columns as wide as they need, and answers with its height.
    #[allow(clippy::too_many_arguments)]
    fn paint_table(rows: &[Vec<Vec<Span>>], (x, y): (f32, f32), width: f32, height: f32, t: &Theme, look: &Look, window: &mut Window, cx: &mut App) -> f32 {
        let text = |cell: &Vec<Span>| cell.iter().map(|s| s.text.as_str()).collect::<String>();
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        let char_width = look.size * 0.62;
        let pad = ui(10.0);
        let widths: Vec<f32> = (0..columns)
            .map(|c| rows.iter().filter_map(|r| r.get(c)).map(|cell| text(cell).chars().count()).max().unwrap_or(0) as f32 * char_width + pad * 2.0)
            .collect();
        let row_height = height + ui(5.0);
        let rule = theme::mix(t.palette.bg, t.text, 0.22);
        let table_w: f32 = widths.iter().sum::<f32>().min(width);
        let bounds = Bounds::new(point(px(x), px(y)), size_of(table_w, rows.len() as f32 * row_height));
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for (r, row) in rows.iter().enumerate() {
                let top = y + r as f32 * row_height;
                let mut left = x;
                if r == 0 {
                    window.paint_quad(fill(Bounds::new(point(px(x), px(top)), size_of(table_w, row_height)), rgb(theme::mix(t.palette.bg, t.text, 0.08))));
                }
                for (c, cell) in row.iter().enumerate() {
                    let content = text(cell);
                    let mut f: Font = font(look.font.clone());
                    if r == 0 {
                        f.weight = FontWeight::BOLD;
                    }
                    let run = plain_run(content.len(), rgb(t.text).into(), f);
                    let shaped = window.text_system().shape_line(content.into(), px(look.size * 0.95), &[run], None);
                    let _ = shaped.paint(point(px(left + pad), px(top + ui(2.5))), px(height), window, cx);
                    left += widths.get(c).copied().unwrap_or(0.0);
                }
                window.paint_quad(fill(Bounds::new(point(px(x), px(top + row_height - 1.0)), size_of(table_w, 1.0)), rgb(rule)));
            }
        });
        rows.len() as f32 * row_height
    }
}

fn size_of(w: f32, h: f32) -> gpui::Size<Pixels> {
    size(px(w), px(h))
}
