//! A document's life on disk: saving (queued, on a background thread), opening, and noticing when
//! another program changes the file.
//!
//! Named `disk` rather than `files` so it does not shadow the crate-level `crate::files` module
//! (`read`, `write`, `write_atomically`) that this code calls.

use super::*;

impl Prototype {
    /// Restores the file's BOM and line ending. A clean buffer and a lossy decode are left on disk as they are.
    /// A New Tab document has no file yet, so Save asks for a name instead.
    pub(super) fn write_active(&mut self, cx: &mut Context<Self>) {
        self.write_document(self.doc().id, cx);
    }

    pub(super) fn write_document(&mut self, id: u64, cx: &mut Context<Self>) {
        self.open_menu = None;
        cx.notify();
        let Some(index) = self.document_index(id) else { return };
        let doc = &self.documents[index];
        if doc.path.is_empty() {
            self.save_as_document(id, cx);
            return;
        }
        if doc.read_only {
            let message = format!("Not saved: {} is not valid UTF-8", doc.path);
            self.report(message);
            return;
        }
        if !doc.is_dirty() {
            return;
        }
        let target = PathBuf::from(&doc.path);
        self.start_save(id, target, cx);
    }

    pub(super) fn document_index(&self, id: u64) -> Option<usize> {
        self.documents.iter().position(|doc| doc.id == id)
    }

    /// Writes the document on a background thread: syncing a file to disk can take tens of
    /// milliseconds, which typing must not wait for. One save per document runs at a time, so an
    /// older snapshot never lands after a newer one; a save asked for meanwhile runs next.
    fn start_save(&mut self, id: u64, target: PathBuf, cx: &mut Context<Self>) {
        let Some(index) = self.document_index(id) else { return };
        let doc = &mut self.documents[index];
        if doc.save_task.is_some() {
            doc.queued_save = Some(target);
            return;
        }
        let point = doc.buffer.save_point();
        let rope = doc.buffer.snapshot();
        let meta = doc.meta;
        doc.save_task = Some(cx.spawn(async move |this, cx| {
            let path = target.clone();
            let written = cx.background_executor().spawn(async move { files::write(&path, &rope.to_string(), meta) }).await;
            let _ = this.update(cx, |this, cx| this.finish_save(id, target, point, written, cx));
        }));
    }

    /// The file now holds the text as it was when the save started; later edits stay unsaved.
    /// A save to a new path (Save As) also moves the tab, its watcher and the recent list there.
    fn finish_save(&mut self, id: u64, target: PathBuf, point: SavePoint, written: std::io::Result<u64>, cx: &mut Context<Self>) {
        let Some(index) = self.document_index(id) else { return };
        let doc = &mut self.documents[index];
        doc.save_task = None;
        let queued = doc.queued_save.take();
        cx.notify();
        match written {
            Ok(mtime) => {
                let moved = doc.path.is_empty() || !files::same_path(Path::new(&doc.path), &target);
                doc.path = target.to_string_lossy().into_owned();
                doc.mtime_ms = mtime;
                doc.read_only = false;
                doc.restored = false;
                doc.buffer.mark_saved_at(point);
                self.status = None;
                self.record_recent(&target, cx);
                if moved {
                    self.watch_document(id, cx);
                }
            }
            Err(err) => self.report(format!("Could not save {}: {err}", target.display())),
        }
        let Some(next) = queued else { return };
        let doc = &self.documents[index];
        if files::same_path(Path::new(&doc.path), &next) && !doc.is_dirty() {
            return;
        }
        self.start_save(id, next, cx);
    }

    pub(super) fn save(&mut self, _: &Save, _: &mut Window, cx: &mut Context<Self>) {
        self.write_active(cx);
    }

    pub(super) fn open_file(&mut self, cx: &mut Context<Self>) {
        self.open_menu = None;
        cx.notify();
        let Some(path) = rfd::FileDialog::new().add_filter("Markdown", files::MARKDOWN_EXTS).pick_file() else {
            return;
        };
        self.open_path(&path, cx);
    }

    /// Opens `path` as a tab, or switches to the tab that already has it.
    pub(super) fn open_path(&mut self, path: &Path, cx: &mut Context<Self>) {
        if let Some(index) = self.documents.iter().position(|doc| files::same_path(Path::new(&doc.path), path)) {
            self.record_recent(path, cx);
            self.activate_tab(index, cx);
            return;
        }
        match files::read(path) {
            Ok(opened) => {
                self.status = None;
                let id = self.next_document_id;
                self.next_document_id += 1;
                let document = Document::open(opened, id, cx);
                self.documents.push(document);
                self.schedule_parse(id, Duration::ZERO, cx);
                self.watch_document(id, cx);
                self.record_recent(path, cx);
                self.activate_tab(self.documents.len() - 1, cx);
            }
            Err(err) => self.report(format!("Could not open {}: {err}", path.display())),
        }
    }

    pub(super) fn new_tab(&mut self, cx: &mut Context<Self>) {
        let id = self.next_document_id;
        self.next_document_id += 1;
        let opened = OpenedFile::untitled(self.settings.new_file_line_ending);
        self.documents.push(Document::open(opened, id, cx));
        self.schedule_parse(id, Duration::ZERO, cx);
        self.activate_tab(self.documents.len() - 1, cx);
        self.status = None;
    }

