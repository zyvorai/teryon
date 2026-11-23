// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-2.0

//! `teryon ssh-audit`: read an OpenSSH client config, resolve a host the way `ssh` does (first value
//! wins, patterns and negations in file order), follow its `ProxyJump` chain hop by hop, and report
//! what would surprise you: loops, injection-shaped host names, agent forwarding, weakened host-key
//! checking, local commands.
//!
//! It only reads text. It never connects, never reads a key file and never runs `ssh`. `Match` blocks
//! are not evaluated (they can run commands); their presence is reported so you can confirm with
//! `ssh -G HOST`. TERYON itself passes `StrictHostKeyChecking=yes` and `BatchMode=yes` on the command
//! line, which beat the config file; the audit says which config lines TERYON therefore overrides.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: u64 = 1_000_000;
const MAX_LINE: usize = 8_192;
const MAX_INCLUDE_DEPTH: usize = 5;
const MAX_FILES: usize = 64;
const MAX_HOPS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct Finding {
    pub severity: Severity,
    pub host: String,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct Block {
    /// `Host` patterns; `["*"]` for options before the first `Host` line.
    pub patterns: Vec<String>,
    /// A `Match` block: reported, never evaluated.
    pub is_match: bool,
    pub options: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub blocks: Vec<Block>,
    pub problems: Vec<String>,
    pub included: Vec<PathBuf>,
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        v[1..v.len() - 1].to_string()
    } else {
        v.to_string()
    }
}

/// Parse config text. Unknown keywords are kept (the audit only looks at the ones it knows).
pub fn parse(text: &str) -> Config {
    let mut cfg = Config::default();
    cfg.blocks.push(Block {
        patterns: vec!["*".into()],
        ..Default::default()
    });
    parse_into(text, &mut cfg);
    cfg
}

fn parse_into(text: &str, cfg: &mut Config) {
    for (n, raw) in text.lines().enumerate() {
        let n = n + 1;
        if raw.len() > MAX_LINE {
            cfg.problems
                .push(format!("line {n}: longer than {MAX_LINE} bytes, ignored"));
            continue;
        }
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.chars().any(|c| c == '\0') {
            cfg.problems
                .push(format!("line {n}: contains a NUL byte, ignored"));
            continue;
        }
        let split = line.find(|c: char| c.is_whitespace() || c == '=');
        let (key, value) = match split {
            Some(i) => (
                &line[..i],
                line[i..].trim_start_matches(|c: char| c.is_whitespace() || c == '='),
            ),
            None => (line, ""),
        };
        let key = key.to_ascii_lowercase();
        if key == "host" || key == "match" {
            let patterns: Vec<String> = value
                .split_whitespace()
                .map(unquote)
                .filter(|p| !p.is_empty())
                .collect();
            if patterns.is_empty() {
                cfg.problems
                    .push(format!("line {n}: `{key}` with no pattern"));
            }
            cfg.blocks.push(Block {
                patterns,
                is_match: key == "match",
                options: Vec::new(),
            });
            continue;
        }
        if value.is_empty() {
            cfg.problems.push(format!("line {n}: `{key}` has no value"));
            continue;
        }
        if let Some(block) = cfg.blocks.last_mut() {
            block.options.push((key, unquote(value)));
        }
    }
}

/// Read `path` and the files it `Include`s (relative paths are taken from `path`'s directory, as `ssh`
/// takes them from `~/.ssh`). Depth, file count and file size are capped; every file read is listed
/// in `included`.
pub fn load(path: &Path) -> std::io::Result<Config> {
    let mut cfg = Config::default();
    cfg.blocks.push(Block {
        patterns: vec!["*".into()],
        ..Default::default()
    });
    let base = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    load_file(path, &base, 0, &mut cfg)?;
    Ok(cfg)
}

