//! Shared native and static-browser controls for instrument lane markers.
use super::state::Studio;
use crate::{
    editor::Action,
    lane_ranges::{self, Edit, Kind, Range, VisibleRange},
    song_files::game_midi::LaneScope,
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
fn current(studio: Studio, track: &str) -> Option<Vec<VisibleRange>> {
    lane_ranges::view(
        studio.doc.read().as_ref()?,
        &studio.source_lane_ranges.read(),
        track,
    )
}
fn cursor(studio: Studio) -> Range {
    let timing = (studio.timing)();
    let last = timing
        .as_ref()
        .and_then(|t| studio.snapshot().and_then(|d| t.tick(d.duration)))
        .unwrap_or(1)
        .max(1);
    let start = timing
        .as_ref()
        .and_then(|t| t.tick(*studio.playhead.peek()))
        .unwrap_or(0)
        .min(last - 1);
    Range {
        start,
        end: start
            .saturating_add(timing.map_or(1, |t| u64::from(t.resolution) * 4))
            .min(last),
        kind: Kind::Trill,
    }
}
fn scope_label(scope: &Option<LaneScope>) -> &'static str {
    match scope {
        Some(LaneScope::Expert) => "Expert",
        Some(LaneScope::ExpertHard) => "Expert + Hard",
        Some(LaneScope::Dedicated { .. }) => "This Pro Keys tier",
        None => "This chart tier",
    }
}
fn commit(
    mut studio: Studio,
    bound: &Option<Binding>,
    track: &str,
    scope: Option<LaneScope>,
    edit: Edit,
) -> bool {
    if binding(studio) != *bound || studio.snapshot().is_none_or(|d| d.selected_track != track) {
        studio.error.set(Some(
            "The project or track changed. Select the current lane range again.".into(),
        ));
        return false;
    }
    let action = if studio.source_lane_ranges.read().midi {
        let Some((owner, _)) = lane_ranges::midi_owner(track) else {
            return false;
        };
        let Some(scope) = scope else { return false };
        Action::EditMidiLaneRange { owner, scope, edit }
    } else {
        Action::EditLaneRange {
            track: track.into(),
            edit,
        }
    };
    studio.try_dispatch(action)
}

