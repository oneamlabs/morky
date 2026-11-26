//! The settings panel: theme, font size, word wrap, the typographer, auto-save, the line ending of
//! new files and which panes show.
//!
//! A side panel, not a modal: Esc or Close puts it away. Tab and Shift+Tab cycle its controls and
//! wrap at either end, so the focus stays inside while it is open.

use super::*;

impl Prototype {
    // ----- Settings panel -----

    pub(super) fn toggle_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = !self.settings_open;
        if self.settings_open {
            self.recent_open = false;
            window.focus(&self.settings_controls[settings_slot::THEME_SYSTEM], cx);
        } else {
            self.focus_editor(window, cx);
        }
        cx.notify();
    }

    /// Tab and Shift+Tab cycle the settings controls and wrap at either end, so the focus stays in
    /// the open panel.
    pub(super) fn settings_step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.settings_controls.len() as isize;
        if count == 0 {
            return;
        }
        let current = self.settings_controls.iter().position(|handle| handle.is_focused(window)).map_or(-1, |index| index as isize);
        let next = if current < 0 {
            if delta > 0 { 0 } else { count - 1 }
        } else {
            (current + delta).rem_euclid(count)
        };
        window.focus(&self.settings_controls[next as usize], cx);
        cx.notify();
    }

    pub fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.settings_open {
            return false;
        }
        self.settings_open = false;
        self.restore_editor_focus(window, cx);
        true
    }

    /// The settings panel (auto-save
    /// and the line ending of new files). A side panel, not a modal: Esc or Close puts it away.
    pub fn settings_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.dark;
        let settings = self.settings.clone();
        let heading = |text: &'static str| div().pt_3().pb_1().font_weight(gpui::FontWeight::BOLD).child(text);
        let theme_choice = |id: &'static str, label: &'static str, choice: crate::settings::ThemeChoice, focus: FocusHandle, cx: &mut Context<Self>| {
            choice_row(id, label, Role::RadioButton, settings.theme == choice, dark)
                .track_focus(&focus)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.settings.theme = choice;
                    this.keep_settings(cx);
                }))
        };
        let flag = |id: &'static str, label: &'static str, on: bool, change: fn(&mut ViewSettings), focus: FocusHandle, cx: &mut Context<Self>| {
            choice_row(id, label, Role::CheckBox, on, dark).track_focus(&focus).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                change(&mut this.settings);
                this.keep_settings(cx);
            }))
        };
        let size_button = |id: &'static str, label: &'static str, name: &'static str, delta: isize, focus: FocusHandle, cx: &mut Context<Self>| {
            focus_ring(
                div()
                    .id(id)
                    .role(Role::Button)
                    .aria_label(name)
                    .focusable()
                    .tab_stop(true)
                    .track_focus(&focus)
                    .px_2()
                    .rounded(px(theme::RADIUS_CONTROL))
                    .bg(color(dark, Color::Card))
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.step_font(delta, cx))),
                dark,
            )
        };
        let size = settings.font_size;
        let typographer = settings.typographer;
        div()
            .id("settings")
            .key_context("Settings")
            .track_focus(&self.settings_focus)
            .role(Role::Group)
            .aria_label("Settings")
            .w(px(280.))
            .h_full()
            .overflow_y_scroll()
            .px_3()
            .py_2()
            .text_size(px(theme::CHROME_TEXT))
            .bg(color(dark, Color::Base))
            .border_l_1()
            .border_color(color(dark, Color::Stroke))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .child(div().id("settings-title").role(Role::Heading).aria_level(2).aria_label("Settings").font_weight(gpui::FontWeight::BOLD).child("Settings"))
                    .child(
                        focus_ring(
                            div()
                                .id("settings-close")
                                .role(Role::Button)
                                .aria_label("Close settings")
                                .focusable()
                                .tab_stop(true)
                                .track_focus(&self.settings_controls[settings_slot::CLOSE])
                                .px_2()
                                .child("×"),
                            dark,
                        )
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.close_settings(window, cx);
                        })),
                    ),
            )
            .child(heading("Theme"))
            .child(
                div()
                    .id("theme-group")
                    .role(Role::RadioGroup)
                    .aria_label("Theme")
                    .child(theme_choice("theme-system", "System", crate::settings::ThemeChoice::System, self.settings_controls[settings_slot::THEME_SYSTEM].clone(), cx))
                    .child(theme_choice("theme-light", "Light", crate::settings::ThemeChoice::Light, self.settings_controls[settings_slot::THEME_LIGHT].clone(), cx))
                    .child(theme_choice("theme-dark", "Dark", crate::settings::ThemeChoice::Dark, self.settings_controls[settings_slot::THEME_DARK].clone(), cx)),
            )
            .child(heading("Editor font size"))
            .child(
                div()
                    .id("font-size")
                    .role(Role::SpinButton)
                    .aria_label(SharedString::from(format!("Editor font size {size} px")))
                    .aria_numeric_value(size as f64)
                    .aria_min_numeric_value(crate::settings::FONT_SIZES[0] as f64)
                    .aria_max_numeric_value(crate::settings::FONT_SIZES[crate::settings::FONT_SIZES.len() - 1] as f64)
                    .on_a11y_action(gpui::AccessibleAction::Increment, {
                        let view = cx.entity().downgrade();
                        move |_, _, cx| {
                            let _ = view.update(cx, |this, cx| this.step_font(1, cx));
                        }
                    })
                    .on_a11y_action(gpui::AccessibleAction::Decrement, {
                        let view = cx.entity().downgrade();
                        move |_, _, cx| {
                            let _ = view.update(cx, |this, cx| this.step_font(-1, cx));
                        }
                    })
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(size_button("font-smaller", "−", "Smaller text", -1, self.settings_controls[settings_slot::FONT_SMALLER].clone(), cx))
                    .child(format!("{size} px"))
                    .child(size_button("font-larger", "+", "Larger text", 1, self.settings_controls[settings_slot::FONT_LARGER].clone(), cx)),
            )
            .child(heading("Editor and preview"))
            .child(flag("set-wrap", "Word wrap", settings.word_wrap, |s| s.word_wrap = !s.word_wrap, self.settings_controls[settings_slot::WORD_WRAP].clone(), cx))
            .child(
                choice_row("set-typographer", "Smart quotes and dashes in the preview", Role::CheckBox, typographer, dark)
                    .track_focus(&self.settings_controls[settings_slot::TYPOGRAPHER])
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_typographer(!typographer, cx))),
            )
            .child(heading("Saving"))
            .child(flag("set-autosave", "Auto save", settings.auto_save, |s| s.auto_save = !s.auto_save, self.settings_controls[settings_slot::AUTO_SAVE].clone(), cx))
            .child(
                choice_row(
                    "line-lf",
                    "LF for new files",
                    Role::RadioButton,
                    settings.new_file_line_ending == crate::files::LineEnding::Lf,
                    dark,
                )
                .track_focus(&self.settings_controls[settings_slot::LINE_LF])
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_line_ending(crate::files::LineEnding::Lf, cx))),
            )
            .child(
                choice_row(
                    "line-crlf",
                    "CRLF for new files",
                    Role::RadioButton,
                    settings.new_file_line_ending == crate::files::LineEnding::Crlf,
                    dark,
                )
                .track_focus(&self.settings_controls[settings_slot::LINE_CRLF])
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_line_ending(crate::files::LineEnding::Crlf, cx))),
            )
            .child(heading("Panes"))
            .child(flag("set-editor", "Show editor", settings.show_editor, |s| s.show_editor = !s.show_editor, self.settings_controls[settings_slot::SHOW_EDITOR].clone(), cx))
            .child(flag("set-preview", "Show preview", settings.show_preview, |s| s.show_preview = !s.show_preview, self.settings_controls[settings_slot::SHOW_PREVIEW].clone(), cx))
            .child(flag("set-outline", "Show outline", settings.show_outline, |s| s.show_outline = !s.show_outline, self.settings_controls[settings_slot::SHOW_OUTLINE].clone(), cx))
            .on_action(cx.listener(|this, _: &SettingsNext, window, cx| this.settings_step(1, window, cx)))
            .on_action(cx.listener(|this, _: &SettingsPrevious, window, cx| this.settings_step(-1, window, cx)))
    }

    pub(super) fn set_line_ending(&mut self, ending: crate::files::LineEnding, cx: &mut Context<Self>) {
        self.settings.new_file_line_ending = ending;
        self.keep_settings(cx);
    }

    /// Smart punctuation changes what the preview shows, so every open document is parsed again.
    pub(super) fn set_typographer(&mut self, on: bool, cx: &mut Context<Self>) {
        self.settings.typographer = on;
        self.keep_settings(cx);
        let ids: Vec<u64> = self.tabs.iter().map(|doc| doc.id).collect();
        for id in ids {
            self.schedule_parse(id, Duration::ZERO, cx);
        }
    }

    /// Saves the view settings and closes any menu. Every control in this panel ends here. The
    /// write runs in the background, so a slow disk cannot stall the frame that changed a setting.
    pub(crate) fn keep_settings(&mut self, cx: &mut Context<Self>) {
        self.open_menu = None;
        let config_dir = self.config_dir.clone();
        let settings = self.settings.clone();
        self.settings_task = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { crate::settings::save(&config_dir, &settings) }).await;
            let _ = this.update(cx, |this, cx| {
                this.settings_task = None;
                if let Err(message) = result {
                    this.report(message);
                }
                cx.notify();
            });
        }));
        // A changed setting can reveal a cached pane (the outline, the preview), so repaint them.
        self.notify_views(cx);
        cx.notify();
    }
}