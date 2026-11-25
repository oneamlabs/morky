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
mod disk;
mod drag;
mod draw;
mod editing;
mod find;
mod lists;
mod recovery;
mod tabs;

pub use chrome::*;
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

struct Document {
    /// Finds the document again when a background parse finishes, even if tabs moved meanwhile.
    id: u64,
    /// Empty for a New Tab document until it is saved.
    path: String,
    meta: FileMeta,
    /// From the last read or save; a watcher event is ours when the file still has this time.
    mtime_ms: u64,
    /// Lossy decode. Saving would corrupt the file, so Save refuses it.
    read_only: bool,
    /// A recovered document's text is unsaved even though its undo history is empty.
    restored: bool,
    buffer: Buffer,
    selected: Range<usize>,
    /// The caret is at `selected.start` when the selection was made backwards.
    selection_reversed: bool,
    marked: Option<Range<usize>>,
    /// Shared with the next background parse, which compares its result against them.
    blocks: Arc<Vec<Block>>,
    /// The formulas and diagrams in `blocks`, gathered by the parse: freeing unused images reads these.
    images: Arc<ImagesInUse>,
    /// The preview's links and the outline's headings, gathered from `blocks` by the parse, so
    /// drawing a frame does not walk every block.
    links: Vec<(Arc<str>, usize)>,
    headings: Vec<Heading>,
    /// Syntax colours per source line, from the last background parse.
    spans: Vec<Vec<Span>>,
    /// The longest line in characters, from the last parse: the width to scroll to when word wrap is off.
    longest_line: usize,
    parse_task: Option<Task<()>>,
    last_parse_ms: f64,
    /// The last edit the preview has not caught up with; logged as `preview_ms` when it does.
    edited_at: Option<Instant>,
    /// One item per source line.
    lines: ListState,
    /// One item per preview block.
    block_list: ListState,
    followed_cursor_line: Option<usize>,
    synced_source_line: Option<usize>,
    /// Words and UTF-16 characters, counted with each background parse, for the status bar.
    counts: Counts,
    /// The last update's time from keystroke to preview, and its parse time, for the status bar.
    last_preview: Option<(f64, f64)>,
    /// Dropping this stops watching the file.
    watcher: Option<notify::RecommendedWatcher>,
    /// Replaced by every watcher event, so a burst checks the file once after it settles.
    external_task: Option<Task<()>>,
    /// Replaced by every edit: auto-saves once typing stops.
    auto_save_task: Option<Task<()>>,
    /// The save being written on a background thread, and the target of one asked for meanwhile.
    save_task: Option<Task<()>>,
    queued_save: Option<PathBuf>,
}

/// What a background parse hands back to the window. Everything that costs time in proportion to
/// the document is done on the parse thread, so applying it stays cheap at any size (at 8 MB the
/// comparison alone took 55–67 ms on the main thread).
struct Parsed {
    blocks: Vec<Block>,
    spans: Vec<Vec<Span>>,
    longest: usize,
    counts: Counts,
    /// The blocks this parse was compared against, and what changed: the range of old blocks to
    /// replace and how many new ones replace them.
    base: Arc<Vec<Block>>,
    changed: (Range<usize>, usize),
    images: ImagesInUse,
    links: Vec<(Arc<str>, usize)>,
    headings: Vec<Heading>,
}

/// The formula and diagram sources a document shows, which keep their bitmaps alive.
#[derive(Default)]
struct ImagesInUse {
    diagrams: HashSet<String>,
    /// TeX and whether it is display math.
    formulas: HashSet<(String, bool)>,
}

impl ImagesInUse {
    fn of(blocks: &[Block]) -> Self {
        let mut images = Self::default();
        for block in blocks {
            match block.kind {
                Kind::Diagram => {
                    images.diagrams.insert(block.text.clone());
                }
                Kind::Math => {
                    images.formulas.insert((block.text.clone(), true));
                }
                _ => {}
            }
            let cells = block.table.iter().flat_map(|table| table.rows.iter().flatten());
            for run in block.runs.iter().chain(cells.flatten()).filter(|run| run.style.math) {
                images.formulas.insert((run.text.clone(), run.style.display));
            }
        }
        images
    }
}

