//! `teryon shell-init`: the few lines a shell needs to tell TERYON where prompts, commands and the
//! working directory are (OSC 133 and OSC 7). Everything works without them, only less smartly.
use std::path::{Path, PathBuf};

const ZSH: &str = r#"# TERYON shell integration for zsh (OSC 133 prompt marks, OSC 7 working directory).
if [[ -z "$__teryon_integrated" ]]; then
  typeset -g __teryon_integrated=1 __teryon_ran=
  __teryon_precmd() {
    local s=$?
    [[ -n "$__teryon_ran" ]] && printf '\e]133;D;%s\a' "$s"
    __teryon_ran=
    local p=${PWD//\%/%25}; p=${p// /%20}; p=${p//$'\n'/%0A}
    printf '\e]7;file://%s%s\a' "${HOST}" "$p"
    printf '\e]133;A\a'
  }
  __teryon_preexec() { __teryon_ran=1; printf '\e]133;C\a'; }
  autoload -Uz add-zsh-hook
  add-zsh-hook precmd __teryon_precmd
  add-zsh-hook preexec __teryon_preexec
fi
"#;

const BASH: &str = r#"# TERYON shell integration for bash (OSC 133 prompt marks, OSC 7 working directory).
if [ -z "$__teryon_integrated" ]; then
  __teryon_integrated=1
  __teryon_ran=
  __teryon_precmd() {
    local s=$?
    if [ -n "$__teryon_ran" ]; then printf '\e]133;D;%s\a' "$s"; fi
    __teryon_ran=
    local p=${PWD//\%/%25}; p=${p// /%20}
    printf '\e]7;file://%s%s\a' "${HOSTNAME}" "$p"
    printf '\e]133;A\a'
  }
  __teryon_preexec() {
    [ -n "$__teryon_ran" ] && return
    case "$BASH_COMMAND" in __teryon_*|*PROMPT_COMMAND*) return ;; esac
    __teryon_ran=1
    printf '\e]133;C\a'
  }
  PROMPT_COMMAND="__teryon_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
  trap '__teryon_preexec' DEBUG
fi
"#;

const FISH: &str = r#"# TERYON shell integration for fish (OSC 133 prompt marks, OSC 7 working directory).
if not set -q __teryon_integrated
  set -g __teryon_integrated 1
  function __teryon_prompt --on-event fish_prompt
    set -l s $status
    if set -q __teryon_ran
      printf '\e]133;D;%s\a' $s
      set -e __teryon_ran
    end
    printf '\e]7;file://%s%s\a' (hostname) (string replace -a '%' '%25' -- $PWD | string replace -a ' ' '%20')
    printf '\e]133;A\a'
  end
  function __teryon_preexec --on-event fish_preexec
    set -g __teryon_ran 1
    printf '\e]133;C\a'
  end
end
"#;

pub const SHELLS: [&str; 3] = ["zsh", "bash", "fish"];

/// The integration script for `shell` (`zsh`, `bash` or `fish`).
pub fn script(shell: &str) -> Option<&'static str> {
    match shell {
        "zsh" => Some(ZSH),
        "bash" => Some(BASH),
        "fish" => Some(FISH),
        _ => None,
    }
}
/// The shell named by `$SHELL` (just the file name), if it is one we support.
pub fn detect(shell_env: &str) -> Option<&'static str> {
    let name = Path::new(shell_env).file_name()?.to_str()?;
    SHELLS.into_iter().find(|s| *s == name)
}
/// Start-up file where the loader line goes.
pub fn rc_file(shell: &str, home: &Path) -> PathBuf {
    match shell {
        "zsh" => home.join(".zshrc"),
        "fish" => home.join(".config/fish/conf.d/teryon.fish"),
        _ if cfg!(target_os = "macos") => home.join(".bash_profile"),
        _ => home.join(".bashrc"),
    }
}
const START: &str = "# >>> teryon shell integration >>>";
const END: &str = "# <<< teryon shell integration <<<";
/// The marked block that loads the integration (guarded, so a machine without teryon is unaffected).
pub fn block(shell: &str) -> String {
    let load = if shell == "fish" {
        "command -q teryon; and teryon shell-init fish | source".to_string()
    } else {
        format!("command -v teryon >/dev/null 2>&1 && eval \"$(teryon shell-init {shell})\"")
    };
    format!("{START}\n{load}\n{END}\n")
}
/// `existing` with the block added or refreshed; the flag says whether anything changed.
pub fn ensure_block(existing: &str, block: &str) -> (String, bool) {
    if let (Some(a), Some(b)) = (existing.find(START), existing.find(END)) {
        if a < b {
            let end = b + END.len();
            let end = if existing[end..].starts_with('\n') {
                end + 1
            } else {
                end
            };
            let updated = format!("{}{}{}", &existing[..a], block, &existing[end..]);
            let changed = updated != existing;
            return (updated, changed);
        }
    }
    let mut out = existing.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(block);
    (out, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    #[test]
    fn every_script_emits_the_marks_teryon_reads() {
        for shell in SHELLS {
            let s = script(shell).unwrap();
            for needle in ["133;A", "133;C", "133;D", "]7;file://"] {
                assert!(s.contains(needle), "{shell} lacks {needle}");
            }
        }
        assert!(script("tcsh").is_none());
    }
    #[test]
    fn scripts_are_syntactically_valid_for_the_shells_that_are_installed() {
        let dir = tempfile::tempdir().unwrap();
        for shell in SHELLS {
            let path = dir.path().join(format!("init.{shell}"));
            std::fs::write(&path, script(shell).unwrap()).unwrap();
            let Ok(status) = Command::new(shell)
                .arg("-n")
                .arg(&path)
                .stdin(Stdio::null())
                .status()
            else {
                continue;
            };
            assert!(status.success(), "{shell} rejected its integration script");
        }
    }
    #[test]
    fn detection_and_rc_files() {
        assert_eq!(detect("/bin/zsh"), Some("zsh"));
        assert_eq!(detect("/opt/homebrew/bin/fish"), Some("fish"));
        assert_eq!(detect("/bin/tcsh"), None);
        let home = Path::new("/h");
        assert_eq!(rc_file("zsh", home), PathBuf::from("/h/.zshrc"));
        assert!(rc_file("fish", home).ends_with("conf.d/teryon.fish"));
    }
    #[test]
    fn the_rc_block_is_added_once_refreshed_in_place_and_leaves_other_text_alone() {
        let b = block("zsh");
        assert!(b.contains("command -v teryon") && b.contains("shell-init zsh"));
        let (first, changed) = ensure_block("export A=1", &b);
        assert!(changed && first.starts_with("export A=1\n\n# >>>"));
        let (again, changed) = ensure_block(&first, &b);
        assert!(!changed && again == first);
        let old = first.replace("shell-init zsh", "shell-init zsh --old");
        let (fixed, changed) = ensure_block(&format!("{old}tail\n"), &b);
        assert!(changed && fixed.ends_with("tail\n") && !fixed.contains("--old"));
        assert_eq!(fixed.matches(START).count(), 1);
        let (empty, _) = ensure_block("", &b);
        assert_eq!(empty, b);
    }
}
