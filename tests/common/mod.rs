//! Shared by the pseudo-terminal tests.
#![allow(dead_code)]
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Output with terminal escape sequences and backspaces removed.
pub fn clean(text: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b\][^\x07]*\x07|\x08").unwrap();
    re.replace_all(text, "").into_owned()
}
pub struct Session {
    writer: Box<dyn Write + Send>,
    output: mpsc::Receiver<Vec<u8>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    seen: String,
}
impl Session {
    pub fn start(home: &std::path::Path, args: &[&str]) -> Self {
        Self::start_program(env!("CARGO_BIN_EXE_teryon"), home, args, None)
    }
    pub fn start_program(
        program: &str,
        home: &std::path::Path,
        args: &[&str],
        dir: Option<&std::path::Path>,
    ) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 30,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut cmd = CommandBuilder::new(program);
        if let Some(dir) = dir {
            cmd.cwd(dir);
        }
        let path = format!(
            "{}:{}",
            std::path::Path::new(env!("CARGO_BIN_EXE_teryon"))
                .parent()
                .unwrap()
                .display(),
            std::env::var("PATH").unwrap_or_default()
        );
        cmd.env("PATH", path);
        cmd.env("PS1", "% ");
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
    pub fn expect(&mut self, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.seen.contains(needle) {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.output.recv_timeout(left) {
                Ok(bytes) => self.seen.push_str(&clean(&String::from_utf8_lossy(&bytes))),
                Err(_) => panic!("never saw {needle:?}; saw {:?}", self.seen),
            }
        }
    }
    pub fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
    }
    pub fn exit_code(&mut self) -> u32 {
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
