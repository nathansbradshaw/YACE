//! Context follows the selected note. Song metadata and review stay close by.
use super::{
    state::{Studio, clock},
    track_picker,
};
use crate::editor::{Action, DecisionStatus, EditorNote};
use dioxus::prelude::*;

fn update(
    studio: Studio,
    id: &str,
    apply: impl FnOnce(&mut crate::chart::Note, &mut bool),
) -> bool {
    let Some(document) = studio.snapshot() else {
        return false;
    };
    let Some((track, item)) = document.find_note(id) else {
        return false;
    };
    let mut note = item.note.clone();
    let mut cymbal = document.is_cymbal(track, &note);
    apply(&mut note, &mut cymbal);
    cymbal &= track.ends_with("Drums") && (2..=4).contains(&note.lane);
    studio.try_dispatch(Action::UpdateNote {
        note_id: id.into(),
        tick: note.tick,
        lane: note.lane,
        sustain: note.sustain,
        cymbal,
    })
}

fn update_pro(studio: Studio, id: &str, change: impl FnOnce(&mut u8, &mut u8, &mut bool)) -> bool {
    let Some(doc) = studio.snapshot() else {
        return false;
    };
    let Some((track, note)) = doc.find_note(id) else {
        return false;
    };
    if !crate::pro_guitar::is_track(track) || note.note.lane >= 6 {
        return false;
    }
    let mut fret = crate::pro_guitar::fret(note);
    let mut channel = crate::pro_guitar::channel(note);
    let mut forced_hopo = crate::pro_guitar::forced(&doc.content.tracks[track], note.note.tick);
    change(&mut fret, &mut channel, &mut forced_hopo);
    studio.try_dispatch(Action::EditProGuitarNote {
        note_id: id.into(),
        fret,
        channel,
        forced_hopo,
    })
}

#[component]
fn ProGuitarProperties(studio: Studio, track: String, item: EditorNote) -> Element {
    let fret_reset = super::field_reset::use_field_reset();
    let channel_reset = super::field_reset::use_field_reset();
    let hopo_reset = super::field_reset::use_field_reset();
    let Some(limit) = crate::pro_guitar::max_fret(&track) else {
        return rsx! {};
    };
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let fret = crate::pro_guitar::fret(&item);
    let channel = crate::pro_guitar::channel(&item);
    let forced = crate::pro_guitar::forced(&doc.content.tracks[&track], item.note.tick);
    let fret_id = item.id.clone();
    let channel_id = item.id.clone();
    let hopo_id = item.id.clone();
    let part = crate::pro_guitar::label(&track);
    rsx! {div {class:"selection-pro-guitar", aria_label:"{part} properties",
        label {"Fret"
            for epoch in [fret_reset.key()] {input {key:"{epoch}", aria_label:"{part} fret", r#type:"number", min:"0", max:"{limit}", value:"{fret}",
                onmounted:move |event|fret_reset.mounted(event), onkeydown:move |event|event.stop_propagation(),
                onchange:{let id=fret_id.clone();move |event|{if !event.value().parse::<u8>().ok().is_some_and(|value|update_pro(studio,&id,|f,_,_|*f=value)){fret_reset.rejected();}}}
            }}
        }
        label {"Articulation"
            for epoch in [channel_reset.key()] {select {key:"{epoch}", aria_label:"{part} articulation", value:"{channel}",
                onmounted:move |event|channel_reset.mounted(event), onkeydown:move |event|event.stop_propagation(),
                onchange:{let id=channel_id.clone();move |event|{if !event.value().parse::<u8>().ok().is_some_and(|value|update_pro(studio,&id,|_,c,_|*c=value)){channel_reset.rejected();}}},
                for value in 0..=6u8 {option {value:"{value}", selected:value==channel,"{crate::pro_guitar::articulation(value)}"}}
                if channel>6 {option {value:"{channel}", selected:true,"Preserved MIDI channel {channel}"}}
            }}
        }
        label {class:"form-check",
            for epoch in [hopo_reset.key()] {input {key:"{epoch}",r#type:"checkbox",aria_label:"{part} forced HOPO",checked:forced,
                onmounted:move |event|hopo_reset.mounted(event),onkeydown:move |event|event.stop_propagation(),
                onchange:{let id=hopo_id.clone();move |event|{if !update_pro(studio,&id,|_,_,h|*h=event.checked()){hopo_reset.rejected();}}}
            }} "Force HOPO for this attack"
        }
        p {class:"field-hint","Fret 0 is an open string. Force HOPO applies to every string at this tick. Preview does not judge fingering or controller playability."}
    }}
}

