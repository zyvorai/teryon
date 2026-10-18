//! Real subprocess + real PTY tests; no mock session engine.
use std::{
    collections::BTreeMap,
    io::BufReader,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use teryon::ipc::{Endpoint, Stream};
use teryon::protocol::*;

struct Daemon {
    child: Child,
    dir: tempfile::TempDir,
}
impl Daemon {
    fn start() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("zl-")
            .tempdir_in("/tmp")
            .unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_teryon"))
            .arg("daemon")
            .env("TERYON_RUNTIME_DIR", dir.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let daemon = Self { child, dir };
        let deadline = Instant::now() + Duration::from_secs(5);
        while !daemon.dir.path().join("endpoint.json").exists() {
            assert!(Instant::now() < deadline, "daemon startup timeout");
            thread::sleep(Duration::from_millis(20));
        }
        daemon
    }
    fn stream(&self) -> Stream {
        let endpoint: Endpoint =
            serde_json::from_slice(&std::fs::read(self.dir.path().join("endpoint.json")).unwrap())
                .unwrap();
        let mut stream = if let Some(addr) = endpoint.address.strip_prefix("tcp:") {
            Stream::Tcp(std::net::TcpStream::connect(addr).unwrap())
        } else {
            Stream::Unix(
                std::os::unix::net::UnixStream::connect(
                    endpoint.address.strip_prefix("unix:").unwrap(),
                )
                .unwrap(),
            )
        };
        write_frame(&mut stream, &endpoint.token).unwrap();
        stream
    }
    fn client(&self) -> Client {
        Client::new(self.stream())
    }
}
impl Drop for Daemon {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let mut c = self.client();
            let _ = c.call(Request::Shutdown { force: true });
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
struct Client {
    reader: BufReader<Stream>,
    writer: Stream,
}
impl Client {
    fn new(stream: Stream) -> Self {
        stream.timeout(Duration::from_secs(5)).unwrap();
        let mut c = Self {
            reader: BufReader::new(stream.try_clone().unwrap()),
            writer: stream,
        };
        assert!(matches!(
            c.call(Request::Hello { version: VERSION }),
            Response::Hello { .. }
        ));
        c
    }
    fn call(&mut self, r: Request) -> Response {
        write_frame(&mut self.writer, &r).unwrap();
        read_frame(&mut self.reader).unwrap().unwrap()
    }
    fn attach(&mut self, name: &str, readonly: bool) -> Response {
        self.call(Request::Attach {
            workspace: name.into(),
            readonly,
            rows: 24,
            cols: 100,
            cwd: Some("/tmp".into()),
        })
    }
    fn snapshot(&mut self) -> Vec<PaneView> {
        let Response::State { panes } = self.call(Request::Poll {
            known: BTreeMap::new(),
        }) else {
            panic!("no state")
        };
        panes
    }
    fn capture(&mut self, pane: u64) -> String {
        let Response::Captured { text, .. } = self.call(Request::Capture { pane }) else {
            panic!("no capture")
        };
        text
    }
    fn until(&mut self, pane: u64, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            if self.capture(pane).contains(needle) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "missing output: {needle}; the pane shows: {:?}",
                self.capture(pane)
            );
            thread::sleep(Duration::from_millis(30));
        }
    }
}

