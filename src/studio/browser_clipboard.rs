//! Browser clipboard access around the same portable, guarded group commands.
use super::state::Studio;
use crate::note_groups::{Clipboard, Edit, Snapshot};
use anyhow::{Context, Result, ensure};
use dioxus::prelude::*;

impl Studio {
    pub fn copy_selection(mut self, cut: bool) {
        let prepared = (|| -> Result<_> {
            let project = self.snapshot().context("Open a chart first")?;
            let root = self
                .store
                .read()
                .as_ref()
                .context("Open a chart first")?
                .root
                .clone();
            let timing = (self.timing)().context("Chart timing unavailable")?;
            let selection = self.selection();
            let text = crate::note_groups::copy(&project, &timing, &selection)?.encode()?;
            let snapshot = Snapshot::capture(&project, &selection)?;
            Ok((
                root,
                project.selected_track.clone(),
                text,
                snapshot,
                selection.len(),
            ))
        })();
        let (root, track, text, selection, count) = match prepared {
            Ok(value) => value,
            Err(error) => {
                self.error.set(Some(format!("{error:#}")));
                return;
            }
        };
        spawn(async move {
            let result = async {
                let script = format!(
                    "await navigator.clipboard.writeText({}); return true;",
                    serde_json::to_string(&text)?
                );
                dioxus_document::eval(&script).await?;
                ensure!(
                    self.store
                        .read()
                        .as_ref()
                        .is_some_and(|store| store.root == root)
                        && self
                            .snapshot()
                            .is_some_and(|project| project.selected_track == track),
                    "The project or track changed while copying; no notes were cut"
                );
                self.error.set(None);
                if cut {
                    self.edit_group(Edit::Delete { selection });
                }
                if (self.error)().is_none() {
                    self.status.set(format!(
                        "{} {count} notes",
                        if cut { "Cut" } else { "Copied" }
                    ));
                }
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if let Err(error) = result {
                self.error.set(Some(format!("Clipboard: {error:#}")));
            }
        });
    }
    pub fn paste_selection(mut self) {
        let prepared = (|| -> Result<_> {
            let project = self.snapshot().context("Open a chart first")?;
            let root = self
                .store
                .read()
                .as_ref()
                .context("Open a chart first")?
                .root
                .clone();
            let tick = super::note_canvas::snap(self, (self.playhead)())
                .context("Paste point is outside the chart")?;
            Ok((root, project.selected_track.clone(), tick))
        })();
        let (root, track, tick) = match prepared {
            Ok(value) => value,
            Err(error) => {
                self.error.set(Some(format!("{error:#}")));
                return;
            }
        };
        spawn(async move {
            let result = async {
                let value =
                    dioxus_document::eval("return await navigator.clipboard.readText();").await?;
                let text = value
                    .as_str()
                    .context("The browser clipboard did not return text")?;
                let clipboard = Clipboard::decode(text)?;
                ensure!(
                    self.store
                        .read()
                        .as_ref()
                        .is_some_and(|store| store.root == root)
                        && self
                            .snapshot()
                            .is_some_and(|project| project.selected_track == track),
                    "The project or track changed while reading the clipboard; paste again"
                );
                self.edit_group(Edit::Paste {
                    track,
                    tick,
                    clipboard,
                });
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if let Err(error) = result {
                self.error.set(Some(format!("Clipboard: {error:#}")));
            }
        });
    }
}
