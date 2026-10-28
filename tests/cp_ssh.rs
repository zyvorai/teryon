//! `teryon cp` against a real OpenSSH server run as the current user on a free loopback port.
//! Skipped (with a note) when no usable `sshd` can be started here.
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

struct Server {
    child: Child,
    ssh_config: PathBuf,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn sftp_server() -> Option<&'static str> {
    [
        "/usr/libexec/sftp-server",
        "/usr/lib/openssh/sftp-server",
        "/usr/lib/ssh/sftp-server",
    ]
    .into_iter()
    .find(|p| Path::new(p).exists())
}

fn keygen(path: &Path) -> bool {
    Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(path)
        .status()
        .is_ok_and(|s| s.success())
}

fn start(dir: &Path) -> Option<Server> {
    let sshd = ["/usr/sbin/sshd", "/usr/bin/sshd"]
        .into_iter()
        .find(|p| Path::new(p).exists())?;
    let sftp = sftp_server()?;
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .ok()?
        .local_addr()
        .ok()?
        .port();
    let (host, client) = (dir.join("hostkey"), dir.join("client"));
    if !keygen(&host) || !keygen(&client) {
        return None;
    }
    std::fs::copy(dir.join("client.pub"), dir.join("authorized_keys")).ok()?;
    let d = dir.display();
    std::fs::write(
        dir.join("sshd_config"),
        format!(
            "Port {port}\nListenAddress 127.0.0.1\nHostKey {d}/hostkey\nPidFile {d}/sshd.pid\n\
             AuthorizedKeysFile {d}/authorized_keys\nPasswordAuthentication no\n\
             KbdInteractiveAuthentication no\nUsePAM no\nStrictModes no\nLogLevel ERROR\n\
             Subsystem sftp {sftp}\n"
        ),
    )
    .ok()?;
    let key = std::fs::read_to_string(dir.join("hostkey.pub")).ok()?;
    let key: Vec<&str> = key.split_whitespace().take(2).collect();
    std::fs::write(
        dir.join("known_hosts"),
        format!("[127.0.0.1]:{port} {} {}\n", key[0], key[1]),
    )
    .ok()?;
    let user = std::env::var("USER").ok()?;
    let ssh_config = dir.join("ssh_config");
    std::fs::write(
        &ssh_config,
        format!(
            "Host tsrv\n  HostName 127.0.0.1\n  Port {port}\n  User {user}\n  IdentityFile {d}/client\n  \
             IdentitiesOnly yes\n  UserKnownHostsFile {d}/known_hosts\n"
        ),
    )
    .ok()?;
    let child = Command::new(sshd)
        .arg("-D")
        .arg("-f")
        .arg(dir.join("sshd_config"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let server = Server { child, ssh_config };
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let ok = Command::new("ssh")
            .arg("-F")
            .arg(&server.ssh_config)
            .args([
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=2",
                "tsrv",
                "true",
            ])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if ok {
            return Some(server);
        }
    }
    None
}

fn rsync_ok() -> bool {
    Command::new("rsync")
        .arg("--version")
        .output()
        .is_ok_and(|o| {
            o.status.success() && !String::from_utf8_lossy(&o.stdout).contains("openrsync")
        })
}

struct Env {
    home: tempfile::TempDir,
    work: tempfile::TempDir,
    server: Server,
}
impl Env {
    fn cp(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_teryon"))
            .env("HOME", self.home.path())
            .env("TERYON_SSH_CONFIG", &self.server.ssh_config)
            .arg("cp")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
    fn p(&self, rel: &str) -> String {
        self.work.path().join(rel).display().to_string()
    }
    fn remote(&self, rel: &str) -> String {
        format!("tsrv:{}", self.p(rel))
    }
}

fn setup() -> Option<Env> {
    let home = tempfile::tempdir().ok()?;
    let ssh_dir = tempfile::tempdir().ok()?;
    let server = start(ssh_dir.path())?;
    // Keep the key material alive as long as the server.
    std::mem::forget(ssh_dir);
    let work = tempfile::tempdir().ok()?;
    std::fs::create_dir_all(work.path().join("src/sub")).ok()?;
    std::fs::create_dir_all(work.path().join("src/with space")).ok()?;
    std::fs::write(work.path().join("src/a.txt"), "alpha\n").ok()?;
    std::fs::write(work.path().join("src/sub/b.txt"), "beta\n").ok()?;
    std::fs::write(work.path().join("src/with space/c d.txt"), "gamma\n").ok()?;
    Some(Env { home, work, server })
}

fn read(path: String) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn cp_copies_files_folders_streams_and_wildcards_over_real_ssh() {
    let Some(e) = setup() else {
        eprintln!("skipped: cannot start a user-level sshd here");
        return;
    };
    let engines: Vec<&str> = if rsync_ok() {
        vec!["scp", "rsync"]
    } else {
        vec!["scp"]
    };
    for engine in engines {
        let dst = format!("dst-{engine}");
        let out = e.cp(&[
            "--engine",
            engine,
            "-p",
            &e.p("src/a.txt"),
            &e.remote(&format!("{dst}/a.txt")),
        ]);
        assert!(out.status.success(), "{engine} file up: {out:?}");
        assert_eq!(read(e.p(&format!("{dst}/a.txt"))), "alpha\n", "{engine}");

        let out = e.cp(&[
            "--engine",
            engine,
            "-p",
            &e.p("src"),
            &e.remote(&format!("{dst}/dir/")),
        ]);
        assert!(out.status.success(), "{engine} folder up: {out:?}");
        assert_eq!(
            read(e.p(&format!("{dst}/dir/src/sub/b.txt"))),
            "beta\n",
            "{engine}"
        );

        let out = e.cp(&[
            "--engine",
            engine,
            &e.remote("src/with space/c d.txt"),
            &e.p(&format!("down-{engine}.txt")),
        ]);
        assert!(
            out.status.success(),
            "{engine} name with spaces down: {out:?}"
        );
        assert_eq!(
            read(e.p(&format!("down-{engine}.txt"))),
            "gamma\n",
            "{engine}"
        );
    }

    // Standard input and output.
    let mut child = Command::new(env!("CARGO_BIN_EXE_teryon"))
        .env("HOME", e.home.path())
        .env("TERYON_SSH_CONFIG", &e.server.ssh_config)
        .args(["cp", "-", &e.remote("streamed.txt")])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"from stdin\n")
        .unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(read(e.p("streamed.txt")), "from stdin\n");
    let out = e.cp(&[&e.remote("streamed.txt"), "-"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "from stdin\n");

    // Wildcards expand on the host; no match is an error, not a silent no-op.
    std::fs::create_dir_all(e.p("globbed")).unwrap();
    let out = e.cp(&[&e.remote("src/*.txt"), &e.p("globbed/")]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(read(e.p("globbed/a.txt")), "alpha\n");
    let out = e.cp(&[&e.remote("src/nomatch*"), &e.p("globbed/")]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no match"),
        "{out:?}"
    );

    // --no-clobber keeps what is there; host to host goes through scp.
    std::fs::write(e.p("keep.txt"), "original\n").unwrap();
    let out = e.cp(&[
        "--no-clobber",
        "--engine",
        "scp",
        &e.p("src/a.txt"),
        &e.remote("keep.txt"),
    ]);
    assert!(
        !out.status.success(),
        "scp cannot honour --no-clobber: {out:?}"
    );
    assert_eq!(read(e.p("keep.txt")), "original\n");
    let out = e.cp(&[&e.remote("src/a.txt"), &e.remote("h2h.txt")]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(read(e.p("h2h.txt")), "alpha\n");
    if rsync_ok() {
        let out = e.cp(&[
            "--no-clobber",
            "--engine",
            "rsync",
            &e.p("src/a.txt"),
            &e.remote("keep.txt"),
        ]);
        assert!(out.status.success(), "{out:?}");
        assert_eq!(read(e.p("keep.txt")), "original\n");
    }
}

#[test]
fn remote_paths_complete_over_real_ssh() {
    let Some(e) = setup() else {
        eprintln!("skipped: cannot start a user-level sshd here");
        return;
    };
    let complete = |prefix: &str| -> Vec<String> {
        let out = Command::new(env!("CARGO_BIN_EXE_teryon"))
            .env("HOME", e.home.path())
            .env("TERYON_SSH_CONFIG", &e.server.ssh_config)
            .args(["complete", "remote-path", "tsrv", prefix])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_string)
            .collect()
    };
    let base = e.p("src");
    assert_eq!(
        complete(&format!("{base}/")),
        [
            format!("{base}/a.txt"),
            format!("{base}/sub/"),
            format!("{base}/with space/"),
        ]
    );
    assert_eq!(complete(&format!("{base}/s")), [format!("{base}/sub/")]);
    assert_eq!(
        complete(&format!("{base}/with space/")),
        [format!("{base}/with space/c d.txt")]
    );
    assert!(complete(&format!("{base}/zzz")).is_empty());
}

#[test]
fn hist_sync_copies_a_hosts_history_over_real_ssh_and_auto_mode_stays_quiet() {
    let Some(e) = setup() else {
        eprintln!("skipped: cannot start a user-level sshd here");
        return;
    };
    let cfg_dir = e.home.path().join(".config/teryon");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::write(
        cfg_dir.join("config.toml"),
        format!(
            "[hosts.tsrv]\ndestination = \"tsrv\"\nremote_bin = \"{}\"\n[shell]\nsync_hosts = [\"tsrv\"]\n",
            env!("CARGO_BIN_EXE_teryon")
        ),
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_teryon"))
            .env("HOME", e.home.path())
            .env("TERYON_SSH_CONFIG", &e.server.ssh_config)
            .args(["hist", "sync"])
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let out = run(&[]);
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).starts_with("tsrv: "),
        "{out:?}"
    );
    assert!(e.home.path().join(".teryon/history-hosts").exists());

    let out = run(&["--auto"]);
    assert!(out.status.success() && out.stdout.is_empty(), "{out:?}");
    let stamp = e.home.path().join(".teryon/hist-sync-stamp");
    assert!(stamp.exists(), "auto mode leaves a stamp");
    let first = std::fs::metadata(&stamp).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    run(&["--auto"]);
    assert_eq!(
        std::fs::metadata(&stamp).unwrap().modified().unwrap(),
        first,
        "not again within the interval"
    );
}
