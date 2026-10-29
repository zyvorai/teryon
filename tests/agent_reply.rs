//! The agent reply path end to end: a fake agent asks, the daemon pushes to a fake ntfy with
//! Approve/Deny buttons, the fake ntfy plays a button tap on the reply topic, and the key lands in the pane.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Serves `POST /topic` (records the buttons) and `GET /topic-reply/json` (plays `word:token`).
fn fake_ntfy(word: &'static str, posted: Arc<Mutex<Vec<String>>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let posted = posted.clone();
            thread::spawn(move || {
                let mut reader = BufReader::new(conn.try_clone().unwrap());
                let mut conn = conn;
                let mut request = String::new();
                reader.read_line(&mut request).unwrap_or(0);
                let mut actions = String::new();
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                        break;
                    }
                    let lower = line.to_lowercase();
                    if lower.starts_with("actions:") {
                        actions = line["actions:".len()..].trim().to_string();
                    }
                    if let Some(n) = lower.strip_prefix("content-length:") {
                        length = n.trim().parse().unwrap_or(0);
                    }
                }
                if request.starts_with("POST") {
                    let mut body = vec![0u8; length];
                    let _ = reader.read_exact(&mut body);
                    posted.lock().unwrap().push(actions);
                    let _ = conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                } else if request.contains("topic-reply/json") {
                    let end = Instant::now() + Duration::from_secs(30);
                    let token = loop {
                        let found = posted.lock().unwrap().iter().find_map(|a| {
                            a.split("body=").skip(1).find_map(|r| {
                                r.starts_with(word).then(|| {
                                    r.split(':')
                                        .nth(1)
                                        .unwrap()
                                        .split(',')
                                        .next()
                                        .unwrap()
                                        .to_string()
                                })
                            })
                        });
                        if let Some(t) = found {
                            break t;
                        }
                        if Instant::now() > end {
                            return;
                        }
                        thread::sleep(Duration::from_millis(100));
                    };
                    let json = format!(
                        "{{\"event\":\"message\",\"topic\":\"t\",\"message\":\"{word}:{token}\"}}\n"
                    );
                    let _ = conn.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n");
                    let _ = conn.write_all(json.as_bytes());
                }
            });
        }
    });
    port
}

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
    fn capture(&self) -> String {
        self.cli(&["capture", "local", "-w", "r", "--pane", "1"])
    }
    fn eventually(&self, needle: &str, secs: u64) -> bool {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if self.capture().contains(needle) {
                return true;
            }
            thread::sleep(Duration::from_millis(200));
        }
        false
    }
}

fn run(word: &'static str, expect_byte: &str) {
    let posted = Arc::new(Mutex::new(Vec::new()));
    let port = fake_ntfy(word, posted.clone());
    let dir = tempfile::Builder::new()
        .prefix("rp-")
        .tempdir_in("/tmp")
        .unwrap();
    std::fs::create_dir_all(dir.path().join(".config/teryon")).unwrap();
    std::fs::write(
        dir.path().join(".config/teryon/config.toml"),
        format!("[notify]\nntfy_url = \"http://127.0.0.1:{port}/topic\"\nreply = true\n"),
    )
    .unwrap();
    // Looks like Claude to the daemon (`sh …/claude`): works for a bit, then asks, then reports the key.
    let agent = dir.path().join("claude");
    std::fs::write(
        &agent,
        "stty -icanon -echo min 1\necho busy\nsleep 4\nprintf 'Do you want to proceed?\\n1. Yes\\n2. No\\n'\n\
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
    daemon.cli(&["send", "local", "-w", "r", "--pane", "1", "true", "--enter"]);
    assert!(daemon.eventually("$", 10));
    thread::sleep(Duration::from_secs(2)); // let the watcher see the shell first
    let cmd = format!("sh {}", agent.display());
    daemon.cli(&["send", "local", "-w", "r", "--pane", "1", &cmd, "--enter"]);
    assert!(
        daemon.eventually("Do you want to proceed?", 15),
        "{}",
        daemon.capture()
    );
    assert!(
        daemon.eventually(&format!("GOT[{expect_byte}]"), 40),
        "the {word} tap never reached the agent; screen:\n{}\nposted: {:?}",
        daemon.capture(),
        posted.lock().unwrap()
    );
    let actions = posted.lock().unwrap().first().cloned().unwrap();
    assert!(
        actions.contains("Approve") && actions.contains("Deny"),
        "{actions}"
    );
}

#[test]
fn tapping_approve_presses_enter_in_the_asking_pane() {
    run("approve", r"\n");
}

#[test]
fn tapping_deny_sends_escape() {
    run("deny", r"033");
}
