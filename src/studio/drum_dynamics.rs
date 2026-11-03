//! The same single/batch drum control is compiled for native and static web.
use super::state::Studio;
use crate::{
    drum_dynamics::{self, Kind},
    editor::Action,
    note_groups::Snapshot,
};
use dioxus::prelude::*;

#[component]
pub fn DrumDynamics(studio: Studio) -> Element {
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let selection = studio.selection();
    if !drum_dynamics::supported(&selection.track) || selection.is_empty() {
        return rsx! {};
    }
    let Ok(snapshot) = Snapshot::capture(&doc, &selection) else {
        return rsx! {};
    };
    let notes = &doc.content.tracks[&selection.track];
    let selected: Vec<_> = notes
        .iter()
        .filter(|n| selection.ids.contains(&n.id) && drum_dynamics::pad(n.note.lane))
        .collect();
    if selected.is_empty() {
        return rsx! {p{class:"field-hint","Accents and ghost notes apply to drum pads. Kick notes stay unchanged."}};
    }
    let markers = drum_dynamics::markers(notes);
    let mut kinds = selected.iter().map(|n| {
        markers
            .get(&(n.note.tick, n.note.lane))
            .copied()
            .unwrap_or(Some(Kind::Normal))
    });
    let first = kinds.next().flatten();
    let kind = first.filter(|first| kinds.all(|kind| kind == Some(*first)));
    let value = kind.map_or("mixed", Kind::key);
    let skipped = selection.len() - selected.len();
    rsx! {label{class:"selection-note-type","Drum dynamics"
        select{aria_label:"Drum dynamics",value:"{value}",onkeydown:move|event|event.stop_propagation(),onchange:move|event|{
            if let Some(kind)=Kind::parse(&event.value()){studio.dispatch(Action::SetDrumDynamics{selection:snapshot.clone(),kind});}
        },
            if kind.is_none(){option{value:"mixed",disabled:true,selected:true,"Mixed"}}
            option{value:"normal",selected:value=="normal","Normal"}
            option{value:"accent",selected:value=="accent","Accent"}
            option{value:"ghost",selected:value=="ghost","Ghost"}
        }
    }
    if skipped>0{p{class:"field-hint","Drum pads affected: {selected.len()}. Kick notes unchanged: {skipped}."}}}
}
