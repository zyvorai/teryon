// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! `teryon wait` and `teryon capture --follow|--tail` against a real daemon.
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Daemon {
    child: Child,
    dir: tempfile::TempDir,
}
impl Daemon {
    fn start() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("zw-")
            .tempdir_in("/tmp")
            .unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_teryon"))
            .arg("daemon")
            .env("TERYON_RUNTIME_DIR", dir.path())
            .env("HOME", dir.path())
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
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_teryon"))
            .env("TERYON_RUNTIME_DIR", self.dir.path())
            .env("HOME", self.dir.path())
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
    /// Start `sh -c script` in a new pane of workspace `w` and return the pane's number.
    fn task(&self, script: &str) -> String {
        let out = self.run(&["exec", "local", "-w", "w", "--", "sh", "-c", script]);
        assert!(out.status.success(), "{out:?}");
        String::from_utf8_lossy(&out.stderr)
            .split_whitespace()
            .last()
            .unwrap()
            .to_string()
    }
    fn wait(&self, pane: &str, extra: &[&str]) -> (Option<i32>, String, String) {
        let mut args = vec!["wait", "local", "-w", "w", "--pane", pane];
        args.extend_from_slice(extra);
        let o = self.run(&args);
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
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
fn wait_returns_for_a_match_quiet_or_exit_and_tells_them_apart() {
    let d = Daemon::start();
    let p = d.task("sleep 0.5; echo READY_$((40+2)); sleep 30");
    let (code, out, err) = d.wait(&p, &["--for", "READY_[0-9]+", "--timeout", "20"]);
    assert_eq!((code, out.as_str()), (Some(0), "READY_42\n"), "{err}");
    // Already on the screen: found at once, unless only new output counts.
    let (code, ..) = d.wait(&p, &["--for", "READY_42", "--timeout", "0"]);
    assert_eq!(code, Some(0));
    let (code, out, err) = d.wait(&p, &["--for", "READY_42", "--new", "--timeout", "1"]);
    assert_eq!((code, out.as_str()), (Some(1), ""), "{err}");
    assert!(err.contains("timed out"), "{err}");
    let (code, out, err) = d.wait(&p, &["--idle", "1", "--timeout", "20"]);
    assert_eq!((code, out.as_str()), (Some(0), "idle\n"), "{err}");

    let p = d.task("sleep 0.5; echo last words; exit 7");
    let (code, out, err) = d.wait(&p, &["--for", "never printed", "--timeout", "20"]);
    assert_eq!((code, out.as_str()), (Some(3), "exited 7\n"), "{err}");
    let (code, out, _) = d.wait(&p, &["--exit", "--timeout", "20"]);
    assert_eq!((code, out.as_str()), (Some(0), "exited 7\n"));
    // Output printed just before the exit is still matched.
    let p = d.task("sleep 0.5; echo parting-shot");
    let (code, out, _) = d.wait(&p, &["--for", "parting", "--timeout", "20"]);
    assert_eq!((code, out.as_str()), (Some(0), "parting-shot\n"));
}

#[test]
fn wait_rejects_bad_requests_with_status_2() {
    let d = Daemon::start();
    let p = d.task("sleep 30");
    for extra in [
        &[][..],
        &["--for", "("][..],
        &["--idle", "0"][..],
        &["--new", "--exit"][..],
    ] {
        let (code, _, err) = d.wait(&p, extra);
        assert_eq!(code, Some(2), "{extra:?}: {err}");
    }
    let (code, _, err) = d.wait("999", &["--exit"]);
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("pane 999 not found"), "{err}");
}

#[test]
fn capture_follow_streams_each_line_once_until_the_program_ends() {
    let d = Daemon::start();
    let p = d.task("echo one; sleep 1.5; echo two; sleep 0.5; echo three; sleep 0.5");
    let o = d.run(&["capture", "local", "-w", "w", "--pane", &p, "--follow"]);
    assert!(o.status.success(), "{o:?}");
    assert_eq!(String::from_utf8_lossy(&o.stdout), "one\ntwo\nthree\n");

    let tail = d.run(&["capture", "local", "-w", "w", "--pane", &p, "--tail", "2"]);
    assert_eq!(String::from_utf8_lossy(&tail.stdout), "two\nthree\n");
    // Following a finished pane prints what --tail asks for and returns.
    let o = d.run(&[
        "capture", "local", "-w", "w", "--pane", &p, "-f", "-n", "1", "--plain",
    ]);
    assert!(o.status.success(), "{o:?}");
    assert_eq!(String::from_utf8_lossy(&o.stdout), "three\n");
}
