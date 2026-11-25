//! Windows text input and IME for the source view.
//!
//! Text reaches a GPUI view only through its platform input handler, so typing, paste and an IME
//! composition arrive at `SourcePane`; every change is handed to the window's `edit`, which keeps
//! the undo grouping, the source line list and the preview in step.

use super::source_pane::SourcePane;
use super::*;

/// A byte offset for a UTF-16 offset in `text`, clamping past the end. Windows text input counts
/// UTF-16 units (a surrogate pair is two); the buffer counts bytes.
pub(super) fn utf16_to_byte(text: &str, utf16_offset: usize) -> usize {
    let mut utf16 = 0;
    for (index, ch) in text.char_indices() {
        if utf16 >= utf16_offset {
            return index;
        }
        utf16 += ch.len_utf16();
    }
    text.len()
}

impl Prototype {
    /// The buffer range an input event replaces: the range Windows named, the IME composition, or
    /// the selection.
    pub(super) fn edit_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        let doc = self.doc();
        if let Some(range) = range_utf16 {
            return doc.buffer.offset_of_utf16(range.start)..doc.buffer.offset_of_utf16(range.end);
        }
        doc.marked.clone().unwrap_or_else(|| doc.selected.clone())
    }
}

impl EntityInputHandler for SourcePane {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let app = self.app.upgrade()?;
        let app = app.read(cx);
        let buffer = &app.doc().buffer;
        let range = buffer.offset_of_utf16(range_utf16.start)..buffer.offset_of_utf16(range_utf16.end);
        *actual_range = Some(buffer.utf16_of(range.start)..buffer.utf16_of(range.end));
        Some(buffer.slice(range))
    }

    fn selected_text_range(&mut self, _ignore_disabled_input: bool, _window: &mut Window, cx: &mut Context<Self>) -> Option<UTF16Selection> {
        let app = self.app.upgrade()?;
        let app = app.read(cx);
        let doc = app.doc();
        Some(UTF16Selection {
            range: doc.buffer.utf16_of(doc.selected.start)..doc.buffer.utf16_of(doc.selected.end),
            reversed: doc.selection_reversed,
        })
    }

    fn marked_text_range(&self, _window: &mut Window, cx: &mut Context<Self>) -> Option<Range<usize>> {
        let app = self.app.upgrade()?;
        let app = app.read(cx);
        let doc = app.doc();
        doc.marked.as_ref().map(|range| doc.buffer.utf16_of(range.start)..doc.buffer.utf16_of(range.end))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app.upgrade() else { return };
        app.update(cx, |app, _| app.doc_mut().marked = None);
    }

    /// Typed text and the end of an IME composition. Typing at the caret groups into one undo step.
    fn replace_text_in_range(&mut self, range_utf16: Option<Range<usize>>, new_text: &str, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app.upgrade() else { return };
        let (range, kind) = {
            let app = app.read(cx);
            let range = app.edit_range(range_utf16);
            let kind = if range.is_empty() || app.doc().marked.as_ref() == Some(&range) { EditKind::Typing } else { EditKind::Other };
            (range, kind)
        };
        app.update(cx, |app, cx| app.edit(range, new_text, kind, cx));
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(app) = self.app.upgrade() else { return };
        app.update(cx, |app, cx| {
            let range = app.edit_range(range_utf16);
            let replaced_at = range.start;
            app.edit(range, new_text, EditKind::Typing, cx);
            let doc = app.doc_mut();
            let end = replaced_at + new_text.len();
            doc.marked = (!new_text.is_empty()).then_some(replaced_at..end);
            if let Some(selected) = new_selected_utf16 {
                let start = replaced_at + utf16_to_byte(new_text, selected.start);
                doc.selected = start..replaced_at + utf16_to_byte(new_text, selected.end);
            }
        });
    }

    /// The IME asks where to put its candidate window. The caret painted last frame is the
    /// composition point; its bounds already include scrolling, padding and the column.
    fn bounds_for_range(&mut self, _range_utf16: Range<usize>, element_bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let caret = self.app.upgrade().and_then(|app| app.read(cx).caret_bounds);
        Some(caret.unwrap_or_else(|| Bounds::new(element_bounds.origin, size(px(1.5), window.line_height()))))
    }

    /// The character under a point, in UTF-16 units, for IME reconversion by mouse. Only lines
    /// painted in the last frame can answer. (GPUI's Windows backend does not yet ask: it handles
    /// no `WM_IME_REQUEST`, so Windows' reconversion key cannot reach any GPUI view, at 9a8ab54
    /// either.)
    fn character_index_for_point(&mut self, position: Point<Pixels>, _window: &mut Window, cx: &mut Context<Self>) -> Option<usize> {
        let app = self.app.upgrade()?;
        let app = app.read(cx);
        let line = app.visible_line_at(position.y)?;
        let layout = app.painted_layout(line)?;
        let offset = app.offset_at(line, layout, position);
        Some(app.doc().buffer.utf16_of(offset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows text input and GPUI speak UTF-16; the buffer counts bytes.
    #[test]
    fn utf16_offsets_map_to_byte_offsets() {
        assert_eq!(utf16_to_byte("abc", 0), 0);
        assert_eq!(utf16_to_byte("abc", 2), 2);
        // "é" is one UTF-16 unit but two bytes.
        assert_eq!(utf16_to_byte("aéb", 2), 1 + 2);
        // An astral character is a surrogate pair: two units, four bytes.
        assert_eq!(utf16_to_byte("a😀b", 3), 1 + 4);
        // Past the end clamps to the length.
        assert_eq!(utf16_to_byte("abc", 99), 3);
    }
}
