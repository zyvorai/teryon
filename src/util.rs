// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

use anyhow::{bail, Context, Result};
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

pub fn runtime_dir() -> Result<PathBuf> {
    let path = match std::env::var_os("TERYON_RUNTIME_DIR") {
        Some(p) => PathBuf::from(p),
        None => home()?.join(".teryon/run"),
    };
    fs::create_dir_all(&path).context("create private runtime directory")?;
    let metadata = fs::symlink_metadata(&path)?;
    // SAFETY: geteuid takes no arguments and has no preconditions.
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.uid() != uid {
        bail!("runtime directory must be a real directory owned by the current user");
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    Ok(path)
}
pub fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")
}
pub fn socket_path() -> Result<PathBuf> {
    let p = runtime_dir()?.join("daemon.sock");
    if p.as_os_str().as_encoded_bytes().len() > 100 {
        bail!("runtime socket path is too long; set TERYON_RUNTIME_DIR to a shorter private path");
    }
    Ok(p)
}
/// The local time zone's offset from UTC right now, in seconds (0 if the C library cannot say).
pub fn utc_offset_secs() -> i64 {
    // SAFETY: as for `clock_hhmm`; `tm_gmtoff` is filled in by a successful localtime_r.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return 0;
        }
        #[allow(clippy::useless_conversion)] // tm_gmtoff is a narrower type on some targets
        i64::from(tm.tm_gmtoff)
    }
}
/// Local wall-clock time as `HH:MM`; empty if the C library cannot convert it.
pub fn clock_hhmm() -> String {
    // SAFETY: time(NULL) has no preconditions; localtime_r writes only into our zeroed `tm`.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return String::new();
        }
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
    }
}
/// A macOS notification banner: a bold title, a subtitle under it, the body, and optionally a
/// system sound (`Glass`, `Tink`, ...).
pub struct Banner<'a> {
    pub title: &'a str,
    pub subtitle: &'a str,
    pub body: &'a str,
    pub sound: Option<&'a str>,
    /// What only the native notifier can use (grouping, buttons, click target); `None` is a plain banner.
    pub native: Option<Native<'a>>,
}

/// How urgent a native notification is. `Urgent` breaks through Focus modes that allow time-sensitive
/// notifications; `Quiet` is delivered without sound or lighting the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Quiet,
    Normal,
    Urgent,
}
impl Level {
    fn word(self) -> &'static str {
        match self {
            Self::Quiet => "passive",
            Self::Normal => "active",
            Self::Urgent => "time-sensitive",
        }
    }
}

/// Extras for the native notifier: notifications with the same `thread` are grouped, a `token`
/// adds Approve and Deny buttons, and a click opens TERYON at `workspace` and `pane`.
#[derive(Clone, Debug, Default)]
pub struct Native<'a> {
    pub thread: &'a str,
    /// The host whose workspace it is (`local` for this machine).
    pub host: &'a str,
    pub workspace: &'a str,
    pub pane: u64,
    pub level: Option<Level>,
    /// The one-time token (`reply::issue`) the buttons answer with.
    pub token: Option<&'a str>,
}

/// The helper app that posts native notifications, `TeryonNotifier.app`, next to this program in
/// `TERYON.app/Contents/Library`, or in an installed TERYON.app. `TERYON_NOTIFIER` overrides it.
pub fn notifier_path() -> Option<PathBuf> {
    const REL: &str = "Contents/Library/TeryonNotifier.app/Contents/MacOS/TeryonNotifier";
    if let Some(p) = std::env::var_os("TERYON_NOTIFIER") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let mut found: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        // TERYON.app/Contents/MacOS/teryon -> TERYON.app
        if let Some(app) = exe.ancestors().nth(3) {
            found.push(app.join(REL));
        }
    }
    found.push(PathBuf::from("/Applications/TERYON.app").join(REL));
    if let Ok(h) = home() {
        found.push(h.join("Applications/TERYON.app").join(REL));
    }
    found.into_iter().find(|p| p.is_file())
}

/// The JSON the native notifier reads on stdin. `bin` is the program its buttons run.
pub fn native_payload(banner: &Banner, native: &Native, bin: &str) -> String {
    serde_json::json!({
        "title": banner.title,
        "subtitle": banner.subtitle,
        "body": banner.body,
        "sound": banner.sound,
        "thread": native.thread,
        "host": native.host,
        "workspace": native.workspace,
        "pane": native.pane,
        "level": native.level.unwrap_or(Level::Normal).word(),
        "token": native.token,
        "bin": bin,
        // A relaunched helper does not inherit this environment, so the buttons carry it.
        "runtime": std::env::var("TERYON_RUNTIME_DIR").ok(),
    })
    .to_string()
}

