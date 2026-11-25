//! The window: tabs, the source editor, the preview, the outline, menus and the find bar.
//!
//! Text lives in a rope (`buffer.rs`) with undo and redo. Characters arrive through GPUI's platform
//! input handler, because that is how Windows text input and IME reach a view. Both panes are
//! virtualised GPUI lists, so only the lines and blocks near the viewport are laid out, and the
//! preview is re-parsed on a background thread once typing pauses, as in the app.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, FocusHandle, Focusable, HighlightStyle,
    ExternalPaths, IntoElement, ListAlignment, ListOffset, ListScrollEvent, ListState, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollHandle,
    PromptLevel, SharedString, StyledText, Subscription, Task, TextAlign, TextLayout, TextRun, UTF16Selection, Window, WindowBounds, actions, canvas,
    deferred, div, fill, list, point, prelude::*, px, rgb, size, svg,
};

use crate::allowlist;
use crate::buffer::{self, Buffer, EditKind, LineSplice, SavePoint};
use crate::{diagrams, diagnostics, instance, math, store, watcher};
use crate::files::{self, FileMeta, OpenedFile};
use crate::perf;
use crate::preview::{self, Block, Kind};
use crate::preview_view::{self, Look};
use crate::high_contrast::tone;
use crate::settings::{self, ViewSettings};
use crate::syntax::{self, Span};
use crate::text_field::{FieldEvent, InputLayer, TextField};

actions!(
    marky,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectAll,
        LineStart,
        LineEnd,
        Enter,
        Paste,
        Cut,
        Copy,
        Quit,
        Save,
        Undo,
        Redo,
        Find,
        FindReplace,
        FindNext,
        FindPrevious,
        FindClose,
        SelectUp,
        SelectDown,
        SelectLineStart,
        SelectLineEnd,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        DocumentStart,
        DocumentEnd,
        SelectDocumentStart,
        SelectDocumentEnd,
        DeleteWordLeft,
        DeleteWordRight,
        Indent,
        Outdent,
    ]
);

mod a11y;
mod chrome;
mod closing;
mod disk;
mod document;
mod drag;
mod draw;
mod editing;
mod find;
mod input;
mod lists;
mod recovery;
mod tabs;

pub use chrome::*;
pub use document::Document;
use document::{file_name, headings_of, links_of, Counts, ImagesInUse, Parsed};
use drag::Drag;
use recovery::Recovery;
use tabs::Tabs;

/// CodeMirror's default indent unit, which Tab adds and Shift+Tab removes (the app binds `indentWithTab`).
const INDENT: &str = "  ";

const CARET: u32 = 0x2563eb;

/// Lines and blocks laid out beyond the viewport, so scrolling a little does not wait for layout.
const OVERDRAW: f32 = 400.;

/// The app's `--font-mono` list, in its order. Windows 10 has only Consolas; Windows 11 adds Cascadia.
const MONO_FONTS: [&str; 3] = ["Cascadia Code", "Cascadia Mono", "Consolas"];

/// GPUI replaces a missing family with its UI font instead of trying the next one, so pick the first installed.
fn mono_family(cx: &App) -> SharedString {
    let installed = cx.text_system().all_font_names();
    let family = MONO_FONTS.into_iter().find(|name| installed.iter().any(|font| font == name)).unwrap_or("Consolas");
    family.into()
}

