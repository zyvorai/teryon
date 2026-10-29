//! The bash and fish integrations driven through real shells in a pseudo-terminal. Each test is
//! skipped when its shell is missing (bash also needs version 4+ for the pickers).
mod common;
use common::Session;
use std::process::Command;
use std::time::{Duration, Instant};

fn have(shell: &str) -> bool {
    Command::new(shell)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}
fn bash_ok() -> bool {
    Command::new("bash")
        .args(["-c", "(( BASH_VERSINFO[0] >= 4 ))"])
        .status()
        .is_ok_and(|s| s.success())
}
fn teryon(home: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_teryon"))
        .env("HOME", home)
        .args(args)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}
/// `hist add` runs in the background in the shells; wait for it to land.
fn eventually_exported(home: &std::path::Path, needle: &str) -> bool {
    let end = Instant::now() + Duration::from_secs(8);
    while Instant::now() < end {
        if teryon(home, &["hist", "export"]).contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    false
}
fn pause() {
    std::thread::sleep(Duration::from_millis(400));
}
fn bash(home: &std::path::Path, work: &std::path::Path) -> Session {
    let init = teryon(home, &["shell-init", "bash"]);
    std::fs::write(home.join("init.bash"), init).unwrap();
    let mut s = Session::start_program("bash", home, &["--norc", "--noprofile", "-i"], Some(work));
    s.send(b"PS1='READY> '; HISTFILE=$HOME/.bash_history; HISTCONTROL=ignorespace\n");
    s.expect("READY> ");
    s.send(b"source $HOME/init.bash\n");
    s.send(b"echo LOADED_$((6*7))\n");
    s.expect("LOADED_42");
    // Let the prompt come back before keys go in, or the terminal is still in line mode.
    pause();
    s
}

#[test]
fn bash_records_commands_flushes_history_and_skips_hidden_ones() {
    if !have("bash") {
        return;
    }
    let (home, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut s = bash(home.path(), work.path());
    s.send(b"echo BASH_REC_$((6*7))\n");
    s.expect("BASH_REC_42");
    s.send(b" echo BASH_HIDDEN_$((6*7))\n");
    s.expect("BASH_HIDDEN_42");
    s.send(b"false\n");
    s.send(b"echo AFTER_$((6*7))\n");
    s.expect("AFTER_42");
    // Each command is recorded by its own background `teryon hist add`, so they can finish in any
    // order: wait for every entry that is asserted on, not just the last one.
    for entry in [
        "echo BASH_REC_$((6*7))",
        "\"cmd\":\"false\"",
        "echo AFTER_$((6*7))",
    ] {
        assert!(
            eventually_exported(home.path(), entry),
            "never recorded: {entry}"
        );
    }
    let export = teryon(home.path(), &["hist", "export"]);
    assert!(export.contains("echo BASH_REC_$((6*7))"), "{export}");
    assert!(!export.contains("BASH_HIDDEN"), "{export}");
    assert!(
        export.contains("\"cmd\":\"false\"") && export.contains("\"exit\":1"),
        "{export}"
    );
    // The shell is still running, yet its history is already on disk.
    let file = std::fs::read_to_string(home.path().join(".bash_history")).unwrap_or_default();
    assert!(file.contains("echo BASH_REC_$((6*7))"), "{file}");
}

#[test]
fn bash_z_jumps_to_a_visited_directory() {
    if !have("bash") {
        return;
    }
    let (home, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::create_dir_all(work.path().join("proj-alpha")).unwrap();
    std::fs::create_dir_all(work.path().join("other")).unwrap();
    let mut s = bash(home.path(), work.path());
    s.send(b"cd proj-alpha\n");
    s.send(b"cd ../other\n");
    s.send(b"echo VISITED_$((6*7))\n");
    s.expect("VISITED_42");
    pause();
    s.send(b"z proj\n");
    s.send(b"basename \"$PWD\"; echo JUMPED_$((6*7))\n");
    s.expect("proj-alpha");
    s.expect("JUMPED_42");
}

#[test]
fn bash_ctrl_r_opens_the_picker_and_puts_the_choice_on_the_line() {
    if !have("bash") || !bash_ok() {
        return;
    }
    let (home, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    teryon(
        home.path(),
        &[
            "hist",
            "add",
            "--cwd",
            "/x",
            "--",
            "echo PICKED_FROM_HISTORY",
        ],
    );
    let mut s = bash(home.path(), work.path());
    s.send(b"\x12");
    s.expect("history ❯");
    s.expect("PICKED_FROM_HISTORY");
    s.send(b"PICKED");
    pause();
    s.send(b"\r");
    s.expect("READY> echo PICKED_FROM_HISTORY");
    pause();
    s.send(b"\n");
    s.expect("PICKED_FROM_HISTORY\r\n");
}

/// fish drops keys typed while its startup terminal queries are unanswered, so it is driven with `-c`:
/// the event handlers are emitted by hand and the bindings are read back.
fn fish_c(home: &std::path::Path, work: &std::path::Path, script: &str) -> String {
    let init = teryon(home, &["shell-init", "fish"]);
    std::fs::write(home.join("init.fish"), init).unwrap();
    let out = Command::new("fish")
        .args([
            "--no-config",
            "-i",
            "-c",
            &format!("source $HOME/init.fish; {script}"),
        ])
        .env("HOME", home)
        .env(
            "PATH",
            format!(
                "{}:{}",
                std::path::Path::new(env!("CARGO_BIN_EXE_teryon"))
                    .parent()
                    .unwrap()
                    .display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .current_dir(work)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn fish_records_commands_and_jumps_with_z() {
    if !have("fish") {
        return;
    }
    let (home, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::create_dir_all(work.path().join("proj-alpha")).unwrap();
    std::fs::create_dir_all(work.path().join("other")).unwrap();
    let out = fish_c(
        home.path(),
        work.path(),
        "emit fish_postexec 'echo FISH_REC_42'; cd proj-alpha; cd ../other; sleep 1; z proj; echo JUMPED_(basename $PWD)",
    );
    assert!(out.contains("JUMPED_proj-alpha"), "{out}");
    assert!(eventually_exported(home.path(), "echo FISH_REC_42"));
}

#[test]
fn fish_binds_the_pickers() {
    if !have("fish") {
        return;
    }
    let (home, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let out = fish_c(home.path(), work.path(), "bind \\cr; bind \\ct; bind \\ec");
    for w in [
        "__teryon_history_widget",
        "__teryon_file_widget",
        "__teryon_cd_widget",
    ] {
        assert!(out.contains(w), "{w} not bound: {out}");
    }
}

#[test]
fn bash_loads_a_trusted_project_env_on_cd_and_puts_it_back() {
    if !have("bash") {
        return;
    }
    let (home, work) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let proj = work.path().join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join(".teryon.toml"),
        "[env]\nPROJ_VAR = \"hello\"\nKEPT = \"project\"\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_teryon"))
        .env("HOME", home.path())
        .current_dir(&proj)
        .args(["env", "allow"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let mut s = bash(home.path(), work.path());
    s.send(b"export KEPT=mine\n");
    s.send(b"cd proj\n");
    s.send(b"echo IN_${PROJ_VAR:-unset}_${KEPT}\n");
    s.expect("IN_hello_project");
    s.send(b"cd ..\n");
    s.send(b"echo OUT_${PROJ_VAR:-unset}_${KEPT}\n");
    s.expect("OUT_unset_mine");
}
