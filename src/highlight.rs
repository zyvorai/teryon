//! Colouring of a zsh command line while it is typed. A small tokenizer, not a parser: it knows
//! command position, options, quotes, variables, operators and comments. Spans use character
//! offsets because that is what zsh's `region_highlight` takes.
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A command found by the lookup.
    Command,
    /// A command-position word the lookup could not find; only the shell knows aliases and functions.
    Unknown,
    Reserved,
    Option,
    Str,
    /// A quote that is never closed.
    Unclosed,
    Var,
    Op,
    Comment,
    /// An argument that names an existing file or directory.
    Path,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}
impl Kind {
    /// The `region_highlight` style for this kind; `?` asks the shell to decide.
    pub fn style(self) -> &'static str {
        match self {
            Kind::Command => "fg=green",
            Kind::Unknown => "?",
            Kind::Reserved | Kind::Str => "fg=yellow",
            Kind::Option => "fg=cyan",
            Kind::Unclosed => "fg=red",
            Kind::Var => "fg=magenta",
            Kind::Op => "fg=blue",
            Kind::Comment => "fg=8",
            Kind::Path => "underline",
        }
    }
}

const RESERVED: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "function", "select", "time", "coproc", "in", "!", "{", "}", "[[", "]]",
];
/// Reserved words after which another command follows.
const THEN_COMMAND: &[&str] = &[
    "if", "then", "else", "elif", "do", "while", "until", "time", "coproc", "!", "{",
];
/// Commands that run the next word as a command.
const PRECOMMAND: &[&str] = &[
    "sudo",
    "doas",
    "command",
    "builtin",
    "exec",
    "noglob",
    "nocorrect",
    "env",
    "nohup",
    "time",
];
const BUILTINS: &[&str] = &[
    ".",
    ":",
    "[",
    "alias",
    "autoload",
    "bg",
    "bindkey",
    "builtin",
    "cd",
    "command",
    "compdef",
    "declare",
    "dirs",
    "disable",
    "echo",
    "emulate",
    "enable",
    "eval",
    "exec",
    "exit",
    "export",
    "false",
    "fc",
    "fg",
    "float",
    "functions",
    "getopts",
    "hash",
    "history",
    "integer",
    "jobs",
    "kill",
    "let",
    "local",
    "noglob",
    "popd",
    "print",
    "printf",
    "pushd",
    "pwd",
    "read",
    "readonly",
    "rehash",
    "return",
    "set",
    "setopt",
    "shift",
    "source",
    "test",
    "trap",
    "true",
    "type",
    "typeset",
    "ulimit",
    "umask",
    "unalias",
    "unset",
    "unsetopt",
    "wait",
    "whence",
    "where",
    "which",
    "zle",
    "zmodload",
];

/// True when `word` is a builtin or an executable on `$PATH`.
pub fn is_command(word: &str) -> bool {
    if BUILTINS.contains(&word) {
        return true;
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| is_executable(&dir.join(word)))
}
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
/// True when `word` (`~` allowed) names something that exists.
pub fn path_exists(word: &str) -> bool {
    let expanded = match word.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => match std::env::var_os("HOME") {
            Some(home) => format!("{}{rest}", home.to_string_lossy()),
            None => return false,
        },
        Some(_) => return false,
        None => word.to_string(),
    };
    Path::new(&expanded).exists()
}

fn is_op(c: char) -> bool {
    matches!(c, '|' | '&' | ';' | '(' | ')' | '<' | '>')
}
fn is_assignment(word: &str) -> bool {
    let name = word
        .split_once('=')
        .map_or("", |(n, _)| n.trim_end_matches('+'));
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}
fn plain_path_word(word: &str) -> bool {
    (word.contains('/') || word.starts_with(['~', '.']))
        && !word.contains(['$', '*', '?', '[', '\'', '"', '\\', '`'])
}

