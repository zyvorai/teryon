//! `teryon search` against a real daemon with two workspaces.
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Daemon {
    child: Child,
    dir: tempfile::TempDir,
}
impl Daemon {
    fn start() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("zs-")
            .tempdir_in("/tmp")
            .unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_teryon"))
            .arg("daemon")
            .env("TERYON_RUNTIME_DIR", dir.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let d = Self { child, dir };
        let end = Instant::now() + Duration::from_secs(5);
        while !d.dir.path().join("endpoint.json").exists() {
            assert!(Instant::now() < end, "daemon startup timeout");
            std::thread::sleep(Duration::from_millis(20));
        }
        d
    }
    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_teryon"))
            .env("TERYON_RUNTIME_DIR", self.dir.path())
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
}
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.run(&["stop", "local", "--force"]);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn search_finds_text_in_pane_scrollback_with_context_across_workspaces() {
    let d = Daemon::start();
    for (ws, script) in [
        (
            "alpha",
            "echo first-line; echo NEEDLE one; echo tail-line; sleep 20",
        ),
        ("beta", "echo another needle two; sleep 20"),
    ] {
        let out = d.run(&["exec", "local", "-w", ws, "--", "sh", "-c", script]);
        assert!(out.status.success(), "{out:?}");
    }
    // Wait until both panes have printed.
    let end = Instant::now() + Duration::from_secs(10);
    let all = loop {
        let out = d.run(&["search", "needle", "--all"]);
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        if (text.contains("alpha/") && text.contains("beta/")) || Instant::now() > end {
            break text;
        }
        std::thread::sleep(Duration::from_millis(150));
    };
    assert!(
        all.contains("NEEDLE one") && all.contains("another needle two"),
        "{all}"
    );

    // Case-sensitive when the pattern has capitals, and only the workspace asked for.
    let one = d.run(&["search", "NEEDLE", "-w", "alpha", "-C", "1"]);
    let text = String::from_utf8_lossy(&one.stdout);
    assert!(one.status.success(), "{one:?}");
    assert!(text.contains(": NEEDLE one"), "{text}");
    assert!(
        text.contains("- first-line") && text.contains("- tail-line"),
        "context lines: {text}"
    );
    assert!(!text.contains("another needle"), "{text}");

    // Nothing found exits 1; a bad regex is an error.
    let none = d.run(&["search", "nomatch-anywhere", "-w", "alpha"]);
    assert_eq!(none.status.code(), Some(1), "{none:?}");
    assert!(none.stdout.is_empty());
    let bad = d.run(&["search", "(", "-E", "-w", "alpha"]);
    assert_eq!(bad.status.code(), Some(2), "{bad:?}");
    assert!(String::from_utf8_lossy(&bad.stderr).contains("not a valid pattern"));
}
