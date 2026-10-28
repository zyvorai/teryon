//! `teryon search`: find text in the scrollback the daemon keeps for each pane, grep-style.
use anyhow::{Context, Result};
use regex::{Regex, RegexBuilder};

/// One matching line of a pane's capture, with its absolute line number and the lines around it.
#[derive(Debug, PartialEq)]
pub struct Hit {
    /// Absolute line number (the first line the pane ever printed is 0).
    pub line: u64,
    pub text: String,
    pub before: Vec<(u64, String)>,
    pub after: Vec<(u64, String)>,
}

/// A pattern as typed: literal text unless `regex`; case-insensitive when `ignore_case`, or when the
/// pattern is all lowercase and `smart_case` is on.
pub fn matcher(pattern: &str, regex: bool, ignore_case: bool, smart_case: bool) -> Result<Regex> {
    let source = if regex {
        pattern.to_string()
    } else {
        regex::escape(pattern)
    };
    let insensitive = ignore_case || (smart_case && !pattern.chars().any(char::is_uppercase));
    RegexBuilder::new(&source)
        .case_insensitive(insensitive)
        .size_limit(1 << 20)
        .build()
        .with_context(|| format!("'{pattern}' is not a valid pattern"))
}

/// Matching lines of `text` (whose first line is absolute line `first_line`), at most `limit`, each
/// with up to `context` lines on both sides. Trailing spaces of screen rows are not part of a line.
pub fn search(text: &str, first_line: u64, re: &Regex, context: usize, limit: usize) -> Vec<Hit> {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let mut hits = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if hits.len() >= limit {
            break;
        }
        if !re.is_match(line) {
            continue;
        }
        let number = |k: usize| first_line + k as u64;
        let from = i.saturating_sub(context);
        let to = (i + 1 + context).min(lines.len());
        hits.push(Hit {
            line: number(i),
            text: (*line).to_string(),
            before: (from..i)
                .map(|k| (number(k), lines[k].to_string()))
                .collect(),
            after: (i + 1..to)
                .map(|k| (number(k), lines[k].to_string()))
                .collect(),
        });
    }
    hits
}

/// grep-style output for one pane: `title:line: text`, context lines as `title-line- text`, and `--`
/// between groups that do not touch.
pub fn render(prefix: &str, hits: &[Hit]) -> String {
    let mut out = String::new();
    let mut last_printed: Option<u64> = None;
    for (i, h) in hits.iter().enumerate() {
        let start = h.before.first().map_or(h.line, |b| b.0);
        if let Some(prev) = last_printed {
            if start > prev + 1 {
                out.push_str("--\n");
            }
        }
        for (n, t) in &h.before {
            if last_printed.is_none_or(|p| *n > p) {
                out.push_str(&format!("{prefix}-{n}- {t}\n"));
            }
        }
        out.push_str(&format!("{prefix}:{}: {}\n", h.line, h.text));
        // A line that is itself the next match is printed once, as that match.
        let next = hits.get(i + 1).map(|n| n.line);
        let mut last = h.line;
        for (n, t) in h.after.iter().filter(|(n, _)| next.is_none_or(|x| *n < x)) {
            out.push_str(&format!("{prefix}-{n}- {t}\n"));
            last = *n;
        }
        last_printed = Some(last);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str =
        "build start\ncompiling a\nERROR: boom\nnote\nmore\nmore\nmore\nerror again   \ndone\n";

    #[test]
    fn matching_lines_get_absolute_numbers_and_context() {
        let re = matcher("error", false, false, true).unwrap();
        let hits = search(TEXT, 100, &re, 1, 10);
        assert_eq!(hits.len(), 2, "smart case: lowercase pattern ignores case");
        assert_eq!((hits[0].line, hits[0].text.as_str()), (102, "ERROR: boom"));
        assert_eq!(hits[0].before, [(101, "compiling a".to_string())]);
        assert_eq!(hits[1].text, "error again", "trailing spaces dropped");
        assert_eq!(hits[1].after, [(108, "done".to_string())]);
    }

    #[test]
    fn case_sensitivity_literals_regexes_and_the_limit() {
        assert_eq!(
            search(
                TEXT,
                0,
                &matcher("ERROR", false, false, true).unwrap(),
                0,
                10
            )
            .len(),
            1
        );
        assert_eq!(
            search(
                TEXT,
                0,
                &matcher("error", false, false, false).unwrap(),
                0,
                10
            )
            .len(),
            1
        );
        assert_eq!(
            search(
                TEXT,
                0,
                &matcher("error", false, true, false).unwrap(),
                0,
                10
            )
            .len(),
            2
        );
        // A dot is literal unless asked for as a regex.
        assert!(
            search(
                "a.b\naxb\n",
                0,
                &matcher("a.b", false, false, false).unwrap(),
                0,
                10
            )
            .len()
                == 1
        );
        assert!(
            search(
                "a.b\naxb\n",
                0,
                &matcher("a.b", true, false, false).unwrap(),
                0,
                10
            )
            .len()
                == 2
        );
        assert_eq!(
            search(
                TEXT,
                0,
                &matcher("more", false, false, false).unwrap(),
                0,
                2
            )
            .len(),
            2
        );
        assert!(matcher("(", true, false, false).is_err());
    }

    #[test]
    fn output_is_grep_shaped_with_separators_between_distant_groups() {
        let re = matcher("error", false, false, true).unwrap();
        let hits = search(TEXT, 0, &re, 1, 10);
        assert_eq!(
            render("work", &hits),
            "work-1- compiling a\nwork:2: ERROR: boom\nwork-3- note\n--\nwork-6- more\nwork:7: error again\nwork-8- done\n"
        );
        let touching = search(
            "x\nhit\nhit\ny\n",
            0,
            &matcher("hit", false, false, false).unwrap(),
            1,
            10,
        );
        assert_eq!(
            render("p", &touching),
            "p-0- x\np:1: hit\np-2- hit\np:2: hit\np-3- y\n".replace("p-2- hit\n", "")
        );
    }
}
