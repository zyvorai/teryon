//! What an AI agent changed: the commits on its branch since it left the main line, plus whatever is
//! still uncommitted (`teryon agent-diff`). Read-only: it only runs `git` queries in the pane's directory.
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, PartialEq)]
pub struct Report {
    pub root: PathBuf,
    pub branch: String,
    /// The branch the work is measured from (`origin/main`, `main`, ...), when this branch is not it.
    pub base: Option<String>,
    /// One line per commit since the base, newest first.
    pub commits: Vec<String>,
    /// `git diff --stat` of the working tree against the base (or against HEAD with no base).
    pub stat: String,
    pub untracked: Vec<String>,
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .context("could not run git")?;
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn exists(dir: &Path, rev: &str) -> bool {
    git(dir, &["rev-parse", "--verify", "--quiet", rev]).is_ok()
}

/// The line of development this branch split from: the remote's default branch, else a conventional name.
fn base_of(dir: &Path, branch: &str) -> Option<String> {
    let mut names: Vec<String> = Vec::new();
    if let Ok(head) = git(
        dir,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    ) {
        names.push(head.trim().to_string());
    }
    for n in ["main", "master", "develop"] {
        names.push(format!("origin/{n}"));
        names.push(n.to_string());
    }
    names
        .into_iter()
        .find(|n| n != branch && n.rsplit('/').next() != Some(branch) && exists(dir, n))
}

pub fn report(cwd: &Path) -> Result<Report> {
    let root = PathBuf::from(
        git(cwd, &["rev-parse", "--show-toplevel"])
            .with_context(|| format!("{} is not inside a Git repository", cwd.display()))?
            .trim(),
    );
    let branch = match git(&root, &["symbolic-ref", "--short", "-q", "HEAD"]) {
        Ok(b) => b.trim().to_string(),
        Err(_) => format!(
            "(detached at {})",
            git(&root, &["rev-parse", "--short", "HEAD"])?.trim()
        ),
    };
    let base = base_of(&root, &branch);
    let from = match &base {
        Some(b) => git(&root, &["merge-base", "HEAD", b])
            .map(|s| s.trim().to_string())
            .ok(),
        None => None,
    };
    let (commits, anchor) = match &from {
        Some(mb) => (
            git(&root, &["log", "--oneline", &format!("{mb}..HEAD")])?
                .lines()
                .map(str::to_string)
                .collect(),
            mb.as_str(),
        ),
        None => (Vec::new(), "HEAD"),
    };
    let stat = git(&root, &["diff", anchor, "--stat"])?;
    let untracked = git(&root, &["ls-files", "--others", "--exclude-standard"])?
        .lines()
        .map(str::to_string)
        .collect();
    Ok(Report {
        root,
        branch,
        base: from.and(base),
        commits,
        stat,
        untracked,
    })
}

pub fn render(r: &Report) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = write!(out, "{} on {}", r.root.display(), r.branch);
    if let Some(b) = &r.base {
        let _ = write!(out, " (since {b})");
    }
    out.push('\n');
    if !r.commits.is_empty() {
        let _ = writeln!(out, "\n{} commit(s):", r.commits.len());
        for c in &r.commits {
            let _ = writeln!(out, "  {c}");
        }
    }
    if !r.stat.trim().is_empty() {
        let _ = write!(out, "\nchanges:\n{}", r.stat);
    }
    if !r.untracked.is_empty() {
        let _ = writeln!(out, "\nnew, not yet tracked:");
        for f in &r.untracked {
            let _ = writeln!(out, "  {f}");
        }
    }
    if r.commits.is_empty() && r.stat.trim().is_empty() && r.untracked.is_empty() {
        out.push_str("\nNo changes yet.\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        sh(d.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(d.path().join("a.txt"), "one\n").unwrap();
        sh(d.path(), &["add", "."]);
        sh(d.path(), &["commit", "-q", "-m", "start"]);
        d
    }

    #[test]
    fn a_branch_is_measured_from_where_it_left_main_and_shows_uncommitted_work() {
        let d = repo();
        sh(d.path(), &["checkout", "-q", "-b", "agent/claude-1"]);
        std::fs::write(d.path().join("a.txt"), "one\ntwo\n").unwrap();
        sh(d.path(), &["commit", "-q", "-am", "add two"]);
        std::fs::write(d.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(d.path().join("new.txt"), "x\n").unwrap();
        let sub = d.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let r = report(&sub).unwrap();
        assert_eq!(r.branch, "agent/claude-1");
        assert_eq!(r.base.as_deref(), Some("main"));
        assert_eq!(r.commits.len(), 1);
        assert!(r.commits[0].ends_with("add two"));
        assert!(r.stat.contains("a.txt"), "{}", r.stat);
        assert_eq!(r.untracked, vec!["new.txt"]);
        let text = render(&r);
        assert!(text.contains("on agent/claude-1 (since main)"), "{text}");
        assert!(
            text.contains("1 commit(s):") && text.contains("new.txt"),
            "{text}"
        );
    }

    #[test]
    fn on_the_main_line_only_uncommitted_work_counts() {
        let d = repo();
        assert!(render(&report(d.path()).unwrap()).contains("No changes yet."));
        std::fs::write(d.path().join("a.txt"), "changed\n").unwrap();
        let r = report(d.path()).unwrap();
        assert_eq!((r.base, r.commits.len()), (None, 0));
        assert!(r.stat.contains("a.txt"));
    }

    #[test]
    fn outside_a_repository_the_error_names_the_directory() {
        let d = tempfile::tempdir().unwrap();
        let e = report(d.path()).unwrap_err().to_string();
        assert!(e.contains("is not inside a Git repository"), "{e}");
    }
}