#[test]
fn process_and_shell_survive_disconnect() {
    let d = Daemon::start();
    let mut c = d.client();
    assert!(matches!(
        c.attach("persist", false),
        Response::Attached { .. }
    ));
    let pid = c.snapshot()[0].pid;
    let Response::Created { pane } = c.call(Request::NewPane {
        cwd: None,
        command: vec![
            "/bin/sh".into(),
            "-c".into(),
            "sleep 0.3; printf 'SURVIVED_NETWORK_DROP\\n'; sleep 0.2".into(),
        ],
        rows: 24,
        cols: 100,
    }) else {
        panic!()
    };
    drop(c);
    thread::sleep(Duration::from_millis(650));
    let mut c = d.client();
    assert!(matches!(
        c.attach("persist", false),
        Response::Attached { .. }
    ));
    assert_eq!(c.snapshot()[0].pid, pid);
    c.until(pane, "SURVIVED_NETWORK_DROP");
    assert_eq!(
        c.snapshot().iter().find(|p| p.id == pane).unwrap().exited,
        Some(0)
    );
}
#[test]
fn exclusive_controller_and_readonly_are_enforced_server_side() {
    let d = Daemon::start();
    let mut controller = d.client();
    controller.attach("leases", false);
    let mut other = d.client();
    assert!(matches!(
        other.attach("leases", false),
        Response::Error { .. }
    ));
    assert!(matches!(
        other.attach("leases", true),
        Response::Attached { .. }
    ));
    assert!(matches!(
        other.call(Request::Input {
            pane: 1,
            seq: 1,
            data: b"echo bad\r".to_vec()
        }),
        Response::Error { .. }
    ));
    assert!(matches!(
        other.call(Request::Resize {
            pane: 1,
            rows: 30,
            cols: 90
        }),
        Response::Error { .. }
    ));
    assert!(matches!(
        other.call(Request::Close { pane: 1 }),
        Response::Error { .. }
    ));
    assert!(matches!(
        other.call(Request::Shutdown { force: true }),
        Response::Error { .. }
    ));
    assert!(matches!(
        other.call(Request::Remove {
            workspace: "leases".into(),
            force: true
        }),
        Response::Error { .. }
    ));
    drop(controller);
    thread::sleep(Duration::from_millis(100));
    assert!(matches!(
        other.attach("leases", false),
        Response::Attached { .. }
    ));
}
#[test]
fn input_sequence_deduplicates_and_rejects_gaps() {
    let d = Daemon::start();
    let mut c = d.client();
    c.attach("input", false);
    let file = d.dir.path().join("counter");
    // The output marker is split in the typed text so the shell's echo of the command cannot match it.
    let command = format!("printf x >> {}; printf 'ACC%sED\\n' EPT\r", file.display());
    for _ in 0..2 {
        assert!(matches!(
            c.call(Request::Input {
                pane: 1,
                seq: 1,
                data: command.as_bytes().to_vec()
            }),
            Response::Ack { seq: Some(1) }
        ));
    }
    c.until(1, "ACCEPTED");
    assert_eq!(std::fs::read(&file).unwrap(), b"x");
    assert!(matches!(
        c.call(Request::Input {
            pane: 1,
            seq: 3,
            data: b"echo rejected\r".to_vec()
        }),
        Response::Error { .. }
    ));
    assert!(matches!(
        c.call(Request::Input {
            pane: 1,
            seq: 2,
            data: b"printf 'NEXT_SEQUENCE\\n'\r".to_vec()
        }),
        Response::Ack { .. }
    ));
    c.until(1, "NEXT_SEQUENCE");
}
#[test]
fn resize_unicode_color_and_incremental_snapshot() {
    let d = Daemon::start();
    let mut c = d.client();
    c.attach("terminal", false);
    c.call(Request::Resize {
        pane: 1,
        rows: 31,
        cols: 117,
    });
    let s = c.snapshot();
    assert_eq!((s[0].rows, s[0].cols), (31, 117));
    c.call(Request::Input {
        pane: 1,
        seq: 1,
        data: "printf '\\033[32mλ界\\033[0m\\n'\r".as_bytes().to_vec(),
    });
    c.until(1, "λ界");
    thread::sleep(Duration::from_millis(100));
    let s = c.snapshot();
    let mut parser = vt100::Parser::new(31, 117, 0);
    parser.process(s[0].screen.as_ref().unwrap());
    assert!(parser.screen().contents().contains("λ界"));
    let known = s.iter().map(|p| (p.id.to_string(), p.revision)).collect();
    let Response::State { panes } = c.call(Request::Poll { known }) else {
        panic!()
    };
    assert!(panes[0].screen.is_none());
}
#[test]
fn exited_tasks_report_status_and_scrollback_retains_old_output() {
    let d = Daemon::start();
    let mut c = d.client();
    c.attach("scroll", false);
    let Response::Created { pane } = c.call(Request::NewPane {
        cwd: None,
        command: vec![
            "/bin/sh".into(),
            "-c".into(),
            "i=1; while [ $i -le 150 ]; do echo LINE_$i; i=$((i+1)); done; exit 7".into(),
        ],
        rows: 10,
        cols: 80,
    }) else {
        panic!()
    };
    c.until(pane, "LINE_150");
    let text = c.capture(pane);
    assert!(text.contains("LINE_1\n"));
    assert!(text.contains("LINE_100\n"));
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if c.snapshot().iter().find(|p| p.id == pane).unwrap().exited == Some(7) {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(20));
    }
}
#[test]
fn validation_shutdown_and_workspace_removal() {
    let d = Daemon::start();
    let mut c = d.client();
    assert!(matches!(c.attach("../bad", false), Response::Error { .. }));
    c.attach("safe", false);
    assert!(matches!(
        c.call(Request::Resize {
            pane: 1,
            rows: 65535,
            cols: 65535
        }),
        Response::Error { .. }
    ));
    assert!(matches!(
        c.call(Request::Shutdown { force: false }),
        Response::Error { .. }
    ));
    assert!(matches!(
        c.call(Request::Remove {
            workspace: "safe".into(),
            force: false
        }),
        Response::Error { .. }
    ));
    assert!(matches!(
        c.call(Request::Remove {
            workspace: "safe".into(),
            force: true
        }),
        Response::Ack { .. }
    ));
    let Response::Workspaces { items } = c.call(Request::List) else {
        panic!()
    };
    assert!(items.is_empty());
}

