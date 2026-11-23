//! `teryon doctor --mac`: is everything the Mac app side needs in place? The checks are a pure function
//! of `Facts`, so they can be tested; `gather` collects the facts from this machine.
use std::path::PathBuf;

/// What was found on this machine.
#[derive(Default)]
pub struct Facts {
    pub notifier: Option<PathBuf>,
    /// The notifier's own report (`TeryonNotifier status`), if it answered.
    pub notifier_status: Option<serde_json::Value>,
    pub menu_app: Option<PathBuf>,
    pub menu_running: bool,
    pub daemon_up: bool,
    /// `[notify] macos = true` in the config.
    pub banners_from_daemon: bool,
    pub shares: usize,
}

pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
    /// What to do about it when `ok` is false.
    pub fix: Option<String>,
}

fn check(name: &'static str, ok: bool, detail: impl Into<String>, fix: Option<&str>) -> Check {
    Check {
        name,
        ok,
        detail: detail.into(),
        fix: fix.filter(|_| !ok).map(String::from),
    }
}

pub fn checks(f: &Facts) -> Vec<Check> {
    let mut out = Vec::new();
    out.push(check(
        "Daemon",
        f.daemon_up,
        if f.daemon_up {
            "answering"
        } else {
            "not answering"
        },
        Some("Run `teryon` once, or `scripts/install-service.sh` to keep it running."),
    ));
    let advice = crate::util::notifier_advice(f.notifier_status.as_ref(), f.notifier.is_some());
    let native_on = f.notifier_status.as_ref().is_some_and(|s| {
        matches!(
            s.get("authorization").and_then(|v| v.as_str()),
            Some("authorized" | "provisional" | "ephemeral")
        )
    });
    out.push(check(
        "Native notifications",
        native_on,
        advice.first().cloned().unwrap_or_default(),
        Some(&advice.get(1).cloned().unwrap_or_default()),
    ));
    out.push(check(
        "Banners from the daemon",
        f.banners_from_daemon,
        if f.banners_from_daemon {
            "on"
        } else {
            "off: agents that need you will not notify when TERYON is closed"
        },
        Some("Add `macos = true` under `[notify]` in config.toml."),
    ));
    out.push(check(
        "Menu-bar item",
        f.menu_app.is_some(),
        match &f.menu_app {
            Some(p) => p.display().to_string(),
            None => "TeryonMenu.app not found".into(),
        },
        Some("Use the TERYON.app build (scripts/bundle-macos.sh adds it)."),
    ));
    if f.menu_app.is_some() {
        out.push(check(
            "Menu-bar item running",
            f.menu_running,
            if f.menu_running { "yes" } else { "no" },
            Some("Open TERYON.app/Contents/Library/TeryonMenu.app, then turn on Launch at Login in its menu."),
        ));
    }
    out.push(Check {
        name: "Read-only shares",
        ok: true,
        detail: match f.shares {
            0 => "none".into(),
            n => format!("{n} (see `teryon shares`)"),
        },
        fix: None,
    });
    out
}

pub fn render(list: &[Check]) -> String {
    let mut s = String::new();
    for c in list {
        s.push_str(&format!(
            "{} {:<24} {}\n",
            if c.ok { "✓" } else { "✗" },
            c.name,
            c.detail
        ));
        if let Some(fix) = &c.fix {
            s.push_str(&format!("    → {fix}\n"));
        }
    }
    let bad = list.iter().filter(|c| !c.ok).count();
    s.push_str(&match bad {
        0 => "\nAll good.\n".to_string(),
        1 => "\n1 thing to fix.\n".to_string(),
        n => format!("\n{n} things to fix.\n"),
    });
    s
}

/// Collect the facts from this machine.
pub fn gather() -> Facts {
    let notifier = crate::util::notifier_path();
    let menu_app = {
        const REL: &str = "Contents/Library/TeryonMenu.app";
        let mut cands: Vec<PathBuf> = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(app) = exe.ancestors().nth(3) {
                cands.push(app.join(REL));
            }
        }
        cands.push(PathBuf::from("/Applications/TERYON.app").join(REL));
        if let Ok(h) = crate::util::home() {
            cands.push(h.join("Applications/TERYON.app").join(REL));
        }
        cands.into_iter().find(|p| p.is_dir())
    };
    // The menu-bar app sets this when it runs us (it plainly is running); otherwise look for it.
    let menu_running = std::env::var_os("TERYON_MENU_RUNNING").is_some()
        || std::process::Command::new("pgrep")
            .args(["-x", "TeryonMenu"])
            .stdout(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    let daemon_up = crate::config::target("local", None)
        .and_then(|t| t.connect())
        .is_ok();
    let banners_from_daemon = crate::config::load().is_ok_and(|c| c.notify.macos);
    let shares = crate::share::default_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map_or(0, |t| crate::share::find_shares(&t).len());
    Facts {
        notifier_status: crate::util::notifier_status(),
        notifier,
        menu_app,
        menu_running,
        daemon_up,
        banners_from_daemon,
        shares,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn good() -> Facts {
        Facts {
            notifier: Some("/a/TeryonNotifier".into()),
            notifier_status: Some(
                serde_json::json!({"authorization": "authorized", "alert": "enabled", "time_sensitive": "enabled"}),
            ),
            menu_app: Some("/a/TeryonMenu.app".into()),
            menu_running: true,
            daemon_up: true,
            banners_from_daemon: true,
            shares: 0,
        }
    }
    #[test]
    fn a_fully_set_up_mac_is_all_good() {
        let c = checks(&good());
        assert!(c.iter().all(|c| c.ok && c.fix.is_none()));
        assert!(render(&c).ends_with("All good.\n"));
    }
    #[test]
    fn each_problem_gets_its_own_fix() {
        let f = Facts {
            notifier_status: Some(serde_json::json!({"authorization": "denied"})),
            menu_running: false,
            banners_from_daemon: false,
            daemon_up: false,
            ..good()
        };
        let c = checks(&f);
        let bad: Vec<&str> = c.iter().filter(|c| !c.ok).map(|c| c.name).collect();
        assert_eq!(
            bad,
            [
                "Daemon",
                "Native notifications",
                "Banners from the daemon",
                "Menu-bar item running"
            ]
        );
        assert!(c
            .iter()
            .filter(|c| !c.ok)
            .all(|c| c.fix.as_deref().is_some_and(|f| !f.is_empty())));
        assert!(render(&c).ends_with("4 things to fix.\n"));
    }
    #[test]
    fn nothing_installed_says_so_without_a_running_check() {
        let c = checks(&Facts::default());
        assert!(c.iter().all(|c| c.name != "Menu-bar item running"));
        assert!(c.iter().any(|c| c.name == "Menu-bar item" && !c.ok));
    }
}
