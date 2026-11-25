//! Drag selection that leaves the source pane, as CodeMirror does: beside the pane it extends to the
//! start or end of the line level with the pointer; above or below it the pane keeps scrolling
//! towards the pointer, faster the further away it is, until the button comes up.

use super::*;

impl Prototype {
    /// While the left button is held after a click in the source, the selection follows the pointer.
    pub(super) fn drag_line(&mut self, line: usize, layout: &TextLayout, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if !self.dragging {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.dragging = false;
            return;
        }
        let offset = self.offset_at(line, layout, event.position);
        if offset != self.doc().cursor() {
            self.select_to(offset, cx);
        }
    }

    /// A drag selection that leaves the source pane keeps going, as in CodeMirror: beside the pane it
    /// extends to the start or end of the line level with the pointer; above or below it the pane
    /// scrolls towards the pointer, faster the further away it is, until the button comes up.
    pub(super) fn drag_anywhere(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if !self.dragging || !self.settings.show_editor {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.dragging = false;
            self.drag_outside = None;
            return;
        }
        let pane = self.doc().lines.viewport_bounds();
        let position = event.position;
        if position.y >= pane.top() && position.y < pane.bottom() {
            self.drag_outside = None;
            let beside = position.x < pane.left() || position.x >= pane.right();
            if let Some(line) = beside.then(|| self.visible_line_at(position.y)).flatten() {
                let range = self.doc().buffer.line_range(line);
                self.select_to(if position.x < pane.left() { range.start } else { range.end }, cx);
            }
            return;
        }
        let started = self.drag_outside.is_none();
        self.drag_outside = Some(position);
        if started {
            self.drag_scroll = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_millis(16)).await;
                    if !this.update(cx, |this, cx| this.drag_scroll_step(cx)).unwrap_or(false) {
                        break;
                    }
                }
            }));
        }
    }

    /// One frame of scrolling while a drag is above or below the pane. Returns whether to go on.
    fn drag_scroll_step(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(pointer) = self.drag_outside.filter(|_| self.dragging) else { return false };
        let doc = self.doc();
        let pane = doc.lines.viewport_bounds();
        let up = pointer.y < pane.top();
        let distance = if up { pane.top() - pointer.y } else { pointer.y - pane.bottom() };
        let step = (distance * 0.5).clamp(px(4.), px(80.));
        doc.lines.scroll_by(if up { -step } else { step });
        let target = if up {
            doc.buffer.line_start(doc.lines.logical_scroll_top().item_ix)
        } else {
            let last = self.visible_line_at(pane.bottom() - px(1.)).unwrap_or(doc.buffer.line_count().saturating_sub(1));
            doc.buffer.line_range(last).end
        };
        self.select_to(target, cx);
        // The caret is meant to be at the edge, so the follow-the-caret scroll must not undo the drag.
        let line = self.doc().buffer.line_of(self.doc().cursor());
        self.doc_mut().followed_cursor_line = Some(line);
        true
    }

    /// The source line drawn at height `y`, among the lines laid out in the last frame.
    pub(super) fn visible_line_at(&self, y: Pixels) -> Option<usize> {
        let lines = &self.doc().lines;
        let top = lines.logical_scroll_top().item_ix;
        (top..lines.item_count()).take(400).map_while(|line| lines.bounds_for_item(line).map(|bounds| (line, bounds))).find(|(_, bounds)| bounds.bottom() > y).map(|(line, _)| line)
    }
}
