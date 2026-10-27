//! `teryon cp SRC... DEST`: copy files between this machine and SSH hosts, scp-style (`host:path`).
//!
//! Directories always copy recursively. rsync 3+ is used when both ends have it (resumable, with
//! progress); otherwise `scp`. A trailing `/` on a directory source copies its contents, like rsync.
use crate::{config, transport, util};
use anyhow::{bail, Result};
use std::process::Command;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Endpoint {
    Local(String),
    Remote { host: String, path: String },
}

/// `host:path` and `user@host:path` are remote; anything with a `/` before its first `:` is a local
/// path (write `./a:b` for a local file whose name has a colon).
pub fn parse_endpoint(arg: &str) -> Endpoint {
    let Some((host, path)) = arg.split_once(':') else {
        return Endpoint::Local(arg.to_string());
    };
    let valid = !host.is_empty()
        && !host.starts_with('-')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.@-".contains(&b));
    if !valid {
        return Endpoint::Local(arg.to_string());
    }
    if host == "local" {
        return Endpoint::Local(path.to_string());
    }
    Endpoint::Remote {
        host: host.to_string(),
        path: path.to_string(),
    }
}

/// A remote path as rsync/scp expect it: relative paths are under the login directory, so `~` and
/// `~/x` become `.` and `x`.
pub fn remote_path(path: &str) -> String {
    match path {
        "" | "~" => ".".to_string(),
        _ => match path.strip_prefix("~/") {
            Some("") => "./".to_string(),
            Some(rest) => rest.to_string(),
            None => path.to_string(),
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Engine {
    Auto,
    Rsync,
    Scp,
}

#[derive(Clone, Debug)]
pub struct Opts {
    pub engine: Engine,
    pub dry_run: bool,
    pub no_clobber: bool,
    pub parents: bool,
    pub limit_kbps: Option<u32>,
}

/// What `plan` decided: the command to run and, with `--parents`, a directory to create first.
pub struct Job {
    pub engine: &'static str,
    pub command: Command,
    /// (destination host, directory) to `mkdir -p` before copying; `None` host means this machine.
    pub mkdir: Option<(Option<String>, String)>,
}

/// Facts about the machines involved, so `plan` stays pure and testable.
pub trait Env {
    /// The SSH destination for a host alias.
    fn resolve(&self, host: &str) -> Result<String>;
    fn local_rsync(&self) -> bool;
    fn remote_rsync(&self, destination: &str) -> bool;
}

pub struct RealEnv;
impl Env for RealEnv {
    fn resolve(&self, host: &str) -> Result<String> {
        Ok(config::target(host, None)?.host)
    }
    fn local_rsync(&self) -> bool {
        transport::rsync_available()
    }
    fn remote_rsync(&self, destination: &str) -> bool {
        transport::Target::new(destination.to_string(), None).is_ok_and(|t| t.has_rsync())
    }
}

fn check_path(path: &str) -> Result<()> {
    if path.contains(['\n', '\0']) {
        bail!("paths cannot contain newlines");
    }
    Ok(())
}

/// The directory a copy lands in, when it has to exist first.
fn dest_dir(dest: &str, many: bool) -> Option<String> {
    if dest.ends_with('/') || many {
        return Some(dest.to_string());
    }
    match dest.rsplit_once('/') {
        Some(("", _)) => Some("/".to_string()),
        Some((dir, _)) => Some(dir.to_string()),
        None => None,
    }
}

pub fn plan(paths: &[String], opts: &Opts, env: &dyn Env) -> Result<Job> {
    let Some((dest, sources)) = paths.split_last().filter(|(_, s)| !s.is_empty()) else {
        bail!("usage: teryon cp SOURCE... DEST (use host:path for the remote side)");
    };
    for p in paths {
        check_path(p)?;
    }
    let dest = parse_endpoint(dest);
    let sources: Vec<Endpoint> = sources.iter().map(|s| parse_endpoint(s)).collect();
    let remote_src = sources
        .iter()
        .find(|s| matches!(s, Endpoint::Remote { .. }));
    if remote_src.is_some() && sources.iter().any(|s| matches!(s, Endpoint::Local(_))) {
        bail!("sources must all be local or all be on one host");
    }
    let src_hosts: Vec<&str> = sources
        .iter()
        .filter_map(|s| match s {
            Endpoint::Remote { host, .. } => Some(host.as_str()),
            _ => None,
        })
        .collect();
    if src_hosts.windows(2).any(|w| w[0] != w[1]) {
        bail!("sources must all come from the same host");
    }
    let many = sources.len() > 1;
    if remote_src.is_none() && matches!(dest, Endpoint::Local(_)) {
        bail!("neither side is a host; use host:path for the remote side (or plain cp)");
    }

    // Resolve aliases and normalise remote paths.
    let mut args_src: Vec<String> = Vec::new();
    let mut src_dest: Option<String> = None;
    for s in &sources {
        match s {
            Endpoint::Local(p) => args_src.push(p.clone()),
            Endpoint::Remote { host, path } => {
                let d = env.resolve(host)?;
                args_src.push(format!("{d}:{}", remote_path(path)));
                src_dest = Some(d);
            }
        }
    }
    let (args_dest, dest_host, dest_path) = match &dest {
        Endpoint::Local(p) => (p.clone(), None, p.clone()),
        Endpoint::Remote { host, path } => {
            let d = env.resolve(host)?;
            let rp = remote_path(path);
            (format!("{d}:{rp}"), Some(d), rp)
        }
    };

    let both_remote = src_dest.is_some() && dest_host.is_some();
    let rsync_ok = !both_remote
        && env.local_rsync()
        && src_dest
            .iter()
            .chain(dest_host.iter())
            .all(|d| env.remote_rsync(d));
    let use_rsync = match opts.engine {
        Engine::Rsync if both_remote => {
            bail!("rsync cannot copy between two hosts; use --engine scp")
        }
        Engine::Rsync if !rsync_ok => {
            bail!("rsync 3+ is not available on every machine involved; use --engine scp")
        }
        Engine::Rsync => true,
        Engine::Scp => false,
        Engine::Auto => rsync_ok,
    };

    let mut command = if use_rsync {
        let mut c = transport::rsync_command();
        if let Some(limit) = opts.limit_kbps {
            c.arg(format!("--bwlimit={limit}"));
        }
        if opts.no_clobber {
            c.arg("--ignore-existing");
        }
        if opts.dry_run {
            c.args(["-n", "--itemize-changes"]);
        }
        c
    } else {
        let mut c = transport::scp_command();
        if let Some(limit) = opts.limit_kbps {
            // scp's -l is in Kbit/s; ours is KB/s.
            c.arg("-l").arg((u64::from(limit) * 8).to_string());
        }
        if both_remote {
            c.arg("-3");
        }
        c
    };
    command.arg("--").args(&args_src).arg(&args_dest);

    let mkdir = if opts.parents {
        dest_dir(&dest_path, many).map(|d| (dest_host.clone(), d))
    } else {
        None
    };
    Ok(Job {
        engine: if use_rsync { "rsync" } else { "scp" },
        command,
        mkdir,
    })
}

pub fn run(paths: &[String], opts: &Opts, verbose: bool) -> Result<()> {
    let mut job = plan(paths, opts, &RealEnv)?;
    if verbose || (opts.dry_run && job.engine == "scp") {
        let c = &job.command;
        let line: Vec<String> = std::iter::once(c.get_program())
            .chain(c.get_args())
            .map(|a| util::shell_quote(&a.to_string_lossy()))
            .collect();
        eprintln!("[{}] {}", job.engine, line.join(" "));
    }
    if opts.dry_run && job.engine == "scp" {
        return Ok(());
    }
    if let Some((host, dir)) = &job.mkdir {
        if !opts.dry_run {
            match host {
                Some(h) => transport::Target::new(h.clone(), None)?.mkdir_p(dir)?,
                None => std::fs::create_dir_all(dir)?,
            }
        }
    }
    let status = job
        .command
        .status()
        .map_err(|e| anyhow::anyhow!("cannot start {}: {e} (is it installed?)", job.engine))?;
    if !status.success() {
        bail!(
            "{} failed ({status}); repeat the same command to resume, or try --engine {}",
            job.engine,
            if job.engine == "rsync" {
                "scp"
            } else {
                "rsync"
            }
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        rsync: bool,
    }
    impl Env for Fake {
        fn resolve(&self, host: &str) -> Result<String> {
            Ok(host.to_string())
        }
        fn local_rsync(&self) -> bool {
            self.rsync
        }
        fn remote_rsync(&self, _: &str) -> bool {
            self.rsync
        }
    }
    fn opts() -> Opts {
        Opts {
            engine: Engine::Auto,
            dry_run: false,
            no_clobber: false,
            parents: false,
            limit_kbps: None,
        }
    }
    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }
    fn tail(job: &Job, n: usize) -> Vec<String> {
        let a: Vec<String> = job
            .command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        a[a.len() - n..].to_vec()
    }

    #[test]
    fn endpoints_split_on_host_colon() {
        assert_eq!(parse_endpoint("a.txt"), Endpoint::Local("a.txt".into()));
        assert_eq!(parse_endpoint("./a:b"), Endpoint::Local("./a:b".into()));
        assert_eq!(parse_endpoint("/x:y"), Endpoint::Local("/x:y".into()));
        assert_eq!(parse_endpoint("local:/tmp"), Endpoint::Local("/tmp".into()));
        assert_eq!(
            parse_endpoint("me@mini:~/x"),
            Endpoint::Remote {
                host: "me@mini".into(),
                path: "~/x".into()
            }
        );
    }
    #[test]
    fn tilde_paths_become_login_relative() {
        assert_eq!(remote_path("~"), ".");
        assert_eq!(remote_path("~/"), "./");
        assert_eq!(remote_path("~/a b"), "a b");
        assert_eq!(remote_path("/etc"), "/etc");
        assert_eq!(remote_path(""), ".");
    }
    #[test]
    fn upload_uses_rsync_when_everyone_has_it() {
        let job = plan(
            &v(&["a", "b", "mini:~/dst/"]),
            &opts(),
            &Fake { rsync: true },
        )
        .unwrap();
        assert_eq!(job.engine, "rsync");
        assert_eq!(tail(&job, 4), ["--", "a", "b", "mini:dst/"]);
    }
    #[test]
    fn falls_back_to_scp_without_rsync() {
        let job = plan(&v(&["mini:x", "."]), &opts(), &Fake { rsync: false }).unwrap();
        assert_eq!(job.engine, "scp");
        assert_eq!(tail(&job, 3), ["--", "mini:x", "."]);
    }
    #[test]
    fn host_to_host_goes_through_scp_3() {
        let job = plan(&v(&["a:x", "b:y"]), &opts(), &Fake { rsync: true }).unwrap();
        assert_eq!(job.engine, "scp");
        assert!(job.command.get_args().any(|a| a == "-3"));
        let forced = Opts {
            engine: Engine::Rsync,
            ..opts()
        };
        assert!(plan(&v(&["a:x", "b:y"]), &forced, &Fake { rsync: true }).is_err());
    }
    #[test]
    fn rejects_bad_shapes() {
        let env = Fake { rsync: true };
        assert!(plan(&v(&["only"]), &opts(), &env).is_err());
        assert!(plan(&v(&["a", "b"]), &opts(), &env).is_err());
        assert!(plan(&v(&["a", "m:b", "."]), &opts(), &env).is_err());
        assert!(plan(&v(&["m:a", "n:b", "."]), &opts(), &env).is_err());
        assert!(plan(&v(&["a\nb", "m:c"]), &opts(), &env).is_err());
    }
    #[test]
    fn flags_map_to_each_engine() {
        let o = Opts {
            no_clobber: true,
            dry_run: true,
            limit_kbps: Some(100),
            ..opts()
        };
        let r = plan(&v(&["a", "m:b"]), &o, &Fake { rsync: true }).unwrap();
        let args: Vec<_> = r
            .command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        for want in ["--bwlimit=100", "--ignore-existing", "-n"] {
            assert!(args.iter().any(|a| a == want), "{want}");
        }
        let s = plan(&v(&["a", "m:b"]), &o, &Fake { rsync: false }).unwrap();
        let args: Vec<_> = s
            .command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let l = args.iter().position(|a| a == "-l").unwrap();
        assert_eq!(args[l + 1], "800");
    }
    #[test]
    fn parents_picks_the_directory_to_create() {
        let o = Opts {
            parents: true,
            ..opts()
        };
        let env = Fake { rsync: true };
        let j = plan(&v(&["a", "m:~/x/y/f.txt"]), &o, &env).unwrap();
        assert_eq!(j.mkdir, Some((Some("m".into()), "x/y".into())));
        let j = plan(&v(&["a", "b", "m:out"]), &o, &env).unwrap();
        assert_eq!(j.mkdir, Some((Some("m".into()), "out".into())));
        let j = plan(&v(&["m:a", "/tmp/new/"]), &o, &env).unwrap();
        assert_eq!(j.mkdir, Some((None, "/tmp/new/".into())));
        let j = plan(&v(&["a", "m:f.txt"]), &o, &env).unwrap();
        assert_eq!(j.mkdir, None);
    }
}
