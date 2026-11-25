//! GPUI keeps every bitmap it has drawn until it is told to drop it, and every edit of a formula or
//! a diagram makes a new one. The preview's caches make their bitmaps themselves (instead of handing
//! GPUI an SVG `Image`, whose bitmap only GPUI can reach) so they can drop the ones no longer shown.

use std::sync::Arc;

use gpui::{App, Image, ImageFormat, RenderImage, SvgRenderer};

/// An SVG drawn into a bitmap at GPUI's 2× scale, with its natural size in pixels. Formulas and
/// diagrams both hang onto one of these; a bitmap made here is ours to drop (see [`drop_bitmaps`]).
pub struct Bitmap {
    pub image: Arc<RenderImage>,
    pub width: f32,
    pub height: f32,
}

/// Draws `svg` into a bitmap. Glyphs in both RaTeX's and merman's output are paths, so no fonts are
/// loaded for it.
pub fn rasterize(svg: String, renderer: &SvgRenderer) -> Result<Bitmap, String> {
    let (width, height) = view_box_size(&svg).ok_or("the SVG has no size")?;
    let image = Image::from_bytes(ImageFormat::Svg, svg.into_bytes())
        .to_image_data(renderer.clone())
        .map_err(|err| format!("could not draw the SVG: {err}"))?;
    Ok(Bitmap { image, width, height })
}

/// Width and height from the root `viewBox`, in pixels. Neither SVG we draw has usable `width` and
/// `height` attributes: RaTeX writes them in points, which would draw a third too large, and merman
/// writes `width="100%"`.
pub fn view_box_size(svg: &str) -> Option<(f32, f32)> {
    let start = svg.find("viewBox=\"")? + "viewBox=\"".len();
    let values: Vec<f32> = svg[start..].split('"').next()?.split_whitespace().filter_map(|value| value.parse().ok()).collect();
    match values.as_slice() {
        [_, _, width, height] => Some((*width, *height)),
        _ => None,
    }
}

/// Drops `bitmaps` from every window's texture atlas. Deferred so that no window is borrowed by
/// the update that called it: `drop_image` then reaches every window.
pub fn drop_bitmaps(bitmaps: Vec<Arc<RenderImage>>, cx: &mut App) {
    if bitmaps.is_empty() {
        return;
    }
    cx.defer(move |cx| {
        for bitmap in bitmaps {
            cx.drop_image(bitmap, None);
        }
    });
}
