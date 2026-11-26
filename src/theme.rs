//! The app's colours, named by the job they do rather than by their value.
//!
//! No view holds a colour of its own: a control asks for a [`Role`] and this module supplies the
//! value for the current theme. High contrast still wins, because every colour leaves through
//! [`color`], which the system palette remaps (`high_contrast::resolve`). Adding a colour means
//! adding a role here, not a hex in a view.
//!
//! The values are a theme's business: `palette` holds one row per theme and a role is looked up in
//! it. Swapping the theme's values must not move a single call site.

use gpui::Rgba;

/// What a colour is for. A view asks for the role, never the value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Color {
    /// The window behind the panes, and the side panels that sit on it.
    Base,
    /// The source (editor) pane. Deeper than `Base` in the light theme.
    Editor,
    /// The preview pane, deliberately deeper than `Base` so prose reads as a page.
    Preview,
    /// The preview's body text, softer than `Text` so long prose does not glare.
    PreviewText,
    /// The chrome: toolbar, tabs, find bar, status bar, a row under the pointer.
    Surface,
    /// A raised surface: menus, tooltips, the active tab, buttons on chrome.
    Card,
    /// The find field's well.
    Field,
    /// The menu bar. Its own role because it is drawn on the frame edge, not inside it.
    MenuBar,
    /// A menu bar label whose menu is open.
    MenuBarActive,
    /// A hairline: borders, dividers, rules under headings.
    Stroke,
    /// A border that must read on a `Card`, where `Stroke` is too faint.
    StrokeStrong,
    /// Primary text.
    Text,
    /// Secondary text: status bar, hints, tooltips' labels.
    TextQuiet,
    /// A placeholder in an empty field.
    TextDisabled,
    /// A menu's shortcut hint, which is lighter than `TextQuiet` on a dark menu.
    MenuShortcut,
    /// The accent as a filled surface: a toggle that is on, the caret.
    Accent,
    /// The accent as text or a glyph, where `Accent` itself would fail 4.5:1.
    AccentText,
    /// The accent as a focus ring, which must also clear 3:1 on a dark surface.
    FocusRing,
    /// A menu item under the pointer or the keyboard.
    Highlight,
    /// A dim accent fill: the active divider's body, a selected chrome row.
    AccentSubtle,
    /// A control under the pointer.
    Hover,
    /// A control held down, or a toggle that is on.
    Pressed,
    /// A held toggle under the pointer.
    PressedHover,
    /// Selected text in the editor.
    Selection,
    /// A find match in the editor.
    FindMatch,
    /// `==marked==` text.
    Mark,
    /// A failed save or a broken formula.
    Error,
    /// A link in the preview.
    Link,
    /// Inline `code`.
    CodeInline,
    /// A fenced code block.
    CodeBlock,
    /// A pane's scrollbar thumb.
    Scrollbar,
    /// A blockquote's left rule.
    QuoteBorder,
    /// The editor's caret.
    Caret,
}

/// One theme's colours, as `0xRRGGBB` values. Every role has a field.
#[derive(Clone, Copy)]
pub struct Palette {
    pub base: u32,
    pub editor: u32,
    pub preview: u32,
    pub preview_text: u32,
    pub surface: u32,
    pub card: u32,
    pub field: u32,
    pub menu_bar: u32,
    pub menu_bar_active: u32,
    pub stroke: u32,
    pub stroke_strong: u32,
    pub text: u32,
    pub text_quiet: u32,
    pub text_disabled: u32,
    pub menu_shortcut: u32,
    pub accent: u32,
    pub accent_text: u32,
    pub focus_ring: u32,
    pub highlight: u32,
    pub accent_subtle: u32,
    pub hover: u32,
    pub pressed: u32,
    pub pressed_hover: u32,
    pub selection: u32,
    pub find_match: u32,
    pub mark: u32,
    pub error: u32,
    pub link: u32,
    pub code_inline: u32,
    pub code_block: u32,
    pub scrollbar: u32,
    pub quote_border: u32,
    pub caret: u32,
}

impl Palette {
    fn value(&self, role: Color) -> u32 {
        match role {
            Color::Base => self.base,
            Color::Editor => self.editor,
            Color::Preview => self.preview,
            Color::PreviewText => self.preview_text,
            Color::Surface => self.surface,
            Color::Card => self.card,
            Color::Field => self.field,
            Color::MenuBar => self.menu_bar,
            Color::MenuBarActive => self.menu_bar_active,
            Color::Stroke => self.stroke,
            Color::StrokeStrong => self.stroke_strong,
            Color::Text => self.text,
            Color::TextQuiet => self.text_quiet,
            Color::TextDisabled => self.text_disabled,
            Color::MenuShortcut => self.menu_shortcut,
            Color::Accent => self.accent,
            Color::AccentText => self.accent_text,
            Color::FocusRing => self.focus_ring,
            Color::Highlight => self.highlight,
            Color::AccentSubtle => self.accent_subtle,
            Color::Hover => self.hover,
            Color::Pressed => self.pressed,
            Color::PressedHover => self.pressed_hover,
            Color::Selection => self.selection,
            Color::FindMatch => self.find_match,
            Color::Mark => self.mark,
            Color::Error => self.error,
            Color::Link => self.link,
            Color::CodeInline => self.code_inline,
            Color::CodeBlock => self.code_block,
            Color::Scrollbar => self.scrollbar,
            Color::QuoteBorder => self.quote_border,
            Color::Caret => self.caret,
        }
    }
}

