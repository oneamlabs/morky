//! One open document: its buffer, selection, the preview it was last parsed into, and the tasks
//! watching and saving it. `Tabs` owns a list of these; `Prototype` draws the active one.

use super::*;

pub struct Document {
    /// Finds the document again when a background parse finishes, even if tabs moved meanwhile.
    pub(super) id: u64,
    /// Empty for a New Tab document until it is saved.
    pub(super) path: String,
    pub(super) meta: FileMeta,
    /// From the last read or save; a watcher event is ours when the file still has this time.
    pub(super) mtime_ms: u64,
    /// Lossy decode. Saving would corrupt the file, so Save refuses it.
    pub(super) read_only: bool,
    /// A recovered document's text is unsaved even though its undo history is empty.
    pub(super) restored: bool,
    pub(super) buffer: Buffer,
    pub(super) selected: Range<usize>,
    /// The caret is at `selected.start` when the selection was made backwards.
    pub(super) selection_reversed: bool,
    pub(super) marked: Option<Range<usize>>,
    /// Shared with the next background parse, which compares its result against them.
    pub(super) blocks: Arc<Vec<Block>>,
    /// The formulas and diagrams in `blocks`, gathered by the parse: freeing unused images reads these.
    pub(super) images: Arc<ImagesInUse>,
    /// The outline's headings, gathered from `blocks` by the parse, so drawing a frame does not
    /// walk every block. The preview keeps its own link list.
    pub(super) headings: Vec<Heading>,
    /// Syntax colours per source line, from the last background parse.
    pub(super) spans: Vec<Vec<Span>>,
    /// The longest line in characters, from the last parse: the width to scroll to when word wrap is off.
    pub(super) longest_line: usize,
    pub(super) parse_task: Option<Task<()>>,
    pub(super) last_parse_ms: f64,
    /// The last edit the preview has not caught up with; logged as `preview_ms` when it does.
    pub(super) edited_at: Option<Instant>,
    /// One item per source line.
    pub(super) lines: ListState,
    pub(super) followed_cursor_line: Option<usize>,
    pub(super) synced_source_line: Option<usize>,
    /// Words and UTF-16 characters, counted with each background parse, for the status bar.
    pub(super) counts: Counts,
    /// Dropping this stops watching the file.
    pub(super) watcher: Option<notify::RecommendedWatcher>,
    /// Replaced by every watcher event, so a burst checks the file once after it settles.
    pub(super) external_task: Option<Task<()>>,
    /// Replaced by every edit: auto-saves once typing stops.
    pub(super) auto_save_task: Option<Task<()>>,
    /// The save being written on a background thread, and the target of one asked for meanwhile.
    pub(super) save_task: Option<Task<()>>,
    pub(super) queued_save: Option<PathBuf>,
}

/// A heading for the outline: its 1-based source line, level and text.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Heading {
    pub(super) line: usize,
    pub(super) level: u8,
    pub(super) text: SharedString,
}

/// The formula and diagram sources a document shows, which keep their bitmaps alive.
#[derive(Default)]
pub(super) struct ImagesInUse {
    pub(super) diagrams: HashSet<String>,
    /// TeX and whether it is display math.
    pub(super) formulas: HashSet<(String, bool)>,
}

/// The status bar's counts: words as the app counts them (`/\S+/g`), characters as JavaScript's `length`.
#[derive(Clone, Copy, Default)]
pub(super) struct Counts {
    pub(super) words: usize,
    pub(super) chars: usize,
}

/// What a background parse hands back to the window. Everything that costs time in proportion to
/// the document is done on the parse thread, so applying it stays cheap at any size (at 8 MB the
/// comparison alone took 55–67 ms on the main thread).
pub(super) struct Parsed {
    pub(super) blocks: Vec<Block>,
    pub(super) spans: Vec<Vec<Span>>,
    pub(super) longest: usize,
    pub(super) counts: Counts,
    /// The blocks this parse was compared against, and what changed: the range of old blocks to
    /// replace and how many new ones replace them.
    pub(super) base: Arc<Vec<Block>>,
    pub(super) changed: (Range<usize>, usize),
    pub(super) images: ImagesInUse,
    pub(super) links: Vec<(Arc<str>, usize)>,
    pub(super) headings: Vec<Heading>,
}

impl ImagesInUse {
    pub(super) fn of(blocks: &[Block]) -> Self {
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

impl Counts {
    pub(super) fn of(text: &str) -> Self {
        Self { words: text.split_whitespace().count(), chars: text.encode_utf16().count() }
    }
}

pub(super) fn headings_of(blocks: &[Block]) -> Vec<Heading> {
    blocks
        .iter()
        .filter_map(|block| match block.kind {
            Kind::Heading(level) => Some(Heading { line: block.line, level, text: SharedString::from(block.text.clone()) }),
            _ => None,
        })
        .collect()
}

/// Every link in `blocks`, images aside, in document order, with the index of its block.
pub(super) fn links_of(blocks: &[Block]) -> Vec<(Arc<str>, usize)> {
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

pub(super) fn file_name(path: &str) -> String {
    Path::new(path).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

impl Document {
    pub(super) fn open(opened: OpenedFile, id: u64, cx: &mut Context<Prototype>) -> Self {
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
            headings: Vec::new(),
            spans: Vec::new(),
            longest_line: 0,
            parse_task: None,
            last_parse_ms: 0.,
            edited_at: None,
            lines,
            followed_cursor_line: None,
            synced_source_line: None,
            counts: Counts::default(),
            watcher: None,
            external_task: None,
            auto_save_task: None,
            save_task: None,
            queued_save: None,
        }
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.restored || self.buffer.is_dirty()
    }

    pub(super) fn display_name(&self) -> String {
        if self.path.is_empty() { "Untitled".to_string() } else { file_name(&self.path) }
    }

    pub(super) fn cursor(&self) -> usize {
        if self.selection_reversed { self.selected.start } else { self.selected.end }
    }

    /// Keeps the line list and the syntax colours in step with an edit. The edited lines keep their
    /// old colours until the next parse (at most 150 ms after typing stops), so a heading being typed
    /// does not flicker plain; lines the edit added start plain.
    pub(super) fn splice_lines(&mut self, splice: LineSplice) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_words_and_utf16_characters() {
        let counts = Counts::of("one two  three\n");
        assert_eq!(counts.words, 3);
        assert_eq!(counts.chars, 15);
        // An astral character counts as two, as JavaScript's length does.
        assert_eq!(Counts::of("😀").chars, 2);
        assert_eq!(Counts::of("").words, 0);
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

    /// A `Block` is the only input these helpers need; make sure they tolerate the shapes a parse
    /// can produce (no blocks at all, and a block with no runs).
    #[test]
    fn helpers_tolerate_empty_blocks() {
        let blocks: Vec<Block> = Vec::new();
        assert!(headings_of(&blocks).is_empty());
        assert!(links_of(&blocks).is_empty());
        assert!(ImagesInUse::of(&blocks).formulas.is_empty());
        let no_runs = Block { kind: Kind::Paragraph, text: String::new(), runs: Vec::new(), line: 1, table: None, code: None, src: None, footnote: None };
        assert!(links_of(std::slice::from_ref(&no_runs)).is_empty());
    }
}
