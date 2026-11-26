//! Toolbar icons. GPUI paints an SVG as a monochrome mask, so the shapes are black and the button tints them.
//! The editor pane and preview pane are the same shapes as the app's toolbar.

use std::borrow::Cow;

use gpui::{AssetSource, SharedString};

const EDITOR: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect x="1.5" y="2.5" width="13" height="11" rx="1.5" fill="none" stroke="black"/><path d="M2 3h6v10H2z" fill="black"/><path d="M8 3v10" stroke="black"/></svg>"#;
const EDITOR_OFF: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect x="1.5" y="2.5" width="13" height="11" rx="1.5" fill="none" stroke="black"/><path d="M8 3v10" stroke="black"/></svg>"#;
const PREVIEW: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect x="1.5" y="2.5" width="13" height="11" rx="1.5" fill="none" stroke="black"/><path d="M8 3h6v10H8z" fill="black"/><path d="M8 3v10" stroke="black"/></svg>"#;
const PREVIEW_OFF: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect x="1.5" y="2.5" width="13" height="11" rx="1.5" fill="none" stroke="black"/><path d="M8 3v10" stroke="black"/></svg>"#;
const OUTLINE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M3 4.5h10M3 8h10M5 11.5h8" fill="none" stroke="black" stroke-linecap="round"/></svg>"#;
/// The caption row's app mark: the M-and-arrow of `app-icon.svg`, solid so the row can tint it.
const APP: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M2.5 12.5V3.5l3 5 3-5v9" fill="none" stroke="black" stroke-width="1.6" stroke-linejoin="round" stroke-linecap="round"/><path d="M11 6.5l2.5 1.5L11 9.5" fill="black"/></svg>"#;
/// Caption buttons. Drawn as geometry rather than font glyphs: `U+2750` is missing from Segoe UI and
/// falls back to a plain square, so a restore and a maximize button looked identical.
const CAPTION_MINIMIZE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M0 5h10" stroke="black" stroke-width="1"/></svg>"#;
const CAPTION_MAXIMIZE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="black" stroke-width="1"/></svg>"#;
const CAPTION_RESTORE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M2.5 2.5h7v7" fill="none" stroke="black" stroke-width="1"/><rect x="0.5" y="3.5" width="6" height="6" fill="none" stroke="black" stroke-width="1"/></svg>"#;
const CAPTION_CLOSE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M0.5 0.5l9 9M9.5 0.5l-9 9" stroke="black" stroke-width="1"/></svg>"#;

pub struct IconAssets;

impl AssetSource for IconAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let svg = match path {
            "icons/editor.svg" => EDITOR,
            "icons/editor-off.svg" => EDITOR_OFF,
            "icons/preview.svg" => PREVIEW,
            "icons/preview-off.svg" => PREVIEW_OFF,
            "icons/outline.svg" => OUTLINE,
            "icons/app.svg" => APP,
            "icons/caption-minimize.svg" => CAPTION_MINIMIZE,
            "icons/caption-maximize.svg" => CAPTION_MAXIMIZE,
            "icons/caption-restore.svg" => CAPTION_RESTORE,
            "icons/caption-close.svg" => CAPTION_CLOSE,
            _ => return Ok(None),
        };
        Ok(Some(Cow::Borrowed(svg.as_bytes())))
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}
