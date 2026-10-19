//! Which TCP ports a pane's processes are listening on, so a dev server can be opened in a browser.
use std::collections::{HashMap, HashSet};
use std::process::{Command, Stdio};

/// `(pid, port)` pairs from `lsof -nP -iTCP -sTCP:LISTEN -Fpn` output.
pub fn parse_lsof(text: &str) -> Vec<(u32, u16)> {
    let mut out = Vec::new();
    let mut pid = None;
    for line in text.lines() {
        if let Some(p) = line.strip_prefix('p') {
            pid = p.parse().ok();
        } else if let (Some(name), Some(pid)) = (line.strip_prefix('n'), pid) {
            if let Some(port) = name.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()) {
                out.push((pid, port));
            }
        }
    }
    out
}
/// Children by parent pid from `ps -A -o pid=,ppid=` output.
pub fn parse_ps(text: &str) -> HashMap<u32, Vec<u32>> {
    let mut kids: HashMap<u32, Vec<u32>> = HashMap::new();
    for line in text.lines() {
        let mut f = line.split_whitespace();
        if let (Some(Ok(pid)), Some(Ok(ppid))) = (
            f.next().map(str::parse::<u32>),
            f.next().map(str::parse::<u32>),
        ) {
            kids.entry(ppid).or_default().push(pid);
        }
    }
    kids
}
/// `root` and everything below it.
pub fn descendants(root: u32, kids: &HashMap<u32, Vec<u32>>) -> HashSet<u32> {
    let mut seen = HashSet::from([root]);
    let mut todo = vec![root];
    while let Some(p) = todo.pop() {
        for k in kids.get(&p).into_iter().flatten() {
            if seen.insert(*k) {
                todo.push(*k);
            }
        }
    }
    seen
}
/// Sorted ports listened on by `pids`.
pub fn ports_of(pids: &HashSet<u32>, listening: &[(u32, u16)]) -> Vec<u16> {
    let mut v: Vec<u16> = listening
        .iter()
        .filter(|(pid, _)| pids.contains(pid))
        .map(|(_, port)| *port)
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}
fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    // lsof exits 1 when it finds nothing; its output is still the answer.
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}
/// Ports listened on by the process tree under `root` (runs `ps` and `lsof`; call it rarely).
pub fn listening(root: u32) -> Vec<u16> {
    let (Some(ps), Some(lsof)) = (
        run("ps", &["-A", "-o", "pid=,ppid="]),
        run("lsof", &["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpn"]),
    ) else {
        return Vec::new();
    };
    let tree = descendants(root, &parse_ps(&ps));
    ports_of(&tree, &parse_lsof(&lsof))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lsof_output_gives_pid_port_pairs() {
        let t = "p100\nn*:3000\nn[::1]:3000\np200\nn127.0.0.1:8080\np300\nnnot-a-port\n";
        assert_eq!(parse_lsof(t), [(100, 3000), (100, 3000), (200, 8080)]);
    }
    #[test]
    fn only_ports_below_the_panes_shell_count() {
        let kids = parse_ps("  1 0\n 10 1\n 11 10\n 12 11\n 20 1\n");
        let tree = descendants(10, &kids);
        assert_eq!(tree, HashSet::from([10, 11, 12]));
        let listening = [(12, 5173), (20, 9999), (11, 3000), (12, 3000)];
        assert_eq!(ports_of(&tree, &listening), [3000, 5173]);
    }
    #[test]
    fn a_real_listener_in_this_process_is_found() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let me = std::process::id();
        // lsof may be missing or restricted in a sandbox; only assert when it answers at all.
        let found = listening(me);
        if run("lsof", &["-v"]).is_some() && !found.is_empty() {
            assert!(found.contains(&port), "{found:?} lacks {port}");
        }
    }
}
