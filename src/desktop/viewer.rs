//! Syntax colouring for Quick Look: a `bat`-style read-only view with a line-number gutter.
//! syntect's grammars decide where each token starts; the colours come from the active theme's
//! ANSI palette, so every Teryon theme gets a matching code view without a second theme file.
use std::sync::OnceLock;
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

/// Past this much text the rest is shown plain: a large file must not stall the window.
const MAX_COLOURED: usize = 256 * 1024;
/// Lines longer than this are shown plain: minified files make a regex grammar crawl.
const MAX_LINE: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tok {
    Plain,
    Comment,
    Str,
    Num,
    Keyword,
    Func,
    Type,
    Const,
}

/// A file as lines of coloured runs.
pub(super) type Lines = Vec<Vec<(Tok, String)>>;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// The grammar for a file name (whole name first, so `Makefile` works), then its first line.
fn syntax_for<'a>(set: &'a SyntaxSet, name: &str, text: &str) -> Option<&'a SyntaxReference> {
    let ext = name.rsplit_once('.').map(|(_, e)| e);
    set.find_syntax_by_extension(name)
        .or_else(|| ext.and_then(|e| set.find_syntax_by_extension(e)))
        .or_else(|| set.find_syntax_by_first_line(text.lines().next().unwrap_or("")))
        .filter(|s| s.name != "Plain Text")
}

fn classify(stack: &ScopeStack) -> Tok {
    // The innermost scope that means something wins.
    for scope in stack.as_slice().iter().rev() {
        let s = scope.build_string();
        let s = s.as_str();
        let tok = if s.starts_with("comment") {
            Tok::Comment
        } else if s.starts_with("string") {
            Tok::Str
        } else if s.starts_with("constant.numeric") {
            Tok::Num
        } else if s.starts_with("constant") {
            Tok::Const
        } else if s.starts_with("keyword") || s.starts_with("storage") {
            Tok::Keyword
        } else if s.starts_with("entity.name.function")
            || s.starts_with("support.function")
            || s.starts_with("meta.function-call")
        {
            Tok::Func
        } else if s.starts_with("entity.name")
            || s.starts_with("support.type")
            || s.starts_with("support.class")
        {
            Tok::Type
        } else {
            continue;
        };
        return tok;
    }
    Tok::Plain
}

/// `text` as lines of coloured runs, or `None` when no grammar fits (the caller shows it plain).
pub(super) fn highlight(name: &str, text: &str) -> Option<Lines> {
    let set = syntaxes();
    let syntax = syntax_for(set, name, text)?;
    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut out = Vec::new();
    let mut seen = 0;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        seen += line.len();
        if body.len() > MAX_LINE || seen > MAX_COLOURED {
            out.push(vec![(Tok::Plain, body.to_string())]);
            continue;
        }
        let Ok(ops) = state.parse_line(line, set) else {
            out.push(vec![(Tok::Plain, body.to_string())]);
            continue;
        };
        let mut runs: Vec<(Tok, String)> = Vec::new();
        let mut at = 0;
        let mut push = |stack: &ScopeStack, from: usize, to: usize| {
            let to = to.min(body.len());
            if from >= to || !body.is_char_boundary(from) || !body.is_char_boundary(to) {
                return;
            }
            let tok = classify(stack);
            match runs.last_mut() {
                Some((t, s)) if *t == tok => s.push_str(&body[from..to]),
                _ => runs.push((tok, body[from..to].to_string())),
            }
        };
        for (pos, op) in ops {
            push(&stack, at, pos);
            at = at.max(pos);
            // A bad pop only means the grammar and the file disagree; keep colouring.
            let _ = stack.apply(&op);
        }
        push(&stack, at, body.len());
        out.push(runs);
    }
    Some(out)
}

/// A grammar-less file still gets line numbers.
pub(super) fn plain(text: &str) -> Lines {
    text.lines()
        .map(|l| vec![(Tok::Plain, l.to_string())])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lines: &[Vec<(Tok, String)>], line: usize) -> Vec<Tok> {
        lines[line].iter().map(|(t, _)| *t).collect()
    }

    #[test]
    fn rust_code_is_split_into_keywords_strings_and_comments() {
        let src = "fn main() {\n    let s = \"hi\"; // note\n}\n";
        let lines = highlight("main.rs", src).expect("rust grammar");
        assert_eq!(lines.len(), 3);
        assert!(lines[0]
            .iter()
            .any(|(t, s)| *t == Tok::Keyword && s == "fn"));
        assert!(kinds(&lines, 1).contains(&Tok::Str));
        assert!(kinds(&lines, 1).contains(&Tok::Comment));
        // Colouring never drops or invents text.
        let back: String = lines
            .iter()
            .map(|l| l.iter().map(|(_, s)| s.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(back, src.trim_end());
    }

    #[test]
    fn names_without_an_extension_and_shebangs_find_a_grammar() {
        assert!(highlight("Makefile", "all:\n\techo hi\n").is_some());
        assert!(highlight("run", "#!/bin/bash\necho hi\n").is_some());
    }

    #[test]
    fn unknown_files_and_huge_lines_fall_back_to_plain() {
        assert!(highlight("data.zzzz", "just words\n").is_none());
        let long = format!("x = \"{}\"\n", "a".repeat(MAX_LINE + 10));
        let lines = highlight("a.py", &long).unwrap();
        assert_eq!(kinds(&lines, 0), vec![Tok::Plain]);
        assert_eq!(plain("a\nb\n").len(), 2);
    }

    #[test]
    fn a_large_file_is_coloured_only_at_the_top() {
        let src = "let a = 1; // c\n".repeat(MAX_COLOURED / 8);
        let lines = highlight("a.js", &src).unwrap();
        assert!(kinds(&lines, 0).len() > 1);
        assert_eq!(kinds(lines.as_slice(), lines.len() - 1), vec![Tok::Plain]);
    }

    #[test]
    fn multibyte_text_does_not_split_a_character() {
        let lines = highlight("a.py", "s = \"héllo → wörld\"  # ünï\n").unwrap();
        let joined: String = lines[0].iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(joined, "s = \"héllo → wörld\"  # ünï");
    }
}
