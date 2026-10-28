//! `teryon doctor --shell`: why Tab, Ctrl-R, history or the working directory might not work in a pane.
//! It starts a zsh the way the daemon does for a pane and reports what that shell ended up with.
use crate::{history, shellinit, util};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn,
    Fail,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub level: Level,
    pub name: &'static str,
    pub detail: String,
}
fn check(level: Level, name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        level,
        name,
        detail: detail.into(),
    }
}

/// What the probe shell reported (`TPROBE key=value ...` on one line).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Probe {
    pub integrated: bool,
    pub precmd: bool,
    pub compdef: bool,
    pub fzf_tab: bool,
    pub inc_append: bool,
    pub share: bool,
    pub histfile: String,
    pub savehist: u64,
    pub zdotdir_restored: bool,
}

const PROBE_SCRIPT: &str =
    "print -r -- \"TPROBE I=${__teryon_integrated:-0} P=${+functions[__teryon_precmd]} \
C=${+functions[compdef]} F=${+functions[_fzf_tab_complete]} A=${options[incappendhistory]} \
S=${options[sharehistory]} Z=${ZDOTDIR:-unset} SV=${SAVEHIST:-0} HF=${HISTFILE:-none}\"\nexit\n";

pub fn parse_probe(output: &str) -> Option<Probe> {
    let line = output.lines().rev().find_map(|l| {
        // The prompt and OSC marks share the line, so find the marker anywhere in it.
        l.find("TPROBE I=").map(|i| &l[i + "TPROBE ".len()..])
    })?;
    let mut p = Probe::default();
    for field in line.split_whitespace() {
        let Some((k, v)) = field.split_once('=') else {
            continue;
        };
        match k {
            "I" => p.integrated = v == "1",
            "P" => p.precmd = v == "1",
            "C" => p.compdef = v == "1",
            "F" => p.fzf_tab = v == "1",
            "A" => p.inc_append = v == "on",
            "S" => p.share = v == "on",
            "Z" => p.zdotdir_restored = v == "unset" || !v.contains(".teryon/zdotdir"),
            "SV" => p.savehist = v.trim_matches('"').parse().unwrap_or(0),
            "HF" => p.histfile = v.trim_matches('"').to_string(),
            _ => {}
        }
    }
    Some(p)
}

