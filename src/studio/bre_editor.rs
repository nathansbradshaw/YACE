//! Shared global coda and representable big rock ending controls.
use super::state::Studio;
use crate::{
    bre::{self, Edit, Phrase, Range, State},
    editor::Action,
};
use dioxus::prelude::*;

fn current(studio: Studio) -> Result<State, String> {
    let doc = studio.snapshot().ok_or("Open a project")?;
    let source = studio
        .source_bre
        .read()
        .clone()
        .unwrap_or_else(|| Err("BRE source unavailable".into()))?;
    bre::current(&doc, &source).map_err(|e| format!("{e:#}"))
}
#[derive(Clone, PartialEq)]
enum Choice {
    Midi(Phrase),
    Drum(crate::drum_ranges::Range),
}
impl Choice {
    fn range(&self) -> Range {
        match self {
            Self::Midi(p) => p.range.clone(),
            Self::Drum(r) => Range {
                start: r.start,
                end: r.end,
            },
        }
    }
}
fn ranges(studio: Studio, track: &str) -> Result<Vec<Choice>, String> {
    let state = current(studio)?;
    let Some(coda) = state.coda else {
        return Ok(vec![]);
    };
    if track.ends_with("Drums") {
        let doc = studio.snapshot().ok_or("Open a project")?;
        let ranges = crate::drum_ranges::view(&doc, &studio.source_drum_ranges.read(), track)
            .ok_or("Independent activation lanes remain read-only")?;
        return Ok(ranges
            .into_iter()
            .filter(|r| r.kind == crate::drum_ranges::Kind::FillActivation && r.start >= coda.tick)
            .map(Choice::Drum)
            .collect());
    }
    let owner = bre::owner(track).ok_or("Choose a supported guitar, bass, keys or drum part")?;
    Ok(state
        .owners
        .get(owner)
        .into_iter()
        .flatten()
        .filter(|p| p.range.start >= coda.tick)
        .cloned()
        .map(Choice::Midi)
        .collect())
}
pub fn is_drum_bre(studio: Studio, range: &crate::drum_ranges::Range) -> bool {
    range.kind == crate::drum_ranges::Kind::FillActivation
        && current(studio)
            .ok()
            .and_then(|s| s.coda)
            .is_some_and(|c| range.start >= c.tick)
}
fn at_cursor(studio: Studio) -> Range {
    let map = (studio.timing)();
    let cursor = map
        .as_ref()
        .and_then(|m| m.tick((studio.playhead)()))
        .unwrap_or(0);
    let start = current(studio)
        .ok()
        .and_then(|s| s.coda)
        .map_or(cursor, |c| cursor.max(c.tick));
    let last = studio
        .snapshot()
        .and_then(|d| map.as_ref()?.tick(d.duration))
        .unwrap_or(start + 1);
    Range {
        start,
        end: start
            .saturating_add(map.map_or(480, |m| u64::from(m.resolution) * 4))
            .min(last),
    }
}
#[component]
pub fn Controls(mut studio: Studio, track: String) -> Element {
    let mut chosen = use_signal(|| None::<Choice>);
    let mut start = use_signal(|| at_cursor(studio).start.to_string());
    let mut end = use_signal(|| at_cursor(studio).end.to_string());
    let mut coda_tick = use_signal(|| {
        current(studio).ok().and_then(|s| s.coda).map_or_else(
            || at_cursor(studio).start.to_string(),
            |c| c.tick.to_string(),
        )
    });
    let mut coda_witness = use_signal(|| current(studio).ok().and_then(|s| s.coda));
    let mut error = use_signal(|| None::<String>);
    let mut page = use_signal(|| 0usize);
    let binding = use_signal(|| {
        studio.snapshot().map(|doc| {
            (
                studio.store.read().as_ref().map(|s| s.root.clone()),
                doc.source_chart_sha256.clone(),
                doc.selected_track.clone(),
            )
        })
    });
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let state = match current(studio) {
        Ok(state) => state,
        Err(message) => {
            return rsx! {details{class:"context-fold",summary{"Big rock endings"}p{class:"field-hint","{message}. Original controls remain read-only."}}};
        }
    };
    let available = ranges(studio, &track);
    let range_error = available.as_ref().err().cloned();
    let ranges = available.unwrap_or_default();
    let selected = chosen();
    let stale = selected.as_ref().is_some_and(|r| !ranges.contains(r));
    let coda_stale = state.coda != coda_witness();
    let at = page().min(ranges.len().div_ceil(24).max(1) - 1);
    let mut send = move |action: Action| {
        let now = studio.snapshot().map(|doc| {
            (
                studio.store.read().as_ref().map(|s| s.root.clone()),
                doc.source_chart_sha256.clone(),
                doc.selected_track.clone(),
            )
        });
        if now != *binding.peek() {
            error.set(Some(
                "Project or part changed. Select the BRE again.".into(),
            ));
            return false;
        }
        if studio.try_dispatch(action) {
            error.set(None);
            true
        } else {
            error.set((studio.error)());
            false
        }
    };
    let mut dispatch = move |edit: Edit| send(Action::EditBre { edit });
    rsx! {details{class:"context-fold bre-controls",onkeydown:move|e|e.stop_propagation(),summary{"Big rock endings · {ranges.len()}"}
    p{class:"field-hint","A global coda makes later activation ranges big rock endings across instruments. Changing it can reclassify existing source ranges. Notes and finishing hits stay in place. Guitar and keys endings require MIDI; drum endings can use .chart."}
    label{class:"field-stack",span{class:"field-label","Global coda tick"}input{class:"field-control",r#type:"number",min:"0",aria_label:"Global coda tick",value:coda_tick(),oninput:move|e|coda_tick.set(e.value())}}
    if coda_stale{p{class:"field-hint",role:"status","Coda changed. Your draft is kept; select the current coda before applying."}}
    button{class:"studio-tool",onclick:{let coda=state.coda.clone();move |_|{coda_witness.set(coda.clone());coda_tick.set(coda.as_ref().map_or_else(||at_cursor(studio).start.to_string(),|c|c.tick.to_string()));}},"Select current coda"}
    button{class:"field-button",onclick:move |_|match coda_tick().parse::<u64>(){Err(_)=>error.set(Some("Coda must be a whole tick.".into())),Ok(tick)=>{if dispatch(Edit::SetCoda{previous:coda_witness(),tick:Some(tick)}){coda_witness.set(current(studio).ok().and_then(|s|s.coda));}}},"Set global coda"}
    if state.coda.is_some(){button{class:"studio-tool",onclick:move |_|{if dispatch(Edit::SetCoda{previous:coda_witness(),tick:None}){coda_witness.set(None);}},"Remove global coda"}}
    if let Some(message)=range_error.clone(){p{class:"field-hint",role:"status","{message}"}}
    div{class:"vocal-syllables",role:"group",aria_label:"Big rock ending ranges",for choice in ranges.iter().skip(at*24).take(24){{let target=choice.clone();let r=choice.range();rsx!{button{class:"studio-tool",aria_pressed:selected.as_ref()==Some(choice),onclick:move |_|{chosen.set(Some(target.clone()));start.set(target.range().start.to_string());end.set(target.range().end.to_string());error.set(None);},"BRE · {r.start}–{r.end} ticks"}}}}}
    if ranges.len()>24{div{class:"field-row",button{disabled:at==0,onclick:move |_|page.set(at-1),"Previous BRE ranges"}button{disabled:(at+1)*24>=ranges.len(),onclick:move |_|page.set(at+1),"Next BRE ranges"}}}
    if stale{p{class:"field-hint",role:"status","This BRE changed. Your draft is kept; select a current range before applying."}}
    button{class:"studio-tool",onclick:move |_|{let r=at_cursor(studio);chosen.set(None);start.set(r.start.to_string());end.set(r.end.to_string());error.set(None);},"New BRE at playhead"}
    div{class:"field-row",label{class:"field-stack field-half",span{class:"field-label","Start tick"}input{class:"field-control",r#type:"number",min:"0",aria_label:"BRE start tick",value:start(),oninput:move|e|start.set(e.value())}}label{class:"field-stack field-half",span{class:"field-label","End tick"}input{class:"field-control",r#type:"number",min:"1",aria_label:"BRE end tick",value:end(),oninput:move|e|end.set(e.value())}}}
    button{class:"studio-tool",disabled:doc.transport.loop_range.is_none(),onclick:move |_|{if let Some(doc)=studio.snapshot()&&let(Some(r),Some(map))=(doc.transport.loop_range,(studio.timing)())&&let(Some(a),Some(b))=(map.tick(r.start),map.tick(r.end)){start.set(a.to_string());end.set(b.to_string());}},"Use current loop for BRE"}
    button{class:"field-button",disabled:state.coda.is_none()||range_error.is_some(),onclick:{let track=track.clone();move |_|{
     let parsed=start().parse::<u64>().and_then(|a|end().parse::<u64>().map(|b|Range{start:a,end:b}));let range=match parsed{Ok(r)=>r,Err(_)=>{error.set(Some("Use whole ticks for both bounds.".into()));return;}};
     let edit=if track.ends_with("Drums"){Edit::DrumPut{track:track.clone(),previous:chosen().and_then(|c|if let Choice::Drum(r)=c{Some(r)}else{None}),range:range.clone()}}else{let Some(owner)=bre::owner(&track)else{return};Edit::Put{owner:owner.into(),previous:chosen().and_then(|c|if let Choice::Midi(p)=c{Some(p)}else{None}),range:range.clone()}};
     if dispatch(edit){chosen.set(self::ranges(studio,&track).ok().and_then(|v|v.into_iter().find(|c|c.range()==range)));}
    }},if selected.is_some(){"Apply BRE"}else{"Add BRE"}}
    if let Some(choice)=selected{button{class:"studio-tool",disabled:state.coda.is_none()||range_error.is_some(),onclick:{let track=track.clone();move |_|{let edit=match &choice{Choice::Drum(range)=>Edit::DrumRemove{track:track.clone(),range:range.clone()},Choice::Midi(phrase)=>{let Some(owner)=bre::owner(&track)else{return};Edit::Remove{owner:owner.into(),phrase:phrase.clone()}}};if dispatch(edit){chosen.set(None);}}},"Remove BRE"}}
    div{class:"field-row",button{class:"studio-tool",disabled:doc.undo.is_empty(),onclick:move |_|{send(Action::Undo);},"Undo"}button{class:"studio-tool",disabled:doc.redo.is_empty(),onclick:move |_|{send(Action::Redo);},"Redo"}}
    if let Some(message)=error(){p{class:"vocal-edit-error",role:"alert","{message}"}}
    }}
}
#[component]
pub fn Lane(studio: Studio, track: String) -> Element {
    let ranges = ranges(studio, &track).unwrap_or_default();
    let Some(map) = (studio.timing)() else {
        return rsx! {};
    };
    let start = (studio.view_start)();
    let span = (studio.span)();
    if ranges.is_empty() || !span.is_finite() || span <= 0. {
        return rsx! {};
    }
    rsx! {
      div {class:"power-lane",
        span {class:"power-lane-label","BRE"}
        svg {class:"power-lane-plot",view_box:"0 0 1000 16",preserve_aspect_ratio:"none",role:"img","aria-label":"Big rock endings on the timeline",
          for choice in &ranges {{
            let r=choice.range();
            let a=((map.seconds(r.start)-start)/span*1000.).max(0.);
            let b=((map.seconds(r.end)-start)/span*1000.).min(1000.);
            rsx! {if b>a {rect {x:"{a}",y:"2",width:"{b-a}",height:"12",rx:"2",fill:"#f1cb5a",fill_opacity:".4",stroke:"#f1cb5a","data-bre-start":"{r.start}","data-bre-end":"{r.end}",
              title {"BRE · {r.start}–{r.end} ticks"}
            }}}
          }}
        }
      }
    }
}
#[component]
pub fn Preview(studio: Studio, track: String, position: f64) -> Element {
    let Some(map) = (studio.timing)() else {
        return rsx! {};
    };
    let active = ranges(studio, &track).unwrap_or_default().iter().any(|c| {
        let r = c.range();
        map.seconds(r.start) <= position && position < map.seconds(r.end)
    });
    rsx! {if active{div{class:"game-preview-policy-warning","data-big-rock-ending":"true","Big rock ending"}}}
}
