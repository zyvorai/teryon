//! `teryon setup`: a short, re-runnable wizard. Every step asks before it changes anything.
use crate::{
    config,
    protocol::{Request, Response},
    theme::BUILTINS,
};
use anyhow::Result;
use std::io::{BufRead, Write};

pub trait Io {
    fn say(&mut self, text: &str);
    fn ask(&mut self, question: &str, default: &str) -> String;
    fn confirm(&mut self, question: &str, default: bool) -> bool;
}
/// Everything the wizard touches outside itself, so the flow can be tested.
pub trait Deps {
    fn tool_exists(&self, name: &str) -> bool;
    fn ssh_hosts(&self) -> Vec<String>;
    fn configured_hosts(&self) -> Vec<String>;
    fn add_hosts(&mut self, names: &[String]) -> Result<Vec<String>>;
    /// A one-line description of the daemon on `host`, or why it cannot be reached.
    fn check_host(&mut self, host: &str) -> Result<String>;
    fn current_theme(&self) -> String;
    fn set_theme(&mut self, name: &str) -> Result<()>;
    /// The user's login shell if shell integration supports it.
    fn shell(&self) -> Option<String>;
    /// Add the integration block to the shell's rc file; returns the file and whether it changed.
    fn install_shell_integration(&mut self, shell: &str) -> Result<(std::path::PathBuf, bool)>;
}

pub struct Console {
    /// Accept every default without asking (`teryon setup --yes`).
    pub assume_yes: bool,
}
impl Io for Console {
    fn say(&mut self, text: &str) {
        println!("{text}");
    }
    fn ask(&mut self, question: &str, default: &str) -> String {
        if self.assume_yes {
            return default.to_string();
        }
        print!("{question} [{default}]: ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        let line = line.trim();
        if line.is_empty() {
            default.to_string()
        } else {
            line.to_string()
        }
    }
    fn confirm(&mut self, question: &str, default: bool) -> bool {
        if self.assume_yes {
            return default;
        }
        let hint = if default { "Y/n" } else { "y/N" };
        let answer = self.ask(&format!("{question} ({hint})"), "");
        match answer.to_ascii_lowercase().as_str() {
            "" => default,
            "y" | "yes" => true,
            _ => false,
        }
    }
}

pub struct Real {
    pub bin: Option<String>,
}
impl Deps for Real {
    fn tool_exists(&self, name: &str) -> bool {
        std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let p = dir.join(name);
                p.is_file()
            })
        })
    }
    fn ssh_hosts(&self) -> Vec<String> {
        crate::util::home()
            .ok()
            .and_then(|h| std::fs::read_to_string(h.join(".ssh/config")).ok())
            .map(|t| config::ssh_hosts(&t))
            .unwrap_or_default()
    }
    fn configured_hosts(&self) -> Vec<String> {
        config::load()
            .map(|c| c.hosts.keys().cloned().collect())
            .unwrap_or_default()
    }
    fn add_hosts(&mut self, names: &[String]) -> Result<Vec<String>> {
        config::add_hosts(names)
    }
    fn check_host(&mut self, host: &str) -> Result<String> {
        let mut rpc = config::target(host, self.bin.clone())?.connect()?;
        match rpc.call(Request::Info)? {
            Response::Info {
                os,
                arch,
                sessions,
                panes,
                ..
            } => Ok(format!(
                "{os} {arch}, {sessions} workspace(s), {panes} pane(s)"
            )),
            _ => anyhow::bail!("unexpected response"),
        }
    }
    fn current_theme(&self) -> String {
        config::load().map_or_else(|_| "mac27-dark".into(), |c| c.theme)
    }
    fn set_theme(&mut self, name: &str) -> Result<()> {
        config::set_theme(name)
    }
    fn shell(&self) -> Option<String> {
        std::env::var("SHELL")
            .ok()
            .and_then(|s| crate::shellinit::detect(&s))
            .map(str::to_string)
    }
    fn install_shell_integration(&mut self, shell: &str) -> Result<(std::path::PathBuf, bool)> {
        let path = crate::shellinit::rc_file(shell, &crate::util::home()?);
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let (updated, changed) =
            crate::shellinit::ensure_block(&existing, &crate::shellinit::block(shell));
        if changed {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let backup = path.with_extension("teryon-bak");
            if !existing.is_empty() && !backup.exists() {
                std::fs::write(&backup, &existing)?;
            }
            std::fs::write(&path, updated)?;
        }
        Ok((path, changed))
    }
}