fn load_file(path: &Path, base: &Path, depth: usize, cfg: &mut Config) -> std::io::Result<()> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > MAX_FILE_BYTES {
        cfg.problems.push(format!(
            "{}: larger than {MAX_FILE_BYTES} bytes, skipped",
            path.display()
        ));
        return Ok(());
    }
    let text = String::from_utf8_lossy(&std::fs::read(path)?).into_owned();
    cfg.included.push(path.to_path_buf());
    // Parse this file's lines; `Include` lines are expanded in place so file order is kept.
    let mut chunk = String::new();
    for line in text.lines() {
        let t = line.trim();
        let is_include = t.len() > 8
            && t[..7].eq_ignore_ascii_case("include")
            && t[7..].starts_with(|c: char| c.is_whitespace() || c == '=');
        if !is_include {
            chunk.push_str(line);
            chunk.push('\n');
            continue;
        }
        parse_into(&chunk, cfg);
        chunk.clear();
        if depth + 1 > MAX_INCLUDE_DEPTH {
            cfg.problems.push(format!(
                "{}: Include nested deeper than {MAX_INCLUDE_DEPTH}, skipped",
                path.display()
            ));
            continue;
        }
        let targets = t[7..].trim_start_matches(|c: char| c.is_whitespace() || c == '=');
        for target in targets.split_whitespace().map(unquote) {
            for file in expand_include(&target, base) {
                if cfg.included.len() >= MAX_FILES {
                    cfg.problems.push(format!(
                        "more than {MAX_FILES} files included, rest skipped"
                    ));
                    break;
                }
                if cfg.included.contains(&file) {
                    cfg.problems.push(format!(
                        "{}: included twice (loop?), skipped",
                        file.display()
                    ));
                    continue;
                }
                if let Err(e) = load_file(&file, base, depth + 1, cfg) {
                    cfg.problems
                        .push(format!("{}: cannot read ({e})", file.display()));
                }
            }
        }
    }
    parse_into(&chunk, cfg);
    Ok(())
}

/// A literal path, or a path whose last component has `*`/`?` (the only wildcard form supported).
fn expand_include(target: &str, base: &Path) -> Vec<PathBuf> {
    let p = if let Some(rest) = target.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(rest))
            .unwrap_or_else(|| base.join(rest))
    } else if Path::new(target).is_absolute() {
        PathBuf::from(target)
    } else {
        base.join(target)
    };
    let Some(name) = p.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
        return Vec::new();
    };
    if !name.contains(['*', '?']) {
        return vec![p];
    }
    let Some(dir) = p.parent() else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_type().is_ok_and(|t| t.is_file())
                        && e.file_name().to_str().is_some_and(|f| glob(&name, f))
                })
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

/// `*` and `?` glob match, as used in `Host` patterns. Iterative, so a hostile pattern cannot
/// recurse or take exponential time.
pub fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Whether a `Host` pattern list matches `name`: some positive pattern matches and no `!pattern` does.
pub fn host_matches(patterns: &[String], name: &str) -> bool {
    let mut hit = false;
    for p in patterns {
        if let Some(neg) = p.strip_prefix('!') {
            if glob(neg, name) {
                return false;
            }
        } else if glob(p, name) {
            hit = true;
        }
    }
    hit
}

/// The effective options for `alias`: the first value of each keyword wins, as in OpenSSH.
/// Repeatable keywords (`IdentityFile`, forwards) keep every value.
pub fn resolve(cfg: &Config, alias: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for b in cfg
        .blocks
        .iter()
        .filter(|b| !b.is_match && host_matches(&b.patterns, alias))
    {
        for (k, v) in &b.options {
            let repeatable = matches!(
                k.as_str(),
                "identityfile"
                    | "localforward"
                    | "remoteforward"
                    | "dynamicforward"
                    | "sendenv"
                    | "setenv"
            );
            let slot = out.entry(k.clone()).or_default();
            if repeatable || slot.is_empty() {
                slot.push(v.clone());
            }
        }
    }
    out
}

fn first<'a>(o: &'a BTreeMap<String, Vec<String>>, k: &str) -> Option<&'a str> {
    o.get(k).and_then(|v| v.first()).map(String::as_str)
}

/// One step of a connection: who we log in to, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hop {
    pub alias: String,
    pub hostname: String,
    pub user: Option<String>,
    pub port: Option<u16>,
}

