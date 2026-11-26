//! The window: tabs, the source editor, the preview, the outline, menus and the find bar.
//!
//! Text lives in a rope (`buffer.rs`) with undo and redo. Characters arrive through GPUI's platform
//! input handler, because that is how Windows text input and IME reach a view. Both panes are
//! virtualised GPUI lists, so only the lines and blocks near the viewport are laid out, and the
//! preview is re-parsed on a background thread once typing pauses, as in the app.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, FocusHandle, Focusable, HighlightStyle,
    ExternalPaths, IntoElement, ListAlignment, ListOffset, ListScrollEvent, ListState, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollHandle,
    PromptLevel, SharedString, StyledText, Subscription, Task, TextAlign, TextLayout, TextRun, UTF16Selection, Window, WindowBounds,
    actions, canvas, deferred, div, fill, list, point, prelude::*, px, relative, size, svg,
};

use crate::allowlist;
use crate::buffer::{self, Buffer, EditKind, LineSplice, SavePoint};
use crate::{diagrams, diagnostics, instance, math, store, watcher};
use crate::files::{self, FileMeta, OpenedFile};
use crate::perf;
use crate::preview::{self, Block, Kind};
use crate::preview_view;
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
mod links;
mod lists;
mod outline_pane;
mod preview_pane;
mod recovery;
mod scrollbar;
mod source_pane;
mod tabs;
mod window;

pub(crate) use crate::theme::{self, color, quiet, ring, Color};
pub use chrome::*;
pub use document::Document;
use document::{file_name, headings_of, links_of, Counts, ImagesInUse, Parsed};
use drag::Drag;
use outline_pane::OutlinePane;
use preview_pane::PreviewPane;
use recovery::Recovery;
use source_pane::SourcePane;
use tabs::Tabs;

/// CodeMirror's default indent unit, which Tab adds and Shift+Tab removes (the app binds `indentWithTab`).
const INDENT: &str = "  ";

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