#[test]
fn terminal_cursor_queries_are_answered_without_a_client_terminal() {
    let d = Daemon::start();
    let mut c = d.client();
    c.attach("queries", false);
    let Response::Created{pane}=c.call(Request::NewPane{cwd:None,command:vec!["/bin/sh".into(),"-c".into(),r"stty raw -echo; printf '\033[6n'; dd bs=1 count=6 2>/dev/null | od -An -tx1; stty sane; printf '\nQUERY_DONE\n'".into()],rows:24,cols:80})else{panic!()};
    c.until(pane, "QUERY_DONE");
    // `od` pads with different spacing on macOS and Linux.
    let shown = c
        .capture(pane)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(shown.contains("1b 5b 31 3b 31 52"), "{shown}");
}
#[test]
fn ipc_rejects_a_wrong_local_token() {
    let d = Daemon::start();
    let endpoint: Endpoint =
        serde_json::from_slice(&std::fs::read(d.dir.path().join("endpoint.json")).unwrap())
            .unwrap();
    let mut stream = if let Some(addr) = endpoint.address.strip_prefix("tcp:") {
        Stream::Tcp(std::net::TcpStream::connect(addr).unwrap())
    } else {
        Stream::Unix(
            std::os::unix::net::UnixStream::connect(
                endpoint.address.strip_prefix("unix:").unwrap(),
            )
            .unwrap(),
        )
    };
    stream.timeout(Duration::from_secs(2)).unwrap();
    write_frame(&mut stream, &"wrong-token").unwrap();
    let mut reader = BufReader::new(stream);
    assert!(read_frame::<Response>(&mut reader).unwrap().is_none());
    assert!(matches!(
        d.client().call(Request::Info),
        Response::Info { .. }
    ));
}
#[test]
fn blocked_pane_input_does_not_stall_the_session_service() {
    let d = Daemon::start();
    let mut c = d.client();
    c.attach("backpressure", false);
    let Response::Created { pane } = c.call(Request::NewPane {
        cwd: None,
        command: vec![
            "/bin/sh".into(),
            "-c".into(),
            "stty raw -echo; echo READY; sleep 5".into(),
        ],
        rows: 24,
        cols: 80,
    }) else {
        panic!()
    };
    c.until(pane, "READY");
    let start = Instant::now();
    let mut full = false;
    for seq in 1..=16 {
        if matches!(
            c.call(Request::Input {
                pane,
                seq,
                data: vec![b'x'; 32768]
            }),
            Response::Error { .. }
        ) {
            full = true;
            break;
        }
    }
    assert!(full, "bounded queue should reject excess input");
    assert!(matches!(c.call(Request::Info), Response::Info { .. }));
    assert!(start.elapsed() < Duration::from_secs(3));
}
#[test]
fn explicit_pane_close_terminates_a_job_that_ignores_hangup() {
    let d = Daemon::start();
    let mut c = d.client();
    c.attach("terminate", false);
    let Response::Created { pane } = c.call(Request::NewPane {
        cwd: None,
        command: vec![
            "/bin/sh".into(),
            "-c".into(),
            "trap '' HUP; echo READY; while :; do sleep 10; done".into(),
        ],
        rows: 24,
        cols: 80,
    }) else {
        panic!()
    };
    c.until(pane, "READY");
    let pid = c
        .snapshot()
        .into_iter()
        .find(|p| p.id == pane)
        .unwrap()
        .pid
        .unwrap();
    assert!(matches!(
        c.call(Request::Close { pane }),
        Response::Ack { .. }
    ));
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let alive = unsafe { libc::kill(pid as i32, 0) } == 0;
        if !alive {
            break;
        }
        assert!(Instant::now() < deadline, "terminated process still alive");
        thread::sleep(Duration::from_millis(20));
    }
}

