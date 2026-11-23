//! `respond` and the view-only share relay, end to end against a real daemon: a fake agent asks, the
//! answer lands (and only while it asks), and a view-only key can watch but never type or reach
//! other workspaces.
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

struct Daemon {
    child: Child,
    dir: tempfile::TempDir,
}
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Daemon {
    fn start() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("rs-")
            .tempdir_in("/tmp")
            .unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_teryon"))
            .arg("daemon")
            .env("SHELL", "/bin/sh")
            .env("PS1", "$ ")
            .env("HOME", dir.path())
            .env_remove("ENV")
            .env("TERYON_RUNTIME_DIR", dir.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let end = Instant::now() + Duration::from_secs(5);
        while !Path::new(&dir.path().join("endpoint.json")).exists() {
            assert!(Instant::now() < end, "daemon startup timeout");
            thread::sleep(Duration::from_millis(20));
        }
        Self { child, dir }
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_teryon"));
        c.env("HOME", self.dir.path())
            .env("TERYON_RUNTIME_DIR", self.dir.path())
            .env("SHELL", "/bin/sh")
            .args(args);
        c
    }
    /// stdout and exit code.
    fn cli(&self, args: &[&str]) -> (String, i32) {
        let out = self.command(args).output().unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }
    fn capture(&self, pane: &str) -> String {
        self.cli(&["capture", "local", "-w", "r", "--pane", pane]).0
    }
    fn eventually(&self, pane: &str, needle: &str, secs: u64) -> bool {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if self.capture(pane).contains(needle) {
                return true;
            }
            thread::sleep(Duration::from_millis(150));
        }
        false
    }
    /// A fake agent that asks, records the one byte it is given, shows a busy screen for a while, and asks again.
    fn start_agent(&self) -> std::path::PathBuf {
        let keys = self.dir.path().join("keys.bin");
        let script = self.dir.path().join("claude");
        std::fs::write(
            &script,
            format!(
                "stty -icanon -echo -icrnl min 1\n\
                 while true; do\n\
                 printf '\\033[2J\\033[HDo you want to proceed?\\n1. Yes\\n2. No\\n'\n\
                 k=$(dd bs=1 count=1 2>/dev/null)\n\
                 printf '%s' \"$k\" >> {keys}\n\
                 printf '\\033[2J\\033[Hworking... esc to interrupt\\n'\n\
                 sleep 3\n\
                 done\n",
                keys = keys.display()
            ),
        )
        .unwrap();
        let (out, code) = self.cli(&[
            "exec",
            "local",
            "-w",
            "r",
            "--cwd",
            self.dir.path().to_str().unwrap(),
            "--",
            "sh",
            script.to_str().unwrap(),
        ]);
        assert_eq!(code, 0, "{out}");
        keys
    }
}

fn received(keys: &Path) -> Vec<u8> {
    std::fs::read(keys).unwrap_or_default()
}