/// The preview waits for typing to pause: twice the last parse, 30–150 ms, like the app's `typingPauseMs`.
/// A typical document updates about 40 ms after the last keystroke; a large one never parses mid-burst.
fn parse_pause(last_parse_ms: f64) -> Duration {
    Duration::from_secs_f64((last_parse_ms * 2.0).clamp(30.0, 150.0) / 1000.0)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OpenMenu {
    File,
    Edit,
    View,
}

struct FindBar {
    query: Entity<TextField>,
    replacement: Entity<TextField>,
    show_replace: bool,
    /// Sorted, non-overlapping byte ranges.
    matches: Vec<Range<usize>>,
    /// Document id, buffer version and query the matches belong to.
    searched: Option<(u64, u64, String)>,
    _subscriptions: Vec<Subscription>,
}

pub struct Prototype {
    focus: FocusHandle,
    /// The open documents, which one is active, and the next id.
    tabs: Tabs,
    logged_first_frame: bool,
    /// When the last edit arrived; logged as `type_ms` once that frame has painted.
    typed_at: Option<Instant>,
    settings: ViewSettings,
    dark: bool,
    /// The editor's and the preview's code font.
    mono: SharedString,
    config_dir: PathBuf,
    open_menu: Option<OpenMenu>,
    find: Option<FindBar>,
    /// Where the caret was painted in the last frame. The IME places its candidate window here.
    caret_bounds: Option<Bounds<Pixels>>,
    /// The last error or result, shown in the status bar: a release build has no console.
    status: Option<String>,
    window_title: String,
    /// Set once the user chose Discard, so the second close request goes through.
    close_confirmed: bool,
    /// Whether a drag selection is in progress, and where the pointer is outside the pane.
    drag: Drag,
    /// Horizontal scroll of the source pane when word wrap is off.
    h_scroll: ScrollHandle,
    /// Where the caret was last painted, and for which offset: the horizontal follow needs a current one.
    caret_painted: Option<(usize, Bounds<Pixels>)>,
    /// The caret moved; scroll sideways to it once it has been painted.
    follow_x: bool,
    last_cursor: Option<(u64, usize)>,
    /// The monospace font's character width at the current size.
    char_width: Pixels,
    /// One per menu item, so the arrows can move the focus through an open menu.
    menu_focus: Vec<FocusHandle>,
    /// The open menu itself, focused when it was opened with the mouse, so the arrows work without an item highlighted.
    menu_root: FocusHandle,
    /// The outline is one tab stop; the arrows move this highlight through its headings.
    outline_focus: FocusHandle,
    outline_cursor: Option<usize>,
    preview_focus: FocusHandle,
    /// The preview is one tab stop; the left and right arrows select one of its links.
    preview_link: Option<usize>,
    /// Lines laid out this frame, shared with the accessibility callback, which needs their bounds
    /// after prepaint to give each text run a rectangle a screen magnifier can follow.
    prepaint_lines: Rc<RefCell<Vec<(usize, TextLayout)>>>,
    settings_focus: FocusHandle,
    /// One handle per control in the settings panel, so Tab and Shift+Tab stay inside it while it is
    /// open instead of moving out into the editor.
    settings_controls: Vec<FocusHandle>,
    settings_open: bool,
    /// The most recent files, newest first, from the app's `recent.json`.
    recent: Vec<PathBuf>,
    recent_open: bool,
    recent_focus: FocusHandle,
    /// Crash-recovery snapshots: where they live, any held until the user answers, and what the
    /// last write contained.
    recovery: Recovery,
    /// Watches `open-request.txt`, written by a second launch.
    request_watcher: Option<notify::RecommendedWatcher>,
    /// The single-instance lock; held for the window's lifetime.
    _instance_lock: Option<std::fs::File>,
    /// Esc was pressed in the editor: the next Tab moves the focus instead of indenting, as with
    /// CodeMirror's `indentWithTab`, so a keyboard user can always leave the editor.
    tab_leaves: bool,
    /// Source lines painted in the last frame, with their layouts.
    painted_lines: Vec<(usize, TextLayout)>,
    /// When this frame's render began: a frame over 50 ms is logged as `stall_ms`, the
    /// counterpart of the long tasks the large-file benchmark counts.
    frame_started: Option<Instant>,
    /// The last window bounds, written after a resize settles.
    geometry_task: Option<Task<()>>,
    /// A close asked for while a save was still being written; it asks again once the save is done.
    close_task: Option<Task<()>>,
    _appearance: Subscription,
    _bounds: Subscription,
}

impl Prototype {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        opened: OpenedFile,
        settings: ViewSettings,
        config_dir: PathBuf,
        recovery_dir: PathBuf,
        recovery: Vec<store::Snapshot>,
        instance_lock: Option<std::fs::File>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // "System" follows Windows' light or dark mode while the window is open.
        let appearance = cx.observe_window_appearance(window, |_, _, cx| cx.notify());
        let bounds = cx.observe_window_bounds(window, |this, window, cx| this.schedule_geometry_save(window, cx));
        let document = Document::open(opened, 0, cx);
        perf::mark("open");
        let mono = mono_family(cx);
        perf::mark("fonts");
        let mut this = Self {
            focus: cx.focus_handle().tab_stop(true),
            tabs: Tabs::new(document),
            logged_first_frame: false,
            typed_at: None,
            dark: settings.is_dark(window.appearance()),
            mono,
            settings,
            config_dir: config_dir.clone(),
            open_menu: None,
            find: None,
            caret_bounds: None,
            status: None,
            window_title: String::new(),
            close_confirmed: false,
            drag: Drag::default(),
            h_scroll: ScrollHandle::new(),
            caret_painted: None,
            follow_x: false,
            last_cursor: None,
            char_width: px(9.),
            menu_focus: (0..MENU_ITEMS).map(|_| cx.focus_handle()).collect(),
            menu_root: cx.focus_handle(),
            outline_focus: cx.focus_handle().tab_stop(true),
            outline_cursor: None,
            preview_focus: cx.focus_handle().tab_stop(true),
            preview_link: None,
            prepaint_lines: Rc::new(RefCell::new(Vec::new())),
            settings_focus: cx.focus_handle(),
            settings_controls: (0..chrome::settings_slot::COUNT).map(|_| cx.focus_handle()).collect(),
            settings_open: false,
            recent: store::load_recent(&config_dir).into_iter().filter(|path| path.is_file()).collect(),
            recent_open: false,
            recent_focus: cx.focus_handle(),
            recovery: Recovery::new(recovery_dir),
            request_watcher: None,
            _instance_lock: instance_lock,
            tab_leaves: false,
            painted_lines: Vec::new(),
            frame_started: None,
            geometry_task: None,
            close_task: None,
            _appearance: appearance,
            _bounds: bounds,
        };
        // The source paints first; the preview follows when the background parse is done, like the app.
        this.schedule_parse(0, Duration::ZERO, cx);
        this.watch_document(0, cx);
        this.restore_or_discard(recovery, cx);
        this.start_recovery_timer(cx);
        this.watch_requests(cx);
        this.check_pending_request(cx);
        this
    }

    fn doc(&self) -> &Document {
        self.tabs.active_doc()
    }

    fn doc_mut(&mut self) -> &mut Document {
        self.tabs.active_doc_mut()
    }

    fn report(&mut self, message: String) {
        eprintln!("{message}");
        self.status = Some(message);
    }

    // ----- Find and replace -----

    /// Esc closes what is open, innermost first: a menu, the settings panel, the find bar. With
    /// nothing open, in the editor, it lets the next Tab leave the editor instead of indenting.
    fn find_close(&mut self, _: &FindClose, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        if self.open_menu.is_some() {
            self.close_menus(window, cx);
            return;
        }
        if self.close_recent(window, cx) {
            return;
        }
        if self.close_settings(window, cx) {
            return;
        }
        if self.find.take().is_some() {
            self.focus_editor(window, cx);
            return;
        }
        if self.focus.is_focused(window) {
            self.tab_leaves = true;
        }
    }

    // ----- Files, tabs and closing -----

    // ----- Crash recovery, recent files and the single-instance request -----

    fn watch_requests(&mut self, cx: &mut Context<Self>) {
        let dir = self.config_dir.clone();
        let (sender, receiver) = async_channel::unbounded::<()>();
        match watcher::watch(&instance::request_path(&dir), move || {
            let _ = sender.try_send(());
        }) {
            Ok(watcher) => self.request_watcher = Some(watcher),
            Err(err) => diagnostics::write(&format!("could not watch for a second launch: {err}")),
        }
        cx.spawn(async move |this, cx| {
            while receiver.recv().await.is_ok() {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                while receiver.try_recv().is_ok() {}
                if this.update(cx, |this, cx| this.check_pending_request(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn check_pending_request(&mut self, cx: &mut Context<Self>) {
        match instance::take_request(&self.config_dir) {
            Some(Some(path)) => self.open_path(&path, cx),
            Some(None) => cx.activate(true),
            None => {}
        }
    }

    /// Every link the preview draws, in document order, with the block it is in. The preview is one
    /// tab stop; the left and right arrows move through this list and Enter follows the chosen link.
    fn preview_links(&self) -> &[(Arc<str>, usize)] {
        &self.doc().links
    }

    /// The preview's text as one read-only AccessKit text pattern, capped like the editor's so a
    /// large document does not build a tree the size of its text on every frame.
    fn preview_described(&self, window: &Window) -> Option<Vec<a11y::Line>> {
        if !window.is_a11y_active() {
            return None;
        }
        let lines: Vec<a11y::Line> = self
            .doc()
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

    /// A click on a link in the preview, with the app's rules (`followLink`): web and mail links open
    /// in the default program; a footnote reference scrolls the preview to its footnote; a relative
    /// link opens a .md, .markdown or .txt file next to this one as a tab. The preview never navigates.
    pub fn follow_link(&mut self, link: &str, _window: &mut Window, cx: &mut Context<Self>) {
        if link.starts_with("http://") || link.starts_with("https://") || link.starts_with("mailto:") {
            cx.open_url(link);
            return;
        }
        if let Some(number) = link.strip_prefix("#fn:").and_then(|number| number.parse::<u32>().ok()) {
            let doc = self.doc();
            if let Some(index) = doc.blocks.iter().position(|block| block.kind == Kind::Footnote(number)) {
                doc.block_list.scroll_to(ListOffset { item_ix: index, offset_in_item: px(0.) });
                cx.notify();
            }
            return;
        }
        // A footnote's ↩ scrolls back to the block that refers to it.
        if let Some(number) = link.strip_prefix("#fnref:") {
            let target = format!("#fn:{number}");
            let doc = self.doc();
            let refers = |runs: &[preview::Run]| runs.iter().any(|run| run.style.link.as_deref() == Some(target.as_str()));
            let found = doc.blocks.iter().position(|block| refers(&block.runs) || block.table.as_ref().is_some_and(|table| table.rows.iter().flatten().any(|cell| refers(cell))));
            if let Some(index) = found {
                doc.block_list.scroll_to(ListOffset { item_ix: index, offset_in_item: px(0.) });
                cx.notify();
            }
            return;
        }
        // The app gives headings no ids, so other #anchors go nowhere, as there.
        if link.starts_with('#') {
            return;
        }
        let target = link.split('#').next().unwrap_or_default().replace("%20", " ");
        match allowlist::link_target(Path::new(&self.doc().path), &target) {
            Ok(path) => self.open_path(&path, cx),
            Err(err) => {
                self.report(format!("Not opened: {link} ({err})"));
                cx.notify();
            }
        }
    }

    /// Each tab keeps its own lists, so switching back restores its scroll positions.
    fn activate_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.tabs.activate(index) {
            return;
        }
        self.preview_link = None;
        self.doc_mut().synced_source_line = None;
        cx.notify();
    }

    // ----- Menus and view settings -----

    fn close_menu(&mut self, _: &MouseDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.open_menu.is_none() {
            return;
        }
        self.open_menu = None;
        cx.notify();
    }

    fn step_font(&mut self, delta: isize, cx: &mut Context<Self>) {
        let current = settings::FONT_SIZES.iter().position(|size| *size == self.settings.font_size).unwrap_or(2);
        let next = current as isize + delta;
        if next < 0 || next >= settings::FONT_SIZES.len() as isize {
            self.open_menu = None;
            cx.notify();
            return;
        }
        self.settings.font_size = settings::FONT_SIZES[next as usize];
        // Formulas are laid out per font size, so the old size's bitmaps are no longer drawn.
        self.release_unused_images(cx);
        self.keep_settings(cx);
    }

    /// Writes the restored window bounds a moment after a resize or move stops, so dragging does
    /// not write the file on every frame.
    fn schedule_geometry_save(&mut self, window: &Window, cx: &mut Context<Self>) {
        let bounds = match window.window_bounds() {
            WindowBounds::Windowed(bounds) => (bounds, false),
            WindowBounds::Maximized(bounds) | WindowBounds::Fullscreen(bounds) => (bounds, true),
        };
        let geometry = store::WindowGeometry {
            width: f64::from(f32::from(bounds.0.size.width)),
            height: f64::from(f32::from(bounds.0.size.height)),
            x: f64::from(f32::from(bounds.0.origin.x)),
            y: f64::from(f32::from(bounds.0.origin.y)),
            maximized: bounds.1,
        };
        let config_dir = self.config_dir.clone();
        self.geometry_task = Some(cx.spawn(async move |_this, cx| {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            if let Err(err) = store::save_window_geometry(&config_dir, &geometry) {
                diagnostics::write(&format!("could not save the window geometry: {err}"));
            }
        }));
    }

    fn keep_settings(&mut self, cx: &mut Context<Self>) {
        if let Err(message) = settings::save(&self.config_dir, &self.settings) {
            self.report(message);
        }
        self.open_menu = None;
        cx.notify();
    }

    // ----- Scrolling and timing -----

    /// The caret stays visible, and the preview shows the block for the top visible source line.
    fn sync_scroll(&mut self) {
        let show_editor = self.settings.show_editor;
        let show_preview = self.settings.preview_visible();
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
        let block = preview::block_at_line(&doc.blocks, top + 1);
        doc.block_list.scroll_to(ListOffset { item_ix: block, offset_in_item: px(0.) });
    }

    /// With word wrap off, keeps the caret in view sideways after it moves. The caret's bounds are
    /// only known once it is painted, so the scroll happens on the frame after the move; scrolling by
    /// hand is left alone until the caret moves again. Returns whether the offset changed.
    fn follow_caret_sideways(&mut self) -> bool {
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

    /// Startup and typing are timed to the end of the frame's paint, like the app's
    /// measurements (a rAF callback that reports after paint).
    fn frame_timer(&mut self) -> impl IntoElement {
        let first_frame = !self.logged_first_frame;
        self.logged_first_frame = true;
        let typed_at = self.typed_at.take();
        let started = self.frame_started.take();
        canvas(
            // Last child: its prepaint runs once layout and every other prepaint are done.
            move |_, _, _| {
                if first_frame {
                    perf::mark("layout");
                }
            },
            move |_, _, _, _| {
                if first_frame {
                    perf::log_startup();
                }
                if let Some(typed_at) = typed_at {
                    perf::log(&format!("type_ms={:.1}", typed_at.elapsed().as_secs_f64() * 1000.0));
                }
                // Render, layout, prepaint and paint of one frame, all on the main thread. The first
                // frame is start-up, timed as such.
                if let Some(started) = started.filter(|_| !first_frame) {
                    let ms = started.elapsed().as_secs_f64() * 1000.0;
                    if ms > 50.0 {
                        perf::log(&format!("stall_ms={ms:.0} frame"));
                    }
                }
            },
        )
        .absolute()
        .size_0()
    }

    fn update_title(&mut self, window: &mut Window) {
        let doc = self.doc();
        let title = format!("{}{} - Marky", doc.display_name(), if doc.is_dirty() { " •" } else { "" });
        if title != self.window_title {
            window.set_window_title(&title);
            self.window_title = title;
        }
    }
}



impl Focusable for Prototype {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Prototype {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frame_started = Some(Instant::now());
        math::begin_frame();
        crate::high_contrast::refresh_periodically();
        if !self.logged_first_frame {
            perf::mark("render");
        }
        let dark = self.settings.is_dark(window.appearance());
        if dark != self.dark {
            self.dark = dark;
            self.release_unused_images(cx);
        }
        self.caret_bounds = None;
        self.painted_lines.clear();
        self.prepaint_lines.borrow_mut().clear();
        self.update_title(window);
        self.refresh_matches(cx);
        self.sync_scroll();
        if !self.settings.word_wrap {
            let font = gpui::font(self.mono.clone());
            let font_id = window.text_system().resolve_font(&font);
            if let Ok(width) = window.text_system().em_advance(font_id, px(self.settings.font_size)) {
                self.char_width = width;
            }
            if self.follow_caret_sideways() {
                cx.notify();
            }
        }
        let dark = self.dark;
        let show_editor = self.settings.show_editor;
        let show_preview = self.settings.preview_visible();
        let show_outline = self.settings.show_outline;
        let font_size = self.settings.font_size;

        let panes = div()
            .flex()
            .flex_1()
            .min_h_0()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::close_menu))
            .when(show_outline, |row| row.child(self.outline_column(cx)))
            .when(show_editor, |row| row.child(self.editor_column(window, cx)))
            .when(show_preview, |row| row.child(self.preview_column(window, cx)))
            .when(self.recent_open, |row| row.child(self.recent_panel(cx)))
            .when(self.settings_open, |row| row.child(self.settings_panel(cx)));
        let find_bar = self.find_bar(cx);

        let root = div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(tone(dark, 0x18181b, 0xf4f4f5))
            .text_size(px(font_size))
            .text_color(tone(dark, 0xfafafa, 0x18181b))
            .key_context("Prototype")
            // Wherever the button comes up, a drag selection ends.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| {
                    this.drag.end();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| this.drag_anywhere(event, cx)))
            // Dropping files on the window opens each one as a tab, as the app does.
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                for path in paths.paths() {
                    if path.is_file() {
                        this.open_path(path, cx);
                    }
                }
            }))
            .on_action(cx.listener(Self::find))
            .on_action(cx.listener(Self::find_replace))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_previous))
            .on_action(cx.listener(Self::find_close))
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::save))
            .child(self.menu_bar(cx))
            .child(self.toolbar(cx))
            .child(self.tab_strip(cx))
            .children(find_bar)
            .child(panes)
            .child(self.status_bar(cx))
            // Last child, so it paints after the panes.
            .child(self.frame_timer());
        self.command_actions(root, cx)
    }
}

