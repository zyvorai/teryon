// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! `hist stats`, `hist prune` and `cast compact|redact` through the real binary.
use std::process::{Command, Output};

fn teryon(home: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_teryon"))
        .env("HOME", home)
        .env("SHELL", "/bin/sh")
        .args(args)
        .output()
        .unwrap()
}
fn add(home: &std::path::Path, exit: &str, cmd: &str) {
    let out = teryon(
        home,
        &[
            "hist", "add", "--cwd", "/tmp", "--exit", exit, "--ms", "50", "--", cmd,
        ],
    );
    assert!(out.status.success(), "{out:?}");
}
fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn stats_rank_commands_and_report_success_as_text_and_json() {
    let home = tempfile::tempdir().unwrap();
    add(home.path(), "0", "git status");
    add(home.path(), "1", "git status -s");
    add(home.path(), "0", "ls -l");
    let out = teryon(home.path(), &["hist", "stats"]);
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("3 commands (3 different), 67% succeeded"), "{t}");
    assert!(t.contains("2  git status"), "{t}");
    let out = teryon(home.path(), &["hist", "stats", "--json", "--top", "1"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["total"], 3);
    assert_eq!(v["top"].as_array().unwrap().len(), 1);
    let none = teryon(home.path(), &["hist", "stats", "--host", "elsewhere"]);
    assert!(text(&none).contains("No commands recorded"));
}

#[test]
fn prune_needs_a_criterion_supports_dry_run_and_dedupes() {
    let home = tempfile::tempdir().unwrap();
    add(home.path(), "0", "make");
    add(home.path(), "0", "make");
    add(home.path(), "0", "ls");
    let none = teryon(home.path(), &["hist", "prune"]);
    assert!(!none.status.success());
    let dry = teryon(home.path(), &["hist", "prune", "--dedupe", "--dry-run"]);
    assert!(
        text(&dry).contains("would drop 1 commands, would keep 2"),
        "{}",
        text(&dry)
    );
    let file = home.path().join(".teryon/history.jsonl");
    assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 3);
    let real = teryon(home.path(), &["hist", "prune", "--dedupe"]);
    assert!(
        text(&real).contains("dropped 1 commands, kept 2"),
        "{}",
        text(&real)
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 2);
    let old = teryon(home.path(), &["hist", "prune", "--older-than", "1"]);
    assert!(old.status.success());
    assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 2);
}

const CAST: &str = "{\"version\":2,\"width\":80,\"height\":24}\n[1.0,\"o\",\"a\"]\n[60.0,\"o\",\"b\"]\n[61.0,\"i\",\"hunter2\\n\"]\n";

#[test]
fn cast_compact_shortens_pauses_and_refuses_to_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let (src, dst) = (dir.path().join("a.cast"), dir.path().join("b.cast"));
    std::fs::write(&src, CAST).unwrap();
    let args = |o: &std::path::Path| {
        vec![
            "cast".to_string(),
            "compact".into(),
            src.display().to_string(),
            "-o".into(),
            o.display().to_string(),
            "--max-idle".into(),
            "1".into(),
        ]
    };
    let run = |o: &std::path::Path| {
        let a = args(o);
        let a: Vec<&str> = a.iter().map(String::as_str).collect();
        teryon(dir.path(), &a)
    };
    let out = run(&dst);
    assert!(out.status.success(), "{out:?}");
    let body = std::fs::read_to_string(&dst).unwrap();
    assert!(body.contains("[2.0,\"o\",\"b\"]"), "{body}");
    assert!(!run(&dst).status.success());
}

#[test]
fn cast_redact_replaces_secrets_and_drops_typed_input() {
    let dir = tempfile::tempdir().unwrap();
    let (src, dst) = (dir.path().join("a.cast"), dir.path().join("b.cast"));
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    std::fs::write(
        &src,
        format!("{{\"version\":2,\"width\":80,\"height\":24}}\n[1.0,\"o\",\"key {secret}\"]\n[2.0,\"i\",\"hunter2\"]\n"),
    )
    .unwrap();
    let out = teryon(
        dir.path(),
        &[
            "cast",
            "redact",
            src.to_str().unwrap(),
            "-o",
            dst.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "{out:?}");
    assert!(text(&out).contains("1 secrets replaced"), "{}", text(&out));
    let body = std::fs::read_to_string(&dst).unwrap();
    assert!(
        !body.contains(secret) && !body.contains("hunter2"),
        "{body}"
    );
    let bad = teryon(
        dir.path(),
        &[
            "cast",
            "redact",
            dir.path().join("none").to_str().unwrap(),
            "-o",
            "x",
        ],
    );
    assert!(!bad.status.success());
}
