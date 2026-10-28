//! `teryon env`: per-project environment variables from `.teryon.toml`.
use std::process::Command;

fn teryon(home: &std::path::Path, dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_teryon"))
        .env("HOME", home)
        .current_dir(dir)
        .args(["env"])
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn allow_status_and_deny_follow_the_table_and_an_edit_revokes_trust() {
    let (home, proj) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let toml = proj.path().join(".teryon.toml");
    std::fs::write(
        &toml,
        "[env]\nAPI_URL = \"http://localhost:3000\"\npath_prepend = [\"bin\"]\n",
    )
    .unwrap();
    let status = |h: &std::path::Path| {
        String::from_utf8_lossy(&teryon(h, proj.path(), &["status"]).stdout).into_owned()
    };
    let text = status(home.path());
    assert!(
        text.contains("not trusted") && text.contains("API_URL=http://localhost:3000"),
        "{text}"
    );

    let allow = teryon(home.path(), proj.path(), &["allow"]);
    assert!(allow.status.success(), "{allow:?}");
    assert!(String::from_utf8_lossy(&allow.stdout).contains("API_URL=http://localhost:3000"));
    assert!(status(home.path()).contains(": trusted"));

    // Editing the table asks again.
    std::fs::write(&toml, "[env]\nAPI_URL = \"http://evil.example\"\n").unwrap();
    assert!(status(home.path()).contains("not trusted"));

    std::fs::write(
        &toml,
        "[env]\nAPI_URL = \"http://localhost:3000\"\npath_prepend = [\"bin\"]\n",
    )
    .unwrap();
    assert!(status(home.path()).contains(": trusted"));
    let deny = teryon(home.path(), proj.path(), &["deny"]);
    assert!(String::from_utf8_lossy(&deny.stdout).contains("No longer trusting"));
    assert!(status(home.path()).contains("not trusted"));

    let none = tempfile::tempdir().unwrap();
    let out = teryon(home.path(), none.path(), &["allow"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no .teryon.toml with an [env] table"));
}

#[test]
fn a_blocked_variable_is_reported_instead_of_loaded() {
    let (home, proj) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::write(
        proj.path().join(".teryon.toml"),
        "[env]\nPATH = \"/evil\"\n",
    )
    .unwrap();
    let out = teryon(home.path(), proj.path(), &["allow"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("use path_prepend"),
        "{out:?}"
    );
}
