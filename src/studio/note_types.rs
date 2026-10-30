//! One shared control for a selected note or group of five-fret attacks.
use super::state::Studio;
use crate::{
    editor::Action,
    note_groups::Snapshot,
    note_types::{self, Kind},
};
use dioxus::prelude::*;
use std::collections::BTreeSet;

#[component]
pub fn NoteTypes(mut studio: Studio) -> Element {
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let selection = studio.selection();
    if !note_types::supported(&selection.track) || selection.is_empty() {
        return rsx! {};
    }
    let Ok(snapshot) = Snapshot::capture(&doc, &selection) else {
        return rsx! {};
    };
    let notes = &doc.content.tracks[&selection.track];
    let ticks: BTreeSet<_> = notes
        .iter()
        .filter(|n| selection.ids.contains(&n.id))
        .map(|n| n.note.tick)
        .collect();
    let markers = note_types::markers(notes);
    let mut kinds = ticks
        .iter()
        .map(|tick| markers.get(tick).copied().unwrap_or(Kind::Auto));
    let first = kinds.next().unwrap_or(Kind::Auto);
    let value = if kinds.all(|kind| kind == first) {
        first.key()
    } else {
        "mixed"
    };
    rsx! {div {class:"selection-note-types",
        label {class:"selection-lane","Type"
            select {aria_label:"Selected note type",title:"Forced switches the automatic strum/HOPO type. Changes the whole chord.",value,
                onkeydown:move |event|event.stop_propagation(),
                onchange:move |event|{if let Some(kind)=Kind::parse(&event.value()){studio.dispatch(Action::SetNoteType {selection:snapshot.clone(),kind});}},
                if value=="mixed" {option {value:"mixed",disabled:true,selected:true,"Mixed"}}
                option {value:"auto",selected:value=="auto","Auto"}
                option {value:"forced",selected:value=="forced","Forced"}
                option {value:"tap",selected:value=="tap","Tap"}
            }
        }
        p {class:"field-hint","Applies to whole chords."}
    }}
}
