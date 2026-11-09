//! Shared drum marker controls. Commands preserve the original MIDI authority.
use super::state::Studio;
use crate::{
    drum_ranges::{Edit, Kind, Range},
    editor::Action,
};
use anyhow::{Context, Result, ensure};
use dioxus::prelude::*;
use std::path::PathBuf;

const PAGE: usize = 48;
fn name(kind: Kind) -> &'static str {
    match kind {
        Kind::FillActivation => "Fill / activation",
        Kind::SingleRoll => "Single roll",
        Kind::DoubleRoll => "Double roll",
    }
}
fn color(kind: Kind) -> &'static str {
    match kind {
        Kind::FillActivation => "#eab55c",
        Kind::SingleRoll => "#68c6b0",
        Kind::DoubleRoll => "#b499ed",
    }
}
#[derive(Clone, PartialEq)]
pub(super) struct Witness {
    root: Option<PathBuf>,
    chart: String,
    track: String,
    revision: u64,
}
fn witness(studio: Studio) -> Option<Witness> {
    let doc = studio.snapshot()?;
    Some(Witness {
        root: studio.store.read().as_ref().map(|s| s.root.clone()),
        chart: doc.source_chart_sha256.clone(),
        track: doc.selected_track.clone(),
        revision: doc.revision,
    })
}
fn same_project(studio: Studio, expected: &Witness) -> bool {
    witness(studio).is_some_and(|now| {
        now.root == expected.root && now.chart == expected.chart && now.track == expected.track
    })
}
fn commit(mut studio: Studio, expected: &Witness, action: Action) -> bool {
    if !same_project(studio, expected) {
        studio.error.set(Some(
            "The project or track changed. Select the drum range again.".into(),
        ));
        return false;
    }
    studio.try_dispatch(action)
}
pub(super) fn current(studio: Studio, track: &str) -> Option<Vec<Range>> {
    let doc = studio.doc.read();
    crate::drum_ranges::view(doc.as_ref()?, &studio.source_drum_ranges.read(), track)
}
fn at_cursor(studio: Studio, kind: Kind) -> Range {
    let Some(map) = (studio.timing)() else {
        return Range {
            start: 0,
            end: 1,
            kind,
        };
    };
    let last = studio
        .snapshot()
        .and_then(|doc| map.tick(doc.duration))
        .unwrap_or(1)
        .max(1);
    let start = map.tick(*studio.playhead.peek()).unwrap_or(0).min(last - 1);
    Range {
        start,
        end: start
            .saturating_add(u64::from(map.resolution) * 4)
            .min(last),
        kind,
    }
}
pub(super) fn copy_text(studio: Studio, range: &Range) -> Result<String> {
    let doc = studio.snapshot().context("Open a drum chart")?;
    ensure!(
        current(studio, &doc.selected_track).is_some_and(|ranges| ranges.contains(range)),
        "The drum range changed. Select it again."
    );
    let map = (studio.timing)().context("Chart timing unavailable")?;
    crate::drum_ranges::encode_clipboard(std::slice::from_ref(range), map.resolution)
}
pub(super) fn prepare_paste(studio: Studio) -> Result<(Witness, u64)> {
    let bound = witness(studio).context("Open a drum chart")?;
    ensure!(bound.track.ends_with("Drums"), "Choose a drum track");
    let at = super::note_canvas::snap(studio, (studio.playhead)())
        .context("Paste point is outside the chart")?;
    Ok((bound, at))
}
pub(super) fn finish_paste(mut studio: Studio, bound: Witness, at: u64, text: &str) -> Result<()> {
    ensure!(
        witness(studio) == Some(bound.clone()),
        "The chart changed while reading the clipboard. Paste again."
    );
    let clipboard = crate::drum_ranges::decode_clipboard(text)?;
    ensure!(
        commit(
            studio,
            &bound,
            Action::EditDrumRange {
                edit: Edit::Paste {
                    track: bound.track.clone(),
                    at,
                    clipboard
                }
            }
        ),
        "{}",
        (studio.error)().unwrap_or_else(|| "Drum paste refused".into())
    );
    studio.status.set("Drum ranges pasted".into());
    Ok(())
}
#[derive(Clone)]
struct Draft {
    start: String,
    end: String,
    kind: Kind,
}
impl From<Range> for Draft {
    fn from(range: Range) -> Self {
        Self {
            start: range.start.to_string(),
            end: range.end.to_string(),
            kind: range.kind,
        }
    }
}
impl Draft {
    fn parse(&self) -> Result<Range> {
        Ok(Range {
            start: self.start.parse().context("Start must be a whole tick")?,
            end: self.end.parse().context("End must be a whole tick")?,
            kind: self.kind,
        })
    }
}
#[component]
pub fn Controls(mut studio: Studio, track: String) -> Element {
    let mut chosen = use_signal(|| None::<Range>);
    let mut draft = use_signal(|| {
        Draft::from(at_cursor(
            studio,
            if (studio.drum_activation_editable)() {
                Kind::FillActivation
            } else {
                Kind::SingleRoll
            },
        ))
    });
    let mut page = use_signal(|| 0usize);
    let mut error = use_signal(|| None::<String>);
    let bound = use_hook(|| witness(studio));
    let Some(bound) = bound else { return rsx! {} };
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let Some(ranges) = current(studio, &track) else {
        return rsx! {details {class:"context-fold",summary {"Drum fills and rolls"}p {class:"field-hint","These drum ranges cannot be safely represented. Their original events remain read-only."}}};
    };
    let selected = chosen();
    let value = draft.read().clone();
    let activation = (studio.drum_activation_editable)();
    let blocked = !activation
        && (value.kind == Kind::FillActivation
            || selected
                .as_ref()
                .is_some_and(|r| r.kind == Kind::FillActivation));
    let pages = ranges.len().div_ceil(PAGE).max(1);
    let at = page().min(pages - 1);
    let stale = selected
        .as_ref()
        .is_some_and(|range| !ranges.contains(range));
    rsx! {details {class:"context-fold drum-range-controls",onkeydown:move|event|event.stop_propagation(),
        summary {"Drum fills and rolls · {ranges.len()}"}
        p {class:"field-hint","Set tick bounds to create, move or resize a range. MIDI drum markers apply to every drum difficulty. Fill and activation markers do not distinguish a big rock ending."}
        if !activation {p {class:"field-hint",role:"status","Original activation lanes cannot be represented as one range. They remain read-only; roll editing is available."}}
        div {class:"vocal-syllables",role:"group",aria_label:"Drum ranges",
            for range in ranges.iter().skip(at*PAGE).take(PAGE) {{let target=range.clone();let label=name(range.kind);rsx!{button {class:if selected.as_ref()==Some(range){"studio-tool active"}else{"studio-tool"},aria_pressed:selected.as_ref()==Some(range),onclick:move |_|{chosen.set(Some(target.clone()));draft.set(Draft::from(target.clone()));error.set(None);},"{label} · {range.start}–{range.end} ticks"}}}}
        }
        if pages>1 {div {class:"field-row",button {disabled:at==0,onclick:move |_|page.set(at-1),"Previous ranges"}span {"Page {at+1} / {pages}"}button {disabled:at+1>=pages,onclick:move |_|page.set(at+1),"Next ranges"}}}
        if stale {p {role:"status",class:"field-hint","This range changed. Your draft is kept; select a current range before applying it."}}
        button {class:"studio-tool",onclick:move |_|{chosen.set(None);draft.set(Draft::from(at_cursor(studio,if activation {Kind::FillActivation}else{Kind::SingleRoll})));error.set(None);},"New drum range at playhead"}
        label {class:"field-stack",span {class:"field-label","Range kind"}select {class:"field-control",aria_label:"Drum range kind",value:match value.kind {Kind::FillActivation=>"fill",Kind::SingleRoll=>"single",Kind::DoubleRoll=>"double"},onchange:move|event|draft.write().kind=match event.value().as_str(){"single"=>Kind::SingleRoll,"double"=>Kind::DoubleRoll,_=>Kind::FillActivation},option {value:"fill",disabled:!activation,"Fill / activation"}option {value:"single","Single roll"}option {value:"double","Double roll"}}}
        div {class:"field-row",
            label {class:"field-stack field-half",span {class:"field-label","Start tick"}input {class:"field-control",aria_label:"Drum range start tick",r#type:"number",min:"0",value:value.start,oninput:move|event|draft.write().start=event.value()}}
            label {class:"field-stack field-half",span {class:"field-label","End tick"}input {class:"field-control",aria_label:"Drum range end tick",r#type:"number",min:"1",value:value.end,oninput:move|event|draft.write().end=event.value()}}
        }
        button {class:"studio-tool",disabled:doc.transport.loop_range.is_none(),onclick:move |_|{let Some(doc)=studio.snapshot() else{return};let Some(loop_range)=doc.transport.loop_range else{return};let Some(map)=(studio.timing)() else{return};if let (Some(a),Some(b))=(map.tick(loop_range.start),map.tick(loop_range.end)){draft.write().start=a.to_string();draft.write().end=b.to_string();}},"Use current loop"}
        button {class:"field-button",disabled:blocked,onclick:{let bound=bound.clone();let track=track.clone();move |_|{match draft.read().parse(){Err(message)=>error.set(Some(format!("{message:#}"))),Ok(range)=>{if commit(studio,&bound,Action::EditDrumRange {edit:Edit::Put {track:track.clone(),previous:chosen(),range:range.clone()}}){chosen.set(Some(range));error.set(None);}else{error.set((studio.error)());}}}}},if selected.is_some(){"Apply drum range"}else{"Add drum range"}}
        if let Some(range)=selected {div {class:"field-row",
            button {class:"studio-tool",disabled:!activation&&range.kind==Kind::FillActivation,onclick:{let bound=bound.clone();let track=track.clone();let range=range.clone();move |_|{if commit(studio,&bound,Action::EditDrumRange {edit:Edit::Remove {track:track.clone(),range:range.clone()}}){chosen.set(None);error.set(None);}else{error.set((studio.error)());}}},"Remove drum range"}
            button {class:"studio-tool",disabled:stale||!doc.has_recording(),onclick:{let bound=bound.clone();let track=track.clone();let range=range.clone();move |_|{
                if !current(studio,&track).is_some_and(|ranges|ranges.contains(&range)){error.set(Some("This drum range changed. Select it again.".into()));return;}
                let Some(doc)=studio.snapshot() else{return};let Some(map)=(studio.timing)() else{return};
                let loop_range=crate::editor::TimeRange {start:map.seconds(range.start).clamp(0.,doc.duration),end:map.seconds(range.end).clamp(0.,doc.duration)};
                if loop_range.end<=loop_range.start {error.set(Some("This range is outside the recording.".into()));return;}
                if commit(studio,&bound,Action::SetLoop {range:Some(loop_range)}){studio.seek(loop_range.start);studio.dispatch(Action::Playback {playing:true});}
            }},"Loop recording range"}
            button {class:"studio-tool",disabled:stale,onclick:move |_|studio.copy_drum_range(range.clone()),"Copy drum range"}
        }}
        div {class:"field-row",
            button {class:"studio-tool",onclick:move |_|studio.paste_drum_ranges(),"Paste drum ranges at playhead"}
            button {class:"studio-tool",disabled:doc.undo.is_empty(),onclick:{let bound=bound.clone();move |_|{commit(studio,&bound,Action::Undo);}},"Undo"}
            button {class:"studio-tool",disabled:doc.redo.is_empty(),onclick:move |_|{commit(studio,&bound,Action::Redo);},"Redo"}
        }
        if let Some(message)=error(){p {class:"vocal-edit-error",role:"alert","{message}"}}
    }}
}

#[component]
pub fn Preview(studio: Studio, track: String, position: f64) -> Element {
    let ranges = current(studio, &track).unwrap_or_default();
    let Some(map) = (studio.timing)() else {
        return rsx! {};
    };
    rsx! {for kind in [Kind::FillActivation,Kind::SingleRoll,Kind::DoubleRoll] {
        if ranges.iter().any(|range|range.kind==kind&&map.seconds(range.start)<=position&&position<map.seconds(range.end)) {
            div {class:"game-preview-policy-warning","data-drum-range-kind":"{kind:?}","{name(kind)}"}
        }
    }}
}
#[component]
pub fn Lane(studio: Studio, track: String) -> Element {
    let ranges = current(studio, &track).unwrap_or_default();
    let Some(map) = (studio.timing)() else {
        return rsx! {};
    };
    let start = (studio.view_start)();
    let span = (studio.span)();
    if ranges.is_empty() || !span.is_finite() || span <= 0. {
        return rsx! {};
    };
    rsx! {for kind in [Kind::FillActivation,Kind::SingleRoll,Kind::DoubleRoll] {{let label=name(kind);let fill=color(kind);rsx!{
        div {class:"power-lane",span {class:"power-lane-label","{label}"}svg {class:"power-lane-plot",view_box:"0 0 1000 16",preserve_aspect_ratio:"none",role:"img","aria-label":"{label} ranges on the drum timeline",
            for range in ranges.iter().filter(|r|r.kind==kind&&map.seconds(r.end)>start&&map.seconds(r.start)<start+span) {{let a=((map.seconds(range.start)-start)/span*1000.).max(0.);let b=((map.seconds(range.end)-start)/span*1000.).min(1000.);rsx!{if b>a {rect {x:"{a}",y:"2",width:"{b-a}",height:"12",rx:"2",fill,fill_opacity:".4",stroke:fill,"data-drum-range-start":"{range.start}","data-drum-range-end":"{range.end}",title {"{label} · {range.start}–{range.end} ticks"}}}}}}
        }}
    }}}}
}