/// What macOS lets the native notifier do right now, as the JSON the helper prints, or `None` if the
/// helper is not installed or did not answer.
#[cfg(target_os = "macos")]
pub fn notifier_status() -> Option<serde_json::Value> {
    let out = std::process::Command::new(notifier_path()?)
        .arg("status")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    serde_json::from_slice(&out.stdout).ok()
}
#[cfg(not(target_os = "macos"))]
pub fn notifier_status() -> Option<serde_json::Value> {
    None
}

/// Plain words for what `notifier_status` found, and what to do about it. The first line says whether
/// native notifications work; the rest are the fix.
pub fn notifier_advice(status: Option<&serde_json::Value>, helper_found: bool) -> Vec<String> {
    let Some(status) = status else {
        return if helper_found {
            vec![
                "The notifier is installed but did not answer.".into(),
                "Run it from the Finder once (TERYON.app/Contents/Library/TeryonNotifier.app), then try again.".into(),
            ]
        } else {
            vec![
                "Native notifications are off: TeryonNotifier.app was not found next to this program.".into(),
                "Banners use the AppleScript fallback (no Approve/Deny buttons, no click to open a pane).".into(),
                "Use the TERYON.app build, or set TERYON_NOTIFIER to the helper's executable.".into(),
            ]
        };
    };
    let get = |k: &str| status.get(k).and_then(|v| v.as_str()).unwrap_or("?");
    match get("authorization") {
        "authorized" | "provisional" | "ephemeral" => {
            let mut v = vec!["Native notifications are on.".to_string()];
            if get("alert") == "disabled" {
                v.push("But banners are turned off for TERYON: set the alert style to Banners or Alerts in System Settings > Notifications > TERYON.".into());
            }
            if get("time_sensitive") == "disabled" {
                v.push("Time Sensitive is off, so \"needs you\" can be held back by a Focus.".into());
            }
            v
        }
        "denied" => vec![
            "Native notifications are blocked: macOS says TERYON may not notify.".into(),
            "Turn on System Settings > Notifications > TERYON > Allow Notifications. (macOS never asks twice.)".into(),
            "Until then the AppleScript fallback is used.".into(),
        ],
        _ => vec![
            "macOS has not been asked yet, so TERYON cannot notify natively.".into(),
            "Open TERYON.app/Contents/Library/TeryonNotifier.app from the Finder once and allow notifications when asked.".into(),
        ],
    }
}

/// Post `banner` through the native notifier. `Some(shown)` if it ran, `None` if it is not installed
/// or could not be started (the caller then falls back to `osascript`).
#[cfg(target_os = "macos")]
fn notify_native(banner: &Banner, native: &Native) -> Option<bool> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let helper = notifier_path()?;
    let bin = std::env::current_exe().ok()?;
    let payload = native_payload(banner, native, &bin.to_string_lossy());
    let mut child = Command::new(helper)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(payload.as_bytes()).ok()?;
    // 0 = posted. Anything else, including 2 (macOS says TERYON may not notify), falls back to the
    // AppleScript banner: an agent that needs you must not go unnoticed because of a settings mix-up.
    match child.wait().ok()?.code()? {
        0 => Some(true),
        _ => None,
    }
}

/// The `osascript` arguments that show `banner`. The text goes in as arguments, never into the
/// script, and `--` keeps a text that starts with `-` from being read as an option.
pub fn banner_args(banner: &Banner) -> Vec<String> {
    let sound = if banner.sound.is_some() {
        " sound name (item 4 of argv)"
    } else {
        ""
    };
    let mut args: Vec<String> = [
        "-e",
        "on run argv",
        "-e",
        &format!(
            "display notification (item 1 of argv) with title (item 2 of argv) subtitle (item 3 of argv){sound}"
        ),
        "-e",
        "end run",
        "--",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.extend([banner.body, banner.title, banner.subtitle].map(String::from));
    args.extend(banner.sound.map(String::from));
    args
}

/// Show `banner` and say whether macOS accepted it. No-op (and `false`) elsewhere.
pub fn notify_banner(banner: &Banner) -> bool {
    #[cfg(target_os = "macos")]
    {
        if let Some(shown) = notify_native(banner, &banner.native.clone().unwrap_or_default()) {
            return shown;
        }
        std::process::Command::new("osascript")
            .args(banner_args(banner))
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = banner;
        false
    }
}

/// A plain macOS notification titled TERYON; the message is passed as an argument and never
/// interpolated. No-op elsewhere.
pub fn notify(message: &str) {
    notify_banner(&Banner {
        title: "TERYON",
        subtitle: "",
        body: message,
        sound: None,
        native: None,
    });
}
/// A short system sound (macOS); silent elsewhere.
pub fn beep() {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("afplay")
        .arg("/System/Library/Sounds/Tink.aiff")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
pub fn nonce() -> Result<String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
/// Replace `path` with `bytes` through a 0600 temp file and a rename, creating parent directories.
pub fn write_private_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    // A unique, exclusively created temp file: concurrent writers never share one, and a stale file's mode is never reused.
    let tmp = path.with_extension(format!("{}.tmp", nonce()?));
    let written = (|| -> Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        Ok(())
    })();
    if let Err(e) = written.and_then(|()| Ok(fs::rename(&tmp, path)?)) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}
