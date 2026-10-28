//! `teryon explain`: what each part of a command line does, from the manual pages already on this
//! machine. The command is never run (not even with `--help`); only `man` is consulted.
use std::collections::HashMap;
use std::process::{Command, Stdio};

#[derive(Debug, PartialEq)]
pub struct Part {
    pub text: String,
    pub note: String,
}

#[derive(Debug, PartialEq)]
enum Tok {
    Word(String),
    Op(String),
}

/// Split a command line the way a shell would, keeping operators (`|`, `&&`, `;`, `>` ...) apart.
fn tokenize(line: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = line.chars().peekable();
    let flush = |out: &mut Vec<Tok>, word: &mut String, in_word: &mut bool| {
        if *in_word {
            out.push(Tok::Word(std::mem::take(word)));
            *in_word = false;
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    word.push(q);
                }
            }
            '"' => {
                in_word = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' => {
                            if let Some(n) = chars.next() {
                                word.push(n);
                            }
                        }
                        _ => word.push(q),
                    }
                }
            }
            '\\' => {
                in_word = true;
                if let Some(n) = chars.next() {
                    word.push(n);
                }
            }
            c if c.is_whitespace() => flush(&mut out, &mut word, &mut in_word),
            '|' | '&' | ';' | '<' | '>' => {
                // `2>` is the redirect of descriptor 2, not a word "2" followed by `>`.
                let fd = in_word
                    && word.len() == 1
                    && word.chars().all(|d| d.is_ascii_digit())
                    && matches!(c, '<' | '>');
                let mut op = String::new();
                if fd {
                    op.push_str(&std::mem::take(&mut word));
                    in_word = false;
                } else {
                    flush(&mut out, &mut word, &mut in_word);
                }
                op.push(c);
                while let Some(&n) = chars.peek() {
                    if n == c || (c == '>' && n == '&') || (c == '&' && n == '>') {
                        op.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                out.push(Tok::Op(op));
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    flush(&mut out, &mut word, &mut in_word);
    out
}

fn expand_tabs(line: &str) -> String {
    let mut out = String::new();
    for c in line.chars() {
        if c == '\t' {
            let n = 8 - out.chars().count() % 8;
            out.push_str(&" ".repeat(n));
        } else {
            out.push(c);
        }
    }
    out
}

/// Drop the `X\bX` / `_\bX` overstrikes a man page uses for bold and underline.
fn strip_overstrike(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == '\u{8}' {
            out.pop();
        } else {
            out.push(c);
        }
    }
    out
}

/// The one-line summary under NAME: `ls – list directory contents` gives `list directory contents`.
pub fn summary(man: &str, command: &str) -> Option<String> {
    let mut lines = man.lines();
    lines.find(|l| l.trim() == "NAME")?;
    // The line can wrap, so take the whole paragraph.
    let line = lines
        .skip_while(|l| l.trim().is_empty())
        .take_while(|l| !l.trim().is_empty())
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ");
    let (names, what) = [" \u{2013} ", " - ", " \u{2014} "]
        .iter()
        .filter_map(|sep| line.split_once(sep))
        .min_by_key(|(names, _)| names.len())?;
    names
        .split(',')
        .any(|n| n.trim() == command)
        .then(|| what.trim().to_string())
}

fn option_names(spec: &str) -> Vec<String> {
    spec.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|t| t.starts_with('-') && t.len() > 1)
        .map(|t| {
            let t = t.split(['=', '[']).next().unwrap_or(t);
            t.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '@' && c != '%')
                .to_string()
        })
        .filter(|t| {
            t.len() > 1
                && t.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-@%_".contains(c))
        })
        .collect()
}

