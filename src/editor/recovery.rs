//! Crash-recovery snapshots: the restore/discard prompt at startup, the 30 s timer, and the rule
//! that an unchanged dirty document is not serialised again.
//!
//! `RecoveryWrite` is built on the main thread from cheap rope snapshots and turned into text on the
//! thread that writes it, so the timer never copies a large document on the UI thread.

use super::*;

/// The recovery state: where snapshots live, any held until the user answers, and what the last
/// write contained so an unchanged document is not serialised again.
pub(super) struct Recovery {
    dir: PathBuf,
    /// Snapshots held because the user chose Cancel; written back with the next snapshot and on close.
    held: Vec<store::Snapshot>,
    task: Option<Task<()>>,
    /// Document ids and buffer versions in the last write.
    versions: Vec<(u64, u64)>,
    /// Whether the held snapshots changed since the last write; covers `versions` alone missing it.
    dirty: bool,
}

impl Recovery {
    pub(super) fn new(dir: PathBuf) -> Self {
        // `dirty` starts true so the first tick writes even when nothing is dirty yet.
        Self { dir, held: Vec::new(), task: None, versions: Vec::new(), dirty: true }
    }

    /// Holds `snapshots` until the user answers; they go out with the next write.
    pub(super) fn hold(&mut self, snapshots: Vec<store::Snapshot>) {
        self.held = snapshots;
        self.dirty = true;
    }

    /// Records the task writing snapshots, so it lives as long as the window does.
    pub(super) fn set_task(&mut self, task: Task<()>) {
        self.task = Some(task);
    }

    /// What the next write holds, or `None` when no buffer version and no held snapshot changed
    /// since the last one: serialising every dirty document is itself a cost at 8 MB.
    pub(super) fn take_changes(&mut self, versions: Vec<(u64, u64)>, documents: Vec<(Option<String>, ropey::Rope, FileMeta)>) -> Option<RecoveryWrite> {
        if !self.dirty && versions == self.versions {
            return None;
        }
        self.versions = versions;
        self.dirty = false;
        Some(RecoveryWrite { held: self.held.clone(), documents })
    }

