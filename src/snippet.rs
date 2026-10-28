//! Saved commands with `{{placeholders}}` (`[[desktop.snippets]]`).
use crate::config::Snippet;
use crate::util;
use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;

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

/// Where `teryon snip add` keeps snippets (the config file's own `[[desktop.snippets]]` stay read-only).
pub fn store_path() -> Result<PathBuf> {
    Ok(util::home()?.join(".teryon/snippets.jsonl"))
}
pub fn load_saved() -> Vec<Snippet> {
    store_path()
        .and_then(|p| Ok(std::fs::read_to_string(p)?))
        .map(|t| {
            t.lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        })
        .unwrap_or_default()
}
pub fn save_saved(snippets: &[Snippet]) -> Result<()> {
    let mut text = String::new();
    for sn in snippets {
        text.push_str(&serde_json::to_string(sn)?);
        text.push('\n');
    }
    util::write_private_atomic(&store_path()?, text.as_bytes())
}
/// The config's snippets and the saved ones together, by name; a saved one replaces a config one of the same name.
pub fn merged(config: &[Snippet], saved: Vec<Snippet>) -> Vec<Snippet> {
    let mut all: BTreeMap<String, Snippet> =
        config.iter().map(|s| (s.name.clone(), s.clone())).collect();
    for s in saved {
        all.insert(s.name.clone(), s);
    }
    all.into_values().collect()
}
/// Check a name and command for `snip add`.
pub fn check(name: &str, command: &str) -> Result<()> {
    if name.is_empty() || name.chars().count() > 60 || name.contains(['\n', '\t']) {
        bail!("a snippet name is 1 to 60 characters without tabs or newlines");
    }
    if command.trim().is_empty() || command.len() > 4000 {
        bail!("a snippet needs a command of up to 4000 characters");
    }
    validate(command)
}
/// `key=value` words into a map.
pub fn assignments(words: &[String]) -> Result<BTreeMap<String, String>> {
    words
        .iter()
        .map(|w| {
            let (k, v) = w
                .split_once('=')
                .with_context(|| format!("'{w}' is not KEY=VALUE"))?;
            if !valid_param(k) {
                bail!("'{k}' is not a placeholder name");
            }
            Ok((k.to_string(), v.to_string()))
        })
        .collect()
}
/// An answer for a placeholder that offers choices (hosts): the exact name, or the one choice it is a prefix of.
pub fn resolve_choice(answer: &str, choices: &[String]) -> String {
    let answer = answer.trim();
    if choices.iter().any(|c| c == answer) {
        return answer.to_string();
    }
    let mut hits = choices
        .iter()
        .filter(|c| !answer.is_empty() && c.starts_with(answer));
    match (hits.next(), hits.next()) {
        (Some(one), None) => one.clone(),
        _ => answer.to_string(),
    }
}
/// Ask for every placeholder of `command` that `values` lacks. `host` placeholders list `hosts` and
/// accept a prefix of one.
pub fn ask(
    command: &str,
    values: &mut BTreeMap<String, String>,
    hosts: &[String],
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> Result<()> {
    for name in placeholders(command) {
        if values.contains_key(&name) {
            continue;
        }
        let is_host = name == "host" && !hosts.is_empty();
        if is_host {
            let shown: Vec<&str> = hosts.iter().take(8).map(String::as_str).collect();
            write!(out, "{name} ({}): ", shown.join(", "))?;
        } else {
            write!(out, "{name}: ")?;
        }
        out.flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            bail!("no answer for {name}");
        }
        let line = line.trim_end_matches(['\n', '\r']);
        values.insert(
            name,
            if is_host {
                resolve_choice(line, hosts)
            } else {
                line.to_string()
            },
        );
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
    #[test]
    fn saved_snippets_override_config_ones_and_names_are_sorted() {
        let sn = |n: &str, c: &str| Snippet {
            name: n.into(),
            command: c.into(),
            description: None,
            run: false,
        };
        let all = merged(&[sn("b", "old"), sn("a", "x")], vec![sn("b", "new")]);
        assert_eq!(
            all.iter().map(|s| s.command.as_str()).collect::<Vec<_>>(),
            ["x", "new"]
        );
    }
    #[test]
    fn answers_fill_the_missing_placeholders_and_hosts_accept_a_prefix() {
        let hosts = vec!["mini".to_string(), "build".to_string()];
        let mut values = assignments(&["port=22".to_string()]).unwrap();
        let mut out = Vec::new();
        ask(
            "ssh {{host}} -p {{port}} {{extra}}",
            &mut values,
            &hosts,
            &mut std::io::Cursor::new("bu\n--v\n"),
            &mut out,
        )
        .unwrap();
        assert_eq!(
            fill("ssh {{host}} -p {{port}} {{extra}}", &values),
            "ssh build -p 22 --v"
        );
        let shown = String::from_utf8(out).unwrap();
        assert!(shown.contains("host (mini, build): ") && !shown.contains("port:"));
        assert!(ask(
            "{{x}}",
            &mut BTreeMap::new(),
            &[],
            &mut std::io::Cursor::new(""),
            &mut Vec::new()
        )
        .is_err());
        assert!(assignments(&["nokey".to_string()]).is_err());
        assert_eq!(resolve_choice("m", &["mini".into(), "mac".into()]), "m");
    }
    #[test]
    fn add_rejects_bad_names_and_commands() {
        assert!(check("deploy", "make {{env}}").is_ok());
        assert!(check("", "ls").is_err());
        assert!(check("x", "  ").is_err());
        assert!(check("x", "ls {{bad name}}").is_err());
    }
}
