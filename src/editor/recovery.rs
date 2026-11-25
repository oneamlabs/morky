//! Crash-recovery snapshots: the restore/discard prompt at startup, the 30 s timer, and the rule
//! that an unchanged dirty document is not serialised again.
//!
//! `RecoveryWrite` is built on the main thread from cheap rope snapshots and turned into text on the
//! thread that writes it, so the timer never copies a large document on the UI thread.

use super::*;

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
                if let Err(err) = store::clear_snapshots(&self.recovery_dir) {
                    diagnostics::write(&format!("could not clear recovery snapshots: {err}"));
                }
            }
            _ => {
                self.held_recovery = recovery;
                self.recovery_dirty = true;
            }
        }
    }

    pub(super) fn start_recovery_timer(&mut self, cx: &mut Context<Self>) {
        self.recovery_task = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(30)).await;
            if this.update(cx, |this, cx| this.write_recovery(cx)).is_err() {
                break;
            }
        }));
    }

    /// What the next recovery write holds, or `None` when no buffer version and no held snapshot
    /// changed since the last one: serialising every dirty document is itself a cost at 8 MB.
    fn take_recovery_changes(&mut self) -> Option<RecoveryWrite> {
        let versions = self.tabs.dirty_versions();
        if !self.recovery_dirty && versions == self.recovery_versions {
            return None;
        }
        self.recovery_versions = versions;
        self.recovery_dirty = false;
        let documents = self
            .tabs
            .iter()
            .filter(|doc| doc.is_dirty())
            .map(|doc| ((!doc.path.is_empty()).then(|| doc.path.clone()), doc.buffer.snapshot(), doc.meta))
            .collect();
        Some(RecoveryWrite { held: self.held_recovery.clone(), documents })
    }

    fn write_recovery(&mut self, cx: &mut Context<Self>) {
        let Some(changes) = self.take_recovery_changes() else { return };
        let dir = self.recovery_dir.clone();
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
        if let Err(err) = changes.write(&self.recovery_dir) {
            diagnostics::write(&format!("could not save recovery snapshots on close: {err}"));
        }
    }
}
