//! `teryon hosts --check`: is each host reachable, how fast, how much disk is free, and is teryon there.
use crate::{config, transport::Target};
use std::fmt::Write;
use std::process::Stdio;
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq)]
pub enum State {
    Up {
        rtt: Duration,
        /// Free and total kilobytes of the home directory's filesystem, when `df` answered.
        disk: Option<(u64, u64)>,
        /// What `teryon --version` printed, when it is installed there.
        version: Option<String>,
    },
    Down(String),
}

/// Run on the host: two lines at most, so a chatty login shell cannot confuse the parser.
pub fn probe_script(exe: &str) -> String {
    format!(
        "df -Pk \"$HOME\" 2>/dev/null | awk 'NR==2{{print \"disk\",$2,$4}}'; \
         v=$({exe} --version 2>/dev/null) && echo \"ver $v\"; true"
    )
}

/// The disk and version lines of the probe's output (anything else, such as a banner, is ignored).
pub fn parse(output: &str) -> (Option<(u64, u64)>, Option<String>) {
    let (mut disk, mut version) = (None, None);
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("disk ") {
            let mut n = rest
                .split_whitespace()
                .filter_map(|w| w.parse::<u64>().ok());
            if let (Some(total), Some(free)) = (n.next(), n.next()) {
                disk = Some((free, total));
            }
        } else if let Some(v) = line.strip_prefix("ver ") {
            version = Some(v.trim().trim_start_matches("teryon ").to_string());
        }
    }
    (disk, version)
}

pub fn size(kb: u64) -> String {
    let mut v = kb as f64;
    for unit in ["K", "M", "G", "T"] {
        if v < 1024.0 || unit == "T" {
            return if v >= 10.0 {
                format!("{v:.0}{unit}")
            } else {
                format!("{v:.1}{unit}")
            };
        }
        v /= 1024.0;
    }
    unreachable!()
}

/// Ask `host` how it is doing. Never fails: a host that cannot be reached is reported as down.
pub fn check(host: &str) -> State {
    let target: Target = match config::target(host, None) {
        Ok(t) => t,
        Err(e) => return State::Down(format!("{e:#}")),
    };
    if target.local() {
        return State::Up {
            rtt: Duration::ZERO,
            disk: None,
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        };
    }
    let started = Instant::now();
    let out = target
        .ssh()
        .arg(probe_script(&target.remote_executable()))
        .stdin(Stdio::null())
        .output();
    let rtt = started.elapsed();
    match out {
        Ok(o) if o.status.success() => {
            let (disk, version) = parse(&String::from_utf8_lossy(&o.stdout));
            State::Up { rtt, disk, version }
        }
        Ok(o) => State::Down(last_line(&String::from_utf8_lossy(&o.stderr))),
        Err(e) => State::Down(format!("cannot start ssh: {e}")),
    }
}

fn last_line(text: &str) -> String {
    let line = text
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("no answer");
    line.chars().take(70).collect()
}

/// Check every host at the same time, in the order given.
pub fn check_all(hosts: &[String]) -> Vec<(String, State)> {
    std::thread::scope(|s| {
        let jobs: Vec<_> = hosts
            .iter()
            .map(|h| s.spawn(move || (h.clone(), check(h))))
            .collect();
        jobs.into_iter().filter_map(|j| j.join().ok()).collect()
    })
}

pub fn render(rows: &[(String, State)]) -> String {
    let width = rows.iter().map(|(h, _)| h.len()).max().unwrap_or(4).max(4);
    let mut out = format!(
        "{:<width$}  {:<5}  {:>6}  {:>9}  TERYON\n",
        "HOST", "STATE", "RTT", "DISK FREE"
    );
    for (host, state) in rows {
        match state {
            State::Up { rtt, disk, version } => {
                let rtt = if rtt.is_zero() {
                    "-".to_string()
                } else {
                    format!("{}ms", rtt.as_millis())
                };
                let disk = disk.map_or("-".to_string(), |(free, total)| {
                    format!("{}/{}", size(free), size(total))
                });
                let _ = writeln!(
                    out,
                    "{host:<width$}  {:<5}  {rtt:>6}  {disk:>9}  {}",
                    "up",
                    version.as_deref().unwrap_or("not installed")
                );
            }
            State::Down(why) => {
                let _ = writeln!(out, "{host:<width$}  {:<5}  {why}", "down");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_output_gives_disk_and_version_and_skips_banners() {
        let text = "Welcome to the box\ndisk 102400 51200\nver teryon 0.21.0\n";
        assert_eq!(
            parse(text),
            (Some((51200, 102400)), Some("0.21.0".to_string()))
        );
        assert_eq!(parse("only a banner\n"), (None, None));
    }

    #[test]
    fn sizes_are_short() {
        assert_eq!(size(512), "512K");
        assert_eq!(size(1536), "1.5M");
        assert_eq!(size(250 * 1024), "250M");
        assert_eq!(size(3 * 1024 * 1024 * 1024), "3.0T");
    }

    #[test]
    fn the_table_shows_up_and_down_hosts() {
        let rows = vec![
            (
                "mini".to_string(),
                State::Up {
                    rtt: Duration::from_millis(42),
                    disk: Some((51200, 102400)),
                    version: None,
                },
            ),
            (
                "gone".to_string(),
                State::Down("Connection refused".to_string()),
            ),
        ];
        let text = render(&rows);
        assert!(
            text.contains("mini  up       42ms   50M/100M  not installed"),
            "{text}"
        );
        assert!(text.contains("gone  down   Connection refused"), "{text}");
    }
}
