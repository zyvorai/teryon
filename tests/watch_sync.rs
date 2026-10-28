//! `teryon watch-sync` argument checks that need no SSH server (the copying itself is tested in cp_ssh.rs).
use std::process::Command;

fn watch_sync(args: &[&str]) -> std::process::Output {
    let home = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_teryon"))
        .env("HOME", home.path())
        .arg("watch-sync")
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn the_source_must_be_a_directory_and_the_destination_a_host() {
    let dir = tempfile::tempdir().unwrap();
    let missing = watch_sync(&["/definitely/not/here", "mini:~/x", "--once"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("is not a directory"));

    let local = watch_sync(&[dir.path().to_str().unwrap(), "/tmp/elsewhere", "--once"]);
    assert!(!local.status.success());
    assert!(
        String::from_utf8_lossy(&local.stderr).contains("DEST must be host:path"),
        "{local:?}"
    );
}
