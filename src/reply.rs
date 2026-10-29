//! Answering an agent's approval prompt from a phone. The "needs you" push carries Approve and Deny
//! buttons; each posts a one-time token to a reply topic that the daemon listens on, so the daemon
//! only ever makes outbound connections. A token answers one prompt in one pane, once, and only
//! while that pane is still asking.
use crate::agents::Kind;
use crate::util::{nonce, LockRecover};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a button stays valid.
pub const TOKEN_LIFE: Duration = Duration::from_secs(30 * 60);
/// Tokens held at once; the oldest goes first.
const MAX_TOKENS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Approve,
    Deny,
}
impl Answer {
    pub fn word(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Deny => "deny",
        }
    }
    /// The keystrokes this answer types into the agent's prompt.
    pub fn keys(self, kind: Kind) -> &'static [u8] {
        match self {
            Self::Approve => kind.approve_keys(),
            Self::Deny => kind.deny_keys(),
        }
    }
}

/// What came of a tap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The keys were typed into the pane.
    Sent,
    /// The pane no longer shows a prompt (answered elsewhere, or the agent moved on).
    NotAsking,
    /// The pane or its workspace is gone.
    Gone,
}

/// The ntfy title and body that tell the phone what its tap did. Titles are ASCII; the place is in the body.
pub fn confirmation(answer: Answer, entry: &Entry, outcome: Outcome) -> (&'static str, String) {
    let place = format!("{} · pane {}", entry.workspace, entry.pane);
    match (outcome, answer) {
        (Outcome::Sent, Answer::Approve) => ("Approved", place),
        (Outcome::Sent, Answer::Deny) => ("Denied", place),
        (Outcome::NotAsking, _) => (
            "Not sent",
            format!("{place}: the agent is no longer asking, nothing was typed"),
        ),
        (Outcome::Gone, _) => ("Not sent", format!("{place}: that pane is gone")),
    }
}

/// What one token is allowed to answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub workspace: String,
    pub pane: u64,
    pub kind: Kind,
    issued: Instant,
}

