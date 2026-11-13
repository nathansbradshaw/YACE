//! Shared native/browser vocal power authoring; HARM1 owns harmony power.
use super::state::Studio;
use crate::{
    editor::Action,
    vocal_overdrive::{Edit, Part, Phrase, Range, State},
};
use dioxus::prelude::*;
use std::path::PathBuf;

#[derive(Clone, PartialEq)]
struct Binding {
    root: Option<PathBuf>,
    chart: String,
    audio: String,
}
fn binding(studio: Studio) -> Option<Binding> {
    let doc = studio.snapshot()?;
    Some(Binding {
        root: studio.store.read().as_ref().map(|s| s.root.clone()),
        chart: doc.source_chart_sha256.clone(),
        audio: doc.audio_sha256.clone(),
    })
}
fn current(studio: Studio) -> Result<State, String> {
    let doc = studio.snapshot().ok_or("No project is open")?;
    if let Some(state) = &doc.content.vocal_overdrive {
        return Ok(state.clone());
    }
    studio
        .source_vocal_overdrive
        .read()
        .clone()
        .unwrap_or_else(|| Err("Vocal overdrive source is unavailable".into()))
}
fn at_cursor(studio: Studio) -> Range {
    let timing = (studio.timing)();
    let end = timing
        .as_ref()
        .and_then(|t| studio.snapshot().and_then(|d| t.tick(d.duration)))
        .unwrap_or(1)
        .max(1);
    let start = timing
        .as_ref()
        .and_then(|t| t.tick(*studio.playhead.peek()))
        .unwrap_or(0)
        .min(end - 1);
    Range {
        start,
        end: start
            .saturating_add(timing.map_or(1, |t| u64::from(t.resolution) * 4))
            .min(end),
    }
}
fn label(part: Part) -> &'static str {
    match part {
        Part::Lead => "Lead vocals",
        Part::Harm1 => "Shared harmonies (HARM1)",
    }
}
fn commit(mut studio: Studio, bound: &Option<Binding>, action: Action) -> bool {
    if binding(studio) != *bound {
        studio.error.set(Some(
            "The project changed. Select a current vocal overdrive range.".into(),
        ));
        return false;
    }
    studio.try_dispatch(action)
}