/// One recovery write: the held snapshots and each dirty document as a rope, which is cheap to
/// copy on the main thread and is turned into text on the thread that writes it.
struct RecoveryWrite {
    held: Vec<store::Snapshot>,
    documents: Vec<(Option<String>, ropey::Rope, FileMeta)>,
}

impl RecoveryWrite {
    /// Saves the snapshots, or clears the file when nothing is left to recover.
    fn write(self, dir: &Path) -> std::io::Result<()> {
        let mut snapshots = self.held;
        snapshots.extend(self.documents.into_iter().map(|(path, rope, meta)| store::Snapshot { path, text: rope.to_string(), meta }));
        if snapshots.is_empty() { store::clear_snapshots(dir) } else { store::save_snapshots(dir, &snapshots) }
    }
}

/// The status bar's counts: words as the app counts them (`/\S+/g`), characters as JavaScript's `length`.
#[derive(Clone, Copy, Default)]
struct Counts {
    words: usize,
    chars: usize,
}

impl Counts {
    fn of(text: &str) -> Self {
        Self { words: text.split_whitespace().count(), chars: text.encode_utf16().count() }
    }
}

impl Document {
    fn open(opened: OpenedFile, id: u64, cx: &mut Context<Prototype>) -> Self {
        let buffer = Buffer::new(&opened.text);
        let lines = ListState::new(buffer.line_count(), ListAlignment::Top, px(OVERDRAW));
        // The preview follows the source pane, so scrolling the source re-renders the view.
        lines.set_scroll_handler(cx.listener(|_, _: &ListScrollEvent, _, cx| cx.notify()));
        Self {
            id,
            path: opened.path,
            meta: opened.meta,
            mtime_ms: opened.mtime_ms,
            read_only: opened.read_only,
            restored: false,
            buffer,
            selected: 0..0,
            selection_reversed: false,
            marked: None,
            blocks: Arc::default(),
            images: Arc::default(),
            links: Vec::new(),
            headings: Vec::new(),
            spans: Vec::new(),
            longest_line: 0,
            parse_task: None,
            last_parse_ms: 0.,
            edited_at: None,
            lines,
            block_list: ListState::new(0, ListAlignment::Top, px(OVERDRAW)),
            followed_cursor_line: None,
            synced_source_line: None,
            counts: Counts::default(),
            last_preview: None,
            watcher: None,
            external_task: None,
            auto_save_task: None,
            save_task: None,
            queued_save: None,
        }
    }

    fn is_dirty(&self) -> bool {
        self.restored || self.buffer.is_dirty()
    }

    fn display_name(&self) -> String {
        if self.path.is_empty() { "Untitled".to_string() } else { file_name(&self.path) }
    }

    fn cursor(&self) -> usize {
        if self.selection_reversed { self.selected.start } else { self.selected.end }
    }

    /// Keeps the line list and the syntax colours in step with an edit. The edited lines keep their
    /// old colours until the next parse (at most 150 ms after typing stops), so a heading being typed
    /// does not flicker plain; lines the edit added start plain.
    fn splice_lines(&mut self, splice: LineSplice) {
        self.lines.splice(splice.first..splice.first + splice.old, splice.new);
        if splice.first >= self.spans.len() {
            return;
        }
        let kept = splice.old.min(splice.new);
        let old_end = (splice.first + splice.old).min(self.spans.len());
        let replace = (splice.first + kept).min(old_end)..old_end;
        self.spans.splice(replace, std::iter::repeat_with(Vec::new).take(splice.new - kept));
    }
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
    /// The left button went down in the source and has not come up: moves extend the selection.
    dragging: bool,
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
    /// Crash-recovery snapshots, and the snapshots held until the user restores or discards them.
    recovery_dir: PathBuf,
    held_recovery: Vec<store::Snapshot>,
    recovery_task: Option<Task<()>>,
    /// Document ids and buffer versions in the last recovery write, so an unchanged dirty document
    /// is not serialised again every 30 seconds. `recovery_dirty` covers held snapshots changing.
    recovery_versions: Vec<(u64, u64)>,
    recovery_dirty: bool,
    /// Watches `open-request.txt`, written by a second launch.
    request_watcher: Option<notify::RecommendedWatcher>,
    /// The single-instance lock; held for the window's lifetime.
    _instance_lock: Option<std::fs::File>,
    /// Esc was pressed in the editor: the next Tab moves the focus instead of indenting, as with
    /// CodeMirror's `indentWithTab`, so a keyboard user can always leave the editor.
    tab_leaves: bool,
    /// Where the pointer is while a drag selection is outside the source pane, and the task that
    /// keeps scrolling towards it.
    drag_outside: Option<Point<Pixels>>,
    drag_scroll: Option<Task<()>>,
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
            dragging: false,
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
            recovery_dir,
            held_recovery: Vec::new(),
            recovery_task: None,
            recovery_versions: Vec::new(),
            recovery_dirty: true,
            request_watcher: None,
            _instance_lock: instance_lock,
            tab_leaves: false,
            drag_outside: None,
            drag_scroll: None,
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