#[component]
fn NoteSelection(studio: Studio, track: String, item: EditorNote) -> Element {
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let layout = doc.drum_layout();
    let tick_reset = super::field_reset::use_field_reset();
    let length_reset = super::field_reset::use_field_reset();
    let lane_reset = super::field_reset::use_field_reset();
    let cymbal_reset = super::field_reset::use_field_reset();
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
                    for epoch in [tick_reset.key()] { input { key: "{epoch}", aria_label: "Note position in ticks", r#type: "number", min: "0", value: "{item.note.tick}", onmounted: move |event| tick_reset.mounted(event), onkeydown: move |event| event.stop_propagation(), onchange: {let id=tick_id.clone(); move |event| { if !event.value().parse().ok().is_some_and(|value| update(studio, &id, |note, _| note.tick = value)) { tick_reset.rejected(); } } } } }
                }
                label { "Length ticks"
                    for epoch in [length_reset.key()] { input { key: "{epoch}", aria_label: "Note duration in ticks", r#type: "number", min: "0", value: "{item.note.sustain}", disabled: drums, onmounted: move |event| length_reset.mounted(event), onkeydown: move |event| event.stop_propagation(), onchange: {let id=length_id.clone(); move |event| { if !event.value().parse().ok().is_some_and(|value| update(studio, &id, |note, _| note.sustain = value)) { length_reset.rejected(); } } } } }
                }
            }
            div { class: if crate::note_types::supported(&track) || drums {"selection-properties"} else {""},
              label { class: "selection-lane", "Lane"
                for epoch in [lane_reset.key()] { select { key: "{epoch}", aria_label: "Note lane", value: "{item.note.lane}", onmounted: move |event| lane_reset.mounted(event), onkeydown: move |event| event.stop_propagation(), onchange: {let id=lane_id.clone(); move |event| { if !event.value().parse().ok().is_some_and(|value| update(studio, &id, |note, _| note.lane = value)) { lane_reset.rejected(); } } },
                    for lane in doc.lane_rows(&track) { option { value: "{lane}", selected: lane == item.note.lane, "{super::note_canvas::lane_name_in(layout, &track, lane)}" } }
                } }
              }
              super::note_types::NoteTypes {studio}
              super::drum_dynamics::DrumDynamics {studio}
            }
            ProGuitarProperties {studio,track:track.clone(),item:item.clone()}
            if drums && item.note.lane == 32 && track != "ExpertDrums" {
                p { class: "field-hint", "MIDI supports double kick on Expert only. Lower difficulties need .chart export, which is unavailable for imported MIDI songs." }
            }
            div { class: "selection-actions",
                if drums && layout.cymbal_flag(item.note.lane) { label { class: "form-check",
                    for epoch in [cymbal_reset.key()] { input { key: "{epoch}", r#type: "checkbox", checked: cymbal, onmounted: move |event| cymbal_reset.mounted(event), onkeydown: move |event| event.stop_propagation(), onchange: {let id=cymbal_id.clone(); move |event| { if !update(studio, &id, |_, cymbal| *cymbal = event.checked()) { cymbal_reset.rejected(); } } } } } "Cymbal"
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
    if super::dance::active() {
        return rsx! {super::dance::Inspector{key:"{super::dance::view_epoch()}",studio}};
    }
    let Some(document) = studio.snapshot() else {
        return rsx! {};
    };
    let selected = document
        .selected_note
        .as_ref()
        .and_then(|id| document.find_note(id));
    let vocals = document.selected_track == "Vocals";
    let count = studio.selection().len();
    let solo_key = format!(
        "{:?}:{}:{}:{}",
        studio.store.read().as_ref().map(|store| &store.root),
        document.audio_sha256,
        document.source_chart_sha256,
        document.selected_track
    );
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
                if vocals && document.content.vocal_review.is_none() && document.content.sync.as_ref().is_some_and(|sync| sync.archived_vocal_review.is_some()) {
                    p {class:"field-hint", "Tempo changed. Review vocals again."}
                }
                if let Some(crate::parts::Origin::Generated { .. }) = document.content.track_origins.get(&document.selected_track) {
                    p { class: "field-hint", "Generated draft · listen and review this part." }
                }
                if count > 1 {
                    section { class: "selection-note", aria_label: "Selected notes",
                        p { class: "selection-caption", "{count} notes selected" }
                        p { class: "field-hint", "Drag to move. Shift-click to add or remove." }
                        super::note_types::NoteTypes {studio}
                        super::drum_dynamics::DrumDynamics {studio}
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
                if !vocals {super::star_power::Controls {key:"{document.selected_track}",studio,track:document.selected_track.clone()}}
                if !vocals {super::solo_editor::Controls {key:"{solo_key}",studio,track:document.selected_track.clone()}}
                if crate::lane_ranges::supports(&document.selected_track,crate::lane_ranges::Kind::Trill) {super::lane_range_editor::Controls {key:"{solo_key}",studio,track:document.selected_track.clone()}}
                if document.selected_track.ends_with("Drums") {
                    super::disco_editor::Controls {key:"disco:{solo_key}",studio,track:document.selected_track.clone()}
                }
                if document.selected_track.ends_with("Drums") {
                    super::drum_range_editor::Controls {key:"{studio.store.read().as_ref().map(|s|s.root.display().to_string()).unwrap_or_default()}:{document.selected_track}",studio,track:document.selected_track.clone()}
                }
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