#[test]
fn respond_types_the_agents_key_only_while_it_asks() {
    let d = Daemon::start();
    let keys = d.start_agent();
    assert!(
        d.eventually("2", "Do you want to proceed?", 15),
        "{}",
        d.capture("2")
    );
    // Give the daemon's screen reader a moment to see the prompt as a prompt.
    thread::sleep(Duration::from_secs(1));

    let (out, code) = d.cli(&["respond", "local", "-w", "r", "--pane", "2"]);
    assert_eq!((out.trim(), code), ("Approved", 0), "{out}");
    let end = Instant::now() + Duration::from_secs(5);
    while received(&keys).is_empty() && Instant::now() < end {
        thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(received(&keys), b"\r", "approve types Enter");

    // The agent is busy now: a second answer must be refused and type nothing.
    assert!(d.eventually("2", "esc to interrupt", 5));
    let (out, code) = d.cli(&["respond", "local", "-w", "r", "--pane", "2", "--deny"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("Not sent"), "{out}");
    assert_eq!(received(&keys), b"\r", "nothing was typed while it worked");

    // It asks again; now deny types Esc.
    assert!(d.eventually("2", "Do you want to proceed?", 10));
    thread::sleep(Duration::from_secs(1));
    let (out, code) = d.cli(&["respond", "local", "-w", "r", "--pane", "2", "--deny"]);
    assert_eq!((out.trim(), code), ("Denied", 0), "{out}");
    let end = Instant::now() + Duration::from_secs(5);
    while received(&keys).len() < 2 && Instant::now() < end {
        thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(received(&keys), b"\r\x1b", "deny types Esc");

    // A plain shell pane and a missing pane are refused too.
    assert_eq!(d.cli(&["respond", "local", "-w", "r", "--pane", "1"]).1, 1);
    assert_eq!(d.cli(&["respond", "local", "-w", "r", "--pane", "9"]).1, 1);
}

/// One request and its one response over a `relay --view` process.
struct View {
    child: Child,
    out: BufReader<std::process::ChildStdout>,
}
impl View {
    fn start(d: &Daemon, workspace: &str) -> Self {
        let mut child = d
            .command(&["relay", "--view", workspace])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        Self { child, out }
    }
    fn call(&mut self, request: serde_json::Value) -> serde_json::Value {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        self.out.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|_| panic!("not JSON: {line:?}"))
    }
}
impl Drop for View {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn a_view_key_can_watch_its_workspace_and_do_nothing_else() {
    use serde_json::json;
    let d = Daemon::start();
    let keys = d.start_agent();
    assert!(d.eventually("2", "Do you want to proceed?", 15));
    let mut v = View::start(&d, "r");
    let kind = |r: &serde_json::Value| r["type"].as_str().unwrap_or("").to_string();

    assert_eq!(kind(&v.call(json!({"op": "hello", "version": 1}))), "hello");
    // It cannot list workspaces or watch another one.
    assert_eq!(kind(&v.call(json!({"op": "list"}))), "error");
    let other = v.call(
        json!({"op": "attach", "workspace": "nope", "readonly": false, "rows": 24, "cols": 80}),
    );
    assert_eq!(kind(&other), "error");
    // Asking for control and a takeover still gets a read-only attachment.
    let attached = v.call(json!({"op": "attach", "workspace": "r", "readonly": false,
        "rows": 24, "cols": 80, "takeover": true}));
    assert_eq!(kind(&attached), "attached");
    assert_eq!(attached["readonly"], true);
    // It can look.
    let state = v.call(json!({"op": "poll", "known": {}, "diff": false, "wait_ms": 0}));
    assert_eq!(kind(&state), "state");
    assert!(state["panes"].as_array().is_some_and(|p| p.len() >= 2));
    // It can neither type, answer a prompt, close a pane nor stop the daemon.
    for req in [
        json!({"op": "input", "pane": 2, "seq": 1, "data": [121]}),
        json!({"op": "respond", "workspace": "r", "pane": 2, "approve": true}),
        json!({"op": "close", "pane": 2}),
        json!({"op": "shutdown", "force": true}),
    ] {
        assert_eq!(kind(&v.call(req.clone())), "error", "{req}");
    }
    thread::sleep(Duration::from_millis(500));
    assert!(received(&keys).is_empty(), "nothing reached the agent");
    // The daemon is still up and the workspace untouched.
    assert!(d.capture("2").contains("Do you want to proceed?"));
}

#[test]
fn share_prints_a_line_that_shares_finds_again() {
    let dir = tempfile::tempdir().unwrap();
    let bin = env!("CARGO_BIN_EXE_teryon");
    let key = dir.path().join("bob.pub");
    std::fs::write(&key, "ssh-ed25519 AAAAC3Nza bob@laptop\n").unwrap();
    let out = Command::new(bin)
        .args(["share", "build", "--pubkey", key.to_str().unwrap()])
        .env("HOME", dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text
        .lines()
        .find(|l| l.starts_with("command="))
        .expect("an authorized_keys line");
    assert!(line.contains("relay --view build\",restrict ssh-ed25519 AAAAC3Nza bob@laptop"));

    let keys = dir.path().join("authorized_keys");
    std::fs::write(&keys, format!("ssh-rsa AAAAB3 me@mini\n{line}\n")).unwrap();
    let out = Command::new(bin)
        .args(["shares", "--file", keys.to_str().unwrap()])
        .env("HOME", dir.path())
        .output()
        .unwrap();
    let listed = String::from_utf8_lossy(&out.stdout);
    assert!(
        listed.contains("line 2")
            && listed.contains("watches build")
            && listed.contains("bob@laptop"),
        "{listed}"
    );
    // Not a key, and not a workspace name that could break out of the quotes.
    std::fs::write(&key, "nonsense\n").unwrap();
    assert!(!Command::new(bin)
        .args(["share", "build", "--pubkey", key.to_str().unwrap()])
        .env("HOME", dir.path())
        .status()
        .unwrap()
        .success());
    assert!(!Command::new(bin)
        .args(["share", "a\"b"])
        .env("HOME", dir.path())
        .status()
        .unwrap()
        .success());
}
