//! The window's chrome around the panes: menus, toolbar, tabs, find bar, outline, settings panel and
//! status bar (which carries the document's path).
//!
//! Menu items, toolbar buttons and shortcuts all run a [`Command`], as the app's menus, toolbar and
//! `SHORTCUTS` all go through its `commands` table. Every control can be reached from the keyboard
//! (Tab, the menu keys, or arrows inside a menu or the outline) and has a role and a name for
//! assistive technology; GPUI reports them through AccessKit.

use gpui::{ClickEvent, Role, Stateful, Toggled, transparent_black};

use super::*;

mod about;
mod menu;
mod recent;
mod settings;
mod title_bar;

pub use menu::*;

actions!(
    marky,
    [
        OpenFile,
        OpenSettings,
        NewTab,
        SaveAs,
        OpenRecent,
        CloseTab,
        NextTab,
        PreviousTab,
        ToggleEditor,
        TogglePreview,
        ToggleOutline,
        ToggleWordWrap,
        OpenFileMenu,
        OpenEditMenu,
        OpenViewMenu,
        OpenHelpMenu,
        MenuNext,
        MenuPrevious,
        MenuLeft,
        MenuRight,
        FocusNext,
        FocusPrevious,
        OutlineNext,
        OutlinePrevious,
        OutlineOpen,
        SettingsNext,
        SettingsPrevious,
        PreviewLinkNext,
        PreviewLinkPrevious,
        PreviewLinkOpen,
        PreviewLineUp,
        PreviewLineDown,
        PreviewPageUp,
        PreviewPageDown,
        PreviewTop,
        PreviewBottom,
    ]
);

/// An underlined menu letter. It is an action, not a key listener, so GPUI consumes the key-down
/// and Windows does not also translate it into a character that would be typed into the editor.
#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = marky, no_json)]
pub struct MenuAccessKey(pub char);

/// What a menu item, toolbar button or shortcut does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    NewTab,
    Open,
    OpenRecent,
    Save,
    SaveAs,
    ToggleAutoSave,
    Settings,
    CloseTab,
    Quit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Find,
    Replace,
    ToggleEditor,
    TogglePreview,
    ToggleOutline,
    ToggleWordWrap,
    NextTab,
    PreviousTab,
    LargerText,
    SmallerText,
    About,
    CheckForUpdates,
    OpenProjectPage,
}

impl Command {
    /// Commands that put the focus somewhere of their own; the others hand it back to the editor
    /// when they come from a menu or the toolbar.
    fn moves_focus(self) -> bool {
        matches!(
            self,
            Self::Find
                | Self::Replace
                | Self::Settings
                | Self::Open
                | Self::OpenRecent
                | Self::About
                | Self::CheckForUpdates
                | Self::Quit
        )
    }
}


/// Where each control in the settings panel keeps its focus handle, so Tab and Shift+Tab can cycle
/// them and wrap. Named rather than written as numbers: adding a control used to shift every index
/// after it, which broke the Tab order silently. `COUNT` sizes the list in `Prototype::new`.
pub mod settings_slot {
    pub const THEME_SYSTEM: usize = 0;
    pub const THEME_LIGHT: usize = 1;
    pub const THEME_DARK: usize = 2;
    pub const FONT_SMALLER: usize = 3;
    pub const FONT_LARGER: usize = 4;
    pub const WORD_WRAP: usize = 5;
    pub const TYPOGRAPHER: usize = 6;
    pub const AUTO_SAVE: usize = 7;
    pub const LINE_LF: usize = 8;
    pub const LINE_CRLF: usize = 9;
    pub const SHOW_EDITOR: usize = 10;
    pub const SHOW_PREVIEW: usize = 11;
    pub const SHOW_OUTLINE: usize = 12;
    pub const CLOSE: usize = 13;

    /// One past the last slot: the number of handles the panel needs.
    pub const COUNT: usize = CLOSE + 1;
}

fn toggled(on: bool) -> Toggled {
    if on { Toggled::True } else { Toggled::False }
}

