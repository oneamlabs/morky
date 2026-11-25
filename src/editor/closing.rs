//! Closing the window: the unsaved-changes prompt, waiting for saves in flight, and the close
//! requested from the title bar, Quit, or the last tab.

use super::*;

impl Prototype {
    fn dirty_count(&self) -> usize {
        self.tabs.dirty_count()
    }

    pub(super) fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        // The last tab is the window: Ctrl+W there asks to close it, as the shortcut table says.
        if self.tabs.len() == 1 {
            if self.may_close(window, cx) {
                window.remove_window();
            }
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
        let id = self.tabs.documents()[index].id;
        if !self.tabs.remove(index) {
            return;
        }
        self.views.remove(&id);
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
        if self.tabs.iter().any(|doc| doc.save_task.is_some()) || self.settings_task.is_some() {
            self.close_after_saves(window, cx);
            return false;
        }
        let dirty = self.dirty_count();
        if dirty == 0 || self.close_confirmed {
            // A postponed recovery survives a normal close; with nothing left to recover, this clears it.
            self.flush_geometry(window);
            self.flush_recovery(cx);
            return true;
        }
        let message = if dirty == 1 { "Discard unsaved changes in 1 file?".to_string() } else { format!("Discard unsaved changes in {dirty} files?") };
        self.confirm_discard(&message, window, cx, |this, window, cx| {
            // Discarding everything: nothing is left to recover, and held snapshots stay held.
            for document in this.tabs.documents_mut() {
                document.restored = false;
                document.buffer.mark_saved();
            }
            this.close_confirmed = true;
            this.flush_geometry(window);
            this.flush_recovery(cx);
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
                let Ok(saving) = this.update(cx, |this, _| {
                    this.tabs.documents().iter().any(|doc| doc.save_task.is_some()) || this.settings_task.is_some()
                }) else {
                    return;
                };
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

    pub(super) fn quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        if self.may_close(window, cx) {
            cx.quit();
        }
    }
}
