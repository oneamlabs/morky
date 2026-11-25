//! Drawing the window's panes: each source line as a virtualised list item (with its syntax colours,
//! find matches, selection, script text and caret), the preview pane, and the caret and accessibility
//! helpers that need the painted text layouts.

use super::*;

impl Prototype {
    /// One source line, laid out only when the list needs it. Syntax colours, find matches and the
    /// selection are layered highlights on the line's own `StyledText`, whose layout also maps clicks
    /// and places the caret.
    fn render_line(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let dark = self.dark;
        let doc = &self.tabs.active_doc();
        let start = doc.buffer.line_start(ix);
        let text = doc.buffer.line(ix);
        let line = start..start + text.len();
        let selected = doc.selected.clone();
        let cursor = doc.cursor();
        // `~sub~` and `^sup^` are drawn at 80% and shifted; their delimiters are hidden. Ranges
        // inside inline code are left as typed, as in the preview.
        let scripts: Vec<(Range<usize>, bool)> = preview::script_ranges(&text)
            .into_iter()
            .filter(|(range, _)| {
                !doc.spans
                    .get(ix)
                    .is_some_and(|spans| spans.iter().any(|(span, token)| *token == syntax::Token::Code && span.start < range.end && range.start < span.end))
            })
            .collect();

        let mut highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
        if !text.is_empty() {
            // Colours from the last parse. Right after an edit they can be a few characters off, or
            // past the end of the line, until the next parse; clamp them to character boundaries.
            for (range, token) in doc.spans.get(ix).map(Vec::as_slice).unwrap_or(&[]) {
                let span = text.floor_char_boundary(range.start)..text.floor_char_boundary(range.end);
                if !span.is_empty() {
                    highlights.push((span, token.style(dark)));
                }
            }
            let found_style = HighlightStyle { background_color: Some(tone(dark, 0x713f12, 0xfef08a).into()), ..HighlightStyle::default() };
            let matches = self.matches();
            let first = matches.partition_point(|found| found.end <= line.start);
            for found in matches[first..].iter().take_while(|found| found.start < line.end) {
                if found.start < selected.end && selected.start < found.end {
                    continue;
                }
                highlights.push((found.start.max(line.start) - start..found.end.min(line.end) - start, found_style));
            }
            let overlap = selected.start.max(line.start)..selected.end.min(line.end);
            if !overlap.is_empty() {
                let selection_style = HighlightStyle { background_color: Some(tone(dark, 0x1e3a8a, 0xbfdbfe).into()), ..HighlightStyle::default() };
                highlights.push((overlap.start - start..overlap.end - start, selection_style));
            }
            // The full-size text under a script stays for layout, caret and clicks, but is not drawn;
            // the 80% copy is painted over it in the canvas below, with the delimiters hidden.
            let hidden = HighlightStyle { color: Some(gpui::transparent_black()), ..HighlightStyle::default() };
            for (range, _) in &scripts {
                highlights.push((range.start.saturating_sub(1)..(range.end + 1).min(text.len()), hidden));
            }
        }
        // One copy of the line, shared by the text, the script overlay and the caret lookup.
        let text = SharedString::from(text);
        let styled_text = if text.is_empty() { SharedString::from(" ") } else { text.clone() };
        let styled = StyledText::new(styled_text).with_highlights(syntax::merge(&highlights));
        let layout = styled.layout().clone();
        self.prepaint_lines.borrow_mut().push((ix, layout.clone()));
        let click_layout = layout.clone();
        let drag_layout = layout.clone();
        let script_text = text.clone();
        let base_size = self.settings.font_size;
        let row = div()
            .relative()
            .px_3()
            .when(ix == 0, |row| row.pt_2())
            .map(|row| if self.settings.word_wrap { row.whitespace_normal() } else { row.whitespace_nowrap() })
            .child(styled)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.click_line(ix, &click_layout, event, window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| this.drag_line(ix, &drag_layout, event, cx)));
        let caret_column = (selected.is_empty() && (line.start..=line.end).contains(&cursor)).then(|| cursor - line.start);
        let caret_next = caret_column.and_then(|column| text.get(column..)?.chars().next().map(|ch| column + ch.len_utf8()));
        let view = cx.entity();
        // Paints the caret, and records the line's layout once it has been painted: Home and End on
        // wrapped rows and the IME's point-to-character lookup may only use painted layouts.
        row.child(
            canvas(
                |_, _, _| {},
                move |_, _, window, cx| {
                    let caret = caret_column.and_then(|column| caret_origin(&layout, column, caret_next)).map(|origin| Bounds::new(origin, size(px(1.5), layout.line_height())));
                    if let Some(caret) = caret {
                        let color = crate::high_contrast::caret().unwrap_or_else(|| rgb(CARET));
                        window.paint_quad(fill(caret, color));
                    }
                    // The small script text: 80% of the line, above or below the baseline. The
                    // full-size copy underneath is transparent, so only this is visible.
                    let base = window.text_style();
                    for (range, sup) in &scripts {
                        let Some(origin) = layout.position_for_index(range.start) else { continue };
                        let content = &script_text[range.clone()];
                        if content.is_empty() {
                            continue;
                        }
                        let run = TextRun {
                            len: content.len(),
                            font: base.font(),
                            color: base.color,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        let line_height = layout.line_height() * 0.8;
                        let small = window.text_system().shape_line(
                            SharedString::from(content.to_string()),
                            px(base_size * 0.8),
                            std::slice::from_ref(&run),
                            None,
                        );
                        let top = layout.bounds().top() + if *sup { px(0.) } else { layout.line_height() * 0.45 };
                        small.paint(point(origin.x, top), line_height, TextAlign::Left, None, window, cx).ok();
                    }
                    view.update(cx, |view, cx| {
                        view.painted_lines.push((ix, layout.clone()));
                        let Some(caret) = caret else { return };
                        view.caret_bounds = Some(caret);
                        view.caret_painted = Some((cursor, caret));
                        // Painted where it moved to: one more frame scrolls sideways to it.
                        if view.follow_x {
                            cx.notify();
                        }
                    });
                },
            )
            .absolute()
            .size_full(),
        )
        .into_any_element()
    }

