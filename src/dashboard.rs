//! `teryon dashboard`: one-shot or refreshing overview of every configured host.
use crate::{protocol::*, transport::Target};
use std::time::Instant;

pub struct Row {
    pub host: String,
    pub status: String,
    pub system: String,
    pub workspaces: String,
    pub panes: String,
    pub latency: String,
}

/// Probe one host; failures become a `down` row instead of aborting the dashboard.
pub fn probe(host: &str, target: Result<Target, anyhow::Error>) -> Row {
    let started = Instant::now();
    let result = (|| -> anyhow::Result<(String, usize, usize)> {
        let mut rpc = target?.connect()?;
        let system = match rpc.call(Request::Info)? {
            Response::Info { os, arch, .. } => format!("{os}/{arch}"),
            _ => "?".into(),
        };
        let (workspaces, panes) = match rpc.call(Request::List)? {
            Response::Workspaces { items } => (items.len(), items.iter().map(|w| w.panes).sum()),
            _ => (0, 0),
        };
        Ok((system, workspaces, panes))
    })();
    match result {
        Ok((system, workspaces, panes)) => Row {
            host: host.into(),
            status: "up".into(),
            system,
            workspaces: workspaces.to_string(),
            panes: panes.to_string(),
            latency: format!("{} ms", started.elapsed().as_millis()),
        },
        Err(e) => {
            let reason = format!("{e:#}");
            let first = reason.lines().next().unwrap_or("").to_string();
            Row {
                host: host.into(),
                status: format!("down: {}", first.chars().take(48).collect::<String>()),
                system: "-".into(),
                workspaces: "-".into(),
                panes: "-".into(),
                latency: "-".into(),
            }
        }
    }
}

/// Fixed-width table; `color` adds green/amber status markers.
pub fn format_table(rows: &[Row], color: bool) -> String {
    let head = ["HOST", "STATUS", "SYSTEM", "WORKSPACES", "PANES", "LATENCY"];
    let cells: Vec<[&str; 6]> = rows
        .iter()
        .map(|r| {
            [
                r.host.as_str(),
                r.status.as_str(),
                r.system.as_str(),
                r.workspaces.as_str(),
                r.panes.as_str(),
                r.latency.as_str(),
            ]
        })
        .collect();
    let mut widths = head.map(|h| h.chars().count());
    for row in &cells {
        for (w, c) in widths.iter_mut().zip(row) {
            *w = (*w).max(c.chars().count());
        }
    }
    let line = |row: &[&str; 6]| {
        let mut out = String::new();
        for (i, (c, w)) in row.iter().zip(widths).enumerate() {
            let pad = " ".repeat(w - c.chars().count());
            if color && i == 1 && *c != "STATUS" {
                let code = if *c == "up" { "32" } else { "33" };
                out.push_str(&format!("\x1b[{code}m{c}\x1b[0m{pad}  "));
            } else {
                out.push_str(&format!("{c}{pad}  "));
            }
        }
        out.trim_end().to_string() + "\n"
    };
    let mut out = line(&head);
    for row in &cells {
        out.push_str(&line(row));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(host: &str, status: &str) -> Row {
        Row {
            host: host.into(),
            status: status.into(),
            system: "macos/aarch64".into(),
            workspaces: "2".into(),
            panes: "5".into(),
            latency: "41 ms".into(),
        }
    }
    #[test]
    fn table_aligns_columns_and_colors_status() {
        let plain = format_table(
            &[row("mini", "up"), row("long-host-name", "down: timeout")],
            false,
        );
        let lines: Vec<_> = plain.lines().collect();
        assert_eq!(lines.len(), 3);
        let col = lines[0].find("STATUS").unwrap();
        assert!(lines.iter().all(|l| l.len() > col));
        assert_eq!(lines[1].find("up"), Some(col));
        assert!(format_table(&[row("mini", "up")], true).contains("\x1b[32mup"));
    }
    #[test]
    fn unreachable_host_is_a_down_row() {
        let r = probe("bad", Target::new("-bad".into(), None));
        assert!(r.status.starts_with("down: "));
        assert_eq!(r.panes, "-");
    }
}
