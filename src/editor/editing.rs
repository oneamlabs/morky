//! Text editing, the caret and the selection: everything that changes the buffer or moves the
//! caret, plus the parse scheduling that follows an edit.
//!
//! Every change goes through `edit`, so the buffer, its undo history, the source line list, the
//! preview and the find bar's matches stay in step.

use super::*;

impl Prototype {
    /// Every change to the text goes through here: the buffer, its undo history, the source list and the preview.
    pub(super) fn edit(&mut self, range: Range<usize>, text: &str, kind: EditKind, cx: &mut Context<Self>) {
        self.tab_leaves = false;
        if range.is_empty() && text.is_empty() {
            return;
        }
        let doc = self.doc_mut();
        let before = doc.selected.clone();
        let (len_before, version_before) = (doc.buffer.len(), doc.buffer.version());
        let (caret, splice) = doc.buffer.edit(range, text, kind, before);
        doc.splice_lines(splice);
        doc.selected = caret..caret;
        doc.selection_reversed = false;
        doc.marked = None;
        self.update_matches_after_edit(splice, len_before, version_before);
        self.after_change(cx);
    }

    /// With the find bar open, an edit re-searches only the lines it replaced; searching the whole
    /// document on every keystroke cost the main thread time in proportion to its size.
    fn update_matches_after_edit(&mut self, splice: LineSplice, len_before: usize, version_before: u64) {
        let Some(find) = self.find.as_mut() else { return };
        let Some((id, version, query)) = find.searched.as_mut() else { return };
        let buffer = &self.tabs.active_doc().buffer;
        // A query with a line break could match across the edited lines; search it all again.
        if *id != self.tabs.active_doc().id || *version != version_before || query.contains('\n') {
            return;
        }
        let new_lines = buffer.line_start(splice.first)..buffer.line_start(splice.first + splice.new);
        let old_end = new_lines.end + len_before - buffer.len();
        let fresh = buffer::find_all(&buffer.slice(new_lines.clone()), query)
            .into_iter()
            .map(|found| found.start + new_lines.start..found.end + new_lines.start)
            .collect();
        buffer::splice_matches(&mut find.matches, new_lines.start..old_end, new_lines.end, fresh);
        *version = buffer.version();
    }

    pub(super) fn after_change(&mut self, cx: &mut Context<Self>) {
        self.typed_at = Some(Instant::now());
        self.doc_mut().edited_at = self.typed_at;
        let (id, pause) = (self.doc().id, parse_pause(self.doc().last_parse_ms));
        if let Some(views) = self.views.get(&id) {
            views.preview.update(cx, |pane, cx| pane.clear_selection(cx));
        }
        self.schedule_parse(id, pause, cx);
        self.schedule_auto_save(id, cx);
        cx.notify();
    }

    /// Parses a snapshot of the text on a background thread after `pause`. Each edit replaces the
    /// task, which cancels the previous one, so only the text after the last keystroke is parsed.
    pub(super) fn schedule_parse(&mut self, id: u64, pause: Duration, cx: &mut Context<Self>) {
        let Some(doc) = self.tabs.iter_mut().find(|doc| doc.id == id) else { return };
        let rope = doc.buffer.snapshot();
        let version = doc.buffer.version();
        let flavor = preview::Flavor { typographer: self.settings.typographer };
        let base = doc.blocks.clone();
        doc.parse_task = Some(cx.spawn(async move |this, cx| {
            if !pause.is_zero() {
                cx.background_executor().timer(pause).await;
            }
            let started = Instant::now();
            // Both panes from one snapshot: the preview's blocks and the editor's colours. Code in
            // fences is highlighted once; the second call finds it in the tree-sitter cache.
            let parsed = cx
                .background_executor()
                .spawn(async move {
                    let text = rope.to_string();
                    let longest = text.lines().map(|line| line.chars().count()).max().unwrap_or(0);
                    let blocks = preview::blocks_with(&text, flavor);
                    let changed = preview_view::changed_blocks(&base, &blocks);
                    let images = ImagesInUse::of(&blocks);
                    let (links, headings) = (links_of(&blocks), headings_of(&blocks));
                    Parsed { blocks, spans: syntax::source_spans(&text), longest, counts: Counts::of(&text), base, changed, images, links, headings }
                })
                .await;
            let parse_ms = started.elapsed().as_secs_f64() * 1000.0;
            let _ = this.update(cx, |this, cx| this.apply_parse(id, version, parsed, parse_ms, cx));
        }));
    }

