//! A thin, read-only scrollbar over a virtualised list.
//!
//! GPUI's lists reserve space for a scrollbar but do not draw one; without it a pane gives no clue
//! where the viewport sits or that more content exists. The thumb is proportional to the measured
//! content and repaints with its pane (each list notifies its owner on scroll).

use super::*;

/// The overlay track and thumb, in the pane's top-right corner. Empty when everything fits.
pub(super) fn indicator(list: &ListState, dark: bool) -> AnyElement {
    let viewport = list.viewport_bounds();
    let height = f32::from(viewport.size.height);
    let max = f32::from(list.max_offset_for_scrollbar().y);
    if height <= 0.0 || max <= 0.0 {
        return div().size_0().into_any_element();
    }
    let offset = (-f32::from(list.scroll_px_offset_for_scrollbar().y)).clamp(0.0, max);
    let content = height + max;
    let thumb = (height * height / content).max(24.0_f32.min(height)).min(height);
    let top = offset / max * (height - thumb);
    let color = color(dark, Color::Scrollbar);
    div()
        .absolute()
        .right(px(2.))
        .top(px(2.))
        .bottom(px(2.))
        .w(px(6.))
        .child(div().absolute().top(px(top)).h(px(thumb)).w_full().rounded_full().bg(color))
        .into_any_element()
}

/// The same indicator for the source pane's horizontal scroll when word wrap is off.
pub(super) fn horizontal_indicator(scroll: &ScrollHandle, dark: bool) -> AnyElement {
    let viewport = scroll.bounds();
    let width = f32::from(viewport.size.width);
    let max = f32::from(scroll.max_offset().x);
    if width <= 0.0 || max <= 0.0 {
        return div().size_0().into_any_element();
    }
    let offset = (-f32::from(scroll.offset().x)).clamp(0.0, max);
    let content = width + max;
    let thumb = (width * width / content).max(24.0_f32.min(width)).min(width);
    let left = offset / max * (width - thumb);
    let color = color(dark, Color::Scrollbar);
    div()
        .absolute()
        .left(px(2.))
        .right(px(2.))
        .bottom(px(2.))
        .h(px(4.))
        .child(div().absolute().left(px(left)).w(px(thumb)).h_full().rounded_full().bg(color))
        .into_any_element()
}