impl Prototype {
    /// Every command, whichever control or key started it.
    pub fn run(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        self.open_menu = None;
        match command {
            Command::NewTab => self.new_tab(cx),
            Command::Open => self.open_file(cx),
            Command::OpenRecent => self.toggle_recent(window, cx),
            Command::Save => self.write_active(cx),
            Command::SaveAs => self.save_as_document(self.doc().id, cx),
            Command::ToggleAutoSave => {
                self.settings.auto_save = !self.settings.auto_save;
                self.keep_settings(cx);
            }
            Command::Settings => self.toggle_settings(window, cx),
            Command::CloseTab => self.close_tab(self.tabs.active(), window, cx),
            Command::Quit => self.quit(&Quit, window, cx),
            Command::Undo => self.undo(&Undo, window, cx),
            Command::Redo => self.redo(&Redo, window, cx),
            Command::Cut => self.cut(&Cut, window, cx),
            Command::Copy => self.copy(&Copy, window, cx),
            Command::Paste => self.paste(&Paste, window, cx),
            Command::SelectAll => self.select_all(&SelectAll, window, cx),
            Command::Find => self.open_find(false, window, cx),
            Command::Replace => self.open_find(true, window, cx),
            Command::ToggleEditor => {
                // Hiding the editor never changes `showPreview`; the preview shows anyway, as in the app.
                self.settings.show_editor = !self.settings.show_editor;
                self.keep_settings(cx);
            }
            Command::TogglePreview => {
                // With the editor hidden the preview cannot be turned off without emptying the
                // window, so the click reveals the editor and hides the preview.
                if self.settings.show_editor {
                    self.settings.show_preview = !self.settings.show_preview;
                } else {
                    self.settings.show_editor = true;
                    self.settings.show_preview = false;
                }
                self.keep_settings(cx);
                if self.settings.preview_visible() {
                    self.refresh_revealed_views(cx);
                }
            }
            Command::ToggleOutline => {
                self.settings.show_outline = !self.settings.show_outline;
                self.keep_settings(cx);
                if self.settings.show_outline {
                    self.refresh_revealed_views(cx);
                }
            }
            Command::ToggleWordWrap => {
                self.settings.word_wrap = !self.settings.word_wrap;
                self.keep_settings(cx);
            }
            Command::NextTab => {
                if let Some(next) = self.tabs.next_index() {
                    self.activate_tab(next, cx);
                }
            }
            Command::PreviousTab => {
                if let Some(previous) = self.tabs.previous_index() {
                    self.activate_tab(previous, cx);
                }
            }
            Command::LargerText => self.step_font(1, cx),
            Command::SmallerText => self.step_font(-1, cx),
            Command::About => self.open_about(window, cx),
            Command::CheckForUpdates => self.check_for_updates(window, cx),
            Command::OpenProjectPage => cx.open_url(crate::update::RELEASES_URL),
        }
        cx.notify();
    }