    fn dirty_count(&self) -> usize {
        self.tabs.dirty_count()
    }

    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() == 1 || index >= self.tabs.len() {
            return;
        }
        if !self.tabs.documents()[index].is_dirty() {
            self.remove_tab(index, cx);
            return;
        }
        // Tabs can move while the prompt is open, so the answer finds the tab again by id.
        let id = self.tabs.documents()[index].id;
        let message = format!("Discard unsaved changes in {}?", self.tabs.documents()[index].display_name());
        self.confirm_discard(&message, window, cx, move |this, _, cx| {
            if let Some(index) = this.document_index(id) {
                this.remove_tab(index, cx);
            }
        });
    }

    fn remove_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.tabs.remove(index) {
            return;
        }
        self.doc_mut().synced_source_line = None;
        self.release_unused_images(cx);
        cx.notify();
    }

    /// Asks before unsaved edits are thrown away. The prompt is asynchronous, so `then` runs later, and only on Discard.
    fn confirm_discard(
        &mut self,
        message: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let answer = window.prompt(PromptLevel::Warning, message, Some("Unsaved changes will be lost."), &["Discard", "Cancel"], cx);
        cx.spawn_in(window, async move |this, cx| {
            if !matches!(answer.await, Ok(0)) {
                return;
            }
            let _ = this.update_in(cx, |this, window, cx| then(this, window, cx));
        })
        .detach();
    }

    /// The title-bar X and Quit both come here. Returns whether the window may close now.
    pub fn may_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.tabs.iter().any(|doc| doc.save_task.is_some()) {
            self.close_after_saves(window, cx);
            return false;
        }
        let dirty = self.dirty_count();
        if dirty == 0 || self.close_confirmed {
            // A postponed recovery survives a normal close; with nothing left to recover, this clears it.
            self.flush_recovery();
            return true;
        }
        let message = if dirty == 1 { "Discard unsaved changes in 1 file?".to_string() } else { format!("Discard unsaved changes in {dirty} files?") };
        self.confirm_discard(&message, window, cx, |this, window, _| {
            // Discarding everything: nothing is left to recover, and held snapshots stay held.
            for document in this.tabs.documents_mut() {
                document.restored = false;
                document.buffer.mark_saved();
            }
            this.close_confirmed = true;
            this.flush_recovery();
            window.remove_window();
        });
        false
    }

    /// A close while a save is being written waits for it and then asks again, so the file is
    /// complete and the unsaved-changes prompt sees what was saved. Saves take milliseconds.
    fn close_after_saves(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_task.is_some() {
            return;
        }
        self.close_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(15)).await;
                let Ok(saving) = this.update(cx, |this, _| this.tabs.documents().iter().any(|doc| doc.save_task.is_some())) else { return };
                if !saving {
                    break;
                }
            }
            let _ = this.update_in(cx, |this, window, cx| {
                this.close_task = None;
                if this.may_close(window, cx) {
                    window.remove_window();
                }
            });
        }));
    }

    fn quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        if self.may_close(window, cx) {
            cx.quit();
        }
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


