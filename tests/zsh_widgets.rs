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
    shell_with(home, work, completion, b"")
}
/// Like `shell`, sending `before` (for example an `export`) ahead of loading the integration.
fn shell_with(
    home: &std::path::Path,
    work: &std::path::Path,
    completion: bool,
    before: &[u8],
) -> Session {
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
    s.send(before);
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
    std::fs::create_dir_all(work.path().join("proj-a")).unwrap();
    std::fs::create_dir_all(work.path().join("proj-b")).unwrap();
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
#[test]
fn z_jumps_to_the_most_used_match_and_zi_lets_you_choose() {
    if !have_zsh() {
        return;
    }
    let (home, work) = (tempfile::tempdir().unwrap(), workdir());
    let mut s = shell(home.path(), work.path(), false);
    s.send(b"cd proj-b && cd .. && cd proj-a && cd .. && cd proj-a && cd ..\n");
    pause();
    pause();
    s.send(b"z proj\n");
    pause();
    s.send(b"print -r -- AT_$(basename $PWD)\n");
    s.expect("AT_proj-a");
    s.send(b"z nosuchplace\n");
    s.expect("z: no directory matches: nosuchplace");
    s.send(b"cd ..; zi\n");
    s.expect("pick \u{276f}");
    s.send(b"proj-b");
    pause();
    s.send(b"\r");
    pause();
    s.send(b"print -r -- NOW_$(basename $PWD)\n");
    s.expect("NOW_proj-b");
}
#[test]
fn the_opt_in_prompt_shows_git_state_and_never_runs_a_hostile_branch_name() {
    if !have_zsh() || Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let (home, work) = (tempfile::tempdir().unwrap(), workdir());
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .current_dir(work.path())
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .status()
            .unwrap();
        assert!(ok.success(), "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["commit", "-q", "--allow-empty", "-m", "first"]);
    git(&["checkout", "-q", "-b", "x$(touch${IFS}PWNED)%F{red}"]);
    std::fs::write(work.path().join("new.txt"), "").unwrap();
    let mut s = shell_with(home.path(), work.path(), false, b"export TERYON_PROMPT=1\n");
    s.send(b"\n");
    s.expect("on x$(touch${IFS}PWNED)%F{red}");
    s.expect("[?");
    s.expect("\u{276f} ");
    assert!(!work.path().join("PWNED").exists());
    s.send(b"false\n");
    s.expect("\u{2717}1");
}

/// What the daemon does for a zsh pane: point ZDOTDIR at the injection directory, keep the user's own.
fn injected_shell(home: &std::path::Path, work: &std::path::Path, zshrc: &str) -> Session {
    std::fs::write(home.join(".zshrc"), zshrc).unwrap();
    let dir = Command::new(env!("CARGO_BIN_EXE_teryon"))
        .env("HOME", home)
        .args(["shell-init", "zdotdir"])
        .output()
        .unwrap()
        .stdout;
    let dir = String::from_utf8(dir).unwrap();
    let mut s = Session::start_program("zsh", home, &["-fi"], Some(work));
    s.send(
        format!(
            "export ZDOTDIR={} TERYON_USER_ZDOTDIR=; exec zsh -i\n",
            dir.trim()
        )
        .as_bytes(),
    );
    s.expect("READY> ");
    s
}

#[test]
fn panes_load_the_integration_without_a_zshrc_line_and_keep_the_real_zdotdir() {
    if !have_zsh() {
        return;
    }
    let (home, work) = (tempfile::tempdir().unwrap(), workdir());
    let mut s = injected_shell(home.path(), work.path(), "PROMPT='READY> '\n");
    // The integration loads before the first prompt is used (precmd), so give it one more.
    s.send(b"print -r -- INT_${__teryon_integrated:-no}_ZD_${ZDOTDIR:-unset}_$((6*7))\n");
    s.expect("INT_1_ZD_unset_42");
}

#[test]
fn panes_flush_history_as_they_go() {
    if !have_zsh() {
        return;
    }
    let (home, work) = (tempfile::tempdir().unwrap(), workdir());
    let mut s = injected_shell(
        home.path(),
        work.path(),
        "PROMPT='READY> '\nHISTFILE=$HOME/.zsh_history\nSAVEHIST=100\n",
    );
    s.send(b"print -r -- FLUSH_$((6*7))\n");
    s.expect("FLUSH_42");
    s.send(b"[[ -o inc_append_history ]] && print -r -- INC_$((6*7))\n");
    s.expect("INC_42");
    // The shell has not exited, yet the command is already on disk.
    let text = std::fs::read_to_string(home.path().join(".zsh_history")).unwrap_or_default();
    assert!(text.contains("FLUSH_$((6*7))"), "{text}");
}

#[test]
fn a_zshrc_that_already_loads_the_integration_is_not_loaded_twice() {
    if !have_zsh() {
        return;
    }
    let (home, work) = (tempfile::tempdir().unwrap(), workdir());
    let mut s = injected_shell(
        home.path(),
        work.path(),
        "PROMPT='READY> '\neval \"$(teryon shell-init zsh)\"\n",
    );
    s.send(b"print -r -- N_${#${(M)precmd_functions:#__teryon_precmd}}_$((6*7))\n");
    s.expect("N_1_42");
}

#[test]
fn doctor_shell_reports_a_pane_that_loads_the_integration() {
    if !have_zsh() {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join(".zshrc"),
        "autoload -Uz compinit; compinit -u -d $HOME/.zcompdump\nHISTFILE=$HOME/.zsh_history\nSAVEHIST=100\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_teryon"))
        .env("HOME", home.path())
        .env("SHELL", "/bin/zsh")
        .args(["doctor", "--shell"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    assert!(
        text.contains("[ok  ] integration: loads in a new pane automatically"),
        "{text}"
    );
    assert!(text.contains("[ok  ] Tab: compinit is loaded"), "{text}");
    assert!(text.contains("[ok  ] shell history"), "{text}");

    let off = Command::new(env!("CARGO_BIN_EXE_teryon"))
        .env("HOME", home.path())
        .env("SHELL", "/bin/zsh")
        .env("TERYON_INJECT", "0")
        .args(["doctor", "--shell"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&off.stdout);
    assert!(text.contains("[FAIL] integration"), "{text}");
}

#[test]
fn a_multi_line_paste_waits_for_a_y_and_a_single_line_does_not() {
    if !have_zsh() {
        return;
    }
    let (home, work) = (tempfile::tempdir().unwrap(), workdir());
    let mut s = shell(home.path(), work.path(), false);
    // One command: inserted at once.
    s.send(b"\x1b[200~print -r -- ONE_$((6*7))\x1b[201~");
    s.expect("ONE_$((6*7))");
    pause();
    s.send(b"\x15");
    // Two commands: held back, and "n" drops them.
    s.send(b"\x1b[200~print -r -- A_$((6*7))\nprint -r -- B_$((6*7))\x1b[201~");
    s.expect("Paste contains 2 lines");
    pause();
    s.send(b"n");
    s.expect("paste dropped");
    pause();
    // The same paste with "y" lands on the line, and Enter runs both.
    s.send(b"\x1b[200~print -r -- C_$((6*7))\nprint -r -- D_$((6*7))\x1b[201~");
    s.expect("Paste contains 2 lines");
    pause();
    s.send(b"y");
    pause();
    s.send(b"\n");
    s.expect("C_42");
}

#[test]
fn alt_s_picks_a_snippet_asks_for_its_placeholders_and_types_it() {
    if !have_zsh() {
        return;
    }
    let (home, work) = (tempfile::tempdir().unwrap(), workdir());
    teryon(
        home.path(),
        &[
            "snip",
            "add",
            "deploy",
            "--",
            "print -r -- DEPLOY_{{env}}_$((6*7))",
        ],
    );
    let mut s = shell(home.path(), work.path(), false);
    s.send(b"\x1bs");
    s.expect("pick \u{276f}");
    s.send(b"deplo");
    pause();
    s.send(b"\r");
    s.expect("env: ");
    s.send(b"prod\n");
    s.expect("READY> print -r -- DEPLOY_prod_");
    pause();
    s.send(b"\n");
    s.expect("DEPLOY_prod_42");
}

#[test]
fn a_trusted_project_env_loads_on_cd_and_is_put_back_on_leaving() {
    if !have_zsh() {
        return;
    }
    let (home, work) = (tempfile::tempdir().unwrap(), workdir());
    let proj = work.path().join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(
        proj.join(".teryon.toml"),
        "[env]\nPROJ_VAR = \"hello\"\nKEPT = \"project\"\n",
    )
    .unwrap();
    let mut s = shell_with(home.path(), work.path(), false, b"export KEPT=mine\n");
    // Not trusted yet: told how to trust it, and nothing is set.
    s.send(b"cd proj\n");
    s.expect("run `teryon env allow`");
    pause();
    s.send(b"print -r -- UNTRUSTED_${PROJ_VAR:-unset}\n");
    s.expect("UNTRUSTED_unset");
    s.send(b"teryon env allow\n");
    s.expect("Loaded the next time");
    pause();
    s.send(b"cd .. && cd proj\n");
    s.expect("loaded 2 variable(s)");
    pause();
    s.send(b"print -r -- IN_${PROJ_VAR:-unset}_${KEPT}\n");
    s.expect("IN_hello_project");
    s.send(b"cd ..\n");
    pause();
    s.send(b"print -r -- OUT_${PROJ_VAR:-unset}_${KEPT}\n");
    s.expect("OUT_unset_mine");
}
