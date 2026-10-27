//! Candidates for shell tab completion. `teryon completions zsh` wires these into the generated
//! script through the hidden `teryon complete` command.
use crate::{config, files, util};
use anyhow::{bail, Result};
use std::process::Stdio;

/// Host aliases worth completing: configured hosts plus concrete `Host` entries in `~/.ssh/config`.
pub fn hosts() -> Vec<String> {
    let mut names: Vec<String> = config::load()
        .map(|c| c.hosts.keys().cloned().collect())
        .unwrap_or_default();
    if let Ok(home) = util::home() {
        names.extend(config::ssh_hosts_file(&home.join(".ssh/config")));
    }
    names.sort();
    names.dedup();
    names
}

/// Split a typed remote path into the directory to list (as typed) and the name being completed.
pub fn split_prefix(prefix: &str) -> (&str, &str) {
    match prefix.rfind('/') {
        Some(i) => prefix.split_at(i + 1),
        None => ("", prefix),
    }
}

/// Shell script that lists `dir` (as typed: `~/x/`, `/etc/`, `docs/` or empty) on the host.
pub fn list_script(dir: &str) -> String {
    let ls = "LC_ALL=C ls -1Ap";
    if dir.is_empty() {
        return format!("cd && {ls}");
    }
    if let Some(rest) = dir.strip_prefix("~/") {
        if rest.is_empty() {
            return format!("cd && {ls}");
        }
        return format!("cd -- \"$HOME\"/{} && {ls}", util::shell_quote(rest));
    }
    if dir.starts_with('/') {
        return format!("cd -- {} && {ls}", util::shell_quote(dir));
    }
    format!("cd && cd -- {} && {ls}", util::shell_quote(dir))
}

/// Completed paths for `prefix`, given the host's `ls -1Ap` output for the directory part.
pub fn candidates(prefix: &str, listing: &str) -> Vec<String> {
    if prefix == "~" {
        return vec!["~/".to_string()];
    }
    let (dir, partial) = split_prefix(prefix);
    let mut out: Vec<String> = files::parse_ls(listing)
        .into_iter()
        .filter(|e| e.name.starts_with(partial) && !e.name.contains(['\n', '\0']))
        .filter(|e| partial.starts_with('.') || !e.name.starts_with('.'))
        .map(|e| format!("{dir}{}{}", e.name, if e.dir { "/" } else { "" }))
        .collect();
    out.sort();
    out
}

/// Ask `host` which paths start with `prefix`. Errors (offline host, no key) just mean no candidates.
pub fn remote_paths(host: &str, prefix: &str) -> Result<Vec<String>> {
    if prefix.contains(['\n', '\0']) {
        bail!("invalid path");
    }
    let target = config::target(host, None)?;
    if target.local() {
        return Ok(Vec::new());
    }
    if prefix == "~" {
        return Ok(candidates(prefix, ""));
    }
    let (dir, _) = split_prefix(prefix);
    let out = target
        .ssh_for_completion()
        .arg(list_script(dir))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !out.status.success() {
        bail!("cannot list {dir} on {host}");
    }
    Ok(candidates(prefix, &String::from_utf8_lossy(&out.stdout)))
}

/// A miniature CLI shaped like teryon's, so the script patching is tested without the binary.
#[cfg(test)]
#[derive(clap::Parser)]
pub struct Probe {
    #[command(subcommand)]
    cmd: ProbeCmd,
}
#[cfg(test)]
#[derive(clap::Subcommand)]
enum ProbeCmd {
    Connect {
        host: String,
    },
    Push {
        host: String,
        source: String,
        destination: String,
    },
    Pull {
        host: String,
        source: String,
        destination: String,
    },
    Cp {
        #[arg(num_args = 2..)]
        paths: Vec<String>,
    },
}

const ZSH_HELPERS: &str = r#"# TERYON dynamic completion: hosts and remote paths come from `teryon complete`.
_teryon_hosts() {
  local -a h
  h=(${(f)"$(teryon complete hosts 2>/dev/null)"})
  compadd -a h
}
_teryon_remote_path() {
  local -a p
  p=(${(f)"$(teryon complete remote-path "$1" "$PREFIX" 2>/dev/null)"})
  compadd -S '' -f -a p
}
_teryon_cp_arg() {
  if [[ $PREFIX == *:* && $PREFIX != [./]* ]]; then
    local host=${PREFIX%%:*}
    compset -P '*:'
    _teryon_remote_path "$host"
  else
    local -a h
    h=(${(f)"$(teryon complete hosts 2>/dev/null)"})
    _alternative 'files:file:_files' 'hosts:host:compadd -S : -a h'
  fi
}
"#;

