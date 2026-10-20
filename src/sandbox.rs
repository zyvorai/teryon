//! `teryon sandbox -- COMMAND…`: run a command (an AI coding agent) so it can only write to the
//! repository it works in and the scratch and settings directories it needs, and cannot read the
//! credentials in `~/.ssh`, `~/.aws`, `~/.gnupg` or `~/.kube`. The network stays open, because an agent
//! needs its model API. macOS uses `sandbox-exec`, Linux uses `bwrap` (bubblewrap); if the tool is
//! missing the command is not run at all, instead of running without the protection.
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// Directories under `$HOME` the agents keep their settings, caches and logins in.
const HOME_WRITABLE: [&str; 11] = [
    ".claude",
    ".codex",
    ".gemini",
    ".cursor",
    ".opencode",
    ".aider",
    ".config",
    ".cache",
    ".npm",
    ".local/state",
    ".local/share",
];
/// Credential stores the sandboxed command cannot read.
const HOME_HIDDEN: [&str; 4] = [".ssh", ".aws", ".gnupg", ".kube"];

#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    /// Where the command may write (the repository and the directories above).
    pub writable: Vec<PathBuf>,
    /// What it cannot even read.
    pub hidden: Vec<PathBuf>,
    /// Files in `$HOME` whose name may be followed by anything (`.claude.json`, `.claude.json.tmp.123`):
    /// programs replace these by writing a sibling and renaming it.
    pub home_files: Vec<PathBuf>,
}

/// What the sandbox allows for work in `root` by someone whose home is `home`. `git_common` is the
/// repository's shared Git directory when `root` is a linked worktree (commits write there).
pub fn plan(root: &Path, git_common: Option<&Path>, home: &Path, extra: &[PathBuf]) -> Plan {
    let mut writable = vec![root.to_path_buf()];
    writable.extend(git_common.map(Path::to_path_buf));
    writable.extend(extra.iter().cloned());
    for tmp in [
        "/tmp",
        "/private/tmp",
        "/var/folders",
        "/private/var/folders",
    ] {
        writable.push(tmp.into());
    }
    if let Some(t) = std::env::var_os("TMPDIR") {
        writable.push(t.into());
    }
    writable.extend(HOME_WRITABLE.iter().map(|d| home.join(d)));
    writable.dedup();
    Plan {
        writable,
        hidden: HOME_HIDDEN.iter().map(|d| home.join(d)).collect(),
        home_files: vec![home.join(".claude.json")],
    }
}