/// Option descriptions in a man page: `-l`, `--long` and what they do. Handles the BSD layout (text
/// after two or more spaces) and the GNU one (text on the lines below).
pub fn options(man: &str) -> HashMap<String, String> {
    let lines: Vec<String> = man.lines().map(expand_tabs).collect();
    let indent = |l: &str| l.len() - l.trim_start().len();
    let mut out: HashMap<String, String> = HashMap::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if !trimmed.starts_with('-') {
            continue;
        }
        let base = indent(line);
        let (spec, inline) = match trimmed.find("  ") {
            Some(k) => (&trimmed[..k], trimmed[k..].trim()),
            None => (trimmed, ""),
        };
        let names = option_names(spec);
        if names.is_empty() {
            continue;
        }
        let mut desc = inline.to_string();
        let mut skipped_blank = false;
        for next in &lines[i + 1..] {
            if next.trim().is_empty() {
                if desc.is_empty() && !skipped_blank {
                    skipped_blank = true;
                    continue;
                }
                break;
            }
            if indent(next) <= base {
                break;
            }
            if !desc.is_empty() {
                desc.push(' ');
            }
            desc.push_str(next.trim());
        }
        let desc = first_sentence(&desc);
        if desc.is_empty() {
            continue;
        }
        for n in names {
            out.entry(n).or_insert_with(|| desc.clone());
        }
    }
    out
}

fn first_sentence(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut = text
        .match_indices(". ")
        .map(|(k, _)| k + 1)
        .find(|&k| k > 12)
        .unwrap_or(text.len());
    let mut s: String = text[..cut].chars().take(150).collect();
    if cut < text.len() && s.chars().count() == 150 {
        s.push('…');
    }
    s
}

/// The text of `man COMMAND` for a plain command name, or `None` when there is no page.
pub fn man_text(command: &str) -> Option<String> {
    let ok = !command.is_empty()
        && command
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        && command
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.+-".contains(c));
    if !ok {
        return None;
    }
    let out = Command::new("man")
        .arg(command)
        .env("MANPAGER", "cat")
        .env("PAGER", "cat")
        .env("MANWIDTH", "100")
        .env("MAN_KEEP_FORMATTING", "")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| strip_overstrike(&String::from_utf8_lossy(&out.stdout)))
        .filter(|t| !t.trim().is_empty())
}

const WRAPPERS: [(&str, &str); 8] = [
    (
        "sudo",
        "run the command as another user (root unless -u says otherwise)",
    ),
    ("doas", "run the command as another user"),
    ("env", "run the command with changed environment variables"),
    ("time", "run the command and report how long it took"),
    (
        "nohup",
        "keep the command running after the terminal closes",
    ),
    (
        "command",
        "run the command, skipping shell functions and aliases",
    ),
    ("exec", "replace the shell with the command"),
    ("nice", "run the command at a lower priority"),
];

fn operator_note(op: &str) -> &'static str {
    match op {
        "|" => {
            "pipe: the output of the command on the left becomes the input of the one on the right"
        }
        "||" => "run the next command only if the previous one failed",
        "&&" => "run the next command only if the previous one succeeded",
        ";" => "then run the next command, whatever happened",
        "&" => "run the command in the background",
        ">" => "write the output to the file after it, replacing it",
        ">>" => "append the output to the file after it",
        "<" => "read the input from the file after it",
        "2>" => "write error messages to the file after it, replacing it",
        "2>>" => "append error messages to the file after it",
        "&>" | ">&" | "2>&1" => "send errors and output to the same place",
        _ => "shell operator",
    }
}

/// What is known about one command: its NAME summary (when the page describes it) and its options.
type Page = Option<(Option<String>, HashMap<String, String>)>;

