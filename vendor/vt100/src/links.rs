//! TERYON patch: the OSC 8 hyperlink table. Cells hold a small id; the URLs live here once.
use std::collections::HashMap;

/// Most distinct links kept at once; beyond this, links no cell uses any more are dropped.
pub(crate) const MAX_LINKS: usize = 4096;
/// Longest accepted URL, in bytes.
const MAX_URL: usize = 2048;

#[derive(Clone, Debug, Default)]
pub(crate) struct Links {
    urls: Vec<String>,
    index: HashMap<String, u16>,
}

impl Links {
    pub fn get(&self, id: u16) -> Option<&str> {
        self.urls
            .get(usize::from(id.checked_sub(1)?))
            .map(String::as_str)
    }

    pub fn get_id_for(&self, url: &str) -> Option<u16> {
        self.index.get(url).copied()
    }

    pub fn is_full(&self) -> bool {
        self.urls.len() >= MAX_LINKS
    }

    /// The id for `url`, adding it if new; `None` when the table is full.
    pub fn intern(&mut self, url: &str) -> Option<u16> {
        if let Some(id) = self.index.get(url) {
            return Some(*id);
        }
        if self.is_full() {
            return None;
        }
        self.urls.push(url.to_string());
        let id = u16::try_from(self.urls.len()).ok()?;
        self.index.insert(url.to_string(), id);
        Some(id)
    }

    /// A table with only the links `keep` names, and the old id -> new id mapping.
    pub fn retain(&self, keep: &[bool]) -> (Self, Vec<u16>) {
        let mut map = vec![0u16; self.urls.len() + 1];
        let mut out = Self::default();
        for (i, url) in self.urls.iter().enumerate() {
            if keep.get(i + 1).copied().unwrap_or(false) {
                map[i + 1] = out.intern(url).unwrap_or(0);
            }
        }
        (out, map)
    }
}

/// The URL in the `8;params;URI` payload, or `None` for an empty one (which ends a link) or an
/// unusable one (too long, not UTF-8, control characters). The URI may itself contain `;`.
pub(crate) fn parse_uri(rest: &[&[u8]]) -> Option<String> {
    let joined = rest.join(&b';');
    if joined.is_empty() || joined.len() > MAX_URL {
        return None;
    }
    let url = String::from_utf8(joined).ok()?;
    if url.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(url)
}

#[cfg(test)]
mod tests {
    use crate::Hyperlink;

    fn spans(input: &str) -> Vec<Hyperlink> {
        let mut parser = crate::Parser::new(4, 40, 0);
        parser.process(input.as_bytes());
        parser.screen().hyperlinks()
    }
    fn link(row: u16, start: u16, end: u16, url: &str) -> Hyperlink {
        Hyperlink {
            row,
            start,
            end,
            url: url.into(),
        }
    }

    #[test]
    fn a_link_covers_the_cells_printed_inside_it() {
        let got = spans("go \x1b]8;;https://a.example/x\x1b\\here\x1b]8;;\x1b\\ now");
        assert_eq!(got, vec![link(0, 3, 7, "https://a.example/x")]);
    }
    #[test]
    fn bel_terminated_and_semicolons_in_the_url_work() {
        let got = spans("\x1b]8;id=1;https://a.example/?a=1;b=2\x07ab\x1b]8;;\x07");
        assert_eq!(got, vec![link(0, 0, 2, "https://a.example/?a=1;b=2")]);
    }
    #[test]
    fn sgr_reset_and_colors_do_not_end_a_link() {
        let got = spans("\x1b]8;;https://a.example\x1b\\\x1b[31ma\x1b[0mb\x1b[1mc");
        assert_eq!(got, vec![link(0, 0, 3, "https://a.example")]);
    }
    #[test]
    fn two_links_and_two_rows() {
        let got = spans(
            "\x1b]8;;https://a.example\x1b\\A\x1b]8;;https://b.example\x1b\\B\x1b]8;;\x1b\\\r\n\x1b]8;;https://a.example\x1b\\C",
        );
        assert_eq!(
            got,
            vec![
                link(0, 0, 1, "https://a.example"),
                link(0, 1, 2, "https://b.example"),
                link(1, 0, 1, "https://a.example"),
            ]
        );
    }
    #[test]
    fn erased_cells_and_overwritten_text_lose_the_link() {
        let got = spans("\x1b]8;;https://a.example\x1b\\abcd\x1b]8;;\x1b\\\x1b[3G\x1b[K");
        assert_eq!(got, vec![link(0, 0, 2, "https://a.example")]);
        let got = spans("\x1b]8;;https://a.example\x1b\\abcd\x1b]8;;\x1b\\\rXY");
        assert_eq!(got, vec![link(0, 2, 4, "https://a.example")]);
    }
    #[test]
    fn wide_characters_stay_in_one_span() {
        let got = spans("\x1b]8;;https://a.example\x1b\\日本語\x1b]8;;\x1b\\");
        assert_eq!(got, vec![link(0, 0, 6, "https://a.example")]);
    }
    #[test]
    fn oversized_urls_are_ignored() {
        let long = format!("\x1b]8;;https://a.example/{}\x1b\\text", "x".repeat(3000));
        assert!(spans(&long).is_empty());
    }
    #[test]
    fn a_full_table_drops_links_nothing_shows_any_more() {
        let mut parser = crate::Parser::new(4, 40, 0);
        for i in 0..(super::MAX_LINKS + 50) {
            parser.process(format!("\x1b]8;;https://a.example/{i}\x1b\\x\x1b]8;;\x1b\\\r\n").as_bytes());
        }
        parser.process(b"\x1b]8;;https://last.example\x1b\\L\x1b]8;;\x1b\\");
        let got = parser.screen().hyperlinks();
        assert!(got.iter().any(|h| h.url == "https://last.example"), "{got:?}");
        assert!(got.iter().all(|h| h.url.starts_with("https://")));
    }
}
