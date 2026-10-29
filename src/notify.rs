//! Delivering a daemon notification to the configured sinks (macOS, ntfy, a webhook).
use crate::config::NotifyConfig;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// How long to wait before each new attempt at a notification the sink refused or could not be
/// reached for (the Mac mini asleep, no network). After the last one it is dropped.
const RETRY_DELAYS: [Duration; 5] = [
    Duration::from_secs(5),
    Duration::from_secs(30),
    Duration::from_secs(120),
    Duration::from_secs(600),
    Duration::from_secs(1800),
];
/// Notifications waiting for a retry at once. Each holds a sleeping thread, so past this a failed
/// notification is dropped instead of retried.
const MAX_RETRYING: usize = 32;
static RETRYING: AtomicUsize = AtomicUsize::new(0);
/// Notifications given up on since the daemon started (all attempts failed, or too many were waiting).
static DROPPED: AtomicUsize = AtomicUsize::new(0);

/// How many notifications have been given up on since this process started.
#[allow(dead_code)]
pub fn dropped() -> usize {
    DROPPED.load(Ordering::Relaxed)
}

/// One notification: `event` is a stable machine name such as `agent_needs_you`.
pub struct Note {
    pub event: &'static str,
    pub title: String,
    pub message: String,
    pub workspace: String,
    pub pane: u64,
}