/// Shell integration through a real PTY: OSC 7 sets the live cwd, OSC 133 records prompt/command/done
/// marks with exit status and duration, all at absolute line numbers.
#[test]
fn shell_integration_marks_cwd_and_exact_line_numbers_reach_clients() {
    let d = Daemon::start();
    let mut c = d.client();
    c.attach("marks", false);
    let script = "printf '\\033]7;file://h/tmp/x%%20y\\007\\033]133;A\\007'; echo prompt; printf '\\033]133;C\\007'; sleep 0.3; echo out; printf '\\033]133;D;3\\007'; sleep 6";
    let Response::Created { pane } = c.call(Request::NewPane {
        cwd: None,
        command: vec!["/bin/sh".into(), "-c".into(), script.into()],
        rows: 24,
        cols: 80,
    }) else {
        panic!("no pane")
    };
    let deadline = Instant::now() + Duration::from_secs(6);
    let view = loop {
        let v = c.snapshot().into_iter().find(|p| p.id == pane).unwrap();
        if v.marks
            .as_ref()
            .is_some_and(|m| m.iter().any(|m| m.kind == 'D'))
        {
            break v;
        }
        assert!(Instant::now() < deadline, "marks never arrived");
        thread::sleep(Duration::from_millis(30));
    };
    assert_eq!(view.cwd, "/tmp/x y");
    let marks = view.marks.unwrap();
    let kinds: String = marks.iter().map(|m| m.kind).collect();
    assert_eq!(kinds, "ACD");
    let done = marks.last().unwrap();
    assert_eq!(done.exit, Some(3));
    assert!(
        done.ms.is_some_and(|ms| (250..5000).contains(&ms)),
        "duration {:?}",
        done.ms
    );
    // `prompt` is printed on the line after the A mark; output starts at C.
    assert_eq!(marks[0].line, 0);
    assert_eq!(marks[1].line, 1);
    assert_eq!(done.line, 2);
}
#[test]
fn captured_text_has_one_line_per_row_with_exact_absolute_numbers() {
    let d = Daemon::start();
    let mut c = d.client();
    c.attach("lines", false);
    let Response::Created { pane } = c.call(Request::NewPane {
        cwd: None,
        command: vec![
            "/bin/sh".into(),
            "-c".into(),
            "i=1; while [ $i -le 2600 ]; do echo l$i; i=$((i+1)); done; sleep 6".into(),
        ],
        rows: 24,
        cols: 80,
    }) else {
        panic!("no pane")
    };
    c.until(pane, "l2600");
    let Response::Captured { text, first_line } = c.call(Request::Capture { pane }) else {
        panic!("no capture")
    };
    assert!(
        first_line > 0,
        "2600 lines exceed the 2000-line history, so the oldest were dropped"
    );
    for (i, line) in text.lines().enumerate() {
        assert_eq!(line, format!("l{}", first_line + i as u64 + 1), "line {i}");
    }
    let view = c.snapshot().into_iter().find(|p| p.id == pane).unwrap();
    assert!(view.top_line >= first_line && view.first_line == first_line);
}