    /// The layout of source line `line` if it was painted in the last frame.
    pub(super) fn painted_layout(&self, line: usize) -> Option<&TextLayout> {
        self.painted_lines.iter().find(|(painted, _)| *painted == line).map(|(_, layout)| layout)
    }

    /// The visual row the caret is on, as byte offsets into its line, when word wrap has split the
    /// line into several rows on screen.
    fn caret_row(&self) -> Option<Range<usize>> {
        if !self.settings.word_wrap {
            return None;
        }
        let doc = self.doc();
        let cursor = doc.cursor();
        let line = doc.buffer.line_of(cursor);
        let layout = self.painted_layout(line)?;
        let column = cursor - doc.buffer.line_start(line);
        let text = doc.buffer.line(line);
        let next = text.get(column..).and_then(|rest| rest.chars().next()).map(|ch| column + ch.len_utf8());
        let caret = caret_origin(layout, column, next)?;
        let bounds = layout.bounds();
        let y = caret.y + layout.line_height() / 2.;
        let (Ok(start) | Err(start)) = layout.index_for_position(point(bounds.left(), y));
        let (Ok(end) | Err(end)) = layout.index_for_position(point(bounds.right() - px(1.), y));
        let length = doc.buffer.line_range(line).len();
        (start > 0 || end < length).then(|| start.min(length)..end.min(length))
    }

    /// Home, as CodeMirror's `cursorLineBoundaryBackward`: the start of the visual row; from there
    /// (or on the first row), the end of the indentation, then the start of the line.
    pub(super) fn home_target(&self) -> usize {
        let doc = self.doc();
        let cursor = doc.cursor();
        let line = doc.buffer.line_of(cursor);
        let start = doc.buffer.line_start(line);
        if let Some(row) = self.caret_row().filter(|row| row.start > 0 && start + row.start != cursor) {
            return start + row.start;
        }
        let text = doc.buffer.line(line);
        let indent = text.len() - text.trim_start().len();
        if indent > 0 && cursor != start + indent { start + indent } else { start }
    }

