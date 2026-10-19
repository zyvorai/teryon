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
        Self::start_with(&[])
    }
    fn start_with(env: &[(&str, &str)]) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("zl-")
            .tempdir_in("/tmp")
            .unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_teryon"))
            .arg("daemon")
            .envs(env.iter().copied())
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
            takeover: false,
        })
    }
    fn take_over(&mut self, name: &str) -> Response {
        self.call(Request::Attach {
            workspace: name.into(),
            readonly: false,
            rows: 24,
            cols: 100,
            cwd: Some("/tmp".into()),
            takeover: true,
        })
    }
    fn snapshot(&mut self) -> Vec<PaneView> {
        let Response::State { panes } = self.call(Request::Poll {
            known: BTreeMap::new(),
            diff: false,
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
    let Response::State { panes } = c.call(Request::Poll { known, diff: false }) else {
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

struct Mcp {
    child: Child,
    out: std::io::BufReader<std::process::ChildStdout>,
    next: u64,
}
impl Mcp {
    fn start(d: &Daemon, extra: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_teryon"))
            .arg("mcp")
            .args(extra)
            .env("TERYON_RUNTIME_DIR", d.dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let out = std::io::BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            out,
            next: 1,
        }
    }
    fn rpc(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        use std::io::{BufRead, Write};
        let id = self.next;
        self.next += 1;
        let line =
            serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
        let mut reply = String::new();
        self.out.read_line(&mut reply).unwrap();
        let v: serde_json::Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(v["id"], id);
        v
    }
    fn call(&mut self, name: &str, args: serde_json::Value) -> (String, bool) {
        let v = self.rpc(
            "tools/call",
            serde_json::json!({"name": name, "arguments": args}),
        );
        (
            v["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_string(),
            v["result"]["isError"].as_bool().unwrap(),
        )
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn mcp_server_runs_a_persistent_pane_waits_for_output_and_respects_the_controller() {
    let d = Daemon::start();
    let mut m = Mcp::start(&d, &["--allow-write"]);
    let init = m.rpc("initialize", serde_json::json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}));
    assert_eq!(init["result"]["serverInfo"]["name"], "teryon");
    // A durable pane running a job that prints after a short delay.
    let (msg, err) = m.call(
        "new_pane",
        serde_json::json!({"command": ["sh", "-c", "sleep 1; echo ready-for-mcp; sleep 20"]}),
    );
    assert!(!err && msg.contains("started pane"), "{msg}");
    let pane: u64 = msg.split_whitespace().nth(2).unwrap().parse().unwrap();
    let (hit, err) = m.call(
        "wait_for",
        serde_json::json!({"pane": pane, "pattern": "ready-for-[a-z]+", "timeout_s": 10}),
    );
    assert!(!err && hit == "MATCH: ready-for-mcp", "{hit}");
    let (text, _) = m.call("read_pane", serde_json::json!({"pane": pane}));
    assert!(text.contains("ready-for-mcp"));
    let (list, _) = m.call("list_workspaces", serde_json::json!({}));
    assert!(list.contains("mcp\tpanes="), "{list}");
    let (panes, _) = m.call("list_panes", serde_json::json!({}));
    assert!(panes.contains("running"), "{panes}");
    // The pane belongs to the daemon, so it outlives this MCP server.
    drop(m);
    let mut c = d.client();
    c.attach("mcp", true);
    assert!(c
        .snapshot()
        .iter()
        .any(|p| p.id == pane && p.exited.is_none()));
    // When another client controls the workspace, reads still work and writes explain why they fail.
    let mut owner = d.client();
    owner.attach("mcp", false);
    let mut m2 = Mcp::start(&d, &["--allow-write"]);
    let (text, err) = m2.call("read_pane", serde_json::json!({"pane": pane}));
    assert!(!err && text.contains("ready-for-mcp"), "{text}");
    let (msg, err) = m2.call("send_keys", serde_json::json!({"pane": pane, "text": "x"}));
    assert!(err && msg.contains("controlled by another"), "{msg}");
    // Without --allow-write there are no write tools at all.
    let mut ro = Mcp::start(&d, &[]);
    let tools = ro.rpc("tools/list", serde_json::json!({}));
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 6);
}

#[test]
fn an_inline_image_reserves_its_rows_and_its_bytes_can_be_fetched() {
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    let d = Daemon::start();
    let mut c = d.client();
    c.attach("images", false);
    let script = format!(
        "printf 'before\\n'; printf '\\033]1337;File=inline=1;width=10:{PNG}\\007'; printf '\\nafter\\n'; sleep 6"
    );
    let Response::Created { pane } = c.call(Request::NewPane {
        cwd: None,
        command: vec!["/bin/sh".into(), "-c".into(), script],
        rows: 24,
        cols: 80,
    }) else {
        panic!("no pane")
    };
    c.until(pane, "after");
    let view = c.snapshot().into_iter().find(|p| p.id == pane).unwrap();
    let images = view.images.expect("images are sent with the screen");
    assert_eq!(images.len(), 1);
    // The picture starts under "before", covers 10 columns by 5 rows, and "after" follows it.
    assert_eq!((images[0].line, images[0].cols, images[0].rows), (1, 10, 5));
    let text = c.capture(pane);
    let after = text.lines().position(|l| l.starts_with("after")).unwrap();
    assert_eq!(after, 6, "{text:?}");
    let Response::ImageData { id, data } = c.call(Request::Image {
        pane,
        id: images[0].id,
    }) else {
        panic!("no image")
    };
    assert_eq!(id, images[0].id);
    assert_eq!(
        teryon::inlineimg::decode_base64(&data).unwrap()[..4],
        *b"\x89PNG"
    );
    assert!(matches!(
        c.call(Request::Image { pane, id: 999 }),
        Response::Error { .. }
    ));
}
#[test]
fn a_recipe_starts_named_panes_that_run_their_commands_and_refuses_to_start_twice() {
    let d = Daemon::start();
    let mut c = d.client();
    let recipe = teryon::recipe::parse(
        "[[pane]]\nname = \"one\"\ncwd = \"/tmp\"\ncommand = \"echo FIRST-$((20+22))\"\n\
         [[pane]]\nname = \"two\"\ncommand = \"echo SECOND-$((30+12))\"\n",
    )
    .unwrap();
    let base = std::path::Path::new("/tmp");
    let ids =
        teryon::recipe::up(&mut |r| Ok(c.call(r)), &recipe, "recipe-test", base, true).unwrap();
    assert_eq!(ids, [1, 2]);
    c.until(1, "FIRST-42");
    c.until(2, "SECOND-42");
    let titles: Vec<String> = c.snapshot().into_iter().map(|p| p.title).collect();
    assert_eq!(titles, ["one", "two"]);
    let err =
        teryon::recipe::up(&mut |r| Ok(c.call(r)), &recipe, "recipe-test", base, true).unwrap_err();
    assert!(format!("{err:#}").contains("already exists"));
}
#[test]
fn a_soft_closed_pane_keeps_running_and_comes_back_until_its_grace_period_ends() {
    let d = Daemon::start_with(&[("TERYON_UNDO_SECS", "2")]);
    let mut c = d.client();
    c.attach("undo", false);
    let sh = |script: &str| Request::NewPane {
        cwd: None,
        command: vec!["/bin/sh".into(), "-c".into(), script.into()],
        rows: 24,
        cols: 80,
    };
    let Response::Created { pane } = c.call(sh("echo KEEP-42; while :; do sleep 1; done")) else {
        panic!()
    };
    c.until(pane, "KEEP-42");
    let pid = c
        .snapshot()
        .into_iter()
        .find(|p| p.id == pane)
        .unwrap()
        .pid
        .unwrap();
    // Closing hides the pane but its process keeps running.
    assert!(matches!(
        c.call(Request::SoftClose { pane }),
        Response::Ack { .. }
    ));
    assert!(c.snapshot().iter().all(|p| p.id != pane));
    assert_eq!(unsafe { libc::kill(pid as i32, 0) }, 0, "still alive");
    // Reopening returns the same pane with its output.
    let Response::Created { pane: back } = c.call(Request::Reopen) else {
        panic!("reopen")
    };
    assert_eq!(back, pane);
    c.until(pane, "KEEP-42");
    // Nothing left to reopen, and a pane that is never reopened is ended after the grace period.
    assert!(matches!(c.call(Request::Reopen), Response::Error { .. }));
    assert!(matches!(
        c.call(Request::SoftClose { pane }),
        Response::Ack { .. }
    ));
    let deadline = Instant::now() + Duration::from_secs(6);
    while unsafe { libc::kill(pid as i32, 0) } == 0 {
        assert!(Instant::now() < deadline, "the reaper never ended the pane");
        thread::sleep(Duration::from_millis(100));
    }
    assert!(matches!(c.call(Request::Reopen), Response::Error { .. }));
    // The last pane cannot be soft-closed.
    assert!(matches!(
        c.call(Request::SoftClose { pane: 1 }),
        Response::Error { .. }
    ));
}

#[test]
fn osc8_hyperlinks_reach_the_client_as_spans_beside_the_screen() {
    let d = Daemon::start();
    let mut c = d.client();
    c.attach("links", false);
    let script = "printf 'see \\033]8;;https://docs.example/a?x=1;y=2\\033\\\\the docs\\033]8;;\\033\\\\ now\\n'; printf 'done\\n'; sleep 6";
    let Response::Created { pane } = c.call(Request::NewPane {
        cwd: None,
        command: vec!["/bin/sh".into(), "-c".into(), script.into()],
        rows: 24,
        cols: 80,
    }) else {
        panic!("no pane")
    };
    c.until(pane, "done");
    let view = c.snapshot().into_iter().find(|p| p.id == pane).unwrap();
    let links = view.links.expect("links are sent with the screen");
    assert_eq!(
        links,
        vec![teryon::protocol::LinkSpan {
            row: 0,
            start: 4,
            end: 12,
            url: "https://docs.example/a?x=1;y=2".into(),
        }]
    );
    // The text itself is untouched, and the link markup never shows up in it.
    assert!(c.capture(pane).starts_with("see the docs now"));
}

#[test]
fn a_takeover_moves_the_controller_seat_and_the_old_controller_keeps_watching() {
    let d = Daemon::start();
    let mut laptop = d.client();
    assert!(matches!(
        laptop.attach("seat", false),
        Response::Attached { .. }
    ));
    let mut phone = d.client();
    // Without asking, a second controller is still refused.
    assert!(matches!(
        phone.attach("seat", false),
        Response::Error { .. }
    ));
    // Asking takes the seat at once, even though the laptop's connection is alive.
    assert!(matches!(
        phone.take_over("seat"),
        Response::Attached {
            readonly: false,
            ..
        }
    ));
    let input = |pane, seq, text: &str| Request::Input {
        pane,
        seq,
        data: text.as_bytes().to_vec(),
    };
    assert!(matches!(
        phone.call(input(1, 1, "echo from-phone\r")),
        Response::Ack { .. }
    ));
    phone.until(1, "from-phone");
    // The laptop is told why it cannot type, and can still see the screen.
    let Response::Error { message } = laptop.call(input(1, 1, "echo from-laptop\r")) else {
        panic!("the old controller must not be able to type")
    };
    assert!(message.contains("taken over"), "{message}");
    assert!(matches!(
        laptop.call(Request::Resize {
            pane: 1,
            rows: 30,
            cols: 90
        }),
        Response::Error { .. }
    ));
    assert!(laptop.capture(1).contains("from-phone"));
    assert!(!laptop.capture(1).contains("from-laptop"));
    // A plain read-only attachment is still just read-only, not "taken over".
    let mut viewer = d.client();
    viewer.attach("seat", true);
    let Response::Error { message } = viewer.call(input(1, 1, "x")) else {
        panic!("viewer typed")
    };
    assert!(message.contains("read-only"), "{message}");
    // And the seat can be taken back.
    assert!(matches!(
        laptop.take_over("seat"),
        Response::Attached { .. }
    ));
    assert!(matches!(
        laptop.call(input(1, 1, "echo back\r")),
        Response::Ack { .. }
    ));
    assert!(matches!(
        phone.call(input(1, 2, "x")),
        Response::Error { .. }
    ));
}

/// Start `script` in a new pane of workspace `name` and return the client and the pane id.
fn pane_running(d: &Daemon, name: &str, script: &str) -> (Client, u64) {
    let mut c = d.client();
    c.attach(name, false);
    let Response::Created { pane } = c.call(Request::NewPane {
        cwd: None,
        command: vec!["/bin/sh".into(), "-c".into(), script.into()],
        rows: 24,
        cols: 80,
    }) else {
        panic!("no pane")
    };
    (c, pane)
}
fn kbd(c: &mut Client, pane: u64) -> u8 {
    c.snapshot().into_iter().find(|p| p.id == pane).unwrap().kbd
}

#[test]
fn kitty_keyboard_flags_are_answered_pushed_popped_and_reach_the_client() {
    let d = Daemon::start();
    // The program asks for the disambiguate flag, then asks what is in force and reads the answer
    // from its own terminal input.
    let (mut c, pane) = pane_running(
        &d,
        "kitty",
        "stty -icanon -echo; printf '\\033[>1u\\033[?u'; dd bs=1 count=5 2>/dev/null | od -c; printf '\\rpushed\\n'; sleep 2; printf '\\033[<u'; printf '\\rpopped\\n'; sleep 6",
    );
    c.until(pane, "pushed");
    let shown = c.capture(pane);
    assert!(
        shown.contains("033   [   ?   1   u"),
        "reply to the query: {shown:?}"
    );
    assert_eq!(kbd(&mut c, pane), 1);
    c.until(pane, "popped");
    assert_eq!(kbd(&mut c, pane), 0);
}

#[test]
fn kitty_flags_ask_only_for_what_is_supported_and_leave_with_the_alternate_screen() {
    let d = Daemon::start();
    // Flags this terminal does not honor are not reported back; a full-screen program that exits
    // without popping does not leave its encoding behind.
    let (mut c, pane) = pane_running(
        &d,
        "kitty2",
        "printf '\\033[?1049h\\033[>31u'; printf 'inalt\\n'; sleep 2; printf '\\033[?1049l'; printf 'left\\n'; sleep 6",
    );
    c.until(pane, "inalt");
    assert_eq!(
        kbd(&mut c, pane),
        1,
        "only the disambiguate bit of 31 is kept"
    );
    c.until(pane, "left");
    assert_eq!(kbd(&mut c, pane), 0);
}

#[test]
fn screen_diffs_rebuild_the_same_screen_in_far_fewer_bytes() {
    use teryon::screensync::{Applied, ClientMirrors};
    let d = Daemon::start();
    // A pane that keeps printing, so the screen changes many times while two viewers poll it.
    let script = "i=0; while [ $i -lt 120 ]; do printf 'line %03d \\033[3%dmcolored text here\\033[0m and some more words\\n' $i $((i % 8)); i=$((i+1)); sleep 0.02; done; printf 'finished\\n'; sleep 8";
    let (mut owner, pane) = pane_running(&d, "diffs", script);
    let viewer = |diff: bool| {
        let mut c = d.client();
        c.attach("diffs", true);
        (c, diff)
    };
    let (mut with_diff, _) = viewer(true);
    let (mut whole, _) = viewer(false);
    assert!(matches!(
        with_diff.call(Request::Info),
        Response::Info { diff: true, .. }
    ));
    let (mut known_d, mut known_w) = (BTreeMap::new(), BTreeMap::new());
    let (mut bytes_d, mut bytes_w, mut diffs_seen) = (0usize, 0usize, 0usize);
    let mut mirrors = ClientMirrors::default();
    let mut meta = teryon::screensync::MetaCache::default();
    let mut details = None;
    let mut rebuilt: Option<Vec<u8>> = None;
    let mut full_w: Option<Vec<u8>> = None;
    let mut whole_details = None;
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        let Response::State { panes } = with_diff.call(Request::Poll {
            known: known_d.clone(),
            diff: true,
        }) else {
            panic!("no state")
        };
        // What crossed the wire: the JSON of the whole reply, as the daemon wrote it.
        bytes_d += serde_json::to_string(&panes).unwrap().len();
        for p in &panes {
            known_d.insert(p.id.to_string(), p.revision);
            let mut p = p.clone();
            let p = &mut p;
            assert!(
                meta.complete(p),
                "details were left out that were never sent"
            );
            if p.id == pane {
                details = Some((p.title.clone(), p.cwd.clone(), p.fg.clone()));
            }
            assert!(p.screen.is_none(), "a diff client is sent the compact form");
            let bytes = p
                .screen_b64
                .as_deref()
                .map(|t| teryon::inlineimg::decode_base64(t).unwrap());
            if let Some(b) = &bytes {
                diffs_seen += usize::from(p.diff_from.is_some());
                match mirrors.apply(
                    p.id,
                    p.revision,
                    (p.rows, p.cols),
                    Some(b),
                    p.diff_from,
                    p.digest,
                ) {
                    Applied::Plain => rebuilt = Some(b.clone()),
                    Applied::Rebuilt(full) => rebuilt = Some(full),
                    Applied::Resync => panic!("a diff did not fit"),
                }
            }
        }
        let Response::State { panes } = whole.call(Request::Poll {
            known: known_w.clone(),
            diff: false,
        }) else {
            panic!("no state")
        };
        bytes_w += serde_json::to_string(&panes).unwrap().len();
        for p in &panes {
            known_w.insert(p.id.to_string(), p.revision);
            if p.id == pane {
                whole_details = Some((p.title.clone(), p.cwd.clone(), p.fg.clone()));
            }
            if let Some(b) = &p.screen {
                full_w = Some(b.clone());
            }
        }
        if owner.capture(pane).contains("finished") && known_d == known_w {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    // One last poll each so both hold the final screen.
    let text = |bytes: &[u8]| {
        let mut p = vt100::Parser::new(24, 80, 0);
        p.process(bytes);
        p.screen().contents()
    };
    let (a, b) = (text(&rebuilt.unwrap()), text(&full_w.unwrap()));
    assert!(
        diffs_seen > 5,
        "the viewer should have been sent diffs, saw {diffs_seen}"
    );
    eprintln!("scrolling pane: {bytes_d} bytes of JSON as diffs, {bytes_w} as whole screens");
    assert_eq!(
        details, whole_details,
        "the title, directory and command survive being left out of quiet replies"
    );
    assert!(a.contains("line") && b.contains("line"));
    assert_eq!(a, b, "diffs rebuild exactly what whole snapshots show");
    assert!(
        bytes_d * 5 < bytes_w,
        "diffs sent {bytes_d} bytes against {bytes_w} for whole screens"
    );
}

#[test]
fn a_recipe_pane_waits_for_the_one_it_depends_on_to_print_its_ready_text() {
    let d = Daemon::start();
    let mut c = d.client();
    let recipe = teryon::recipe::parse(
        "[[pane]]\nname = \"app\"\nafter = \"db\"\ncommand = \"echo APP-STARTED\"\n\
         [[pane]]\nname = \"db\"\nready_text = \"DB-IS-READY\"\nready_timeout = 20\n\
         command = \"sleep 2; echo DB-IS-READY; sleep 30\"\n",
    )
    .unwrap();
    let base = std::path::Path::new("/tmp");
    let began = Instant::now();
    let mut seen_while_waiting = None;
    let ids = teryon::recipe::up_with(
        &mut |r| Ok(c.call(r)),
        &recipe,
        "recipe-wait",
        base,
        true,
        &mut |line| {
            // Called when the wait begins: the app must not have been started yet.
            assert!(line.contains("`db`"), "{line}");
            let mut obs = d.client();
            obs.attach("recipe-wait", true);
            seen_while_waiting = Some(obs.capture(1));
        },
    )
    .unwrap();
    assert_eq!(ids, [1, 2]);
    assert!(
        began.elapsed() >= Duration::from_millis(1800),
        "{:?}",
        began.elapsed()
    );
    assert!(
        !seen_while_waiting
            .expect("it had to wait")
            .contains("APP-STARTED"),
        "the app started before the database was ready"
    );
    c.until(1, "APP-STARTED");
    c.until(2, "DB-IS-READY");
}

#[test]
fn a_recipe_pane_that_never_gets_ready_stops_what_waits_for_it() {
    let d = Daemon::start();
    let mut c = d.client();
    let recipe = teryon::recipe::parse(
        "[[pane]]\nname = \"db\"\nready_text = \"NEVER-PRINTED\"\nready_timeout = 1\n\
         command = \"echo STARTING-DB; sleep 30\"\n\
         [[pane]]\nname = \"app\"\nafter = \"db\"\ncommand = \"echo APP-STARTED\"\n",
    )
    .unwrap();
    let base = std::path::Path::new("/tmp");
    let err = teryon::recipe::up(&mut |r| Ok(c.call(r)), &recipe, "recipe-stuck", base, true)
        .unwrap_err();
    let text = format!("{err:#}");
    assert!(text.contains("`db` was not ready after 1 s"), "{text}");
    assert!(text.contains("`app` was not started"), "{text}");
    // What had been started keeps running; the dependent never ran.
    c.until(1, "STARTING-DB");
    assert!(!c.capture(2).contains("APP-STARTED"));
}

#[test]
fn a_recipe_pane_can_be_ready_when_it_listens_on_a_port() {
    use std::process::Command;
    if Command::new("python3").arg("--version").output().is_err()
        || Command::new("lsof").arg("-v").output().is_err()
    {
        return;
    }
    // A port that is free right now.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let d = Daemon::start();
    let mut c = d.client();
    let serve = format!(
        "python3 -c \\\"import socket,time; s=socket.socket(); s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1); s.bind(('127.0.0.1',{port})); s.listen(); time.sleep(40)\\\""
    );
    let recipe = teryon::recipe::parse(&format!(
        "[[pane]]\nname = \"web\"\nready_port = {port}\nready_timeout = 30\ncommand = \"sleep 1; {serve}\"\n\
         [[pane]]\nname = \"check\"\nafter = \"web\"\ncommand = \"echo CHECK-RAN\"\n"
    ))
    .unwrap();
    let base = std::path::Path::new("/tmp");
    teryon::recipe::up(&mut |r| Ok(c.call(r)), &recipe, "recipe-port", base, true).unwrap();
    c.until(2, "CHECK-RAN");
}
