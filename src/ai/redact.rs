//! Secret redaction for everything that goes from a terminal to a model.
use regex::Regex;
use std::sync::OnceLock;

struct Rule {
    kind: &'static str,
    re: Regex,
    /// Replace only this capture group (so `NAME=` stays readable); 0 replaces the whole match.
    group: usize,
}

fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let r = |kind, pat: &str, group| Rule { kind, re: Regex::new(pat).expect("valid pattern"), group };
        vec![
            r("private-key", r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?(-----END [A-Z ]*PRIVATE KEY-----|\z)", 0),
            r("anthropic-key", r"\bsk-ant-[A-Za-z0-9_\-]{10,}", 0),
            r("openai-key", r"\bsk-(?:proj-|live-|test-)?[A-Za-z0-9_\-]{20,}", 0),
            r("github-token", r"\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,})", 0),
            r("slack-token", r"\bxox[abprs]-[A-Za-z0-9\-]{10,}", 0),
            r("aws-key", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", 0),
            r("google-key", r"\bAIza[0-9A-Za-z_\-]{30,}", 0),
            r("jwt", r"\beyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}", 0),
            r("bearer-token", r"(?i)\bBearer\s+([A-Za-z0-9._~+/\-]{16,}=*)", 1),
            // NAME=value / NAME: value where NAME names a secret.
            r(
                "secret-value",
                r#"(?i)\b[A-Z0-9_\-]*(?:API[_\-]?KEY|SECRET|TOKEN|PASSWD|PASSWORD|PRIVATE[_\-]?KEY|CREDENTIALS?)[A-Z0-9_\-]*\s*[=:]\s*["']?([^\s"']{6,})"#,
                1,
            ),
        ]
    })
}

/// Replace secrets in `text` with `[redacted:kind]`; returns the clean text and how many were hidden.
pub fn redact(text: &str) -> (String, usize) {
    let mut out = text.to_string();
    let mut count = 0;
    for rule in rules() {
        let mut next = String::with_capacity(out.len());
        let mut last = 0;
        for caps in rule.re.captures_iter(&out) {
            let Some(m) = caps.get(rule.group) else {
                continue;
            };
            if m.as_str().starts_with("[redacted:") {
                continue;
            }
            next.push_str(&out[last..m.start()]);
            next.push_str(&format!("[redacted:{}]", rule.kind));
            last = m.end();
            count += 1;
        }
        next.push_str(&out[last..]);
        out = next;
    }
    (out, count)
}

/// Redact and, when something was hidden, say so (the model and the user both see the note).
pub fn clean(text: &str) -> String {
    let (out, n) = redact(text);
    if n == 0 {
        out
    } else {
        format!(
            "{out}\n[{n} secret{} hidden]",
            if n == 1 { "" } else { "s" }
        )
    }
}

/// Frame terminal-derived text as data so instructions inside it are not followed.
pub fn untrusted(text: &str) -> String {
    frame(&clean(text))
}
/// Wrap already-cleaned text in the untrusted-data tags.
pub fn frame(text: &str) -> String {
    format!("<terminal_output untrusted=\"true\">\n{text}\n</terminal_output>")
}
/// Tools whose output comes from a terminal or a command and must be framed as untrusted.
pub fn is_terminal_tool(name: &str) -> bool {
    matches!(name, "read_pane" | "list_panes" | "run_command")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake secrets are assembled at runtime so no real-looking key is committed.
    fn fake(prefix: &str, n: usize) -> String {
        format!("{prefix}{}", "aB3_".repeat(n))
    }

    #[test]
    fn every_key_shape_is_hidden() {
        let cases = [
            ("anthropic-key", fake("sk-ant-usr-", 8)),
            ("openai-key", fake("sk-proj-", 8)),
            ("github-token", format!("ghp_{}", "aB3d".repeat(10))),
            ("slack-token", format!("xoxb-{}", "1234-abcd".repeat(4))),
            ("aws-key", format!("AKIA{}", "ABCDEFGHIJKLMNOP")),
            ("google-key", fake("AIza", 10)),
            (
                "jwt",
                format!("eyJ{}.{}.{}", "a1B2c3D4e5", "f6G7h8I9j0", "k1L2m3N4o5"),
            ),
        ];
        for (kind, secret) in cases {
            let (out, n) = redact(&format!("value is {secret} ok"));
            assert_eq!(n, 1, "{kind}");
            assert!(out.contains(&format!("[redacted:{kind}]")), "{kind}: {out}");
            assert!(!out.contains(&secret), "{kind} leaked");
        }
    }

    #[test]
    fn named_secrets_and_headers_keep_their_names() {
        let (out, n) = redact("export OPENAI_API_KEY=\"hunter2hunter2\"\nAuthorization: Bearer abcdefghijklmnop12345\nDB_PASSWORD: s3cretpass");
        assert_eq!(n, 3);
        assert!(out.contains("OPENAI_API_KEY=\"[redacted:secret-value]"));
        assert!(out.contains("Bearer [redacted:bearer-token]"));
        assert!(out.contains("DB_PASSWORD: [redacted:secret-value]"));
        assert!(!out.contains("hunter2") && !out.contains("s3cretpass"));
    }

    #[test]
    fn private_key_blocks_are_removed_even_when_cut_short() {
        let pem = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk\nAAAA\n-----END OPENSSH PRIVATE KEY-----";
        let (out, n) = redact(&format!("a\n{pem}\nb"));
        assert_eq!(n, 1);
        assert_eq!(out, "a\n[redacted:private-key]\nb");
        let (out, _) = redact("x\n-----BEGIN PRIVATE KEY-----\nMIIE");
        assert!(!out.contains("MIIE"));
    }

    #[test]
    fn ordinary_text_is_untouched() {
        for s in [
            "ls -la /usr/local/bin\ncargo test --all-features",
            "the token count is 5 and key = 3",
            "git commit -m \"fix password reset page\"",
            "sk-learn is a library; sk-1 is short",
            "export PATH=/usr/bin:$PATH",
        ] {
            assert_eq!(redact(s), (s.to_string(), 0), "{s}");
        }
    }

    #[test]
    fn clean_notes_the_count_and_untrusted_wraps_it() {
        let key = fake("sk-ant-api-", 8);
        let out = clean(&format!("a {key} b {key}"));
        assert!(out.ends_with("[2 secrets hidden]") && !out.contains(&key));
        let wrapped = untrusted("hello");
        assert!(
            wrapped.starts_with("<terminal_output untrusted=\"true\">")
                && wrapped.contains("hello")
        );
    }
}