    /// Saves under a new name. A New Tab document has no path until this runs.
    pub(super) fn save_as_document(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.document_index(id) else { return };
        let suggested = if self.documents[index].path.is_empty() { "Untitled.md".to_string() } else { self.documents[index].display_name() };
        let Some(target) = rfd::FileDialog::new().add_filter("Markdown", files::MARKDOWN_EXTS).set_file_name(suggested).save_file() else {
            return;
        };
        self.start_save(id, target, cx);
    }

    /// Writing the list syncs it to disk, so it happens in the background, and not at all when the
    /// path is already at the top (every auto-save of the same file).
    fn record_recent(&mut self, path: &Path, cx: &mut Context<Self>) {
        if path.as_os_str().is_empty() || self.recent.first().is_some_and(|first| files::same_path(first, path)) {
            return;
        }
        let config_dir = self.config_dir.clone();
        let path = path.to_path_buf();
        cx.spawn(async move |this, cx| {
            let recent = cx
                .background_executor()
                .spawn(async move {
                    store::push_recent(&config_dir, &path)?;
                    Ok::<_, std::io::Error>(store::load_recent(&config_dir).into_iter().filter(|path| path.is_file()).collect::<Vec<_>>())
                })
                .await;
            let _ = this.update(cx, |this, cx| match recent {
                Ok(recent) => {
                    this.recent = recent;
                    cx.notify();
                }
                Err(err) => diagnostics::write(&format!("could not update the recent files list: {err}")),
            });
        })
        .detach();
    }

    /// Watches a document's file for changes made by another program. The app debounces and compares
    /// mtimes; `check_external_change` does the same, after the burst settles.
    pub(super) fn watch_document(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.document_index(id) else { return };
        if self.documents[index].path.is_empty() {
            self.documents[index].watcher = None;
            self.documents[index].external_task = None;
            return;
        }
        let path = PathBuf::from(&self.documents[index].path);
        let (sender, receiver) = async_channel::unbounded::<()>();
        match watcher::watch(&path, move || {
            let _ = sender.try_send(());
        }) {
            Ok(watcher) => self.documents[index].watcher = Some(watcher),
            Err(err) => {
                self.documents[index].watcher = None;
                diagnostics::write(&format!("could not watch {}: {err}", path.display()));
            }
        }
        self.documents[index].external_task = Some(cx.spawn(async move |this, cx| {
            while receiver.recv().await.is_ok() {
                // A save can arrive as several events; read once after they stop.
                while receiver.try_recv().is_ok() {}
                cx.background_executor().timer(Duration::from_millis(200)).await;
                while receiver.try_recv().is_ok() {}
                if this.update(cx, |this, cx| this.check_external_change(id, cx)).is_err() {
                    break;
                }
            }
        }));
    }

    fn check_external_change(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.document_index(id) else { return };
        // Our own save is still being written; its mtime is recorded when it finishes.
        if self.documents[index].save_task.is_some() {
            return;
        }
        let path = PathBuf::from(&self.documents[index].path);
        if path.as_os_str().is_empty() {
            return;
        }
        let Some(now) = files::mtime_ms(&path).ok() else {
            self.report(format!("{} was removed from disk", self.documents[index].display_name()));
            return;
        };
        if now == self.documents[index].mtime_ms {
            return;
        }
        let Ok(opened) = files::read(&path) else { return };
        // Our own save, or another program writing the same bytes: only the time changed.
        if opened.text == self.documents[index].buffer.text() {
            self.documents[index].mtime_ms = opened.mtime_ms;
            self.documents[index].meta = opened.meta;
            return;
        }
        if self.documents[index].is_dirty() {
            let name = self.documents[index].display_name();
            let reload = rfd::MessageDialog::new()
                .set_title("Marky")
                .set_level(rfd::MessageLevel::Warning)
                .set_description(format!("{name} changed on disk. Reload it and lose your unsaved changes?"))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show();
            if reload != rfd::MessageDialogResult::Yes {
                // Keep mine: later events for the same file are ignored until it changes again.
                self.documents[index].mtime_ms = opened.mtime_ms;
                return;
            }
        }
        let name = self.documents[index].display_name();
        self.reload_document(id, opened, cx);
        self.report(format!("Reloaded {name} from disk"));
    }

    pub(super) fn reload_document(&mut self, id: u64, opened: OpenedFile, cx: &mut Context<Self>) {
        let Some(index) = self.document_index(id) else { return };
        self.documents[index] = Document::open(opened, id, cx);
        self.preview_link = None;
        self.schedule_parse(id, Duration::ZERO, cx);
        self.watch_document(id, cx);
        self.release_unused_images(cx);
        cx.notify();
    }

    pub(super) fn schedule_auto_save(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.settings.auto_save {
            return;
        }
        let Some(index) = self.document_index(id) else { return };
        if self.documents[index].path.is_empty() || self.documents[index].read_only {
            return;
        }
        self.documents[index].auto_save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(1000)).await;
            let _ = this.update(cx, |this, cx| {
                if !this.settings.auto_save {
                    return;
                }
                if this.documents.iter().any(|doc| doc.id == id && doc.is_dirty()) {
                    this.write_document(id, cx);
                }
            });
        }));
    }
}
