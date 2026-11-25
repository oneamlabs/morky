//! Drag selection that leaves the source pane, as CodeMirror does: beside the pane it extends to the
//! start or end of the line level with the pointer; above or below it the pane keeps scrolling
//! towards the pointer, faster the further away it is, until the button comes up.
//!
//! The state is one flag plus the pointer position outside the pane, so it has a unit test for the
//! part that is easy to get wrong: a drag ends when the button is no longer down.

use super::*;

/// Whether a drag selection is in progress, and where the pointer is while it is outside the pane.
#[derive(Default)]
pub(super) struct Drag {
    /// The left button went down in the source and has not come up.
    active: bool,
    /// The pointer's position while it is outside the pane; `None` inside it or when not dragging.
    outside: Option<Point<Pixels>>,
    /// The task scrolling towards `outside`, replaced when a new drag starts.
    scroll: Option<Task<()>>,
}

impl Drag {
    pub(super) fn active(&self) -> bool {
        self.active
    }

    /// The button went down in the source: moves now extend the selection.
    pub(super) fn start(&mut self) {
        self.active = true;
    }

    /// The button came up, or a move arrived with it already up: the drag is over.
    pub(super) fn end(&mut self) {
        self.active = false;
        self.outside = None;
    }

    pub(super) fn outside(&self) -> Option<Point<Pixels>> {
        self.outside
    }

    /// Records the pointer outside the pane. Returns whether this started a new scroll task: only
    /// the first move outside begins one, later ones just move the target.
    pub(super) fn move_outside(&mut self, position: Point<Pixels>) -> bool {
        let started = self.outside.is_none();
        self.outside = Some(position);
        started
    }

    pub(super) fn leave_pane(&mut self) {
        self.outside = None;
    }

    pub(super) fn set_scroll(&mut self, task: Task<()>) {
        self.scroll = Some(task);
    }
}

impl Prototype {
    /// While the left button is held after a click in the source, the selection follows the pointer.
    pub(super) fn drag_line(&mut self, line: usize, layout: &TextLayout, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if !self.drag.active() {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag.end();
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
        if !self.drag.active() || !self.settings.show_editor {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.drag.end();
            return;
        }
        let pane = self.doc().lines.viewport_bounds();
        let position = event.position;
        if position.y >= pane.top() && position.y < pane.bottom() {
            self.drag.leave_pane();
            let beside = position.x < pane.left() || position.x >= pane.right();
            if let Some(line) = beside.then(|| self.visible_line_at(position.y)).flatten() {
                let range = self.doc().buffer.line_range(line);
                self.select_to(if position.x < pane.left() { range.start } else { range.end }, cx);
            }
            return;
        }
        if !self.drag.move_outside(position) {
            return;
        }
        let task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(16)).await;
                if !this.update(cx, |this, cx| this.drag_scroll_step(cx)).unwrap_or(false) {
                    break;
                }
            }
        });
        self.drag.set_scroll(task);
    }

    /// One frame of scrolling while a drag is above or below the pane. Returns whether to go on.
    fn drag_scroll_step(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(pointer) = self.drag.outside().filter(|_| self.drag.active()) else { return false };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_is_over_once_the_button_comes_up() {
        let mut drag = Drag::default();
        assert!(!drag.active());
        drag.start();
        assert!(drag.active());
        drag.end();
        assert!(!drag.active());
        assert!(drag.outside().is_none());
    }

    /// Only the first move outside the pane starts a scroll task; later ones move its target.
    #[test]
    fn only_the_first_move_outside_starts_scrolling() {
        let mut drag = Drag::default();
        drag.start();
        assert!(drag.move_outside(point(px(0.), px(0.))));
        assert!(!drag.move_outside(point(px(0.), px(10.))));
        assert_eq!(drag.outside(), Some(point(px(0.), px(10.))));
    }

    #[test]
    fn coming_back_inside_the_pane_stops_scrolling() {
        let mut drag = Drag::default();
        drag.start();
        drag.move_outside(point(px(0.), px(0.)));
        drag.leave_pane();
        assert!(drag.outside().is_none());
        // Re-entering outside starts a fresh scroll task.
        assert!(drag.move_outside(point(px(0.), px(5.))));
    }
}
