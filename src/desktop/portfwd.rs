//! "Open listening port": a dev server in a pane, opened in the local browser (through `ssh -L` for remote hosts).
use super::*;

/// Arguments for an SSH tunnel that makes the remote `port` reachable on this machine's same port.
pub(super) fn forward_args(destination: &str, port: u16) -> Vec<String> {
    [
        "-N",
        "-o",
        "ExitOnForwardFailure=yes",
        "-o",
        "BatchMode=yes",
        "-L",
        &format!("{port}:localhost:{port}"),
        "--",
        destination,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

impl Desktop {
    /// Open the first port the focused pane is listening on; remote hosts get a tunnel first.
    pub(super) fn open_port(&mut self) {
        let ports = self
            .panes
            .get(&self.focus)
            .map(|p| p.info.ports.clone())
            .unwrap_or_default();
        let Some(&port) = ports.first() else {
            self.toast("No listening ports in this pane");
            return;
        };
        let url = format!("http://localhost:{port}");
        if self.host != "local" {
            if let Some(child) = self.forwards.get_mut(&port) {
                if child.try_wait().ok().flatten().is_some() {
                    self.forwards.remove(&port);
                }
            }
            if !self.forwards.contains_key(&port) {
                let destination = config::load()
                    .ok()
                    .and_then(|c| c.hosts.get(&self.host).map(|h| h.destination.clone()))
                    .unwrap_or_else(|| self.host.clone());
                match std::process::Command::new("ssh")
                    .args(forward_args(&destination, port))
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                {
                    Ok(child) => {
                        self.forwards.insert(port, child);
                    }
                    Err(e) => {
                        self.toast(format!("Could not start ssh: {e}"));
                        return;
                    }
                }
                // Give the tunnel a moment to come up before the browser asks.
                let url = url.clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(1200));
                    crate::uikit::open_url(&url);
                });
            } else {
                crate::uikit::open_url(&url);
            }
        } else {
            crate::uikit::open_url(&url);
        }
        let others: Vec<String> = ports.iter().skip(1).map(|p| format!(":{p}")).collect();
        self.toast(if others.is_empty() {
            format!("Opening {url}")
        } else {
            format!("Opening {url} (also listening: {})", others.join(" "))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::tests::app_with_screen;

    #[test]
    fn the_tunnel_arguments_forward_one_port_and_never_prompt() {
        let a = forward_args("me@mini", 3000);
        assert!(a.windows(2).any(|w| w == ["-L", "3000:localhost:3000"]));
        assert!(a.contains(&"BatchMode=yes".to_string()));
        assert_eq!(a.last().unwrap(), "me@mini");
        let dash = forward_args("-oProxyCommand=evil", 1);
        assert_eq!(
            dash[dash.len() - 2],
            "--",
            "a hostile destination cannot become an option"
        );
    }
    #[test]
    fn without_a_listening_port_the_action_says_so() {
        let (mut app, _rx) = app_with_screen("");
        app.open_port();
        assert!(app
            .toast
            .as_ref()
            .is_some_and(|(t, _)| t.contains("No listening ports")));
    }
}