/// Explain `line` part by part. `man` returns the manual page text for a command name.
pub fn explain(line: &str, man: &mut dyn FnMut(&str) -> Option<String>) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut pages: HashMap<String, Page> = HashMap::new();
    let mut expect_command = true;
    let mut after_redirect = false;
    let mut command = String::new();
    for tok in tokenize(line) {
        match tok {
            Tok::Op(op) => {
                parts.push(Part {
                    note: operator_note(&op).into(),
                    text: op.clone(),
                });
                after_redirect = op.contains(['<', '>']) && !op.ends_with("&1");
                if matches!(op.as_str(), "|" | "||" | "&&" | ";" | "&") {
                    expect_command = true;
                }
            }
            Tok::Word(w) if after_redirect => {
                after_redirect = false;
                parts.push(Part {
                    text: w,
                    note: "file".into(),
                });
            }
            Tok::Word(w) if expect_command => {
                if w.contains('=') && !w.starts_with('=') && !w.starts_with('-') {
                    let name = w.split('=').next().unwrap_or("");
                    parts.push(Part {
                        note: format!("set the variable {name} for this command"),
                        text: w,
                    });
                    continue;
                }
                if let Some((_, what)) = WRAPPERS.iter().find(|(n, _)| *n == w) {
                    parts.push(Part {
                        text: w,
                        note: (*what).into(),
                    });
                    continue;
                }
                let page = pages
                    .entry(w.clone())
                    .or_insert_with(|| man(&w).map(|t| (summary(&t, &w), options(&t))));
                let note = match page {
                    Some((Some(what), _)) => what.clone(),
                    Some((None, _)) => "(the manual page does not describe this name)".into(),
                    None => "(no manual page found)".into(),
                };
                parts.push(Part {
                    text: w.clone(),
                    note,
                });
                command = w;
                expect_command = false;
            }
            Tok::Word(w) if w.starts_with('-') && w.len() > 1 && w != "--" => {
                let opts = match pages.get(&command) {
                    Some(Some((Some(_), o))) => Some(o),
                    _ => None,
                };
                let flag = w.split('=').next().unwrap_or(&w).to_string();
                let described = |f: &str| opts.and_then(|o| o.get(f)).cloned();
                if let Some(d) = described(&flag) {
                    parts.push(Part { text: w, note: d });
                } else if !flag.starts_with("--")
                    && flag.len() > 2
                    && flag[1..].chars().all(|c| c.is_ascii_alphanumeric())
                    && flag[1..]
                        .chars()
                        .all(|c| described(&format!("-{c}")).is_some())
                {
                    // `-la` is `-l -a`.
                    for c in flag[1..].chars() {
                        let f = format!("-{c}");
                        let note = described(&f).unwrap_or_default();
                        parts.push(Part { text: f, note });
                    }
                } else if !flag.starts_with("--")
                    && flag.len() > 2
                    && described(&flag[..2]).is_some()
                {
                    // `-j8`: the option with its value attached.
                    let note = format!(
                        "{} (value: {})",
                        described(&flag[..2]).unwrap_or_default(),
                        &flag[2..]
                    );
                    parts.push(Part { text: w, note });
                } else {
                    parts.push(Part {
                        text: w,
                        note: if opts.is_some() {
                            "(not found in the manual page)".into()
                        } else {
                            "option".into()
                        },
                    });
                }
            }
            Tok::Word(w) => {
                let note = if w == "--" {
                    "end of options: what follows is not a flag"
                } else if w.starts_with('/') || w.starts_with('~') || w.starts_with('.') {
                    "path"
                } else {
                    "argument"
                };
                parts.push(Part {
                    text: w,
                    note: note.into(),
                });
            }
        }
    }
    parts
}

