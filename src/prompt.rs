//! `teryon prompt`: a two-line zsh prompt with the directory, git state, language, last command's
//! duration and exit status. Collecting and rendering are separate so rendering stays testable.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// How long `git status` may take before the prompt shows just the branch.
const GIT_TIMEOUT: Duration = Duration::from_millis(250);
/// Commands that ran at least this long (milliseconds) show their duration.
const SHOW_DURATION_MS: u64 = 2000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Git {
    pub branch: String,
    pub ahead: u32,
    pub behind: u32,
    pub staged: u32,
    pub modified: u32,
    pub untracked: u32,
    pub conflicted: u32,
    /// True when only the branch is known (git was too slow).
    pub partial: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub status: i32,
    pub ms: u64,
    pub cwd: String,
    pub home: String,
    /// `user@host` when connected over SSH.
    pub remote: Option<String>,
    pub git: Option<Git>,
    pub langs: Vec<&'static str>,
}

/// Parse `git status --porcelain=v2 -b` output.
pub fn parse_status(text: &str) -> Option<Git> {
    let mut g = Git::default();
    let mut oid = String::new();
    let mut seen_branch = false;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# branch.oid ") {
            oid = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("# branch.head ") {
            seen_branch = true;
            g.branch = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
            for part in rest.split_whitespace() {
                if let Some(n) = part.strip_prefix('+') {
                    g.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix('-') {
                    g.behind = n.parse().unwrap_or(0);
                }
            }
        } else if line.starts_with("1 ") || line.starts_with("2 ") {
            let xy: Vec<char> = line.chars().skip(2).take(2).collect();
            if xy.first().is_some_and(|c| *c != '.') {
                g.staged += 1;
            }
            if xy.get(1).is_some_and(|c| *c != '.') {
                g.modified += 1;
            }
        } else if line.starts_with("u ") {
            g.conflicted += 1;
        } else if line.starts_with("? ") {
            g.untracked += 1;
        }
    }
    if !seen_branch {
        return None;
    }
    if g.branch == "(detached)" {
        g.branch = oid.chars().take(7).collect();
    }
    Some(g)
}
/// The branch (or short hash) from `.git/HEAD`, found by walking up from `dir`; no process needed.
pub fn head_branch(dir: &Path) -> Option<String> {
    for d in dir.ancestors() {
        let dot = d.join(".git");
        let git_dir = if dot.is_dir() {
            dot
        } else if dot.is_file() {
            let text = std::fs::read_to_string(&dot).ok()?;
            let target = text.strip_prefix("gitdir:")?.trim();
            let p = PathBuf::from(target);
            if p.is_absolute() {
                p
            } else {
                d.join(p)
            }
        } else {
            continue;
        };
        let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
        let head = head.trim();
        return Some(match head.strip_prefix("ref: refs/heads/") {
            Some(branch) => branch.to_string(),
            None => head.chars().take(7).collect(),
        });
    }
    None
}
fn run_git(dir: &Path, timeout: Duration) -> Option<String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["status", "--porcelain=v2", "-b"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = out.read_to_string(&mut text);
        let _ = tx.send(text);
    });
    match rx.recv_timeout(timeout) {
        Ok(text) => {
            let ok = child.wait().is_ok_and(|s| s.success());
            ok.then_some(text)
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}
/// Git state for `dir`: full when `git status` answers in time, otherwise just the branch.
pub fn collect_git(dir: &Path) -> Option<Git> {
    head_branch(dir)?;
    match run_git(dir, GIT_TIMEOUT).and_then(|t| parse_status(&t)) {
        Some(git) => Some(git),
        None => head_branch(dir).map(|branch| Git {
            branch,
            partial: true,
            ..Git::default()
        }),
    }
}
/// Languages recognised from marker files in `dir`.
pub fn detect_langs(dir: &Path) -> Vec<&'static str> {
    const MARKERS: &[(&str, &[&str])] = &[
        ("rust", &["Cargo.toml"]),
        ("node", &["package.json"]),
        (
            "python",
            &["pyproject.toml", "requirements.txt", "setup.py"],
        ),
        ("go", &["go.mod"]),
        ("swift", &["Package.swift"]),
        ("ruby", &["Gemfile"]),
        ("java", &["pom.xml", "build.gradle", "build.gradle.kts"]),
    ];
    MARKERS
        .iter()
        .filter(|(_, files)| files.iter().any(|f| dir.join(f).exists()))
        .map(|(name, _)| *name)
        .collect()
}

