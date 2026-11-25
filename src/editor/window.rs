//! The window itself rather than its contents: the saved geometry, the title bar, and the timing
//! marks that startup and typing are measured with.

use super::*;

impl Prototype {
    /// Writes the restored window bounds a moment after a resize or move stops, so dragging does
    /// not write the file on every frame.
    pub(super) fn schedule_geometry_save(&mut self, window: &Window, cx: &mut Context<Self>) {
        let geometry = Self::geometry_of(window);
        let config_dir = self.config_dir.clone();
        self.geometry_task = Some(cx.spawn(async move |_this, cx| {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            // The write itself does not run on the UI thread; dragging must not wait for a disk sync.
            cx.background_executor().spawn(async move { Self::save_geometry(&config_dir, &geometry) }).detach();
        }));
    }

    /// Writes the current bounds now. Closing drops the debounced task, so this keeps the last move
    /// or resize instead of losing it.
    pub(super) fn flush_geometry(&self, window: &Window) {
        Self::save_geometry(&self.config_dir, &Self::geometry_of(window));
    }

    /// The title is the document's name and a dot while it has unsaved changes.
    pub(super) fn update_title(&mut self, window: &mut Window) {
        let doc = self.doc();
        let title = format!("{}{} - Marky", doc.display_name(), if doc.is_dirty() { " •" } else { "" });
        if title != self.window_title {
            window.set_window_title(&title);
            self.window_title = title;
        }
    }

    /// The window's current bounds, as the settings file stores them.
    fn geometry_of(window: &Window) -> store::WindowGeometry {
        let (bounds, maximized) = match window.window_bounds() {
            WindowBounds::Windowed(bounds) => (bounds, false),
            WindowBounds::Maximized(bounds) | WindowBounds::Fullscreen(bounds) => (bounds, true),
        };
        store::WindowGeometry {
            width: f64::from(f32::from(bounds.size.width)),
            height: f64::from(f32::from(bounds.size.height)),
            x: f64::from(f32::from(bounds.origin.x)),
            y: f64::from(f32::from(bounds.origin.y)),
            maximized,
        }
    }

    fn save_geometry(config_dir: &Path, geometry: &store::WindowGeometry) {
        if let Err(err) = store::save_window_geometry(config_dir, geometry) {
            diagnostics::write(&format!("could not save the window geometry: {err}"));
        }
    }

    /// Startup and typing are timed to the end of the frame's paint, like the app's
    /// measurements (a rAF callback that reports after paint).
    pub(super) fn frame_timer(&mut self) -> impl IntoElement {
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
}
