//! Recently used `host / workspace` pairs (`~/.teryon/recent.toml`), shown first by the start picker.
use crate::util;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MAX_RECENT: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recent {
    pub host: String,
    pub workspace: String,
    pub last_used: u64,
    pub count: u32,
}
#[derive(Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    recent: Vec<Recent>,
}

pub fn path() -> Result<PathBuf> {
    Ok(util::home()?.join(".teryon/recent.toml"))
}
/// Parse the file text; garbage yields an empty list. Most recently used first.
pub fn parse(text: &str) -> Vec<Recent> {
    let mut list = toml::from_str::<File>(text).unwrap_or_default().recent;
    list.sort_by_key(|r| std::cmp::Reverse(r.last_used));
    list
}
pub fn load() -> Vec<Recent> {
    path()
        .and_then(|p| Ok(std::fs::read_to_string(p)?))
        .map(|t| parse(&t))
        .unwrap_or_default()
}
/// Note one use of `host`/`workspace` at time `now`, keeping the list short and newest first.
pub fn touch(list: &mut Vec<Recent>, host: &str, workspace: &str, now: u64) {
    match list
        .iter_mut()
        .find(|r| r.host == host && r.workspace == workspace)
    {
        Some(r) => {
            r.last_used = now;
            r.count = r.count.saturating_add(1);
        }
        None => list.push(Recent {
            host: host.into(),
            workspace: workspace.into(),
            last_used: now,
            count: 1,
        }),
    }
    list.sort_by_key(|r| std::cmp::Reverse(r.last_used));
    list.truncate(MAX_RECENT);
}
fn save(list: &[Recent]) -> Result<()> {
    let text = toml::to_string(&File {
        recent: list.to_vec(),
    })?;
    util::write_private_atomic(&path()?, text.as_bytes())
}
/// Best-effort bookkeeping after a successful start; failures are ignored.
pub fn record(host: &str, workspace: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut list = load();
    touch(&mut list, host, workspace, now);
    let _ = save(&list);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn touch_updates_orders_and_caps() {
        let mut l = Vec::new();
        touch(&mut l, "mini", "a", 10);
        touch(&mut l, "mini", "b", 20);
        touch(&mut l, "mini", "a", 30);
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].workspace.as_str(), l[0].count), ("a", 2));
        for i in 0..80 {
            touch(&mut l, "h", &format!("w{i}"), 100 + i);
        }
        assert_eq!(l.len(), MAX_RECENT);
        assert_eq!(l[0].workspace, "w79");
    }
    #[test]
    fn parse_round_trips_and_ignores_garbage() {
        let mut l = Vec::new();
        touch(&mut l, "mini", "a", 5);
        let text = toml::to_string(&File { recent: l.clone() }).unwrap();
        assert_eq!(parse(&text), l);
        assert!(parse("not toml [").is_empty());
        assert!(parse("").is_empty());
    }
}
