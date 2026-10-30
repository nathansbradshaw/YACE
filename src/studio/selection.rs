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
                    .filter(|n| crate::editor::playable(&group.track, n.note.lane))
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
    pub fn selection_snapshot(self) -> Option<Snapshot> {
        let project = self.snapshot()?;
        Snapshot::capture(&project, &self.selection()).ok()
    }
    pub fn delete_selection(self) {
        if let Some(selection) = self.selection_snapshot() {
            self.edit_group(Edit::Delete { selection });
        }
    }
    pub fn edit_group(self, edit: Edit) {
        let Some(before) = self.snapshot() else {
            return;
        };
        let chosen = self.selection();
        // Sorting can change vocal indices. Recover moved notes by value.
        let moved_vocals: Vec<_> = if let Edit::Move { ticks, rows, .. } = &edit {
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
                            note.start = note.start.checked_add_signed(*ticks)?;
                            note.end = note.end.checked_add_signed(*ticks)?;
                            if let crate::vocals::VocalTone::Pitched { midi } = &mut note.tone {
                                *midi = u8::try_from(*midi as i16 - rows).ok()?;
                            }
                            Some(note)
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        self.dispatch(Action::EditNoteGroup { edit: edit.clone() });
        if (self.error)().is_some() {
            return;
        }
        let Some(after) = self.snapshot() else { return };
        let mut group = Selection {
            track: after.selected_track.clone(),
            ..Selection::default()
        };
        match edit {
            Edit::Move { .. } => {
                group.ids = chosen.ids;
                if let Some(chart) = &after.content.vocals {
                    group.vocals.extend(
                        chart
                            .notes
                            .iter()
                            .enumerate()
                            .filter(|(_, n)| moved_vocals.contains(n))
                            .map(|(i, _)| i),
                    );
                }
            }
            Edit::Paste { .. } => {
                if group.track == "Vocals" {
                    if let Some(chart) = &after.content.vocals {
                        group.vocals.extend(
                            chart
                                .notes
                                .iter()
                                .enumerate()
                                .filter(|(_, n)| {
                                    before
                                        .content
                                        .vocals
                                        .as_ref()
                                        .is_none_or(|old| !old.notes.contains(n))
                                })
                                .map(|(i, _)| i),
                        );
                    }
                } else if let Some(notes) = after.content.tracks.get(&group.track) {
                    group.ids.extend(
                        notes
                            .iter()
                            .filter(|n| {
                                crate::editor::playable(&group.track, n.note.lane)
                                    && before.find_note(&n.id).is_none()
                            })
                            .map(|n| n.id.clone()),
                    );
                }
            }
            Edit::Delete { .. } => {}
        }
        self.set_selection(group);
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
                }
                if (self.error)().is_none() {
                    self.status.set(format!(
                        "{} {count} notes",
                        if cut { "Cut" } else { "Copied" }
                    ));
                }
            }
            Err(error) => self.error.set(Some(format!("{error:#}"))),
        }
    }
    #[cfg(not(feature = "browser"))]
    pub fn paste_selection(mut self) {
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
