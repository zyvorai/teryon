//! Pure helpers for the desktop front end (kept free of egui so they are easy to test).

/// Subsequence match of `query` in `text`, case-insensitive; higher is better, `None` if absent.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut at = 0;
    let mut last: Option<usize> = None;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let found = (at..text.len()).find(|&i| text[i] == q)?;
        score += 10;
        if last.is_some_and(|l| found == l + 1) {
            score += 8;
        }
        if found == 0 || text[found - 1] == ' ' {
            score += 6;
        }
        last = Some(found);
        at = found + 1;
    }
    Some(score * 4 - i32::try_from(text.len()).unwrap_or(0))
}

/// The http(s) URL under column `col` of a screen row, with its `[start, end)` column span.
pub fn url_at(row: &[char], col: usize) -> Option<(String, usize, usize)> {
    if col >= row.len() || row[col].is_whitespace() {
        return None;
    }
    let stop = |c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`');
    let mut start = col;
    while start > 0 && !stop(row[start - 1]) {
        start -= 1;
    }
    let mut end = col + 1;
    while end < row.len() && !stop(row[end]) {
        end += 1;
    }
    while end > start
        && matches!(
            row[end - 1],
            '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}'
        )
    {
        end -= 1;
    }
    if col >= end {
        return None;
    }
    let token: String = row[start..end].iter().collect();
    let rest = token
        .strip_prefix("https://")
        .or_else(|| token.strip_prefix("http://"))?;
    (!rest.is_empty()).then(|| (token.clone(), start, end))
}

/// Text between two cells inclusive of both, in reading order, with trailing blanks trimmed.
pub fn selected_text(screen: &vt100::Screen, a: (u16, u16), b: (u16, u16)) -> String {
    let (from, to) = if a <= b { (a, b) } else { (b, a) };
    screen
        .contents_between(from.0, from.1, to.0, to.1.saturating_add(1))
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Rectangular selection text: columns `a.1..=b.1` (either order) of rows `a.0..=b.0`.
pub fn selected_block(screen: &vt100::Screen, a: (u16, u16), b: (u16, u16)) -> String {
    let (r0, r1) = (a.0.min(b.0), a.0.max(b.0));
    let (c0, c1) = (a.1.min(b.1), a.1.max(b.1));
    (r0..=r1)
        .map(|row| {
            screen
                .contents_between(row, c0, row, c1.saturating_add(1))
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Inclusive column range of the "word" (letters, digits and `_-./~:@%+=`) around `col`; a click
/// on a blank selects just that blank.
pub fn word_bounds(row: &[char], col: usize) -> (usize, usize) {
    let in_word = |c: char| c.is_alphanumeric() || "_-./~:@%+=#?&,".contains(c);
    let col = col.min(row.len().saturating_sub(1));
    if row.is_empty() || !in_word(row[col]) {
        return (col, col);
    }
    let mut start = col;
    while start > 0 && in_word(row[start - 1]) {
        start -= 1;
    }
    let mut end = col;
    while end + 1 < row.len() && in_word(row[end + 1]) {
        end += 1;
    }
    (start, end)
}

/// Start columns of every case-insensitive occurrence of `needle` (already lowercase) in `cells`.
pub fn match_starts(cells: &[char], needle: &[char]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > cells.len() {
        return Vec::new();
    }
    let lower: Vec<char> = cells
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    (0..=lower.len() - needle.len())
        .filter(|&s| lower[s..s + needle.len()] == *needle)
        .collect()
}

/// Index of the font file in `files` (file names) that best matches `family`, ignoring case,
/// spaces and dashes: `JetBrains Mono` finds `JetBrainsMono-Regular.ttf`.
pub fn match_font_file(family: &str, files: &[String]) -> Option<usize> {
    let norm = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase()
    };
    let want = norm(family);
    if want.is_empty() {
        return None;
    }
    let stems: Vec<String> = files
        .iter()
        .map(|f| norm(f.rsplit_once('.').map_or(f.as_str(), |(stem, _)| stem)))
        .collect();
    let exts_ok = |f: &String| {
        let l = f.to_lowercase();
        l.ends_with(".ttf") || l.ends_with(".otf") || l.ends_with(".ttc")
    };
    [
        want.clone(),
        format!("{want}regular"),
        format!("{want}mono"),
    ]
    .iter()
    .find_map(|w| (0..files.len()).find(|&i| exts_ok(&files[i]) && stems[i] == *w))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintKind {
    Url,
    Path,
    Sha,
    Ip,
    Uuid,
    Email,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    pub start: usize,
    pub end: usize,
    pub kind: HintKind,
    pub text: String,
}

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit())
}
fn is_uuid(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 5
        && [8, 4, 4, 4, 12]
            .iter()
            .zip(&parts)
            .all(|(n, p)| p.len() == *n && is_hex(p))
}
fn is_ipv4(s: &str) -> bool {
    let host = s.split(':').next().unwrap_or(s);
    let octets: Vec<&str> = host.split('.').collect();
    octets.len() == 4
        && octets.iter().all(|o| {
            !o.is_empty()
                && o.len() <= 3
                && o.chars().all(|c| c.is_ascii_digit())
                && o.parse::<u16>().is_ok_and(|n| n <= 255)
        })
}
fn is_ipv6(s: &str) -> bool {
    s.matches(':').count() >= 2
        && s.len() >= 5
        && s.chars().all(|c| c.is_ascii_hexdigit() || c == ':')
        && s.chars().any(|c| c.is_ascii_hexdigit())
}
fn is_email(s: &str) -> bool {
    let Some((user, host)) = s.split_once('@') else {
        return false;
    };
    !user.is_empty()
        && host.contains('.')
        && !host.ends_with('.')
        && !s.contains('/')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "@._+-".contains(c))
}
fn is_path(s: &str) -> bool {
    if s.contains("://") || s.len() < 2 {
        return false;
    }
    let body = s.split(':').next().unwrap_or(s);
    body.starts_with('/')
        || body.starts_with("./")
        || body.starts_with("../")
        || body.starts_with("~/")
        || (body.contains('/')
            && body
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
            && !body.ends_with('/')
            && body
                .chars()
                .all(|c| c.is_alphanumeric() || "/._-+@".contains(c)))
}
/// Things worth picking on a screen row: URLs, paths (with `:line`), git SHAs, IPs, UUIDs, emails.
pub fn hint_matches(row: &[char]) -> Vec<Hint> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < row.len() {
        if row[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < row.len() && !row[i].is_whitespace() {
            i += 1;
        }
        // Strip wrapping punctuation from the token.
        let (mut a, mut b) = (start, i);
        while a < b && "([{<\"'`".contains(row[a]) {
            a += 1;
        }
        while b > a && ")]}>\"'`,;.!?".contains(row[b - 1]) {
            b -= 1;
        }
        // A URL may legitimately end in ")" balanced elsewhere; keep it simple and trim.
        if a >= b {
            continue;
        }
        let tok: String = row[a..b].iter().collect();
        let tok_trim = tok.trim_end_matches(':').to_string();
        let b = a + tok_trim.chars().count();
        let kind = if tok_trim.starts_with("http://") || tok_trim.starts_with("https://") {
            Some(HintKind::Url)
        } else if is_uuid(&tok_trim) {
            Some(HintKind::Uuid)
        } else if is_email(&tok_trim) {
            Some(HintKind::Email)
        } else if is_ipv4(&tok_trim) {
            Some(HintKind::Ip)
        } else if is_path(&tok_trim) {
            Some(HintKind::Path)
        } else if (7..=40).contains(&tok_trim.len())
            && is_hex(&tok_trim)
            && tok_trim.chars().any(|c| c.is_ascii_digit())
            && tok_trim.chars().any(|c| c.is_ascii_alphabetic())
        {
            Some(HintKind::Sha)
        } else if is_ipv6(&tok_trim) {
            Some(HintKind::Ip)
        } else {
            None
        };
        if let Some(kind) = kind {
            out.push(Hint {
                start: a,
                end: b,
                kind,
                text: tok_trim,
            });
        }
    }
    out
}
/// Home-row-first labels: one letter for up to 26 hints, two letters (all of them) beyond that.
pub fn labels(n: usize) -> Vec<String> {
    const ABC: &str = "asdfghjklqwertyuiopzxcvbnm";
    let letters: Vec<char> = ABC.chars().collect();
    if n <= letters.len() {
        return letters.iter().take(n).map(char::to_string).collect();
    }
    (0..n.min(letters.len() * letters.len()))
        .map(|i| {
            format!(
                "{}{}",
                letters[i / letters.len()],
                letters[i % letters.len()]
            )
        })
        .collect()
}

