//! Drawing the window's panes: each source line as a virtualised list item (with its syntax colours,
//! find matches, selection, script text and caret), the preview pane, and the caret and accessibility
//! helpers that need the painted text layouts.

use super::*;

impl Prototype {
    /// The caret stays visible, and the preview shows the block for the top visible source line.
    pub(super) fn sync_scroll(&mut self, cx: &mut Context<Self>) {
        let show_editor = self.settings.show_editor;
        let show_preview = self.settings.preview_visible();
        let (id, block) = {
            let doc = self.doc_mut();
            let cursor_line = doc.buffer.line_of(doc.cursor());
            if doc.followed_cursor_line != Some(cursor_line) {
                doc.followed_cursor_line = Some(cursor_line);
                if show_editor {
                    doc.lines.scroll_to_reveal_item(cursor_line);
                }
            }
            if !show_editor || !show_preview || doc.blocks.is_empty() {
                return;
            }
            let top = doc.lines.logical_scroll_top().item_ix;
            if doc.synced_source_line == Some(top) {
                return;
            }
            doc.synced_source_line = Some(top);
            (doc.id, preview::block_at_line(&doc.blocks, top + 1))
        };
        if let Some(views) = self.views.get(&id) {
            views.preview.update(cx, |pane, cx| pane.scroll_to_block(block, cx));
        }
    }

    /// With word wrap off, keeps the caret in view sideways after it moves. The caret's bounds are
    /// only known once it is painted, so the scroll happens on the frame after the move; scrolling by
    /// hand is left alone until the caret moves again. Returns whether the offset changed.
    pub(super) fn follow_caret_sideways(&mut self) -> bool {
        let doc = self.doc();
        let cursor = (doc.id, doc.cursor());
        if self.last_cursor != Some(cursor) {
            self.last_cursor = Some(cursor);
            self.follow_x = true;
        }
        if !self.follow_x {
            return false;
        }
        let Some((_, caret)) = self.caret_painted.filter(|(painted, _)| *painted == cursor.1) else {
            return false;
        };
        self.follow_x = false;
        let view = self.h_scroll.bounds();
        let margin = px(32.);
        let mut offset = self.h_scroll.offset();
        if caret.right() > view.right() - margin {
            offset.x -= caret.right() - (view.right() - margin);
        } else if caret.left() < view.left() + margin {
            offset.x = (offset.x + (view.left() + margin - caret.left())).min(px(0.));
        } else {
            return false;
        }
        self.h_scroll.set_offset(offset);
        true
    }

    /// One source line, laid out only when the list needs it. Syntax colours, find matches and the
    /// selection are layered highlights on the line's own `StyledText`, whose layout also maps clicks
    /// and places the caret.
    pub(super) fn render_line(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
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
            let found_style = HighlightStyle { background_color: Some(color(dark, Color::FindMatch).into()), ..HighlightStyle::default() };
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
                let selection_style = HighlightStyle { background_color: Some(color(dark, Color::Selection).into()), ..HighlightStyle::default() };
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
                        let color = color(dark, Color::Caret);
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
    pub(super) fn describe_text(&self, window: &Window) -> Option<(Vec<a11y::Line>, a11y::Point, a11y::Point)> {
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
    pub(super) fn editor_actions<E: InteractiveElement>(&self, element: E, cx: &mut Context<Self>) -> E {
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

}


