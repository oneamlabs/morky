//! The About dialog: version, what Marky is, the licence and log location, and a button that asks
//! GitHub for the latest release.
//!
//! A card over the panes rather than a platform dialog, so it can hold a button and show the
//! answer. Esc, Close or a click outside puts it away.

use super::*;

/// A quiet line of the card. A `Label` role exposes its text through `value`, which is what UI
/// Automation reads as the name; a bare text child alone names nothing here.
fn label(dark: bool, id: &'static str, text: &str) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .role(Role::Label)
        .aria_value(SharedString::from(text.to_string()))
        .text_color(quiet(dark))
        .child(text.to_string())
}

impl Prototype {
    pub(super) fn open_about(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.recent_open = false;
        self.about = Some(About { check: None, checking: false });
        window.focus(&self.about_check, cx);
        cx.notify();
    }

    /// Esc and the title-bar close both land here. Returns whether the dialog was open.
    pub fn close_about(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.about.take().is_none() {
            return false;
        }
        self.restore_editor_focus(window, cx);
        true
    }

    /// Asks GitHub for the latest release off the main thread, then paints the answer. From the
    /// Help menu it opens the card first, so the answer has somewhere to show.
    pub(super) fn check_for_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.about.is_none() {
            self.open_about(window, cx);
        }
        let Some(about) = self.about.as_mut() else { return };
        if about.checking {
            return;
        }
        about.checking = true;
        let result = cx.background_executor().spawn(async move { crate::update::latest() });
        self.update_task = Some(cx.spawn_in(window, async move |this, cx| {
            let found = result.await;
            let _ = this.update(cx, |this, cx| this.finish_update_check(found, cx));
        }));
        cx.notify();
    }

    fn finish_update_check(&mut self, found: Result<String, String>, cx: &mut Context<Self>) {
        self.update_task = None;
        let Some(about) = self.about.as_mut() else { return };
        about.checking = false;
        about.check = Some(match found {
            Ok(tag) => {
                let latest = tag.trim_start_matches(['v', 'V']).to_string();
                let newer = crate::update::is_newer(&latest, env!("CARGO_PKG_VERSION"));
                crate::update::Check::Ready { latest, newer }
            }
            Err(err) => crate::update::Check::Failed(err),
        });
        cx.notify();
    }

    /// The About card, over the panes. Its own layer, so a click on the card is not the click that
    /// dismisses it.
    pub fn about_dialog(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let dark = self.dark;
        let Some(about) = self.about.as_ref() else { return div().into_any_element() };
        let checking = about.checking;
        let result = about.check.clone();
        let log = crate::settings::own_local_dir().join("marky.log");
        let version = env!("CARGO_PKG_VERSION");
        let quiet_color = quiet(dark);

        let message = match &result {
            None if checking => "Checking GitHub for the latest release…".to_string(),
            None => "Marky does not check for updates on its own.".to_string(),
            Some(crate::update::Check::Ready { latest, newer: true }) => format!("Version {latest} is available."),
            Some(crate::update::Check::Ready { latest, newer: false }) => format!("Marky is up to date (latest is {latest})."),
            Some(crate::update::Check::Failed(err)) => format!("The check failed: {err}"),
        };
        let message_color = match &result {
            Some(crate::update::Check::Failed(_)) => color(dark, Color::Error),
            Some(crate::update::Check::Ready { newer: true, .. }) => color(dark, Color::AccentText),
            _ => quiet_color,
        };
        let can_download = matches!(&result, Some(crate::update::Check::Ready { newer: true, .. }));
        let check_label = if checking { "Checking…" } else { "Check for updates" };

        let button = |id: &'static str, label: &'static str, name: &'static str, primary: bool, dark: bool| {
            let background = if primary { color(dark, Color::Accent) } else { color(dark, Color::Card) };
            // A primary button's label sits on the accent fill, which is the user's colour, so black
            // or white is chosen by contrast. High contrast instead maps both to system colours,
            // where the theme's own pairing already reads.
            let text = if !primary {
                color(dark, Color::Text)
            } else if crate::high_contrast::is_active() {
                color(dark, Color::Base)
            } else {
                crate::accent::on_accent()
            };
            focus_ring(
                div()
                    .id(id)
                    .role(Role::Button)
                    .aria_label(name)
                    .focusable()
                    .tab_stop(true)
                    .px_3()
                    .py_1()
                    .rounded(px(theme::RADIUS_CONTROL))
                    .bg(background)
                    .text_color(text)
                    .hover(move |style| style.bg(color(dark, Color::Pressed)))
                    .child(label),
                dark,
            )
        };

        let card = div()
            .id("about-card")
            .role(Role::Dialog)
            .aria_label("About Marky")
            .occlude()
            // A click anywhere on the card is not the click that dismisses it.
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.stop_propagation()))
            .flex()
            .flex_col()
            .gap_2()
            .w(px(420.))
            .p_5()
            .rounded(px(theme::RADIUS_OVERLAY))
            .shadow_md()
            .bg(color(dark, Color::Card))
            .border_1()
            .border_color(color(dark, Color::StrokeStrong))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .child(div().id("about-title").role(Role::Heading).aria_level(2).aria_label("Marky").text_size(px(theme::TITLE_TEXT)).font_weight(gpui::FontWeight::BOLD).child("Marky"))
                    .child(
                        focus_ring(
                            div().id("about-close").role(Role::Button).aria_label("Close About").focusable().tab_stop(true).px_2().child("×"),
                            dark,
                        )
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.close_about(window, cx);
                        })),
                    ),
            )
            .child(div().id("about-version").role(Role::Label).aria_value(SharedString::from(format!("Version {version}"))).child(format!("Version {version}")))
            .child(label(dark, "about-blurb", "A fast Markdown editor for Windows."))
            .child(label(dark, "about-licence", "MIT licence. Third-party notices ship with Marky."))
            .child(label(dark, "about-log", &format!("Log: {}", log.display())))
            .child(div().id("about-status").role(Role::Status).aria_label(SharedString::from(message.clone())).pt_2().text_color(message_color).child(message))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .pt_2()
                    .child(
                        button("about-download", "Download", "Open the releases page", true, dark)
                            .when(!can_download, |button| button.opacity(0.5))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                if matches!(&this.about.as_ref().and_then(|about| about.check.as_ref()), Some(crate::update::Check::Ready { newer: true, .. })) {
                                    cx.open_url(crate::update::RELEASES_URL);
                                }
                            })),
                    )
                    .child(
                        button("about-check", check_label, "Check for updates", false, dark)
                            .track_focus(&self.about_check)
                            .when(checking, |button| button.opacity(0.5))
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.check_for_updates(window, cx))),
                    ),
            );

        // The scrim closes the card when clicked, and dims the panes behind it.
        div()
            .id("about-scrim")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::rgba(0x00000088))
            .track_focus(&self.about_focus)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.close_about(window, cx);
            }))
            .child(card)
            .into_any_element()
    }
}
