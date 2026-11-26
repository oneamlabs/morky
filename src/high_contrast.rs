//! Windows high-contrast themes. GPUI reports only light or dark, so Marky reads the system colours
//! itself and remaps its palette onto them while high contrast is on.
//!
//! The mapping is by role, not by guessing from a colour's brightness: a surface becomes
//! `COLOR_WINDOW` or `COLOR_BTNFACE`, text becomes `COLOR_WINDOWTEXT` (or `COLOR_GRAYTEXT` when it
//! is meant to be quiet), and accents, links, selections and find matches become `COLOR_HOTLIGHT`
//! or `COLOR_HIGHLIGHT`. Exact shades are the theme's job, not the app's.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{Rgba, rgb};
use windows_sys::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HOTLIGHT, COLOR_WINDOW, COLOR_WINDOWTEXT, GetSysColor,
};
use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETHIGHCONTRAST, SystemParametersInfoW};

use crate::theme::Color;

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemColors {
    pub window: u32,
    pub window_text: u32,
    pub button_face: u32,
    pub highlight: u32,
    pub gray_text: u32,
    pub hotlight: u32,
}

static ACTIVE: AtomicBool = AtomicBool::new(false);
static WINDOW: AtomicU32 = AtomicU32::new(0);
static WINDOW_TEXT: AtomicU32 = AtomicU32::new(0);
static BUTTON_FACE: AtomicU32 = AtomicU32::new(0);
static HIGHLIGHT: AtomicU32 = AtomicU32::new(0);
static GRAY_TEXT: AtomicU32 = AtomicU32::new(0);
static HOTLIGHT: AtomicU32 = AtomicU32::new(0);
static LAST_REFRESH: AtomicU32 = AtomicU32::new(0);

fn palette() -> SystemColors {
    let get = |value: &AtomicU32| value.load(Ordering::Relaxed);
    SystemColors {
        window: get(&WINDOW),
        window_text: get(&WINDOW_TEXT),
        button_face: get(&BUTTON_FACE),
        highlight: get(&HIGHLIGHT),
        gray_text: get(&GRAY_TEXT),
        hotlight: get(&HOTLIGHT),
    }
}

/// Reads the current theme. Called at startup and at most once a second from the frame loop, so a
/// theme switch applies without restarting the app. Returns whether the palette changed, so cached
/// views can be repainted.
pub fn refresh() -> bool {
    let before = snapshot();
    let mut contrast = HIGHCONTRASTW { cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cbSize,
            &mut contrast as *mut HIGHCONTRASTW as *mut core::ffi::c_void,
            0,
        )
    };
    let active = ok != 0 && contrast.dwFlags & HCF_HIGHCONTRASTON != 0;
    ACTIVE.store(active, Ordering::Relaxed);
    if active {
        let color = |index| from_colorref(unsafe { GetSysColor(index) });
        WINDOW.store(color(COLOR_WINDOW), Ordering::Relaxed);
        WINDOW_TEXT.store(color(COLOR_WINDOWTEXT), Ordering::Relaxed);
        BUTTON_FACE.store(color(COLOR_BTNFACE), Ordering::Relaxed);
        HIGHLIGHT.store(color(COLOR_HIGHLIGHT), Ordering::Relaxed);
        GRAY_TEXT.store(color(COLOR_GRAYTEXT), Ordering::Relaxed);
        HOTLIGHT.store(color(COLOR_HOTLIGHT), Ordering::Relaxed);
    }
    before != snapshot()
}

fn snapshot() -> (bool, u32, u32, u32, u32, u32, u32) {
    let get = |value: &AtomicU32| value.load(Ordering::Relaxed);
    (ACTIVE.load(Ordering::Relaxed), get(&WINDOW), get(&WINDOW_TEXT), get(&BUTTON_FACE), get(&HIGHLIGHT), get(&GRAY_TEXT), get(&HOTLIGHT))
}

/// As `refresh`, but only reads the system once a second. Returns whether the palette changed.
pub fn refresh_periodically() -> bool {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map(|time| time.as_secs() as u32).unwrap_or(0);
    if LAST_REFRESH.swap(seconds, Ordering::Relaxed) != seconds {
        return refresh();
    }
    false
}

