use super::state::Studio;
use crate::drum_mix::{Edit, Range};
use crate::editor::Action;
use dioxus::prelude::*;

fn events(studio: Studio, track: &str) -> Result<Vec<crate::drum_mix::Event>, String> {
    let doc = studio.snapshot().ok_or("Open a project")?;
    crate::drum_mix::view(&doc, &studio.source_drum_mix.read(), track).map_err(|e| format!("{e:#}"))
}
fn current(studio: Studio, track: &str) -> Result<Vec<Range>, String> {
    let events = events(studio, track)?;
    crate::drum_mix::ranges(
        &events,
        crate::drum_mix::difficulty(track).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("{e:#}"))
}
#[derive(Clone)]
struct Draft {
    start: String,
    end: String,
    configuration: u8,
}
fn at_cursor(studio: Studio, configuration: u8) -> Draft {
    let map = (studio.timing)();
    let start = map
        .as_ref()
        .and_then(|m| m.tick((studio.playhead)()))
        .unwrap_or(0);
    let last = studio
        .snapshot()
        .and_then(|d| map.as_ref()?.tick(d.duration))
        .unwrap_or(start + 1);
    Draft {
        start: start.min(last.saturating_sub(1)).to_string(),
        end: start
            .saturating_add(map.map_or(480, |m| u64::from(m.resolution) * 4))
            .min(last)
            .to_string(),
        configuration,
    }
}
#[component]
pub fn Controls(mut studio: Studio, track: String) -> Element {
    let mut selected = use_signal(|| None::<Range>);
    let mut draft = use_signal(|| at_cursor(studio, 0));
    let mut error = use_signal(|| None::<String>);
    let mut page = use_signal(|| 0usize);
    let bound = use_signal(|| {
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
    let ranges = match current(studio, &track) {
        Ok(r) => r,
        Err(message) => {
            return rsx! {details{class:"context-fold",summary{"Disco flip"}p{class:"field-hint","{message}. Original mix events remain read-only."}}};
        }
    };
    let all = crate::drum_mix::all(&doc, &studio.source_drum_mix.read());
    let configuration = all
        .as_ref()
        .ok()
        .and_then(|e| e.first())
        .map(|e| e.mix.configuration);
    let routing_error = all.as_ref().err().map(ToString::to_string).or_else(|| {
        all.as_ref()
            .ok()
            .filter(|events| {
                events
                    .iter()
                    .any(|e| Some(e.mix.configuration) != configuration)
            })
            .map(|_| "Changing source drum stem configurations are read-only".into())
    });
    let chosen = selected();
    let value = draft.read().clone();
    let stale = chosen.as_ref().is_some_and(|r| !ranges.contains(r));
    let page_index = page().min(ranges.len().div_ceil(24).max(1) - 1);
    let mut dispatch = move |action: Action| {
        let now = studio.snapshot().map(|doc| {
            (
                studio.store.read().as_ref().map(|s| s.root.clone()),
                doc.source_chart_sha256.clone(),
                doc.selected_track.clone(),
            )
        });
        if now != *bound.peek() {
            studio.error.set(Some(
                "Project or drum part changed. Select the range again.".into(),
            ));
            return false;
        }
        studio.try_dispatch(action)
    };
    rsx! {details{class:"context-fold disco-controls",onkeydown:move|e|e.stop_propagation(),summary{"Disco flip · {ranges.len()}"}
    p{class:"field-hint","Disco swaps red and yellow in the Pro Drums preview. Tick ranges keep explicit resets and the song’s drum stem configuration. Other mix settings remain intact."}
    if let Some(message)=routing_error.clone(){p{class:"field-hint",role:"status","{message}"}}
    div{class:"vocal-syllables",role:"group",aria_label:"Disco ranges",for range in ranges.iter().skip(page_index*24).take(24){{let target=range.clone();rsx!{button{class:"studio-tool",aria_pressed:chosen.as_ref()==Some(range),onclick:move |_|{selected.set(Some(target.clone()));draft.set(Draft{start:target.start().to_string(),end:target.end().to_string(),configuration:target.on.mix.configuration});error.set(None);},"Disco · {range.start()}–{range.end()} ticks"}}}}
    }
    if ranges.len()>24{div{class:"field-row",button{disabled:page_index==0,onclick:move |_|page.set(page_index-1),"Previous disco ranges"}button{disabled:(page_index+1)*24>=ranges.len(),onclick:move |_|page.set(page_index+1),"Next disco ranges"}}}
    if stale{p{class:"field-hint",role:"status","This range changed. Your draft is kept; select a current range before applying it."}}
    button{class:"studio-tool",onclick:move |_|{selected.set(None);draft.set(at_cursor(studio,configuration.unwrap_or(0)));error.set(None);},"New disco range at playhead"}
    label{class:"field-stack",span{class:"field-label","Drum stem configuration"}select{class:"field-control",aria_label:"Disco drum stem configuration",disabled:configuration.is_some(),value:configuration.unwrap_or(value.configuration).to_string(),onchange:move|e|{if let Ok(n)=e.value().parse(){draft.write().configuration=n;}},option{value:"0","0 · Whole kit"}option{value:"1","1 · Mono kick and snare"}option{value:"2","2 · Mono kick, stereo snare"}option{value:"3","3 · Stereo kick and snare"}option{value:"4","4 · Mono kick and kit"}option{value:"5","5 · Four drum stems"}}}
    div{class:"field-row",label{class:"field-stack field-half",span{class:"field-label","Start tick"}input{class:"field-control",r#type:"number",min:"0",aria_label:"Disco start tick",value:value.start,oninput:move|e|draft.write().start=e.value()}}label{class:"field-stack field-half",span{class:"field-label","End tick"}input{class:"field-control",r#type:"number",min:"1",aria_label:"Disco end tick",value:value.end,oninput:move|e|draft.write().end=e.value()}}}
    button{class:"studio-tool",disabled:doc.transport.loop_range.is_none(),onclick:move |_|{if let Some(doc)=studio.snapshot() && let (Some(range),Some(map))=(doc.transport.loop_range,(studio.timing)()) && let (Some(a),Some(b))=(map.tick(range.start),map.tick(range.end)){draft.write().start=a.to_string();draft.write().end=b.to_string();}},"Use current loop for disco"}
    button{class:"field-button",disabled:routing_error.is_some(),onclick:{let track=track.clone();move |_|{let value=draft.read().clone();let bounds=value.start.parse::<u64>().and_then(|a|value.end.parse::<u64>().map(|b|(a,b)));match bounds{Err(_)=>error.set(Some("Use whole ticks for both bounds.".into())),Ok((start,end))=>{if dispatch(Action::EditDiscoRange{track:track.clone(),edit:Edit::Put{previous:selected(),start,end,configuration:configuration.unwrap_or(value.configuration)}}){selected.set(current(studio,&track).ok().and_then(|r|r.into_iter().find(|r|r.start()==start&&r.end()==end)));error.set(None);}else{error.set((studio.error)());}}}}},if chosen.is_some(){"Apply disco range"}else{"Add disco range"}}
    if let Some(range)=chosen{button{class:"studio-tool",disabled:routing_error.is_some(),onclick:{let track=track.clone();move |_|{if dispatch(Action::EditDiscoRange{track:track.clone(),edit:Edit::Remove{range:range.clone()}}){selected.set(None);error.set(None);}else{error.set((studio.error)());}}},"Remove disco range"}}
    div{class:"field-row",button{class:"studio-tool",disabled:doc.undo.is_empty(),onclick:move |_|{dispatch(Action::Undo);},"Undo"}button{class:"studio-tool",disabled:doc.redo.is_empty(),onclick:move |_|{dispatch(Action::Redo);},"Redo"}}
    if let Some(message)=error(){p{class:"vocal-edit-error",role:"alert","{message}"}}
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
    }
    rsx! {
      div { class:"power-lane",
        span {class:"power-lane-label","Disco"}
     svg {class:"power-lane-plot",view_box:"0 0 1000 16",preserve_aspect_ratio:"none",role:"img","aria-label":"Disco flip ranges on the drum timeline",
          for range in ranges.iter().filter(|r|map.seconds(r.end())>start&&map.seconds(r.start())<start+span) {{
            let a=((map.seconds(range.start())-start)/span*1000.).max(0.);
            let b=((map.seconds(range.end())-start)/span*1000.).min(1000.);
            rsx! { if b>a { rect {x:"{a}",y:"2",width:"{b-a}",height:"12",rx:"2",fill:"#e887bb",fill_opacity:".4",stroke:"#e887bb","data-disco-start":"{range.start()}","data-disco-end":"{range.end()}",
              title {"Disco · {range.start()}–{range.end()} ticks"}
            }}}
          }}
        }
      }
    }
}
pub fn preview_events(studio: Studio, track: &str) -> Vec<crate::drum_mix::Event> {
    events(studio, track).unwrap_or_default()
}