/// Where the caret is drawn for `column` in a painted line. A wrap point is both the end of one
/// visual row and the start of the next; GPUI places it at the end, but a caret there belongs at the
/// start of the next row (CodeMirror's forward association), where typing will put the text.
/// `next` is the offset after the character at `column`, if there is one.
fn caret_origin(layout: &TextLayout, column: usize, next: Option<usize>) -> Option<Point<Pixels>> {
    let here = layout.position_for_index(column)?;
    let Some(after) = next.and_then(|next| layout.position_for_index(next)) else { return Some(here) };
    if after.y > here.y { Some(point(layout.bounds().left(), after.y)) } else { Some(here) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The preview's pause is twice the last parse, held inside 30–150 ms: too short and a burst of
    /// keystrokes parses on every one, too long and the preview lags behind typing.
    #[test]
    fn preview_pause_is_twice_the_last_parse_within_bounds() {
        assert_eq!(parse_pause(0.0), Duration::from_millis(30));
        assert_eq!(parse_pause(5.0), Duration::from_millis(30));
        assert_eq!(parse_pause(20.0), Duration::from_millis(40));
        assert_eq!(parse_pause(75.0), Duration::from_millis(150));
        assert_eq!(parse_pause(500.0), Duration::from_millis(150));
    }
}

