//! Source-timed SVG lanes, note gestures and waveform geometry.
use super::inline_review::{InlineReview, flag_color};
use super::state::{EditTool, Studio, clock, instrument};
use crate::{
    editor::{self, Action},
    stem_review::StemId,
    timeline::lane_rows,
};
use dioxus::prelude::*;
use std::fmt::Write;

pub fn lane_name(track: &str, lane: u8) -> &'static str {
    if crate::keys::is_pro(track) {
        crate::keys::pitch_name(lane)
    } else if track.ends_with("Drums") {
        match lane {
            0 => "Kick",
            1 => "Snare",
            2 => "Yellow",
            3 => "Blue",
            4 => "Green",
            32 => "Double kick",
            _ => "Control",
        }
    } else {
        match lane {
            0 => "Green",
            1 => "Red",
            2 => "Yellow",
            3 => "Blue",
            4 => "Orange",
            7 => "Open",
            _ => "Control",
        }
    }
}
fn note_count(count: usize) -> String {
    format!("{count} {}", if count == 1 { "note" } else { "notes" })
}
pub fn color(track: &str, lane: u8) -> &'static str {
    if crate::keys::is_pro(track) {
        if matches!(lane % 12, 1 | 3 | 6 | 8 | 10) {
            "#949dac"
        } else {
            "#e8e4db"
        }
    } else if lane == 7 {
        "#b174dc"
    } else if track.ends_with("Drums") {
        match lane {
            0 | 32 => "#f2a65a",
            1 => "#ec5360",
            2 => "#ead452",
            3 => "#4b9be9",
            4 => "#45c878",
            _ => "#666",
        }
    } else {
        match lane {
            0 => "#45c878",
            1 => "#ec5360",
            2 => "#ead452",
            3 => "#4b9be9",
            4 => "#f2a65a",
            _ => "#666",
        }
    }
}