    /// The shortcuts for commands that have no action of their own in `editor.rs`.
    pub fn command_actions<E: InteractiveElement>(&self, element: E, cx: &mut Context<Self>) -> E {
        element
            .on_action(cx.listener(|this, _: &NewTab, window, cx| this.run(Command::NewTab, window, cx)))
            .on_action(cx.listener(|this, _: &OpenFile, window, cx| this.run(Command::Open, window, cx)))
            .on_action(cx.listener(|this, _: &OpenRecent, window, cx| this.run(Command::OpenRecent, window, cx)))
            .on_action(cx.listener(|this, _: &SaveAs, window, cx| this.run(Command::SaveAs, window, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| this.run(Command::Settings, window, cx)))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| this.run(Command::CloseTab, window, cx)))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.run(Command::NextTab, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, window, cx| this.run(Command::PreviousTab, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleEditor, window, cx| this.run(Command::ToggleEditor, window, cx)))
            .on_action(cx.listener(|this, _: &TogglePreview, window, cx| this.run(Command::TogglePreview, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleOutline, window, cx| this.run(Command::ToggleOutline, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleWordWrap, window, cx| this.run(Command::ToggleWordWrap, window, cx)))
            .on_action(cx.listener(|this, _: &OpenFileMenu, window, cx| this.open_menu_from_keys(OpenMenu::File, window, cx)))
            .on_action(cx.listener(|this, _: &OpenEditMenu, window, cx| this.open_menu_from_keys(OpenMenu::Edit, window, cx)))
            .on_action(cx.listener(|this, _: &OpenViewMenu, window, cx| this.open_menu_from_keys(OpenMenu::View, window, cx)))
            .on_action(cx.listener(|this, _: &OpenHelpMenu, window, cx| this.open_menu_from_keys(OpenMenu::Help, window, cx)))
            .on_action(cx.listener(|_, _: &FocusNext, window, cx| window.focus_next(cx)))
            .on_action(cx.listener(|_, _: &FocusPrevious, window, cx| window.focus_prev(cx)))
    }

    pub fn focus_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// A cached pane revealed this frame can miss the first accessibility tree rebuild; one extra
    /// notify on the next frame makes the reveal reliable for screen readers and UI Automation.
    fn refresh_revealed_views(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(50)).await;
            let _ = this.update(cx, |this, cx| {
                this.notify_views(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Gives the keyboard back to the editor after a command that did not move it there itself.
    /// With the editor hidden, the preview (the pane that must be showing) takes it instead.
    fn restore_editor_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.window_closing {
            return;
        }
        if self.settings.show_editor {
            self.focus_editor(window, cx);
            return;
        }
        if let Some(views) = self.active_views() {
            let preview = views.preview.clone();
            preview.update(cx, |pane, cx| pane.focus(window, cx));
            return;
        }
        self.focus_editor(window, cx);
    }

    // ----- Toolbar, tabs -----

    /// View toggles only; Open and Save live in the File menu and on Ctrl+O / Ctrl+S.
    pub fn toolbar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.dark;
        let editor = self.settings.show_editor;
        let preview = self.settings.preview_visible();
        let outline = self.settings.show_outline;
        div()
            .id("toolbar")
            .role(Role::Toolbar)
            .aria_label("Toolbar")
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .bg(color(dark, Color::Surface))
            .text_size(px(theme::CHROME_TEXT))
            .child(self.tool_button("tool-editor", "Editor", if editor { "icons/editor.svg" } else { "icons/editor-off.svg" }, Command::ToggleEditor, editor, cx))
            .child(self.tool_button("tool-preview", "Preview", if preview { "icons/preview.svg" } else { "icons/preview-off.svg" }, Command::TogglePreview, preview, cx))
            .child(self.tool_button("tool-outline", "Outline", "icons/outline.svg", Command::ToggleOutline, outline, cx))
    }

    /// A toolbar toggle button; `on` draws it pressed.
    fn tool_button(&self, id: &'static str, label: &'static str, icon: &'static str, command: Command, on: bool, cx: &mut Context<Self>) -> Stateful<gpui::Div> {
        let dark = self.dark;
        let text_color = color(dark, Color::Text);
        let background = if on { color(dark, Color::Pressed) } else { color(dark, Color::Surface) };
        let hover = if on { color(dark, Color::PressedHover) } else { color(dark, Color::Hover) };
        focus_ring(
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .aria_toggled(toggled(on))
                .focusable()
                .tab_stop(true)
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .px_2()
                .py_1()
                .rounded(px(theme::RADIUS_CONTROL))
                .bg(background)
                .when(on, |button| button.font_weight(gpui::FontWeight::BOLD))
                .hover(move |style| style.bg(hover))
                .text_color(text_color)
                .child(svg().path(icon).w(px(16.)).h(px(16.)).text_color(text_color))
                .child(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.run(command, window, cx);
                    if !command.moves_focus() {
                        this.restore_editor_focus(window, cx);
                    }
                })),
            dark,
        )
    }

