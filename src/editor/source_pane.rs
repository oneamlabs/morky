//! The source editor as its own GPUI view: the editor shell, its focus, Windows text input/IME and
//! the editor's accessibility node.
//!
//! The buffer, selection, find matches and painted layouts stay on the window, which owns editing;
//! this view draws the shell and asks the window to build each visible line. That keeps the editing
//! rules in one place while the window no longer builds the editor column itself.

use super::*;

pub(super) struct SourcePane {
    pub(super) app: gpui::WeakEntity<Prototype>,
    focus: FocusHandle,
}

impl SourcePane {
    pub(super) fn new(app: gpui::WeakEntity<Prototype>, focus: FocusHandle, _cx: &mut Context<Self>) -> Self {
        Self { app, focus }
    }
}

impl Render for SourcePane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(app) = self.app.upgrade() else {
            return div().size_full().into_any_element();
        };
        let (lines, mono, word_wrap, longest_line, char_width, h_scroll, dark, mut described, prepaint) = {
            let app = app.read(cx);
            (
                app.doc().lines.clone(),
                app.mono.clone(),
                app.settings.word_wrap,
                app.doc().longest_line,
                app.char_width,
                app.h_scroll.clone(),
                app.dark,
                app.describe_text(window),
                app.prepaint_lines.clone(),
            )
        };
        let scale = window.scale_factor();
        let scroll_lines = lines.clone();
        let source = list(
            lines,
            cx.processor(|this: &mut SourcePane, ix, _window, cx| {
                let Some(app) = this.app.upgrade() else { return div().into_any_element() };
                app.update(cx, |app, cx| app.render_line(ix, cx))
            }),
        )
        .size_full();
        // Without word wrap, long lines scroll sideways: the list is as wide as the longest line.
        let source = if word_wrap {
            source.into_any_element()
        } else {
            let width = char_width * longest_line as f32 + px(48.);
            div()
                .id("source-x")
                .size_full()
                .overflow_x_scroll()
                .track_scroll(&h_scroll)
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
            .child(
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
                    .font_family(mono)
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, window, cx| {
                        let Some(app) = this.app.upgrade() else { return };
                        app.update(cx, |app, cx| app.click_source(event, window, cx));
                    }))
                    .child(source),
            )
            .child(scrollbar::indicator(&scroll_lines, dark))
            .when(!word_wrap, |pane| pane.child(scrollbar::horizontal_indicator(&h_scroll, dark)))
            .into_any_element()
    }
}