pub(super) fn snap(studio: Studio, seconds: f64) -> Option<u64> {
    let map = studio.timing.read();
    let map = map.as_ref()?;
    let tick = map.tick(seconds)?;
    if (studio.snap)() == 0 {
        Some(tick)
    } else {
        let step = map.resolution as f64 / (studio.snap)() as f64;
        Some((tick as f64 / step).round().mul_add(step, 0.0).round() as u64)
    }
}
fn place(studio: Studio, track: &str, event: &MouseEvent, width: f64) {
    let seconds = (studio.view_start)() + event.element_coordinates().x / width * (studio.span)();
    let rows = lane_rows(track);
    let row = (event.element_coordinates().y / 20.0)
        .floor()
        .clamp(0.0, rows.len() as f64 - 1.0) as usize;
    if let Some(tick) = snap(studio, seconds.max(0.0)) {
        studio.dispatch(Action::AddNote {
            track: track.into(),
            tick,
            lane: rows[row],
            sustain: 0,
            cymbal: false,
        });
    }
}
#[derive(Clone)]
struct Drag {
    id: String,
    tick: u64,
    lane: u8,
    sustain: u64,
    cymbal: bool,
    start_x: f64,
    start_y: f64,
    resize: bool,
}
#[component]
pub(super) fn VocalRoll(mut studio: Studio) -> Element {
    use crate::vocals::{Syllable, VocalTone};
    let mut width = use_signal(|| 1000.0);
    let mut drag = use_signal(|| None::<(usize, crate::vocals::VocalNote, f64, f64, bool)>);
    let gestures = super::note_gestures::use_gestures(studio);
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let start = (studio.view_start)();
    let span = (studio.span)();
    let active = doc.selected_track == "Vocals";
    let selected_vocals = studio.selection().vocals;
    let notes = doc
        .content
        .vocals
        .as_ref()
        .map(|chart| chart.notes.clone())
        .unwrap_or_default();
    let mut select = move |index: Option<usize>| {
        studio.dispatch(Action::SelectTrack {
            track: "Vocals".into(),
        });
        studio.vocal_note.set(index);
    };
    if !active && notes.is_empty() {
        return rsx! {header{class:"lane-track-header",button{aria_pressed:active.to_string(),onclick:move |_|select(None),"Vocals"}span{"Select to add vocals"}}};
    }
    rsx! {
        header{class:"lane-track-header",button{aria_pressed:active.to_string(),onclick:move |_|select(None),"Vocals"}span{"{note_count(notes.len())}"}}
        if has_stem_waveform(studio, StemId::Vocals) {div{class:"midi-waveform-strip",div{class:"midi-waveform-label",style:"width:140px;flex-shrink:0","Vocal stem"}Waveform{studio,stem:Some(StemId::Vocals)}}}
        div{class:"lane-surface",
            ol{class:"lane-labels",for pitch in (36..=84).rev(){li{class:"lane-label",style:"height:8px;font-size:7px",if pitch%12==0{"C{pitch/12-1} · {pitch}"}}}li{class:"lane-label",style:"height:8px;font-size:7px","Talkie"}}
            div{class:"lane-plot",onresize:move|e|{if let Ok(size)=e.get_content_box_size(){width.set(size.width.max(1.0));}},
                svg{class:"studio-note-svg",height:"400",view_box:"0 0 {width} 400",preserve_aspect_ratio:"none",
                    onclick:move|e:MouseEvent|{
                        if gestures.consume_click() { e.stop_propagation(); return; }
                        select(None); studio.clear_selection(); studio.focus_editor();
                        let seconds=start+e.element_coordinates().x/width()*span;
                        if (studio.tool)()==EditTool::Place { place_vocal(studio,&e,width()); }
                        else {studio.seek(seconds);}
                    },
                    ondoubleclick:move|e:MouseEvent|{if (studio.tool)()==EditTool::Select{place_vocal(studio,&e,width());}},
                    onmousedown:move|e:MouseEvent|gestures.background("Vocals",&e),
                    onmousemove:move|e:MouseEvent|gestures.moving(&e),
                    onmouseup:move|e:MouseEvent|{
                        gestures.release(&e,width(),8.);
                        let Some((index,expected,x,y,resize))=drag.take() else{return};
                        let dx=e.page_coordinates().x-x;let dy=e.page_coordinates().y-y;
                        if dx.abs()+dy.abs()<3.0{return;}
                        let Some(doc)=studio.snapshot()else{return};
                        let Some(mut chart)=doc.content.vocals else{return};
                        if chart.notes.get(index)!=Some(&expected){return;}
                        let Some(map)=(studio.timing)()else{return};
                        let mut note=expected.clone();
                        if resize {
                            let Some(end)=snap(studio,(map.seconds(note.end)+dx/width()*span).max(0.0))else{return};
                            note.end=end.max(note.start+1);
                        }else{
                            let Some(tick)=snap(studio,(map.seconds(note.start)+dx/width()*span).max(0.0))else{return};
                            note.end=tick.saturating_add(note.end-note.start);note.start=tick;
                            if let VocalTone::Pitched{midi}=note.tone {note.tone=VocalTone::Pitched{midi:(midi as i32-(dy/8.0).round() as i32).clamp(36,84) as u8};}
                        }
                        chart.notes[index]=note;
                        studio.dispatch(Action::SetVocals{document:chart});
                    },
                    onmouseleave:move |_|{drag.set(None);gestures.cancel();},
                    for row in 0..50{line{x1:"0",x2:"{width}",y1:"{row*8}",y2:"{row*8}",stroke:"#30343b",pointer_events:"none"}}
                    {let first=timing.tick(start.max(timing.offset)).unwrap_or(0);let last=timing.tick(start+span).unwrap_or(first);let step=(timing.resolution as u64/4).max(1);
                    rsx!{for tick in (first/step*step..=last).step_by(step as usize){
                        line{x1:"{(timing.seconds(tick)-start)/span*width()}",x2:"{(timing.seconds(tick)-start)/span*width()}",y1:"0",y2:"400",stroke:if tick%timing.resolution as u64==0{"#51515c"}else{"#292b34"},pointer_events:"none"}
                    }}}
                    if let Some(chart)=doc.content.vocals.as_ref(){for phrase in &chart.phrases{
                        rect{x:"{(timing.seconds(phrase.start)-start)/span*width()}",y:"0",width:"{(timing.seconds(phrase.end)-timing.seconds(phrase.start))/span*width()}",height:"400",fill:"#41c6b8",opacity:"0.04",stroke:"#41c6b8",pointer_events:"none"}
                    }}
                    for (index,note) in notes.iter().enumerate(){
                        {let x=(timing.seconds(note.start)-start)/span*width();let end=(timing.seconds(note.end)-start)/span*width();
                        let row=match note.tone{VocalTone::Pitched{midi}=>84-midi,VocalTone::Talkie{..}=>49};let y=row as f64*8.0+1.0;
                        let label=match &note.lyric{Syllable::Text{text,join_next}=>format!("{text}{}",if *join_next{"-"}else{""}),Syllable::Continue=>"+".into(),Syllable::ContinueWord=>"Continue word".into(),Syllable::Missing=>"Missing lyric".into()};
                        let note_tick=note.start;let resize_note=note.clone();let selected=active&&selected_vocals.contains(&index);
                        rsx!{g{
                            ondoubleclick:move|e|e.stop_propagation(),
                            onclick:move|e:MouseEvent|{e.stop_propagation();if gestures.consume_click(){return;}studio.select_vocal(index,e.modifiers().shift(),false);if (studio.tool)()==EditTool::Erase{studio.delete_selection();}},
                            onmousedown:move|e:MouseEvent|{e.stop_propagation();gestures.note("Vocals",None,index,note_tick,&e);},
                            rect{x:"{x}",y:"{y}",width:"{(end-x).max(8.0)}",height:"6",rx:"2",fill:"#41c6b8",stroke:if selected{"white"}else{"#15191f"},role:"button","aria-pressed":selected.to_string(),"aria-label":"Edit vocal syllable {label}"}
                            text{x:"{x+2.0}",y:"{y-2.0}",font_size:"10",fill:"#e2e8ed",pointer_events:"none","{label}"}
                            rect{x:"{end-4.0}",y:"{y-1.0}",width:"8",height:"8",fill:"transparent",style:"cursor:ew-resize",onmousedown:move|e:MouseEvent|{e.stop_propagation();studio.select_vocal(index,false,false);drag.set(Some((index,resize_note.clone(),e.page_coordinates().x,e.page_coordinates().y,true)));}}
                        }} }
                    }
                    {gestures.overlay(width(),8.)}
                    Playhead{studio,width:width(),height:400.0}
                }
            }
        }
    }
}