    pub fn tab_strip(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.tabs.active();
        let dark = self.dark;
        let tabs: Vec<(String, bool)> = self.tabs.iter().map(|doc| (doc.display_name(), doc.is_dirty())).collect();
        let count = tabs.len();
        let row = div().id("tab-row").flex().flex_row().flex_1().min_w(px(0.)).overflow_x_scroll().children(
            tabs.into_iter().enumerate().map(|(index, (name, dirty))| {
                let label = if dirty { format!("{name}, unsaved changes") } else { name.clone() };
                let shown = if dirty { format!("{name} •") } else { name.clone() };
                let current = index == active;
                let background = if current { color(dark, Color::Card) } else { color(dark, Color::Surface) };
                div()
                    .id(("tab", index))
                    .role(Role::Tab)
                    .aria_label(SharedString::from(label))
                    .aria_selected(current)
                    .aria_position_in_set(index + 1)
                    .aria_size_of_set(count)
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .px_2()
                    .py_1()
                    .rounded_t(px(theme::RADIUS_CONTROL))
                    .bg(background)
                    .hover(move |style| style.bg(color(dark, Color::Hover)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.activate_tab(index, cx);
                        this.restore_editor_focus(window, cx);
                    }))
                    .child(shown)
                    .when(count > 1, |tab| {
                        tab.child(
                            div()
                                .id(("close-tab", index))
                                .role(Role::Button)
                                .aria_label(SharedString::from(format!("Close {name}")))
                                .pl_2()
                                .child("×")
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.close_tab(index, window, cx);
                                })),
                        )
                    })
            }),
        );
        let new_tab = focus_ring(
            div()
                .id("new-tab")
                .role(Role::Button)
                .aria_label("New tab")
                .focusable()
                .tab_stop(true)
                .flex_none()
                .px_3()
                .py_1()
                .hover(|style| style.bg(color(dark, Color::Hover)))
                .child("+"),
            dark,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.new_tab(cx);
            this.focus_editor(window, cx);
        }));
        div()
            .id("tabs")
            .role(Role::TabList)
            .aria_label("Open files")
            .flex()
            .flex_row()
            .flex_1()
            .min_w(px(0.))
            .items_center()
            .child(row)
            .child(new_tab)
    }

    pub fn find_bar(&mut self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let dark = self.dark;
        let find = self.find.as_ref()?;
        for field in [&find.query, &find.replacement] {
            field.update(cx, |field, _| field.dark = dark);
        }
        let selected = self.doc().selected.clone();
        let count = match (find.matches.len(), find.matches.iter().position(|found| *found == selected)) {
            (0, _) if find.query.read(cx).text().is_empty() => String::new(),
            (0, _) => "No results".to_string(),
            (total, Some(current)) => format!("{} of {total}", current + 1),
            (total, None) => format!("{total} matches"),
        };
        let button = |id: &'static str, label: &'static str, name: &'static str| {
            focus_ring(
                div()
                    .id(id)
                    .role(Role::Button)
                    .aria_label(name)
                    .focusable()
                    .tab_stop(true)
                    .px_2()
                    .py(px(3.))
                    .rounded(px(theme::RADIUS_CONTROL))
                    .bg(color(dark, Color::Card))
                    .text_color(color(dark, Color::Text))
                    .child(label),
                dark,
            )
        };
        Some(
            div()
                .id("find-bar")
                .role(Role::Search)
                .aria_label("Find and replace")
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_2()
                .px_3()
                .py_1()
                .text_size(px(theme::CHROME_TEXT))
                .bg(color(dark, Color::Surface))
                .text_color(quiet(dark))
                .child(find.query.clone())
                .child(div().id("find-count").role(Role::Status).aria_label(SharedString::from(count.clone())).min_w(px(80.)).child(count))
                .child(button("find-previous", "Previous", "Previous match").on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.find_step(-1, cx))))
                .child(button("find-next", "Next", "Next match").on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.find_step(1, cx))))
                .when(find.show_replace, |bar| {
                    bar.child(find.replacement.clone())
                        .child(button("replace-one", "Replace", "Replace").on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.replace_current(cx))))
                        .child(button("replace-all", "Replace all", "Replace all").on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.replace_all(cx))))
                })
                .child(button("find-close", "×", "Close find").on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.find_close(&FindClose, window, cx)))),
        )
    }

    // ----- Outline -----

    /// The last heading at or above the caret: the one the outline highlights. The outline itself
    /// is `OutlinePane`.
    pub(super) fn heading_at_caret(&self) -> Option<usize> {
        let caret_line = self.doc().buffer.line_of(self.doc().cursor()) + 1;
        self.doc().headings.iter().rposition(|heading| heading.line <= caret_line)
    }

    // ----- Status bar -----

    /// The app's footer (`status-bar.ts`): the document's path, the last error, caret position,
    /// counts, line ending, encoding and the two timings. Positions and counts are in UTF-16 units,
    /// like the app's JavaScript strings.
    pub fn status_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.dark;
        let doc = self.doc();
        let cursor = doc.cursor();
        let line = doc.buffer.line_of(cursor);
        let column = doc.buffer.utf16_of(cursor) - doc.buffer.utf16_of(doc.buffer.line_start(line)) + 1;
        let encoding = if doc.read_only {
            "Not UTF-8 (read-only)"
        } else if doc.meta.bom {
            "UTF-8 BOM"
        } else {
            "UTF-8"
        };
        let ending = match doc.meta.line_ending {
            files::LineEnding::Crlf => "CRLF",
            files::LineEnding::Lf => "LF",
        };
        let items = vec![
            (format!("Ln {}, Col {column}", line + 1), None),
            (format!("{} words, {} chars", grouped(doc.counts.words), grouped(doc.counts.chars)), None),
            (
                ending.to_string(),
                Some(match doc.meta.line_ending {
                    files::LineEnding::Crlf => "Windows (CRLF) line endings; kept when saving",
                    files::LineEnding::Lf => "Unix (LF) line endings; kept when saving",
                }),
            ),
            (
                encoding.to_string(),
                Some(if doc.read_only {
                    "The file is not valid UTF-8; saving is disabled"
                } else if doc.meta.bom {
                    "UTF-8 with a byte-order mark"
                } else {
                    "UTF-8 without a byte-order mark"
                }),
            ),
        ];
        let path = SharedString::from(doc.path.clone());
        let status = self.status.clone();
        div()
            .id("status-bar")
            .role(Role::Group)
            .aria_label("Status bar")
            .flex()
            .flex_row()
            .gap_4()
            .px_3()
            .py(px(3.))
            .text_size(px(theme::CAPTION_TEXT))
            .whitespace_nowrap()
            .bg(color(dark, Color::Surface))
            .text_color(quiet(dark))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::close_menu))
            // A long path gives way to the counts, and is cut at the start so the file name stays.
            // AccessKit names a label by its value.
            .child(div().id("path").role(Role::Label).aria_value(path.clone()).flex_1().min_w(px(0.)).overflow_hidden().text_ellipsis_start().child(path))
            // An alert, so a screen reader reads a failed save or open out when it happens.
            .children(status.map(|status| {
                div().id("status-message").role(Role::Alert).aria_label(SharedString::from(status.clone())).flex_none().text_color(color(dark, Color::Error)).child(status)
            }))
            .children(items.into_iter().enumerate().map(|(index, (item, hint))| status_item(index, item, hint, dark)))
    }

}

