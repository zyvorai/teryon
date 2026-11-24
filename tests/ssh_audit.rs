// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! `teryon ssh-audit` reads a config file and never connects: the tests point it at a temporary file.
use std::process::Command;

fn run(config: &str, args: &[&str]) -> (i32, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config");
    std::fs::write(&file, config).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_teryon"))
        .arg("ssh-audit")
        .args(args)
        .arg("--file")
        .arg(&file)
        .env("HOME", dir.path())
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_clean_chain_passes_and_shows_the_path() {
    let (code, out, _) = run(
        "Host jump\n HostName j.example\n User a\nHost app\n HostName a.example\n ProxyJump jump\n",
        &["app"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("a@j.example -> a.example"), "{out}");
}

#[test]
fn an_error_finding_sets_the_exit_code_and_hostile_text_is_not_echoed_raw() {
    let (code, out, err) = run(
        "Host evil\n HostName -oProxyCommand=touch\\ /tmp/x\x1b]52;c;AAAA\x07\n",
        &[],
    );
    assert_eq!(code, 1, "{out}{err}");
    assert!(out.contains("error"), "{out}");
    assert!(!out.contains('\x1b') && !out.contains('\x07'), "{out:?}");
}

#[test]
fn json_output_is_one_object_per_host() {
    let (code, out, _) = run("Host a\n HostName x\nHost b\n HostName y\n", &["--json"]);
    assert_eq!(code, 0);
    let lines: Vec<serde_json::Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["target"], "a");
}

#[test]
fn fail_on_warn_turns_warnings_into_a_failing_exit_code() {
    let cfg = "Host a
 HostName x
 ForwardAgent yes
";
    let (code, out, _) = run(cfg, &["a"]);
    assert_eq!(code, 0, "{out}");
    let (code, out, _) = run(cfg, &["a", "--fail-on", "warn"]);
    assert_eq!(code, 1, "{out}");
    let (code, _, err) = run(cfg, &["a", "--fail-on", "nope"]);
    assert_ne!(code, 0, "{err}");
}
