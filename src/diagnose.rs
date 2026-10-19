//! Why a connection to a host failed or dropped, read from the error text, with the next step to take.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// This machine has no route to the host: Wi-Fi off, VPN down, a network change, or a DNS failure.
    Network,
    /// The host does not answer: asleep, powered off, or a firewall in the way.
    Unreachable,
    /// The host answers but nothing accepts SSH there.
    SshDown,
    /// SSH refused our key or agent.
    Auth,
    /// The host key is not the one on record.
    HostKey,
    /// There is no (usable) teryon on the host.
    NoDaemon,
    /// Client and daemon speak different protocol versions.
    Version,
    /// The link was up and went away (it dropped mid-session, or this machine slept).
    Dropped,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnosis {
    pub cause: Cause,
    pub headline: &'static str,
    /// What to do about it; may name `host`.
    pub action: String,
    /// Retrying by itself can fix this (false for things that need you to act).
    pub retryable: bool,
}

/// Classify `message` (an error chain, possibly with the last lines `ssh` printed) for `host`.
pub fn diagnose(message: &str, host: &str) -> Diagnosis {
    let m = message.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| m.contains(n));
    let (cause, headline, action, retryable) = if has(&[
        "host key verification failed",
        "remote host identification has changed",
    ]) {
        (
            Cause::HostKey,
            "The host key changed",
            format!("If you expect this (a reinstall), run `ssh-keygen -R {host}`; if not, do not connect."),
            false,
        )
    } else if has(&[
        "permission denied",
        "too many authentication failures",
        "no more authentication methods",
    ]) {
        (
            Cause::Auth,
            "SSH refused your key",
            format!("Check `ssh -o BatchMode=yes {host} true` in a terminal and that `ssh-add -l` lists your key."),
            false,
        )
    } else if has(&["protocol version mismatch"]) {
        (
            Cause::Version,
            "teryon versions differ",
            format!("Update the host with `teryon deploy {host} --binary PATH`, then relink."),
            false,
        )
    } else if has(&[
        "command not found",
        "no such file or directory",
        "daemon did not start",
        "exec: ",
        "unexpected handshake",
    ]) {
        (
            Cause::NoDaemon,
            "teryon is not usable on the host",
            format!("Install it with `teryon deploy {host} --binary PATH`, or check `teryon doctor {host}`."),
            false,
        )
    } else if has(&[
        "could not resolve hostname",
        "name or service not known",
        "nodename nor servname",
        "temporary failure in name resolution",
        "network is unreachable",
        "no route to host",
        "network is down",
    ]) {
        (
            Cause::Network,
            "This Mac is offline or cannot find the host",
            "Check Wi-Fi, VPN and DNS. It relinks by itself when the network is back.".into(),
            true,
        )
    } else if has(&["connection refused"]) {
        (
            Cause::SshDown,
            "The host is up but SSH is not answering",
            format!("Is Remote Login on, and sshd running? Try `ssh {host}` in a terminal."),
            true,
        )
    } else if has(&["timed out", "timeout", "operation timed out"]) {
        (
            Cause::Unreachable,
            "The host is not answering",
            format!("It may be asleep or off. If it has a `mac` in the config, `teryon wake {host}` sends a wake-on-LAN packet."),
            true,
        )
    } else if has(&[
        "resumed after",
        "connection closed",
        "broken pipe",
        "connection reset",
        "uncertain input discarded",
        "end of file",
        "eof",
    ]) {
        (
            Cause::Dropped,
            "The link dropped",
            "Usually Wi-Fi, or this Mac or the host slept. It relinks by itself; nothing typed meanwhile is replayed.".into(),
            true,
        )
    } else {
        (
            Cause::Unknown,
            "Cannot reach the host",
            format!("Run `teryon doctor {host}` for details."),
            true,
        )
    };
    Diagnosis {
        cause,
        headline,
        action,
        retryable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cause(msg: &str) -> Cause {
        diagnose(msg, "mini").cause
    }
    #[test]
    fn openssh_messages_map_to_their_cause() {
        let cases = [
            ("ssh: connect to host mini port 22: Connection refused", Cause::SshDown),
            ("ssh: connect to host mini port 22: Operation timed out", Cause::Unreachable),
            ("ssh: connect to host 10.0.0.9 port 22: No route to host", Cause::Network),
            ("ssh: Could not resolve hostname mini: nodename nor servname provided", Cause::Network),
            ("ssh: connect to host mini port 22: Network is unreachable", Cause::Network),
            ("user@mini: Permission denied (publickey).", Cause::Auth),
            (
                "@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @@@\nHost key verification failed.",
                Cause::HostKey,
            ),
            ("bash: /home/u/.local/bin/teryon: No such file or directory", Cause::NoDaemon),
            ("sh: teryon: command not found", Cause::NoDaemon),
            ("server: protocol version mismatch: client 5, daemon 4", Cause::Version),
            ("connection closed", Cause::Dropped),
            ("Broken pipe · uncertain input discarded", Cause::Dropped),
            ("resumed after 312 s · uncertain input discarded", Cause::Dropped),
            ("something odd", Cause::Unknown),
        ];
        for (msg, want) in cases {
            assert_eq!(cause(msg), want, "{msg}");
        }
    }
    #[test]
    fn specific_causes_win_over_the_generic_drop() {
        // ssh prints the reason, then the pipe closes: the reason must decide.
        assert_eq!(
            cause("connection closed: Permission denied (publickey)"),
            Cause::Auth
        );
        assert_eq!(
            cause("connection closed: ssh: connect to host mini port 22: Connection refused"),
            Cause::SshDown
        );
    }
    #[test]
    fn only_causes_that_fix_themselves_are_retryable_and_actions_name_the_host() {
        for (msg, retry) in [
            ("Permission denied", false),
            ("Host key verification failed", false),
            ("protocol version mismatch", false),
            ("No route to host", true),
            ("Operation timed out", true),
        ] {
            assert_eq!(diagnose(msg, "mini").retryable, retry, "{msg}");
        }
        assert!(diagnose("Permission denied", "mini")
            .action
            .contains("mini"));
    }
}
