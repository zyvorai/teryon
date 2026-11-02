//! `teryon agent-guard`: argument checks, and a look at a host that cannot be reached.
use std::process::Command;

fn guard(args: &[&str]) -> std::process::Output {
    let home = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_teryon"))
        .env("HOME", home.path())
        .arg("agent-guard")
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn a_bad_budget_or_interval_is_refused_with_the_accepted_forms() {
    let out = guard(&["--max", "soon", "--looks", "1"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("30s, 45m or 2h"));
    let out = guard(&["--interval", "0", "--looks", "1"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--interval"));
}

#[test]
fn an_unreachable_host_is_named_and_does_not_stop_the_guard() {
    let out = guard(&["no-such-host.invalid", "--max", "1m", "--looks", "1"]);
    assert!(out.status.success(), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Watching no-such-host.invalid"), "{err}");
    assert!(err.contains("no-such-host.invalid: unreachable"), "{err}");
}

#[test]
fn agents_json_prints_nothing_but_json_lines() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_teryon"))
        .env("HOME", home.path())
        .args(["agents", "no-such-host.invalid", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(out.stdout.is_empty(), "{:?}", out.stdout);
}
