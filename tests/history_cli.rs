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

#[test]
fn commands_with_secrets_are_never_recorded_or_suggested_and_old_ones_can_be_scrubbed() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    let add = |cmd: &str| {
        teryon(home.path(), &["hist", "add", "--cwd", "/a", "--", cmd]);
    };
    add("deploy --token=abc123def456 prod");
    add("git clone https://me:p4ssw0rd@example.com/r.git");
    add("deploy --stage prod");
    let file = home.path().join(".teryon/history.jsonl");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("deploy --stage prod"));
    assert!(!text.contains("abc123def456") && !text.contains("p4ssw0rd"));
    // Nothing that looks like a secret comes back as a suggestion.
    assert_eq!(
        teryon(
            home.path(),
            &["hist", "suggest", "--cwd", "/a", "--", "deploy --t"]
        ),
        ""
    );
    // History written before this check can still hold secrets; `scrub` removes exactly those.
    let old =
        "{\"cmd\":\"export API_KEY=hunter2hunter2\",\"cwd\":\"/a\",\"exit\":0,\"ms\":1,\"at\":5}\n";
    let mut all = std::fs::read_to_string(&file).unwrap();
    all.push_str(old);
    std::fs::write(&file, all).unwrap();
    let said = teryon(home.path(), &["hist", "scrub"]);
    assert_eq!(
        said,
        "Removed 1 command that contained a secret from the history.\n"
    );
    let after = std::fs::read_to_string(&file).unwrap();
    assert!(!after.contains("hunter2hunter2") && after.contains("deploy --stage prod"));
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(teryon(home.path(), &["hist", "scrub"]).starts_with("Removed 0 commands"));
}
