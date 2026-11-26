//! The caption row: the app icon, the tab strip, and the window buttons.
//!
//! The OS caption is hidden (`appears_transparent: true`), so this row is the title bar. GPUI maps
//! [`WindowControlArea`] onto Win32 hit-test results (`HTCLOSE`, `HTMAXBUTTON`, …) and performs the
//! minimize/maximize/close itself, which keeps snap layouts, the maximised inset and
//! double-click-to-maximise working without any `WM_NCHITTEST` code of our own.
//!
//! The drag area and the window buttons are **siblings**, never parent and child: the hit-test walks
//! the frame's control hitboxes in order and returns the first that contains the point, so a drag
//! area wrapping the buttons would answer `HTCAPTION` for them and the buttons would never fire.
//! The buttons carry no `on_click` for the same reason — the platform is what acts on them.
//!
//! The row is drawn in the app's theme rather than the system's, which is the point: with a native
//! caption, an app set to Light while Windows is Dark got a dark title bar over a light window.

use gpui::{Role, WindowControlArea, div, px};

use super::*;

/// Windows' caption button metrics: 46 px wide, 32 px tall at 100% scaling.
const BUTTON_WIDTH: f32 = 46.;
pub const ROW_HEIGHT: f32 = 32.;

/// Which caption button this is: its area, its glyph and its hover colours follow from it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Button {
    Min,
    Max,
    Close,
}

impl Button {
    fn name(self) -> &'static str {
        match self {
            Self::Min => "Minimize",
            Self::Max => "Maximize",
            Self::Close => "Close",
        }
    }

    fn area(self) -> WindowControlArea {
        match self {
            Self::Min => WindowControlArea::Min,
            Self::Max => WindowControlArea::Max,
            Self::Close => WindowControlArea::Close,
        }
    }

    fn glyph(self, maximized: bool) -> &'static str {
        match self {
            Self::Min => "icons/caption-minimize.svg",
            Self::Max if maximized => "icons/caption-restore.svg",
            Self::Max => "icons/caption-maximize.svg",
            Self::Close => "icons/caption-close.svg",
        }
    }
}

impl Prototype {
    pub fn title_bar(&mut self, maximized: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = self.dark;
        let tabs = self.tab_strip(cx);
        let glyph = color(dark, Color::Text);
        // The drag region stops at the buttons, so it never claims their clicks.
        let drag = div()
            .window_control_area(WindowControlArea::Drag)
            .flex()
            .flex_row()
            .flex_1()
            .min_w(px(0.))
            .items_center()
            .pl_3()
            .child(div().flex_none().pr_2().child(svg().path("icons/app.svg").w(px(16.)).h(px(16.)).text_color(color(dark, Color::Accent))))
            .child(tabs);
        let mut row = div()
            .id("title-bar")
            .flex()
            .flex_row()
            .items_center()
            .h(px(ROW_HEIGHT))
            .flex_none()
            .bg(color(dark, Color::Base))
            .text_size(px(theme::CHROME_TEXT))
            .child(drag);
        for button in [Button::Min, Button::Max, Button::Close] {
            let (hover, hover_text) = match button {
                // Close is the one button with a red hover, as in every Windows app.
                Button::Close => (gpui::rgb(0xc42b1c), gpui::rgb(0xffffff)),
                _ => (color(dark, Color::Hover), color(dark, Color::Text)),
            };
            row = row.child(
                div()
                    .id(button.name())
                    .role(Role::Button)
                    .aria_label(button.name())
                    .occlude()
                    .window_control_area(button.area())
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .w(px(BUTTON_WIDTH))
                    .h_full()
                    .text_color(glyph)
                    .hover(move |style| style.bg(hover).text_color(hover_text))
                    // GPUI renders an SVG as an alpha mask tinted by its own `text_color`, which
                    // does not inherit; without it the glyph is an invisible black mask.
                    .child(svg().path(button.glyph(maximized)).w(px(10.)).h(px(10.)).text_color(glyph)),
            );
        }
        row
    }
}