/// Run an interactive zsh with the daemon's pane environment and read the probe line.
fn run_probe(inject: bool) -> Option<Probe> {
    let mut cmd = Command::new("zsh");
    cmd.arg("-i");
    if inject {
        let dir = shellinit::zsh_inject_dir().ok()?;
        cmd.env("ZDOTDIR", dir).env(
            "TERYON_USER_ZDOTDIR",
            std::env::var("ZDOTDIR").unwrap_or_default(),
        );
        if let Ok(exe) = std::env::current_exe() {
            cmd.env("TERYON_EXE", exe);
        }
    }
    cmd.env("TERYON", "1").env("TERM", "xterm-256color");
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child
        .stdin
        .take()?
        .write_all(PROBE_SCRIPT.as_bytes())
        .ok()?;
    let started = Instant::now();
    loop {
        if child.try_wait().ok()?.is_some() {
            break;
        }
        if started.elapsed() > Duration::from_secs(15) {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = child.wait_with_output().ok()?;
    parse_probe(&String::from_utf8_lossy(&out.stdout))
}

/// Facts gathered outside the probe shell.
pub struct Facts {
    pub shell: String,
    pub inject_enabled: bool,
    pub zshrc_loads_integration: bool,
    pub path_version: Option<String>,
    pub this_version: &'static str,
    pub history_entries: Option<usize>,
    pub history_mode_ok: bool,
    pub zsh_history_imported: bool,
}

pub fn checks(facts: &Facts, probe: Option<&Probe>) -> Vec<Check> {
    let mut out = Vec::new();
    match &facts.path_version {
        None => out.push(check(
            Level::Fail,
            "teryon on PATH",
            "not found; panes cannot load the integration (install it, e.g. `make install`)",
        )),
        Some(v) if v != facts.this_version => out.push(check(
            Level::Warn,
            "teryon on PATH",
            format!(
                "PATH has {v} but this is {}; install the new build (`make install`) and run `teryon stop --force`",
                facts.this_version
            ),
        )),
        Some(v) => out.push(check(Level::Ok, "teryon on PATH", format!("version {v}"))),
    }
    if !facts.shell.ends_with("/zsh") {
        out.push(check(
            Level::Warn,
            "login shell",
            format!(
                "{} is not zsh; only zsh panes load the integration automatically. Add `eval \"$(teryon shell-init bash)\"` (or fish) to its startup file",
                facts.shell
            ),
        ));
        return out;
    }
    let Some(p) = probe else {
        out.push(check(
            Level::Fail,
            "zsh probe",
            "could not start `zsh -i` to inspect a pane's shell",
        ));
        return out;
    };
    out.push(if p.integrated && p.precmd {
        let how = if facts.zshrc_loads_integration {
            "from your .zshrc"
        } else if facts.inject_enabled {
            "automatically (ZDOTDIR)"
        } else {
            "by other means"
        };
        check(
            Level::Ok,
            "integration",
            format!("loads in a new pane {how}: history, suggestions, Ctrl-R, Tab, cwd tracking"),
        )
    } else if !facts.inject_enabled && !facts.zshrc_loads_integration {
        check(
            Level::Fail,
            "integration",
            "not loaded: TERYON_INJECT=0 and ~/.zshrc has no `eval \"$(teryon shell-init zsh)\"` line",
        )
    } else {
        check(
            Level::Fail,
            "integration",
            "did not load in a probe zsh; run `zsh -i` and `echo $__teryon_integrated`, or set TERYON_PLUS=1",
        )
    });
    if !p.zdotdir_restored {
        out.push(check(
            Level::Fail,
            "ZDOTDIR",
            "still points at ~/.teryon/zdotdir inside the pane, so your own dotfiles are not read",
        ));
    }
    out.push(if p.fzf_tab {
        check(Level::Ok, "Tab", "fzf-tab is loaded and handles Tab; TERYON's fuzzy Tab steps aside")
    } else if p.compdef {
        check(Level::Ok, "Tab", "compinit is loaded; TERYON's fuzzy Tab can use zsh completions")
    } else {
        check(
            Level::Warn,
            "Tab",
            "compinit is not loaded in your zshrc, so Tab falls back to plain zsh (add `autoload -Uz compinit && compinit`)",
        )
    });
    out.push(
        if p.histfile.is_empty() || p.histfile == "none" || p.savehist == 0 {
            check(
                Level::Fail,
                "shell history",
                "HISTFILE/SAVEHIST are not set, so zsh keeps no history between panes",
            )
        } else if p.inc_append || p.share {
            check(
                Level::Ok,
                "shell history",
                format!(
                    "{} is written as you type (SAVEHIST={})",
                    p.histfile, p.savehist
                ),
            )
        } else {
            check(
                Level::Warn,
                "shell history",
                format!(
                "{} is only written when a pane's shell exits (set `setopt inc_append_history`)",
                p.histfile
            ),
            )
        },
    );
    out.push(match facts.history_entries {
        Some(n) if facts.history_mode_ok => check(
            Level::Ok,
            "TERYON history",
            format!(
                "{n} commands in ~/.teryon/history.jsonl{}",
                if facts.zsh_history_imported {
                    " (zsh history imported)"
                } else {
                    ""
                }
            ),
        ),
        Some(_) => check(
            Level::Warn,
            "TERYON history",
            "~/.teryon/history.jsonl is readable by others; `chmod 600` it",
        ),
        None => check(
            Level::Warn,
            "TERYON history",
            "no history yet; it fills as you run commands in a pane (imports ~/.zsh_history once)",
        ),
    });
    out
}

fn path_version() -> Option<String> {
    let out = Command::new("teryon").arg("--version").output().ok()?;
    out.status
        .success()
        .then(|| {
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .rsplit(' ')
                .next()
                .map(str::to_string)
        })
        .flatten()
}

fn gather() -> Facts {
    use std::os::unix::fs::PermissionsExt;
    let home = util::home().ok();
    let zshrc = home
        .as_ref()
        .and_then(|h| std::fs::read_to_string(h.join(".zshrc")).ok())
        .unwrap_or_default();
    let hist = history::path()
        .ok()
        .and_then(|p| std::fs::metadata(&p).ok().map(|m| (p, m)));
    Facts {
        shell: std::env::var("SHELL").unwrap_or_default(),
        inject_enabled: std::env::var("TERYON_INJECT")
            .map(|v| v != "0")
            .unwrap_or_else(|_| crate::config::load().map_or(true, |c| c.shell.inject)),
        zshrc_loads_integration: zshrc
            .lines()
            .any(|l| !l.trim_start().starts_with('#') && l.contains("shell-init")),
        path_version: path_version(),
        this_version: env!("CARGO_PKG_VERSION"),
        history_entries: hist
            .as_ref()
            .and_then(|(p, _)| std::fs::read_to_string(p).ok())
            .map(|t| history::parse(&t).len()),
        history_mode_ok: hist
            .as_ref()
            .is_none_or(|(_, m)| m.permissions().mode() & 0o077 == 0),
        zsh_history_imported: home
            .as_deref()
            .is_some_and(|h| Path::new(&h.join(".teryon/zsh-history-imported")).exists()),
    }
}

/// Inspect this machine's pane shell setup.
pub fn run() -> Vec<Check> {
    let facts = gather();
    let probe = if facts.shell.ends_with("/zsh") {
        run_probe(facts.inject_enabled)
    } else {
        None
    };
    checks(&facts, probe.as_ref())
}

pub fn render(checks: &[Check]) -> String {
    let mut out = String::new();
    for c in checks {
        let mark = match c.level {
            Level::Ok => "ok  ",
            Level::Warn => "warn",
            Level::Fail => "FAIL",
        };
        out.push_str(&format!("[{mark}] {}: {}\n", c.name, c.detail));
    }
    let bad = checks.iter().filter(|c| c.level == Level::Fail).count();
    let warn = checks.iter().filter(|c| c.level == Level::Warn).count();
    out.push_str(&format!(
        "{bad} problem{}, {warn} warning{}.\n",
        plural(bad),
        plural(warn)
    ));
    out
}
fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn facts() -> Facts {
        Facts {
            shell: "/bin/zsh".into(),
            inject_enabled: true,
            zshrc_loads_integration: false,
            path_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            this_version: env!("CARGO_PKG_VERSION"),
            history_entries: Some(10),
            history_mode_ok: true,
            zsh_history_imported: true,
        }
    }
    fn good() -> Probe {
        Probe {
            integrated: true,
            precmd: true,
            compdef: true,
            inc_append: true,
            histfile: "/h/.zsh_history".into(),
            savehist: 1000,
            zdotdir_restored: true,
            ..Probe::default()
        }
    }
    fn levels(c: &[Check]) -> Vec<Level> {
        c.iter().map(|c| c.level).collect()
    }
    #[test]
    fn probe_line_is_found_among_prompt_noise() {
        let out = "\x1b[1m%\x1b[0m \x1b]133;A\x07TPROBE I=1 P=1 C=1 F=0 A=on S=off Z=unset SV=1000 HF=/h/.zsh_history\r\n";
        let p = parse_probe(out).unwrap();
        assert_eq!(p, good());
        assert!(parse_probe("nothing here").is_none());
    }
    #[test]
    fn a_healthy_setup_has_no_warnings() {
        let c = checks(&facts(), Some(&good()));
        assert!(levels(&c).iter().all(|l| *l == Level::Ok), "{c:?}");
    }
    #[test]
    fn old_binary_missing_integration_and_unsaved_history_are_reported() {
        let mut f = facts();
        f.path_version = Some("0.20.0".into());
        let p = Probe {
            integrated: false,
            inc_append: false,
            savehist: 0,
            ..good()
        };
        let c = checks(&f, Some(&p));
        let by = |n: &str| c.iter().find(|c| c.name == n).unwrap().level;
        assert_eq!(by("teryon on PATH"), Level::Warn);
        assert_eq!(by("integration"), Level::Fail);
        assert_eq!(by("shell history"), Level::Fail);
        let mut late = good();
        late.inc_append = false;
        let c = checks(&facts(), Some(&late));
        assert_eq!(
            c.iter().find(|c| c.name == "shell history").unwrap().level,
            Level::Warn
        );
    }
    #[test]
    fn injection_off_without_a_zshrc_line_says_so() {
        let mut f = facts();
        f.inject_enabled = false;
        let p = Probe {
            integrated: false,
            ..good()
        };
        let c = checks(&f, Some(&p));
        let i = c.iter().find(|c| c.name == "integration").unwrap();
        assert!(i.detail.contains("TERYON_INJECT=0"), "{i:?}");
    }
    #[test]
    fn other_shells_get_one_hint_and_no_probe() {
        let mut f = facts();
        f.shell = "/bin/bash".into();
        let c = checks(&f, None);
        assert_eq!(c.len(), 2);
        assert_eq!(c[1].level, Level::Warn);
    }
    #[test]
    fn the_report_counts_problems() {
        let r = render(&[check(Level::Fail, "a", "x"), check(Level::Warn, "b", "y")]);
        assert!(
            r.contains("[FAIL] a: x") && r.ends_with("1 problem, 1 warning.\n"),
            "{r}"
        );
    }
}
