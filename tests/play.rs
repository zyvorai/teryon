//! `teryon play` against a recording made from a real daemon pane.
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Daemon {
    child: Child,
    dir: tempfile::TempDir,
}
impl Daemon {
    fn start() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("zp-")
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
fn a_recorded_pane_plays_back_and_a_hostile_recording_is_filtered() {
    let d = Daemon::start();
    let out = d.run(&[
        "exec",
        "local",
        "-w",
        "rec",
        "--",
        "sh",
        "-c",
        "sleep 0.5; echo PLAY_ME_42; sleep 20",
    ]);
    assert!(out.status.success(), "{out:?}");
    let dir = tempfile::tempdir().unwrap();
    let cast = dir.path().join("pane.cast");
    // The pane `exec` created is the newest one (pane 1 is the workspace's own shell).
    let listing = d.run(&["panes", "local", "-w", "rec"]);
    let pane = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .filter_map(|l| l.split('\t').next()?.parse::<u64>().ok())
        .max()
        .expect("a pane")
        .to_string();
    let rec = d.run(&[
        "record",
        "local",
        "-w",
        "rec",
        "--pane",
        &pane,
        "-o",
        cast.to_str().unwrap(),
        "--seconds",
        "3",
    ]);
    assert!(rec.status.success(), "{rec:?}");

    let info = d.run(&["play", cast.to_str().unwrap(), "--info"]);
    let text = String::from_utf8_lossy(&info.stdout);
    assert!(info.status.success() && text.contains("frames"), "{info:?}");
    let played = d.run(&[
        "play",
        cast.to_str().unwrap(),
        "--speed",
        "100",
        "--max-idle",
        "0.1",
    ]);
    assert!(
        String::from_utf8_lossy(&played.stdout).contains("PLAY_ME_42"),
        "{played:?}"
    );

    // A file that tries to write the clipboard and ask the terminal questions.
    let evil = dir.path().join("evil.cast");
    std::fs::write(
        &evil,
        "{\"version\":2,\"width\":80,\"height\":24}\n[0.0,\"o\",\"ok\\u001b]52;c;Zm9v\\u0007\\u001b[6n\\u001b[c done\"]\n",
    )
    .unwrap();
    let safe = d.run(&["play", evil.to_str().unwrap()]);
    let bytes = safe.stdout;
    let shown = String::from_utf8_lossy(&bytes);
    assert!(
        shown.starts_with("ok done") && !shown.contains("52;c") && !shown.contains("[6n"),
        "{shown:?}"
    );
    let raw = d.run(&["play", evil.to_str().unwrap(), "--raw"]);
    assert!(String::from_utf8_lossy(&raw.stdout).contains("52;c;Zm9v"));

    let bad = d.run(&["play", dir.path().join("missing.cast").to_str().unwrap()]);
    assert!(!bad.status.success());
    let bad_speed = d.run(&["play", evil.to_str().unwrap(), "--speed", "0"]);
    assert!(!bad_speed.status.success());
}
