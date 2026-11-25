//! The find and replace bar: two text fields, the matches they select in the active document, and
//! stepping and replacing through them.
//!
//! Only the lines an edit replaced are searched again (see `Prototype::update_matches_after_edit`);
//! searching a large document on every keystroke cost the main thread time in proportion to its size.

use super::*;

impl Prototype {
    pub(super) fn open_find(&mut self, show_replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.open_menu = None;
        if self.find.is_none() {
            let query = cx.new(|cx| TextField::new("Find", cx));
            let replacement = cx.new(|cx| TextField::new("Replace with", cx));
            let subscriptions = vec![
                cx.subscribe(&query, |this, _, event: &FieldEvent, cx| match event {
                    FieldEvent::Changed => this.search_from_caret(cx),
                    FieldEvent::Submitted => this.find_step(1, cx),
                }),
                cx.subscribe(&replacement, |this, _, event: &FieldEvent, cx| {
                    if let FieldEvent::Submitted = event {
                        this.replace_current(cx);
                    }
                }),
            ];
            self.find = Some(FindBar { query, replacement, show_replace, matches: Vec::new(), searched: None, _subscriptions: subscriptions });
        }
        let Some(find) = self.find.as_mut() else { return };
        find.show_replace |= show_replace;
        let query = find.query.clone();
        // A selection on one line becomes the query, as in most editors.
        let doc = self.doc();
        let selected = doc.buffer.slice(doc.selected.clone());
        if !selected.is_empty() && !selected.contains('\n') {
            query.update(cx, |field, cx| field.set_text(&selected, cx));
        }
        let handle = query.focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    pub(super) fn find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        self.open_find(false, window, cx);
    }

    pub(super) fn find_replace(&mut self, _: &FindReplace, window: &mut Window, cx: &mut Context<Self>) {
        self.open_find(true, window, cx);
    }

    pub(super) fn find_next(&mut self, _: &FindNext, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_none() {
            self.open_find(false, window, cx);
            return;
        }
        self.find_step(1, cx);
    }

    pub(super) fn find_previous(&mut self, _: &FindPrevious, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_none() {
            self.open_find(false, window, cx);
            return;
        }
        self.find_step(-1, cx);
    }

    /// Searches again only when the query, the document or its text changed.
    pub(super) fn refresh_matches(&mut self, cx: &App) {
        let Some(find) = self.find.as_mut() else { return };
        let doc = &self.tabs.active_doc();
        let query = find.query.read(cx).text();
        // This runs every frame; compare before copying the query.
        let current = find.searched.as_ref().is_some_and(|(id, version, searched)| *id == doc.id && *version == doc.buffer.version() && searched == query);
        if current {
            return;
        }
        find.matches = buffer::find_all(&doc.buffer.text(), query);
        find.searched = Some((doc.id, doc.buffer.version(), query.to_string()));
    }

    pub(super) fn matches(&self) -> &[Range<usize>] {
        self.find.as_ref().map(|find| find.matches.as_slice()).unwrap_or(&[])
    }

    /// While typing a query, the first match at or after the caret is selected.
    fn search_from_caret(&mut self, cx: &mut Context<Self>) {
        self.refresh_matches(cx);
        let from = self.doc().selected.start;
        let matches = self.matches();
        let Some(found) = matches.iter().find(|found| found.start >= from).or(matches.first()).cloned() else {
            cx.notify();
            return;
        };
        self.select(found, cx);
    }

    pub(super) fn find_step(&mut self, direction: isize, cx: &mut Context<Self>) {
        self.refresh_matches(cx);
        let selected = self.doc().selected.clone();
        let matches = self.matches();
        let found = if direction > 0 {
            matches.iter().find(|found| found.start >= selected.end && **found != selected).or(matches.first())
        } else {
            matches.iter().rev().find(|found| found.end <= selected.start).or(matches.last())
        };
        let Some(found) = found.cloned() else { return };
        self.select(found, cx);
    }

    fn replacement_text(&self, cx: &App) -> String {
        self.find.as_ref().map(|find| find.replacement.read(cx).text().to_string()).unwrap_or_default()
    }

    /// Replaces the selected match, then selects the next one.
    pub(super) fn replace_current(&mut self, cx: &mut Context<Self>) {
        self.refresh_matches(cx);
        let selected = self.doc().selected.clone();
        if self.matches().contains(&selected) {
            let replacement = self.replacement_text(cx);
            self.edit(selected, &replacement, EditKind::Other, cx);
            self.refresh_matches(cx);
        }
        self.find_step(1, cx);
    }

    /// One edit from the first match to the last, so Replace all is a single undo step.
    pub(super) fn replace_all(&mut self, cx: &mut Context<Self>) {
        self.refresh_matches(cx);
        let matches = self.matches().to_vec();
        let (Some(first), Some(last)) = (matches.first(), matches.last()) else { return };
        let span = first.start..last.end;
        let replacement = self.replacement_text(cx);
        let text = self.doc().buffer.slice(span.clone());
        let mut replaced = String::with_capacity(text.len());
        let mut copied = span.start;
        for found in &matches {
            replaced.push_str(&text[copied - span.start..found.start - span.start]);
            replaced.push_str(&replacement);
            copied = found.end;
        }
        self.edit(span, &replaced, EditKind::Other, cx);
        self.status = Some(format!("Replaced {} matches", matches.len()));
    }
}
