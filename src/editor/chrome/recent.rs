//! The recent-files panel, built from the app's `recent.json`. A side panel, like the settings:
//! Esc or Close puts it away.

use super::*;

impl Prototype {
    // ----- Recent files -----

    pub(super) fn toggle_recent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.recent_open = !self.recent_open;
        if self.recent_open {
            self.settings_open = false;
            self.recent = store::load_recent(&self.config_dir).into_iter().filter(|path| path.is_file()).collect();
            window.focus(&self.recent_focus, cx);
        } else {
            self.focus_editor(window, cx);
        }
        cx.notify();
    }

    pub fn close_recent(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.recent_open {
            return false;
        }
        self.recent_open = false;
        self.focus_editor(window, cx);
        true
    }

    /// The app's Open Recent list. A side panel, like the settings: Esc or Close puts it away.
    pub fn recent_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.dark;
        let entries = self.recent.clone();
        let rows = entries.into_iter().enumerate().map(|(index, path)| {
            let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string_lossy().into_owned());
            let folder = path.parent().map(|folder| folder.to_string_lossy().into_owned()).unwrap_or_default();
            let outcome = path.clone();
            focus_ring(
                div()
                    .id(("recent", index))
                    .role(Role::Button)
                    .aria_label(SharedString::from(path.to_string_lossy().into_owned()))
                    .focusable()
                    .tab_stop(true)
                    .flex()
                    .flex_col()
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .child(div().child(name))
                    .child(div().text_size(px(11.)).text_color(quiet(dark)).child(folder))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.recent_open = false;
                        this.open_path(&outcome, cx);
                        this.focus_editor(window, cx);
                    })),
                dark,
            )
        });
        div()
            .id("recent")
            .key_context("Recent")
            .track_focus(&self.recent_focus)
            .role(Role::Group)
            .aria_label("Recent files")
            .w(px(300.))
            .h_full()
            .overflow_y_scroll()
            .px_3()
            .py_2()
            .text_size(px(13.))
            .bg(tone(dark, 0x18181b, 0xf4f4f5))
            .border_l_1()
            .border_color(tone(dark, 0x3f3f46, 0xd4d4d8))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .child(div().id("recent-title").role(Role::Heading).aria_level(2).aria_label("Recent files").font_weight(gpui::FontWeight::BOLD).child("Recent files"))
                    .child(
                        focus_ring(
                            div().id("recent-close").role(Role::Button).aria_label("Close recent files").focusable().tab_stop(true).px_2().child("×"),
                            dark,
                        )
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.close_recent(window, cx);
                        })),
                    ),
            )
            .children(rows.collect::<Vec<_>>())
            .when(self.recent.is_empty(), |panel| panel.child("No recent files"))
    }
}