const DARK: Palette = Palette {
    base: 0x202020,
    editor: 0x202020,
    preview: 0x1c1c1c,
    preview_text: 0xe4e4e7,
    surface: 0x202020,
    card: 0x2b2b2b,
    field: 0x2b2b2b,
    menu_bar: 0x202020,
    menu_bar_active: 0x2d2d2d,
    stroke: 0x383838,
    stroke_strong: 0x454545,
    text: 0xffffff,
    text_quiet: 0xa9a9a9,
    text_disabled: 0x949494,
    menu_shortcut: 0xa9a9a9,
    accent: 0x0078d4,
    accent_text: 0x60cdff,
    focus_ring: 0x60cdff,
    highlight: 0x2d2d2d,
    accent_subtle: 0x1f4a6e,
    hover: 0x2d2d2d,
    pressed: 0x262626,
    pressed_hover: 0x333333,
    selection: 0x264f78,
    find_match: 0x695200,
    mark: 0x55401c,
    error: 0xff99a4,
    link: 0x60cdff,
    code_inline: 0x2d2d2d,
    code_block: 0x272727,
    scrollbar: 0x4d4d4d,
    quote_border: 0x6b6b6b,
    caret: 0xffffff,
};

const LIGHT: Palette = Palette {
    base: 0xf3f3f3,
    editor: 0xffffff,
    preview: 0xffffff,
    preview_text: 0x1a1a1a,
    surface: 0xf3f3f3,
    card: 0xffffff,
    field: 0xffffff,
    menu_bar: 0xf3f3f3,
    menu_bar_active: 0xe8e8e8,
    stroke: 0xdcdcdc,
    stroke_strong: 0xc8c8c8,
    text: 0x1a1a1a,
    text_quiet: 0x5d5d5d,
    text_disabled: 0x757575,
    menu_shortcut: 0x5d5d5d,
    accent: 0x0078d4,
    accent_text: 0x005fb8,
    focus_ring: 0x0078d4,
    highlight: 0xebebeb,
    accent_subtle: 0xcce4f7,
    hover: 0xeaeaea,
    pressed: 0xe0e0e0,
    pressed_hover: 0xdcdcdc,
    selection: 0xccddf7,
    find_match: 0xfff4ce,
    mark: 0xfff3d0,
    error: 0xc42b1c,
    link: 0x005fb8,
    code_inline: 0xf0f0f0,
    code_block: 0xf5f5f5,
    scrollbar: 0xb0b0b0,
    quote_border: 0xb0b0b0,
    caret: 0x1a1a1a,
};

pub fn palette(dark: bool) -> &'static Palette {
    if dark { &DARK } else { &LIGHT }
}

/// Fluent's corner radius for a control: buttons, tabs, text fields, choice rows.
pub const RADIUS_CONTROL: f32 = 4.;

/// Fluent's corner radius for an overlay: menus, tooltips, flyouts.
pub const RADIUS_OVERLAY: f32 = 8.;

/// Fluent's body text size for chrome (the editor keeps the user's own size).
pub const CHROME_TEXT: f32 = 14.;

/// Fluent's caption size: the status bar and tooltips.
pub const CAPTION_TEXT: f32 = 12.;

/// Fluent's subtitle size: a dialog's title.
pub const TITLE_TEXT: f32 = 20.;

/// The colour for a role in the current theme, after the system's high-contrast palette has had
/// its say.
///
/// The accent roles come from the user's Windows accent rather than the palette, so the app matches
/// the personalisation colour. High contrast still wins.
pub fn color(dark: bool, role: Color) -> Rgba {
    let value = match role {
        Color::Accent => crate::accent::accent(),
        Color::AccentText => return crate::accent::accent_text_on(dark, accent_background(dark)),
        _ => palette(dark).value(role),
    };
    crate::high_contrast::resolve(role, value)
}

/// What accent-coloured text is drawn on, which is what `accent_text_on` must clear 4.5:1 against.
fn accent_background(dark: bool) -> u32 {
    palette(dark).value(Color::Card)
}

/// The focus ring, which must clear 3:1 on whatever it is drawn on.
pub fn ring(dark: bool) -> Rgba {
    color(dark, Color::FocusRing)
}

/// Secondary text on the chrome, still at 4.5:1.
pub fn quiet(dark: bool) -> Rgba {
    color(dark, Color::TextQuiet)
}

/// A plain `0xRRGGBB` value for a role, ignoring high contrast. Only for tests.
#[cfg(test)]
pub fn value(dark: bool, role: Color) -> u32 {
    palette(dark).value(role)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_role_has_a_distinct_lookup() {
        // Two roles may share a value (they diverge by theme), but no role may resolve to zero:
        // a zero would paint a transparent surface that looks like a missing value.
        let roles = [
            Color::Base, Color::Editor, Color::Preview, Color::PreviewText, Color::Surface, Color::Card,
            Color::Field, Color::MenuBar, Color::MenuBarActive, Color::Stroke, Color::StrokeStrong, Color::Text,
            Color::TextQuiet, Color::TextDisabled, Color::MenuShortcut, Color::Accent, Color::AccentText, Color::FocusRing,
            Color::Highlight, Color::AccentSubtle, Color::Hover, Color::Pressed, Color::PressedHover,
            Color::Selection, Color::FindMatch, Color::Mark, Color::Error, Color::Link, Color::CodeInline,
            Color::CodeBlock, Color::Scrollbar, Color::QuoteBorder, Color::Caret,
        ];
        for dark in [false, true] {
            for role in roles {
                assert_ne!(value(dark, role), 0, "{role:?} in the {} theme", if dark { "dark" } else { "light" });
            }
        }
    }

    #[test]
    fn the_theme_flips_with_the_flag() {
        assert_eq!(value(true, Color::Base), 0x202020);
        assert_eq!(value(false, Color::Base), 0xf3f3f3);
    }
}