#[component]
pub fn Controls(mut studio: Studio, track: String) -> Element {
    let mut previous = use_signal(|| None::<VisibleRange>);
    let mut start = use_signal(|| cursor(studio).start.to_string());
    let mut end = use_signal(|| cursor(studio).end.to_string());
    let mut kind = use_signal(|| Kind::Trill);
    let initial_scope = lane_ranges::midi_owner(&track).map(|(_, scope)| scope);
    let mut scope = use_signal(|| initial_scope.clone());
    let mut page = use_signal(|| 0usize);
    let mut error = use_signal(|| None::<String>);
    let bound = use_hook(|| binding(studio));
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let source = studio.source_lane_ranges.read();
    let midi = source.midi;
    let Some(ranges) = current(studio, &track) else {
        return rsx! {details {class:"context-fold",summary {"Trills and tremolos"}p {class:"field-hint","The source has unsupported or ambiguous lane markers. Original MIDI events are retained read-only."}}};
    };
    if midi && initial_scope.is_none() {
        return rsx! {details {class:"context-fold",summary {"Trills and tremolos"}p {class:"field-hint","Standard MIDI lane markers apply to Expert, or Expert and Hard together. Select Expert or Hard to edit them. Pro Keys uses a separate trill owner for each tier."}}};
    }
    let chosen = previous();
    let stale = chosen.as_ref().is_some_and(|r| !ranges.contains(r));
    let pages = ranges.len().div_ceil(64).max(1);
    let at = page().min(pages - 1);
    let supported_tremolo = lane_ranges::supports(&track, Kind::Tremolo);
    let selected = studio.selection().track == track && !studio.selection().ids.is_empty();
    let current_scope_label = scope_label(&if midi { scope() } else { None });
    rsx! {details {class:"context-fold lane-range-controls",onkeydown:move|e|e.stop_propagation(),
        summary {"Trills and tremolos · {ranges.len()}"}
        p {class:"field-hint","Set tick bounds to create, move or resize a range. MIDI scope is explicit; edits retain original channel and velocity attributes. Undo and Redo retain each change."}
        div {class:"vocal-syllables",role:"group",aria_label:"Instrument lane ranges",
            for r in ranges.iter().skip(at*64).take(64) {{let target=r.clone();let label=scope_label(&r.scope);let name=if r.range.kind==Kind::Trill{"Trill"}else{"Tremolo"};rsx!{button {class:if chosen.as_ref()==Some(r){"studio-tool active"}else{"studio-tool"},aria_pressed:chosen.as_ref()==Some(r),onclick:move |_|{start.set(target.range.start.to_string());end.set(target.range.end.to_string());kind.set(target.range.kind);scope.set(target.scope.clone());previous.set(Some(target.clone()));error.set(None);},"{name} · {r.range.start}–{r.range.end} · {label}"}}}}
        }
        if pages>1 {div {class:"field-row",button {class:"studio-tool",disabled:at==0,onclick:move |_|page.set(at-1),"Previous ranges"}span {"Page {at+1} / {pages}"}button {class:"studio-tool",disabled:at+1>=pages,onclick:move |_|page.set(at+1),"Next ranges"}}}
        if stale {p {class:"field-hint",role:"status","This range changed. Your draft is kept; select a current range before applying it."}}
        div {class:"field-row",
            button {class:"studio-tool",onclick:{let initial_scope=initial_scope.clone();move |_|{let r=cursor(studio);start.set(r.start.to_string());end.set(r.end.to_string());kind.set(r.kind);scope.set(initial_scope.clone());previous.set(None);error.set(None);}},"New lane range at playhead"}
            button {class:"studio-tool",disabled:!selected,onclick:{let track=track.clone();move |_|{let Some(doc)=studio.snapshot() else{return};let selection=studio.selection();if selection.track!=track{return}match crate::solos::selected(&doc,&selection,1) {Ok(r)=>{start.set(r.start.to_string());end.set(r.end.to_string());error.set(None)},Err(e)=>error.set(Some(e.to_string()))}}},"Use selected notes"}
            button {class:"studio-tool",disabled:doc.transport.loop_range.is_none(),onclick:move |_|{let Some(r)=studio.snapshot().and_then(|d|d.transport.loop_range) else{return};let Some(t)=(studio.timing)() else{return};if let (Some(a),Some(b))=(t.tick(r.start),t.tick(r.end)){start.set(a.to_string());end.set(b.to_string());error.set(None);}},"Use current loop"}
        }
        div {class:"field-row",
            label {class:"field-stack field-half",span {class:"field-label","Start tick"}input {class:"field-control",r#type:"number",min:"0",aria_label:"Lane range start tick",value:start,oninput:move|e|start.set(e.value())}}
            label {class:"field-stack field-half",span {class:"field-label","End tick"}input {class:"field-control",r#type:"number",min:"1",aria_label:"Lane range end tick",value:end,oninput:move|e|end.set(e.value())}}
        }
        label {class:"field-stack",span {class:"field-label","Kind"}select {class:"field-control",aria_label:"Lane range kind",value:if kind()==Kind::Trill{"trill"}else{"tremolo"},onchange:move|e|kind.set(if e.value()=="tremolo"{Kind::Tremolo}else{Kind::Trill}),option {value:"trill","Trill"}if supported_tremolo {option {value:"tremolo","Tremolo"}}}}
        if midi && track.starts_with("Expert") && !track.ends_with("ProKeys") {
            label {class:"field-stack",span {class:"field-label","MIDI scope"}select {class:"field-control",aria_label:"Lane range MIDI scope",disabled:chosen.is_some(),value:if scope()==Some(LaneScope::ExpertHard){"both"}else{"expert"},onchange:move|e|scope.set(Some(if e.value()=="both"{LaneScope::ExpertHard}else{LaneScope::Expert})),option {value:"expert","Expert"}option {value:"both","Expert + Hard"}}}
        } else {p {class:"field-hint","{current_scope_label}"}}
        if let Some(message)=error() {p {class:"vocal-edit-error",role:"alert","{message}"}}
        button {class:"field-button",onclick:{let bound=bound.clone();let track=track.clone();move |_|{match (start.read().parse(),end.read().parse()) {(Ok(a),Ok(b))=>{let r=Range{start:a,end:b,kind:kind()};if commit(studio,&bound,&track,if midi{scope()}else{None},Edit::Put {previous:previous().map(|r|r.range),range:r.clone()}){previous.set(Some(VisibleRange {range:r,scope:if midi{scope()}else{None}}));error.set(None)}else{error.set((studio.error)())}},_=>error.set(Some("Range bounds must be whole ticks.".into()))}}},if chosen.is_some(){"Apply lane range"}else{"Add lane range"}}
        if let Some(r)=chosen {button {class:"studio-tool",onclick:{let bound=bound.clone();let track=track.clone();move |_|{if commit(studio,&bound,&track,r.scope.clone(),Edit::Remove{range:r.range.clone()}){previous.set(None);error.set(None)}else{error.set((studio.error)())}}},"Remove lane range"}}
        div {class:"field-row",button {class:"studio-tool",disabled:doc.undo.is_empty(),onclick:{let bound=bound.clone();let track=track.clone();move |_|{if binding(studio)==bound&&studio.snapshot().is_some_and(|d|d.selected_track==track){studio.try_dispatch(Action::Undo);}}},"Undo"}button {class:"studio-tool",disabled:doc.redo.is_empty(),onclick:move |_|{if binding(studio)==bound&&studio.snapshot().is_some_and(|d|d.selected_track==track){studio.try_dispatch(Action::Redo);}},"Redo"}}
    }}
}

#[component]
pub fn Lane(studio: Studio, track: String) -> Element {
    let ranges = current(studio, &track).unwrap_or_default();
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let start = (studio.view_start)();
    let span = (studio.span)();
    if ranges.is_empty() || !span.is_finite() || span <= 0. {
        return rsx! {};
    }
    rsx! {div {class:"power-lane",span {class:"power-lane-label","Lane ranges"}
        svg {class:"power-lane-plot",view_box:"0 0 1000 16",preserve_aspect_ratio:"none",role:"img","aria-label":"Trill and tremolo ranges on the song timeline",
            for r in ranges.iter().filter(|r|timing.seconds(r.range.end)>start&&timing.seconds(r.range.start)<start+span) {{let a=((timing.seconds(r.range.start)-start)/span*1000.).max(0.);let b=((timing.seconds(r.range.end)-start)/span*1000.).min(1000.);let name=if r.range.kind==Kind::Trill{"Trill"}else{"Tremolo"};rsx!{rect {x:"{a}",y:"2",width:"{b-a}",height:"12",rx:"2",fill:if r.range.kind==Kind::Trill{"#554481"}else{"#79552b"},stroke:"#d9b7f3","data-lane-range-start":"{r.range.start}","data-lane-range-end":"{r.range.end}",title {"{name} · {r.range.start}–{r.range.end} ticks · {scope_label(&r.scope)}"}}}}}
        }
    }}
}
