//! Where the API key comes from: `ANTHROPIC_API_KEY`, then the macOS Keychain. Never config files or logs.
use anyhow::{bail, Result};
use std::process::Command;
#[cfg(target_os = "macos")]
use std::process::Stdio;

pub const KEYCHAIN_SERVICE: &str = "teryon-anthropic";

/// Look the key up through `env` and `keychain`; a blank value counts as missing.
pub fn find_with(
    env: impl Fn(&str) -> Option<String>,
    keychain: impl Fn() -> Option<String>,
) -> Option<String> {
    let clean = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    clean(env("ANTHROPIC_API_KEY")).or_else(|| clean(keychain()))
}

pub fn find() -> Option<String> {
    find_with(|k| std::env::var(k).ok(), keychain_read)
}

#[cfg(target_os = "macos")]
fn keychain_read() -> Option<String> {
    let out = Command::new("security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}
#[cfg(not(target_os = "macos"))]
fn keychain_read() -> Option<String> {
    None
}

/// Store a key in the Keychain. `security` asks for the secret on the terminal itself, so it never
/// appears in this process's arguments or in `ps`.
pub fn set_interactively() -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("--set-key uses the macOS Keychain; on this system export ANTHROPIC_API_KEY instead");
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "teryon".into());
    let status = Command::new("security")
        .args([
            "add-generic-password",
            "-U",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            &user,
            "-w",
        ])
        .status()?;
    if !status.success() {
        bail!("the Keychain refused the key");
    }
    Ok(())
}

/// A key is recognizable by prefix; used only to give a friendlier hint, never to reject a key.
pub fn looks_valid(key: &str) -> bool {
    key.starts_with("sk-ant-") && key.len() > 20
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn env_wins_over_keychain_and_blank_is_missing() {
        let env = |v: &str| (v == "ANTHROPIC_API_KEY").then(|| " sk-env ".to_string());
        assert_eq!(
            find_with(env, || Some("sk-kc".into())).as_deref(),
            Some("sk-env")
        );
        let blank = |_: &str| Some("  ".to_string());
        assert_eq!(
            find_with(blank, || Some("sk-kc".into())).as_deref(),
            Some("sk-kc")
        );
        assert_eq!(find_with(|_| None, || None), None);
    }
}
