//! The native banner path end to end, with a stub standing in for TeryonNotifier.app: the daemon
//! hands the notifier a payload when an agent asks, and the token in it answers that agent.
#![cfg(target_os = "macos")]
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
    fn cli(&self, args: &[&str]) -> String {
        let out = Command::new(env!("CARGO_BIN_EXE_teryon"))
            .env("HOME", self.dir.path())
            .env("TERYON_RUNTIME_DIR", self.dir.path())
            .env("SHELL", "/bin/sh")
            .args(args)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
    fn eventually(&self, needle: &str, secs: u64) -> bool {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if self
                .cli(&["capture", "local", "-w", "n", "--pane", "1"])
                .contains(needle)
            {
                return true;
            }
            thread::sleep(Duration::from_millis(200));
        }
        false
    }
}

#[test]
fn the_banner_payload_carries_a_token_that_answers_the_agent() {
    let dir = tempfile::Builder::new()
        .prefix("nn-")
        .tempdir_in("/tmp")
        .unwrap();
    std::fs::create_dir_all(dir.path().join(".config/teryon")).unwrap();
    std::fs::write(
        dir.path().join(".config/teryon/config.toml"),
        "[notify]\nmacos = true\n",
    )
    .unwrap();
    let seen = dir.path().join("payload.json");
    let stub = dir.path().join("notifier");
    std::fs::write(
        &stub,
        format!("#!/bin/sh\ncat > {}\nexit 0\n", seen.display()),
    )
    .unwrap();
    Command::new("chmod").arg("+x").arg(&stub).status().unwrap();
    let agent = dir.path().join("claude");
    std::fs::write(
        &agent,
        "stty -icanon -echo -icrnl min 1\necho busy\nsleep 4\n\
         printf 'Do you want to proceed?\\n1. Yes\\n2. No\\n'\n\
         k=$(dd bs=1 count=1 2>/dev/null | od -An -c | tr -d ' ')\nprintf 'GOT[%s]\\n' \"$k\"\n",
    )
    .unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_teryon"))
        .arg("daemon")
        .env("SHELL", "/bin/sh")
        .env("PS1", "$ ")
        .env("HOME", dir.path())
        .env_remove("ENV")
        .env("TERYON_RUNTIME_DIR", dir.path())
        .env("TERYON_NOTIFIER", &stub)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let daemon = Daemon { child, dir };
    let end = Instant::now() + Duration::from_secs(5);
    while !Path::new(&daemon.dir.path().join("endpoint.json")).exists() {
        assert!(Instant::now() < end, "daemon startup timeout");
        thread::sleep(Duration::from_millis(20));
    }
    daemon.cli(&["send", "local", "-w", "n", "--pane", "1", "true", "--enter"]);
    assert!(daemon.eventually("$", 10));
    thread::sleep(Duration::from_secs(2));
    let cmd = format!("sh {}", agent.display());
    daemon.cli(&["send", "local", "-w", "n", "--pane", "1", &cmd, "--enter"]);
    assert!(daemon.eventually("Do you want to proceed?", 15));

    let end = Instant::now() + Duration::from_secs(15);
    let payload = loop {
        if let Ok(s) = std::fs::read_to_string(&seen) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                break v;
            }
        }
        assert!(Instant::now() < end, "the notifier was never called");
        thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(payload["workspace"], "n");
    assert_eq!(payload["pane"], 1);
    assert_eq!(payload["level"], "time-sensitive");
    assert_eq!(
        payload["runtime"].as_str().unwrap(),
        daemon.dir.path().to_str().unwrap()
    );
    let token = payload["token"]
        .as_str()
        .expect("a reply token")
        .to_string();

    let out = daemon.cli(&["notify", "answer", &format!("approve:{token}")]);
    assert!(out.starts_with("sent"), "{out}");
    assert!(
        daemon.eventually(r"GOT[\r]", 20),
        "approve never reached the agent"
    );
    // A token works once.
    let again = daemon.cli(&["notify", "answer", &format!("approve:{token}")]);
    assert!(again.starts_with("expired"), "{again}");
}
