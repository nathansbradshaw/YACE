//! OCTAVE interface adaptation. Original copyright and license: assets/octave/README.md.
use super::{
    icons::Icon,
    state::{Studio, clock},
};
use crate::editor::{Action, EditorNote};
use dioxus::prelude::*;
fn change_note(
    studio: Studio,
    note_id: &str,
    apply: impl FnOnce(&mut crate::chart::Note, &mut bool),
) {
    let Some(doc) = studio.snapshot() else { return };
    let Some((track, item)) = doc.find_note(note_id) else {
        return;
    };
    let mut note = item.note.clone();
    let mut cymbal = doc.is_cymbal(track, &note);
    apply(&mut note, &mut cymbal);
    if !track.ends_with("Drums") || !(2..=4).contains(&note.lane) {
        cymbal = false;
    }
    studio.dispatch(Action::UpdateNote {
        note_id: note_id.into(),
        tick: note.tick,
        lane: note.lane,
        sustain: note.sustain,
        cymbal,
    });
}

#[component]
fn NoteProperties(studio: Studio, item: EditorNote, track: String) -> Element {
    let note = &item.note;
    let tick_id = item.id.clone();
    let length_id = item.id.clone();
    let lane_id = item.id.clone();
    let cymbal_id = item.id.clone();
    let delete_id = item.id.clone();
    let drums = track.ends_with("Drums");
    let cymbal = studio
        .snapshot()
        .is_some_and(|doc| doc.is_cymbal(&track, note));
    let start_seconds = studio
        .timing
        .read()
        .as_ref()
        .map_or(0.0, |timing| timing.seconds(note.tick));
    rsx! {
        div { class: "note-editor",
            div { class: "property-section",
                div { class: "property-section-title", "Selected Note" }
                super::difficulty::NoteAncestry { studio, note_id: item.id.clone() }
                div { class: "property-group",
                    label { class: "property-label", "Position (Tick)" }
                    input { aria_label: "Note position in ticks", onkeydown: move |event| event.stop_propagation(), class: "property-input", r#type: "number", min: "0", value: "{note.tick}",
                        oninput: move |event| { if let Ok(tick) = event.value().parse() { change_note(studio, &tick_id, |note, _| note.tick = tick); } }
                    }
                    div { class: "difficulty-hint", "{clock(start_seconds)}" }
                }
                div { class: "property-group",
                    label { class: "property-label", "Duration (Ticks)" }
                    input { aria_label: "Note duration in ticks", onkeydown: move |event| event.stop_propagation(), class: "property-input", r#type: "number", min: "0", value: "{note.sustain}", disabled: drums,
                        oninput: move |event| { if let Ok(sustain) = event.value().parse() { change_note(studio, &length_id, |note, _| note.sustain = sustain); } }
                    }
                }
                div { class: "property-group",
                    label { class: "property-label", "Lane" }
                    select { aria_label: "Note lane", onkeydown: move |event| event.stop_propagation(), class: "property-select", value: "{note.lane}",
                        onchange: move |event| { if let Ok(lane) = event.value().parse() { change_note(studio, &lane_id, |note, _| note.lane = lane); } },
                        for lane in crate::timeline::lane_rows(&track) { option { value: "{lane}", selected: lane == note.lane, "{lane_name(&track, lane)}" } }
                    }
                }
                if drums && (2..=4).contains(&note.lane) {
                    div { class: "property-group",
                        label { class: "property-label", "Flags" }
                        div { class: "property-flags",
                            label { class: "property-checkbox",
                                input { onkeydown: move |event| event.stop_propagation(), r#type: "checkbox", checked: cymbal, onchange: move |event| change_note(studio, &cymbal_id, |_, cymbal| *cymbal = event.checked()) }
                                span { "Cymbal" }
                            }
                        }
                    }
                }
                div { class: "property-actions",
                    button { class: "property-button property-button-danger", onclick: move |_| studio.dispatch(Action::DeleteNote { note_id: delete_id.clone() }), "Delete Note" }
                }
            }
        }
    }
}

fn lane_name(track: &str, lane: u8) -> &'static str {
    if crate::keys::is_pro(track) {
        crate::keys::pitch_name(lane)
    } else if track.ends_with("Drums") {
        match lane {
            0 => "Kick",
            1 => "Snare",
            2 => "Yellow",
            3 => "Blue",
            _ => "Green",
        }
    } else {
        match lane {
            0 => "Green",
            1 => "Red",
            2 => "Yellow",
            3 => "Blue",
            4 => "Orange",
            _ => "Open",
        }
    }
}

#[component]
pub fn Properties(mut studio: Studio) -> Element {
    let doc = studio.snapshot();
    let item = doc
        .as_ref()
        .and_then(|d| d.selected_note.as_ref().and_then(|id| d.find_note(id)))
        .map(|(track, item)| (track.to_owned(), item.clone()));
    let note_tab = (studio.property_tab)().eq_ignore_ascii_case("note");
    let vocal_tab = note_tab
        && doc
            .as_ref()
            .is_some_and(|doc| doc.selected_track == "Vocals");
    let has_note = item.is_some()
        || doc
            .as_ref()
            .is_some_and(|doc| doc.selected_track == "Vocals");
    let fields = ["album", "year", "genre"];
    let tempo = studio
        .timing
        .read()
        .as_ref()
        .and_then(|t| t.tempos.first())
        .map_or(0.0, |t| t.bpm);
    rsx! {
        div { class: "property-panel",
            div { class: "panel-header", span { class: "panel-header-title", span { Icon { name: "sliders" } } span { "Inspector" } } }
            div { class: "panel-tabs",
                button { class: if note_tab || vocal_tab { "panel-tab" } else { "panel-tab active" }, onclick: move |_| studio.set_property_tab("Song"), "Song" }
                button { class: if note_tab { "panel-tab active" } else { "panel-tab" }, disabled: !has_note, onclick: move |_| studio.set_property_tab("Note"), "Note" }
            }
            div { class: "panel-content",
                if doc.is_none() {
                    div { class: "empty-state", div { class: "empty-state-description", "Open a song to inspect its properties." } }
                } else if note_tab && !vocal_tab {
                    if let Some((track, item)) = item { NoteProperties { studio, track, item } }
                    else { div { class: "empty-state", div { class: "empty-state-description", "Select a note in the piano roll." } } }
                } else if !vocal_tab {
                    div { class: "metadata-editor",
                        div { class: "property-section",
                            div { class: "property-section-title", "Song details" }
                            dl { class: "song-details", title: "Source metadata · read only",
                                for field in fields { dt { "{field_label(field)}" } dd { {let value=studio.metadata_value(field);if value.is_empty(){"—".into()}else{value}} } }
                                dt { "BPM" } dd { "{tempo:.3}" }
                                if let Some(doc)=doc.as_ref() { dt { "Length" } dd { "{clock(doc.duration)}" } }
                            }
                        }
                    }
                }
                if doc.is_some(){div{hidden:!vocal_tab,super::vocal_editor::VocalEditor {studio}}}
                super::difficulty::DifficultyControls { studio }
            }
        }
    }
}

fn field_label(field: &str) -> &'static str {
    match field {
        "name" => "Title",
        "artist" => "Artist",
        "album" => "Album",
        "year" => "Year",
        _ => "Genre",
    }
}
