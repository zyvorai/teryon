//! The zsh integration driven through a real zsh in a pseudo-terminal. Skipped without zsh.
mod common;
use common::Session;
use std::process::Command;

fn have_zsh() -> bool {
    Command::new("zsh").arg("--version").output().is_ok()
}
fn teryon(home: &std::path::Path, args: &[&str]) {
    let ok = Command::new(env!("CARGO_BIN_EXE_teryon"))
        .env("HOME", home)
        .args(args)
        .status()
        .unwrap();
    assert!(ok.success());
}
/// A zsh with the integration loaded (and compinit plus a `foo` completer when asked for).
fn shell(home: &std::path::Path, work: &std::path::Path, completion: bool) -> Session {
    let init = Command::new(env!("CARGO_BIN_EXE_teryon"))
        .args(["shell-init", "zsh"])
        .output()
        .unwrap()
        .stdout;
    std::fs::write(home.join("init.zsh"), init).unwrap();
    let mut s = Session::start_program("zsh", home, &["-fi"], Some(work));
    s.send(b"PROMPT='READY> '\n");
    s.expect("READY> ");
    if completion {
        s.send(b"autoload -Uz compinit; compinit -u -d $HOME/.zcompdump\n");
        s.send(b"_foo() { compadd alpha beta gamma; }; compdef _foo foo\n");
        s.send(b"foo() { print -r -- \"ARGS:$*\"; }\n");
        s.send(b"print -r -- COMP_$((6*7))\n");
        s.expect("COMP_42");
    }
    s.send(b"source $HOME/init.zsh\n");
    s.send(b"print -r -- LOADED_$((6*7))\n");
    s.expect("LOADED_42");
    s
}
/// zsh abandons a completion when more input is already waiting, so keys go in one at a time.
fn pause() {
    std::thread::sleep(std::time::Duration::from_millis(400));
}
fn workdir() -> tempfile::TempDir {
    let work = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(work.path().join("src/deep")).unwrap();
    for f in ["alpha.txt", "my file.txt", "src/main.rs", "src/lib.rs"] {
        std::fs::write(work.path().join(f), "").unwrap();
    }
    work
}

#[test]
fn suggestions_come_from_history_and_the_right_arrow_accepts_them() {
    if !have_zsh() {
        return;
    }
    let (home, work) = (tempfile::tempdir().unwrap(), workdir());
    teryon(
        home.path(),
        &["hist", "add", "--cwd", "/x", "--", "echo GHOST_TEXT_OK"],
    );
    let mut s = shell(home.path(), work.path(), false);
    s.send(b"echo GH");
    s.expect("GHOST_TEXT_OK");
    s.send(b"\x1b[C\n");
    s.expect("GHOST_TEXT_OK\r\n");
}
#[test]
fn tab_opens_the_picker_for_several_candidates_and_inserts_the_choice() {
    if !have_zsh() {
        return;
    }
    let (home, work) = (tempfile::tempdir().unwrap(), workdir());
    let mut s = shell(home.path(), work.path(), true);
    s.send(b"foo ");
    pause();
    s.send(b"\t");
    s.expect("pick ❯");
    s.expect("gamma");
    s.send(b"bet");
    pause();
    s.send(b"\r");
    s.expect("READY> foo beta");
    pause();
    s.send(b"\n");
    s.expect("ARGS:beta");
}
#[test]
fn tab_completes_a_single_match_inline_and_picks_fuzzily_inside_a_directory() {
    if !have_zsh() {
        return;
    }
    let (home, work) = (tempfile::tempdir().unwrap(), workdir());
    let mut s = shell(home.path(), work.path(), true);
    s.send(b"print -r -- BASE alp");
    pause();
    s.send(b"\t");
    s.expect("BASE alpha.txt");
    pause();
    s.send(b"\n");
    s.expect("BASE alpha.txt\r\n");
    s.send(b"print -r -- SUB src/");
    pause();
    s.send(b"\t");
    s.expect("deep/");
    s.send(b"mr");
    pause();
    s.send(b"\r");
    s.expect("SUB src/main.rs");
    pause();
    s.send(b"\n");
    s.expect("SUB src/main.rs\r\n");
    s.send(b"print -r -- Q my");
    pause();
    s.send(b"\t");
    s.expect("Q my\\ file.txt");
}
