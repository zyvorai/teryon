//! `teryon snip`: saved commands with placeholders.
use std::process::Command;

fn snip(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_teryon"))
        .env("HOME", home)
        .env("SHELL", "/bin/sh")
        .arg("snip")
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap()
}
fn out(o: &std::process::Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn snippets_are_added_listed_filled_run_and_removed() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    let add = snip(
        h,
        &[
            "add",
            "greet",
            "-d",
            "say hi",
            "--",
            "echo",
            "hello {{who}} from {{where}}",
        ],
    );
    assert!(add.status.success(), "{add:?}");
    assert!(
        snip(h, &["add", "bad", "--", "echo {{no close"])
            .status
            .code()
            != Some(0)
    );

    assert_eq!(out(&snip(h, &["list", "--names"])), "greet\n");
    assert_eq!(
        out(&snip(h, &["list"])),
        "greet\techo hello {{who}} from {{where}}\n"
    );
    assert_eq!(
        out(&snip(h, &["show", "greet"])),
        "echo hello {{who}} from {{where}}\n"
    );
    assert_eq!(
        out(&snip(h, &["fill", "greet", "who=Ada", "where=here"])),
        "echo hello Ada from here\n"
    );

    let run = snip(h, &["run", "greet", "who=Ada", "where=the lab"]);
    assert_eq!(out(&run), "hello Ada from the lab\n");

    let bad = snip(h, &["fill", "greet", "who"]);
    assert!(!bad.status.success());
    let none = snip(h, &["show", "nope"]);
    assert!(String::from_utf8_lossy(&none.stderr).contains("no snippet named 'nope'"));

    assert!(snip(h, &["rm", "greet"]).status.success());
    assert_eq!(out(&snip(h, &["list"])), "");
    assert!(!snip(h, &["rm", "greet"]).status.success());
}

#[test]
fn config_snippets_are_listed_and_a_saved_one_replaces_them() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".config/teryon");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("config.toml"),
        "[[desktop.snippets]]\nname = \"logs\"\ncommand = \"tail -f {{file}}\"\n",
    )
    .unwrap();
    assert_eq!(out(&snip(home.path(), &["list", "--names"])), "logs\n");
    snip(home.path(), &["add", "logs", "--", "journalctl -f"]);
    assert_eq!(
        out(&snip(home.path(), &["show", "logs"])),
        "journalctl -f\n"
    );
}
