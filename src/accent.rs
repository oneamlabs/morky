//! The user's Windows accent colour.
//!
//! Windows stores the personalised accent under `HKCU\Software\Microsoft\Windows\DWM\AccentColor`
//! as an `ABGR` DWORD (`0xAABBGGRR`), the same channel order as a `COLORREF`. It is read at startup
//! and at most once a second from the frame loop, so changing it in Settings applies without a
//! restart.
//!
//! High contrast owns colour while it is on, so `theme::color` consults it first; this module only
//! supplies the value the accent roles resolve to when it is not.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use gpui::Rgba;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};

use crate::diagnostics;

/// WinUI's default accent, used when the key is missing or unreadable.
pub const FALLBACK: u32 = 0x0078d4;

static ACCENT: AtomicU32 = AtomicU32::new(FALLBACK);
static LOADED: AtomicBool = AtomicBool::new(false);

/// The accent colour, as `0xRRGGBB`.
pub fn accent() -> u32 {
    if !LOADED.load(Ordering::Relaxed) {
        // First use: read it now so the very first frame is already correct.
        refresh();
    }
    ACCENT.load(Ordering::Relaxed)
}

/// Reads the accent from the registry. Returns whether it changed.
pub fn refresh() -> bool {
    let before = ACCENT.load(Ordering::Relaxed);
    let read = read_registry().unwrap_or(FALLBACK);
    LOADED.store(true, Ordering::Relaxed);
    if read == before {
        return false;
    }
    ACCENT.store(read, Ordering::Relaxed);
    true
}

/// The registry's `ABGR` value as `0xRRGGBB`, or `None` if it cannot be read.
fn read_registry() -> Option<u32> {
    // UTF-16, NUL-terminated: `RegGetValueW` takes a length and does not want the terminator.
    let subkey: Vec<u16> = "Software\\Microsoft\\Windows\\DWM\0".encode_utf16().collect();
    let value: Vec<u16> = "AccentColor\0".encode_utf16().collect();
    let mut data: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut data as *mut u32 as *mut core::ffi::c_void,
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        // A missing key is normal on a fresh profile; only a real failure is worth a log line.
        if status != windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND {
            diagnostics::write(&format!("could not read the accent colour: registry status {status}"));
        }
        return None;
    }
    Some(from_abgr(data))
}

/// `ABGR` (`0xAABBGGRR`) to `0xRRGGBB`. `DwmSetWindowAttribute`'s sibling, `COLORREF`, is the same
/// channel order, so the conversion matches `high_contrast::from_colorref`.
fn from_abgr(value: u32) -> u32 {
    let red = value & 0xFF;
    let green = (value >> 8) & 0xFF;
    let blue = (value >> 16) & 0xFF;
    (red << 16) | (green << 8) | blue
}

/// The accent as text or a glyph on `background`, lightened or darkened until it clears 4.5:1.
///
/// Users pick arbitrary accents, so this walks the accent towards white or black in small steps
/// rather than consulting a table. `dark` says which way to move first.
pub fn accent_text_on(dark: bool, background: u32) -> Rgba {
    let base = accent();
    let target = if dark { 0xffffff } else { 0x000000 };
    let steps = 24;
    let mut best = base;
    for step in 1..=steps {
        let candidate = mix(base, target, step as f32 / steps as f32);
        best = candidate;
        if contrast(candidate, background) >= 4.5 {
            break;
        }
    }
    gpui::rgb(best)
}

/// Linear blend from `from` towards `to`, `amount` in 0..=1.
fn mix(from: u32, to: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let a = ((from >> shift) & 0xFF) as f32;
        let b = ((to >> shift) & 0xFF) as f32;
        (a + (b - a) * amount).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn contrast(a: u32, b: u32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn luminance(value: u32) -> f32 {
    let channel = |raw: u32| {
        let value = raw as f32 / 255.0;
        if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel((value >> 16) & 0xFF) + 0.7152 * channel((value >> 8) & 0xFF) + 0.0722 * channel(value & 0xFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `0xRRGGBB` a colour was built from.
    fn to_rgb(color: Rgba) -> u32 {
        let channel = |value: f32| (value * 255.0).round() as u32;
        (channel(color.r) << 16) | (channel(color.g) << 8) | channel(color.b)
    }

    #[test]
    fn abgr_is_converted_to_rgb() {
        // #0078D4 as ABGR is 0xFFD47800: alpha 0xFF, blue 0xD4, green 0x78, red 0x00.
        assert_eq!(from_abgr(0xFFD4_7800), 0x0078d4);
        // A value read from this machine: `AccentColor` 0xFF484A4C matches `ColorizationColor`
        // 0xC44C4A48, whose low three bytes are RGB 0x4C4A48.
        assert_eq!(from_abgr(0xFF48_4A4C), 0x4C4A48);
        assert_eq!(from_abgr(0xFF00_00FF), 0xff0000);
    }

    #[test]
    fn accent_text_clears_aa_against_the_background_it_sits_on() {
        // Any accent the user picks must end up readable as text on either theme's card.
        for accent in [0x0078d4, 0xff0000, 0x00ff00, 0xffff00, 0x8000ff, 0x000000, 0xffffff] {
            ACCENT.store(accent, Ordering::Relaxed);
            for (dark, background) in [(true, 0x2b2b2b), (false, 0xffffff)] {
                let color = accent_text_on(dark, background);
                let value = to_rgb(color);
                assert!(contrast(value, background) >= 4.5, "accent {accent:06x} on {background:06x}: {:.2}:1", contrast(value, background));
            }
        }
    }

    #[test]
    fn a_blend_moves_each_channel() {
        assert_eq!(mix(0x000000, 0xffffff, 1.0), 0xffffff);
        assert_eq!(mix(0x000000, 0xffffff, 0.0), 0x000000);
        assert_eq!(mix(0x000000, 0xffffff, 0.5), 0x808080);
    }
}
