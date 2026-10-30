//! Context follows the selected note. Song metadata and review stay close by.
use super::{
    state::{Studio, clock},
    track_picker,
};
use crate::editor::{Action, DecisionStatus, EditorNote};
use dioxus::prelude::*;

fn update(studio: Studio, id: &str, apply: impl FnOnce(&mut crate::chart::Note, &mut bool)) {
    let Some(document) = studio.snapshot() else {
        return;
    };
    let Some((track, item)) = document.find_note(id) else {
        return;
    };
    let mut note = item.note.clone();
    let mut cymbal = document.is_cymbal(track, &note);
    apply(&mut note, &mut cymbal);
    cymbal &= track.ends_with("Drums") && (2..=4).contains(&note.lane);
    studio.dispatch(Action::UpdateNote {
        note_id: id.into(),
        tick: note.tick,
        lane: note.lane,
        sustain: note.sustain,
        cymbal,
    });
}

#[component]
fn NoteSelection(studio: Studio, track: String, item: EditorNote) -> Element {
    let tick_id = item.id.clone();
    let length_id = item.id.clone();
    let lane_id = item.id.clone();
    let cymbal_id = item.id.clone();
    let delete_id = item.id.clone();
    let drums = track.ends_with("Drums");
    let cymbal = studio
        .snapshot()
        .is_some_and(|doc| doc.is_cymbal(&track, &item.note));
    let seconds = studio
        .timing
        .read()
        .as_ref()
        .map_or(0., |map| map.seconds(item.note.tick));
    rsx! {
        section { class: "selection-note", aria_label: "Selected note",
            p { class: "selection-caption", "{track_picker::title(&track)} · {clock(seconds)}" }
            div { class: "selection-timing",
                label { "Start tick"
                    input { aria_label: "Note position in ticks", r#type: "number", min: "0", value: "{item.note.tick}", onkeydown: move |event| event.stop_propagation(), onchange: move |event| { if let Ok(value) = event.value().parse() { update(studio, &tick_id, |note, _| note.tick = value); } } }
                }
                label { "Length ticks"
                    input { aria_label: "Note duration in ticks", r#type: "number", min: "0", value: "{item.note.sustain}", disabled: drums, onkeydown: move |event| event.stop_propagation(), onchange: move |event| { if let Ok(value) = event.value().parse() { update(studio, &length_id, |note, _| note.sustain = value); } } }
                }
            }
            label { class: "selection-lane", "Lane"
                select { aria_label: "Note lane", value: "{item.note.lane}", onkeydown: move |event| event.stop_propagation(), onchange: move |event| { if let Ok(value) = event.value().parse() { update(studio, &lane_id, |note, _| note.lane = value); } },
                    for lane in crate::timeline::lane_rows(&track) { option { value: "{lane}", selected: lane == item.note.lane, "{super::note_canvas::lane_name(&track, lane)}" } }
                }
            }
            div { class: "selection-actions",
                if drums && (2..=4).contains(&item.note.lane) { label { class: "form-check",
                    input { r#type: "checkbox", checked: cymbal, onkeydown: move |event| event.stop_propagation(), onchange: move |event| update(studio, &cymbal_id, |_, cymbal| *cymbal = event.checked()) } "Cymbal"
                } }
                button { class: "selection-delete", onclick: move |_| studio.dispatch(Action::DeleteNote { note_id: delete_id.clone() }), "Delete note" }
            }
            details { class: "context-fold", summary { "Chart source" } super::difficulty::NoteAncestry { studio, note_id: item.id.clone() } }
        }
    }
}

#[component]
fn ReviewQueue(mut studio: Studio) -> Element {
    let Some(document) = studio.snapshot() else {
        return rsx! {};
    };
    let timing = (studio.timing)();
    let pending: Vec<_> = document
        .review
        .regions
        .iter()
        .filter(|region| {
            region.tracks.contains(&document.selected_track)
                && !document.is_flag_hidden(&region.id, &document.selected_track)
                && timing.as_ref().is_some_and(|map| {
                    document
                        .decision(&region.id, &document.selected_track, map)
                        .is_none_or(|decision| decision.status == DecisionStatus::Pending)
                })
        })
        .collect();
    rsx! {
        section { class: "context-review", aria_label: "Review queue",
            div { class: "context-review-heading", h3 { "Review" } span { "{pending.len()} pending" } }
            if !pending.is_empty() {
                ol { class: "context-review-list",
                    for region in pending {
                        { let id = region.id.clone(); let reasons = region.issues.iter().filter(|issue| issue.track == document.selected_track).map(|issue| issue.reason.label()).collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>().join(", ");
                          rsx! { li { button { aria_pressed: (document.selected_region.as_ref() == Some(&region.id)).to_string(), title: "{reasons}", onclick: move |_| studio.dispatch(Action::SelectRegion { region_id: id.clone() }), time { "{clock(region.start)}" } span { "{reasons}" } } } }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn SelectionDesk(studio: Studio) -> Element {
    let Some(document) = studio.snapshot() else {
        return rsx! {};
    };
    let selected = document
        .selected_note
        .as_ref()
        .and_then(|id| document.find_note(id));
    let vocals = document.selected_track == "Vocals";
    let count = studio.selection().len();
    let tempo = studio
        .timing
        .read()
        .as_ref()
        .and_then(|map| map.tempos.first())
        .map_or(0., |tempo| tempo.bpm);
    rsx! {
        aside { class: "selection-desk", aria_label: "Selection and review", onkeydown: super::shortcuts::keep_space_local,
            header { class: "desk-heading", h2 { if vocals { "Vocals" } else { "Selection" } } }
            div { class: "selection-desk-content",
                if let Some(crate::parts::Origin::Generated { .. }) = document.content.track_origins.get(&document.selected_track) {
                    p { class: "field-hint", "Generated draft · listen and review this part." }
                }
                if count > 1 {
                    section { class: "selection-note", aria_label: "Selected notes",
                        p { class: "selection-caption", "{count} notes selected" }
                        p { class: "field-hint", "Drag to move. Shift-click to add or remove." }
                        div { class: "selection-actions",
                            button { onclick: move |_| studio.copy_selection(false), "Copy" }
                            button { onclick: move |_| studio.delete_selection(), "Delete" }
                        }
                    }
                } else if !vocals {
                    if let Some((track, item)) = selected { NoteSelection { studio, track: track.to_owned(), item: item.clone() } }
                    else { p { class: "selection-empty", "Click a note or drag a box. Shift-click adds notes." } }
                }
                div { hidden: !vocals || count > 1, super::vocal_editor::VocalEditor { studio } }
                ReviewQueue { studio }
                details { class: "context-fold", summary { "Song details" }
                    dl { class: "context-metadata",
                        for (key, label) in [("album", "Album"), ("year", "Year"), ("genre", "Genre")] {
                            dt { "{label}" } dd { { let value = studio.metadata_value(key); if value.is_empty() { "—".into() } else { value } } }
                        }
                        dt { "BPM" } dd { "{tempo:.3}" }
                        dt { "Length" } dd { "{clock(document.duration)}" }
                    }
                }
                super::difficulty::DifficultyControls { studio }
            }
        }
    }
}