pub fn private_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut f = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    Ok(())
}
/// Lock a mutex and keep going if another thread panicked while holding it. The daemon owns every
/// live PTY, so one panicking thread must not poison the shared state and take all connections down.
pub trait LockRecover<T> {
    fn lock_recover(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> LockRecover<T> for std::sync::Mutex<T> {
    fn lock_recover(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_banner_passes_its_text_as_arguments_after_a_separator() {
        let args = banner_args(&Banner {
            title: "Claude needs you",
            subtitle: "demo · pane 2",
            body: "Apply this edit?\" & (do shell script \"x\") & \"",
            sound: Some("Glass"),
            native: None,
        });
        let sep = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(
            &args[sep + 1..],
            [
                "Apply this edit?\" & (do shell script \"x\") & \"",
                "Claude needs you",
                "demo · pane 2",
                "Glass"
            ]
        );
        // Nothing the caller supplied reaches the script text.
        let script = args[..sep].join("\n");
        assert!(
            !script.contains("shell script") && !script.contains("Claude"),
            "{script}"
        );
        assert!(script.contains("sound name (item 4 of argv)"));
    }
    #[test]
    fn a_banner_without_a_sound_has_no_sound_clause() {
        let args = banner_args(&Banner {
            title: "T",
            subtitle: "",
            body: "-starts with a dash",
            sound: None,
            native: None,
        });
        assert!(!args.join("\n").contains("sound name"));
        assert_eq!(args.last().map(String::as_str), Some(""));
        assert_eq!(
            args[args.iter().position(|a| a == "--").unwrap() + 1],
            "-starts with a dash"
        );
    }
    #[test]
    fn atomic_write_ignores_a_stale_loose_temp_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("teryon-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f.toml");
        let stale = path.with_extension("tmp");
        std::fs::write(&stale, "old").unwrap();
        std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o666)).unwrap();
        write_private_atomic(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }
    use super::*;
    #[test]
    fn quoting_handles_shell_metacharacters() {
        let value = "a'b $(touch /tmp/never)\n";
        let out = std::process::Command::new("sh")
            .args(["-c", &format!("printf %s {}", shell_quote(value))])
            .output()
            .unwrap();
        assert_eq!(out.stdout, value.as_bytes());
    }
    #[test]
    fn lock_recover_survives_a_poisoned_mutex() {
        use super::LockRecover;
        let m = std::sync::Arc::new(std::sync::Mutex::new(7));
        let m2 = m.clone();
        let _ = std::thread::spawn(move || {
            let _g = m2.lock().unwrap();
            panic!("poison it");
        })
        .join();
        assert!(m.lock().is_err());
        assert_eq!(*m.lock_recover(), 7);
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    #[test]
    fn the_native_payload_carries_grouping_level_and_token() {
        let b = Banner {
            title: "T",
            subtitle: "s",
            body: "b\"q",
            sound: Some("Glass"),
            native: None,
        };
        let n = Native {
            thread: "w/2",
            host: "local",
            workspace: "w",
            pane: 2,
            level: Some(Level::Urgent),
            token: Some("ab"),
        };
        let v: serde_json::Value =
            serde_json::from_str(&native_payload(&b, &n, "/bin/teryon")).unwrap();
        assert_eq!(v["thread"], "w/2");
        assert_eq!(v["level"], "time-sensitive");
        assert_eq!(v["token"], "ab");
        assert_eq!(v["body"], "b\"q");
        let plain = native_payload(&b, &Native::default(), "x");
        assert!(plain.contains("\"level\":\"active\"") && plain.contains("\"token\":null"));
    }
    #[test]
    fn advice_says_what_is_wrong_and_what_to_do() {
        let v = |t: &str| serde_json::json!({"authorization": t, "alert": "enabled", "time_sensitive": "enabled"});
        assert!(notifier_advice(Some(&v("authorized")), true)[0].contains("are on"));
        assert!(notifier_advice(Some(&v("denied")), true)[1].contains("Allow Notifications"));
        assert!(notifier_advice(Some(&v("not_determined")), true)[1].contains("Finder"));
        assert!(notifier_advice(None, false)[0].contains("not found"));
        assert!(notifier_advice(None, true)[0].contains("did not answer"));
        let off = serde_json::json!({"authorization": "authorized", "alert": "disabled", "time_sensitive": "disabled"});
        assert_eq!(notifier_advice(Some(&off), true).len(), 3);
    }
}
