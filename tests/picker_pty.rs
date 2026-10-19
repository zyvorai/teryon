//! `teryon pick` driven through a real pseudo-terminal, the way the zsh widgets run it.
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct Session {
    writer: Box<dyn Write + Send>,
    output: mpsc::Receiver<Vec<u8>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    seen: String,
}
impl Session {
    fn start(home: &std::path::Path, args: &[&str]) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 30,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_teryon"));
        cmd.args(args);
        cmd.env("HOME", home);
        cmd.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(cmd).unwrap();
        let mut reader = pair.master.try_clone_reader().unwrap();
        let (tx, output) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Self {
            writer: pair.master.take_writer().unwrap(),
            output,
            child,
            seen: String::new(),
        }
    }
    /// Read until `needle` has appeared in the output (escape codes included), or fail.
    fn expect(&mut self, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.seen.contains(needle) {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.output.recv_timeout(left) {
                Ok(bytes) => self.seen.push_str(&String::from_utf8_lossy(&bytes)),
                Err(_) => panic!("never saw {needle:?}; saw {:?}", self.seen),
            }
        }
    }
    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
    }
    fn exit_code(&mut self) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status.exit_code();
            }
            assert!(Instant::now() < deadline, "picker did not exit");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
fn seeded_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    for (exit, cmd) in [("0", "echo FIRST_CMD"), ("3", "echo SECOND_CMD")] {
        let ok = std::process::Command::new(env!("CARGO_BIN_EXE_teryon"))
            .env("HOME", home.path())
            .args(["hist", "add", "--cwd", "/x", "--exit", exit, "--", cmd])
            .status()
            .unwrap();
        assert!(ok.success());
    }
    home
}

#[test]
fn history_picker_filters_and_prints_the_choice() {
    let home = seeded_home();
    let mut s = Session::start(home.path(), &["pick", "--history", "--cwd", "/x"]);
    s.expect("SECOND_CMD");
    s.expect("✗3");
    s.send(b"FIRST");
    s.send(b"\r");
    s.expect("echo FIRST_CMD");
    assert_eq!(s.exit_code(), 0);
}
#[test]
fn alt_enter_asks_the_shell_to_run_it_and_escape_cancels() {
    let home = seeded_home();
    let mut run = Session::start(home.path(), &["pick", "--history", "--cwd", "/x"]);
    run.expect("FIRST_CMD");
    run.send(b"\x1b\r");
    assert_eq!(run.exit_code(), 10);
    let mut cancel = Session::start(home.path(), &["pick", "--history", "--cwd", "/x"]);
    cancel.expect("FIRST_CMD");
    cancel.send(b"\x1b");
    assert_eq!(cancel.exit_code(), 1);
}