    /// Replaces only the blocks that changed, so the preview keeps its measured heights and scroll position.
    fn apply_parse(&mut self, id: u64, version: u64, parsed: Parsed, parse_ms: f64, cx: &mut Context<Self>) {
        let applying = Instant::now();
        let Parsed { blocks, spans, longest, counts, base, changed, images, links, headings } = parsed;
        let views = self.views.get(&id).map(|views| (views.preview.clone(), views.outline.clone()));
        let Some(doc) = self.tabs.iter_mut().find(|doc| doc.id == id) else { return };
        if doc.buffer.version() != version {
            return;
        }
        doc.last_parse_ms = parse_ms;
        doc.counts = counts;
        // From the last keystroke to the preview showing it: the pause, the parse and this update.
        if let Some(edited_at) = doc.edited_at.take() {
            let latency = edited_at.elapsed().as_secs_f64() * 1000.0;
            perf::log(&format!("preview_ms={latency:.0} parse_ms={parse_ms:.1}"));
        }
        // Only one parse per document is in flight (a new one replaces the task), so the blocks it
        // compared against are still the shown ones; if not, compare here.
        let (changed, count) = if Arc::ptr_eq(&base, &doc.blocks) { changed } else { preview_view::changed_blocks(&doc.blocks, &blocks) };
        let blocks = Arc::new(blocks);
        let preview_links = links;
        let preview_headings = headings.clone();
        let old_blocks = std::mem::replace(&mut doc.blocks, blocks.clone());
        let old_spans = std::mem::replace(&mut doc.spans, spans);
        doc.images = Arc::new(images);
        doc.headings = headings;
        doc.longest_line = longest;
        doc.synced_source_line = None;
        if let Some((preview, outline)) = views {
            preview.update(cx, |pane, cx| pane.apply(blocks, preview_links, changed, count, cx));
            outline.update(cx, |pane, cx| pane.set_headings(preview_headings, cx));
        }
        // Freeing a large document's old blocks and colours is itself tens of milliseconds.
        cx.background_spawn(async move { drop((old_blocks, old_spans, base)) }).detach();
        self.release_unused_images(cx);
        // The main thread's part of a parse: splicing the list and freeing images.
        let ms = applying.elapsed().as_secs_f64() * 1000.0;
        if ms > 50.0 {
            perf::log(&format!("stall_ms={ms:.0} apply"));
        }
        cx.notify();
    }

    /// Frees the bitmaps of formulas and diagrams that no open document shows any more at the
    /// current font size and theme: every edit of one makes a new bitmap, and GPUI keeps each until
    /// it is dropped.
    pub(super) fn release_unused_images(&self, cx: &mut App) {
        let images = self.tabs.iter().map(|doc| doc.images.as_ref());
        let mut diagrams_in_use = HashSet::new();
        let mut formulas_in_use = HashSet::new();
        for found in images {
            diagrams_in_use.extend(found.diagrams.iter().map(String::as_str));
            formulas_in_use.extend(found.formulas.iter().map(|(tex, display)| (tex.as_str(), *display)));
        }
        diagrams::release_unused(&diagrams_in_use, self.dark, cx);
        math::release_unused(&formulas_in_use, self.settings.font_size, self.dark, cx);
    }

