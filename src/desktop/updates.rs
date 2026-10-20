//! "Check for updates": asks GitHub in the background whether a newer release is out and says so.
use super::*;

impl Desktop {
    /// Start a check. `asked` is true when the user asked (they get an answer either way and the
    /// release page opens); the automatic check only speaks up when there is something new.
    pub(super) fn check_updates(&mut self, asked: bool) {
        if self.update_check.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(crate::update::check());
        });
        self.update_check = Some((rx, asked));
        if asked {
            self.toast("Checking for updates…");
        }
    }

    /// Run the once-a-day check at start-up when it is switched on, and collect a finished one.
    pub(super) fn update_tick(&mut self, ctx: &egui::Context) {
        if self.cfg.check_updates && !self.update_auto_done {
            self.update_auto_done = true;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            if crate::update::due(now) {
                crate::update::mark(now);
                self.check_updates(false);
            }
        }
        let Some((rx, asked)) = &self.update_check else {
            return;
        };
        let asked = *asked;
        match rx.try_recv() {
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(300)),
            Err(mpsc::TryRecvError::Disconnected) => self.update_check = None,
            Ok(result) => {
                self.update_check = None;
                match result {
                    Ok(Some(r)) if asked => {
                        self.toast(format!(
                            "TERYON {} is available: opening the release page",
                            r.version
                        ));
                        uikit::open_url(&r.url);
                    }
                    Ok(Some(r)) => self.toast(format!(
                        "TERYON {} is available (palette: Check for updates)",
                        r.version
                    )),
                    Ok(None) if asked => self.toast(format!(
                        "TERYON {} is up to date",
                        env!("CARGO_PKG_VERSION")
                    )),
                    Err(e) if asked => self.toast(format!("Update check failed: {e}")),
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::tests::app_with_screen;
    use crate::update::Release;

    fn answer(app: &mut Desktop, asked: bool, r: anyhow::Result<Option<Release>>) -> String {
        let (tx, rx) = mpsc::channel();
        tx.send(r).unwrap();
        app.update_check = Some((rx, asked));
        app.toast = None;
        app.update_tick(&egui::Context::default());
        assert!(app.update_check.is_none());
        app.toast.take().map(|t| t.0).unwrap_or_default()
    }

    #[test]
    fn the_answer_is_spoken_when_asked_and_only_news_is_spoken_otherwise() {
        let (mut app, _rx) = app_with_screen("");
        let newer = Release {
            version: "9.9.9".into(),
            url: "https://github.com/zyvorai/teryon/releases".into(),
        };
        assert!(answer(&mut app, true, Ok(None)).contains("up to date"));
        assert!(answer(&mut app, true, Err(anyhow::anyhow!("offline"))).contains("offline"));
        // The automatic check says nothing when there is nothing new or the network is down.
        assert_eq!(answer(&mut app, false, Ok(None)), "");
        assert_eq!(answer(&mut app, false, Err(anyhow::anyhow!("offline"))), "");
        assert!(answer(&mut app, false, Ok(Some(newer))).contains("9.9.9"));
        // Off by default: nothing starts on its own.
        app.update_auto_done = false;
        app.update_tick(&egui::Context::default());
        assert!(app.update_check.is_none() && !app.cfg.check_updates);
    }
}
