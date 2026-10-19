//! A failed SSH connection reports why ssh failed, so the app can say what to do about it.
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

/// Run `teryon doctor mini` with a fake `ssh` that prints `message` and exits like OpenSSH does.
fn doctor_with_ssh_saying(message: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let ssh = dir.path().join("ssh");
    std::fs::write(&ssh, format!("#!/bin/sh\necho '{message}' >&2\nexit 255\n")).unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let home = tempfile::tempdir().unwrap();
    let path = format!(
        "{}:{}",
        dir.path().display(),
        std::env::var("PATH").unwrap()
    );
    let out = Command::new(env!("CARGO_BIN_EXE_teryon"))
        .env("HOME", home.path())
        .env("PATH", path)
        .args(["doctor", "mini"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn the_reason_ssh_gave_reaches_the_error() {
    for (said, cause) in [
        (
            "user@mini: Permission denied (publickey).",
            teryon::diagnose::Cause::Auth,
        ),
        (
            "ssh: connect to host mini port 22: Connection refused",
            teryon::diagnose::Cause::SshDown,
        ),
        (
            "ssh: connect to host mini port 22: Operation timed out",
            teryon::diagnose::Cause::Unreachable,
        ),
    ] {
        let shown = doctor_with_ssh_saying(said);
        assert!(shown.contains(said), "ssh's words are kept: {shown}");
        assert_eq!(
            teryon::diagnose::diagnose(&shown, "mini").cause,
            cause,
            "{shown}"
        );
    }
}
