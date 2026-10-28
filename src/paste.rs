//! The paste guard behind the zsh integration: a paste that would run several commands at once, or
//! that carries a secret, is held until you confirm it.

/// Why `text` deserves a second look before it goes onto a command line, or `None` when it is fine.
/// One trailing newline is the common "copied a command" case and is not counted.
pub fn risk(text: &str) -> Option<String> {
    let body = text.trim_end_matches(['\n', '\r']);
    let lines = body.lines().filter(|l| !l.trim().is_empty()).count();
    let mut reasons = Vec::new();
    if lines > 1 {
        reasons.push(format!("{lines} lines, which run one after the other"));
    }
    if crate::history::has_secret(body) {
        reasons.push("something that looks like a secret".to_string());
    }
    (!reasons.is_empty()).then(|| format!("contains {}", reasons.join(" and ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_command_is_fine_with_or_without_its_trailing_newline() {
        assert_eq!(risk("ls -la"), None);
        assert_eq!(risk("ls -la\n"), None);
        assert_eq!(risk("echo a\n\n\n"), None);
    }

    #[test]
    fn several_lines_are_reported_with_their_count() {
        assert_eq!(
            risk("cd /tmp\nrm -rf build\nmake").as_deref(),
            Some("contains 3 lines, which run one after the other")
        );
    }

    #[test]
    fn a_secret_is_reported_even_on_one_line() {
        let text = risk("curl -H 'Authorization: Bearer abcdefghijklmnopqrstuvwxyz012345' x");
        assert!(text.unwrap().contains("looks like a secret"));
    }
}
