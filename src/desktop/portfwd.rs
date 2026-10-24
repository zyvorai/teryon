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
    /// Whether an `ssh -L` tunnel for `port` is running; one that has exited is forgotten.
    fn tunnel_up(&mut self, port: u16) -> bool {
        if let Some(child) = self.forwards.get_mut(&port) {
            if child.try_wait().ok().flatten().is_none() {
                return true;
            }
            self.forwards.remove(&port);
        }
        false
    }
    /// Start the tunnel for a remote `port` unless one is already up. True when it was started now.
    fn start_tunnel(&mut self, port: u16) -> Result<bool, String> {
        if self.tunnel_up(port) {
            return Ok(false);
        }
        let destination = config::load()
            .ok()
            .and_then(|c| c.hosts.get(&self.host).map(|h| h.destination.clone()))
            .unwrap_or_else(|| self.host.clone());
        let child = std::process::Command::new("ssh")
            .args(forward_args(&destination, port))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Could not start ssh: {e}"))?;
        self.forwards.insert(port, child);
        Ok(true)
    }
    /// Open `http://localhost:port`, first forwarding it when the workspace is on another host.
    fn open_port_number(&mut self, port: u16) -> bool {
        let url = format!("http://localhost:{port}");
        if self.host == "local" {
            crate::uikit::open_url(&url);
            return true;
        }
        match self.start_tunnel(port) {
            Ok(true) => {
                // Give the tunnel a moment to come up before the browser asks.
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(1200));
                    crate::uikit::open_url(&url);
                });
                true
            }
            Ok(false) => {
                crate::uikit::open_url(&url);
                true
            }
            Err(e) => {
                self.toast(e);
                false
            }
        }
    }
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
        if !self.open_port_number(port) {
            return;
        }
        let others: Vec<String> = ports.iter().skip(1).map(|p| format!(":{p}")).collect();
        self.toast(if others.is_empty() {
            format!("Opening http://localhost:{port}")
        } else {
            format!(
                "Opening http://localhost:{port} (also listening: {})",
                others.join(" ")
            )
        });
    }
    /// Every listening port in the workspace, once each, with the pane that owns it.
    fn listening(&self) -> Vec<(u16, String)> {
        let mut out: Vec<(u16, String)> = Vec::new();
        for p in self.panes.values() {
            for &port in &p.info.ports {
                if !out.iter().any(|(q, _)| *q == port) {
                    out.push((port, p.info.title.clone()));
                }
            }
        }
        out.sort_by_key(|(port, _)| *port);
        out
    }
    /// The Ports window: each listening port with Open, Forward / Stop and Copy URL.
    pub(super) fn ports_ui(&mut self, ctx: &egui::Context) {
        if !self.ports_open {
            return;
        }
        let t = self.theme.clone();
        let remote = self.host != "local";
        let rows = self.listening();
        let mut open = true;
        let mut act: Option<(u16, u8)> = None;
        egui::Window::new("Ports")
            .id(egui::Id::new("teryon-ports"))
            .open(&mut open)
            .collapsible(false)
            .default_width(360.)
            .show(ctx, |ui| {
                if rows.is_empty() {
                    ui.label(RichText::new("Nothing is listening in this workspace.").color(color(t.muted)));
                }
                for (port, title) in &rows {
                    let up = remote && self.forwards.contains_key(port);
                    ui.horizontal(|ui| {
                        ui.label(weighted(ctx, format!(":{port}"), "semibold", 13.).color(color(t.foreground)));
                        ui.label(RichText::new(title).size(12.).color(color(t.muted)));
                        if up {
                            ui.label(RichText::new("forwarded").size(11.).color(color(t.success)));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Copy URL").clicked() {
                                act = Some((*port, 3));
                            }
                            if remote {
                                if up {
                                    if ui.small_button("Stop").clicked() {
                                        act = Some((*port, 2));
                                    }
                                } else if ui.small_button("Forward").clicked() {
                                    act = Some((*port, 1));
                                }
                            }
                            if ui.small_button("Open").clicked() {
                                act = Some((*port, 0));
                            }
                        });
                    });
                }
                if remote {
                    ui.separator();
                    ui.label(
                        RichText::new("Forward uses ssh -L to the same port on this Mac. Tunnels end when the app closes.")
                            .size(11.)
                            .color(color(t.muted)),
                    );
                }
            });
        match act {
            Some((port, 0)) => {
                self.open_port_number(port);
            }
            Some((port, 1)) => match self.start_tunnel(port) {
                Ok(_) => self.toast(format!("Forwarding :{port} to this Mac")),
                Err(e) => self.toast(e),
            },
            Some((port, 2)) => {
                if let Some(mut child) = self.forwards.remove(&port) {
                    let _ = child.kill();
                    let _ = child.wait();
                }
                self.toast(format!("Stopped forwarding :{port}"));
            }
            Some((port, _)) => {
                ctx.copy_text(format!("http://localhost:{port}"));
                self.toast("URL copied");
            }
            None => {}
        }
        // A tunnel that died (the local port was busy, the link dropped) stops showing as forwarded.
        let ports: Vec<u16> = self.forwards.keys().copied().collect();
        for port in ports {
            self.tunnel_up(port);
        }
        if !open {
            self.ports_open = false;
        }
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
    fn the_ports_window_lists_each_listening_port_once_and_draws() {
        let (mut app, _rx) = app_with_screen("");
        app.panes.get_mut(&1).unwrap().info.ports = vec![8080, 3000];
        app.panes.get_mut(&1).unwrap().info.title = "dev".into();
        assert_eq!(
            app.listening(),
            vec![(3000, "dev".to_string()), (8080, "dev".to_string())]
        );
        app.ports_open = true;
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| app.ports_ui(ctx));
        assert!(app.ports_open);
        assert!(!app.tunnel_up(3000), "no tunnel was started");
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