fn parse_hop(spec: &str) -> (Option<String>, String, Option<String>) {
    let spec = spec.trim().strip_prefix("ssh://").unwrap_or(spec.trim());
    let (user, rest) = match spec.rsplit_once('@') {
        Some((u, r)) => (Some(u.to_string()), r),
        None => (None, spec),
    };
    // `[v6]:port`, `host:port` (a bare IPv6 address has several colons and no port).
    let (host, port) = if let Some(r) = rest.strip_prefix('[') {
        match r.split_once(']') {
            Some((h, p)) => (h.to_string(), p.strip_prefix(':').map(str::to_string)),
            None => (rest.to_string(), None),
        }
    } else if rest.matches(':').count() == 1 {
        let (h, p) = rest.split_once(':').unwrap_or((rest, ""));
        (h.to_string(), Some(p.to_string()))
    } else {
        (rest.to_string(), None)
    };
    (user, host, port)
}

/// Characters that make a host or user name dangerous when something downstream (a ProxyCommand's `%h`,
/// a shell, an `ssh` argument list) takes it literally.
pub fn unsafe_name(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return Some("is empty");
    }
    if name.starts_with('-') {
        return Some("starts with '-', so ssh would read it as an option");
    }
    if name.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Some("contains whitespace or a control character");
    }
    if name.chars().any(|c| "`$;&|<>()\\'\"{}*?!#~^".contains(c)) {
        return Some("contains shell metacharacters");
    }
    None
}

pub struct Audit {
    pub target: String,
    pub hops: Vec<Hop>,
    pub findings: Vec<Finding>,
}

impl Audit {
    pub fn has_errors(&self) -> bool {
        self.findings.iter().any(|f| f.severity == Severity::Error)
    }
}

fn find(out: &mut Vec<Finding>, severity: Severity, host: &str, message: impl Into<String>) {
    out.push(Finding {
        severity,
        host: host.to_string(),
        message: message.into(),
    });
}