fn place_vocal(studio: Studio, event: &MouseEvent, width: f64) {
    use crate::vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone};
    let Some(doc) = studio.snapshot() else {
        return;
    };
    let Some(map) = (studio.timing)() else {
        return;
    };
    let seconds = (studio.view_start)() + event.element_coordinates().x / width * (studio.span)();
    let Some(start) = snap(studio, seconds.max(0.0)) else {
        return;
    };
    let row = (event.element_coordinates().y / 8.0)
        .floor()
        .clamp(0.0, 49.0) as u8;
    let mut chart = doc.content.vocals.unwrap_or(VocalChart {
        version: 1,
        resolution: map.resolution,
        source: "Manually authored in YACE".into(),
        phrases: Vec::new(),
        notes: Vec::new(),
    });
    let division = if (studio.snap)() == 0 {
        4
    } else {
        (studio.snap)() as u64
    };
    let mut end = start.saturating_add((map.resolution as u64 / division).max(1));
    if let Some(phrase) = chart
        .phrases
        .iter()
        .find(|phrase| phrase.start <= start && start < phrase.end)
    {
        end = end.min(phrase.end);
    } else {
        let phrase_start = chart
            .phrases
            .iter()
            .filter(|phrase| phrase.end <= start)
            .map(|phrase| phrase.end)
            .max()
            .unwrap_or(0);
        let phrase_end = chart
            .phrases
            .iter()
            .find(|phrase| phrase.start > start)
            .map(|phrase| phrase.start)
            .unwrap_or_else(|| map.tick(doc.duration).unwrap_or(end));
        end = end.min(phrase_end);
        chart.phrases.push(VocalPhrase {
            start: phrase_start,
            end: phrase_end,
            player: 1,
        });
        chart.phrases.sort_by_key(|phrase| phrase.start);
    }
    chart.notes.push(VocalNote {
        start,
        end,
        tone: if row == 49 {
            VocalTone::Talkie { lenient: false }
        } else {
            VocalTone::Pitched { midi: 84 - row }
        },
        lyric: if row == 49 {
            Syllable::Text {
                text: "la".into(),
                join_next: false,
            }
        } else {
            Syllable::Missing
        },
    });
    chart.notes.sort_by_key(|note| note.start);
    let index = chart.notes.iter().position(|note| note.start == start);
    studio.dispatch(Action::SetVocals { document: chart });
    if studio.error.read().is_none()
        && let Some(index) = index
    {
        studio.select_vocal(index, false, false);
    }
}
#[component]
pub(super) fn Waveform(studio: Studio, #[props(default)] stem: Option<StemId>) -> Element {
    let mut width = use_signal(|| 1000.0);
    let geometry = use_memo(use_reactive((&stem,), move |(stem,)| {
        let start = (studio.view_start)();
        let span = (studio.span)();
        let waveforms = studio.waveforms.read();
        waveforms
            .as_ref()
            .and_then(|cache| match stem {
                Some(id) => cache.stems.get(&id),
                None => Some(&cache.original),
            })
            .map(|waveform| waveform.geometry(start, span, width()))
            .unwrap_or_default()
    }));
    let available = stem.is_none_or(|id| {
        studio
            .waveforms
            .read()
            .as_ref()
            .is_some_and(|cache| cache.stems.contains_key(&id))
    });
    let start = (studio.view_start)();
    let span = (studio.span)();
    let color = match stem {
        Some(StemId::Drums) => "#f2a65a",
        Some(StemId::Guitar) => "#709cdd",
        _ => "#41c6b8",
    };
    rsx! {div{class:"midi-waveform-canvas-wrap",style:"position:relative",title:"Shared linear full scale ±1; no individual stem normalization. Amplitude is not confidence. Peaks above full scale are clipped visually.",onresize:move|e|{if let Ok(size)=e.get_content_box_size(){width.set(size.width.max(1.0));}},
        svg{class:"midi-waveform-canvas",view_box:"0 0 {width} 36",preserve_aspect_ratio:"none",onclick:move|e:MouseEvent|{if available{studio.seek(start+e.element_coordinates().x/width()*span);}},
            path{d:"{geometry}",fill:color,pointer_events:"none"}
            if stem.is_none(){for(a,b)in(studio.clean_ranges)(){rect{x:"{((a-start)/span*width()).max(0.0)}",y:"0",width:"{(((b-start)/span*width()).min(width())-((a-start)/span*width()).max(0.0)).max(0.0)}",height:"36",fill:"#dc87dd",opacity:".3",pointer_events:"none"}}}
            Playhead{studio,width:width(),height:36.0}
        }
        if !available{span{style:"position:absolute;top:11px;left:8px;font-size:10px;color:var(--text-muted);pointer-events:none","Isolated stem unavailable · attach matching local stems"}}
    }}
}
#[component]
fn InstrumentWaveform(studio: Studio, track: String) -> Element {
    let stem = studio.guitar_source.read().audition_stem(&track, false);
    if !has_stem_waveform(studio, stem) {
        return rsx! {};
    }
    rsx! {div{class:"midi-waveform-strip",
        div{class:"midi-waveform-label",style:"width:140px;flex-shrink:0;flex-direction:column;align-items:flex-start;justify-content:center;font-size:9px;gap:1px",span{"{instrument(&track)}"}span{style:"font-size:8px;color:var(--text-muted)","Estimated {stem.as_str()} audio"}}
        Waveform{studio,stem:Some(stem)}
    }}
}

