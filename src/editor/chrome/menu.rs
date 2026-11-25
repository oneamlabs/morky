//! The menu bar: the File, Edit and View menus, opening and closing them, and moving through them
//! with the arrows or an underlined access key.
//!
//! Menu mnemonics are actions, not key listeners, so GPUI consumes the key-down and Windows does not
//! also type the letter into the editor (see `MenuAccessKey`).

use super::*;

/// Room for the longest menu: one focus handle per item, so the arrows can move between them.
pub const MENU_ITEMS: usize = 10;

pub const MENUS: [(OpenMenu, &str); 3] = [(OpenMenu::File, "File"), (OpenMenu::Edit, "Edit"), (OpenMenu::View, "View")];

pub struct Entry {
    pub label: &'static str,
    pub shortcut: &'static str,
    /// The underlined letter that activates the item while its menu is open, as on Windows.
    pub access: char,
    pub command: Command,
}

pub const fn entry(label: &'static str, shortcut: &'static str, access: char, command: Command) -> Entry {
    Entry { label, shortcut, access, command }
}

/// The app's menus with their shortcut hints. Larger and Smaller text are Marky's own.
pub fn entries(menu: OpenMenu) -> &'static [Entry] {
    const FILE: [Entry; 10] = [
        entry("New Tab", "Ctrl+N", 'n', Command::NewTab),
        entry("Open…", "Ctrl+O", 'o', Command::Open),
        entry("Open Recent…", "Ctrl+R", 'r', Command::OpenRecent),
        entry("Save", "Ctrl+S", 's', Command::Save),
        entry("Save As…", "Ctrl+Shift+S", 'a', Command::SaveAs),
        entry("Auto Save", "", 'u', Command::ToggleAutoSave),
        entry("Settings…", "Ctrl+,", 't', Command::Settings),
        entry("Close Tab", "Ctrl+W", 'c', Command::CloseTab),
        entry("About Marky", "", 'b', Command::About),
        entry("Quit", "", 'q', Command::Quit),
    ];
    const EDIT: [Entry; 8] = [
        entry("Undo", "Ctrl+Z", 'u', Command::Undo),
        entry("Redo", "Ctrl+Y", 'r', Command::Redo),
        entry("Cut", "Ctrl+X", 'c', Command::Cut),
        entry("Copy", "Ctrl+C", 'o', Command::Copy),
        entry("Paste", "Ctrl+V", 'p', Command::Paste),
        entry("Select All", "Ctrl+A", 'a', Command::SelectAll),
        entry("Find", "Ctrl+F", 'f', Command::Find),
        entry("Replace", "Ctrl+H", 'e', Command::Replace),
    ];
    const VIEW: [Entry; 8] = [
        entry("Editor", "Ctrl+Shift+E", 'e', Command::ToggleEditor),
        entry("Preview", "Ctrl+Shift+V", 'p', Command::TogglePreview),
        entry("Outline", "Ctrl+Shift+O", 'o', Command::ToggleOutline),
        entry("Word Wrap", "Alt+Z", 'w', Command::ToggleWordWrap),
        entry("Next Tab", "Ctrl+Tab", 'n', Command::NextTab),
        entry("Previous Tab", "Ctrl+Shift+Tab", 'v', Command::PreviousTab),
        entry("Larger Text", "", 'l', Command::LargerText),
        entry("Smaller Text", "", 'm', Command::SmallerText),
    ];
    match menu {
        OpenMenu::File => &FILE,
        OpenMenu::Edit => &EDIT,
        OpenMenu::View => &VIEW,
    }
}

