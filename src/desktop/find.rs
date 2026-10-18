//! Find-in-pane queries: literal (optionally case-sensitive) or regular expression.
use super::*;

/// What the find bar is looking for on the visible screen.
pub(super) enum FindQuery {
    Literal { needle: Vec<char>, case: bool },
    Regex(regex::Regex),
}
impl FindQuery {
    /// Build a query; an invalid regex is reported as text for the find bar.
    pub(super) fn new(text: &str, regex: bool, case: bool) -> Result<Option<Self>, String> {
        if text.is_empty() {
            return Ok(None);
        }
        if regex {
            let pattern = if case {
                text.to_string()
            } else {
                format!("(?i){text}")
            };
            return regex::Regex::new(&pattern)
                .map(|r| Some(Self::Regex(r)))
                .map_err(|e| {
                    e.to_string()
                        .lines()
                        .last()
                        .unwrap_or("invalid pattern")
                        .trim()
                        .to_string()
                });
        }
        let needle: Vec<char> = if case {
            text.chars().collect()
        } else {
            text.to_lowercase().chars().collect()
        };
        Ok(Some(Self::Literal { needle, case }))
    }
    /// `(start column, length)` of every match in one screen row (one `char` per column).
    pub(super) fn matches(&self, row: &[char]) -> Vec<(usize, usize)> {
        match self {
            Self::Literal { needle, case } => {
                if needle.is_empty() || needle.len() > row.len() {
                    return Vec::new();
                }
                let hay: Vec<char> = if *case {
                    row.to_vec()
                } else {
                    row.iter()
                        .map(|c| c.to_lowercase().next().unwrap_or(*c))
                        .collect()
                };
                (0..=hay.len() - needle.len())
                    .filter(|&s| hay[s..s + needle.len()] == needle[..])
                    .map(|s| (s, needle.len()))
                    .collect()
            }
            Self::Regex(re) => regex_matches(re, row),
        }
    }
}

/// `(start column, length)` of every non-empty regex match in a row (one `char` per column).
pub(super) fn regex_matches(re: &regex::Regex, row: &[char]) -> Vec<(usize, usize)> {
    let text: String = row.iter().collect();
    // Byte offsets back to columns.
    let cols: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
    let col_of = |byte: usize| cols.iter().position(|b| *b == byte).unwrap_or(cols.len());
    re.find_iter(&text)
        .filter(|m| !m.as_str().is_empty())
        .map(|m| {
            let (a, b) = (col_of(m.start()), col_of(m.end()));
            (a, b.saturating_sub(a).max(1))
        })
        .collect()
}

impl Desktop {
    /// Recompile the query after the text or a toggle changed.
    pub(super) fn rebuild_find(&mut self) {
        match FindQuery::new(&self.find_text, self.find_regex, self.find_case) {
            Ok(q) => {
                self.find_query = q;
                self.find_error = None;
            }
            Err(e) => {
                self.find_query = None;
                self.find_error = Some(e);
            }
        }
        self.find_cur = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(s: &str) -> Vec<char> {
        s.chars().collect()
    }
    #[test]
    fn literal_queries_respect_the_case_toggle() {
        let q = FindQuery::new("err", false, false).unwrap().unwrap();
        assert_eq!(q.matches(&row("Err err ERR")), vec![(0, 3), (4, 3), (8, 3)]);
        let q = FindQuery::new("err", false, true).unwrap().unwrap();
        assert_eq!(q.matches(&row("Err err ERR")), vec![(4, 3)]);
        assert!(FindQuery::new("", false, false).unwrap().is_none());
    }
    #[test]
    fn regex_queries_map_back_to_columns_and_report_bad_patterns() {
        let q = FindQuery::new(r"\d+", true, false).unwrap().unwrap();
        assert_eq!(q.matches(&row("ab 12 cd 345")), vec![(3, 2), (9, 3)]);
        let q = FindQuery::new("error|warn", true, false).unwrap().unwrap();
        assert_eq!(q.matches(&row("ERROR and Warn")).len(), 2);
        let q = FindQuery::new("error", true, true).unwrap().unwrap();
        assert!(q.matches(&row("ERROR")).is_empty());
        assert!(FindQuery::new("(", true, false).is_err());
        let q = FindQuery::new("é+", true, false).unwrap().unwrap();
        assert_eq!(q.matches(&row("aéé b")), vec![(1, 2)]);
    }
}