impl EntityInputHandler for Prototype {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let buffer = &self.doc().buffer;
        let range = buffer.offset_of_utf16(range_utf16.start)..buffer.offset_of_utf16(range_utf16.end);
        *actual_range = Some(buffer.utf16_of(range.start)..buffer.utf16_of(range.end));
        Some(buffer.slice(range))
    }

    fn selected_text_range(&mut self, _ignore_disabled_input: bool, _window: &mut Window, _cx: &mut Context<Self>) -> Option<UTF16Selection> {
        let doc = self.doc();
        Some(UTF16Selection {
            range: doc.buffer.utf16_of(doc.selected.start)..doc.buffer.utf16_of(doc.selected.end),
            reversed: doc.selection_reversed,
        })
    }

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        let doc = self.doc();
        doc.marked.as_ref().map(|range| doc.buffer.utf16_of(range.start)..doc.buffer.utf16_of(range.end))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.doc_mut().marked = None;
    }

    /// Typed text and the end of an IME composition. Typing at the caret groups into one undo step.
    fn replace_text_in_range(&mut self, range_utf16: Option<Range<usize>>, new_text: &str, _window: &mut Window, cx: &mut Context<Self>) {
        let range = self.edit_range(range_utf16);
        let kind = if range.is_empty() || self.doc().marked.as_ref() == Some(&range) { EditKind::Typing } else { EditKind::Other };
        self.edit(range, new_text, kind, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.edit_range(range_utf16);
        let replaced_at = range.start;
        self.edit(range, new_text, EditKind::Typing, cx);
        let doc = self.doc_mut();
        let end = replaced_at + new_text.len();
        doc.marked = (!new_text.is_empty()).then_some(replaced_at..end);
        if let Some(selected) = new_selected_utf16 {
            let start = replaced_at + utf16_to_byte(new_text, selected.start);
            doc.selected = start..replaced_at + utf16_to_byte(new_text, selected.end);
        }
    }

    /// The IME asks where to put its candidate window. The caret painted last frame is the composition point;
    /// its bounds already include scrolling, padding and the column.
    fn bounds_for_range(&mut self, _range_utf16: Range<usize>, element_bounds: Bounds<Pixels>, window: &mut Window, _cx: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        Some(self.caret_bounds.unwrap_or_else(|| Bounds::new(element_bounds.origin, size(px(1.5), window.line_height()))))
    }

    /// The character under a point, in UTF-16 units, for IME reconversion by mouse. Only lines painted
    /// in the last frame can answer. (GPUI's Windows backend does not yet ask: it handles no
    /// `WM_IME_REQUEST`, so Windows' reconversion key cannot reach any GPUI view, at 9a8ab54 either.)
    fn character_index_for_point(&mut self, position: Point<Pixels>, _window: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        let line = self.visible_line_at(position.y)?;
        let layout = self.painted_layout(line)?;
        let offset = self.offset_at(line, layout, position);
        Some(self.doc().buffer.utf16_of(offset))
    }
}

