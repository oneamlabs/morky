//! Following a link out of the preview, with the app's rules.
//!
//! In-preview destinations (`#fn:`, `#fnref:` and other `#anchors`) are handled by the preview pane;
//! web and mail links open in the default program, and a relative link opens a `.md`, `.markdown`
//! or `.txt` file as a tab. The preview never navigates, so no link can replace the document.

use super::*;

impl Prototype {
    /// A link the preview passed on because it does not stay inside the pane.
    pub fn follow_link(&mut self, link: &str, _window: &mut Window, cx: &mut Context<Self>) {
        if opens_outside(link) {
            cx.open_url(link);
            return;
        }
        // Every in-preview anchor is handled by the pane; this is a guard against a missed one.
        if link.starts_with('#') {
            return;
        }
        let target = link.split('#').next().unwrap_or_default().replace("%20", " ");
        match allowlist::link_target(Path::new(&self.doc().path), &target) {
            Ok(path) => self.open_path(&path, cx),
            Err(err) => {
                self.report(format!("Not opened: {link} ({err})"));
                cx.notify();
            }
        }
    }
}

/// Whether a link opens outside Marky. URL schemes are case-insensitive (RFC 3986).
fn opens_outside(link: &str) -> bool {
    let lower = link.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_and_mail_schemes_are_recognised_in_any_case() {
        for link in ["https://example.com", "HTTP://example.com", "HtTpS://example.com/a", "mailto:me@example.com", "MAILTO:me@example.com"] {
            assert!(opens_outside(link), "{link}");
        }
        for link in ["docs/a.md", "file:///C:/a.md", "ftp://example.com", "javascript:alert(1)", "#fn:1"] {
            assert!(!opens_outside(link), "{link}");
        }
    }
}