fn audit_host(
    cfg: &Config,
    alias: &str,
    stack: &mut Vec<String>,
    hops: &mut Vec<Hop>,
    out: &mut Vec<Finding>,
) {
    let o = resolve(cfg, alias);
    let hostname = first(&o, "hostname").unwrap_or(alias).to_string();
    let user = first(&o, "user").map(str::to_string);
    let port = match first(&o, "port") {
        None => None,
        Some(p) => match p.parse::<u16>() {
            Ok(n) if n > 0 => Some(n),
            _ => {
                find(
                    out,
                    Severity::Error,
                    alias,
                    format!("Port {p:?} is not 1-65535"),
                );
                None
            }
        },
    };
    for (what, v) in [
        ("host name", Some(hostname.as_str())),
        ("user", user.as_deref()),
    ] {
        if let Some(v) = v {
            // `%h`-style tokens are expanded by ssh, not the shell; judge the rest.
            let probe = v.replace("%h", "h").replace("%n", "n").replace("%%", "p");
            if let Some(why) = unsafe_name(&probe) {
                find(out, Severity::Error, alias, format!("{what} {v:?} {why}"));
            }
        }
    }
    stack.push(alias.to_string());

    // Hops first (they are connected to first), in order.
    let jump = first(&o, "proxyjump").filter(|j| !j.eq_ignore_ascii_case("none"));
    let command = first(&o, "proxycommand").filter(|c| !c.eq_ignore_ascii_case("none"));
    if jump.is_some() && command.is_some() {
        find(out, Severity::Warn, alias, "both ProxyJump and ProxyCommand are set; ssh uses only one of them (the first in the file)");
    }
    if let Some(c) = command {
        find(
            out,
            Severity::Info,
            alias,
            format!(
                "ProxyCommand runs a local command ({}); it is not followed here",
                sanitize_for_report(c)
            ),
        );
    } else if let Some(j) = jump {
        for spec in j.split(',').filter(|s| !s.trim().is_empty()) {
            if stack.len() > MAX_HOPS {
                find(
                    out,
                    Severity::Error,
                    alias,
                    format!("ProxyJump chain deeper than {MAX_HOPS} hops"),
                );
                break;
            }
            let (juser, jhost, jport) = parse_hop(spec);
            if let Some(why) = unsafe_name(&jhost) {
                find(
                    out,
                    Severity::Error,
                    alias,
                    format!("jump host {jhost:?} {why}"),
                );
                continue;
            }
            if let Some(u) = &juser {
                if let Some(why) = unsafe_name(u) {
                    find(
                        out,
                        Severity::Error,
                        alias,
                        format!("jump user {u:?} {why}"),
                    );
                    continue;
                }
            }
            if let Some(p) = &jport {
                if p.parse::<u16>().map_or(true, |n| n == 0) {
                    find(
                        out,
                        Severity::Error,
                        alias,
                        format!("jump port {p:?} is not 1-65535"),
                    );
                    continue;
                }
            }
            if stack.iter().any(|s| s == &jhost) {
                find(
                    out,
                    Severity::Error,
                    alias,
                    format!("ProxyJump loop: {} -> {jhost}", stack.join(" -> ")),
                );
                continue;
            }
            let before = hops.len();
            audit_host(cfg, &jhost, stack, hops, out);
            // An explicit user/port in the jump spec overrides the config for that hop.
            let last = hops.len().saturating_sub(1);
            if let Some(h) = hops.get_mut(last).filter(|_| last >= before) {
                if juser.is_some() {
                    h.user = juser.clone();
                }
                if let Some(p) = jport.as_ref().and_then(|p| p.parse().ok()) {
                    h.port = Some(p);
                }
            }
        }
    }
    stack.pop();

    hops.push(Hop {
        alias: alias.to_string(),
        hostname,
        user,
        port,
    });

    // Per-host settings.
    if first(&o, "forwardagent").is_some_and(is_yes) {
        find(out, Severity::Warn, alias, "ForwardAgent yes: anyone with root on this host can use your agent's keys while you are connected; prefer ProxyJump and keep agent forwarding off");
    }
    if let Some(v) = first(&o, "stricthostkeychecking") {
        let lv = v.to_ascii_lowercase();
        if lv == "no" || lv == "off" {
            find(out, Severity::Error, alias, "StrictHostKeyChecking no accepts any host key (TERYON overrides it with yes, plain ssh does not)");
        } else if lv == "accept-new" {
            find(out, Severity::Warn, alias, "StrictHostKeyChecking accept-new trusts the first key it sees (TERYON overrides it with yes)");
        } else if lv == "ask" {
            find(out, Severity::Info, alias, "StrictHostKeyChecking ask prompts; TERYON runs ssh with BatchMode and requires yes");
        }
    }
    if first(&o, "userknownhostsfile").is_some_and(|v| v == "/dev/null") {
        find(out, Severity::Error, alias, "UserKnownHostsFile /dev/null forgets every host key, so a changed key is never noticed");
    }
    if first(&o, "verifyhostkeydns").is_some_and(is_yes) {
        find(
            out,
            Severity::Warn,
            alias,
            "VerifyHostKeyDNS yes trusts DNS for host keys; DNS is only as safe as its path",
        );
    }
    if first(&o, "permitlocalcommand").is_some_and(is_yes) {
        let lc = first(&o, "localcommand").unwrap_or("");
        find(
            out,
            Severity::Warn,
            alias,
            format!(
                "PermitLocalCommand yes runs LocalCommand on this machine at every connect ({})",
                sanitize_for_report(lc)
            ),
        );
    }
    if first(&o, "gatewayports")
        .is_some_and(|v| is_yes(v) || v.eq_ignore_ascii_case("clientspecified"))
    {
        find(
            out,
            Severity::Warn,
            alias,
            "GatewayPorts lets forwarded ports listen on every interface",
        );
    }
    for f in o
        .get("localforward")
        .into_iter()
        .flatten()
        .chain(o.get("dynamicforward").into_iter().flatten())
    {
        if f.starts_with("0.0.0.0:") || f.starts_with("*:") || f.starts_with("[::]:") {
            find(
                out,
                Severity::Warn,
                alias,
                format!(
                    "forward {} listens on every interface",
                    sanitize_for_report(f)
                ),
            );
        }
    }
    if let Some(cp) = first(&o, "controlpath") {
        if (cp.starts_with("/tmp/") || cp.starts_with("/var/tmp/"))
            && !cp.contains("%C")
            && !cp.contains("%h")
        {
            find(
                out,
                Severity::Warn,
                alias,
                "ControlPath is a fixed name in a shared directory; use %C and a private directory",
            );
        }
    }
    if let Some(v) =
        first(&o, "passwordauthentication").or(first(&o, "kbdinteractiveauthentication"))
    {
        if is_yes(v) {
            find(out, Severity::Info, alias, "password / keyboard-interactive login cannot work: TERYON runs ssh with BatchMode=yes and needs a key or agent");
        }
    }
    if o.contains_key("identityfile") && first(&o, "identitiesonly").is_none_or(|v| !is_yes(v)) {
        find(out, Severity::Info, alias, "IdentityFile without IdentitiesOnly yes: ssh also offers every key in the agent to this host");
    }
}

