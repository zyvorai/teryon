//! Saved commands with `{{placeholders}}` (`[[desktop.snippets]]`).
use anyhow::{bail, Result};
use std::collections::BTreeMap;

fn valid_param(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}
/// Placeholder names in order of first appearance, e.g. `ssh {{host}} -p {{port}}`.
pub fn placeholders(command: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = command;
    while let Some(open) = rest.find("{{") {
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            break;
        };
        let name = after[..close].trim();
        if valid_param(name) && !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
        rest = &after[close + 2..];
    }
    out
}
/// `command` with every placeholder replaced by its value (missing ones become empty).
pub fn fill(command: &str, values: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = command;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        match after.find("}}") {
            Some(close) if valid_param(after[..close].trim()) => {
                out.push_str(values.get(after[..close].trim()).map_or("", String::as_str));
                rest = &after[close + 2..];
            }
            _ => {
                out.push_str("{{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}
/// Reject unbalanced or badly named placeholders.
pub fn validate(command: &str) -> Result<()> {
    let mut rest = command;
    while let Some(open) = rest.find("{{") {
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            bail!("unclosed {{{{ in \"{command}\"");
        };
        let name = after[..close].trim();
        if !valid_param(name) {
            bail!("placeholder \"{{{{{name}}}}}\" must be letters, digits or _");
        }
        rest = &after[close + 2..];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn placeholders_are_found_once_in_order_and_filled() {
        let c = "ssh {{host}} -p {{ port }} # {{host}}";
        assert_eq!(placeholders(c), ["host", "port"]);
        let mut v = BTreeMap::new();
        v.insert("host".to_string(), "mini".to_string());
        v.insert("port".to_string(), "22".to_string());
        assert_eq!(fill(c, &v), "ssh mini -p 22 # mini");
        assert_eq!(fill("echo {{x}}", &BTreeMap::new()), "echo ");
        assert_eq!(fill("a {{ b c }} d", &BTreeMap::new()), "a {{ b c }} d");
        assert!(placeholders("no params").is_empty());
    }
    #[test]
    fn validation_catches_broken_placeholders() {
        assert!(validate("ls {{dir}}").is_ok());
        assert!(validate("ls {{dir").is_err());
        assert!(validate("ls {{bad name}}").is_err());
        assert!(validate("ls {{}}").is_err());
        assert!(validate("plain").is_ok());
    }
}