/// Run every step in order. Returns the host that was checked, if any.
pub fn run(io: &mut dyn Io, deps: &mut dyn Deps) -> Result<Option<String>> {
    io.say("TERYON setup: each step asks first and can be re-run any time.\n");
    // 1. Tools
    for (tool, need) in [
        ("ssh", "required to reach other machines"),
        ("rsync", "needed for push/pull and file drops (rsync 3+)"),
    ] {
        if deps.tool_exists(tool) {
            io.say(&format!("✓ {tool} found"));
        } else {
            io.say(&format!("✗ {tool} not found: {need}"));
        }
    }
    // 2. Hosts from ~/.ssh/config
    let known = deps.configured_hosts();
    let new: Vec<String> = deps
        .ssh_hosts()
        .into_iter()
        .filter(|h| !known.contains(h))
        .collect();
    if new.is_empty() {
        io.say("\nNo new hosts found in ~/.ssh/config.");
    } else {
        io.say(&format!(
            "\nFound {} host(s) in ~/.ssh/config: {}",
            new.len(),
            new.join(", ")
        ));
        if io.confirm(
            "Add them to TERYON's config? (your SSH config is never changed)",
            true,
        ) {
            match deps.add_hosts(&new) {
                Ok(added) => io.say(&format!("✓ added {}", added.join(", "))),
                Err(e) => io.say(&format!("✗ could not update the config: {e:#}")),
            }
        }
    }
    // 3. Reachability
    let hosts = deps.configured_hosts();
    let default_host = hosts.first().cloned().unwrap_or_else(|| "local".into());
    let host = io.ask("\nWhich host should I check?", &default_host);
    let checked = match deps.check_host(&host) {
        Ok(info) => {
            io.say(&format!("✓ {host}: {info}"));
            Some(host.clone())
        }
        Err(e) => {
            io.say(&format!("✗ {host}: {e:#}"));
            io.say(&format!(
                "  Fix it with: teryon doctor {host} --fix --binary PATH_TO_A_BUILD_FOR_THAT_MACHINE"
            ));
            None
        }
    };
    // 4. Theme
    io.say(&format!("\nThemes: {}", BUILTINS.join(", ")));
    let current = deps.current_theme();
    let name = io.ask("Theme", &current);
    if name != current {
        if BUILTINS.contains(&name.as_str()) {
            match deps.set_theme(&name) {
                Ok(()) => io.say(&format!("✓ theme set to {name}")),
                Err(e) => io.say(&format!("✗ could not save the theme: {e:#}")),
            }
        } else {
            io.say(&format!("✗ unknown theme \"{name}\"; keeping {current}"));
        }
    }
    // 5. Shell integration (this machine's shell; remote hosts get the same line in their own rc)
    match deps.shell() {
        Some(shell) => {
            io.say("\nShell integration (optional) tells TERYON where prompts and commands are, which enables");
            io.say("jumping between prompts, copying a command's output, exit-status chips and \"command finished\"");
            io.say("notifications. It only affects the machine where the shell runs: for a remote host, add the");
            io.say(&format!(
                "same line to that host's shell rc:\n\n{}",
                crate::shellinit::block(&shell)
            ));
            if io.confirm(
                &format!("Add it to your {shell} startup file on THIS machine?"),
                false,
            ) {
                match deps.install_shell_integration(&shell) {
                    Ok((path, true)) => io.say(&format!(
                        "✓ updated {} (a backup is next to it)",
                        path.display()
                    )),
                    Ok((path, false)) => io.say(&format!("✓ {} already has it", path.display())),
                    Err(e) => io.say(&format!("✗ could not update the startup file: {e:#}")),
                }
            }
        }
        None => io.say(
            "\nShell integration supports zsh, bash and fish; your login shell is not one of them.",
        ),
    }
    // 6. Next
    io.say("\nAll set. Start with:");
    io.say("  teryon                      pick a host and workspace");
    io.say(&format!(
        "  teryon connect {host} -w NAME  connect directly"
    ));
    io.say("  teryon desktop              the native app (build with --features desktop)");
    Ok(checked)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Script {
        answers: Vec<String>,
        said: Vec<String>,
        allow_rc: bool,
    }
    impl Io for Script {
        fn say(&mut self, text: &str) {
            self.said.push(text.to_string());
        }
        fn ask(&mut self, _q: &str, default: &str) -> String {
            if self.answers.is_empty() {
                default.to_string()
            } else {
                self.answers.remove(0)
            }
        }
        fn confirm(&mut self, q: &str, default: bool) -> bool {
            if q.contains("startup file") {
                return self.allow_rc;
            }
            default
        }
    }
    #[derive(Default)]
    struct Fake {
        rsync: bool,
        ssh: Vec<String>,
        configured: Vec<String>,
        reachable: bool,
        added: Vec<String>,
        theme: Option<String>,
        shell: Option<String>,
        installed: Vec<String>,
    }
    impl Deps for Fake {
        fn tool_exists(&self, name: &str) -> bool {
            name == "ssh" || self.rsync
        }
        fn ssh_hosts(&self) -> Vec<String> {
            self.ssh.clone()
        }
        fn configured_hosts(&self) -> Vec<String> {
            let mut c = self.configured.clone();
            c.extend(self.added.clone());
            c
        }
        fn add_hosts(&mut self, names: &[String]) -> Result<Vec<String>> {
            self.added.extend(names.to_vec());
            Ok(names.to_vec())
        }
        fn check_host(&mut self, host: &str) -> Result<String> {
            if self.reachable {
                Ok(format!("macOS arm64 for {host}"))
            } else {
                anyhow::bail!("connection refused")
            }
        }
        fn current_theme(&self) -> String {
            "mac27-dark".into()
        }
        fn set_theme(&mut self, name: &str) -> Result<()> {
            self.theme = Some(name.into());
            Ok(())
        }
        fn shell(&self) -> Option<String> {
            self.shell.clone()
        }
        fn install_shell_integration(&mut self, shell: &str) -> Result<(std::path::PathBuf, bool)> {
            self.installed.push(shell.to_string());
            Ok((std::path::PathBuf::from("/h/.zshrc"), true))
        }
    }
    fn go(deps: &mut Fake, answers: &[&str]) -> (Option<String>, String) {
        let mut io = Script {
            answers: answers.iter().map(|s| s.to_string()).collect(),
            said: Vec::new(),
            allow_rc: deps.shell.as_deref() == Some("zsh-yes"),
        };
        let r = run(&mut io, deps).unwrap();
        (r, io.said.join("\n"))
    }
    #[test]
    fn imports_new_hosts_checks_one_and_sets_the_theme() {
        let mut d = Fake {
            ssh: vec!["mini".into(), "box".into()],
            configured: vec!["box".into()],
            reachable: true,
            ..Default::default()
        };
        let (checked, said) = go(&mut d, &["mini", "dracula"]);
        assert_eq!(d.added, ["mini"]);
        assert_eq!(checked.as_deref(), Some("mini"));
        assert_eq!(d.theme.as_deref(), Some("dracula"));
        assert!(said.contains("✓ mini: macOS arm64") && said.contains("✗ rsync not found"));
    }
    #[test]
    fn unreachable_hosts_get_the_exact_fix_command_and_bad_themes_are_rejected() {
        let mut d = Fake::default();
        let (checked, said) = go(&mut d, &["mini", "no-such-theme"]);
        assert_eq!(checked, None);
        assert!(said.contains("teryon doctor mini --fix --binary"));
        assert!(said.contains("unknown theme"));
        assert_eq!(d.theme, None);
    }
    #[test]
    fn shell_integration_is_offered_but_only_installed_after_a_yes() {
        let mut d = Fake {
            shell: Some("zsh".into()),
            reachable: true,
            ..Default::default()
        };
        let (_, said) = go(&mut d, &[]);
        assert!(
            said.contains("Shell integration (optional)")
                && said.contains("# >>> teryon shell integration")
        );
        assert!(d.installed.is_empty(), "default is no");
        let mut d = Fake {
            shell: Some("zsh-yes".into()),
            reachable: true,
            ..Default::default()
        };
        let (_, said) = go(&mut d, &[]);
        assert_eq!(d.installed, ["zsh-yes"]);
        assert!(said.contains("updated /h/.zshrc"));
        let mut d = Fake::default();
        let (_, said) = go(&mut d, &[]);
        assert!(said.contains("supports zsh, bash and fish"));
    }
    #[test]
    fn rerunning_with_everything_configured_changes_nothing() {
        let mut d = Fake {
            ssh: vec!["mini".into()],
            configured: vec!["mini".into()],
            reachable: true,
            rsync: true,
            ..Default::default()
        };
        let (_, said) = go(&mut d, &[]);
        assert!(d.added.is_empty() && d.theme.is_none());
        assert!(said.contains("No new hosts"));
    }
}