/// The generated integration script, loaded by real interactive shells, produces marks and the cwd.
#[test]
fn generated_shell_integration_works_in_real_interactive_shells() {
    let bin = env!("CARGO_BIN_EXE_teryon");
    for (shell, args) in [
        ("bash", &["--noprofile", "--norc", "-i"][..]),
        ("zsh", &["-f", "-i"][..]),
        ("fish", &["--no-config", "-i"][..]),
    ] {
        let Ok(path) = Command::new("which").arg(shell).output() else {
            continue;
        };
        let path = String::from_utf8_lossy(&path.stdout).trim().to_string();
        if path.is_empty() {
            continue;
        }
        let d = Daemon::start();
        let mut c = d.client();
        c.attach("shell", false);
        let mut command = vec![path];
        command.extend(args.iter().map(|s| s.to_string()));
        let Response::Created { pane } = c.call(Request::NewPane {
            cwd: Some("/tmp".into()),
            command,
            rows: 24,
            cols: 100,
        }) else {
            panic!("{shell}: no pane")
        };
        let mut seq = 0;
        let mut send = |c: &mut Client, text: String| {
            seq += 1;
            c.call(Request::Input {
                pane,
                seq,
                data: text.into_bytes(),
            });
        };
        thread::sleep(Duration::from_millis(600));
        let load = if shell == "fish" {
            format!("{bin} shell-init fish | source\r")
        } else {
            format!("eval \"$({bin} shell-init {shell})\"\r")
        };
        send(&mut c, load);
        thread::sleep(Duration::from_millis(500));
        send(&mut c, "false\r".into());
        thread::sleep(Duration::from_millis(400));
        send(&mut c, "cd /usr && true\r".into());
        let deadline = Instant::now() + Duration::from_secs(6);
        let view = loop {
            let v = c.snapshot().into_iter().find(|p| p.id == pane).unwrap();
            let done: Vec<Option<i32>> = v
                .marks
                .iter()
                .flatten()
                .filter(|m| m.kind == 'D')
                .map(|m| m.exit)
                .collect();
            if done.contains(&Some(1)) && done.contains(&Some(0)) && v.cwd == "/usr" {
                break v;
            }
            assert!(
                Instant::now() < deadline,
                "{shell}: marks/cwd missing; saw {done:?} cwd {} screen {}",
                v.cwd,
                c.capture(pane)
            );
            thread::sleep(Duration::from_millis(50));
        };
        let kinds: String = view.marks.unwrap().iter().map(|m| m.kind).collect();
        assert!(kinds.contains("ACD"), "{shell}: {kinds}");
    }
}

#[test]
fn a_fake_agent_is_reported_as_the_foreground_command_and_its_prompt_is_readable() {
    use std::os::unix::fs::PermissionsExt;
    let d = Daemon::start();
    let mut c = d.client();
    c.attach("agent", false);
    let bin = tempfile::Builder::new()
        .prefix("zl-agent-")
        .tempdir_in("/tmp")
        .unwrap();
    let script = bin.path().join("claude");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf 'Edit src/main.rs\\nDo you want to proceed? (y/n)\\n'\nsleep 8\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let Response::Created { pane } = c.call(Request::NewPane {
        cwd: None,
        command: vec![script.display().to_string()],
        rows: 24,
        cols: 80,
    }) else {
        panic!("no pane")
    };
    c.until(pane, "Do you want to proceed");
    let deadline = Instant::now() + Duration::from_secs(6);
    let fg = loop {
        let v = c.snapshot().into_iter().find(|p| p.id == pane).unwrap();
        if let Some(fg) = v.fg {
            break fg;
        }
        assert!(Instant::now() < deadline, "no foreground command reported");
        thread::sleep(Duration::from_millis(50));
    };
    assert!(fg.contains("claude"), "{fg}");
    assert_eq!(
        teryon::agents::detect(&fg),
        Some(teryon::agents::Kind::Claude)
    );
    let rows: Vec<String> = c.capture(pane).lines().map(str::to_string).collect();
    assert_eq!(
        teryon::agents::status(&rows, Duration::from_secs(30)),
        teryon::agents::Status::NeedsYou
    );
}