impl Prototype {
    fn edit_range(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        let doc = self.doc();
        if let Some(range) = range_utf16 {
            return doc.buffer.offset_of_utf16(range.start)..doc.buffer.offset_of_utf16(range.end);
        }
        doc.marked.clone().unwrap_or_else(|| doc.selected.clone())
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
                    this.dragging = false;
                    this.drag_outside = None;
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

/// A heading for the outline: its 1-based source line, level and text.
struct Heading {
    line: usize,
    level: u8,
    text: SharedString,
}

fn headings_of(blocks: &[Block]) -> Vec<Heading> {
    blocks
        .iter()
        .filter_map(|block| match block.kind {
            Kind::Heading(level) => Some(Heading { line: block.line, level, text: SharedString::from(block.text.clone()) }),
            _ => None,
        })
        .collect()
}

/// Every link in `blocks`, images aside, in document order, with the index of its block.
fn links_of(blocks: &[Block]) -> Vec<(Arc<str>, usize)> {
    let mut links = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        let cells = block.table.iter().flat_map(|table| table.rows.iter().flatten().flatten());
        for run in block.runs.iter().chain(cells) {
            let Some(link) = run.style.link.as_ref() else { continue };
            if !link.starts_with("image:") {
                links.push((link.clone(), index));
            }
        }
    }
    links
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

fn utf16_to_byte(text: &str, utf16_offset: usize) -> usize {
    let mut utf16 = 0;
    for (index, ch) in text.char_indices() {
        if utf16 >= utf16_offset {
            return index;
        }
        utf16 += ch.len_utf16();
    }
    text.len()
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
    use crate::preview;

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

    #[test]
    fn headings_keep_their_line_level_and_text() {
        let blocks = preview::blocks("# One\n\nText.\n\n### Three\n");
        let headings = headings_of(&blocks);
        assert_eq!(headings.len(), 2);
        assert_eq!((headings[0].line, headings[0].level), (1, 1));
        assert_eq!(headings[0].text.as_ref(), "One");
        assert_eq!((headings[1].line, headings[1].level), (5, 3));
    }

    /// Image runs carry an `image:` destination so the preview can draw them; the arrow keys walk
    /// links, and an image is not one.
    #[test]
    fn links_skip_images_and_keep_document_order() {
        let blocks = preview::blocks("See [a](one.md) and ![pic](x.png).\n\nThen [b](two.md).\n");
        let links = links_of(&blocks);
        assert_eq!(links.iter().map(|(link, _)| link.as_ref()).collect::<Vec<_>>(), ["one.md", "two.md"]);
        assert_eq!(links.iter().map(|(_, block)| *block).collect::<Vec<_>>(), [0, 1]);
    }

    #[test]
    fn links_in_table_cells_are_found() {
        let blocks = preview::blocks("| Col |\n| --- |\n| [a](cell.md) |\n");
        assert_eq!(links_of(&blocks).iter().map(|(link, _)| link.as_ref()).collect::<Vec<_>>(), ["cell.md"]);
    }

    #[test]
    fn file_name_is_the_last_component() {
        assert_eq!(file_name(r"C:\docs\notes.md"), "notes.md");
        assert_eq!(file_name("/home/a/b.txt"), "b.txt");
        assert_eq!(file_name("bare.md"), "bare.md");
    }

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

    /// Freeing a bitmap needs to know every formula and diagram the open documents still show.
    #[test]
    fn images_in_use_gathers_formulas_and_diagrams() {
        let blocks = preview::blocks("```math\nE=mc^2\n```\n\n```mermaid\nflowchart LR\n  A-->B\n```\n\nInline $x+1$ and `$not math$`.\n");
        let images = ImagesInUse::of(&blocks);
        assert!(images.formulas.contains(&("E=mc^2".to_string(), true)));
        assert!(images.formulas.contains(&("x+1".to_string(), false)));
        assert_eq!(images.diagrams.len(), 1);
        // Inline code is not a formula.
        assert!(!images.formulas.iter().any(|(tex, _)| tex.contains("not math")));
    }

    #[test]
    fn images_in_use_finds_formulas_in_table_cells() {
        let blocks = preview::blocks("| Math |\n| --- |\n| $y$ |\n");
        assert!(ImagesInUse::of(&blocks).formulas.contains(&("y".to_string(), false)));
    }

    /// The status bar reports words as `/\S+/g` and characters as JavaScript's `length`, both of
    /// which the app did in JavaScript.
    #[test]
    fn counts_are_words_and_utf16_characters() {
        let counts = Counts::of("one two  three\n");
        assert_eq!(counts.words, 3);
        assert_eq!(counts.chars, 15);
        // An astral character counts as two, as JavaScript's length does.
        assert_eq!(Counts::of("😀").chars, 2);
        assert_eq!(Counts::of("").words, 0);
    }

    /// A `Block` is the only input these helpers need; make sure they tolerate the shapes a parse
    /// can produce (no blocks at all, and a block with no runs).
    #[test]
    fn helpers_tolerate_empty_blocks() {
        let blocks: Vec<Block> = Vec::new();
        assert!(headings_of(&blocks).is_empty());
        assert!(links_of(&blocks).is_empty());
        assert!(ImagesInUse::of(&blocks).formulas.is_empty());
        let no_runs = Block { kind: Kind::Paragraph, text: String::new(), runs: Vec::new(), line: 1, table: None, code: None, src: None };
        assert!(links_of(std::slice::from_ref(&no_runs)).is_empty());
    }
}