/// The layout a cached pane fills. Cached views are not measured from their contents, so the size
/// has to be supplied; a relative fill resolves against the pane's wrapper.
fn fill_style() -> gpui::StyleRefinement {
    let mut style = gpui::StyleRefinement::default();
    style.size.width = Some(relative(1.0).into());
    style.size.height = Some(relative(1.0).into());
    style
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum OpenMenu {
    File,
    Edit,
    View,
    Help,
}

/// The About dialog: a card over the panes, and the update check it can run.
pub(super) struct About {
    /// The last check, or `None` before one has finished.
    pub check: Option<crate::update::Check>,
    /// A check is in flight.
    pub checking: bool,
}

/// The cached views of one document. Kept beside `Tabs` rather than inside `Document`, which stays
/// free of GPUI entities so its tests need no window.
struct TabViews {
    preview: Entity<PreviewPane>,
    outline: Entity<OutlinePane>,
}

/// Which pane edge a drag is moving.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PaneResize {
    Outline,
    Split,
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
    /// The preview and outline views of each open document, keyed by document id.
    views: HashMap<u64, TabViews>,
    /// The source editor's view, shared by every tab.
    source: Option<Entity<SourcePane>>,
    /// Fraction of the editor-plus-preview width given to the editor, and the outline's width.
    split: f32,
    outline_width: f32,
    /// The pane edge being dragged, if any.
    resizing: Option<PaneResize>,
    /// Last laid-out viewports, so a splitter drag can map the pointer to a fraction.
    editor_viewport: Option<Bounds<Pixels>>,
    preview_viewport: Option<Bounds<Pixels>>,
    /// Whether AccessKit was active last frame, so a change can invalidate the cached panes.
    a11y_active: bool,
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
    /// The About card, while it is open: the update check it shows and its two focus handles.
    about: Option<About>,
    about_focus: FocusHandle,
    about_check: FocusHandle,
    /// The update check in flight, so a second one cannot start over it.
    update_task: Option<Task<()>>,
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
    /// The settings write in flight; closing waits for it so the last change is on disk.
    pub(super) settings_task: Option<Task<()>>,
    /// A close asked for while a save was still being written; it asks again once the save is done.
    close_task: Option<Task<()>>,
    /// Set once the window has been asked to close, so a command handler that is still unwinding
    /// does not try to give focus to a view inside it.
    window_closing: bool,
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
        // The maximize button's glyph follows the window state, which a platform-driven resize
        // changes without any click of ours: repaint so the glyph flips.
        let bounds = cx.observe_window_bounds(window, |this, window, cx| {
            this.schedule_geometry_save(window, cx);
            cx.notify();
        });
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
            views: HashMap::new(),
            source: None,
            split: 0.5,
            outline_width: 220.,
            resizing: None,
            editor_viewport: None,
            preview_viewport: None,
            a11y_active: false,
            prepaint_lines: Rc::new(RefCell::new(Vec::new())),
            settings_focus: cx.focus_handle(),
            settings_controls: (0..chrome::settings_slot::COUNT).map(|_| cx.focus_handle()).collect(),
            settings_open: false,
            recent: store::load_recent(&config_dir).into_iter().filter(|path| path.is_file()).collect(),
            recent_open: false,
            recent_focus: cx.focus_handle(),
            about: None,
            about_focus: cx.focus_handle(),
            about_check: cx.focus_handle(),
            update_task: None,
            recovery: Recovery::new(recovery_dir),
            request_watcher: None,
            _instance_lock: instance_lock,
            tab_leaves: false,
            painted_lines: Vec::new(),
            frame_started: None,
            geometry_task: None,
            settings_task: None,
            close_task: None,
            window_closing: false,
            _appearance: appearance,
            _bounds: bounds,
        };
        if !this.settings.can_save() {
            this.report("Settings could not be read; changes will not be saved".to_string());
        }
        this.open_source(cx);
        this.open_views(0, cx);
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

    /// Creates the preview and outline views for a document that has just opened.
    fn open_views(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(doc) = self.tabs.iter().find(|doc| doc.id == id) else { return };
        let folder = Path::new(&doc.path).parent().map(Path::to_path_buf);
        let app = cx.entity().downgrade();
        let (font_size, dark, mono) = (self.settings.font_size, self.dark, self.mono.clone());
        let preview = cx.new(|cx| PreviewPane::new(app.clone(), font_size, dark, mono, folder, cx));
        let outline = cx.new(|cx| OutlinePane::new(dark, app, cx));
        self.views.insert(id, TabViews { preview, outline });
    }

    fn active_views(&self) -> Option<&TabViews> {
        self.views.get(&self.tabs.active_doc().id)
    }

    /// Creates the source view once, after the window exists. Its focus handle is the window's
    /// editor handle, so existing focus paths keep working.
    fn open_source(&mut self, cx: &mut Context<Self>) {
        let app = cx.entity().downgrade();
        let focus = self.focus.clone();
        self.source = Some(cx.new(|cx| SourcePane::new(app, focus, cx)));
    }

    /// The source view inside a wrapper that owns the editing-key handlers. The wrapper is an
    /// ancestor of the editor only, so Backspace in the find field cannot reach the document.
    fn editor_area(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(source) = &self.source else { return div().into_any_element() };
        let source = source.clone();
        let grow = if self.settings.preview_visible() { self.split } else { 1.0 };
        let wrapper = div().flex().flex_grow(grow).flex_basis(px(0.)).min_w(px(0.)).h_full().child(source);
        self.editor_actions(wrapper, cx).into_any_element()
    }

    /// The active document's preview, filling the flex row. Cached until the pane is notified.
    fn preview_area(&mut self, _cx: &mut Context<Self>) -> AnyElement {
        let grow = if self.settings.show_editor { 1.0 - self.split } else { 1.0 };
        let Some(views) = self.active_views() else { return div().flex_grow(grow).into_any_element() };
        let preview = views.preview.clone();
        div()
            .flex_grow(grow)
            .flex_basis(px(0.))
            .min_w(px(0.))
            .h_full()
            .child(preview.cached(fill_style()))
            .into_any_element()
    }

    /// The active document's outline, at its fixed width. Cached until headings or the caret's
    /// heading change.
    fn outline_area(&mut self, _cx: &mut Context<Self>) -> AnyElement {
        let Some(views) = self.active_views() else { return div().w(px(self.outline_width)).into_any_element() };
        let outline = views.outline.clone();
        div().w(px(self.outline_width)).flex_none().h_full().child(outline.cached(fill_style())).into_any_element()
    }

    /// A six-pixel grab strip with a one-pixel line, between two panes.
    fn pane_divider(&mut self, id: &'static str, resize: PaneResize, cx: &mut Context<Self>) -> AnyElement {
        let dark = self.dark;
        let active = self.resizing == Some(resize);
        let line = color(dark, Color::Stroke);
        let background = if active { color(dark, Color::AccentSubtle) } else { line };
        div()
            .id(id)
            .flex()
            .flex_none()
            .justify_center()
            .w(px(6.))
            .h_full()
            .cursor_col_resize()
            .hover(move |style| style.bg(color(dark, Color::Hover)))
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                this.resizing = Some(resize);
                this.drag.end();
                cx.notify();
            }))
            .child(div().w(px(1.)).h_full().bg(background))
            .into_any_element()
    }

    /// Splitter and pane-edge drags, then the source selection drag.
    fn pointer_moved(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if let Some(resize) = self.resizing {
            if event.pressed_button != Some(MouseButton::Left) {
                self.resizing = None;
                cx.notify();
                return;
            }
            match resize {
                PaneResize::Outline => {
                    self.outline_width = f32::from(event.position.x).clamp(140., 400.);
                }
                PaneResize::Split => {
                    if let (Some(editor), Some(preview)) = (self.editor_viewport, self.preview_viewport) {
                        let left = editor.left();
                        let total = f32::from(preview.right() - left).max(1.);
                        self.split = (f32::from(event.position.x - left) / total).clamp(0.15, 0.85);
                    }
                }
            }
            cx.notify();
            return;
        }
        self.drag_anywhere(event, cx);
    }

    /// Applies a font-size or theme change to every pane.
    fn update_view_looks(&mut self, cx: &mut Context<Self>) {
        let (font_size, dark, mono) = (self.settings.font_size, self.dark, self.mono.clone());
        for views in self.views.values() {
            views.preview.update(cx, |pane, cx| pane.set_look(font_size, dark, mono.clone(), cx));
            views.outline.update(cx, |pane, cx| pane.set_dark(dark, cx));
        }
    }

    /// Feeds the outline the heading under the caret, notifying it only when that heading changes.
    fn sync_outline_current(&mut self, cx: &mut Context<Self>) {
        let current = self.heading_at_caret();
        let Some(views) = self.active_views() else { return };
        views.outline.update(cx, |pane, cx| pane.set_current(current, cx));
    }

    /// A cached pane keeps the tree it built before AccessKit started listening; a change in that
    /// state has to bust both caches, or the preview loses its text pattern.
    fn refresh_views_for_a11y(&mut self, window: &Window, cx: &mut Context<Self>) {
        let active = window.is_a11y_active();
        if active == self.a11y_active {
            return;
        }
        self.a11y_active = active;
        self.notify_views(cx);
    }

    fn notify_views(&mut self, cx: &mut Context<Self>) {
        for views in self.views.values() {
            views.preview.update(cx, |_, cx| cx.notify());
            views.outline.update(cx, |_, cx| cx.notify());
        }
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
        if self.close_about(window, cx) {
            return;
        }
        if self.close_recent(window, cx) {
            return;
        }
        if self.close_settings(window, cx) {
            return;
        }
        if self.find.is_some() {
            // Focus the editor before dropping the find field, whose own focus would otherwise be
            // cleared as it goes and leave the window with none.
            self.focus_editor(window, cx);
            self.find = None;
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

    /// Each tab keeps its own lists, so switching back restores its scroll positions.
    fn activate_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.tabs.activate(index) {
            return;
        }
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
        self.update_view_looks(cx);
        self.keep_settings(cx);
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
        if !self.logged_first_frame {
            perf::mark("render");
        }
        if crate::high_contrast::refresh_periodically() {
            self.notify_views(cx);
        }
        // A new accent repaints cached panes; the window itself redraws every frame regardless.
        if crate::accent::refresh() {
            self.notify_views(cx);
        }
        self.refresh_views_for_a11y(window, cx);
        let dark = self.settings.is_dark(window.appearance());
        if dark != self.dark {
            self.dark = dark;
            self.release_unused_images(cx);
            self.update_view_looks(cx);
        }
        self.caret_bounds = None;
        self.painted_lines.clear();
        self.prepaint_lines.borrow_mut().clear();
        self.update_title(window);
        self.refresh_matches(cx);
        self.sync_scroll(cx);
        self.sync_outline_current(cx);
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
        self.editor_viewport = Some(self.doc().lines.viewport_bounds());
        self.preview_viewport = if show_preview {
            self.active_views().map(|views| views.preview.update(cx, |pane, _| pane.viewport_bounds()))
        } else {
            None
        };

        let panes = div()
            .flex()
            .flex_1()
            .min_h_0()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::close_menu))
            .when(show_outline, |row| row.child(self.outline_area(cx)))
            .when(show_outline, |row| row.child(self.pane_divider("outline-divider", PaneResize::Outline, cx)))
            .when(show_editor, |row| row.child(self.editor_area(cx)))
            .when(show_editor && show_preview, |row| row.child(self.pane_divider("split-divider", PaneResize::Split, cx)))
            .when(show_preview, |row| row.child(self.preview_area(cx)))
            .when(self.recent_open, |row| row.child(self.recent_panel(cx)))
            .when(self.settings_open, |row| row.child(self.settings_panel(cx)));
        let find_bar = self.find_bar(cx);
        let maximized = window.is_maximized();

        let root = div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(color(dark, Color::Base))
            .text_size(px(font_size))
            .text_color(color(dark, Color::Text))
            .key_context("Prototype")
            // Wherever the button comes up, a drag selection ends.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    this.drag.end();
                    if this.resizing.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| this.pointer_moved(event, cx)))
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
            .child(self.title_bar(maximized, cx))
            .child(self.menu_bar(cx))
            .child(self.toolbar(cx))
            .children(find_bar)
            .child(panes)
            .child(self.status_bar(cx))
            // Last child, so it paints after the panes.
            .child(self.frame_timer())
            // And the About card over everything, while it is open.
            .child(self.about_dialog(cx));
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