/// `~` for home and at most the last three components (`…/a/b/c`).
pub fn short_dir(cwd: &str, home: &str) -> String {
    let tilded = match cwd.strip_prefix(home) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => {
            format!("~{rest}")
        }
        _ => cwd.to_string(),
    };
    let parts: Vec<&str> = tilded.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() <= 3 {
        return tilded;
    }
    format!("…/{}", parts[parts.len() - 3..].join("/"))
}
/// Escape `%` so untrusted text cannot become a prompt escape.
fn esc(text: &str) -> String {
    text.replace('%', "%%")
}
fn git_segment(g: &Git) -> String {
    let mut flags = String::new();
    if g.conflicted > 0 {
        flags.push_str(&format!("={}", g.conflicted));
    }
    if g.staged > 0 {
        flags.push_str(&format!("+{}", g.staged));
    }
    if g.modified > 0 {
        flags.push_str(&format!("!{}", g.modified));
    }
    if g.untracked > 0 {
        flags.push_str(&format!("?{}", g.untracked));
    }
    if g.ahead > 0 {
        flags.push_str(&format!("↑{}", g.ahead));
    }
    if g.behind > 0 {
        flags.push_str(&format!("↓{}", g.behind));
    }
    let mut out = format!(" on %F{{magenta}}{}%f", esc(&g.branch));
    if !flags.is_empty() {
        out.push_str(&format!(" %F{{red}}[{}]%f", esc(&flags)));
    }
    out
}
/// The prompt as a zsh prompt string (colours are `%F{...}` escapes; text is already escaped).
pub fn render(i: &Inputs) -> String {
    let mut line = String::new();
    if let Some(remote) = &i.remote {
        line.push_str(&format!("%F{{yellow}}{}%f ", esc(remote)));
    }
    line.push_str(&format!(
        "%B%F{{cyan}}{}%f%b",
        esc(&short_dir(&i.cwd, &i.home))
    ));
    if let Some(g) = &i.git {
        line.push_str(&git_segment(g));
    }
    if !i.langs.is_empty() {
        line.push_str(&format!(" via %F{{yellow}}{}%f", i.langs.join(" ")));
    }
    if i.ms >= SHOW_DURATION_MS {
        line.push_str(&format!(
            " took %F{{yellow}}{}%f",
            crate::picker::short_duration(i.ms)
        ));
    }
    if i.status != 0 {
        line.push_str(&format!(" %F{{red}}✗{}%f", i.status));
    }
    let colour = if i.status == 0 { "green" } else { "red" };
    format!("{line}\n%B%F{{{colour}}}❯%f%b ")
}
/// Gather everything for `cwd` and render.
pub fn prompt(status: i32, ms: u64, cwd: &str, home: &str) -> String {
    let dir = Path::new(cwd);
    let remote = std::env::var_os("SSH_CONNECTION").map(|_| {
        let user = std::env::var("USER").unwrap_or_default();
        let host = std::env::var("HOST")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_default();
        format!("{user}@{host}")
    });
    render(&Inputs {
        status,
        ms,
        cwd: cwd.into(),
        home: home.into(),
        remote,
        git: collect_git(dir),
        langs: detect_langs(dir),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const CLEAN: &str = "# branch.oid abcdef1234567890\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n1 .M N... 100644 100644 100644 aaa bbb src/a.rs\n1 M. N... 100644 100644 100644 aaa bbb src/b.rs\n2 R. N... 100644 100644 100644 aaa bbb R100 new old\nu UU N... 1 2 3 4 aaa bbb ccc conflict.rs\n? new.txt\n? other.txt\n";
    #[test]
    fn porcelain_v2_status_is_counted() {
        let g = parse_status(CLEAN).unwrap();
        assert_eq!(
            g,
            Git {
                branch: "main".into(),
                ahead: 2,
                behind: 1,
                staged: 2,
                modified: 1,
                untracked: 2,
                conflicted: 1,
                partial: false
            }
        );
        let detached =
            parse_status("# branch.oid 1234567890abcdef\n# branch.head (detached)\n").unwrap();
        assert_eq!(detached.branch, "1234567");
        assert!(parse_status("fatal: not a git repository").is_none());
    }
    #[test]
    fn directories_are_shortened() {
        assert_eq!(short_dir("/Users/me", "/Users/me"), "~");
        assert_eq!(short_dir("/Users/me/tt/x", "/Users/me"), "~/tt/x");
        assert_eq!(short_dir("/Users/me/a/b/c/d", "/Users/me"), "…/b/c/d");
        assert_eq!(short_dir("/etc/ssh", "/Users/me"), "/etc/ssh");
        assert_eq!(short_dir("/Users/meow/x", "/Users/me"), "/Users/meow/x");
    }
    #[test]
    fn untrusted_text_cannot_inject_prompt_escapes() {
        let g = Git {
            branch: "fix-%F{red}-100%".into(),
            ..Git::default()
        };
        let out = render(&Inputs {
            cwd: "/tmp/50%".into(),
            git: Some(g),
            ..Inputs::default()
        });
        assert!(out.contains("fix-%%F{red}-100%%"));
        assert!(out.contains("/tmp/50%%"));
    }
    #[test]
    fn the_prompt_shows_each_part_only_when_it_has_something_to_say() {
        let plain = render(&Inputs {
            cwd: "/etc".into(),
            ..Inputs::default()
        });
        assert_eq!(plain, "%B%F{cyan}/etc%f%b\n%B%F{green}❯%f%b ");
        let full = render(&Inputs {
            status: 2,
            ms: 3400,
            cwd: "/Users/me/tt".into(),
            home: "/Users/me".into(),
            remote: Some("me@box".into()),
            git: parse_status(CLEAN),
            langs: vec!["rust"],
        });
        assert_eq!(
            full,
            "%F{yellow}me@box%f %B%F{cyan}~/tt%f%b on %F{magenta}main%f %F{red}[=1+2!1?2↑2↓1]%f via %F{yellow}rust%f took %F{yellow}3.4s%f %F{red}✗2%f\n%B%F{red}❯%f%b "
        );
        let quick = render(&Inputs {
            ms: 1999,
            ..Inputs::default()
        });
        assert!(!quick.contains("took"));
    }
    #[test]
    fn the_branch_is_read_from_git_head_without_running_git() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("a/b");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        assert_eq!(head_branch(&sub).as_deref(), Some("feature/x"));
        std::fs::write(dir.path().join(".git/HEAD"), "0123456789abcdef\n").unwrap();
        assert_eq!(head_branch(&sub).as_deref(), Some("0123456"));
        let outside = tempfile::tempdir().unwrap();
        assert_eq!(head_branch(outside.path()), None);
    }
    #[test]
    fn languages_come_from_marker_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(detect_langs(dir.path()).is_empty());
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        std::fs::write(dir.path().join("package.json"), "").unwrap();
        assert_eq!(detect_langs(dir.path()), ["rust", "node"]);
    }
    #[test]
    fn a_real_repository_reports_its_branch_and_dirty_files() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
        };
        if git(&["init", "-q", "-b", "trunk"]).is_err() {
            return;
        }
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        let g = collect_git(dir.path()).unwrap();
        assert_eq!(
            (g.branch.as_str(), g.untracked, g.partial),
            ("trunk", 1, false)
        );
    }
}
