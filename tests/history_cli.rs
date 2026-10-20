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

#[test]
fn the_git_branch_is_recorded_with_each_command() {
    let home = tempfile::tempdir().unwrap();
    let repo = home.path().join("proj");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/topic\n").unwrap();
    teryon(
        home.path(),
        &["hist", "add", "--cwd", repo.to_str().unwrap(), "--", "make"],
    );
    teryon(
        home.path(),
        &["hist", "add", "--cwd", "/nowhere", "--", "ls"],
    );
    let text = std::fs::read_to_string(home.path().join(".teryon/history.jsonl")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].contains("\"branch\":\"topic\""), "{}", lines[0]);
    assert!(!lines[1].contains("branch"), "{}", lines[1]);
}

#[test]
fn pulling_a_hosts_history_keeps_it_apart_and_leaves_secrets_behind() {
    use std::os::unix::fs::PermissionsExt;
    let local = tempfile::tempdir().unwrap();
    let remote = tempfile::tempdir().unwrap();
    // The "remote" machine: its own HOME with teryon installed where the SSH command looks for it.
    let bin_dir = remote.path().join(".local/bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_teryon"), bin_dir.join("teryon")).unwrap();
    teryon(
        remote.path(),
        &[
            "hist",
            "add",
            "--cwd",
            "/srv/app",
            "--",
            "systemctl restart api",
        ],
    );
    // A secret that reached the remote file some other way must not travel.
    let file = remote.path().join(".teryon/history.jsonl");
    let mut text = std::fs::read_to_string(&file).unwrap();
    text.push_str(
        "{\"cmd\":\"deploy --token=abc123def456\",\"cwd\":\"/srv\",\"exit\":0,\"ms\":1,\"at\":1}\n",
    );
    std::fs::write(&file, text).unwrap();
    // A stand-in for ssh that runs the command it is given with the remote HOME.
    let fake = tempfile::tempdir().unwrap();
    let ssh = fake.path().join("ssh");
    std::fs::write(
        &ssh,
        "#!/bin/sh\nfor last; do :; done\nHOME=\"$REMOTE_HOME\" exec sh -c \"$last\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
    teryon(
        local.path(),
        &["hist", "add", "--cwd", "/a", "--", "cargo build"],
    );
    let path = format!(
        "{}:{}",
        fake.path().display(),
        std::env::var("PATH").unwrap()
    );
    let out = Command::new(env!("CARGO_BIN_EXE_teryon"))
        .env("HOME", local.path())
        .env("PATH", path)
        .env("REMOTE_HOME", remote.path())
        .args(["hist", "pull", "buildbox"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("Pulled 1 command from buildbox"));
    let kept =
        std::fs::read_to_string(local.path().join(".teryon/history-hosts/buildbox.jsonl")).unwrap();
    assert!(kept.contains("systemctl restart api") && kept.contains("\"host\":\"buildbox\""));
    assert!(!kept.contains("abc123def456"), "{kept}");
    assert_eq!(
        std::fs::metadata(local.path().join(".teryon/history-hosts/buildbox.jsonl"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    // The local history is untouched and the ghost suggestion never offers the other machine's command.
    let own = std::fs::read_to_string(local.path().join(".teryon/history.jsonl")).unwrap();
    assert!(!own.contains("systemctl"));
    assert_eq!(
        teryon(
            local.path(),
            &["hist", "suggest", "--cwd", "/srv/app", "--", "systemctl"]
        ),
        ""
    );
}
