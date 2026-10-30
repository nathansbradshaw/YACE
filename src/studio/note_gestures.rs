//! Shared gestures for instrument lanes and the vocal pitch grid.
use super::state::{EditTool, Studio};
use crate::note_groups::{Edit, Selection, Snapshot};
use dioxus::html::input_data::MouseButton;
use dioxus::prelude::*;

#[derive(Clone)]
enum Gesture {
    Box {
        track: String,
        x: f64,
        y: f64,
        page_x: f64,
        page_y: f64,
        additive: bool,
    },
    Move {
        selection: Snapshot,
        tick: u64,
        page_x: f64,
        page_y: f64,
    },
}
#[derive(Clone, Copy)]
pub(super) struct Gestures {
    studio: Studio,
    gesture: Signal<Option<Gesture>>,
    delta: Signal<(f64, f64)>,
    suppress: Signal<bool>,
}
pub(super) fn use_gestures(studio: Studio) -> Gestures {
    Gestures {
        studio,
        gesture: use_signal(|| None),
        delta: use_signal(|| (0., 0.)),
        suppress: use_signal(|| false),
    }
}
impl Gestures {
    pub fn background(mut self, track: &str, event: &MouseEvent) {
        if (self.studio.tool)() != EditTool::Select
            || event.trigger_button() != Some(MouseButton::Primary)
        {
            return;
        }
        event.prevent_default();
        self.studio.selection_dragging.set(true);
        self.delta.set((0., 0.));
        self.suppress.set(false);
        self.gesture.set(Some(Gesture::Box {
            track: track.into(),
            x: event.element_coordinates().x,
            y: event.element_coordinates().y,
            page_x: event.page_coordinates().x,
            page_y: event.page_coordinates().y,
            additive: event.modifiers().shift(),
        }));
    }
    pub fn note(
        mut self,
        track: &str,
        id: Option<&str>,
        index: usize,
        tick: u64,
        event: &MouseEvent,
    ) {
        if (self.studio.tool)() != EditTool::Select
            || event.modifiers().shift()
            || event.trigger_button() != Some(MouseButton::Primary)
        {
            return;
        }
        event.prevent_default();
        self.studio.selection_dragging.set(true);
        if let Some(id) = id {
            self.studio.select_gem(track, id, false, true);
        } else {
            self.studio.select_vocal(index, false, true);
        }
        self.delta.set((0., 0.));
        self.suppress.set(false);
        if let Some(selection) = self.studio.selection_snapshot() {
            self.gesture.set(Some(Gesture::Move {
                selection,
                tick,
                page_x: event.page_coordinates().x,
                page_y: event.page_coordinates().y,
            }));
        }
    }
    pub fn moving(mut self, event: &MouseEvent) {
        let origin = self.gesture.read().as_ref().map(|g| match g {
            Gesture::Box { page_x, page_y, .. } | Gesture::Move { page_x, page_y, .. } => {
                (*page_x, *page_y)
            }
        });
        if let Some((x, y)) = origin {
            self.delta.set((
                event.page_coordinates().x - x,
                event.page_coordinates().y - y,
            ));
        }
    }
    pub fn consume_click(mut self) -> bool {
        self.suppress.replace(false)
    }
    pub fn cancel(mut self) {
        self.gesture.set(None);
        self.delta.set((0., 0.));
        self.studio.selection_dragging.set(false);
    }
    pub fn release(mut self, event: &MouseEvent, width: f64, row_height: f64) {
        if !(self.studio.selection_dragging)() {
            self.gesture.set(None);
            return;
        }
        self.moving(event);
        let Some(gesture) = self.gesture.take() else {
            return;
        };
        self.studio.selection_dragging.set(false);
        let (dx, dy) = (self.delta)();
        if dx.abs() + dy.abs() < 3. {
            return;
        }
        self.suppress.set(true);
        let Some(doc) = self.studio.snapshot() else {
            return;
        };
        let Some(timing) = (self.studio.timing)() else {
            return;
        };
        match gesture {
            Gesture::Move {
                selection, tick, ..
            } => {
                let seconds = timing.seconds(tick) + dx / width * (self.studio.span)();
                let Some(destination) = super::note_canvas::snap(self.studio, seconds) else {
                    self.studio
                        .error
                        .set(Some("The group would start before the chart".into()));
                    return;
                };
                let Ok(ticks) = i64::try_from(destination as i128 - tick as i128) else {
                    return;
                };
                let rows = (dy / row_height).round() as i16;
                if ticks != 0 || rows != 0 {
                    self.studio.edit_group(Edit::Move {
                        selection,
                        ticks,
                        rows,
                    });
                }
            }
            Gesture::Box {
                track,
                x,
                y,
                additive,
                ..
            } => {
                let (left, right) = (x.min(x + dx), x.max(x + dx));
                let (top, bottom) = (y.min(y + dy), y.max(y + dy));
                let mut group = self.studio.selection();
                if !additive || group.track != track {
                    group = Selection {
                        track: track.clone(),
                        ..Selection::default()
                    };
                }
                let hit = |tick, end, row: usize, glyph: f64| {
                    let x = (timing.seconds(tick) - (self.studio.view_start)())
                        / (self.studio.span)()
                        * width;
                    let end = (timing.seconds(end) - (self.studio.view_start)())
                        / (self.studio.span)()
                        * width;
                    let y = row as f64 * row_height;
                    x <= right && end.max(x + glyph) >= left && y + row_height >= top && y <= bottom
                };
                if track == "Vocals" {
                    if let Some(chart) = &doc.content.vocals {
                        group.vocals.extend(
                            chart
                                .notes
                                .iter()
                                .enumerate()
                                .filter(|(_, n)| {
                                    let row = match n.tone {
                                        crate::vocals::VocalTone::Pitched { midi } => {
                                            (84 - midi) as usize
                                        }
                                        _ => 49,
                                    };
                                    hit(n.start, n.end, row, 8.)
                                })
                                .map(|(i, _)| i),
                        );
                    }
                } else if let Some(notes) = doc.content.tracks.get(&track) {
                    let rows = crate::timeline::lane_rows(&track);
                    group.ids.extend(
                        notes
                            .iter()
                            .filter(|n| {
                                rows.iter()
                                    .position(|lane| *lane == n.note.lane)
                                    .is_some_and(|row| {
                                        hit(n.note.tick, n.note.tick + n.note.sustain, row, 14.)
                                    })
                            })
                            .map(|n| n.id.clone()),
                    );
                }
                self.studio.set_selection(group);
            }
        }
    }
    pub fn overlay(self, width: f64, row_height: f64) -> Element {
        let (dx, dy) = (self.delta)();
        match (self.gesture)() {
            Some(Gesture::Box { x, y, .. }) => {
                rsx! { rect { x: "{x.min(x+dx)}", y: "{y.min(y+dy)}", width: "{dx.abs()}", height: "{dy.abs()}", fill: "#e0d7bc", fill_opacity: "0.12", stroke: "#e0d7bc", stroke_width: "1", pointer_events: "none" } }
            }
            Some(Gesture::Move { selection, .. }) => {
                let Some(doc) = self.studio.snapshot() else {
                    return rsx! {};
                };
                let Some(timing) = (self.studio.timing)() else {
                    return rsx! {};
                };
                let positions: Vec<_> = if selection.track == "Vocals" {
                    doc.content
                        .vocals
                        .as_ref()
                        .map(|chart| {
                            chart
                                .notes
                                .iter()
                                .enumerate()
                                .filter(|(i, _)| selection.vocals.contains(i))
                                .map(|(_, n)| {
                                    (
                                        n.start,
                                        n.end,
                                        match n.tone {
                                            crate::vocals::VocalTone::Pitched { midi } => {
                                                (84 - midi) as usize
                                            }
                                            _ => 49,
                                        },
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    let rows = crate::timeline::lane_rows(&selection.track);
                    doc.content.tracks[&selection.track]
                        .iter()
                        .filter(|n| selection.ids.contains(&n.id))
                        .map(|n| {
                            (
                                n.note.tick,
                                n.note.tick + n.note.sustain,
                                rows.iter()
                                    .position(|lane| *lane == n.note.lane)
                                    .unwrap_or(0),
                            )
                        })
                        .collect()
                };
                rsx! { for (tick,end,row) in positions { { let x = (timing.seconds(tick)-(self.studio.view_start)())/(self.studio.span)()*width+dx; let length = (timing.seconds(end)-timing.seconds(tick))/(self.studio.span)()*width;
                    let glyph_width = length.max(if row_height==8. {8.} else {14.});
                    rsx! { rect { x: "{x}", y: "{row as f64*row_height+dy+1.}", width: "{glyph_width}", height: "{row_height-2.}", fill: "none", stroke: "#e0d7bc", pointer_events: "none" } }
                } } }
            }
            None => rsx! {},
        }
    }
}