fn has_stem_waveform(studio: Studio, stem: StemId) -> bool {
    studio
        .waveforms
        .read()
        .as_ref()
        .is_some_and(|cache| cache.stems.contains_key(&stem))
}
/// Only this SVG node subscribes to the audio clock. Paths and note lists do
/// not need to be reconstructed for every playback frame.
#[component]
fn Playhead(studio: Studio, width: f64, height: f64) -> Element {
    let x = ((studio.playhead)() - (studio.view_start)()) / (studio.span)() * width;
    rsx! {line{x1:"{x}",x2:"{x}",y1:"0",y2:"{height}",stroke:"#fff",pointer_events:"none"}}
}
#[component]
pub(super) fn Ruler(studio: Studio) -> Element {
    let mut width = use_signal(|| 1000.0);
    let start = (studio.view_start)();
    let span = (studio.span)();
    let labels: Vec<_> = studio
        .timing
        .read()
        .as_ref()
        .map(|map| {
            let first = map.tick(start.max(map.offset)).unwrap_or(0) / map.resolution as u64;
            (first..first + 200)
                .map(|beat| map.seconds(beat * map.resolution as u64))
                .take_while(|t| *t <= start + span)
                .filter(|t| *t >= start)
                .collect()
        })
        .unwrap_or_default();
    rsx! {
        div {class:"midi-ruler",style:"display:flex",
            div {style:"width:140px;flex-shrink:0;padding-left:12px;font-size:10px;color:var(--text-muted)","Time"}
            div {style:"flex:1;min-width:0",onresize:move|e|{if let Ok(size)=e.get_content_box_size(){width.set(size.width.max(1.0));}},
                svg {class:"midi-ruler-canvas",view_box:"0 0 {width} 20",preserve_aspect_ratio:"none",
                    for seconds in labels {
                        line {x1:"{(seconds-start)/span*width()}",x2:"{(seconds-start)/span*width()}",y1:"14",y2:"20",stroke:"#78848d"}
                        text {x:"{(seconds-start)/span*width()+3.0}",y:"12",font_size:"9",fill:"#afb8bf","{clock(seconds)}"}
                    }
                }
            }
        }
    }
}
#[component]
pub(super) fn TrackRoll(mut studio: Studio, track: String, show_hidden: bool) -> Element {
    let links = super::difficulty::shared_snapshot();
    let links = links.read();
    let project = studio.doc.read();
    let doc = project.as_ref().unwrap();
    let timing = studio.timing.read().as_ref().unwrap().clone();
    let rows = lane_rows(&track);
    let height = rows.len() as f64 * 20.0;
    let mut width = use_signal(|| 1000.0);
    let mut drag = use_signal(|| None::<Drag>);
    let mut delta = use_signal(|| (0.0_f64, 0.0_f64));
    let gestures = super::note_gestures::use_gestures(studio);
    let start = (studio.view_start)();
    let span = (studio.span)();
    let geometry = use_memo(use_reactive(
        (&track, &show_hidden),
        move |(track, show_hidden)| {
            let project = studio.doc.read();
            let timing = studio.timing.read();
            match (project.as_ref(), timing.as_ref()) {
                (Some(doc), Some(timing)) => track_geometry(
                    doc,
                    timing,
                    &track,
                    show_hidden,
                    (studio.view_start)(),
                    (studio.span)(),
                    width(),
                ),
                _ => (String::new(), Vec::new()),
            }
        },
    ));
    let geometry = geometry.read();
    let (grid, notes) = &*geometry;
    let active = doc.selected_track == track;
    let selected = studio.selection();
    let plot_track = track.clone();
    let double_track = track.clone();
    let box_track = track.clone();
    let on_release = move |e: MouseEvent| {
        gestures.release(&e, width(), 20.);
        if let Some(origin) = drag.take() {
            let dx = e.page_coordinates().x - origin.start_x;
            let dy = e.page_coordinates().y - origin.start_y;
            if dx.abs() + dy.abs() < 3.0 {
                return;
            }
            let Some(timing) = studio.timing.read().clone() else {
                return;
            };
            let origin_time = timing.seconds(origin.tick);
            if origin.resize {
                let end =
                    timing.seconds(origin.tick + origin.sustain) + dx / width() * (studio.span)();
                if let Some(end) = snap(studio, end.max(origin_time)) {
                    studio.dispatch(Action::UpdateNote {
                        note_id: origin.id,
                        tick: origin.tick,
                        lane: origin.lane,
                        sustain: end.saturating_sub(origin.tick),
                        cymbal: origin.cymbal,
                    });
                }
            }
        }
    };
    rsx! {
        header{class:"lane-track-header",button{aria_pressed:active.to_string(),onclick:{let track=track.clone();move |_|studio.dispatch(Action::SelectTrack{track:track.clone()})},"{super::track_picker::title(&track)}"}super::difficulty::TrackLinkBadge{track:track.clone()}span{"{note_count(doc.content.tracks[&track].iter().filter(|n|editor::playable(&track,n.note.lane)).count())}"}}
        InstrumentWaveform{studio,track:track.clone()}
        InlineReview{studio,track:track.clone(),show_hidden,plot_width:width()}
        div{class:"lane-surface",ol{class:"lane-labels",for lane in rows.clone(){li{class:"lane-label",style:"color:{color(&track,lane)}","{lane_name(&track,lane)}"}}}
            div{class:"lane-plot",onresize:move|e|{if let Ok(size)=e.get_content_box_size(){width.set(size.width.max(1.0));}},
                svg{class:"studio-note-svg",height:"{height}",view_box:"0 0 {width} {height}",preserve_aspect_ratio:"none",
                    onmousedown:move|e:MouseEvent|gestures.background(&box_track,&e),
                    onclick:move|e:MouseEvent|{if gestures.consume_click(){return;}studio.focus_editor();let seconds=(studio.view_start)()+e.element_coordinates().x/width()*(studio.span)();if (studio.tool)()==EditTool::Place{place(studio,&plot_track,&e,width());}else if (studio.tool)()==EditTool::Select{studio.set_selection(crate::note_groups::Selection{track:plot_track.clone(),..Default::default()});let view=(studio.view_start)();studio.seek(seconds);studio.view_start.set(view);}},
                    ondoubleclick:move|e:MouseEvent|{if (studio.tool)()==EditTool::Select{place(studio,&double_track,&e,width());}},
                    onmousemove:move|e:MouseEvent|{gestures.moving(&e);if let Some(origin)=drag.read().as_ref(){delta.set((e.page_coordinates().x-origin.start_x,e.page_coordinates().y-origin.start_y));}},
                    onmouseup:on_release,
                    onmouseleave:move |_|{drag.set(None);delta.set((0.0,0.0));gestures.cancel();},
                    g{pointer_events:"none",dangerous_inner_html:"{grid}"}
                    for item in notes{
                        {let overridden=links.as_ref().is_some_and(|snapshot|super::difficulty::is_override(snapshot,&track,&item.id));
                        {let row=rows.iter().position(|l|*l==item.note.lane).unwrap();let x=(timing.seconds(item.note.tick)-start)/span*width();let end=(timing.seconds(item.note.tick+item.note.sustain)-start)/span*width();let y=row as f64*20.0+3.0;let fill=color(&track,item.note.lane);let id=item.id.clone();let down_id=item.id.clone();let click_track=track.clone();let down_track=track.clone();let id2=item.id.clone();let note_tick=item.note.tick;let origin=Drag{id:item.id.clone(),tick:item.note.tick,lane:item.note.lane,sustain:item.note.sustain,cymbal:doc.is_cymbal(&track,&item.note),start_x:0.0,start_y:0.0,resize:false};let origin2=origin.clone();let is_selected=active&&selected.ids.contains(&item.id);let stroke=if is_selected{"#fff"}else{"#090b0d"};
                        rsx!{g{
                            ondoubleclick:move|e|e.stop_propagation(),
                            onclick:move|e:MouseEvent|{e.stop_propagation();if gestures.consume_click(){return;}studio.select_gem(&click_track,&id,e.modifiers().shift(),false);if (studio.tool)()==EditTool::Erase{studio.delete_selection();}},
                            onmousedown:move|e:MouseEvent|{e.stop_propagation();gestures.note(&down_track,Some(&down_id),0,note_tick,&e);},
                            if item.note.sustain>0{rect{x:"{x.max(0.0)}",y:"{y+4.0}",width:"{(end.min(width())-x.max(0.0)).max(0.0)}",height:"6",rx:"2",fill,opacity:".55"}}
                            rect{x:"{x}",y:"{y}",width:"14",height:"14",rx:if doc.is_cymbal(&track,&item.note){"1"}else{"3"},fill,stroke,stroke_width:"1.5",role:"button","aria-pressed":is_selected.to_string(),"aria-label":"{lane_name(&track,item.note.lane)} note at {timing.seconds(item.note.tick):.3} seconds",title{"{lane_name(&track,item.note.lane)} · {timing.seconds(item.note.tick):.4}s · {item.id}"}}
                            if overridden{rect{x:"{x-1.0}",y:"{y-1.0}",width:"16",height:"16",rx:"3",fill:"none",stroke:"#c889ff",stroke_width:"1",stroke_dasharray:"2 2",pointer_events:"none",title{"Local difficulty override"}}}
                            if item.note.sustain>0{rect{x:"{end-4.0}",y:"{y}",width:"8",height:"14",fill,style:"cursor:ew-resize",onclick:move|e|{e.stop_propagation();studio.dispatch(Action::SelectNote{note_id:Some(id2.clone())});},onmousedown:move|e:MouseEvent|{e.stop_propagation();let mut origin=origin2.clone();origin.start_x=e.page_coordinates().x;origin.start_y=e.page_coordinates().y;origin.resize=true;drag.set(Some(origin));delta.set((0.0,0.0));}}}
                        }} } }
                    }
                    {drag().map(|origin|{let row=rows.iter().position(|l|*l==origin.lane).unwrap_or(0);let x=(timing.seconds(origin.tick)-start)/span*width()+delta().0;let y=row as f64*20.0+3.0+delta().1;rsx!{rect{x:"{x}",y:"{y}",width:"16",height:"14",fill:"none",stroke:"#fff",pointer_events:"none"}}})}
                    {gestures.overlay(width(),20.)}
                    Playhead{studio,width:width(),height}
                }
            }
        }
    }
}