/// Printable, single-line text of at most `max` characters.
pub fn clean(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    flat.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

/// Quote a value for a curl config file. A raw newline would end the line and let the rest be read as
/// another curl option, so line breaks and tabs use curl's escapes and other control characters are dropped.
fn quote(v: &str) -> String {
    let mut out = String::from("\"");
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A curl `--config -` body, so the URL (often a secret topic) and the message stay off the command line.
pub fn curl_config(url: &str, headers: &[(&str, String)], body: &str) -> String {
    let mut s = format!("url = {}\nrequest = \"POST\"\n", quote(url));
    for (k, v) in headers {
        s.push_str(&format!("header = {}\n", quote(&format!("{k}: {v}"))));
    }
    s.push_str(&format!("data = {}\n", quote(body)));
    s
}

/// POST once; whether the sink accepted it (curl's `-f` turns an HTTP error into a failure).
fn post(config: &str) -> bool {
    let child = Command::new("curl")
        .args(["-fsS", "-m", "10", "--config", "-", "-o", "/dev/null"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(config.as_bytes());
    }
    child.wait().is_ok_and(|status| status.success())
}

/// Keep trying `attempt` after the first try failed, waiting `delays[i]` before try `i + 1`.
/// Returns how many retries it took, or `None` if every one failed.
fn retry(delays: &[Duration], mut attempt: impl FnMut() -> bool) -> Option<usize> {
    for (i, wait) in delays.iter().enumerate() {
        std::thread::sleep(*wait);
        if attempt() {
            return Some(i + 1);
        }
    }
    None
}

/// Deliver one notification to one sink on its own thread, so a sink that is down never holds up
/// the other. At least once: a timeout after the sink did accept it can mean it arrives twice.
fn deliver(sink: &'static str, delays: Vec<Duration>, config: String) {
    std::thread::spawn(move || {
        if post(&config) {
            return;
        }
        if RETRYING.fetch_add(1, Ordering::SeqCst) >= MAX_RETRYING {
            RETRYING.fetch_sub(1, Ordering::SeqCst);
            DROPPED.fetch_add(1, Ordering::Relaxed);
            eprintln!(
                "teryon: {sink} notification dropped: too many are already waiting to be retried"
            );
            return;
        }
        let outcome = retry(&delays, || post(&config));
        RETRYING.fetch_sub(1, Ordering::SeqCst);
        if outcome.is_none() {
            DROPPED.fetch_add(1, Ordering::Relaxed);
            eprintln!(
                "teryon: {sink} notification dropped after {} retries",
                delays.len()
            );
        }
    });
}

/// The JSON a webhook receives.
pub fn webhook_body(note: &Note) -> String {
    serde_json::json!({
        "title": note.title,
        "message": note.message,
        "event": note.event,
        "workspace": note.workspace,
        "pane": note.pane,
    })
    .to_string()
}

/// Send `note` to every configured sink in the background. A sink that refuses it or cannot be
/// reached is tried again for about 45 minutes (see `RETRY_DELAYS`) before it is dropped.
pub fn send(cfg: &NotifyConfig, note: Note) {
    send_with(cfg, note, &RETRY_DELAYS);
}

fn send_with(cfg: &NotifyConfig, note: Note, delays: &[Duration]) {
    if cfg.macos {
        let message = note.message.clone();
        std::thread::spawn(move || crate::util::notify(&message));
    }
    if let Some(url) = &cfg.ntfy_url {
        let headers = [
            ("Title", note.title.clone()),
            ("Tags", "computer".to_string()),
        ];
        deliver(
            "ntfy",
            delays.to_vec(),
            curl_config(url, &headers, &note.message),
        );
    }
    if let Some(url) = &cfg.webhook_url {
        let headers = [("Content-Type", "application/json".to_string())];
        deliver(
            "webhook",
            delays.to_vec(),
            curl_config(url, &headers, &webhook_body(&note)),
        );
    }
}

/// Send `note` to the ntfy and webhook sinks only; the desktop app shows its own macOS notification.
pub fn send_remote(cfg: &NotifyConfig, note: Note) {
    if cfg.ntfy_url.is_none() && cfg.webhook_url.is_none() {
        return;
    }
    send(
        &NotifyConfig {
            macos: false,
            ..cfg.clone()
        },
        note,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_is_flattened_and_capped() {
        assert_eq!(clean("a\nb\t\x1b[31mc  d", 50), "a b [31mc d");
        assert_eq!(clean("abcdef", 3), "abc");
    }
    #[test]
    fn curl_config_escapes_quotes_and_keeps_secrets_out_of_argv() {
        let c = curl_config(
            "https://ntfy.sh/secret",
            &[("Title", "T".into())],
            r#"say "hi" \o/"#,
        );
        assert!(c.contains("url = \"https://ntfy.sh/secret\""));
        assert!(c.contains("header = \"Title: T\""));
        assert!(c.contains(r#"data = "say \"hi\" \\o/""#));
    }
    #[test]
    fn curl_config_cannot_be_injected_through_line_breaks() {
        let c = curl_config(
            "https://x/\noutput = \"/tmp/pwn\"",
            &[("Title", "a\r\nurl = \"http://evil\"".into())],
            "m\nm",
        );
        assert_eq!(c.lines().count(), 4, "{c}");
        assert!(c
            .lines()
            .all(|l| ["url = ", "request = ", "header = ", "data = "]
                .iter()
                .any(|p| l.starts_with(p))));
    }
    #[test]
    fn a_webhook_receives_the_note_over_http() {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/hook", listener.local_addr().unwrap());
        let cfg = NotifyConfig {
            webhook_url: Some(url),
            ..NotifyConfig::default()
        };
        send(
            &cfg,
            Note {
                event: "agent_done",
                title: "TERYON".into(),
                message: "Claude finished in w (pane 1)".into(),
                workspace: "w".into(),
                pane: 1,
            },
        );
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0u8; 4096];
        let mut got = String::new();
        while !got.contains("pane 1") {
            let n = s.read(&mut buf).unwrap();
            assert!(n > 0, "connection closed early: {got}");
            got.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        assert!(got.starts_with("POST /hook"), "{got}");
        assert!(got.contains("\"event\":\"agent_done\""), "{got}");
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    }
    /// Answer `statuses.len()` requests on `listener` with those HTTP statuses, reading each request
    /// completely first (curl objects if the server answers before the body is read).
    fn serve(
        listener: std::net::TcpListener,
        statuses: Vec<u16>,
    ) -> std::thread::JoinHandle<usize> {
        use std::io::Read;
        std::thread::spawn(move || {
            let mut served = 0;
            for status in statuses {
                let (mut s, _) = listener.accept().unwrap();
                s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut got = Vec::new();
                let mut buf = [0u8; 4096];
                let (head_end, length) = loop {
                    let n = s.read(&mut buf).unwrap();
                    assert!(n > 0, "closed early");
                    got.extend_from_slice(&buf[..n]);
                    if let Some(at) = got.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&got[..at]).to_lowercase();
                        let length = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break (at + 4, length);
                    }
                };
                while got.len() < head_end + length {
                    let n = s.read(&mut buf).unwrap();
                    assert!(n > 0, "closed early");
                    got.extend_from_slice(&buf[..n]);
                }
                let _ = s.write_all(
                    format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                );
                served += 1;
            }
            served
        })
    }
    fn note() -> Note {
        Note {
            event: "agent_needs_you",
            title: "TERYON".into(),
            message: "Claude needs you in w (pane 1)".into(),
            workspace: "w".into(),
            pane: 1,
        }
    }
    fn within(secs: u64, handle: std::thread::JoinHandle<usize>) -> usize {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(handle.join().unwrap());
        });
        rx.recv_timeout(Duration::from_secs(secs))
            .expect("the notification was never retried")
    }

    #[test]
    fn retry_stops_at_the_first_success() {
        let mut calls = 0;
        let delays = [Duration::ZERO; 4];
        let out = retry(&delays, || {
            calls += 1;
            calls == 3
        });
        assert_eq!((out, calls), (Some(3), 3));
    }
    #[test]
    fn retry_gives_up_after_the_last_delay() {
        let mut calls = 0;
        let out = retry(&[Duration::ZERO; 3], || {
            calls += 1;
            false
        });
        assert_eq!((out, calls), (None, 3));
        assert_eq!(retry(&[], || false), None);
    }
    #[test]
    fn a_refused_notification_is_delivered_on_a_later_attempt() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let cfg = NotifyConfig {
            webhook_url: Some(format!("http://{}/hook", listener.local_addr().unwrap())),
            ..NotifyConfig::default()
        };
        let server = serve(listener, vec![500, 502, 200]);
        send_with(&cfg, note(), &[Duration::from_millis(20); 4]);
        assert_eq!(
            within(15, server),
            3,
            "refused twice, accepted on the third try"
        );
    }
    #[test]
    fn a_sink_that_accepted_the_notification_is_not_sent_it_again() {
        let ok = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let flaky = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let cfg = NotifyConfig {
            ntfy_url: Some(format!("http://{}/topic", ok.local_addr().unwrap())),
            webhook_url: Some(format!("http://{}/hook", flaky.local_addr().unwrap())),
            ..NotifyConfig::default()
        };
        ok.set_nonblocking(false).unwrap();
        let ok_server = serve(ok.try_clone().unwrap(), vec![200]);
        let flaky_server = serve(flaky, vec![500, 200]);
        send_with(&cfg, note(), &[Duration::from_millis(20); 4]);
        assert_eq!(within(15, flaky_server), 2);
        assert_eq!(within(5, ok_server), 1);
        // Nothing more reaches the sink that already accepted it.
        ok.set_nonblocking(true).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(ok.accept().is_err(), "the ntfy sink was sent a duplicate");
    }
    #[test]
    fn webhook_gets_structured_json() {
        let n = Note {
            event: "agent_needs_you",
            title: "TERYON".into(),
            message: "Claude needs you".into(),
            workspace: "w".into(),
            pane: 2,
        };
        let v: serde_json::Value = serde_json::from_str(&webhook_body(&n)).unwrap();
        assert_eq!(v["event"], "agent_needs_you");
        assert_eq!(v["pane"], 2);
    }
}