#[derive(Default)]
pub struct Registry {
    map: HashMap<String, Entry>,
}
impl Registry {
    pub fn issue(&mut self, token: String, workspace: &str, pane: u64, kind: Kind, now: Instant) {
        self.map
            .retain(|_, e| now.duration_since(e.issued) < TOKEN_LIFE);
        // One live prompt per pane: a newer prompt replaces the buttons of the older one.
        self.map
            .retain(|_, e| !(e.workspace == workspace && e.pane == pane));
        if self.map.len() >= MAX_TOKENS {
            if let Some(oldest) = self
                .map
                .iter()
                .min_by_key(|(_, e)| e.issued)
                .map(|(k, _)| k.clone())
            {
                self.map.remove(&oldest);
            }
        }
        self.map.insert(
            token,
            Entry {
                workspace: workspace.into(),
                pane,
                kind,
                issued: now,
            },
        );
    }
    /// Use up a token. `None` when it is unknown, already used, or too old.
    pub fn take(&mut self, token: &str, now: Instant) -> Option<Entry> {
        let entry = self.map.remove(token)?;
        (now.duration_since(entry.issued) < TOKEN_LIFE).then_some(entry)
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

static REGISTRY: Mutex<Option<Registry>> = Mutex::new(None);

/// A fresh token for the prompt in `workspace` pane `pane`.
pub fn issue(workspace: &str, pane: u64, kind: Kind) -> Option<String> {
    let token = nonce().ok()?;
    REGISTRY
        .lock_recover()
        .get_or_insert_with(Registry::default)
        .issue(token.clone(), workspace, pane, kind, Instant::now());
    Some(token)
}

/// Use up the token in a reply message.
pub fn redeem(message: &str) -> Option<(Answer, Entry)> {
    let (answer, token) = parse(message)?;
    let entry = REGISTRY
        .lock_recover()
        .get_or_insert_with(Registry::default)
        .take(&token, Instant::now())?;
    Some((answer, entry))
}

/// `approve:<token>` or `deny:<token>`, nothing else.
pub fn parse(message: &str) -> Option<(Answer, String)> {
    let (word, token) = message.trim().split_once(':')?;
    let answer = match word {
        "approve" => Answer::Approve,
        "deny" => Answer::Deny,
        _ => return None,
    };
    let ok = token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit());
    ok.then(|| (answer, token.to_string()))
}

/// The ntfy `Actions` header: an Approve and a Deny button that post to `reply_url`.
pub fn actions_header(reply_url: &str, token: &str) -> String {
    [Answer::Approve, Answer::Deny]
        .iter()
        .map(|a| {
            let label = match a {
                Answer::Approve => "Approve",
                Answer::Deny => "Deny",
            };
            format!(
                "http, {label}, {reply_url}, method=POST, body={}:{token}, clear=true",
                a.word()
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// The `message` of one line of ntfy's `/json` stream, if it is a message event.
pub fn stream_message(line: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    (v.get("event")?.as_str()? == "message").then_some(())?;
    Some(v.get("message")?.as_str()?.to_string())
}

static TOPIC: Mutex<Option<String>> = Mutex::new(None);
static STARTED: std::sync::Once = std::sync::Once::new();

/// Listen on `topic` (or stop, with `None`). The first call starts the listener thread, which hands
/// every valid, unused answer to `on_answer`. A topic change or a stop is noticed within a stream's
/// lifetime (`STREAM_SECS`).
pub fn listen(topic: Option<String>, on_answer: impl Fn(Answer, Entry) + Send + 'static) {
    *TOPIC.lock_recover() = topic;
    STARTED.call_once(|| {
        std::thread::spawn(move || loop {
            let topic = TOPIC.lock_recover().clone();
            let Some(topic) = topic else {
                std::thread::sleep(Duration::from_secs(5));
                continue;
            };
            stream(&topic, &on_answer);
            std::thread::sleep(Duration::from_secs(3));
        });
    });
}

const STREAM_SECS: &str = "300";

/// One connection to the topic's JSON stream; returns when it ends. `since=` replays the last
/// minute so a reconnect never loses a tap (replays are harmless: a token works once).
fn stream(topic: &str, on_answer: &impl Fn(Answer, Entry)) {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};
    let url = format!("{}/json?since=1m", topic.trim_end_matches('/'));
    let Ok(mut child) = Command::new("curl")
        .args(["-fsSN", "-m", STREAM_SECS, "--config", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(format!("url = \"{url}\"\n").as_bytes());
    }
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some((answer, entry)) = stream_message(&line).and_then(|m| redeem(&m)) {
                on_answer(answer, entry);
            }
        }
    }
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn only_a_word_and_a_hex_token_parse() {
        assert_eq!(
            parse(&format!("approve:{TOKEN}")),
            Some((Answer::Approve, TOKEN.into()))
        );
        assert_eq!(
            parse(&format!(" deny:{TOKEN}\n")),
            Some((Answer::Deny, TOKEN.into()))
        );
        for bad in [
            "",
            "approve",
            "approve:",
            "approve:short",
            "yes:0123456789abcdef0123456789abcdef",
            "approve:0123456789abcdef0123456789abcdeg",
            "approve:0123456789abcdef0123456789abcdef0",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_token_answers_once() {
        let now = Instant::now();
        let mut r = Registry::default();
        r.issue(TOKEN.into(), "w", 2, Kind::Claude, now);
        let e = r.take(TOKEN, now).unwrap();
        assert_eq!((e.workspace.as_str(), e.pane), ("w", 2));
        assert_eq!(r.take(TOKEN, now), None, "a second use must fail");
        assert_eq!(r.take("nope", now), None);
    }

    #[test]
    fn a_token_expires() {
        let now = Instant::now();
        let mut r = Registry::default();
        r.issue(TOKEN.into(), "w", 1, Kind::Claude, now);
        assert_eq!(r.take(TOKEN, now + TOKEN_LIFE), None);
        assert!(r.is_empty(), "an expired token is still spent");
    }

    #[test]
    fn a_newer_prompt_in_a_pane_replaces_the_older_buttons() {
        let now = Instant::now();
        let mut r = Registry::default();
        r.issue("a".repeat(32), "w", 1, Kind::Claude, now);
        r.issue("b".repeat(32), "w", 1, Kind::Claude, now);
        r.issue("c".repeat(32), "w", 2, Kind::Claude, now);
        assert_eq!(r.len(), 2);
        assert_eq!(r.take(&"a".repeat(32), now), None);
        assert!(r.take(&"b".repeat(32), now).is_some());
    }

    #[test]
    fn the_registry_is_bounded() {
        let now = Instant::now();
        let mut r = Registry::default();
        for i in 0..200u64 {
            r.issue(format!("{i:032x}"), "w", i, Kind::Claude, now);
        }
        assert_eq!(r.len(), MAX_TOKENS);
    }

    #[test]
    fn answers_type_the_agents_own_keys() {
        assert_eq!(Answer::Approve.keys(Kind::Claude), b"\r");
        assert_eq!(Answer::Deny.keys(Kind::Claude), b"\x1b");
        assert_eq!(Answer::Approve.keys(Kind::Aider), b"y\r");
        assert_eq!(Answer::Deny.keys(Kind::Aider), b"n\r");
    }

    #[test]
    fn the_actions_header_has_two_buttons_for_the_reply_topic() {
        let h = actions_header("https://ntfy.sh/t-reply", TOKEN);
        assert!(h.is_ascii());
        let buttons: Vec<&str> = h.split("; ").collect();
        assert_eq!(buttons.len(), 2);
        assert!(buttons[0].starts_with("http, Approve, https://ntfy.sh/t-reply, method=POST"));
        assert!(buttons[0].contains(&format!("body=approve:{TOKEN}")));
        assert!(buttons[1].contains(&format!("body=deny:{TOKEN}")));
    }

    #[test]
    fn the_phone_is_told_what_its_tap_did() {
        let entry = Entry {
            workspace: "demo".into(),
            pane: 2,
            kind: Kind::Claude,
            issued: Instant::now(),
        };
        let say = |a, o| confirmation(a, &entry, o);
        assert_eq!(
            say(Answer::Approve, Outcome::Sent),
            ("Approved", "demo · pane 2".into())
        );
        assert_eq!(say(Answer::Deny, Outcome::Sent).0, "Denied");
        let (title, body) = say(Answer::Approve, Outcome::NotAsking);
        assert_eq!(title, "Not sent");
        assert!(body.contains("no longer asking"), "{body}");
        assert!(say(Answer::Deny, Outcome::Gone).1.contains("gone"));
    }

    #[test]
    fn only_message_events_of_the_stream_count() {
        assert_eq!(
            stream_message(r#"{"id":"x","event":"message","topic":"t","message":"approve:abc"}"#),
            Some("approve:abc".into())
        );
        assert_eq!(stream_message(r#"{"event":"open","topic":"t"}"#), None);
        assert_eq!(stream_message(r#"{"event":"keepalive"}"#), None);
        assert_eq!(stream_message("garbage"), None);
    }
}