/// One quiet status-bar value, with a tooltip for the labels that are jargon.
fn status_item(index: usize, text: String, hint: Option<&'static str>, dark: bool) -> AnyElement {
    let item = div().id(("status", index)).role(Role::Label).aria_value(SharedString::from(text.clone())).flex_none().child(text);
    match hint {
        Some(hint) => item
            .tooltip(move |_, cx| {
                let text = SharedString::from(hint);
                cx.new(|_| Hint { text, dark }).into()
            })
            .into_any_element(),
        None => item.into_any_element(),
    }
}

struct Hint {
    text: SharedString,
    dark: bool,
}

impl Render for Hint {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .text_size(px(theme::CAPTION_TEXT))
            .rounded(px(theme::RADIUS_OVERLAY))
            .shadow_md()
            .bg(color(self.dark, Color::Card))
            .text_color(color(self.dark, Color::Text))
            .border_1()
            .border_color(color(self.dark, Color::StrokeStrong))
            .child(self.text.clone())
    }
}

/// A focusable control with the ring drawn in its border, which is transparent until the control
/// is focused from the keyboard, so the layout does not move.
pub fn focus_ring(element: Stateful<gpui::Div>, dark: bool) -> Stateful<gpui::Div> {
    let color = ring(dark);
    element.border_1().border_color(transparent_black()).focus_visible(move |style| style.border_color(color))
}

/// A check box or radio button: a box drawn in text, the label, and its state for assistive technology.
fn choice_row(id: &'static str, label: &'static str, role: Role, on: bool, dark: bool) -> Stateful<gpui::Div> {
    let mark = match (role, on) {
        (Role::RadioButton, true) => "◉",
        (Role::RadioButton, false) => "○",
        (_, true) => "☑",
        (_, false) => "☐",
    };
    focus_ring(
        div()
            .id(id)
            .role(role)
            .aria_label(label)
            .aria_toggled(toggled(on))
            .focusable()
            .tab_stop(true)
            .flex()
            .flex_row()
            .gap_2()
            .py(px(2.))
            .cursor_pointer()
            .child(mark)
            .child(label),
        dark,
    )
}

/// Thousands separated with commas, like `toLocaleString()` in an English locale.
fn grouped(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_grouped_by_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1000), "1,000");
        assert_eq!(grouped(1234567), "1,234,567");
    }

    #[test]
    fn every_menu_fits_the_focus_handles() {
        for (menu, _) in MENUS {
            assert!(entries(menu).len() <= MENU_ITEMS);
        }
    }

    /// A new settings control that reuses a slot, or adds one past `COUNT`, would make Tab skip it
    /// or move the focus nowhere; both are silent at runtime.
    #[test]
    fn every_settings_slot_is_distinct_and_within_count() {
        let slots = [
            settings_slot::THEME_SYSTEM,
            settings_slot::THEME_LIGHT,
            settings_slot::THEME_DARK,
            settings_slot::FONT_SMALLER,
            settings_slot::FONT_LARGER,
            settings_slot::WORD_WRAP,
            settings_slot::TYPOGRAPHER,
            settings_slot::AUTO_SAVE,
            settings_slot::LINE_LF,
            settings_slot::LINE_CRLF,
            settings_slot::SHOW_EDITOR,
            settings_slot::SHOW_PREVIEW,
            settings_slot::SHOW_OUTLINE,
            settings_slot::CLOSE,
        ];
        let mut seen = slots.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), slots.len(), "two controls share a focus slot");
        assert_eq!(seen, (0..settings_slot::COUNT).collect::<Vec<_>>(), "slots skip or exceed COUNT");
    }
}
