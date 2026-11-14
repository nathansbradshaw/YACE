//! Selection stays local; edits and clipboard contents use a guarded transaction.
use super::state::Studio;
#[cfg(not(feature = "browser"))]
use crate::note_groups::Clipboard;
use crate::{
    editor::Action,
    note_groups::{Edit, Selection, Snapshot},
};
#[cfg(not(feature = "browser"))]
use anyhow::{Context, Result};
use dioxus::prelude::*;
#[cfg(not(feature = "browser"))]
use std::{cell::RefCell, rc::Rc};

impl Studio {
    pub fn selection(self) -> Selection {
        let Some(doc) = self.snapshot() else {
            return Selection::default();
        };
        let group = (self.group)();
        if group.track == doc.selected_track && !group.is_empty() {
            return group;
        }
        let mut group = Selection {
            track: doc.selected_track.clone(),
            ..Selection::default()
        };
        if group.track == "Vocals" {
            group.vocals.extend((self.vocal_note)());
        } else {
            group.ids.extend(doc.selected_note.clone());
        }
        group
    }
    pub fn focus_editor(self) {
        if let Some(element) = (self.editor_focus)() {
            spawn(async move {
                let _ = element.set_focus(true).await;
            });
        }
    }
    pub fn set_selection(mut self, group: Selection) {
        let Some(doc) = self.snapshot() else { return };
        if doc.selected_track != group.track {
            self.dispatch(Action::SelectTrack {
                track: group.track.clone(),
            });
        }
        let primary = group.ids.iter().next().cloned();
        if self
            .snapshot()
            .is_some_and(|doc| doc.selected_note != primary)
        {
            self.dispatch(Action::SelectNote { note_id: primary });
        }
        self.vocal_note.set(group.vocals.iter().next().copied());
        self.group.set(group);
        self.focus_editor();
    }
    pub fn select_gem(self, track: &str, id: &str, shift: bool, retain_group: bool) {
        let mut group = self.selection();
        if group.track != track || (!shift && !(retain_group && group.ids.contains(id))) {
            group = Selection {
                track: track.into(),
                ..Selection::default()
            };
        }
        if shift && !group.ids.insert(id.into()) {
            group.ids.remove(id);
        } else {
            group.ids.insert(id.into());
        }
        self.set_selection(group);
    }
    pub fn select_vocal(self, index: usize, shift: bool, retain_group: bool) {
        let mut group = self.selection();
        if group.track != "Vocals" || (!shift && !(retain_group && group.vocals.contains(&index))) {
            group = Selection {
                track: "Vocals".into(),
                ..Selection::default()
            };
        }
        if shift && !group.vocals.insert(index) {
            group.vocals.remove(&index);
        } else {
            group.vocals.insert(index);
        }
        self.set_selection(group);
    }
    pub fn select_all(self) {
        let Some(doc) = self.snapshot() else { return };
        let mut group = Selection {
            track: doc.selected_track.clone(),
            ..Selection::default()
        };
        if group.track == "Vocals" {
            group.vocals.extend(
                0..doc
                    .content
                    .vocals
                    .as_ref()
                    .map_or(0, |chart| chart.notes.len()),
            );
        } else if let Some(notes) = doc.content.tracks.get(&group.track) {
            group.ids.extend(
                notes
                    .iter()
                    .filter(|n| doc.playable(&group.track, n.note.lane))
                    .map(|n| n.id.clone()),
            );
        }
        self.set_selection(group);
    }
    pub fn clear_selection(mut self) {
        self.selection_dragging.set(false);
        if let Some(doc) = self.snapshot() {
            self.set_selection(Selection {
                track: doc.selected_track.clone(),
                ..Selection::default()
            });
        }
    }
    pub fn selection_snapshot(mut self) -> Option<Snapshot> {
        let project = self.snapshot()?;
        let selection = self.selection();
        if selection.is_empty() {
            return None;
        }
        match Snapshot::capture(&project, &selection) {
            Ok(snapshot) => Some(snapshot),
            Err(error) => {
                self.error.set(Some(format!("{error:#}")));
                None
            }
        }
    }
    pub fn delete_selection(self) {
        if let Some(selection) = self.selection_snapshot() {
            self.edit_group(Edit::Delete { selection });
        }
    }
    pub fn edit_group(mut self, edit: Edit) {
        let Some(before) = self.snapshot() else {
            return;
        };
        let root = self.store.read().as_ref().map(|s| s.root.clone());
        let selection = self.selection();
        let Some(guard) = super::activity::BusyGuard::start(self.group_busy) else {
            return;
        };
        let (count, active, done) = match &edit {
            Edit::Repair { selection, .. } => (
                selection.ids.len() + selection.vocals.len(),
                "Repairing",
                "Repaired",
            ),
            Edit::Move { selection, .. } => (
                selection.ids.len() + selection.vocals.len(),
                "Moving",
                "Moved",
            ),
            Edit::Delete { selection } => (
                selection.ids.len() + selection.vocals.len(),
                "Deleting",
                "Deleted",
            ),
            Edit::Paste { clipboard, .. } => (
                match &clipboard.payload {
                    crate::note_groups::Payload::Instrument { notes, .. } => notes.len(),
                    crate::note_groups::Payload::Vocals { chart } => chart.notes.len(),
                },
                "Pasting",
                "Pasted",
            ),
        };
        self.status.set(format!("{active} {count} notes…"));
        spawn(async move {
            let _guard = guard;
            // Paint the status before the bounded transaction, including when
            // a native window is hidden (requestAnimationFrame may suspend).
            #[cfg(target_arch = "wasm32")]
            gloo_timers::future::TimeoutFuture::new(20).await;
            #[cfg(not(target_arch = "wasm32"))]
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            if self.store.read().as_ref().map(|s| s.root.clone()) != root
                || self
                    .snapshot()
                    .is_none_or(|doc| doc.revision != before.revision)
                || self.selection() != selection
            {
                self.error.set(Some("The project or selection changed before the operation started. Select notes again; nothing was changed.".into()));
                self.status
                    .set("Note operation cancelled · project or selection changed".into());
                return;
            }
            let accepted = self.finish_edit_group(edit);
            self.status.set(if !accepted {
                "Note operation failed · no edit was committed".into()
            } else if (self.error)().is_some() {
                "Notes edited; selection refresh failed · see error details".into()
            } else {
                format!("{done} {count} notes")
            });
        });
    }
    fn finish_edit_group(self, edit: Edit) -> bool {
        let Some(before) = self.snapshot() else {
            return false;
        };
        let chosen = self.selection();
        // Sorting can change vocal indices. Recover moved notes by value.
        let moved_vocals: std::collections::BTreeMap<_, _> =
            if matches!(&edit, Edit::Move { .. } | Edit::Repair { .. }) {
                before
                    .content
                    .vocals
                    .as_ref()
                    .map(|chart| {
                        chosen
                            .vocals
                            .iter()
                            .filter_map(|&i| {
                                let mut note = chart.notes.get(i)?.clone();
                                match &edit {
                                    Edit::Move { ticks, rows, .. } => {
                                        note.start = note.start.checked_add_signed(*ticks)?;
                                        note.end = note.end.checked_add_signed(*ticks)?;
                                        if let crate::vocals::VocalTone::Pitched { midi } =
                                            &mut note.tone
                                        {
                                            *midi = u8::try_from(*midi as i16 - rows).ok()?;
                                        }
                                    }
                                    Edit::Repair { selection, policy }
                                        if selection.vocals.contains(&i) =>
                                    {
                                        let grid = policy.grid(chart.resolution).ok()?;
                                        let after = policy.destination(grid, note.start).ok()?;
                                        note.end = after.checked_add(note.end - note.start)?;
                                        note.start = after;
                                    }
                                    _ => {}
                                }
                                Some((note.start, note))
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Default::default()
            };
        let moving = matches!(&edit, Edit::Move { .. } | Edit::Repair { .. });
        let pasting = matches!(&edit, Edit::Paste { .. });
        if !self.try_dispatch(Action::EditNoteGroup { edit }) {
            return false;
        }
        let Some(after) = self.snapshot() else {
            return true;
        };
        let mut group = Selection {
            track: after.selected_track.clone(),
            ..Selection::default()
        };
        if moving {
            group.ids = chosen.ids;
            if let Some(chart) = &after.content.vocals {
                group.vocals.extend(
                    chart
                        .notes
                        .iter()
                        .enumerate()
                        .filter(|(_, n)| moved_vocals.get(&n.start) == Some(n))
                        .map(|(i, _)| i),
                );
            }
        } else if pasting {
            if group.track == "Vocals" {
                if let Some(chart) = &after.content.vocals {
                    let old: std::collections::BTreeMap<_, _> = before
                        .content
                        .vocals
                        .as_ref()
                        .into_iter()
                        .flat_map(|c| c.notes.iter().map(|n| (n.start, n)))
                        .collect();
                    group.vocals.extend(
                        chart
                            .notes
                            .iter()
                            .enumerate()
                            .filter(|(_, n)| old.get(&n.start) != Some(n))
                            .map(|(i, _)| i),
                    );
                }
            } else if let Some(notes) = after.content.tracks.get(&group.track) {
                let old: std::collections::BTreeSet<_> = before
                    .content
                    .tracks
                    .values()
                    .flatten()
                    .map(|n| n.id.as_str())
                    .collect();
                group.ids.extend(
                    notes
                        .iter()
                        .filter(|n| {
                            after.playable(&group.track, n.note.lane)
                                && !old.contains(n.id.as_str())
                        })
                        .map(|n| n.id.clone()),
                );
            }
        }
        self.set_selection(group);
        true
    }
    #[cfg(not(feature = "browser"))]
    pub fn copy_drum_range(mut self, range: crate::drum_ranges::Range) {
        let result = (|| -> Result<()> {
            let text = super::drum_range_editor::copy_text(self, &range)?;
            self.with_clipboard(|clipboard| Ok(clipboard.set_text(text)?))?;
            Ok(())
        })();
        match result {
            Ok(()) => self.status.set("Drum range copied".into()),
            Err(error) => self.error.set(Some(format!("Clipboard: {error:#}"))),
        }
    }
    #[cfg(not(feature = "browser"))]
    pub fn paste_drum_ranges(mut self) {
        let result = (|| -> Result<()> {
            let (bound, at) = super::drum_range_editor::prepare_paste(self)?;
            let text = self.with_clipboard(|clipboard| Ok(clipboard.get_text()?))?;
            super::drum_range_editor::finish_paste(self, bound, at, &text)
        })();
        if let Err(error) = result {
            self.error.set(Some(format!("Clipboard: {error:#}")));
        }
    }
    #[cfg(not(feature = "browser"))]
    fn with_clipboard<T>(
        mut self,
        operation: impl FnOnce(&mut arboard::Clipboard) -> Result<T>,
    ) -> Result<T> {
        if (self.clipboard)().is_none() {
            self.clipboard
                .set(Some(Rc::new(RefCell::new(arboard::Clipboard::new()?))));
        }
        let clipboard = (self.clipboard)().context("Clipboard unavailable")?;
        operation(&mut clipboard.borrow_mut())
    }
    #[cfg(not(feature = "browser"))]
    pub fn copy_selection(mut self, cut: bool) {
        if (self.group_busy)() {
            return;
        }
        let result = (|| {
            let doc = self.snapshot().context("Open a chart first")?;
            let timing = (self.timing)().context("Chart timing unavailable")?;
            let text = crate::note_groups::copy(&doc, &timing, &self.selection())?.encode()?;
            self.with_clipboard(|clipboard| {
                clipboard.set_text(text)?;
                Ok(())
            })?;
            Ok::<_, anyhow::Error>(self.selection().len())
        })();
        match result {
            Ok(count) => {
                self.error.set(None);
                if cut {
                    self.delete_selection();
                } else if (self.error)().is_none() {
                    self.status.set(format!("Copied {count} notes"));
                }
            }
            Err(error) => self.error.set(Some(format!("{error:#}"))),
        }
    }
    #[cfg(not(feature = "browser"))]
    pub fn paste_selection(mut self) {
        if (self.group_busy)() {
            return;
        }
        let result = (|| {
            let text = self.with_clipboard(|clipboard| Ok(clipboard.get_text()?))?;
            let clipboard = Clipboard::decode(&text)?;
            let track = self
                .snapshot()
                .context("Open a chart first")?
                .selected_track
                .clone();
            let tick = super::note_canvas::snap(self, (self.playhead)())
                .context("Paste point is outside the chart")?;
            Ok::<_, anyhow::Error>(Edit::Paste {
                track,
                tick,
                clipboard,
            })
        })();
        match result {
            Ok(edit) => self.edit_group(edit),
            Err(error) => self.error.set(Some(format!("{error:#}"))),
        }
    }
}