    pub(super) fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        let doc = self.doc();
        if !doc.selected.is_empty() {
            let range = doc.selected.clone();
            self.edit(range, "", EditKind::Other, cx);
            return;
        }
        let cursor = doc.cursor();
        let range = doc.buffer.previous_grapheme(cursor)..cursor;
        self.edit(range, "", EditKind::Deleting, cx);
    }

    pub(super) fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        let doc = self.doc();
        if !doc.selected.is_empty() {
            let range = doc.selected.clone();
            self.edit(range, "", EditKind::Other, cx);
            return;
        }
        let cursor = doc.cursor();
        let range = cursor..doc.buffer.next_grapheme(cursor);
        self.edit(range, "", EditKind::Deleting, cx);
    }

    /// In a list item, Enter starts the next item (`lists.rs`); elsewhere it breaks the line.
    pub(super) fn enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        let range = self.doc().selected.clone();
        if let Some(list) = self.list_enter(&range) {
            self.edit(list.range, &list.text, EditKind::Typing, cx);
            self.doc_mut().selected = list.caret..list.caret;
            return;
        }
        self.edit(range, "\n", EditKind::Typing, cx);
    }

    /// The list continuation for Enter with the caret at `range`, unless text is selected or the
    /// caret is in a code, math or diagram block, where `- ` and `1. ` are content, not lists.
    fn list_enter(&self, range: &Range<usize>) -> Option<lists::ListEnter> {
        if !range.is_empty() {
            return None;
        }
        let doc = self.doc();
        let line = doc.buffer.line_of(range.start) + 1;
        if preview::in_code_block(&doc.blocks, line) {
            return None;
        }
        lists::on_enter(&doc.buffer, range.start)
    }

    pub(super) fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else { return };
        let range = self.doc().selected.clone();
        // The buffer is LF only, like the app's editor; files::write restores the file's line ending.
        self.edit(range, &text.replace("\r\n", "\n"), EditKind::Other, cx);
    }

    pub(super) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let doc = self.doc();
        if doc.selected.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(doc.buffer.slice(doc.selected.clone())));
    }

    pub(super) fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.doc().selected.is_empty() {
            return;
        }
        self.copy(&Copy, window, cx);
        let range = self.doc().selected.clone();
        self.edit(range, "", EditKind::Other, cx);
    }

    pub(super) fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        let doc = self.doc_mut();
        let Some((selection, splices)) = doc.buffer.undo() else { return };
        self.restore(selection, splices, cx);
    }

    pub(super) fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        let doc = self.doc_mut();
        let Some((selection, splices)) = doc.buffer.redo() else { return };
        self.restore(selection, splices, cx);
    }

    fn restore(&mut self, selection: Range<usize>, splices: Vec<LineSplice>, cx: &mut Context<Self>) {
        let doc = self.doc_mut();
        for splice in splices {
            doc.splice_lines(splice);
        }
        doc.selected = doc.buffer.boundary(selection.start)..doc.buffer.boundary(selection.end);
        doc.selection_reversed = false;
        doc.marked = None;
        self.after_change(cx);
    }

    // ----- Caret and selection -----

    pub(super) fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let doc = self.doc();
        let target = if doc.selected.is_empty() { doc.buffer.previous_grapheme(doc.cursor()) } else { doc.selected.start };
        self.move_to(target, cx);
    }

    pub(super) fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let doc = self.doc();
        let target = if doc.selected.is_empty() { doc.buffer.next_grapheme(doc.cursor()) } else { doc.selected.end };
        self.move_to(target, cx);
    }

    pub(super) fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.doc().buffer.vertical_target(self.doc().cursor(), -1);
        self.move_to(target, cx);
    }

    pub(super) fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.doc().buffer.vertical_target(self.doc().cursor(), 1);
        self.move_to(target, cx);
    }

    pub(super) fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.doc().buffer.previous_grapheme(self.doc().cursor());
        self.select_to(target, cx);
    }

    pub(super) fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.doc().buffer.next_grapheme(self.doc().cursor());
        self.select_to(target, cx);
    }

    pub(super) fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.doc().buffer.vertical_target(self.doc().cursor(), -1);
        self.select_to(target, cx);
    }

    pub(super) fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.doc().buffer.vertical_target(self.doc().cursor(), 1);
        self.select_to(target, cx);
    }

    pub(super) fn select_line_start(&mut self, _: &SelectLineStart, _: &mut Window, cx: &mut Context<Self>) {
        let start = self.home_target();
        self.select_to(start, cx);
    }

    pub(super) fn select_line_end(&mut self, _: &SelectLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let end = self.end_target();
        self.select_to(end, cx);
    }

    pub(super) fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.doc().buffer.previous_word(self.doc().cursor());
        self.move_to(target, cx);
    }

    pub(super) fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.doc().buffer.next_word(self.doc().cursor());
        self.move_to(target, cx);
    }

    pub(super) fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.doc().buffer.previous_word(self.doc().cursor());
        self.select_to(target, cx);
    }

    pub(super) fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let target = self.doc().buffer.next_word(self.doc().cursor());
        self.select_to(target, cx);
    }

    pub(super) fn document_start(&mut self, _: &DocumentStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    pub(super) fn document_end(&mut self, _: &DocumentEnd, _: &mut Window, cx: &mut Context<Self>) {
        let end = self.doc().buffer.len();
        self.move_to(end, cx);
    }

    pub(super) fn select_document_start(&mut self, _: &SelectDocumentStart, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    pub(super) fn select_document_end(&mut self, _: &SelectDocumentEnd, _: &mut Window, cx: &mut Context<Self>) {
        let end = self.doc().buffer.len();
        self.select_to(end, cx);
    }

    /// Ctrl+Backspace and Ctrl+Delete remove a word, or the selection if there is one.
    pub(super) fn delete_word_left(&mut self, _: &DeleteWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        let doc = self.doc();
        let range = if doc.selected.is_empty() { doc.buffer.previous_word(doc.cursor())..doc.cursor() } else { doc.selected.clone() };
        self.edit(range, "", EditKind::Other, cx);
    }

    pub(super) fn delete_word_right(&mut self, _: &DeleteWordRight, _: &mut Window, cx: &mut Context<Self>) {
        let doc = self.doc();
        let range = if doc.selected.is_empty() { doc.cursor()..doc.buffer.next_word(doc.cursor()) } else { doc.selected.clone() };
        self.edit(range, "", EditKind::Other, cx);
    }

    /// Tab indents the lines the selection touches, as CodeMirror's `indentMore` does; after Esc it
    /// moves the focus on instead.
    pub(super) fn indent(&mut self, _: &Indent, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.tab_leaves) {
            window.focus_next(cx);
            return;
        }
        self.reindent(|line| Some(format!("{INDENT}{line}")), cx);
    }

    /// Shift+Tab removes one indent unit (or a tab) from each line, as `indentLess` does.
    pub(super) fn outdent(&mut self, _: &Outdent, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.tab_leaves) {
            window.focus_prev(cx);
            return;
        }
        self.reindent(
            |line| {
                let removed = line.strip_prefix(INDENT).or_else(|| line.strip_prefix('\t')).or_else(|| line.strip_prefix(' '))?;
                Some(removed.to_string())
            },
            cx,
        );
    }

    /// Rewrites each line the selection touches with `change` (None leaves it), as one undo step,
    /// and keeps the selection on the same text.
    fn reindent(&mut self, change: impl Fn(&str) -> Option<String>, cx: &mut Context<Self>) {
        let doc = self.doc();
        let selected = doc.selected.clone();
        let first = doc.buffer.line_of(selected.start);
        let mut last = doc.buffer.line_of(selected.end);
        // A selection that ends at the start of a line does not touch that line.
        if last > first && doc.buffer.line_start(last) == selected.end {
            last -= 1;
        }
        let span = doc.buffer.line_start(first)..doc.buffer.line_range(last).end;
        let old = doc.buffer.slice(span.clone());
        let mut new = String::with_capacity(old.len() + (last - first + 1) * INDENT.len());
        let (mut shift_start, mut shift_end) = (0isize, 0isize);
        let mut line_start = span.start;
        for (index, line) in old.split('\n').enumerate() {
            if index > 0 {
                new.push('\n');
            }
            let changed = change(line).unwrap_or_else(|| line.to_string());
            let delta = changed.len() as isize - line.len() as isize;
            if line_start <= selected.start {
                shift_start += delta.max(line_start as isize - selected.start as isize);
            }
            if line_start <= selected.end {
                shift_end += delta.max(line_start as isize - selected.end as isize);
            }
            new.push_str(&changed);
            line_start += line.len() + 1;
        }
        if new == old {
            return;
        }
        let reversed = doc.selection_reversed;
        self.edit(span, &new, EditKind::Other, cx);
        let doc = self.doc_mut();
        let start = (selected.start as isize + shift_start).max(0) as usize;
        let end = (selected.end as isize + shift_end).max(start as isize) as usize;
        doc.selected = doc.buffer.boundary(start)..doc.buffer.boundary(end);
        doc.selection_reversed = reversed;
    }

    pub(super) fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let doc = self.doc_mut();
        doc.selected = 0..doc.buffer.len();
        doc.selection_reversed = false;
        doc.buffer.break_group();
        cx.notify();
    }

    pub(super) fn line_start(&mut self, _: &LineStart, _: &mut Window, cx: &mut Context<Self>) {
        let start = self.home_target();
        self.move_to(start, cx);
    }

    pub(super) fn line_end(&mut self, _: &LineEnd, _: &mut Window, cx: &mut Context<Self>) {
        let end = self.end_target();
        self.move_to(end, cx);
    }

    /// A caret move also ends the current typing group, so the next keystroke is a new undo step.
    pub(super) fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.tab_leaves = false;
        let doc = self.doc_mut();
        let offset = doc.buffer.boundary(offset);
        doc.selected = offset..offset;
        doc.selection_reversed = false;
        doc.buffer.break_group();
        cx.notify();
    }

    pub(super) fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.tab_leaves = false;
        let doc = self.doc_mut();
        let offset = doc.buffer.boundary(offset);
        if doc.selection_reversed {
            doc.selected.start = offset;
        } else {
            doc.selected.end = offset;
        }
        if doc.selected.end < doc.selected.start {
            doc.selection_reversed = !doc.selection_reversed;
            doc.selected = doc.selected.end..doc.selected.start;
        }
        doc.buffer.break_group();
        cx.notify();
    }

    pub(super) fn select(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let doc = self.doc_mut();
        doc.selected = doc.buffer.boundary(range.start)..doc.buffer.boundary(range.end);
        doc.selection_reversed = false;
        doc.marked = None;
        doc.buffer.break_group();
        cx.notify();
    }

    /// The character under `position` on source line `line`. GPUI's layout of that line maps the
    /// point, wrapped or not; the line is on screen, so it has been laid out.
    pub(super) fn offset_at(&self, line: usize, layout: &TextLayout, position: Point<Pixels>) -> usize {
        let range = self.doc().buffer.line_range(line);
        let (Ok(index) | Err(index)) = layout.index_for_position(position);
        range.start + index.min(range.len())
    }

    /// A click on a source line: places the caret, or extends the selection with Shift. A double
    /// click selects the word and a triple click the line. Dragging then extends from here.
    pub(super) fn click_line(&mut self, line: usize, layout: &TextLayout, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.open_menu = None;
        let offset = self.offset_at(line, layout, event.position);
        self.drag.start();
        match event.click_count {
            2 => {
                let word = self.doc().buffer.word_at(offset);
                self.select(word, cx);
            }
            count if count >= 3 => {
                let doc = self.doc();
                let line = doc.buffer.line_of(offset);
                let range = doc.buffer.line_start(line)..doc.buffer.line_start(line + 1);
                self.select(range, cx);
            }
            _ if event.modifiers.shift => self.select_to(offset, cx),
            _ => self.move_to(offset, cx),
        }
    }

    /// A click below the last line puts the caret at the end.
    pub(super) fn click_source(&mut self, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.open_menu = None;
        let end = self.doc().buffer.len();
        self.move_to(end, cx);
    }

    pub(super) fn jump_to_line(&mut self, line: usize, cx: &mut Context<Self>) {
        let offset = self.doc().buffer.line_start(line.saturating_sub(1));
        self.move_to(offset, cx);
    }
}
