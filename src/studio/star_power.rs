//! Small per-track controls and draggable ranges above the note lanes.
use super::{
    icons::Icon,
    state::{Studio, clock},
    track_picker,
};
use crate::{
    editor::Action,
    star_power::{Edit, Phrase},
};
use dioxus::{html::input_data::MouseButton, prelude::*};

fn current(studio: Studio, track: &str) -> Option<Vec<Phrase>> {
    let doc = studio.doc.read();
    if let Some(phrases) = doc.as_ref()?.content.star_power.get(track) {
        return Some(phrases.clone());
    }
    studio
        .source_star_power
        .read()
        .get(track)
        .cloned()
        .unwrap_or_else(|| Some(Vec::new()))
}
fn open(mut studio: Studio, track: &str, phrase: Option<Phrase>) {
    if studio
        .snapshot()
        .is_some_and(|doc| doc.selected_track != track)
    {
        studio.dispatch(Action::SelectTrack {
            track: track.into(),
        });
    }
    studio.star_power_target.set(phrase);
    studio.star_power_open.set(true);
}
fn step(studio: Studio) -> u64 {
    let division = (studio.snap)();
    if division == 0 {
        return 1;
    }
    studio
        .timing
        .read()
        .as_ref()
        .map_or(1, |map| (map.resolution as u64 / division as u64).max(1))
}
fn at_playhead(studio: Studio) -> Phrase {
    let timing = studio.timing.read();
    let Some(map) = timing.as_ref() else {
        return Phrase { start: 0, end: 1 };
    };
    let start = super::note_canvas::snap(studio, (studio.playhead)()).unwrap_or(0);
    let last = studio
        .snapshot()
        .and_then(|doc| map.tick(doc.duration))
        .unwrap_or(start);
    Phrase {
        start,
        end: start
            .saturating_add(map.resolution as u64 * 4)
            .min(last)
            .max(start.saturating_add(1)),
    }
}
#[component]
pub fn Controls(studio: Studio, track: String) -> Element {
    let phrases = current(studio, &track);
    let count = phrases.as_ref().map_or(0, Vec::len);
    let noun = if count == 1 { "phrase" } else { "phrases" };
    let edit_track = track.clone();
    rsx! {
        div {class:"power-controls",
            button {class:"studio-tool",aria_label:"Edit Star Power",onclick:move |_|open(studio,&edit_track,None),"Star Power"}
            span {class:"field-hint","{count} {noun}"}
        }
        if (studio.star_power_open)() {Dialog {studio,track}}
    }
}
#[component]
fn Dialog(mut studio: Studio, track: String) -> Element {
    let mut previous = use_signal(|| (studio.star_power_target)());
    let mut draft = use_signal(|| previous().unwrap_or_else(|| at_playhead(studio)));
    let mut defaults = use_signal(&*draft);
    let mut start = use_signal(|| draft().start.to_string());
    let mut end = use_signal(|| draft().end.to_string());
    let mut epoch = use_signal(|| 0u64);
    let mut error = use_signal(|| None::<String>);
    let Some(phrases) = current(studio, &track) else {
        return rsx! {super::modal::Modal {id:"star-power-dialog",label:"Star Power",onclose:move |_|studio.star_power_open.set(false),section {class:"sections-dialog",h2 {"Star Power"}p {"This track has unsupported Star Power data."}button {onclick:move |_|studio.star_power_open.set(false),"Close"}}}};
    };
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let title = track_picker::title(&track);
    let selection = studio.selection();
    let has_selection = selection.track == track && !selection.ids.is_empty();
    let mut reset = move |range: Phrase, target: Option<Phrase>| {
        previous.set(target);
        defaults.set(range.clone());
        start.set(range.start.to_string());
        end.set(range.end.to_string());
        draft.set(range);
        epoch.set(epoch().wrapping_add(1));
        error.set(None);
    };
    let save_track = track.clone();
    rsx! {super::modal::Modal {id:"star-power-dialog",label:"Star Power",onclose:move |_|studio.star_power_open.set(false),
        section {class:"sections-dialog power-dialog",
            header {h2 {"Star Power"}button {aria_label:"Close Star Power",onclick:move |_|studio.star_power_open.set(false),Icon {name:"close",size:16}}}
            p {class:"selection-caption","{title}"}
            ul {class:"sections-list timing-list",
                for phrase in phrases {
                    {let select=phrase.clone();let remove=phrase.clone();let remove_track=track.clone();let a=clock(timing.seconds(phrase.start));let b=clock(timing.seconds(phrase.end));
                    rsx! {li {
                        button {class:"section-go",aria_label:"Edit Star Power at tick {phrase.start}",onclick:move |_|reset(select.clone(),Some(select.clone())),span {"{a}–{b}"}time {"{phrase.start}–{phrase.end} ticks"}}
                        button {aria_label:"Delete Star Power at tick {phrase.start}",onclick:move |_|{studio.dispatch(Action::EditStarPower {edit:Edit::Remove {track:remove_track.clone(),phrase:remove.clone()}});error.set((studio.error)());if error().is_none()&&previous().as_ref()==Some(&remove){reset(at_playhead(studio),None);}},Icon {name:"trash",size:14}}
                    }}}
                }
            }
            for key in [epoch()] {
                form {key:"{key}",class:"section-form power-form",onsubmit:{let track=save_track.clone();move |event| {
                    event.prevent_default();
                    let (Ok(a),Ok(b))=(start().parse::<u64>(),end().parse::<u64>()) else {error.set(Some("Enter whole tick numbers.".into()));return};
                    studio.dispatch(Action::EditStarPower {edit:Edit::Put {track:track.clone(),previous:previous(),phrase:Phrase {start:a,end:b}}});
                    error.set((studio.error)());
                    if error().is_none(){reset(at_playhead(studio),None);}
                }},
                    label {"Start tick" input {aria_label:"Star Power start tick",r#type:"number",min:"0",step:"1",initial_value:defaults().start.to_string(),oninput:move |e|start.set(e.value())}}
                    label {"End tick" input {aria_label:"Star Power end tick",r#type:"number",min:"1",step:"1",initial_value:defaults().end.to_string(),oninput:move |e|end.set(e.value())}}
                    div {class:"power-range-actions",
                        button {r#type:"button",onclick:move |_|reset(at_playhead(studio),previous()),"At playhead"}
                        button {r#type:"button",disabled:!has_selection,onclick:move |_|{
                            if let Some(doc)=studio.snapshot(){match crate::star_power::selected(&doc,&studio.selection(),step(studio)) {Ok(range)=>reset(range,previous()),Err(e)=>error.set(Some(e.to_string()))}}
                        },"Selected notes"}
                    }
                    div {class:"section-form-actions",
                        if previous().is_some() {button {r#type:"button",onclick:move |_|reset(at_playhead(studio),None),"New phrase"}}
                        button {r#type:"submit",if previous().is_some(){"Save"}else{"Add"}}
                    }
                }
            }
            p {class:"field-hint","Drag a range to move it. Drag either edge to resize."}
            if let Some(error)=error(){p {class:"section-error",role:"alert","{error}"}}
        }
    }}
}

#[derive(Clone)]
struct Drag {
    phrase: Phrase,
    x: f64,
    edge: i8,
    revision: u64,
}
#[component]
pub fn Lane(mut studio: Studio, track: String) -> Element {
    let mut width = use_signal(|| 1000.0_f64);
    let mut drag = use_signal(|| None::<Drag>);
    let mut delta = use_signal(|| 0.0_f64);
    let mut suppress = use_signal(|| false);
    let phrases = current(studio, &track).unwrap_or_default();
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let start = (studio.view_start)();
    let span = (studio.span)();
    let revision = studio.snapshot().map_or(0, |doc| doc.revision);
    let release_track = track.clone();
    let label_track = track.clone();
    rsx! {if !phrases.is_empty() {
        div {class:"power-lane",
            button {class:"power-lane-label",onclick:move |_|open(studio,&label_track,None),"Star Power"}
            svg {class:"power-lane-plot",view_box:"0 0 {width} 16",preserve_aspect_ratio:"none",onresize:move |event|{if let Ok(size)=event.get_content_box_size(){width.set(size.width.max(1.));}},
                onmousemove:move |event:MouseEvent|{if let Some(origin)=drag.read().as_ref(){delta.set(event.page_coordinates().x-origin.x);}},
                onmouseleave:move |_|{drag.set(None);delta.set(0.);},
                onmouseup:move |event:MouseEvent|{
                    let Some(origin)=drag.take() else {return};
                    let dx=event.page_coordinates().x-origin.x;delta.set(0.);
                    if dx.abs()<3. {return};suppress.set(true);
                    if studio.snapshot().is_none_or(|doc|doc.revision!=origin.revision){studio.error.set(Some("Chart changed. Drag the phrase again.".into()));return;}
                    let Some(map)=(studio.timing)() else {return};
                    let tick=if origin.edge==1 {origin.phrase.end}else{origin.phrase.start};
                    let Some(at)=super::note_canvas::snap(studio,(map.seconds(tick)+dx/width()*span).max(map.offset)) else {return};
                    let mut phrase=origin.phrase.clone();
                    match origin.edge {
                        -1=>phrase.start=at.min(phrase.end-1),
                        1=>phrase.end=at.max(phrase.start.saturating_add(1)),
                        _=>{let Some(end)=at.checked_add(phrase.end-phrase.start) else {return};phrase.start=at;phrase.end=end;},
                    }
                    studio.dispatch(Action::EditStarPower {edit:Edit::Put {track:release_track.clone(),previous:Some(origin.phrase),phrase}});
                },
                for phrase in phrases {
                    {let x=(timing.seconds(phrase.start)-start)/span*width();let end=(timing.seconds(phrase.end)-start)/span*width();let a=x.max(0.);let b=end.min(width());let edit=phrase.clone();let down=phrase.clone();let left=phrase.clone();let right=phrase.clone();let edit_track=track.clone();let key_track=track.clone();let key_phrase=phrase.clone();let dx=drag.read().as_ref().filter(|d|d.phrase==phrase).map_or(0.,|_|delta());
                    rsx! {if b>a {g {
                        rect {x:"{a}",y:"2",width:"{b-a}",height:"12",rx:"2",class:"power-range",style:"cursor:grab",role:"button",tabindex:"0","aria-label":"Edit Star Power from tick {phrase.start} to {phrase.end}",
                            onmousedown:move |e:MouseEvent|{if e.trigger_button()==Some(MouseButton::Primary){e.prevent_default();suppress.set(false);drag.set(Some(Drag {phrase:down.clone(),x:e.page_coordinates().x,edge:0,revision}));}},
                            onclick:move |_|{if suppress.take(){return}open(studio,&edit_track,Some(edit.clone()));},
                            onkeydown:move |e:KeyboardEvent|{e.stop_propagation();if e.key()==Key::Enter || e.key()==Key::Character(" ".into()){e.prevent_default();open(studio,&key_track,Some(key_phrase.clone()));}},
                            title {"Star Power · drag to move"}
                        }
                        if x>=0. {rect {x:"{x}",y:"1",width:"6",height:"14",class:"power-grip",style:"cursor:ew-resize","aria-label":"Resize Star Power start",onmousedown:move |e:MouseEvent|{if e.trigger_button()==Some(MouseButton::Primary){e.prevent_default();suppress.set(false);drag.set(Some(Drag {phrase:left.clone(),x:e.page_coordinates().x,edge:-1,revision}));}}}}
                        if end<=width() {rect {x:"{end-6.}",y:"1",width:"6",height:"14",class:"power-grip",style:"cursor:ew-resize","aria-label":"Resize Star Power end",onmousedown:move |e:MouseEvent|{if e.trigger_button()==Some(MouseButton::Primary){e.prevent_default();suppress.set(false);drag.set(Some(Drag {phrase:right.clone(),x:e.page_coordinates().x,edge:1,revision}));}}}}
                        if dx.abs()>=3. {{let edge=drag.read().as_ref().map_or(0,|d|d.edge);let left=if edge==1 {a}else{a+dx};let right=if edge== -1 {b}else{b+dx};rsx! {rect {x:"{left}",y:"1",width:"{(right-left).max(1.)}",height:"14",fill:"none",stroke:"var(--text-primary)",pointer_events:"none"}}}}
                    }}}
                    }
                }
            }
        }
    }}
}
