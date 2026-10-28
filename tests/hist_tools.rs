//! `teryon hist report` and `teryon hist replay` against a history file written with `hist add`.
use std::process::Command;

fn teryon(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_teryon"))
        .env("HOME", home)
        .env("SHELL", "/bin/sh")
        .args(args)
        .output()
        .unwrap()
}
fn add(home: &std::path::Path, cwd: &str, exit: &str, ms: &str, cmd: &str) {
    let out = teryon(
        home,
        &[
            "hist", "add", "--cwd", cwd, "--exit", exit, "--ms", ms, "--", cmd,
        ],
    );
    assert!(out.status.success(), "{out:?}");
}

#[test]
fn the_report_lists_slow_and_failing_commands_per_project() {
    let (home, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let cwd = work.path().display().to_string();
    add(home.path(), &cwd, "1", "90000", "cargo test");
    add(home.path(), &cwd, "1", "1000", "cargo test");
    add(home.path(), &cwd, "0", "20", "ls");
    let out = teryon(home.path(), &["hist", "report"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("3 commands, 2 failed (66%)"), "{text}");
    assert!(text.contains("1m30s  cargo test"), "{text}");
    assert!(text.contains("2x  cargo test"), "{text}");
    let none = teryon(home.path(), &["hist", "report", "--host", "elsewhere"]);
    assert!(String::from_utf8_lossy(&none.stdout).contains("No commands recorded"));
}

#[test]
fn replay_prints_a_cd_line_and_can_run_the_command_where_it_ran() {
    let (home, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let dir = work.path().join("it's here");
    std::fs::create_dir_all(&dir).unwrap();
    let cwd = dir.display().to_string();
    add(
        home.path(),
        &cwd,
        "0",
        "5",
        "echo REPLAYED_IN=$(basename \"$PWD\")",
    );
    add(home.path(), "/", "0", "5", "echo something else");

    let out = teryon(home.path(), &["hist", "replay", "--", "replayed"]);
    let line = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        line.trim(),
        format!(
            "cd -- '{}' && echo REPLAYED_IN=$(basename \"$PWD\")",
            cwd.replace('\'', "'\\''")
        )
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("ran just now in"));

    let run = teryon(home.path(), &["hist", "replay", "--run", "--", "replayed"]);
    assert!(
        String::from_utf8_lossy(&run.stdout).contains("REPLAYED_IN=it's here"),
        "{run:?}"
    );

    let miss = teryon(
        home.path(),
        &["hist", "replay", "--failed", "--", "replayed"],
    );
    assert!(!miss.status.success());
    assert!(String::from_utf8_lossy(&miss.stderr).contains("no earlier command matches"));
}