fn is_yes(v: &str) -> bool {
    v.eq_ignore_ascii_case("yes") || v.eq_ignore_ascii_case("true")
}

/// Keep report lines to one printable line of bounded length.
fn sanitize_for_report(s: &str) -> String {
    crate::sanitize::label(s, 120)
}

/// Audit `alias` (and every jump host it reaches).
pub fn audit(cfg: &Config, alias: &str) -> Audit {
    let mut findings = Vec::new();
    let mut hops = Vec::new();
    if let Some(why) = unsafe_name(alias) {
        find(&mut findings, Severity::Error, alias, format!("name {why}"));
        return Audit {
            target: crate::sanitize::label(alias, 120),
            hops,
            findings,
        };
    }
    audit_host(cfg, alias, &mut Vec::new(), &mut hops, &mut findings);
    if cfg.blocks.iter().any(|b| b.is_match) {
        find(
            &mut findings,
            Severity::Info,
            alias,
            "the config has Match blocks; they are not evaluated here, check with `ssh -G`",
        );
    }
    Audit {
        target: alias.to_string(),
        hops,
        findings,
    }
}

/// Concrete (wildcard-free, non-negated) aliases named on `Host` lines.
pub fn aliases(cfg: &Config) -> Vec<String> {
    let mut v: Vec<String> = cfg
        .blocks
        .iter()
        .filter(|b| !b.is_match)
        .flat_map(|b| b.patterns.iter())
        .filter(|p| !p.starts_with('!') && !p.contains(['*', '?']))
        .cloned()
        .collect();
    v.sort();
    v.dedup();
    v
}

pub fn render(a: &Audit) -> String {
    let mut s = format!("{}\n", crate::sanitize::label(&a.target, 120));
    if a.hops.len() > 1 {
        s.push_str("  path: ");
        let path: Vec<String> = a
            .hops
            .iter()
            .map(|h| {
                let mut t = String::new();
                if let Some(u) = &h.user {
                    t.push_str(&format!("{u}@"));
                }
                t.push_str(&crate::sanitize::label(&h.hostname, 120));
                if let Some(p) = h.port {
                    t.push_str(&format!(":{p}"));
                }
                t
            })
            .collect();
        s.push_str(&path.join(" -> "));
        s.push('\n');
    } else if let Some(h) = a.hops.first() {
        s.push_str(&format!(
            "  host: {}\n",
            crate::sanitize::label(&h.hostname, 120)
        ));
    }
    if a.findings.is_empty() {
        s.push_str("  no findings\n");
    }
    for f in &a.findings {
        let tag = match f.severity {
            Severity::Error => "error",
            Severity::Warn => "warn ",
            Severity::Info => "info ",
        };
        s.push_str(&format!(
            "  {tag} [{}] {}\n",
            crate::sanitize::label(&f.host, 80),
            crate::sanitize::label(&f.message, 400)
        ));
    }
    s
}