/// The repository root for `dir`, and its shared Git directory when that lies outside the root.
pub fn find_root(dir: &Path) -> (PathBuf, Option<PathBuf>) {
    let git = |args: &[&str]| -> Option<PathBuf> {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .ok()?;
        let text = String::from_utf8(out.stdout).ok()?;
        let line = text.lines().next()?.trim();
        (out.status.success() && !line.is_empty()).then(|| PathBuf::from(line))
    };
    let canon = |p: PathBuf| p.canonicalize().unwrap_or(p);
    let root = git(&["rev-parse", "--show-toplevel"])
        .map(canon)
        .unwrap_or_else(|| canon(dir.to_path_buf()));
    let common = git(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .map(canon)
        .filter(|c| !c.starts_with(&root));
    (root, common)
}

fn sbpl_string(p: &Path) -> Result<String> {
    let s = p.to_str().context("path is not valid UTF-8")?;
    if s.chars().any(|c| c == '"' || c == '\\' || c.is_control()) {
        bail!("cannot sandbox a path with quotes, backslashes or control characters: {s:?}");
    }
    Ok(format!("\"{s}\""))
}
fn sbpl_regex_prefix(p: &Path) -> Result<String> {
    let s = p.to_str().context("path is not valid UTF-8")?;
    sbpl_string(p)?;
    let mut out = String::from("^");
    for c in s.chars() {
        if ".[]\\*^$+?(){}|".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    Ok(format!("#\"{out}\""))
}

/// The Seatbelt profile for `plan`: everything allowed except writing outside it and reading what is
/// hidden. Later rules win, so the denials come last.
pub fn macos_profile(plan: &Plan) -> Result<String> {
    let mut s =
        String::from("(version 1)\n(allow default)\n(deny file-write*)\n(allow file-write*\n");
    s.push_str("  (subpath \"/dev\")\n");
    for p in &plan.writable {
        s.push_str(&format!("  (subpath {})\n", sbpl_string(p)?));
    }
    for p in &plan.home_files {
        s.push_str(&format!("  (regex {})\n", sbpl_regex_prefix(p)?));
    }
    s.push_str(")\n(deny file-read* file-write*\n");
    for p in &plan.hidden {
        s.push_str(&format!("  (subpath {})\n", sbpl_string(p)?));
    }
    s.push_str(")\n");
    Ok(s)
}

/// `bwrap` arguments for `plan`: the whole filesystem read-only, the writable paths bound read-write
/// (those that exist), and the hidden ones covered by empty temporary directories.
pub fn bwrap_args(plan: &Plan) -> Vec<String> {
    let mut a: Vec<String> = [
        "--ro-bind",
        "/",
        "/",
        "--dev-bind",
        "/dev",
        "/dev",
        "--die-with-parent",
    ]
    .map(String::from)
    .into();
    for p in plan.writable.iter().filter(|p| p.exists()) {
        let p = p.display().to_string();
        a.extend(["--bind".into(), p.clone(), p]);
    }
    for p in plan.hidden.iter().filter(|p| p.exists()) {
        a.extend(["--tmpfs".into(), p.display().to_string()]);
    }
    a
}

/// Replace this process with `command` inside the sandbox. Only returns on failure.
pub fn exec(command: &[String], extra: &[PathBuf]) -> Result<std::convert::Infallible> {
    use std::os::unix::process::CommandExt;
    if command.is_empty() {
        bail!("give the command to run, after `--`");
    }
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    let home = home.canonicalize().unwrap_or(home);
    let (root, common) = find_root(&std::env::current_dir()?);
    let plan = plan(&root, common.as_deref(), &home, extra);
    let mut cmd;
    if cfg!(target_os = "macos") {
        cmd = std::process::Command::new("sandbox-exec");
        cmd.arg("-p").arg(macos_profile(&plan)?).args(command);
    } else if cfg!(target_os = "linux") {
        cmd = std::process::Command::new("bwrap");
        cmd.args(bwrap_args(&plan)).arg("--").args(command);
    } else {
        bail!("sandboxing is only available on macOS and Linux");
    }
    eprintln!(
        "[teryon] sandboxed: writes limited to {} (and scratch/settings directories); ~/.ssh, ~/.aws, ~/.gnupg and ~/.kube are hidden",
        root.display()
    );
    let err = cmd.exec();
    if err.kind() == std::io::ErrorKind::NotFound {
        bail!(
            "{} is not installed, so the command was not started (install it, or run it without the sandbox)",
            if cfg!(target_os = "macos") { "sandbox-exec" } else { "bwrap (bubblewrap)" }
        );
    }
    Err(err).context("start the sandbox")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Plan {
        plan(
            Path::new("/work/app"),
            Some(Path::new("/work/.git")),
            Path::new("/Users/me"),
            &[],
        )
    }

    #[test]
    fn the_plan_allows_the_repository_and_agent_settings_and_hides_credentials() {
        let p = sample();
        assert_eq!(p.writable[0], Path::new("/work/app"));
        assert!(p.writable.contains(&PathBuf::from("/work/.git")));
        assert!(p.writable.contains(&PathBuf::from("/Users/me/.claude")));
        assert!(p.writable.contains(&PathBuf::from("/tmp")));
        assert!(!p.writable.iter().any(|w| w == Path::new("/Users/me")));
        assert!(p.hidden.contains(&PathBuf::from("/Users/me/.ssh")));
    }

    #[test]
    fn the_macos_profile_denies_writes_by_default_and_hides_credentials_last() {
        let s = macos_profile(&sample()).unwrap();
        let deny_write = s.find("(deny file-write*)").unwrap();
        let allow = s.find("(subpath \"/work/app\")").unwrap();
        let hide = s.find("(deny file-read* file-write*").unwrap();
        assert!(deny_write < allow && allow < hide, "{s}");
        assert!(s.contains("(subpath \"/Users/me/.ssh\")"));
        assert!(
            s.contains("(regex #\"^/Users/me/\\.claude\\.json\")"),
            "{s}"
        );
        // A path that could break out of the string is refused, not escaped half-way.
        let bad = plan(
            Path::new("/work/a\")(allow default"),
            None,
            Path::new("/h"),
            &[],
        );
        assert!(macos_profile(&bad).is_err());
    }

    #[test]
    fn bwrap_binds_only_what_exists() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        let p = plan(dir.path(), None, &home, &[]);
        let args = bwrap_args(&p);
        let joined = args.join(" ");
        assert!(joined.starts_with("--ro-bind / /"));
        let me = dir.path().display();
        assert!(joined.contains(&format!("--bind {me} {me}")), "{joined}");
        assert!(joined.contains(&format!("--bind {0}/home/.claude {0}/home/.claude", me)));
        assert!(
            !joined.contains(".codex"),
            "missing directories are not bound"
        );
        assert!(joined.contains(&format!("--tmpfs {me}/home/.ssh")));
    }

    #[test]
    fn a_linked_worktree_finds_its_shared_git_directory() {
        let run = |dir: &Path, args: &[&str]| {
            let ok = std::process::Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .current_dir(dir)
                .output()
                .unwrap()
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("main");
        std::fs::create_dir_all(&main).unwrap();
        run(&main, &["init", "-q"]);
        run(&main, &["commit", "-q", "--allow-empty", "-m", "first"]);
        let wt = tmp.path().join("wt");
        run(
            &main,
            &["worktree", "add", "-q", "-b", "agent", wt.to_str().unwrap()],
        );
        let (root, common) = find_root(&wt.join("."));
        assert_eq!(root, wt.canonicalize().unwrap());
        assert_eq!(common, Some(main.join(".git").canonicalize().unwrap()));
        let (root, common) = find_root(&main);
        assert_eq!(root, main.canonicalize().unwrap());
        assert_eq!(common, None, "an ordinary repository needs nothing extra");
    }

    /// Run `script` under the real macOS sandbox with `repo` as the working repository.
    #[cfg(target_os = "macos")]
    fn sandboxed(repo: &Path, home: &Path, script: &str) -> (bool, String) {
        let (root, common) = find_root(repo);
        let profile = macos_profile(&plan(&root, common.as_deref(), home, &[])).unwrap();
        let out = std::process::Command::new("sandbox-exec")
            .arg("-p")
            .arg(profile)
            .args(["sh", "-c", script])
            .current_dir(repo)
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned()
                + &String::from_utf8_lossy(&out.stderr),
        )
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn under_sandbox_exec_writes_stay_in_the_repository_and_credentials_stay_hidden() {
        // Scratch space is writable, so the test directory must live somewhere that is not.
        let base = std::env::current_dir().unwrap().join("target/sandbox-test");
        std::fs::create_dir_all(&base).unwrap();
        let work = tempfile::tempdir_in(&base).unwrap();
        let repo = work.path().join("repo");
        let elsewhere = work.path().join("elsewhere");
        let home = work.path().join("home");
        for d in [&repo, &elsewhere, &home.join(".ssh"), &home.join(".claude")] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(home.join(".ssh/id_test"), "PRIVATE").unwrap();
        // Its own repository, so the sandbox root is `repo` and not the checkout that holds `target/`.
        assert!(std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
        let ok = |s: &str| sandboxed(&repo, &home, s);
        // Inside the repository and the agent's own settings directory: allowed.
        assert!(ok("echo a > inside.txt && echo b > ../home/.claude/x").0);
        assert!(repo.join("inside.txt").exists() && home.join(".claude/x").exists());
        // Anywhere else under the workspace: denied, and nothing is created.
        let target = elsewhere.join("outside.txt");
        let (passed, out) = ok(&format!("echo c > {}", target.display()));
        assert!(!passed && !target.exists(), "{out}");
        let (passed, _) = ok(&format!("rm {}", repo.join("inside.txt").display()));
        assert!(passed, "files in the repository can be removed");
        // Credentials cannot be read.
        let (passed, out) = ok(&format!("cat {}", home.join(".ssh/id_test").display()));
        assert!(!passed && !out.contains("PRIVATE"), "{out}");
        // Reading everything else still works, and so does scratch space.
        assert!(ok("ls / >/dev/null && echo t > \"${TMPDIR:-/tmp}/teryon-sandbox-probe\"").0);
    }
}
