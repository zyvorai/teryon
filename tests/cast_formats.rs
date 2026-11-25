// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! asciicast v3 files, `cast convert|cat|mark` and `play --pause-on-markers` through the real binary.
mod common;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const V3: &str = r#"{"version":3,"term":{"cols":80,"rows":24,"type":"xterm"},"title":"demo","idle_time_limit":0.1}
# recorded by asciinema 3
[0.5,"o","before\r\n"]
[0.25,"m","chapter two"]
[30.0,"o","after\r\n"]
[0.1,"x","0"]
"#;

fn teryon(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_teryon"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}
fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn file(dir: &Path, name: &str, body: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p.display().to_string()
}
fn out(dir: &Path, name: &str) -> String {
    dir.join(name).display().to_string()
}
fn ok(args: &[&str]) -> Output {
    let o = teryon(args);
    assert!(o.status.success(), "{args:?}: {o:?}");
    o
}

#[test]
fn a_v3_recording_plays_and_reports_its_markers() {
    let d = tempfile::tempdir().unwrap();
    let src = file(d.path(), "a.cast", V3);
    let info = text(&ok(&["play", &src, "--info"]));
    assert!(
        info.contains("80x24 terminal, 2 frames, 30.8s") && info.contains("asciicast v3"),
        "{info}"
    );
    assert!(info.contains("marker at 0.8s: chapter two"), "{info}");
    // The 30 s pause is cut to the file's idle_time_limit, so this returns quickly.
    let start = std::time::Instant::now();
    let played = text(&ok(&["play", &src]));
    assert!(played.starts_with("before\r\nafter\r\n"), "{played:?}");
    assert!(start.elapsed().as_secs() < 10);
}

#[test]
fn convert_goes_to_v2_back_to_v3_and_to_text() {
    let d = tempfile::tempdir().unwrap();
    let src = file(d.path(), "a.cast", V3);
    let (v2, v3, txt) = (
        out(d.path(), "v2.cast"),
        out(d.path(), "v3.cast"),
        out(d.path(), "a.txt"),
    );
    ok(&["cast", "convert", &src, "--to", "v2", "-o", &v2]);
    let body = std::fs::read_to_string(&v2).unwrap();
    let head: serde_json::Value = serde_json::from_str(body.lines().next().unwrap()).unwrap();
    assert_eq!(
        (&head["version"], &head["width"], &head["env"]["TERM"]),
        (&2.into(), &80.into(), &"xterm".into())
    );
    assert!(body.contains(r#"[30.75,"o","after\r\n"]"#), "{body}");
    assert!(!body.contains(r#""x""#), "{body}");
    ok(&["cast", "convert", &v2, "--to", "v3", "-o", &v3]);
    let body = std::fs::read_to_string(&v3).unwrap();
    assert!(body.contains(r#"[30.0,"o","after\r\n"]"#), "{body}");
    assert!(body.contains(r#""cols":80"#), "{body}");
    ok(&["cast", "convert", &src, "--to", "txt", "-o", &txt]);
    assert_eq!(std::fs::read_to_string(&txt).unwrap(), "before\nafter\n");
    // Never overwrites, and says why.
    let again = teryon(&["cast", "convert", &src, "--to", "txt", "-o", &txt]);
    assert!(!again.status.success());
    assert!(String::from_utf8_lossy(&again.stderr).contains("refusing to overwrite"));
    let bad = teryon(&[
        "cast",
        "convert",
        &src,
        "--to",
        "gif",
        "-o",
        &out(d.path(), "g"),
    ]);
    assert!(!bad.status.success());
}

#[test]
fn cat_joins_recordings_and_mark_adds_a_chapter() {
    let d = tempfile::tempdir().unwrap();
    let a = file(d.path(), "a.cast", V3);
    let b = file(
        d.path(),
        "b.cast",
        "{\"version\":2,\"width\":100,\"height\":30}\n[1.0,\"o\",\"second\\r\\n\"]\n",
    );
    let joined = out(d.path(), "ab.cast");
    ok(&["cast", "cat", &a, &b, "-o", &joined]);
    let info = text(&ok(&["play", &joined, "--info"]));
    assert!(info.contains("3 frames"), "{info}");
    // The second file starts where the first ended (its exit event), after a resize to its size.
    let body = std::fs::read_to_string(&joined).unwrap();
    assert!(body.contains(r#"[0.1,"r","100x30"]"#), "{body}");
    assert!(body.contains(r#"[1.0,"o","second\r\n"]"#), "{body}");
    assert!(!body.contains(r#""x""#), "{body}");
    let one = teryon(&["cast", "cat", &a, "-o", &out(d.path(), "one")]);
    assert!(!one.status.success());

    let marked = out(d.path(), "m.cast");
    ok(&[
        "cast", "mark", &b, "--at", "0.5", "--label", "intro", "-o", &marked,
    ]);
    let info = text(&ok(&["play", &marked, "--info"]));
    assert!(info.contains("marker at 0.5s: intro"), "{info}");
    let late = teryon(&[
        "cast",
        "mark",
        &b,
        "--at",
        "99",
        "-o",
        &out(d.path(), "late"),
    ]);
    assert!(!late.status.success());
    assert!(!d.path().join("late").exists());
}

#[test]
fn a_redacted_v3_copy_has_no_secret_in_header_title_or_marker() {
    let d = tempfile::tempdir().unwrap();
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let src = file(
        d.path(),
        "s.cast",
        &format!(
            "{{\"version\":3,\"term\":{{\"cols\":80,\"rows\":24}},\"command\":\"curl -H {secret}\",\"title\":\"{secret}\",\"env\":{{\"SHELL\":\"/bin/sh\",\"OTHER\":\"{secret}\"}}}}\n# {secret}\n[0.1,\"o\",\"\\u001b]0;{secret}\\u0007\\u001b]8;;http://h/{secret}\\u001b\\\\hi\"]\n[0.1,\"m\",\"{secret}\"]\n[0.1,\"i\",\"{secret}\"]\n[0.1,\"z\",\"{secret}\"]\n"
        ),
    );
    let dst = out(d.path(), "r.cast");
    ok(&["cast", "redact", &src, "-o", &dst]);
    let body = std::fs::read_to_string(&dst).unwrap();
    assert!(!body.contains(secret), "{body}");
    assert!(!body.contains("]0;") && !body.contains("]8;"), "{body}");
    assert!(body.contains("/bin/sh") && body.contains("hi"), "{body}");
    assert!(body.contains("\"version\":3"), "{body}");
}

#[test]
fn pause_on_markers_waits_for_a_key_and_q_stops() {
    let d = tempfile::tempdir().unwrap();
    let src = file(d.path(), "a.cast", V3);
    let no_tty = teryon(&["play", &src, "--pause-on-markers"]);
    assert!(!no_tty.status.success());
    assert!(String::from_utf8_lossy(&no_tty.stderr).contains("needs a terminal"));

    let mut s = common::Session::start(d.path(), &["play", &src, "--pause-on-markers"]);
    s.expect("before");
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(!s.has("after"), "playback went past the marker");
    s.send(b" ");
    s.expect("after");
    assert_eq!(s.exit_code(), 0);

    let mut s = common::Session::start(d.path(), &["play", &src, "--pause-on-markers"]);
    s.expect("before");
    s.send(b"q");
    assert_eq!(s.exit_code(), 0);
    assert!(!s.has("after"));
}