impl Prototype {
    pub fn menu_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut bar = div()
            .id("menu-bar")
            .role(Role::MenuBar)
            .aria_label("Menu")
            .flex()
            .flex_row()
            .items_center()
            .bg(tone(self.dark, 0x27272a, 0xf4f4f5))
            .text_size(px(13.));
        for (menu, label) in MENUS {
            bar = bar.child(self.menu_button(label, menu, cx));
        }
        bar
    }

    fn menu_button(&mut self, label: &'static str, menu: OpenMenu, cx: &mut Context<Self>) -> Stateful<gpui::Div> {
        let opened = self.open_menu == Some(menu);
        let dark = self.dark;
        let button = div()
            .id(label)
            .role(Role::MenuItem)
            .aria_label(label)
            .aria_expanded(opened)
            .px_3()
            .py_1()
            .when(opened, |button| button.bg(tone(dark, 0x3f3f46, 0xe4e4e7)))
            .child(menu_label(label, label.chars().next().unwrap_or(' ')))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if this.open_menu == Some(menu) {
                    this.close_menus(window, cx);
                } else {
                    this.open_menu_at(menu, event.is_keyboard(), window, cx);
                }
            }))
            // A screen reader opens and closes a menu with UI Automation's Expand and Collapse.
            .on_a11y_action(gpui::AccessibleAction::Expand, {
                let view = cx.entity().downgrade();
                move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.open_menu_at(menu, true, window, cx));
                }
            })
            .on_a11y_action(gpui::AccessibleAction::Collapse, {
                let view = cx.entity().downgrade();
                move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.close_menus(window, cx));
                }
            });
        if !opened {
            return button;
        }
        // The list is taken out of the bar's height and painted after the panes, so it covers the tab instead of pushing it down.
        button.child(deferred(self.menu_items(menu, cx).absolute().top_full().left_0().occlude()))
    }

    /// Alt+F, Alt+E, Alt+V and F10 open a menu with its first item focused; the arrows move from there.
    pub(super) fn open_menu_from_keys(&mut self, menu: OpenMenu, window: &mut Window, cx: &mut Context<Self>) {
        self.open_menu_at(menu, true, window, cx);
    }

    pub(super) fn open_menu_at(&mut self, menu: OpenMenu, focus_first: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.open_menu = Some(menu);
        window.focus(if focus_first { &self.menu_focus[0] } else { &self.menu_root }, cx);
        cx.notify();
    }

    /// Closes any open menu. If the focus was in it, it goes back to the editor.
    pub fn close_menus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.open_menu.take() else { return };
        if self.menu_root.is_focused(window) || self.menu_focus[..entries(menu).len()].iter().any(|handle| handle.is_focused(window)) {
            self.restore_editor_focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn menu_step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.open_menu else { return };
        let count = entries(menu).len() as isize;
        let current = self.menu_focus[..count as usize].iter().position(|handle| handle.is_focused(window)).map_or(-1, |index| index as isize);
        let next = if current < 0 && delta < 0 { count - 1 } else { (current + delta).rem_euclid(count) };
        window.focus(&self.menu_focus[next as usize], cx);
    }

    /// Left and Right move to the neighbouring menu, as in Windows menus.
    pub(super) fn menu_switch(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.open_menu else { return };
        let index = MENUS.iter().position(|(candidate, _)| *candidate == menu).unwrap_or(0) as isize;
        let (next, _) = MENUS[(index + delta).rem_euclid(MENUS.len() as isize) as usize];
        self.open_menu_at(next, true, window, cx);
    }

    /// A letter pressed in an open menu runs the item whose underlined key it is.
    pub(super) fn menu_access_key(&mut self, key: char, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.open_menu else { return };
        let Some(entry) = entries(menu).iter().find(|entry| entry.access.eq_ignore_ascii_case(&key)) else { return };
        let command = entry.command;
        self.run(command, window, cx);
        if !command.moves_focus() {
            self.restore_editor_focus(window, cx);
        }
    }

    fn checked(&self, command: Command) -> Option<bool> {
        match command {
            Command::ToggleEditor => Some(self.settings.show_editor),
            Command::TogglePreview => Some(self.settings.preview_visible()),
            Command::ToggleOutline => Some(self.settings.show_outline),
            Command::ToggleWordWrap => Some(self.settings.word_wrap),
            Command::ToggleAutoSave => Some(self.settings.auto_save),
            _ => None,
        }
    }

    fn menu_items(&mut self, menu: OpenMenu, cx: &mut Context<Self>) -> Stateful<gpui::Div> {
        let dark = self.dark;
        let highlight = tone(dark, 0x52525b, 0xdbeafe);
        let column = div()
            .id("menu-items")
            .key_context("Menu")
            .track_focus(&self.menu_root)
            .role(Role::Menu)
            .flex()
            .flex_col()
            .py_1()
            .bg(tone(dark, 0x3f3f46, 0xffffff))
            .border_1()
            .border_color(tone(dark, 0x52525b, 0xd4d4d8))
            .on_action(cx.listener(|this, _: &MenuNext, window, cx| this.menu_step(1, window, cx)))
            .on_action(cx.listener(|this, _: &MenuPrevious, window, cx| this.menu_step(-1, window, cx)))
            .on_action(cx.listener(|this, _: &MenuLeft, window, cx| this.menu_switch(-1, window, cx)))
            .on_action(cx.listener(|this, _: &MenuRight, window, cx| this.menu_switch(1, window, cx)))
            // The underlined letter of an item activates it while the menu is open.
            .on_action(cx.listener(|this, action: &MenuAccessKey, window, cx| this.menu_access_key(action.0, window, cx)));
        let items = entries(menu).iter().enumerate().map(|(index, entry)| {
            let command = entry.command;
            let checked = self.checked(command);
            div()
                .id(("menu-item", index))
                .track_focus(&self.menu_focus[index])
                .role(if checked.is_some() { Role::MenuItemCheckBox } else { Role::MenuItem })
                .aria_label(entry.label)
                .when_some(checked, |item, on| item.aria_toggled(toggled(on)))
                .hover(|style| style.bg(highlight))
                .focus(|style| style.bg(highlight))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.run(command, window, cx);
                    if !command.moves_focus() {
                        this.focus_editor(window, cx);
                    }
                }))
                .child(menu_row(entry.label, entry.shortcut, entry.access, checked, dark))
        });
        column.children(items.collect::<Vec<_>>())
    }
}

