//! WCAG 2.2 contrast for Marky's colours, in both themes: 4.5:1 for text (AA), 3:1 for the focus
//! ring against what it is drawn on.
//!
//! These read `theme`'s real values rather than a copy, so changing a token is caught here. A test
//! that asserts its own list would pass while the UI failed, which is the bug this replaces.

#![cfg(test)]

use gpui::{Rgba, rgb};

use crate::syntax::Token;
use crate::theme::{self, Color, Palette};

fn luminance(color: Rgba) -> f32 {
    let channel = |value: f32| if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) };
    0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
}

fn ratio(a: u32, b: u32) -> f32 {
    let (a, b) = (luminance(rgb(a)), luminance(rgb(b)));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// Every (what, text, background) pair the UI actually draws, by role.
fn text_pairs(palette: &Palette) -> Vec<(&'static str, u32, u32)> {
    vec![
        ("text on the editor", palette.text, palette.editor),
        ("text on the base surface", palette.text, palette.base),
        ("preview body on the preview", palette.preview_text, palette.preview),
        ("text on the toolbar and tabs", palette.text, palette.surface),
        ("text on the menu bar", palette.text, palette.menu_bar),
        ("text on a menu", palette.text, palette.card),
        ("text on a highlighted menu item", palette.text, palette.highlight),
        ("menu shortcut hints", palette.menu_shortcut, palette.card),
        ("shortcut hints on a highlighted item", palette.menu_shortcut, palette.highlight),
        ("quiet text on the path and status bars", palette.text_quiet, palette.surface),
        ("quiet text on the outline and settings", palette.text_quiet, palette.base),
        ("quiet text on the preview", palette.text_quiet, palette.preview),
        ("text on a card tooltip", palette.text, palette.card),
        ("a placeholder in a field", palette.text_disabled, palette.field),
        ("text typed into a field", palette.text, palette.field),
        ("the current heading in the outline", palette.accent_text, palette.base),
        ("accent text on a card", palette.accent_text, palette.card),
        ("links in the preview", palette.link, palette.preview),
        ("errors on the status bar", palette.error, palette.surface),
        ("errors in the preview", palette.error, palette.preview),
        ("text on a selection", palette.text, palette.selection),
        ("text on a find match", palette.text, palette.find_match),
        ("text on ==marked== text", palette.text, palette.mark),
        ("text on inline code", palette.text, palette.code_inline),
        ("text on a code block", palette.preview_text, palette.code_block),
        ("a menu bar item under the pointer", palette.text, palette.highlight),
        ("a selected outline row", palette.text, palette.surface),
        ("a tab's label on the active tab", palette.text, palette.card),
    ]
}

#[test]
fn text_meets_aa_in_both_themes() {
    let mut failures = Vec::new();
    for dark in [false, true] {
        let palette = theme::palette(dark);
        for (what, text, background) in text_pairs(palette) {
            let contrast = ratio(text, background);
            if contrast < 4.5 {
                failures.push(format!("{} theme, {what}: {contrast:.2}:1", if dark { "dark" } else { "light" }));
            }
        }
    }
    assert!(failures.is_empty(), "below 4.5:1:\n{}", failures.join("\n"));
}

#[test]
fn syntax_colours_meet_aa_on_the_editor() {
    let tokens = [
        Token::Heading,
        Token::Link,
        Token::Code,
        Token::Quote,
        Token::Muted,
        Token::Strong,
        Token::Emphasis,
        Token::Strike,
        Token::Keyword,
        Token::String,
        Token::Number,
        Token::Comment,
        Token::Function,
        Token::Type,
        Token::Property,
    ];
    let mut failures = Vec::new();
    for dark in [false, true] {
        let background = rgb(theme::value(dark, Color::Editor));
        for token in tokens {
            let Some(color) = token.style(dark).color else { continue };
            let contrast = {
                let (a, b) = (luminance(Rgba::from(color)), luminance(background));
                (a.max(b) + 0.05) / (a.min(b) + 0.05)
            };
            if contrast < 4.5 {
                failures.push(format!("{} theme, {token:?}: {contrast:.2}:1", if dark { "dark" } else { "light" }));
            }
        }
    }
    assert!(failures.is_empty(), "below 4.5:1:\n{}", failures.join("\n"));
}

#[test]
fn the_focus_ring_stands_out_from_every_background_it_is_drawn_on() {
    for dark in [false, true] {
        let palette = theme::palette(dark);
        let backgrounds = [
            ("the toolbar", palette.surface),
            ("the base surface", palette.base),
            ("the preview", palette.preview),
            ("a card", palette.card),
            ("the menu bar", palette.menu_bar),
        ];
        for (what, background) in backgrounds {
            let ring = luminance(theme::ring(dark));
            let behind = luminance(rgb(background));
            let contrast = (ring.max(behind) + 0.05) / (ring.min(behind) + 0.05);
            assert!(contrast >= 3.0, "ring on {what} ({} theme): {contrast:.2}:1", if dark { "dark" } else { "light" });
        }
    }
}
