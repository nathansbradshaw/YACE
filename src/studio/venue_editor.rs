//! Shared desktop/browser controls for the supported VENUE cue subset.
use super::state::Studio;
use crate::{
    editor::Action,
    venue::{self, Document, Edit, Encoding, Event},
};
use dioxus::prelude::*;
use std::path::PathBuf;
const PAGE: usize = 48;
#[derive(Clone, PartialEq)]
struct Binding {
    root: PathBuf,
    chart: String,
    audio: String,
    revision: u64,
}
fn binding(studio: Studio) -> Option<Binding> {
    let p = studio.snapshot()?;
    Some(Binding {
        root: studio.store.read().as_ref()?.root.clone(),
        chart: p.source_chart_sha256.clone(),
        audio: p.audio_sha256.clone(),
        revision: p.revision,
    })
}
fn view(studio: Studio) -> Result<Document, String> {
    let project = studio.snapshot().ok_or("Open a project")?;
    let store = studio.store.read();
    store
        .as_ref()
        .ok_or("Open a project")?
        .venue_events(&project)
        .map_err(|e| e.to_string())
}
#[component]
pub fn Controls(studio: Studio) -> Element {
    let mut open = use_signal(|| false);
    rsx! {button{class:"studio-tool",aria_label:"Edit venue cues",disabled:super::dance::active()||studio.snapshot().is_none()||(studio.analysis_preview)().is_some(),onclick:move|_|open.set(true),"Venue…"}
        if open(){super::modal::Modal{id:"venue-dialog",label:"Venue cues",onclose:move|_|open.set(false),div{class:"snap-custom-panel text-events-panel venue-panel",onkeydown:super::shortcuts::keep_space_local,Dialog{studio}button{class:"studio-tool",onclick:move|_|open.set(false),"Close venue cues"}}}}
    }
}
#[component]
fn Dialog(studio: Studio) -> Element {
    let mut expected = use_signal(|| binding(studio));
    let mut previous = use_signal(|| None::<Event>);
    let mut code = use_signal(|| "light_verse".to_string());
    let mut tick = use_signal(|| "0".to_string());
    let mut end = use_signal(|| "480".to_string());
    let mut page = use_signal(|| 0usize);
    let mut error = use_signal(|| None::<String>);
    let mut preview_tick = use_signal(|| 0u64);
    let mut follow = use_signal(|| false);
    let current = use_memo(move || view(studio));
    let Ok(document) = current() else {
        return rsx! {h2{"Venue cues"}p{role:"status","{current().unwrap_err()}"}p{class:"field-hint","Venue cues use MIDI when you export. Your original chart remains in the complete editing project and export history."}};
    };
    let stale = binding(studio) != expected();
    let chosen = previous();
    let pages = document.events.len().div_ceil(PAGE).max(1);
    let at = page().min(pages - 1);
    let ranged = venue::cue(&code()).is_some_and(|c| matches!(c.encoding, Encoding::Note(_)));
    let timing = (studio.timing)();
    let position = if follow() {
        timing
            .as_ref()
            .and_then(|t| t.tick((studio.playhead)()))
            .unwrap_or(0)
    } else {
        preview_tick()
    };
    let resolution = timing.as_ref().map(|t| t.resolution).unwrap_or(480);
    let duration = studio.snapshot().map(|p| p.duration).unwrap_or(0.);
    let max_tick = timing.as_ref().and_then(|t| t.tick(duration)).unwrap_or(0);
    let mut apply = move |action: Action| {
        if binding(studio) != expected() {
            error.set(Some("The project changed. Your draft is kept; close and reopen venue cues before applying it.".into()));
            return;
        }
        if studio.try_dispatch(action) {
            expected.set(binding(studio));
            previous.set(None);
            error.set(None);
        } else {
            error.set((studio.error)());
        }
    };
    let project = studio.snapshot();
    rsx! {
        h2{"Venue cues"}
        p{class:"field-hint","Author lighting, post-processing, cameras, stage effects and performer ranges on the MIDI VENUE track. Choose a cue, set its tick, then add or apply. Other venue commands and note attributes are preserved."}
        p{class:"field-hint","Cue schematic for the supported YARG/Rock Band mappings. Game visuals depend on the target venue; this is not a game rendering. MIDI export is required."}
        Preview{document:document.clone(),tick:position,resolution}
        label{class:"field-stack",span{class:"field-label","Preview tick · {position}"}input{aria_label:"Venue preview tick",r#type:"range",min:"0",max:"{max_tick}",step:"1",value:"{position}",disabled:follow(),oninput:move|e|if let Ok(t)=e.value().parse(){preview_tick.set(t)}}}
        label{class:"venue-follow",input{r#type:"checkbox",checked:follow(),onchange:move|e|follow.set(e.checked())}"Follow playback playhead"}
        if stale{p{role:"alert","The project changed. Your draft is kept; close and reopen this editor to continue."}}
        div{class:"vocal-syllables",role:"group",aria_label:"Existing venue cues",
            for event in document.events.iter().skip(at*PAGE).take(PAGE){{let event=event.clone();let target=event.clone();let c=venue::cue(&event.cue).unwrap();let suffix=event.end.map(|end|format!("–{end}")).unwrap_or_default();rsx!{
                button{class:if chosen.as_ref()==Some(&event){"studio-tool active"}else{"studio-tool"},aria_pressed:chosen.as_ref()==Some(&event),disabled:stale,onclick:move|_|{code.set(target.cue.clone());tick.set(target.tick.to_string());if let Some(value)=target.end{end.set(value.to_string());}preview_tick.set(target.tick);follow.set(false);previous.set(Some(target.clone()));error.set(None);},"{event.tick}{suffix} · {c.category} · {c.label}"}
            }}}
        }
        if document.events.is_empty(){p{class:"field-hint","No supported venue cues yet. Unknown commands remain retained."}}
        if pages>1{div{class:"field-row",button{class:"studio-tool",disabled:at==0,onclick:move|_|page.set(at-1),"Previous venue cues"}span{"Page {at+1} / {pages}"}button{class:"studio-tool",disabled:at+1>=pages,onclick:move|_|page.set(at+1),"Next venue cues"}}}
        button{class:"studio-tool",disabled:stale,onclick:move|_|{previous.set(None);error.set(None);},"New venue cue"}
        label{class:"field-stack",span{class:"field-label","Cue"}select{class:"field-control",aria_label:"Venue cue",value:code(),onchange:move|e|code.set(e.value()),for c in venue::CUES{option{value:c.code,selected:code()==c.code,"{c.category} · {c.label}"}}}}
        label{class:"field-stack",span{class:"field-label","Start tick"}input{class:"field-control",aria_label:"Venue start tick",r#type:"number",min:"0",step:"1",value:tick(),oninput:move|e|tick.set(e.value())}}
        if ranged{label{class:"field-stack",span{class:"field-label","End tick · exclusive"}input{class:"field-control",aria_label:"Venue end tick",r#type:"number",min:"1",step:"1",value:end(),oninput:move|e|end.set(e.value())}}}
        if let Some(message)=error(){p{role:"alert",class:"vocal-edit-error","{message}"}}
        button{class:"field-button",disabled:stale,onclick:move|_|{let Ok(t)=tick().parse::<u64>()else{error.set(Some("Use a nonnegative whole start tick.".into()));return};let finish=if ranged{let Ok(value)=end().parse::<u64>()else{error.set(Some("Use a positive whole end tick.".into()));return};Some(value)}else{None};apply(Action::EditVenue{edit:Edit::Put{previous:previous(),cue:code(),tick:t,end:finish}});preview_tick.set(t);follow.set(false);},if chosen.is_some(){"Apply venue cue"}else{"Add venue cue"}}
        if let Some(event)=chosen{button{class:"studio-tool",disabled:stale,onclick:move|_|apply(Action::EditVenue{edit:Edit::Remove{event:event.clone()}}),"Remove venue cue"}}
        div{class:"field-row",button{class:"studio-tool",aria_label:"Undo venue edit",disabled:stale||project.as_ref().is_none_or(|p|p.undo.is_empty()),onclick:move|_|apply(Action::Undo),"Undo"}button{class:"studio-tool",aria_label:"Redo venue edit",disabled:stale||project.as_ref().is_none_or(|p|p.redo.is_empty()),onclick:move|_|apply(Action::Redo),"Redo"}}
    }
}
#[component]
fn Preview(document: Document, tick: u64, resolution: u32) -> Element {
    let p = venue::preview(&document, tick, resolution);
    let beam = match p.lighting.as_str() {
        "Blackout" => "#1c2530",
        "Cool automatic" | "Verse" => "#5ec5ef",
        _ => "#efbb69",
    };
    let filter = match p.post.as_str() {
        "Sepia" => "sepia(1)",
        "Negative" => "invert(.8)",
        "Bloom" => "brightness(1.3)",
        _ => "none",
    };
    let label = format!(
        "Venue cue schematic at tick {tick}: {}; {}; {}; fog {}; {}",
        p.lighting,
        p.post,
        p.camera,
        if p.fog { "on" } else { "off" },
        p.performers.join(", ")
    );
    let performer_text = p.performers.join(", ");
    let (frame_x, frame_width) = match p.camera.as_str() {
        "Directed guitar" => (142, 74),
        "Directed drums" => (238, 74),
        "Directed vocals" => (46, 74),
        _ => (22, 508),
    };
    rsx! {figure{class:"venue-preview",aria_label:label.clone(),
        svg{view_box:"0 0 552 180",role:"img","aria-label":label,style:"filter:{filter}",
            rect{x:"0",y:"0",width:"552",height:"180",rx:"8",fill:"#101923"}
            path{d:"M55 0 L0 160 L160 160 Z M275 0 L180 160 L370 160 Z M495 0 L390 160 L552 160 Z",fill:beam,opacity:"0.16"}
            ellipse{cx:"276",cy:"150",rx:"250",ry:"21",fill:"#28394b"}
            for (index,(name,needle)) in [("Vocals","vocal"),("Guitar","guitar"),("Drums","drum"),("Bass","bass"),("Keys","keys")].iter().enumerate(){{let x=83+index*96;let spot=p.performers.iter().any(|s|s.to_lowercase().contains(needle)&&s.contains("spotlight"));let sing=p.performers.iter().any(|s|s.to_lowercase().contains(needle)&&s.contains("singalong"));rsx!{
                if spot{ellipse{cx:"{x}",cy:"106",rx:"35",ry:"56",fill:"#efc56e",opacity:"0.4"}}
                circle{cx:"{x}",cy:"78",r:"11",fill:"#b8c9d8"}
                path{d:"M{x} 91 v32 m0 -20 l-19 15 m19 -15 l19 15 m-19 5 l-13 22 m13 -22 l13 22",fill:"none",stroke:"#b8c9d8",stroke_width:"5",stroke_linecap:"round"}
                text{x:"{x}",y:"169",text_anchor:"middle",fill:"#e7edf3",font_size:"11","{name}"}
                if sing{text{x:"{x}",y:"50",text_anchor:"middle",fill:"#efc56e",font_size:"12","Sing"}}
            }}}
            if p.fog{ellipse{cx:"276",cy:"142",rx:"270",ry:"30",fill:"#d6e0eb",opacity:"0.26"}}
            rect{x:"{frame_x}",y:"38",width:"{frame_width}",height:"112",rx:"4",fill:"none",stroke:"#7de4c4",stroke_width:"2",stroke_dasharray:"7 5"}
            if p.bonus{text{x:"276",y:"28",text_anchor:"middle",fill:"#efc56e",font_size:"18","✦ Bonus effect ✦"}}
        }
        figcaption{class:"field-hint","Tick {tick} · {p.lighting} · {p.post} · {p.camera}"}
        p{class:"field-hint",if p.fog{"Fog on · "}else{"Fog off · "}if p.performers.is_empty(){"No active performer ranges"}else{"{performer_text}"}}
    }}
}
