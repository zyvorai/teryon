//! `teryon pick` driven through a real pseudo-terminal, the way the zsh widgets run it.
mod common;
use common::Session;

fn seeded_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    for (exit, cmd) in [("0", "echo FIRST_CMD"), ("3", "echo SECOND_CMD")] {
        let ok = std::process::Command::new(env!("CARGO_BIN_EXE_teryon"))
            .env("HOME", home.path())
            .args(["hist", "add", "--cwd", "/x", "--exit", exit, "--", cmd])
            .status()
            .unwrap();
        assert!(ok.success());
    }
    home
}

#[test]
fn history_picker_filters_and_prints_the_choice() {
    let home = seeded_home();
    let mut s = Session::start(home.path(), &["pick", "--history", "--cwd", "/x"]);
    s.expect("SECOND_CMD");
    s.expect("✗3");
    s.send(b"FIRST");
    s.send(b"\r");
    s.expect("echo FIRST_CMD");
    assert_eq!(s.exit_code(), 0);
}
#[test]
fn alt_enter_asks_the_shell_to_run_it_and_escape_cancels() {
    let home = seeded_home();
    let mut run = Session::start(home.path(), &["pick", "--history", "--cwd", "/x"]);
    run.expect("FIRST_CMD");
    run.send(b"\x1b\r");
    assert_eq!(run.exit_code(), 10);
    let mut cancel = Session::start(home.path(), &["pick", "--history", "--cwd", "/x"]);
    cancel.expect("FIRST_CMD");
    cancel.send(b"\x1b");
    assert_eq!(cancel.exit_code(), 1);
}