pub fn render(parts: &[Part]) -> String {
    let width = parts
        .iter()
        .map(|p| p.text.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(4, 28);
    parts
        .iter()
        .map(|p| format!("  {:<width$}  {}\n", p.text, p.note))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BSD: &str = "LS(1)\n\nNAME\n     ls \u{2013} list directory contents\n\nSYNOPSIS\n     ls [-A] file\n\nDESCRIPTION\n     The following options are available:\n\n     -@      Display extended attribute keys and sizes in long (-l) output.\n\n     -A      Include directory entries whose names begin with a dot (\u{2018}.\u{2019}) except for . and ...\n\t     Automatically set for the super-user unless -I is specified.\n\n     -l      (The lowercase letter \u{201c}ell\u{201d}.)  List files in the long format.\n";
    const GNU: &str = "GREP(1)\n\nNAME\n       grep, egrep - print lines that match patterns\n\nOPTIONS\n       -i, --ignore-case\n              Ignore case distinctions in patterns and data.  More text.\n\n       --include=GLOB\n              Search only files whose base name matches GLOB.\n\n       -v, --invert-match\n              Invert the sense of matching.\n";

    #[test]
    fn bsd_pages_give_inline_descriptions_and_continuation_lines() {
        let o = options(BSD);
        assert_eq!(
            o["-@"],
            "Display extended attribute keys and sizes in long (-l) output."
        );
        assert!(o["-A"].starts_with("Include directory entries whose names begin with a dot"));
        assert_eq!(
            summary(BSD, "ls").as_deref(),
            Some("list directory contents")
        );
        assert_eq!(summary(BSD, "cat"), None);
    }

    #[test]
    fn gnu_pages_give_descriptions_below_and_both_option_names() {
        let o = options(GNU);
        assert_eq!(o["-i"], "Ignore case distinctions in patterns and data.");
        assert_eq!(o["--ignore-case"], o["-i"]);
        assert_eq!(
            o["--include"],
            "Search only files whose base name matches GLOB."
        );
        assert_eq!(
            summary(GNU, "egrep").as_deref(),
            Some("print lines that match patterns")
        );
    }

    #[test]
    fn quotes_operators_and_descriptor_redirects_are_tokenized() {
        let t = tokenize("echo 'a | b' \"c d\" 2>err.txt && ls|wc");
        let w = |s: &str| Tok::Word(s.into());
        let o = |s: &str| Tok::Op(s.into());
        assert_eq!(
            t,
            vec![
                w("echo"),
                w("a | b"),
                w("c d"),
                o("2>"),
                w("err.txt"),
                o("&&"),
                w("ls"),
                o("|"),
                w("wc")
            ]
        );
    }

    #[test]
    fn a_pipeline_is_explained_part_by_part_and_combined_flags_are_split() {
        let mut man = |c: &str| match c {
            "ls" => Some(BSD.to_string()),
            "grep" => Some(GNU.replace("egrep", "grep")),
            _ => None,
        };
        let parts = explain("sudo ls -lA --color ~/x | grep -iv foo > out.txt", &mut man);
        let note = |t: &str| parts.iter().find(|p| p.text == t).map(|p| p.note.as_str());
        assert!(note("sudo").unwrap().contains("another user"));
        assert_eq!(note("ls"), Some("list directory contents"));
        assert!(note("-l").unwrap().contains("long format"));
        assert!(note("-A").unwrap().starts_with("Include directory entries"));
        assert_eq!(note("--color"), Some("(not found in the manual page)"));
        assert_eq!(note("~/x"), Some("path"));
        assert!(note("|").unwrap().starts_with("pipe"));
        assert_eq!(
            note("-i"),
            Some("Ignore case distinctions in patterns and data.")
        );
        assert_eq!(note("-v"), Some("Invert the sense of matching."));
        assert_eq!(note("foo"), Some("argument"));
        let parts = explain("grep -i2 x", &mut man);
        assert!(parts[1].note.ends_with("(value: 2)"), "{parts:?}");
        assert!(note(">").unwrap().contains("replacing"));
        assert_eq!(note("out.txt"), Some("file"));
    }

    #[test]
    fn unknown_commands_and_hostile_names_get_no_manual_lookup() {
        let mut man = |_: &str| None;
        let parts = explain("FOO=1 nosuchcmd -x", &mut man);
        assert_eq!(parts[0].note, "set the variable FOO for this command");
        assert_eq!(parts[1].note, "(no manual page found)");
        assert_eq!(parts[2].note, "option");
        assert_eq!(man_text("--help"), None);
        assert_eq!(man_text("a b"), None);
        assert_eq!(man_text(""), None);
        assert!(render(&parts).contains("nosuchcmd"));
    }
}
