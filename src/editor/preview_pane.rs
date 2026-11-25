//! One document's preview pane, as its own GPUI view.
//!
//! The parent window places this view with [`Entity::cached`], so a keystroke redraws only the
//! editor: the preview's subtree is reused until a parse, a theme or font change, a Save As, or a
//! link selection notifies it. Scroll position belongs to the document's pane, so switching tabs
//! returns to where that document was left.
//!
//! Links that stay inside the preview (`#fn:`, `#fnref:`, other anchors) are handled here; web and
//! file links go back to the window, which owns the app's link rules.

use super::*;

pub(super) struct PreviewPane {
    blocks: Arc<Vec<Block>>,
    links: Vec<(Arc<str>, usize)>,
    /// One item per preview block, with this tab's scroll position.
    list: ListState,
    font_size: f32,
    dark: bool,
    mono: SharedString,
    folder: Option<PathBuf>,
    /// The link the arrow keys have selected; the pane is one tab stop, not one per link.
    selected_link: Option<usize>,
    focus: FocusHandle,
    /// Focus changes only affect the ring, so they are observed to bust the parent's cache.
    _focus_in: Option<Subscription>,
    _focus_out: Option<Subscription>,
    /// A cached pane renders once before its list has bounds; one nudge brings the scrollbar in.
    layout_nudged: bool,
    app: gpui::WeakEntity<Prototype>,
}

impl PreviewPane {
    pub(super) fn new(app: gpui::WeakEntity<Prototype>, font_size: f32, dark: bool, mono: SharedString, folder: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle().tab_stop(true);
        let list = ListState::new(0, ListAlignment::Top, px(OVERDRAW));
        // Wheel scrolling changes the list without a parent notify; the cached subtree must repaint.
        list.set_scroll_handler(cx.listener(|_, _: &ListScrollEvent, _, cx| cx.notify()));
        Self {
            blocks: Arc::default(),
            links: Vec::new(),
            list,
            font_size,
            dark,
            mono,
            folder,
            selected_link: None,
            focus,
            _focus_in: None,
            _focus_out: None,
            layout_nudged: false,
            app,
        }
    }

    /// Replaces only the changed blocks, so measured heights and the scroll position survive.
    pub(super) fn apply(&mut self, blocks: Arc<Vec<Block>>, links: Vec<(Arc<str>, usize)>, changed: Range<usize>, count: usize, cx: &mut Context<Self>) {
        let unchanged = changed.is_empty() && count == 0;
        if unchanged {
            return;
        }
        self.list.splice(changed, count);
        self.blocks = blocks;
        self.links = links;
        if self.selected_link.is_some_and(|index| index >= self.links.len()) {
            self.selected_link = None;
        }
        cx.notify();
    }

