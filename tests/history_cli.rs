//! `teryon hist` as the zsh widgets call it, against a private HOME.
use std::process::Command;

fn teryon(home: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_teryon"))
        .env("HOME", home)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "{args:?} failed: {out:?}");
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn recorded_commands_come_back_as_suggestions_and_the_file_is_private() {
    let home = tempfile::tempdir().unwrap();
    teryon(
        home.path(),
        &["hist", "add", "--cwd", "/a", "--", "cargo test --all"],
    );
    teryon(
        home.path(),
        &["hist", "add", "--cwd", "/a", "--", " hidden --token x"],
    );
    teryon(
        home.path(),
        &[
            "hist",
            "add",
            "--cwd",
            "/b",
            "--exit",
            "1",
            "--ms",
            "30",
            "--",
            "cargo build",
        ],
    );
    let hint = teryon(
        home.path(),
        &["hist", "suggest", "--cwd", "/a", "--", "cargo t"],
    );
    assert_eq!(hint, "cargo test --all\n");
    let other = teryon(
        home.path(),
        &["hist", "suggest", "--cwd", "/a", "--", "cargo b"],
    );
    assert_eq!(other, "cargo build\n");
    assert_eq!(
        teryon(home.path(), &["hist", "suggest", "--", "hidden"]),
        ""
    );
    let file = home.path().join(".teryon/history.jsonl");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains("hidden") && text.contains("\"exit\":1"));
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