/// A label with its access key underlined, as a Windows menu shows it.
pub fn menu_label(label: &str, access: char) -> gpui::Div {
    let Some((start, matched)) = label.char_indices().find(|(_, ch)| ch.eq_ignore_ascii_case(&access)) else {
        return div().child(label.to_string());
    };
    let end = start + matched.len_utf8();
    div()
        .flex()
        .flex_row()
        .child(label[..start].to_string())
        .child(div().underline().child(label[start..end].to_string()))
        .child(label[end..].to_string())
}

pub fn menu_row(label: &str, shortcut: &str, access: char, checked: Option<bool>, dark: bool) -> gpui::Div {
    let check = match checked {
        Some(true) => "✓",
        _ => "",
    };
    div()
        .flex()
        .flex_row()
        .gap_2()
        .px_3()
        .py_1()
        .w(px(240.))
        .text_color(tone(dark, 0xfafafa, 0x18181b))
        .child(div().w(px(12.)).child(check))
        .child(menu_label(label, access).flex_1())
        // Lighter than `quiet` in the dark theme: the menu's background is lighter than the bars'.
        .child(div().text_color(tone(dark, 0xd4d4d8, 0x52525b)).child(shortcut.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_menu_fits_the_focus_handles() {
        for (menu, _) in MENUS {
            assert!(entries(menu).len() <= MENU_ITEMS);
        }
    }

    /// Every access key must be unique within its menu, or the second item becomes unreachable.
    #[test]
    fn access_keys_are_unique_within_a_menu() {
        for (menu, label) in MENUS {
            let mut keys: Vec<char> = entries(menu).iter().map(|entry| entry.access.to_ascii_lowercase()).collect();
            keys.sort_unstable();
            let before = keys.len();
            keys.dedup();
            assert_eq!(keys.len(), before, "{label} has a duplicate access key");
        }
    }
}