    /// End: the end of the visual row, then the end of the line. Without caret affinity the end of
    /// a wrapped row is drawn at the start of the next one, so the caret stops before the space the
    /// row was broken at (or its last character).
    pub(super) fn end_target(&self) -> usize {
        let doc = self.doc();
        let cursor = doc.cursor();
        let line = doc.buffer.line_of(cursor);
        let range = doc.buffer.line_range(line);
        let Some(row) = self.caret_row().filter(|row| row.end < range.len()) else { return range.end };
        let text = doc.buffer.line(line);
        let target = range.start + text.floor_char_boundary(row.end.saturating_sub(1)).max(row.start);
        if target == cursor { range.end } else { target }
    }

    /// The editor's lines and selection for assistive technology (`a11y.rs`). Nothing is gathered
    /// while no screen reader or other client is listening.
    fn describe_text(&self, window: &Window) -> Option<(Vec<a11y::Line>, a11y::Point, a11y::Point)> {
        if !window.is_a11y_active() {
            return None;
        }
        let doc = self.doc();
        let buffer = &doc.buffer;
        let point_of = |offset: usize| {
            let line = buffer.line_of(offset);
            let start = buffer.line_start(line);
            a11y::Point { line, char_index: buffer.slice(start..offset).chars().count() }
        };
        let anchor_offset = if doc.selection_reversed { doc.selected.end } else { doc.selected.start };
        let (focus, anchor) = (point_of(doc.cursor()), point_of(anchor_offset));
        let last = buffer.line_count().saturating_sub(1);
        let lines = a11y::lines_to_describe(doc.lines.logical_scroll_top().item_ix, buffer.line_count(), focus.line, anchor.line)
            .into_iter()
            .map(|index| {
                let mut text = buffer.line(index);
                if index < last {
                    text.push('\n');
                }
                a11y::Line { index, text, bounds: None, positions: None, widths: None }
            })
            .collect();
        let lines = a11y::within_budget(lines, &[focus.line, anchor.line]);
        Some((lines, anchor, focus))
    }