/// Whether a high-contrast theme is on. The accent colour defers to it (`accent::refresh`).
#[allow(dead_code)]
pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// A syntax token's colour while high contrast is on, or `None` while it is off. Code colours are a
/// highlighter theme rather than the UI palette, so they are mapped here to the two system colours
/// that mean "link" and "text".
pub fn syntax_color(token: crate::syntax::Token) -> Option<Rgba> {
    use crate::syntax::Token;
    if !ACTIVE.load(Ordering::Relaxed) {
        return None;
    }
    Some(match token {
        Token::Link => rgb(HOTLIGHT.load(Ordering::Relaxed)),
        _ => rgb(WINDOW_TEXT.load(Ordering::Relaxed)),
    })
}

/// The colour for a role: the theme's own value, or the system colour that role maps to while high
/// contrast is on.
pub fn resolve(role: Color, value: u32) -> Rgba {
    if !ACTIVE.load(Ordering::Relaxed) {
        return rgb(value);
    }
    rgb(map(role, &palette()))
}

/// Maps one role onto the system palette. Pure, so the rules can be tested without a high-contrast
/// theme installed.
fn map(role: Color, palette: &SystemColors) -> u32 {
    match role {
        // Surfaces: the panes become the window; chrome and cards become button faces.
        Color::Base | Color::Editor | Color::Preview | Color::Field => palette.window,
        Color::Surface | Color::Card | Color::MenuBar | Color::MenuBarActive => palette.button_face,

        // Text.
        Color::Text | Color::PreviewText => palette.window_text,
        Color::TextQuiet | Color::TextDisabled | Color::MenuShortcut => palette.gray_text,

        // Lines between things stay visible.
        Color::Stroke | Color::StrokeStrong | Color::QuoteBorder => palette.window_text,

        // Accents: a focus ring, the caret or a link is a foreground; a blue selection is a
        // background.
        Color::Accent | Color::AccentText | Color::FocusRing | Color::Caret | Color::Link => palette.hotlight,
        Color::Highlight | Color::AccentSubtle | Color::Hover | Color::Pressed | Color::PressedHover => palette.highlight,

        // Highlights.
        Color::Selection | Color::FindMatch | Color::Mark => palette.highlight,

        // An error has no system slot; the hot light is the theme's warning colour.
        Color::Error => palette.hotlight,

        // Code reads as chrome in a high-contrast theme.
        Color::CodeInline | Color::CodeBlock | Color::Scrollbar => palette.button_face,
    }
}

/// `COLORREF` is `0x00BBGGRR`; the app's colours are `0xRRGGBB`.
fn from_colorref(value: u32) -> u32 {
    let red = (value & 0xFF) << 16;
    let green = value & 0xFF00;
    let blue = (value >> 16) & 0xFF;
    red | green | blue
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> SystemColors {
        SystemColors {
            window: 0x000000,
            window_text: 0xffffff,
            button_face: 0x303030,
            highlight: 0x00ffff,
            gray_text: 0x808080,
            hotlight: 0x00a0ff,
        }
    }

    #[test]
    fn surfaces_and_text_take_the_system_colours() {
        let palette = palette();
        assert_eq!(map(Color::Base, &palette), palette.window);
        assert_eq!(map(Color::Editor, &palette), palette.window);
        assert_eq!(map(Color::Preview, &palette), palette.window);
        assert_eq!(map(Color::Surface, &palette), palette.button_face);
        assert_eq!(map(Color::Card, &palette), palette.button_face);
        assert_eq!(map(Color::Text, &palette), palette.window_text);
        assert_eq!(map(Color::TextQuiet, &palette), palette.gray_text);
        assert_eq!(map(Color::TextDisabled, &palette), palette.gray_text);
    }

    #[test]
    fn accents_and_selections_are_highlighted() {
        let palette = palette();
        assert_eq!(map(Color::FocusRing, &palette), palette.hotlight);
        assert_eq!(map(Color::Accent, &palette), palette.hotlight);
        assert_eq!(map(Color::Caret, &palette), palette.hotlight);
        assert_eq!(map(Color::Link, &palette), palette.hotlight);
        assert_eq!(map(Color::Selection, &palette), palette.highlight);
        assert_eq!(map(Color::FindMatch, &palette), palette.highlight);
        assert_eq!(map(Color::Mark, &palette), palette.highlight);
    }

    #[test]
    fn colorref_is_converted_to_rgb() {
        assert_eq!(from_colorref(0x000000FF), 0xFF0000);
        assert_eq!(from_colorref(0x00FF0000), 0x0000FF);
    }
}