    pub(super) fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Prototype {
    /// The app's recovery prompt. Yes restores the tabs, No deletes the snapshots (an explicit
    /// Discard), and Cancel holds them: they are written back with the next snapshot and on close.
    pub(super) fn restore_or_discard(&mut self, recovery: Vec<store::Snapshot>, cx: &mut Context<Self>) {
        if recovery.is_empty() {
            return;
        }
        let names: Vec<String> = recovery
            .iter()
            .map(|snapshot| snapshot.path.as_deref().map(file_name).unwrap_or_else(|| "Untitled".to_string()))
            .collect();
        let answer = rfd::MessageDialog::new()
            .set_title("Marky")
            .set_level(rfd::MessageLevel::Warning)
            .set_description(format!("Restore unsaved changes to {}?", names.join(", ")))
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .show();
        match answer {
            rfd::MessageDialogResult::Yes => {
                for snapshot in recovery {
                    let path = snapshot.path.unwrap_or_default();
                    let mtime_ms = if path.is_empty() { 0 } else { files::mtime_ms(Path::new(&path)).unwrap_or(0) };
                    let opened = OpenedFile { path: path.clone(), text: snapshot.text, meta: snapshot.meta, mtime_ms, read_only: false };
                    // The startup file may already be open; its recovered text replaces what was read.
                    if !path.is_empty() {
                        if let Some(index) = self.tabs.iter().position(|doc| files::same_path(Path::new(&doc.path), Path::new(&path))) {
                            let id = self.tabs.documents_mut()[index].id;
                            let mut document = Document::open(opened, id, cx);
                            document.restored = true;
                            self.tabs.documents_mut()[index] = document;
                            self.schedule_parse(id, Duration::ZERO, cx);
                            self.watch_document(id, cx);
                            continue;
                        }
                    }
                    let id = self.tabs.take_id();
                    let mut document = Document::open(opened, id, cx);
                    document.restored = true;
                    self.tabs.push(document);
                    self.schedule_parse(id, Duration::ZERO, cx);
                    self.watch_document(id, cx);
                }
                self.activate_tab(self.tabs.len() - 1, cx);
            }
            rfd::MessageDialogResult::No => {
                if let Err(err) = store::clear_snapshots(self.recovery.dir()) {
                    diagnostics::write(&format!("could not clear recovery snapshots: {err}"));
                }
            }
            _ => self.recovery.hold(recovery),
        }
    }

    pub(super) fn start_recovery_timer(&mut self, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(30)).await;
            if this.update(cx, |this, cx| this.write_recovery(cx)).is_err() {
                break;
            }
        });
        self.recovery.set_task(task);
    }

    /// The dirty documents as cheap rope snapshots, for the next write.
    fn recovery_documents(&self) -> Vec<(Option<String>, ropey::Rope, FileMeta)> {
        self.tabs
            .iter()
            .filter(|doc| doc.is_dirty())
            .map(|doc| ((!doc.path.is_empty()).then(|| doc.path.clone()), doc.buffer.snapshot(), doc.meta))
            .collect()
    }

    fn take_recovery_changes(&mut self) -> Option<RecoveryWrite> {
        let (versions, documents) = (self.tabs.dirty_versions(), self.recovery_documents());
        self.recovery.take_changes(versions, documents)
    }

    fn write_recovery(&mut self, cx: &mut Context<Self>) {
        let Some(changes) = self.take_recovery_changes() else { return };
        let dir = self.recovery.dir().to_path_buf();
        cx.background_spawn(async move {
            if let Err(err) = changes.write(&dir) {
                diagnostics::write(&format!("could not save recovery snapshots: {err}"));
            }
        })
        .detach();
    }

    /// Synchronous, for closing: a background write could be lost when the process ends.
    pub(super) fn flush_recovery(&mut self) {
        let Some(changes) = self.take_recovery_changes() else { return };
        if let Err(err) = changes.write(self.recovery.dir()) {
            diagnostics::write(&format!("could not save recovery snapshots on close: {err}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(path: Option<&str>) -> store::Snapshot {
        store::Snapshot { path: path.map(Into::into), text: "x".into(), meta: FileMeta { bom: false, line_ending: files::LineEnding::Lf } }
    }

    /// The first write always happens: a launch with nothing dirty still clears a stale file.
    #[test]
    fn the_first_write_always_happens() {
        let mut recovery = Recovery::new(PathBuf::from("."));
        assert!(recovery.take_changes(Vec::new(), Vec::new()).is_some());
    }

    /// A second write with nothing changed is skipped: this is the 30 s timer's whole point.
    #[test]
    fn an_unchanged_write_is_skipped() {
        let mut recovery = Recovery::new(PathBuf::from("."));
        assert!(recovery.take_changes(vec![(1, 5)], Vec::new()).is_some());
        assert!(recovery.take_changes(vec![(1, 5)], Vec::new()).is_none());
        // A new buffer version means something changed.
        assert!(recovery.take_changes(vec![(1, 6)], Vec::new()).is_some());
    }

    /// Holding snapshots counts as a change even when no buffer moved.
    #[test]
    fn holding_snapshots_forces_a_write() {
        let mut recovery = Recovery::new(PathBuf::from("."));
        assert!(recovery.take_changes(Vec::new(), Vec::new()).is_some());
        assert!(recovery.take_changes(Vec::new(), Vec::new()).is_none());
        recovery.hold(vec![snap(None)]);
        let changes = recovery.take_changes(Vec::new(), Vec::new()).expect("held snapshots are a change");
        assert_eq!(changes.held.len(), 1);
    }

    #[test]
    fn held_snapshots_are_carried_into_each_write() {
        let mut recovery = Recovery::new(PathBuf::from("."));
        recovery.hold(vec![snap(Some(r"C:\a.md"))]);
        let changes = recovery.take_changes(Vec::new(), Vec::new()).unwrap();
        assert_eq!(changes.held[0].path.as_deref(), Some(r"C:\a.md"));
        // Still held: a later write, forced by a version change, carries them again.
        recovery.hold(Vec::new());
        let changes = recovery.take_changes(vec![(1, 1)], Vec::new()).unwrap();
        assert!(changes.held.is_empty());
    }
}