    /// The editing keys, which act only while the editor has the focus, so Enter and Space can press
    /// a focused button and the arrows can move through a menu.
    fn editor_actions<E: InteractiveElement>(&self, element: E, cx: &mut Context<Self>) -> E {
        element
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_line_start))
            .on_action(cx.listener(Self::select_line_end))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::document_start))
            .on_action(cx.listener(Self::document_end))
            .on_action(cx.listener(Self::select_document_start))
            .on_action(cx.listener(Self::select_document_end))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::line_start))
            .on_action(cx.listener(Self::line_end))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::indent))
            .on_action(cx.listener(Self::outdent))
    }

    pub(super) fn editor_column(&mut self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.dark;
        let mut described = self.describe_text(window);
        let scale = window.scale_factor();
        let prepaint = self.prepaint_lines.clone();
        let lines = self.doc().lines.clone();
        let source = list(lines, cx.processor(|this, ix, _window, cx| this.render_line(ix, cx))).size_full();
        // Without word wrap, long lines scroll sideways: the list is as wide as the longest line.
        let source = if self.settings.word_wrap {
            source.into_any_element()
        } else {
            let width = self.char_width * self.doc().longest_line as f32 + px(48.);
            div()
                .id("source-x")
                .size_full()
                .overflow_x_scroll()
                .track_scroll(&self.h_scroll)
                .child(div().h_full().min_w_full().w(width).child(source))
                .into_any_element()
        };
        div()
            .relative()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .bg(tone(dark, 0x18181b, 0xfafafa))
            .text_color(tone(dark, 0xfafafa, 0x18181b))
            .child(InputLayer { view: cx.entity(), focus: self.focus.clone() })
            .child(self.editor_actions(
                div()
                    .id("source")
                    .key_context("Editor")
                    .track_focus(&self.focus)
                    .role(gpui::Role::MultilineTextInput)
                    .aria_label("Editor")
                    .a11y_synthetic_children(move |builder| {
                        if let Some((lines, anchor, focus)) = &mut described {
                            // The lines were laid out during this prepaint; their boxes and per-character
                            // positions are in window coordinates, which is what a magnifier uses to
                            // follow the caret. AccessKit needs both to turn the caret into a rectangle.
                            let layouts: std::collections::HashMap<usize, TextLayout> =
                                prepaint.borrow().iter().map(|(line, layout)| (*line, layout.clone())).collect();
                            for line in lines.iter_mut() {
                                let Some(layout) = layouts.get(&line.index) else { continue };
                                let bounds = layout.bounds();
                                let left = f32::from(bounds.left());
                                line.bounds = Some(gpui::accesskit::Rect {
                                    x0: (left * scale) as f64,
                                    y0: (f32::from(bounds.top()) * scale) as f64,
                                    x1: (f32::from(bounds.right()) * scale) as f64,
                                    y1: (f32::from(bounds.bottom()) * scale) as f64,
                                });
                                let mut positions = Vec::with_capacity(line.text.len());
                                let mut widths = Vec::with_capacity(line.text.len());
                                let mut byte = 0;
                                for ch in line.text.chars() {
                                    let at = layout
                                        .position_for_index(byte)
                                        .map(|point| (f32::from(point.x) - left).max(0.0))
                                        .unwrap_or(0.0);
                                    let next = layout
                                        .position_for_index(byte + ch.len_utf8())
                                        .map(|point| (f32::from(point.x) - left).max(0.0))
                                        .unwrap_or(at);
                                    let width = if ch == '\n' { 0.0 } else { (next - at).max(0.0) };
                                    positions.push(at * scale);
                                    widths.push(width * scale);
                                    byte += ch.len_utf8();
                                }
                                line.positions = Some(positions);
                                line.widths = Some(widths);
                            }
                            a11y::describe(builder, lines, *anchor, *focus);
                        }
                    })
                    .size_full()
                    // The I-beam over the whole pane, including the space past the end of short lines.
                    .cursor_text()
                    // Monospace, as in the app's editor, so padded table columns line up.
                    .font_family(self.mono.clone())
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::click_source))
                    .child(source),
                cx,
            ))
    }

    pub(super) fn preview_column(&mut self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.dark;
        let blocks = self.doc().block_list.clone();
        let focused_ring = chrome::ring(dark);
        if self.preview_link.is_some_and(|index| index >= self.preview_links().len()) {
            self.preview_link = None;
        }
        let selected = self.preview_link.and_then(|index| self.preview_links().get(index)).map(|(link, _)| link.clone());
        let preview_label = match &selected {
            Some(link) => SharedString::from(format!("Preview, link {link}")),
            None => SharedString::from("Preview"),
        };
        let described = self.preview_described(window);
        self.preview_keys(
            div()
                .id("preview")
                .key_context("Preview")
                .track_focus(&self.preview_focus)
                .role(gpui::Role::Document)
                .aria_label(preview_label)
                .a11y_synthetic_children(move |builder| {
                    if let Some(lines) = &described {
                        a11y::describe_document(builder, lines);
                    }
                })
                .border_1()
                .border_color(gpui::transparent_black())
                .focus_visible(move |style| style.border_color(focused_ring)),
            cx,
        )
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .bg(tone(dark, 0x09090b, 0xffffff))
            .text_color(tone(dark, 0xfafafa, 0x18181b))
            .child(
                list(
                    blocks,
                    cx.processor(move |this, ix, window, cx| {
                        let folder = Path::new(&this.doc().path).parent().map(Path::to_path_buf);
                        let look = Look {
                            font_size: this.settings.font_size,
                            dark: this.dark,
                            mono: this.mono.clone(),
                            folder,
                            svg: cx.svg_renderer(),
                            selected_link: selected.clone(),
                        };
                        let item = match this.doc().blocks.get(ix) {
                            Some(block) => preview_view::preview_item(block, ix, &look, cx),
                            None => div().into_any_element(),
                        };
                        // A formula over this frame's layout budget is drawn in the next one.
                        if math::take_deferred() {
                            window.request_animation_frame();
                        }
                        item
                    }),
                )
                .size_full(),
            )
    }
}


