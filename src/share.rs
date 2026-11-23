//! Read-only shares: the `authorized_keys` lines that let a teammate's key watch one workspace
//! (`teryon share`) and finding them again (`teryon shares`).
use std::path::PathBuf;

/// What a key's forced command must start with for TERYON to call the line a share.
const MARKER: &str = " relay --view ";

/// The line to put in `authorized_keys` so `key` can only watch `workspace`, read-only.
/// `teryon` is the program's path on this machine. `restrict` switches off forwarding, terminals and
/// everything else the key could otherwise do.
pub fn authorized_line(teryon: &str, workspace: &str, key: &str) -> String {
    format!("command=\"{teryon}{MARKER}{workspace}\",restrict {key}")
}

/// One share found in an `authorized_keys` file.
#[derive(Debug, PartialEq, Eq)]
pub struct Share {
    /// 1-based line number, to find the line again.
    pub line: usize,
    pub workspace: String,
    /// `ssh-ed25519`, `ssh-rsa`, ...
    pub key_type: String,
    /// The comment after the key (usually who it belongs to).
    pub comment: String,
}

/// The shares in an `authorized_keys` file's text: lines whose forced command is
/// `... relay --view WORKSPACE` and that carry `restrict`. A line with that command but without
/// `restrict` is not listed as a share, because it would allow more than watching.
pub fn find_shares(text: &str) -> Vec<Share> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix("command=\"") else {
            continue;
        };
        let Some((command, after)) = rest.split_once("\",") else {
            continue;
        };
        let Some((_, workspace)) = command.split_once(MARKER) else {
            continue;
        };
        let mut fields = after.split_whitespace();
        let options = fields.next().unwrap_or("");
        if !options.split(',').any(|o| o == "restrict") || workspace.is_empty() {
            continue;
        }
        let key_type = fields.next().unwrap_or("").to_string();
        let _blob = fields.next();
        out.push(Share {
            line: i + 1,
            workspace: workspace.to_string(),
            key_type,
            comment: fields.collect::<Vec<_>>().join(" "),
        });
    }
    out
}

/// `~/.ssh/authorized_keys`.
pub fn default_path() -> Option<PathBuf> {
    crate::util::home()
        .ok()
        .map(|h| h.join(".ssh/authorized_keys"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_line_pins_one_workspace_and_restricts_everything_else() {
        let l = authorized_line(
            "/h/.local/bin/teryon",
            "demo",
            "ssh-ed25519 AAAA bob@laptop",
        );
        assert_eq!(
            l,
            "command=\"/h/.local/bin/teryon relay --view demo\",restrict ssh-ed25519 AAAA bob@laptop"
        );
    }
    #[test]
    fn shares_are_found_among_other_keys_and_a_made_line_is_found_again() {
        let mine = authorized_line(
            "$HOME/.local/bin/teryon",
            "build",
            "ssh-ed25519 AAAAC3 bob@laptop",
        );
        let text = format!(
            "# keys\nssh-rsa AAAAB3 me@mini\n\n{mine}\ncommand=\"/usr/bin/other\",restrict ssh-ed25519 AAAA x\n"
        );
        let found = find_shares(&text);
        assert_eq!(
            found,
            vec![Share {
                line: 4,
                workspace: "build".into(),
                key_type: "ssh-ed25519".into(),
                comment: "bob@laptop".into()
            }]
        );
    }
    #[test]
    fn a_view_command_without_restrict_is_not_called_a_share() {
        let text = "command=\"/x/teryon relay --view demo\",no-pty ssh-ed25519 AAAA eve\n";
        assert!(find_shares(text).is_empty());
        assert!(
            find_shares("command=\"/x/teryon relay --view \",restrict ssh-ed25519 A c").is_empty()
        );
        assert!(find_shares("garbage\ncommand=\"unterminated").is_empty());
    }
}