/// The clap-generated zsh script with host, workspace-free positional and path arguments
/// completed dynamically (hosts from config and `~/.ssh/config`, remote paths over SSH).
pub fn zsh_script(generated: &str) -> String {
    let mut out = generated.to_string();
    let hosts = regex::Regex::new(r"(?m)^('\*?:{1,2}hosts?(?: -- [^:']*)?):_default'").unwrap();
    out = hosts.replace_all(&out, "${1}:_teryon_hosts'").into_owned();
    out = out.replace(":HOST:_default'", ":HOST:_teryon_hosts'");
    // `push HOST SOURCE DEST` then `pull HOST SOURCE DEST`, in that order in the generated script.
    let src = "':source:_default'";
    let dst = "':destination:_default'";
    let push_src = "':source:_files'";
    let push_dst = "':destination:{_teryon_remote_path $words[2]}'";
    let pull_src = "':source:{_teryon_remote_path $words[2]}'";
    let pull_dst = "':destination:_files'";
    out = out.replacen(src, push_src, 1).replacen(src, pull_src, 1);
    out = out.replacen(dst, push_dst, 1).replacen(dst, pull_dst, 1);
    let paths =
        regex::Regex::new(r"(?m)^('\*::paths(?: -- [^']*(?:'\\''[^']*)*)?):_default'").unwrap();
    out = paths.replace_all(&out, "${1}:_teryon_cp_arg'").into_owned();
    format!("{ZSH_HELPERS}{out}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zsh_script_patches_every_dynamic_argument() {
        let mut cmd = <crate::complete::Probe as clap::CommandFactory>::command();
        let mut buf = Vec::new();
        clap_complete::generate(clap_complete::Shell::Zsh, &mut cmd, "probe", &mut buf);
        let out = zsh_script(&String::from_utf8(buf).unwrap());
        assert!(out.contains("':host:_teryon_hosts'"));
        assert!(out.contains("':source:_files'"));
        assert!(out.contains("':destination:{_teryon_remote_path $words[2]}'"));
        assert!(out.contains("':source:{_teryon_remote_path $words[2]}'"));
        assert!(out.contains("':destination:_files'"));
        assert!(out.contains(":_teryon_cp_arg'"));
        assert!(!out.contains(":_default'"));
    }
    #[test]
    fn prefix_splits_at_the_last_slash() {
        assert_eq!(split_prefix(""), ("", ""));
        assert_eq!(split_prefix("do"), ("", "do"));
        assert_eq!(split_prefix("~/do"), ("~/", "do"));
        assert_eq!(split_prefix("/a/b/c"), ("/a/b/", "c"));
        assert_eq!(split_prefix("docs/"), ("docs/", ""));
    }
    #[test]
    fn list_script_handles_home_absolute_and_relative() {
        assert_eq!(list_script(""), "cd && LC_ALL=C ls -1Ap");
        assert_eq!(list_script("~/"), "cd && LC_ALL=C ls -1Ap");
        assert_eq!(
            list_script("~/my dir/"),
            "cd -- \"$HOME\"/'my dir/' && LC_ALL=C ls -1Ap"
        );
        assert_eq!(list_script("/etc/"), "cd -- '/etc/' && LC_ALL=C ls -1Ap");
        assert_eq!(
            list_script("a'b/"),
            "cd && cd -- 'a'\\''b/' && LC_ALL=C ls -1Ap"
        );
    }
    #[test]
    fn candidates_filter_and_mark_folders() {
        let ls = "Documents/\n.config/\nnotes.txt\nnotes2.txt\n";
        assert_eq!(
            candidates("~/", ls),
            ["~/Documents/", "~/notes.txt", "~/notes2.txt"]
        );
        assert_eq!(candidates("~/no", ls), ["~/notes.txt", "~/notes2.txt"]);
        assert_eq!(candidates("~/.", ls), ["~/.config/"]);
        assert_eq!(candidates("~", ""), ["~/"]);
    }
}
