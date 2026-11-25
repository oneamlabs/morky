//! The open tabs: the documents, which one is active, and the id the next one gets.
//!
//! Kept apart from `Prototype` so the bookkeeping that is easy to get wrong — which tab is active
//! after one closes, and finding a tab again by id after the list has moved — can be tested without
//! a window.

use super::*;

#[derive(Default)]
pub(super) struct Tabs {
    documents: Vec<Document>,
    active: usize,
    next_id: u64,
}

impl Tabs {
    pub(super) fn new(first: Document) -> Self {
        Self { documents: vec![first], active: 0, next_id: 1 }
    }

    pub(super) fn documents(&self) -> &[Document] {
        &self.documents
    }

    pub(super) fn documents_mut(&mut self) -> &mut [Document] {
        &mut self.documents
    }

    pub(super) fn len(&self) -> usize {
        self.documents.len()
    }

    pub(super) fn active(&self) -> usize {
        self.active
    }

    /// The active document. There is always at least one tab: the view never closes the last one.
    pub(super) fn active_doc(&self) -> &Document {
        &self.documents[self.active]
    }

    pub(super) fn active_doc_mut(&mut self) -> &mut Document {
        &mut self.documents[self.active]
    }

    /// Switches to `index`, if it is a tab. Returns whether it was.
    pub(super) fn activate(&mut self, index: usize) -> bool {
        if index >= self.documents.len() {
            return false;
        }
        self.active = index;
        true
    }

    /// The next tab, wrapping. `None` only when there are no tabs.
    pub(super) fn next_index(&self) -> Option<usize> {
        (!self.documents.is_empty()).then(|| (self.active + 1) % self.documents.len())
    }

    /// The previous tab, wrapping. `None` only when there are no tabs.
    pub(super) fn previous_index(&self) -> Option<usize> {
        (!self.documents.is_empty()).then(|| (self.active + self.documents.len() - 1) % self.documents.len())
    }

    /// Takes a new id for a document being opened or created.
    pub(super) fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub(super) fn push(&mut self, document: Document) {
        self.documents.push(document);
    }

    /// Where the document with `id` is now. Tabs can move while a prompt is open, so answers find
    /// their tab again by id rather than by index.
    pub(super) fn index_of(&self, id: u64) -> Option<usize> {
        self.documents.iter().position(|doc| doc.id == id)
    }

    pub(super) fn iter(&self) -> std::slice::Iter<'_, Document> {
        self.documents.iter()
    }

    pub(super) fn iter_mut(&mut self) -> std::slice::IterMut<'_, Document> {
        self.documents.iter_mut()
    }

    /// Removes the tab at `index` and keeps the active one sensible: the last tab cannot be closed,
    /// and closing the active tab or one before it moves back one place.
    pub(super) fn remove(&mut self, index: usize) -> bool {
        if self.documents.len() == 1 || index >= self.documents.len() {
            return false;
        }
        self.documents.remove(index);
        if self.active > index || self.active == self.documents.len() {
            self.active -= 1;
        }
        true
    }

    pub(super) fn dirty_count(&self) -> usize {
        self.documents.iter().filter(|doc| doc.is_dirty()).count()
    }

    /// Document ids and buffer versions of every dirty tab: everything a recovery snapshot would
    /// contain, in the order it would contain it.
    pub(super) fn dirty_versions(&self) -> Vec<(u64, u64)> {
        self.documents.iter().filter(|doc| doc.is_dirty()).map(|doc| (doc.id, doc.buffer.version())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tab with an id, opened from no file. Enough to exercise the bookkeeping.
    fn doc(id: u64) -> Document {
        Document {
            id,
            path: String::new(),
            meta: FileMeta { bom: false, line_ending: files::LineEnding::Lf },
            mtime_ms: 0,
            read_only: false,
            restored: false,
            buffer: Buffer::new(""),
            selected: 0..0,
            selection_reversed: false,
            marked: None,
            blocks: Arc::default(),
            images: Arc::default(),
            links: Vec::new(),
            headings: Vec::new(),
            outline: ListState::new(0, ListAlignment::Top, px(400.)),
            spans: Vec::new(),
            longest_line: 0,
            parse_task: None,
            last_parse_ms: 0.,
            edited_at: None,
            lines: ListState::new(1, ListAlignment::Top, px(400.)),
            block_list: ListState::new(0, ListAlignment::Top, px(400.)),
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

    fn tabs(count: u64) -> Tabs {
        let mut tabs = Tabs::new(doc(0));
        for id in 1..count {
            tabs.push(doc(id));
        }
        tabs
    }

    #[test]
    fn the_first_tab_is_active_and_ids_come_in_order() {
        let mut tabs = Tabs::new(doc(0));
        assert_eq!(tabs.active(), 0);
        assert_eq!(tabs.take_id(), 1);
        assert_eq!(tabs.take_id(), 2);
    }

    #[test]
    fn activating_a_missing_tab_changes_nothing() {
        let mut tabs = tabs(3);
        assert!(tabs.activate(2));
        assert_eq!(tabs.active(), 2);
        assert!(!tabs.activate(3));
        assert_eq!(tabs.active(), 2);
    }

    #[test]
    fn next_and_previous_wrap() {
        let mut tabs = tabs(3);
        assert_eq!(tabs.next_index(), Some(1));
        assert_eq!(tabs.previous_index(), Some(2));
        tabs.activate(2);
        assert_eq!(tabs.next_index(), Some(0));
        assert_eq!(tabs.previous_index(), Some(1));
    }

    /// The last tab cannot be closed: the view always has one document to show.
    #[test]
    fn the_last_tab_cannot_be_removed() {
        let mut tabs = tabs(1);
        assert!(!tabs.remove(0));
        assert_eq!(tabs.len(), 1);
    }

    /// Closing the active tab keeps its index, so the tab that followed it becomes active.
    #[test]
    fn closing_the_active_tab_selects_the_one_that_followed() {
        let mut tabs = tabs(3);
        tabs.activate(1);
        assert!(tabs.remove(1));
        assert_eq!(tabs.active(), 1);
        assert_eq!(tabs.len(), 2);
    }

    /// Closing a tab after the active one leaves the active index alone.
    #[test]
    fn closing_a_later_tab_keeps_the_active_one() {
        let mut tabs = tabs(3);
        tabs.activate(0);
        assert!(tabs.remove(2));
        assert_eq!(tabs.active(), 0);
    }

    /// Closing the last tab while it is active moves back; closing a tab before the active one
    /// shifts it down so the same document stays active.
    #[test]
    fn closing_a_tab_before_the_active_one_follows_the_same_document() {
        let mut tabs = tabs(4);
        tabs.activate(3);
        let active_id = tabs.active_doc().id;
        assert!(tabs.remove(1));
        assert_eq!(tabs.active(), 2);
        assert_eq!(tabs.active_doc().id, active_id);
    }

    #[test]
    fn closing_the_final_tab_while_it_is_active_moves_back() {
        let mut tabs = tabs(3);
        tabs.activate(2);
        assert!(tabs.remove(2));
        assert_eq!(tabs.active(), 1);
    }

    #[test]
    fn a_tab_is_found_by_id_after_the_list_moves() {
        let mut tabs = tabs(3);
        assert_eq!(tabs.index_of(2), Some(2));
        tabs.remove(0);
        assert_eq!(tabs.index_of(2), Some(1));
        assert_eq!(tabs.index_of(99), None);
    }
}
