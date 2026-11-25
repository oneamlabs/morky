//! Following a link in the preview, with the app's rules, and the link list the arrow keys walk.
//!
//! Web and mail links open in the default program; a footnote reference scrolls the preview to its
//! footnote; a relative link opens a `.md`, `.markdown` or `.txt` file next to this one as a tab.
//! The preview never navigates, so no link can replace the document.

use super::*;

impl Prototype {
    /// Every link the preview draws, in document order, with the block it is in. The preview is one
    /// tab stop; the left and right arrows move through this list and Enter follows the chosen link.
    pub(super) fn preview_links(&self) -> &[(Arc<str>, usize)] {
        &self.doc().links
    }

    /// A click on a link in the preview, with the app's rules (`followLink`).
    pub fn follow_link(&mut self, link: &str, _window: &mut Window, cx: &mut Context<Self>) {
        if opens_outside(link) {
            cx.open_url(link);
            return;
        }
        if let Some(number) = link.strip_prefix("#fn:").and_then(|number| number.parse::<u32>().ok()) {
            let doc = self.doc();
            if let Some(index) = doc.blocks.iter().position(|block| block.footnote == Some(number)) {
                doc.block_list.scroll_to(ListOffset { item_ix: index, offset_in_item: px(0.) });
                cx.notify();
            }
            return;
        }
        // A footnote's ↩ scrolls back to the block that refers to it.
        if let Some(number) = link.strip_prefix("#fnref:") {
            let target = format!("#fn:{number}");
            let doc = self.doc();
            let refers = |runs: &[preview::Run]| runs.iter().any(|run| run.style.link.as_deref() == Some(target.as_str()));
            let found = doc.blocks.iter().position(|block| refers(&block.runs) || block.table.as_ref().is_some_and(|table| table.rows.iter().flatten().any(|cell| refers(cell))));
            if let Some(index) = found {
                doc.block_list.scroll_to(ListOffset { item_ix: index, offset_in_item: px(0.) });
                cx.notify();
            }
            return;
        }
        // The app gives headings no ids, so other #anchors go nowhere, as there.
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
