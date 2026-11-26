//! One document's outline as its own GPUI view.
//!
//! The list is virtualised, so its cost follows the screen rather than the heading count, and the
//! pane scrolls the heading its arrow keys select into view. The parent places this view cached,
//! so caret moves that stay within one heading do not redraw it.

use super::document::Heading;
use super::*;
use gpui::{ClickEvent, Role};

pub(super) struct OutlinePane {
    headings: Vec<Heading>,
    /// One item per heading, with this tab's scroll position.
    list: ListState,
    /// The heading the arrow keys have selected, which Enter opens.
    cursor: Option<usize>,
    /// The heading the caret is under, as the last parse and caret position say.
    current: Option<usize>,
    dark: bool,
    focus: FocusHandle,
    /// Focus changes only affect the ring, so they are observed to bust the parent's cache.
    _focus_in: Option<Subscription>,
    _focus_out: Option<Subscription>,
    /// A cached pane renders once before its list has bounds; one nudge brings the scrollbar in.
    layout_nudged: bool,
    app: gpui::WeakEntity<Prototype>,
}

impl OutlinePane {
    pub(super) fn new(dark: bool, app: gpui::WeakEntity<Prototype>, cx: &mut Context<Self>) -> Self {
        let list = ListState::new(0, ListAlignment::Top, px(OVERDRAW));
        list.set_scroll_handler(cx.listener(|_, _: &ListScrollEvent, _, cx| cx.notify()));
        Self {
            headings: Vec::new(),
            list,
            cursor: None,
            current: None,
            dark,
            focus: cx.focus_handle().tab_stop(true),
            _focus_in: None,
            _focus_out: None,
            layout_nudged: false,
            app,
        }
    }

    pub(super) fn set_headings(&mut self, headings: Vec<Heading>, cx: &mut Context<Self>) {
        if self.headings == headings {
            return;
        }
        let old = self.headings.len();
        self.headings = headings;
        self.list.splice(0..old, self.headings.len());
        self.cursor = self.cursor.filter(|index| *index < self.headings.len());
        cx.notify();
    }

    pub(super) fn set_current(&mut self, current: Option<usize>, cx: &mut Context<Self>) {
        if self.current == current {
            return;
        }
        self.current = current;
        // Keep the heading the caret is under visible; the caret moving is the only trigger, so a
        // reader scrolling the outline by hand is not yanked back.
        if let Some(index) = current {
            self.list.scroll_to_reveal_item(index);
        }
        cx.notify();
    }

    pub(super) fn set_dark(&mut self, dark: bool, cx: &mut Context<Self>) {
        if self.dark == dark {
            return;
        }
        self.dark = dark;
        cx.notify();
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.headings.len();
        if count == 0 {
            return;
        }
        let from = self.cursor.or(self.current).unwrap_or(0);
        let next = (from as isize + delta).clamp(0, count as isize - 1) as usize;
        self.cursor = Some(next);
        self.list.scroll_to_reveal_item(next);
        cx.notify();
    }

    fn step_next(&mut self, _: &OutlineNext, _: &mut Window, cx: &mut Context<Self>) {
        self.step(1, cx);
    }

    fn step_previous(&mut self, _: &OutlinePrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, cx);
    }

    fn open(&mut self, _: &OutlineOpen, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.cursor.or(self.current);
        let Some(line) = index.and_then(|index| self.headings.get(index)).map(|heading| heading.line) else { return };
        self.jump(line, window, cx);
    }

    /// Jumps the editor to `line` and gives it the focus, as clicking a heading does.
    fn jump(&mut self, line: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.cursor = None;
        let Some(app) = self.app.upgrade() else { return };
        app.update(cx, |app, cx| {
            app.jump_to_line(line, cx);
            app.focus_editor(window, cx);
        });
        cx.notify();
    }
}

impl Render for OutlinePane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self._focus_in.is_none() {
            self._focus_in = Some(cx.on_focus(&self.focus, window, |_, _, cx| cx.notify()));
            self._focus_out = Some(cx.on_blur(&self.focus, window, |_, _, cx| cx.notify()));
        }
        if !self.layout_nudged && self.list.item_count() > 0 && self.list.viewport_bounds().size.height == px(0.) {
            self.layout_nudged = true;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_millis(50)).await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }
        let dark = self.dark;
        let count = self.headings.len();
        let selected = self.cursor.filter(|index| *index < count).or(self.current);
        let cursor_active = self.cursor.is_some();
        let current = self.current;
        let column = focus_ring(
            div()
                .id("outline")
                .key_context("Outline")
                .track_focus(&self.focus)
                .role(Role::List)
                .aria_label("Outline")
                .relative()
                .size_full()
                .bg(color(dark, Color::Base))
                .px_2()
                .py_2()
                .text_size(px(theme::CHROME_TEXT))
                .on_action(cx.listener(Self::step_next))
                .on_action(cx.listener(Self::step_previous))
                .on_action(cx.listener(Self::open)),
            dark,
        );
        if count == 0 {
            return column.child(div().text_color(quiet(dark)).child("No headings"));
        }
        let state = self.list.clone();
        column.child(
            list(
                state,
                cx.processor(move |this: &mut OutlinePane, index, _window, cx| {
                    let Some(heading): Option<&Heading> = this.headings.get(index) else { return div().into_any_element() };
                    let (line, level, text) = (heading.line, heading.level, heading.text.clone());
                    let is_current = Some(index) == current;
                    let is_selected = Some(index) == selected;
                    let tip = text.clone();
                    div()
                        .id(("heading", index))
                        .tooltip(move |_, cx| {
                            let text = tip.clone();
                            cx.new(|_| OutlineTip { text, dark }).into()
                        })
                        .role(Role::ListItem)
                        .aria_label(text.clone())
                        .aria_level(level as usize)
                        .aria_position_in_set(index + 1)
                        .aria_size_of_set(count)
                        .aria_selected(is_selected)
                        .when(is_selected, |item| item.aria_active_descendant())
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .py_1()
                        .pl(px(4. + (level.saturating_sub(1) as f32) * 12.))
                        .when(is_current, |item| item.text_color(color(dark, Color::AccentText)).font_weight(gpui::FontWeight::BOLD))
                        .when(!is_current && level <= 2, |item| item.font_weight(gpui::FontWeight::BOLD))
                        .when(!is_current && level >= 5, |item| item.text_color(quiet(dark)))
                        .when(is_selected && cursor_active, |item| item.bg(color(dark, Color::Surface)))
                        .hover(move |style| style.bg(color(dark, Color::Surface)))
                        .cursor_pointer()
                        .child(div().w(px(8.)).when(is_current, |marker| marker.child("•")))
                        .child(div().flex_1().min_w(px(0.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(text))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.jump(line, window, cx)))
                        .into_any_element()
                }),
            )
            .size_full(),
        )
        .child(scrollbar::indicator(&self.list, dark))
    }
}

/// The full heading text for a row shortened with an ellipsis.
struct OutlineTip {
    text: SharedString,
    dark: bool,
}

impl Render for OutlineTip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .text_size(px(theme::CAPTION_TEXT))
            .rounded(px(theme::RADIUS_OVERLAY))
            .shadow_md()
            .bg(color(self.dark, Color::Card))
            .text_color(color(self.dark, Color::Text))
            .border_1()
            .border_color(color(self.dark, Color::StrokeStrong))
            .child(self.text.clone())
    }
}
