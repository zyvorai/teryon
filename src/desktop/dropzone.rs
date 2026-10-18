//! Drag-and-drop: files dropped on a pane are typed as quoted paths (local host) or uploaded to the
//! pane's directory (SSH host). Existing remote files are never overwritten.
use super::*;
use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum DropPlan {
    /// Type these shell-quoted paths at the cursor.
    Paste(String),
    Upload {
        files: Vec<PathBuf>,
        dest: String,
    },
    Refuse(String),
}
/// Decide what a drop means for a pane whose shell is in `cwd`.
pub(super) fn plan_drop(local: bool, files: &[PathBuf], cwd: &str) -> DropPlan {
    if files.is_empty() {
        return DropPlan::Refuse("Nothing to drop".into());
    }
    if local {
        let quoted: Vec<String> = files
            .iter()
            .map(|f| crate::util::shell_quote(&f.to_string_lossy()))
            .collect();
        return DropPlan::Paste(quoted.join(" "));
    }
    if files.len() > 50 {
        return DropPlan::Refuse("Drop at most 50 files at a time".into());
    }
    if !cwd.starts_with('/') || cwd.contains(['\n', '\0']) {
        return DropPlan::Refuse("Can't tell this pane's directory on the host".into());
    }
    DropPlan::Upload {
        files: files.to_vec(),
        dest: cwd.to_string(),
    }
}

impl Desktop {
    /// Pane under the pointer, else the focused one.
    fn drop_target(&self, ctx: &egui::Context) -> u64 {
        let at = ctx.input(|i| i.pointer.hover_pos().or(i.pointer.latest_pos()));
        at.and_then(|p| {
            self.pane_rects
                .iter()
                .find(|(_, r)| r.contains(p))
                .map(|(id, _)| *id)
        })
        .unwrap_or(self.focus)
    }
    /// Handle files dropped on the window, and show a hint while files are hovering.
    pub(super) fn handle_drops(&mut self, ctx: &egui::Context) {
        while let Ok((_ok, message)) = self.upload_rx.try_recv() {
            self.toast(message);
        }
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering {
            let id = self.drop_target(ctx);
            let verb = if self.host == "local" {
                "Drop to type the path"
            } else {
                "Drop to upload to"
            };
            let cwd = self
                .panes
                .get(&id)
                .map_or(String::new(), |p| p.info.cwd.clone());
            let t = self.theme.clone();
            egui::Area::new(egui::Id::new("teryon-drop-hint"))
                .anchor(egui::Align2::CENTER_BOTTOM, [0., -24.])
                .interactable(false)
                .order(egui::Order::Tooltip)
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(color(t.accent))
                        .corner_radius(egui::CornerRadius::same(10))
                        .inner_margin(egui::Margin::symmetric(14, 8))
                        .show(ui, |ui| {
                            let text = if self.host == "local" {
                                verb.to_string()
                            } else {
                                format!("{verb} {cwd}")
                            };
                            ui.label(RichText::new(text).color(Color32::WHITE));
                        });
                });
        }
        let files: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if files.is_empty() {
            return;
        }
        let id = self.drop_target(ctx);
        self.focus = id;
        let cwd = self
            .panes
            .get(&id)
            .map_or(String::new(), |p| p.info.cwd.clone());
        match plan_drop(self.host == "local", &files, &cwd) {
            DropPlan::Paste(text) => {
                let bracketed = self
                    .panes
                    .get(&id)
                    .is_some_and(|p| p.parser.screen().bracketed_paste());
                self.input(keys::paste(&text, bracketed));
            }
            DropPlan::Refuse(why) => self.toast(why),
            DropPlan::Upload { files, dest } => {
                let Some(target) = self.target.clone() else {
                    self.toast("Uploads are unavailable in this session");
                    return;
                };
                if self.readonly {
                    self.toast("Read-only attachment: nothing uploaded");
                    return;
                }
                self.toast(format!(
                    "Uploading {} file(s) to {}:{dest}…",
                    files.len(),
                    target.host
                ));
                let tx = self.upload_tx.clone();
                let n = files.len();
                thread::spawn(move || {
                    let msg = match target.upload_files(&files, &dest) {
                        Ok(()) => (
                            true,
                            format!(
                                "Uploaded {n} file(s) to {dest} (existing files were left alone)"
                            ),
                        ),
                        Err(e) => (false, format!("{e:#}")),
                    };
                    let _ = tx.send(msg);
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_drops_type_quoted_paths_and_remote_drops_upload() {
        let files = vec![
            PathBuf::from("/Users/me/my file.txt"),
            PathBuf::from("/tmp/it's.png"),
        ];
        match plan_drop(true, &files, "/ignored") {
            DropPlan::Paste(t) => assert_eq!(t, "'/Users/me/my file.txt' '/tmp/it'\\''s.png'"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            plan_drop(false, &files, "/srv/app"),
            DropPlan::Upload {
                files: files.clone(),
                dest: "/srv/app".into()
            }
        );
        assert!(matches!(
            plan_drop(false, &files, "relative/dir"),
            DropPlan::Refuse(_)
        ));
        assert!(matches!(
            plan_drop(false, &files, "/a\nb"),
            DropPlan::Refuse(_)
        ));
        assert!(matches!(plan_drop(true, &[], "/"), DropPlan::Refuse(_)));
        let many: Vec<PathBuf> = (0..51).map(|i| PathBuf::from(format!("/f{i}"))).collect();
        assert!(matches!(plan_drop(false, &many, "/x"), DropPlan::Refuse(_)));
    }
}