fn track_geometry(
    doc: &editor::Project,
    timing: &crate::timing::TempoMap,
    track: &str,
    show_hidden: bool,
    start: f64,
    span: f64,
    width: f64,
) -> (String, Vec<editor::EditorNote>) {
    let rows = lane_rows(track);
    let height = rows.len() as f64 * 20.0;
    let mut grid = String::new();
    for row in 0..=rows.len() {
        let y = row * 20;
        let _ = write!(grid, "<path d='M0 {y}H{}' stroke='#2e3239'/>", width);
    }
    for region in &doc.review.regions {
        if region.tracks.contains(track)
            && region.end > start
            && region.start < start + span
            && (show_hidden || !doc.is_flag_hidden(&region.id, track))
        {
            let a = ((region.start - start) / span * width).max(0.0);
            let b = ((region.end - start) / span * width).min(width);
            let _ = write!(
                grid,
                "<rect x='{a}' y='0' width='{}' height='{height}' fill='{}' opacity='.10'/>",
                b - a,
                flag_color(doc, &region.id, track, timing)
            );
        }
    }
    let first = timing.tick(start.max(timing.offset)).unwrap_or(0);
    let last = timing.tick(start + span).unwrap_or(first);
    let step = (timing.resolution as u64 / 4).max(1);
    for tick in (first / step * step..=last).step_by(step as usize) {
        let x = (timing.seconds(tick) - start) / span * width;
        let beat = tick % timing.resolution as u64 == 0;
        let _ = write!(
            grid,
            "<path d='M{x} 0V{height}' stroke='{}'/>",
            if beat { "#51515c" } else { "#292b34" }
        );
    }
    let notes: Vec<_> = doc.content.tracks[track]
        .iter()
        .filter(|n| editor::playable(track, n.note.lane))
        .filter(|n| {
            let s = timing.seconds(n.note.tick);
            let e = timing.seconds(n.note.tick + n.note.sustain);
            e >= start && s <= start + span
        })
        .cloned()
        .collect();
    (grid, notes)
}
