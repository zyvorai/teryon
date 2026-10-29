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
