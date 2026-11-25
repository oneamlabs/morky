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
        let doc = &self.tabs.documents()[index];
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
        self.tabs.index_of(id)
    }

    /// Writes the document on a background thread: syncing a file to disk can take tens of
    /// milliseconds, which typing must not wait for. One save per document runs at a time, so an
    /// older snapshot never lands after a newer one; a save asked for meanwhile runs next.
    fn start_save(&mut self, id: u64, target: PathBuf, cx: &mut Context<Self>) {
        let Some(index) = self.document_index(id) else { return };
        let doc = &mut self.tabs.documents_mut()[index];
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
        let doc = &mut self.tabs.documents_mut()[index];
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
                    let folder = target.parent().map(Path::to_path_buf);
                    if let Some(views) = self.views.get(&id) {
                        views.preview.update(cx, |pane, cx| pane.set_folder(folder, cx));
                    }
                }
            }
            Err(err) => self.report(format!("Could not save {}: {err}", target.display())),
        }
        let Some(next) = queued else { return };
        let already_saved = {
            let doc = &self.tabs.documents()[index];
            files::same_path(Path::new(&doc.path), &next) && !doc.is_dirty()
        };
        if already_saved {
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
        // One spelling per file, so `c:\a\.\b.md` and `C:\a\b.md` reuse the same tab and watcher.
        let normalized = files::normalize(path);
        let path = normalized.as_path();
        if let Some(index) = self.tabs.iter().position(|doc| files::same_path(Path::new(&doc.path), path)) {
            self.record_recent(path, cx);
            self.activate_tab(index, cx);
            return;
        }
        match files::read(path) {
            Ok(opened) => {
                self.status = None;
                let id = self.tabs.take_id();
                let document = Document::open(opened, id, cx);
                self.tabs.push(document);
                self.open_views(id, cx);
                self.schedule_parse(id, Duration::ZERO, cx);
                self.watch_document(id, cx);
                self.record_recent(path, cx);
                self.activate_tab(self.tabs.len() - 1, cx);
            }
            Err(err) => self.report(format!("Could not open {}: {err}", path.display())),
        }
    }

    pub(super) fn new_tab(&mut self, cx: &mut Context<Self>) {
        let id = self.tabs.take_id();
        let opened = OpenedFile::untitled(self.settings.new_file_line_ending);
        self.tabs.push(Document::open(opened, id, cx));
        self.open_views(id, cx);
        self.schedule_parse(id, Duration::ZERO, cx);
        self.activate_tab(self.tabs.len() - 1, cx);
        self.status = None;
    }

    /// Saves under a new name. A New Tab document has no path until this runs.
    pub(super) fn save_as_document(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.document_index(id) else { return };
        let suggested = if self.tabs.documents_mut()[index].path.is_empty() { "Untitled.md".to_string() } else { self.tabs.documents_mut()[index].display_name() };
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
        if self.tabs.documents_mut()[index].path.is_empty() {
            self.tabs.documents_mut()[index].watcher = None;
            self.tabs.documents_mut()[index].external_task = None;
            return;
        }
        let path = PathBuf::from(&self.tabs.documents_mut()[index].path);
        let (sender, receiver) = async_channel::unbounded::<()>();
        match watcher::watch(&path, move || {
            let _ = sender.try_send(());
        }) {
            Ok(watcher) => self.tabs.documents_mut()[index].watcher = Some(watcher),
            Err(err) => {
                self.tabs.documents_mut()[index].watcher = None;
                diagnostics::write(&format!("could not watch {}: {err}", path.display()));
            }
        }
        self.tabs.documents_mut()[index].external_task = Some(cx.spawn(async move |this, cx| {
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
        let doc = &self.tabs.documents()[index];
        // Our own save is still being written; its mtime is recorded when it finishes.
        if doc.save_task.is_some() {
            return;
        }
        let path = PathBuf::from(&doc.path);
        if path.as_os_str().is_empty() {
            return;
        }
        let (mtime_ms, name) = (doc.mtime_ms, doc.display_name());
        let Some(now) = files::mtime_ms(&path).ok() else {
            self.report(format!("{name} was removed from disk"));
            return;
        };
        if now == mtime_ms {
            return;
        }
        let Ok(opened) = files::read(&path) else { return };
        // Our own save, or another program writing the same bytes: only the time changed.
        if opened.text == doc.buffer.text() {
            let doc = &mut self.tabs.documents_mut()[index];
            doc.mtime_ms = opened.mtime_ms;
            doc.meta = opened.meta;
            return;
        }
        if doc.is_dirty() {
            let reload = rfd::MessageDialog::new()
                .set_title("Marky")
                .set_level(rfd::MessageLevel::Warning)
                .set_description(format!("{name} changed on disk. Reload it and lose your unsaved changes?"))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show();
            if reload != rfd::MessageDialogResult::Yes {
                // Keep mine: later events for the same file are ignored until it changes again.
                self.tabs.documents_mut()[index].mtime_ms = opened.mtime_ms;
                return;
            }
        }
        self.reload_document(id, opened, cx);
        self.report(format!("Reloaded {name} from disk"));
    }

    pub(super) fn reload_document(&mut self, id: u64, opened: OpenedFile, cx: &mut Context<Self>) {
        let Some(index) = self.document_index(id) else { return };
        self.tabs.documents_mut()[index] = Document::open(opened, id, cx);
        if let Some(views) = self.views.get(&id) {
            views.preview.update(cx, |pane, cx| pane.clear(cx));
            views.outline.update(cx, |pane, cx| pane.set_headings(Vec::new(), cx));
        }
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
        if self.tabs.documents_mut()[index].path.is_empty() || self.tabs.documents_mut()[index].read_only {
            return;
        }
        self.tabs.documents_mut()[index].auto_save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(1000)).await;
            let _ = this.update(cx, |this, cx| {
                if !this.settings.auto_save {
                    return;
                }
                if this.tabs.documents().iter().any(|doc| doc.id == id && doc.is_dirty()) {
                    this.write_document(id, cx);
                }
            });
        }));
    }
}
