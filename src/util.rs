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
/// macOS notification; the message is passed as argv and never interpolated. No-op elsewhere.
pub fn notify(message: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 1 of argv) with title \"TERYON\"",
            "-e",
            "end run",
        ])
        .arg(message)
        .status();
    #[cfg(not(target_os = "macos"))]
    let _ = message;
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
#[cfg(test)]
mod tests {
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
}