/// Spans for `line`. `command(w)` says whether `w` is a known command, `exists(w)` whether a path exists.
pub fn spans(
    line: &str,
    command: &dyn Fn(&str) -> bool,
    exists: &dyn Fn(&str) -> bool,
) -> Vec<Span> {
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut i = 0;
    let mut cmd_pos = true;
    // After a precommand (`sudo -u root ls`) a word that is not a command may be an option's value.
    let mut after_pre = false;
    let mut last_option = false;
    while i < n {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if chars[i] == '#' {
            out.push(Span {
                start,
                end: n,
                kind: Kind::Comment,
            });
            break;
        }
        if is_op(chars[i]) {
            while i < n && is_op(chars[i]) {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            out.push(Span {
                start,
                end: i,
                kind: Kind::Op,
            });
            if !text.contains(['<', '>']) {
                cmd_pos = true;
                after_pre = false;
            }
            last_option = false;
            continue;
        }
        // One word: runs to unquoted whitespace or an operator character.
        let mut quotes = Vec::new();
        let mut plain = true;
        while i < n && !chars[i].is_whitespace() && !is_op(chars[i]) {
            match chars[i] {
                '\\' => {
                    plain = false;
                    i = (i + 2).min(n);
                }
                q @ ('\'' | '"') => {
                    plain = false;
                    let qs = i;
                    i += 1;
                    let mut closed = false;
                    while i < n {
                        if q == '"' && chars[i] == '\\' {
                            i = (i + 2).min(n);
                        } else if chars[i] == q {
                            i += 1;
                            closed = true;
                            break;
                        } else {
                            i += 1;
                        }
                    }
                    quotes.push(Span {
                        start: qs,
                        end: i,
                        kind: if closed { Kind::Str } else { Kind::Unclosed },
                    });
                }
                _ => i += 1,
            }
        }
        let word: String = chars[start..i].iter().collect();
        let kind = if cmd_pos {
            classify_command(&word, plain, after_pre && last_option, command, exists)
        } else if word.starts_with('-') && word.len() > 1 {
            Some(Kind::Option)
        } else if word.starts_with('$') {
            Some(Kind::Var)
        } else if plain && plain_path_word(&word) && exists(&word) {
            Some(Kind::Path)
        } else {
            None
        };
        let is_option = matches!(kind, Some(Kind::Option));
        if cmd_pos {
            if is_assignment(&word) && plain {
                // `FOO=bar cmd`: still waiting for the command.
            } else if is_option && after_pre {
                // `sudo -E cmd`: still waiting for the command.
            } else if RESERVED.contains(&word.as_str()) {
                cmd_pos = THEN_COMMAND.contains(&word.as_str());
                after_pre = false;
            } else if PRECOMMAND.contains(&word.as_str()) {
                after_pre = true;
            } else if matches!(kind, Some(Kind::Command | Kind::Unknown)) {
                cmd_pos = false;
                after_pre = false;
            }
        }
        if let Some(kind) = kind {
            out.push(Span {
                start,
                end: i,
                kind,
            });
        }
        out.extend(quotes);
        last_option = is_option;
    }
    out
}
/// The kind for a word where a command is expected. `maybe_value` marks a word that may be an
/// option's value after a precommand, so an unknown one is left uncoloured instead of red.
fn classify_command(
    word: &str,
    plain: bool,
    maybe_value: bool,
    command: &dyn Fn(&str) -> bool,
    exists: &dyn Fn(&str) -> bool,
) -> Option<Kind> {
    if is_assignment(word) && plain {
        return Some(Kind::Var);
    }
    if word.starts_with('-') && word.len() > 1 && !plain_path_word(word) {
        return Some(Kind::Option);
    }
    if RESERVED.contains(&word) {
        return Some(Kind::Reserved);
    }
    let found = if word.contains('/') {
        plain && exists(word)
    } else {
        plain && command(word)
    };
    match (found, maybe_value) {
        (true, _) => Some(Kind::Command),
        (false, true) => None,
        (false, false) if plain => Some(Kind::Unknown),
        (false, false) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn known(w: &str) -> bool {
        matches!(w, "git" | "ls" | "sudo" | "grep" | "cd" | "echo")
    }
    fn exists(w: &str) -> bool {
        matches!(w, "src/main.rs" | "./bin" | "/etc")
    }
    fn kinds(line: &str) -> Vec<(String, Kind)> {
        let chars: Vec<char> = line.chars().collect();
        spans(line, &known, &exists)
            .into_iter()
            .map(|s| (chars[s.start..s.end].iter().collect(), s.kind))
            .collect()
    }
    fn k(w: &str, kind: Kind) -> (String, Kind) {
        (w.to_string(), kind)
    }
    #[test]
    fn commands_options_and_paths() {
        assert_eq!(
            kinds("git commit -m src/main.rs"),
            vec![
                k("git", Kind::Command),
                k("-m", Kind::Option),
                k("src/main.rs", Kind::Path)
            ]
        );
        assert_eq!(kinds("frobnicate x"), vec![k("frobnicate", Kind::Unknown)]);
        assert_eq!(kinds("./bin"), vec![k("./bin", Kind::Command)]);
        assert_eq!(kinds("./missing"), vec![k("./missing", Kind::Unknown)]);
    }
    #[test]
    fn operators_start_a_new_command_but_redirects_do_not() {
        assert_eq!(
            kinds("ls | grep x && nope"),
            vec![
                k("ls", Kind::Command),
                k("|", Kind::Op),
                k("grep", Kind::Command),
                k("&&", Kind::Op),
                k("nope", Kind::Unknown)
            ]
        );
        assert_eq!(
            kinds("ls 2>&1 > out"),
            vec![k("ls", Kind::Command), k(">&", Kind::Op), k(">", Kind::Op)]
        );
    }
    #[test]
    fn quotes_variables_assignments_and_comments() {
        assert_eq!(
            kinds("echo \"a b\" 'c' $HOME # note"),
            vec![
                k("echo", Kind::Command),
                k("\"a b\"", Kind::Str),
                k("'c'", Kind::Str),
                k("$HOME", Kind::Var),
                k("# note", Kind::Comment)
            ]
        );
        assert_eq!(
            kinds("echo \"open"),
            vec![k("echo", Kind::Command), k("\"open", Kind::Unclosed)]
        );
        assert_eq!(
            kinds("FOO=1 ls"),
            vec![k("FOO=1", Kind::Var), k("ls", Kind::Command)]
        );
        assert_eq!(
            kinds("git --msg=\"x y\""),
            vec![
                k("git", Kind::Command),
                k("--msg=\"x y\"", Kind::Option),
                k("\"x y\"", Kind::Str)
            ]
        );
    }
    #[test]
    fn reserved_words_and_precommands() {
        assert_eq!(
            kinds("if ls; then echo"),
            vec![
                k("if", Kind::Reserved),
                k("ls", Kind::Command),
                k(";", Kind::Op),
                k("then", Kind::Reserved),
                k("echo", Kind::Command)
            ]
        );
        assert_eq!(
            kinds("sudo -u root ls"),
            vec![
                k("sudo", Kind::Command),
                k("-u", Kind::Option),
                k("ls", Kind::Command)
            ]
        );
        assert_eq!(
            kinds("sudo nope"),
            vec![k("sudo", Kind::Command), k("nope", Kind::Unknown)]
        );
    }
    #[test]
    fn offsets_count_characters_not_bytes() {
        let s = spans("echo é $X", &known, &exists);
        assert_eq!(
            s.last().unwrap(),
            &Span {
                start: 7,
                end: 9,
                kind: Kind::Var
            }
        );
    }
}
