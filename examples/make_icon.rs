//! Renders the icon SVGs into `assets/marky.ico`, which `build.rs` embeds in `marky.exe`.
//!
//! Windows picks the nearest size for the taskbar, Explorer and shortcuts, so every size the
//! shell asks for is supplied rather than downscaled from one bitmap. Below 32 px the arrow and
//! the M merge, so those sizes use `app-icon-small.svg`, the same mark without the arrow. Run it
//! after editing either SVG: `cargo run --example make_icon` (or `./scripts/make-icon.ps1`).

use std::path::Path;

const SIZES: [u32; 9] = [16, 20, 24, 32, 40, 48, 64, 96, 256];
const SMALL_LIMIT: u32 = 24;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let options = resvg::usvg::Options::default();
    let mut trees = Vec::new();
    for name in ["app-icon.svg", "app-icon-small.svg"] {
        let svg = std::fs::read_to_string(root.join(name))?;
        trees.push(resvg::usvg::Tree::from_str(&svg, &options)?);
    }
    let (full, small) = (&trees[0], &trees[1]);

    let images = SIZES
        .iter()
        .map(|&size| {
            let tree = if size <= SMALL_LIMIT { small } else { full };
            let mut pixmap =
                resvg::tiny_skia::Pixmap::new(size, size).ok_or("icon size is out of range")?;
            let scale = size as f32 / tree.size().width();
            resvg::render(
                tree,
                resvg::tiny_skia::Transform::from_scale(scale, scale),
                &mut pixmap.as_mut(),
            );
            Ok((size, pixmap.encode_png()?))
        })
        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;

    let ico = ico(&images);
    let path = root.join("assets/marky.ico");
    std::fs::write(&path, &ico)?;
    println!(
        "{}: {} sizes ({:?}), {} bytes",
        path.display(),
        images.len(),
        SIZES,
        ico.len()
    );
    Ok(())
}

/// Writes an icon directory whose entries are the PNGs Windows 10 and 11 accept for every size.
fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, png) in images {
        let dimension = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[dimension, dimension, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in images {
        out.extend_from_slice(png);
    }
    out
}