pub fn to_json(a: &Audit) -> serde_json::Value {
    serde_json::json!({
        "target": a.target,
        "hops": a.hops.iter().map(|h| serde_json::json!({
            "alias": h.alias, "hostname": h.hostname, "user": h.user, "port": h.port,
        })).collect::<Vec<_>>(),
        "findings": a.findings.iter().map(|f| serde_json::json!({
            "severity": match f.severity { Severity::Error => "error", Severity::Warn => "warn", Severity::Info => "info" },
            "host": f.host,
            "message": f.message,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = "\
# global
Host bastion
  HostName bastion.example.com
  User ops
  Port 2222
  ForwardAgent yes

Host app
  HostName 10.0.0.5
  ProxyJump bastion
  User deploy

Host db
  HostName 10.0.1.9
  ProxyJump app,admin@bastion:2200

Host *
  User fallback
  IdentityFile ~/.ssh/id_x
  ServerAliveInterval 30
";

    fn msgs(a: &Audit) -> String {
        a.findings
            .iter()
            .map(|f| format!("{:?} {} {}", f.severity, f.host, f.message))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn first_value_wins_and_wildcards_fill_in_the_rest() {
        let cfg = parse(CFG);
        let o = resolve(&cfg, "app");
        assert_eq!(first(&o, "user"), Some("deploy"));
        assert_eq!(first(&o, "serveraliveinterval"), Some("30"));
        assert_eq!(resolve(&cfg, "other").get("user").unwrap()[0], "fallback");
    }

    #[test]
    fn the_proxyjump_chain_is_listed_in_connection_order() {
        let a = audit(&parse(CFG), "db");
        let names: Vec<&str> = a.hops.iter().map(|h| h.alias.as_str()).collect();
        // db goes through app (which goes through bastion), then through bastion again.
        assert_eq!(names, ["bastion", "app", "bastion", "db"]);
        assert_eq!(a.hops[2].user.as_deref(), Some("admin"));
        assert_eq!(a.hops[2].port, Some(2200));
        assert!(render(&a).contains("admin@bastion.example.com:2200"));
    }

    #[test]
    fn agent_forwarding_on_a_hop_is_a_warning() {
        let a = audit(&parse(CFG), "app");
        assert!(
            msgs(&a).contains("Warn bastion ForwardAgent yes"),
            "{}",
            msgs(&a)
        );
        assert!(!a.has_errors());
    }

    #[test]
    fn a_proxyjump_loop_is_an_error_not_a_hang() {
        let cfg = parse("Host a\n ProxyJump b\nHost b\n ProxyJump c\nHost c\n ProxyJump a\n");
        let a = audit(&cfg, "a");
        assert!(a.has_errors());
        assert!(msgs(&a).contains("ProxyJump loop"), "{}", msgs(&a));
        let selfloop = audit(&parse("Host a\n ProxyJump a\n"), "a");
        assert!(selfloop.has_errors());
    }

    #[test]
    fn a_very_long_chain_is_cut_off() {
        let mut t = String::new();
        for i in 0..40 {
            t.push_str(&format!("Host h{i}\n ProxyJump h{}\n", i + 1));
        }
        let a = audit(&parse(&t), "h0");
        assert!(msgs(&a).contains("deeper than"), "{}", msgs(&a));
        assert!(a.hops.len() <= MAX_HOPS + 2);
    }

    #[test]
    fn injection_shaped_names_are_errors() {
        for bad in [
            "-oProxyCommand=evil",
            "a;rm -rf ~",
            "$(id)",
            "a b",
            "x`id`",
            "a\u{1b}[2J",
        ] {
            let cfg = parse(&format!("Host ok\n HostName {bad}\n"));
            let a = audit(&cfg, "ok");
            assert!(a.has_errors(), "{bad:?}: {}", msgs(&a));
        }
        let a = audit(&parse("Host ok\n ProxyJump -oProxyCommand=x\n"), "ok");
        assert!(a.has_errors());
        let a = audit(&parse(""), "-oBad");
        assert!(a.has_errors());
        let a = audit(&parse("Host ok\n User $(id)\n"), "ok");
        assert!(a.has_errors());
    }

    #[test]
    fn weak_host_key_settings_are_flagged() {
        let cfg = parse("Host h\n StrictHostKeyChecking no\n UserKnownHostsFile /dev/null\n");
        let a = audit(&cfg, "h");
        let m = msgs(&a);
        assert!(
            m.contains("accepts any host key") && m.contains("forgets every host key"),
            "{m}"
        );
        let a = audit(&parse("Host h\n StrictHostKeyChecking=accept-new\n"), "h");
        assert!(msgs(&a).contains("accept-new"));
    }

    #[test]
    fn local_commands_and_open_listeners_are_flagged() {
        let cfg = parse("Host h\n PermitLocalCommand yes\n LocalCommand echo hi\n LocalForward 0.0.0.0:8080 x:80\n GatewayPorts yes\n ControlPath /tmp/cm\n");
        let m = msgs(&audit(&cfg, "h"));
        for want in [
            "LocalCommand",
            "every interface",
            "GatewayPorts",
            "ControlPath",
        ] {
            assert!(m.contains(want), "{want}: {m}");
        }
    }

    #[test]
    fn hostile_text_in_a_message_cannot_reach_the_terminal() {
        let cfg =
            parse("Host h\n PermitLocalCommand yes\n LocalCommand \x1b]52;c;AAAA\x07\x1b[2Jboom\n");
        let out = render(&audit(&cfg, "h"));
        assert!(!out.contains('\x1b') && !out.contains('\x07'), "{out:?}");
    }

    #[test]
    fn negation_and_glob_patterns_work_and_are_not_exponential() {
        let p = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(host_matches(
            &p(&["*.example.com", "!bad.example.com"]),
            "ok.example.com"
        ));
        assert!(!host_matches(
            &p(&["*.example.com", "!bad.example.com"]),
            "bad.example.com"
        ));
        assert!(glob("a?c", "abc") && !glob("a?c", "ac"));
        let evil = "a*".repeat(200) + "b";
        assert!(!glob(&evil, &"a".repeat(5000)));
    }

    #[test]
    fn malformed_config_is_reported_not_fatal() {
        let big = format!("Host x\n  Note {}\n", "y".repeat(MAX_LINE + 1));
        let cfg = parse(&format!("Host\nUser\n{big}\u{0}\nHost ok\n  Port 99999\n"));
        assert!(cfg.problems.len() >= 3, "{:?}", cfg.problems);
        let a = audit(&cfg, "ok");
        assert!(msgs(&a).contains("Port \"99999\""), "{}", msgs(&a));
        // Binary junk parses to nothing useful but does not panic.
        let junk = String::from_utf8_lossy(&[0xff, 0xfe, 0, 1, 2, b'\n', b'=', b'=']).into_owned();
        let _ = audit(&parse(&junk), "x");
    }

    #[test]
    fn includes_are_read_relative_to_the_config_and_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir(d.join("conf.d")).unwrap();
        std::fs::write(
            d.join("config"),
            "Include conf.d/*.conf\nInclude loop\nInclude missing\nHost *\n User u\n",
        )
        .unwrap();
        std::fs::write(d.join("conf.d/a.conf"), "Host a\n HostName a.example\n").unwrap();
        std::fs::write(d.join("conf.d/skip.txt"), "Host skipped\n").unwrap();
        std::fs::write(d.join("loop"), "Include config\n").unwrap();
        let cfg = load(&d.join("config")).unwrap();
        assert_eq!(aliases(&cfg), ["a"]);
        assert!(cfg.included.len() >= 3);
        assert!(
            cfg.problems.iter().any(|p| p.contains("missing")),
            "{:?}",
            cfg.problems
        );
        assert!(
            cfg.problems.iter().any(|p| p.contains("included twice")),
            "{:?}",
            cfg.problems
        );
    }

    #[test]
    fn a_deep_include_chain_stops_at_the_depth_limit() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for i in 0..20 {
            std::fs::write(d.join(format!("f{i}")), format!("Include f{}\n", i + 1)).unwrap();
        }
        let cfg = load(&d.join("f0")).unwrap();
        assert!(cfg.included.len() <= MAX_INCLUDE_DEPTH + 2);
        assert!(cfg.problems.iter().any(|p| p.contains("nested deeper")));
    }

    #[test]
    fn match_blocks_are_not_evaluated_but_are_mentioned() {
        let cfg = parse("Match host h exec \"curl evil\"\n User pwn\nHost h\n User real\n");
        assert_eq!(first(&resolve(&cfg, "h"), "user"), Some("real"));
        assert!(msgs(&audit(&cfg, "h")).contains("Match blocks"));
    }

    #[test]
    fn json_output_has_hops_and_findings() {
        let j = to_json(&audit(&parse(CFG), "app"));
        assert_eq!(j["hops"].as_array().unwrap().len(), 2);
        assert_eq!(j["findings"][0]["severity"], "warn");
    }
}