#[component]
pub fn Controls(mut studio: Studio) -> Element {
    let mut part = use_signal(|| Part::Lead);
    let mut previous = use_signal(|| None::<Phrase>);
    let mut start = use_signal(|| at_cursor(studio).start.to_string());
    let mut end = use_signal(|| at_cursor(studio).end.to_string());
    let mut page = use_signal(|| 0usize);
    let mut error = use_signal(|| None::<String>);
    let bound = use_hook(|| binding(studio));
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let state = match current(studio) {
        Ok(state) => state,
        Err(message) => {
            return rsx! {details {class:"context-fold",summary {"Vocal overdrive / Star Power"}p {class:"field-hint",role:"status","{message}. Original MIDI controls remain read-only."}}};
        }
    };
    let mut parts: std::collections::BTreeSet<_> = state.tracks.keys().copied().collect();
    if doc.content.vocals.is_some() {
        parts.insert(Part::Lead);
    }
    if doc.content.harmonies.is_some() {
        parts.insert(Part::Harm1);
    }
    if let Some(chart) = doc
        .content
        .vocal_percussion
        .as_ref()
        .and_then(|s| s.document())
    {
        parts.extend(chart.tracks.keys().copied());
    }
    let selected_part = if parts.contains(&part()) {
        part()
    } else {
        parts.first().copied().unwrap_or(Part::Lead)
    };
    let phrases = state
        .tracks
        .get(&selected_part)
        .map_or(&[][..], |t| t.phrases.as_slice());
    let chosen = previous();
    let stale = chosen.as_ref().is_some_and(|p| !phrases.contains(p));
    let pages = phrases.len().div_ceil(64).max(1);
    let at = page().min(pages - 1);
    rsx! {details {class:"context-fold vocal-overdrive-controls",onkeydown:move|e|e.stop_propagation(),
        summary {"Vocal overdrive / Star Power · {phrases.len()}"}
        p {class:"field-hint","Lead power is independent. HARM1 supplies shared power to HARM2 and HARM3. Set tick bounds to create, move or resize ranges; singing notes and scoring phrases stay separate."}
        if parts.is_empty() {p {class:"field-hint","Create or import a vocal part before adding overdrive."}}
        else {
            label {class:"field-stack",span {class:"field-label","Power owner"}select {class:"field-control",aria_label:"Vocal overdrive part",value:if selected_part==Part::Lead{"lead"}else{"harm1"},onchange:move|e|{part.set(if e.value()=="harm1"{Part::Harm1}else{Part::Lead});previous.set(None);page.set(0);error.set(None);},
                option {value:"lead",disabled:!parts.contains(&Part::Lead),"Lead vocals"}
                option {value:"harm1",disabled:!parts.contains(&Part::Harm1),"Shared harmonies (HARM1)"}
            }}
            Lane {studio, part:selected_part}
            div {class:"vocal-syllables",role:"group",aria_label:"Vocal overdrive ranges",
                for phrase in phrases.iter().skip(at*64).take(64) {{let target=phrase.clone();rsx!{button {class:if chosen.as_ref()==Some(phrase){"studio-tool active"}else{"studio-tool"},aria_pressed:chosen.as_ref()==Some(phrase),onclick:move |_|{start.set(target.range.start.to_string());end.set(target.range.end.to_string());previous.set(Some(target.clone()));error.set(None);},"Overdrive · {phrase.range.start}–{phrase.range.end}"}}}}
            }
            if pages>1 {div {class:"field-row",button {class:"studio-tool",disabled:at==0,onclick:move |_|page.set(at-1),"Previous ranges"}span {"Page {at+1} / {pages}"}button {class:"studio-tool",disabled:at+1>=pages,onclick:move |_|page.set(at+1),"Next ranges"}}}
            if stale {p {class:"field-hint",role:"status","This range changed. Your draft is kept; select a current range before applying it."}}
            div {class:"field-row",
                button {class:"studio-tool",onclick:move |_|{let r=at_cursor(studio);start.set(r.start.to_string());end.set(r.end.to_string());previous.set(None);error.set(None);},"New overdrive at playhead"}
                button {class:"studio-tool",disabled:doc.transport.loop_range.is_none(),onclick:move |_|{let Some(r)=studio.snapshot().and_then(|d|d.transport.loop_range) else{return};let Some(t)=(studio.timing)() else{return};if let (Some(a),Some(b))=(t.tick(r.start),t.tick(r.end)){start.set(a.to_string());end.set(b.to_string());error.set(None);}},"Use current loop"}
            }
            div {class:"field-row",
                label {class:"field-stack field-half",span {class:"field-label","Start tick"}input {class:"field-control",r#type:"number",min:"0",aria_label:"Vocal overdrive start tick",value:start,oninput:move|e|start.set(e.value())}}
                label {class:"field-stack field-half",span {class:"field-label","End tick"}input {class:"field-control",r#type:"number",min:"1",aria_label:"Vocal overdrive end tick",value:end,oninput:move|e|end.set(e.value())}}
            }
            if let Some(message)=error() {p {class:"vocal-edit-error",role:"alert","{message}"}}
            button {class:"field-button",disabled:!parts.contains(&selected_part),onclick:{let bound=bound.clone();move |_|match (start.read().parse(),end.read().parse()) {
                (Ok(a),Ok(b))=>{let range=Range{start:a,end:b};if commit(studio,&bound,Action::EditVocalOverdrive{part:selected_part,edit:Edit::Put{previous:previous(),range:range.clone()}}){previous.set(current(studio).ok().and_then(|s|s.tracks.get(&selected_part).and_then(|t|t.phrases.iter().find(|p|p.range==range)).cloned()));error.set(None)}else{error.set((studio.error)())}},
                _=>error.set(Some("Range bounds must be whole ticks.".into())),
            }},if chosen.is_some(){"Apply vocal overdrive"}else{"Add vocal overdrive"}}
            if let Some(phrase)=chosen {button {class:"studio-tool",onclick:{let bound=bound.clone();move |_|{if commit(studio,&bound,Action::EditVocalOverdrive{part:selected_part,edit:Edit::Remove{phrase:phrase.clone()}}){previous.set(None);error.set(None)}else{error.set((studio.error)())}}},"Remove vocal overdrive"}}
            div {class:"field-row",button {class:"studio-tool",disabled:doc.undo.is_empty(),onclick:{let bound=bound.clone();move |_|{commit(studio,&bound,Action::Undo);}},"Undo"}button {class:"studio-tool",disabled:doc.redo.is_empty(),onclick:move |_|{commit(studio,&bound,Action::Redo);},"Redo"}}
        }
    }}
}

#[component]
fn Lane(studio: Studio, part: Part) -> Element {
    let Ok(state) = current(studio) else {
        return rsx! {};
    };
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let phrases = state
        .tracks
        .get(&part)
        .map_or(&[][..], |t| t.phrases.as_slice());
    let start = (studio.view_start)();
    let span = (studio.span)();
    if !span.is_finite() || span <= 0. || phrases.is_empty() {
        return rsx! {};
    };
    rsx! {div {class:"power-lane",span {class:"power-lane-label","Overdrive"}svg {class:"power-lane-plot",view_box:"0 0 1000 16",preserve_aspect_ratio:"none",role:"img","aria-label":"Vocal overdrive ranges on the song timeline",
        for p in phrases.iter().filter(|p|timing.seconds(p.range.end)>start&&timing.seconds(p.range.start)<start+span) {{let a=((timing.seconds(p.range.start)-start)/span*1000.).max(0.);let b=((timing.seconds(p.range.end)-start)/span*1000.).min(1000.);rsx!{rect {x:"{a}",y:"2",width:"{b-a}",height:"12",rx:"2",fill:"#806022",stroke:"#f3d477","data-vocal-overdrive-start":"{p.range.start}","data-vocal-overdrive-end":"{p.range.end}",title {"{label(part)} overdrive · {p.range.start}–{p.range.end} ticks"}}}}}
    }}}
}