/// Open a URL with the platform opener; arguments are passed without a shell.
/// Open a file in a text editor (macOS: TextEdit via `open -t`; elsewhere the default handler).
pub fn open_in_editor(path: &std::path::Path) {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg("-t");
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    let _ = cmd
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
pub fn open_url(url: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }
    #[test]
    fn fuzzy_prefers_tight_and_word_start_matches() {
        assert!(
            fuzzy_score("nw", "New pane").unwrap()
                > fuzzy_score("nw", "Rename pane window").unwrap_or(-999)
        );
        assert!(fuzzy_score("zzz", "New pane").is_none());
        assert!(fuzzy_score("", "x").is_some());
        assert!(
            fuzzy_score("lay", "Layout grid").unwrap()
                > fuzzy_score("lay", "Toggle sidebar layout").unwrap()
        );
    }
    #[test]
    fn urls_are_found_and_trimmed() {
        let row = chars("see https://example.com/a?b=1). now");
        assert_eq!(
            url_at(&row, 8),
            Some(("https://example.com/a?b=1".to_string(), 4, 29))
        );
        assert_eq!(url_at(&row, 0), None);
        assert_eq!(url_at(&row, row.len() - 1), None);
        assert_eq!(url_at(&chars("ftp://x.y"), 3), None);
        assert_eq!(url_at(&chars("http://"), 2), None);
    }
    #[test]
    fn selection_text_is_inclusive_ordered_and_trimmed() {
        let mut p = vt100::Parser::new(3, 20, 0);
        p.process(b"hello world\r\nsecond line   \r\nthird");
        let s = p.screen();
        assert_eq!(selected_text(s, (0, 6), (0, 10)), "world");
        assert_eq!(selected_text(s, (1, 5), (0, 6)), "world\nsecond");
    }
    #[test]
    fn font_family_names_match_files_loosely() {
        let files: Vec<String> = [
            "Menlo.ttc",
            "JetBrainsMono-Regular.ttf",
            "JetBrainsMono-Bold.ttf",
            "notes.txt",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(match_font_file("JetBrains Mono", &files), Some(1));
        assert_eq!(match_font_file("menlo", &files), Some(0));
        assert_eq!(match_font_file("notes", &files), None);
        assert_eq!(match_font_file("", &files), None);
    }
    #[test]
    fn words_and_blocks_select_what_you_expect() {
        let row = chars("cd  /usr/local/bin  ok");
        assert_eq!(word_bounds(&row, 6), (4, 17));
        assert_eq!(word_bounds(&row, 2), (2, 2));
        assert_eq!(word_bounds(&row, 0), (0, 1));
        assert_eq!(word_bounds(&[], 3), (0, 0));
        let mut p = vt100::Parser::new(3, 12, 0);
        p.process(b"abcdefgh\r\nijklmnop\r\nqrstuvwx");
        assert_eq!(selected_block(p.screen(), (0, 2), (2, 4)), "cde\nklm\nstu");
        assert_eq!(selected_block(p.screen(), (2, 4), (0, 2)), "cde\nklm\nstu");
    }
    #[test]
    fn hints_find_the_things_people_copy() {
        let row = chars("see https://x.io/a, src/main.rs:42: err at /var/log/x.log (10.0.0.7:22) a1b2c3d4 me@z.dev");
        let kinds: Vec<(HintKind, String)> = hint_matches(&row)
            .into_iter()
            .map(|h| (h.kind, h.text))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (HintKind::Url, "https://x.io/a".to_string()),
                (HintKind::Path, "src/main.rs:42".to_string()),
                (HintKind::Path, "/var/log/x.log".to_string()),
                (HintKind::Ip, "10.0.0.7:22".to_string()),
                (HintKind::Sha, "a1b2c3d4".to_string()),
                (HintKind::Email, "me@z.dev".to_string()),
            ]
        );
        let u = hint_matches(&chars("id 123e4567-e89b-12d3-a456-426614174000 end"));
        assert_eq!(u[0].kind, HintKind::Uuid);
        assert!(hint_matches(&chars("hello world 12345 deadbeef cafe")).is_empty());
        let spans = hint_matches(&chars("  ./run.sh"));
        assert_eq!((spans[0].start, spans[0].end), (2, 10));
    }
    #[test]
    fn labels_are_unique_and_prefix_free() {
        let few = labels(3);
        assert_eq!(few, ["a", "s", "d"]);
        let many = labels(40);
        assert_eq!(many.len(), 40);
        assert!(many.iter().all(|l| l.len() == 2));
        let mut sorted = many.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 40);
    }
    #[test]
    fn match_starts_are_case_insensitive() {
        assert_eq!(
            match_starts(&chars("Err err ERR"), &chars("err")),
            vec![0, 4, 8]
        );
        assert!(match_starts(&chars("ab"), &chars("abc")).is_empty());
    }
}
