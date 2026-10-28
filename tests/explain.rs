//! `teryon explain` with a stand-in `man`, so it works wherever manual pages are not installed.
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn explain_describes_commands_flags_and_pipes_from_man_and_never_runs_the_command() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let marker = dir.path().join("RAN");
    std::fs::write(
        bin.join("man"),
        "#!/bin/sh\n[ \"$MANPAGER\" = cat ] || exit 3\n\
         [ \"$1\" = frob ] || exit 1\n\
         printf 'FROB(1)\\n\\nNAME\\n       frob - twist things\\n\\nOPTIONS\\n       -q, --quiet\\n              Say nothing.\\n'\n",
    )
    .unwrap();
    std::fs::set_permissions(bin.join("man"), std::fs::Permissions::from_mode(0o755)).unwrap();
    // A command that would leave evidence if it were ever executed.
    std::fs::write(
        bin.join("frob"),
        format!("#!/bin/sh\ntouch {}\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(bin.join("frob"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());

    let out = Command::new(env!("CARGO_BIN_EXE_teryon"))
        .env("PATH", path)
        .args(["explain", "--", "frob -q file | sort"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{out:?}");
    assert!(
        text.contains("frob") && text.contains("twist things"),
        "{text}"
    );
    assert!(
        text.contains("-q") && text.contains("Say nothing."),
        "{text}"
    );
    assert!(text.contains("pipe"), "{text}");
    assert!(
        text.contains("sort") && text.contains("(no manual page found)"),
        "{text}"
    );
    assert!(!marker.exists(), "explain must not run the command");
}
