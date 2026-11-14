//! Ordinary chart text has explicit scope and source-bound, undoable edits.
use super::state::Studio;
use crate::{
    editor::Action,
    text_events::{Edit, Event, Scope},
};
use dioxus::prelude::*;
use std::path::PathBuf;
const PAGE: usize = 64;

#[derive(Clone, PartialEq)]
struct Binding {
    root: PathBuf,
    chart: String,
    audio: String,
    revision: u64,
}
fn binding(studio: Studio) -> Option<Binding> {
    let project = studio.snapshot()?;
    Some(Binding {
        root: studio.store.read().as_ref()?.root.clone(),
        chart: project.source_chart_sha256.clone(),
        audio: project.audio_sha256.clone(),
        revision: project.revision,
    })
}
#[derive(Clone, Debug, PartialEq)]
struct View {
    events: Vec<Event>,
    tracks: Vec<String>,
}
fn view(studio: Studio) -> Result<View, String> {
    let project = studio.snapshot().ok_or("Open a project")?;
    let store = studio.store.read();
    let store = store.as_ref().ok_or("Open a project")?;
    let mut events = store
        .text_events(&project)
        .map_err(|e| e.to_string())?
        .events;
    events.sort_by(|a, b| (a.tick, &a.scope, &a.id).cmp(&(b.tick, &b.scope, &b.id)));
    Ok(View {
        events,
        tracks: super::track_picker::tracks(&project)
            .into_iter()
            .filter(|t| crate::parts::editable_track(t))
            .collect(),
    })
}
fn scope_label(scope: &Scope) -> String {
    match scope {
        Scope::Global => "Global · entire song".into(),
        Scope::Local { track } => format!("Local · {}", super::track_picker::title(track)),
    }
}
#[component]
pub fn Controls(studio: Studio) -> Element {
    let mut open = use_signal(|| false);
    rsx! {
        button {class:"studio-tool",aria_label:"Edit chart text events",disabled:super::dance::active()||studio.snapshot().is_none()||(studio.analysis_preview)().is_some(),onclick:move|_|open.set(true),"Text events…"}
        if open() {super::modal::Modal {id:"text-events-dialog",label:"Chart text events",onclose:move|_|open.set(false),div {class:"snap-custom-panel text-events-panel",onkeydown:super::shortcuts::keep_space_local,Dialog {studio}button {class:"studio-tool",onclick:move|_|open.set(false),"Close text events"}}}}
    }
}
#[component]
fn Dialog(studio: Studio) -> Element {
    let mut expected = use_signal(|| binding(studio));
    let mut previous = use_signal(|| None::<Event>);
    let mut scope = use_signal(String::new);
    let mut tick = use_signal(|| "0".to_string());
    let mut text = use_signal(String::new);
    let mut page = use_signal(|| 0usize);
    let mut error = use_signal(|| None::<String>);
    let current = use_memo(move || view(studio));
    let Ok(current) = current() else {
        return rsx! {h2 {"Chart text events"}p {role:"status","{current().unwrap_err()}"}};
    };
    let stale = binding(studio) != expected();
    let pages = current.events.len().div_ceil(PAGE).max(1);
    let at = page().min(pages - 1);
    let chosen = previous();
    let mut apply = move |action: Action| {
        if binding(studio) != expected() {
            error.set(Some("The project changed. Your draft is kept. Close and reopen text events before applying it.".into()));
            return;
        }
        if studio.try_dispatch(action) {
            expected.set(binding(studio));
            previous.set(None);
            text.set(String::new());
            error.set(None);
        } else {
            error.set((studio.error)());
        }
    };
    let doc = studio.snapshot();
    rsx! {
        h2 {"Chart text events"}
        p {class:"field-hint","Global events apply to the entire song. Local events apply only to the selected instrument difficulty. Edit text or tick, move between scopes, or remove an event. Each change has Undo and Redo."}
        p {class:"field-hint","Sections, solos, BRE and drum-mix controls have dedicated editors. Unrecognized syntax and foreign sections are preserved. Arbitrary local text may require chart export; MIDI conversion refuses unrepresentable events."}
        if stale {p {role:"alert","The project changed. Your draft is kept; close and reopen this editor to continue."}}
        div {class:"vocal-syllables",role:"group",aria_label:"Existing chart text events",
            for event in current.events.iter().skip(at*PAGE).take(PAGE) {{let target=event.clone();let label=scope_label(&event.scope);rsx! {
                button {class:if chosen.as_ref()==Some(event){"studio-tool active"}else{"studio-tool"},aria_pressed:chosen.as_ref()==Some(event),disabled:stale,onclick:move|_|{scope.set(match &target.scope {Scope::Global=>String::new(),Scope::Local {track}=>track.clone()});tick.set(target.tick.to_string());text.set(target.text.clone());previous.set(Some(target.clone()));error.set(None);},"{event.tick} · {label} · {event.text}"}
            }}}
        }
        if current.events.is_empty(){p {class:"field-hint","No editable ordinary text events in this chart."}}
        if pages>1 {div {class:"field-row",button {class:"studio-tool",disabled:at==0,onclick:move|_|page.set(at-1),"Previous text events"}span {"Page {at+1} / {pages}"}button {class:"studio-tool",disabled:at+1>=pages,onclick:move|_|page.set(at+1),"Next text events"}}}
        button {class:"studio-tool",disabled:stale,onclick:move|_|{previous.set(None);text.set(String::new());error.set(None);},"New text event"}
        label {class:"field-stack",span {class:"field-label","Scope"}select {class:"field-control",aria_label:"Text event scope",value:scope(),onchange:move|e|scope.set(e.value()),option {value:"",selected:scope().is_empty(),"Global · entire song"}for track in current.tracks {option {value:track.clone(),selected:scope()==track,"Local · {super::track_picker::title(&track)}"}}}}
        label {class:"field-stack",span {class:"field-label","Tick"}input {class:"field-control",aria_label:"Text event tick",r#type:"number",min:"0",step:"1",value:tick(),oninput:move|e|tick.set(e.value())}}
        label {class:"field-stack",span {class:"field-label","Text"}input {class:"field-control",aria_label:"Text event text",value:text(),oninput:move|e|text.set(e.value())}}
        if let Some(message)=error() {p {role:"alert",class:"vocal-edit-error","{message}"}}
        button {class:"field-button",disabled:stale,onclick:move|_|{let Ok(value)=tick().parse::<u64>() else {error.set(Some("Use a nonnegative whole tick.".into()));return};let target=if scope().is_empty(){Scope::Global}else{Scope::Local {track:scope()}};apply(Action::EditTextEvent {edit:Edit::Put {previous:previous(),scope:target,tick:value,text:text()}});},if chosen.is_some(){"Apply text event"}else{"Add text event"}}
        if let Some(event)=chosen {button {class:"studio-tool",disabled:stale,onclick:move|_|apply(Action::EditTextEvent {edit:Edit::Remove {event:event.clone()}}),"Remove text event"}}
        div {class:"field-row",
            button {class:"studio-tool",aria_label:"Undo text event edit",disabled:stale||doc.as_ref().is_none_or(|p|p.undo.is_empty()),onclick:move|_|apply(Action::Undo),"Undo"}
            button {class:"studio-tool",aria_label:"Redo text event edit",disabled:stale||doc.as_ref().is_none_or(|p|p.redo.is_empty()),onclick:move|_|apply(Action::Redo),"Redo"}
        }
    }
}
