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
pub fn nonce() -> Result<String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
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