    /// An edit can change which link an index names, so the arrow-key selection is dropped.
    pub(super) fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if self.selected_link.take().is_some() {
            cx.notify();
        }
    }

    /// Drops the shown content, as when another program's version replaces the document.
    pub(super) fn clear(&mut self, cx: &mut Context<Self>) {
        let old = self.blocks.len();
        self.list.splice(0..old, 0);
        self.blocks = Arc::default();
        self.links.clear();
        self.selected_link = None;
        cx.notify();
    }

    pub(super) fn set_look(&mut self, font_size: f32, dark: bool, mono: SharedString, cx: &mut Context<Self>) {
        if self.font_size == font_size && self.dark == dark && self.mono == mono {
            return;
        }
        (self.font_size, self.dark, self.mono) = (font_size, dark, mono);
        cx.notify();
    }

    pub(super) fn set_folder(&mut self, folder: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.folder == folder {
            return;
        }
        self.folder = folder;
        cx.notify();
    }

    pub(super) fn scroll_to_block(&mut self, index: usize, cx: &mut Context<Self>) {
        self.list.scroll_to(ListOffset { item_ix: index, offset_in_item: px(0.) });
        cx.notify();
    }

    pub(super) fn focus(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.focus, cx);
    }

    pub(super) fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.list.viewport_bounds()
    }

    fn follow(&mut self, link: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.follow_fragment(link) {
            cx.notify();
            return;
        }
        let Some(app) = self.app.upgrade() else { return };
        app.update(cx, |app, cx| app.follow_link(link, window, cx));
    }

    /// Handles the destinations that point back into this pane. Returns whether it did.
    fn follow_fragment(&mut self, link: &str) -> bool {
        if let Some(number) = link.strip_prefix("#fn:").and_then(|number| number.parse::<u32>().ok()) {
            if let Some(index) = self.blocks.iter().position(|block| block.footnote == Some(number)) {
                self.scroll_to_index(index);
            }
            return true;
        }
        if let Some(number) = link.strip_prefix("#fnref:") {
            let target = format!("#fn:{number}");
            let refers = |runs: &[preview::Run]| runs.iter().any(|run| run.style.link.as_deref() == Some(target.as_str()));
            let found = self.blocks.iter().position(|block| {
                refers(&block.runs) || block.table.as_ref().is_some_and(|table| table.rows.iter().flatten().any(|cell| refers(cell)))
            });
            if let Some(index) = found {
                self.scroll_to_index(index);
            }
            return true;
        }
        // The app gives headings no ids, so other #anchors go nowhere.
        link.starts_with('#')
    }

    fn scroll_to_index(&mut self, index: usize) {
        self.list.scroll_to(ListOffset { item_ix: index, offset_in_item: px(0.) });
    }

    fn link_next(&mut self, _: &PreviewLinkNext, _: &mut Window, cx: &mut Context<Self>) {
        self.step_link(1, cx);
    }

    fn link_previous(&mut self, _: &PreviewLinkPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.step_link(-1, cx);
    }

    fn step_link(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.links.len() as isize;
        if count == 0 {
            self.selected_link = None;
            cx.notify();
            return;
        }
        let next = match self.selected_link {
            Some(index) => (index as isize + delta).rem_euclid(count),
            None if delta > 0 => 0,
            None => count - 1,
        } as usize;
        self.selected_link = Some(next);
        self.scroll_to_index(self.links[next].1);
        cx.notify();
    }

    fn link_open(&mut self, _: &PreviewLinkOpen, window: &mut Window, cx: &mut Context<Self>) {
        let Some((link, _)) = self.selected_link.and_then(|index| self.links.get(index).cloned()) else { return };
        self.follow(&link, window, cx);
    }

    fn line_up(&mut self, _: &PreviewLineUp, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_by(px(40.), cx);
    }

    fn line_down(&mut self, _: &PreviewLineDown, _: &mut Window, cx: &mut Context<Self>) {
        self.scroll_by(-px(40.), cx);
    }

    fn page_up(&mut self, _: &PreviewPageUp, _: &mut Window, cx: &mut Context<Self>) {
        let page = self.list.viewport_bounds().size.height * 0.9;
        self.scroll_by(page, cx);
    }

    fn page_down(&mut self, _: &PreviewPageDown, _: &mut Window, cx: &mut Context<Self>) {
        let page = self.list.viewport_bounds().size.height * 0.9;
        self.scroll_by(-page, cx);
    }

    fn top(&mut self, _: &PreviewTop, _: &mut Window, cx: &mut Context<Self>) {
        self.list.scroll_to(ListOffset { item_ix: 0, offset_in_item: px(0.) });
        cx.notify();
    }

    fn bottom(&mut self, _: &PreviewBottom, _: &mut Window, cx: &mut Context<Self>) {
        self.list.scroll_to_end();
        cx.notify();
    }

    /// Positive `by` scrolls up, like a wheel turned away from you.
    fn scroll_by(&mut self, by: Pixels, cx: &mut Context<Self>) {
        self.list.scroll_by(-by);
        cx.notify();
    }

    /// The preview's text as one read-only AccessKit text pattern, capped so a large document does
    /// not build a tree the size of its text on every frame.
    fn described(&self, window: &Window) -> Option<Vec<a11y::Line>> {
        if !window.is_a11y_active() {
            return None;
        }
        let lines: Vec<a11y::Line> = self
            .blocks
            .iter()
            .enumerate()
            .take(a11y::LINES_DESCRIBED)
            .map(|(index, block)| a11y::Line { index, text: preview_view::block_label(block), bounds: None, positions: None, widths: None })
            .filter(|line| !line.text.is_empty())
            .collect();
        let mut lines = a11y::within_budget(lines, &[]);
        a11y::end_lines(&mut lines);
        Some(lines)
    }
}

impl preview_view::PreviewHost for PreviewPane {
    fn follow_preview_link(&mut self, link: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.follow(link, window, cx);
    }
}

impl Render for PreviewPane {
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
        let focused_ring = ring(dark);
        if self.selected_link.is_some_and(|index| index >= self.links.len()) {
            self.selected_link = None;
        }
        let selected = self.selected_link.and_then(|index| self.links.get(index)).map(|(link, _)| link.clone());
        let label = match &selected {
            Some(link) => SharedString::from(format!("Preview, link {link}")),
            None => SharedString::from("Preview"),
        };
        let described = self.described(window);
        let blocks = self.list.clone();
        div()
            .id("preview")
            .key_context("Preview")
            .track_focus(&self.focus)
            .role(gpui::Role::Document)
            .aria_label(label)
            .a11y_synthetic_children(move |builder| {
                if let Some(lines) = &described {
                    a11y::describe_document(builder, lines);
                }
            })
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(move |style| style.border_color(focused_ring))
            .relative()
            .size_full()
            .bg(tone(dark, 0x09090b, 0xffffff))
            .text_color(tone(dark, 0xe4e4e7, 0x18181b))
            .on_action(cx.listener(Self::link_next))
            .on_action(cx.listener(Self::link_previous))
            .on_action(cx.listener(Self::link_open))
            .on_action(cx.listener(Self::line_up))
            .on_action(cx.listener(Self::line_down))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::top))
            .on_action(cx.listener(Self::bottom))
            .child(
                list(
                    blocks,
                    cx.processor(move |this, index, window, cx| {
                        let look = preview_view::Look {
                            font_size: this.font_size,
                            dark: this.dark,
                            mono: this.mono.clone(),
                            folder: this.folder.clone(),
                            svg: cx.svg_renderer(),
                            selected_link: selected.clone(),
                        };
                        let item = match this.blocks.get(index) {
                            Some(block) => preview_view::preview_item(block, index, &look, cx),
                            None => div().into_any_element(),
                        };
                        // A formula over this frame's layout budget is drawn in the next one; the
                        // pane must notify itself too, or the parent reuses this cached frame.
                        if math::take_deferred() {
                            window.request_animation_frame();
                            cx.notify();
                        }
                        item
                    }),
                )
                .size_full(),
            )
            .child(scrollbar::indicator(&self.list, dark))
    }
}
