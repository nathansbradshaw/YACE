//! Shared-transport practice controls. No extra player or playback clock.
use super::icons::Icon;
use super::state::{Studio, clock};
use crate::editor::{Action, Project, TimeRange};
use dioxus::prelude::*;
use std::path::PathBuf;

#[derive(Clone)]
struct LoopDraft {
    source: Option<PathBuf>,
    observed: Option<TimeRange>,
    start: String,
    end: String,
    dirty: bool,
    changed_elsewhere: bool,
    error: Option<String>,
}
impl Default for LoopDraft {
    fn default() -> Self {
        Self {
            source: None,
            observed: None,
            start: "0".into(),
            end: "8".into(),
            dirty: false,
            changed_elsewhere: false,
            error: None,
        }
    }
}
fn selected(doc: &Project) -> Option<(&str, TimeRange)> {
    let id = doc.selected_region.as_deref()?;
    doc.review
        .regions
        .iter()
        .find(|region| region.id == id)
        .map(|region| {
            (
                id,
                TimeRange {
                    start: region.start,
                    end: region.end,
                },
            )
        })
}
fn valid(range: TimeRange, duration: f64) -> bool {
    range.start.is_finite()
        && range.end.is_finite()
        && range.start >= 0.
        && range.start < range.end
        && range.end <= duration
}
fn parse(draft: &LoopDraft, duration: f64) -> Result<TimeRange, String> {
    let start = draft
        .start
        .trim()
        .parse::<f64>()
        .map_err(|_| "Loop in must be a number of seconds.".to_owned())?;
    let end = draft
        .end
        .trim()
        .parse::<f64>()
        .map_err(|_| "Loop out must be a number of seconds.".to_owned())?;
    let range = TimeRange { start, end };
    if !valid(range, duration) {
        return Err(format!(
            "Use finite seconds with 0 ≤ loop in < loop out ≤ {duration:.3}."
        ));
    }
    Ok(range)
}
fn stage(mut draft: Signal<LoopDraft>, start: Option<String>, end: Option<String>) {
    draft.with_mut(|draft| {
        if let Some(start) = start {
            draft.start = start;
        }
        if let Some(end) = end {
            draft.end = end;
        }
        draft.dirty = true;
        draft.error = None;
    });
}
fn commit(studio: Studio, mut draft: Signal<LoopDraft>) {
    let Some(doc) = studio.snapshot() else { return };
    let current = draft.read().clone();
    if current.source != (studio.source_audio)() {
        draft.write().error =
            Some("The recording changed; use the current project's loop points.".into());
        return;
    }
    match parse(&current, doc.duration) {
        Ok(range) => {
            studio.dispatch(Action::SetLoop { range: Some(range) });
            if studio
                .snapshot()
                .is_some_and(|project| project.transport.loop_range == Some(range))
            {
                draft.with_mut(|draft| {
                    draft.dirty = false;
                    draft.changed_elsewhere = false;
                    draft.error = None;
                });
            } else {
                draft.write().error=Some("Loop was not applied. Your staged points are retained; check the shared transport error and retry.".into());
            }
        }
        Err(error) => draft.write().error = Some(error),
    }
}
fn enable_loop(studio: Studio, mut draft: Signal<LoopDraft>, enabled: bool) {
    if !enabled {
        studio.dispatch(Action::SetLoop { range: None });
        draft.write().error = None;
        return;
    }
    let Some(doc) = studio.snapshot() else { return };
    if !draft.read().dirty {
        let range = selected(&doc).map(|(_, range)| range).unwrap_or_else(|| {
            let start = (studio.playhead)().clamp(0., (doc.duration - 0.001).max(0.));
            TimeRange {
                start,
                end: (start + 8.).min(doc.duration),
            }
        });
        draft.with_mut(|draft| {
            draft.start = range.start.to_string();
            draft.end = range.end.to_string();
        });
    }
    commit(studio, draft);
}

#[component]
pub fn PracticeControls(studio: Studio, children: Element) -> Element {
    let mut draft = use_signal(LoopDraft::default);
    let mut edit_loop = use_signal(|| false);
    let mut options_open = use_signal(|| false);
    let mut summary_node = use_signal(|| None::<std::rc::Rc<MountedData>>);
    use_effect(move || {
        let source = (studio.source_audio)();
        let Some(doc) = studio.snapshot() else { return };
        let active = doc.transport.loop_range;
        draft.with_mut(|draft| {
            if draft.source != source {
                let range = active
                    .or_else(|| selected(&doc).map(|(_, range)| range))
                    .unwrap_or(TimeRange {
                        start: 0.,
                        end: doc.duration.min(8.),
                    });
                *draft = LoopDraft {
                    source,
                    observed: active,
                    start: range.start.to_string(),
                    end: range.end.to_string(),
                    ..LoopDraft::default()
                };
            } else if draft.observed != active {
                if draft.dirty {
                    draft.changed_elsewhere = true;
                } else if let Some(range) = active {
                    draft.start = range.start.to_string();
                    draft.end = range.end.to_string();
                }
                draft.observed = active;
            }
        });
    });
    let document = studio.doc.read();
    let doc = document.as_ref();
    let loaded = doc.is_some();
    let playing = doc.as_ref().is_some_and(|doc| doc.transport.playing);
    let loop_on = doc
        .as_ref()
        .is_some_and(|doc| doc.transport.loop_range.is_some());
    let selected_id = doc
        .as_ref()
        .and_then(|doc| selected(doc).map(|(id, _)| id.to_owned()));
    let duration = doc.as_ref().map_or(0., |doc| doc.duration);
    let rate = doc.as_ref().map_or(1., |doc| doc.transport.playback_rate);
    let position = (studio.playhead)().clamp(0., duration);
    let view = draft.read().clone();
    let valid_draft = loaded && parse(&view, duration).is_ok();
    let error = view
        .error
        .clone()
        .or_else(|| (view.dirty && !valid_draft).then(|| parse(&view, duration).unwrap_err()));
    let source_ready = (studio.audio_ready)();
    let playback_icon = if playing { "pause" } else { "play" };
    rsx! {
        div{class:"practice-controls",role:"toolbar",aria_label:"Preview and practice controls",
            onkeydown:move|event:KeyboardEvent|event.stop_propagation(),
            button{class:"instrument-toggle transport-control",aria_pressed:playing.to_string(),disabled:!loaded||!source_ready,aria_label:if playing{"Pause practice recording"}else{"Play practice recording"},title:"Play / pause recording (Space)",onclick:move|_|studio.toggle_playback(),Icon { name: playback_icon, size: 16 }}
            button{class:"instrument-toggle transport-control",disabled:!loaded,aria_label:"Stop and return to beginning",title:"Stop and return to beginning",onclick:move|_|{studio.dispatch(Action::Playback{playing:false});studio.seek(0.0);},Icon{name:"square",size:16}}
            span{class:"timeline-time",style:"white-space:nowrap;",title:"{position:.3} / {duration:.3} seconds","{clock(position)} / {clock(duration)}"}
            label{class:"preview-snap-selector","Speed" select{disabled:!loaded,aria_label:"Pitch-preserving practice speed",value:rate.to_string(),onchange:move|event|{if let Ok(rate)=event.value().parse(){studio.set_playback_rate(rate);}},
                option{value:"0.5",selected:rate==0.5,"50%"}option{value:"0.75",selected:rate==0.75,"75%"}option{value:"1",selected:rate==1.0,"100%"}
                if ![0.5,0.75,1.0].contains(&rate){option{value:rate.to_string(),selected:true,"{rate:.2}× (current)"}}
            }}
            label{class:"practice-volume",title:"Recording volume",Icon{name:"volume",size:15}
                input{r#type:"range",min:"0",max:"1",step:"0.01",value:"{(studio.volume)()}",disabled:!loaded,aria_label:"Recording volume",oninput:move|event|{if let Ok(volume)=event.value().parse(){studio.update_volume(volume);}}}
            }
            if loaded&&!source_ready{span{class:"field-hint",role:"status","Audio not ready"}}
            details { class: "preview-options", open: options_open(),
              onkeydown: move |event: KeyboardEvent| {
                if event.key() == Key::Escape {
                  event.prevent_default();
                  options_open.set(false);
                  if let Some(node) = summary_node() { spawn(async move { let _ = node.set_focus(true).await; }); }
                }
              },
              summary { onmounted: move |event| summary_node.set(Some(event.data())), onclick: move |event| { event.prevent_default(); options_open.set(!options_open()); }, "Playback options" }
              div { class: "preview-options-body",
                div { class: "practice-loop-actions",
                  button{class:"instrument-toggle",disabled:!loaded,title:"Seek backward two original-recording seconds",aria_label:"Seek backward 2 seconds",onclick:move|_|studio.seek((studio.playhead)()-2.),Icon { name: "skip-back", size: 15 } "2 s"}
                  button{class:"instrument-toggle",disabled:!loaded,title:"Seek forward two original-recording seconds",aria_label:"Seek forward 2 seconds",onclick:move|_|studio.seek((studio.playhead)()+2.),"2 s" Icon { name: "skip-forward", size: 15 }}
                  label{class:"preview-snap-selector",input{r#type:"checkbox",checked:loop_on,disabled:!loaded,onchange:move|event|enable_loop(studio,draft,event.checked())},"Loop"}
                  button{class:"instrument-toggle",disabled:selected_id.is_none(),title:"Loop the selected review region",onclick:move|_|{if let Some(id)=studio.snapshot().and_then(|doc|selected(&doc).map(|(id,_)|id.to_owned())){draft.with_mut(|draft|{draft.dirty=false;draft.changed_elsewhere=false;draft.error=None;});studio.dispatch(Action::SelectRegion{region_id:id});}},"Loop section"}
            button{class:if edit_loop(){"instrument-toggle active"}else{"instrument-toggle"},style:"margin-left:auto;white-space:nowrap;",aria_expanded:edit_loop(),onclick:move|_|edit_loop.set(!edit_loop()),"Edit loop"}
                }
            if edit_loop(){
                div{style:"display:flex;flex:1 0 100%;flex-wrap:wrap;align-items:center;gap:8px;padding-top:4px;",
                    label{class:"preview-snap-selector","In (s)" input{class:"field-control",style:"width:88px;height:28px;border-radius:4px;padding:4px 8px;",r#type:"text",inputmode:"decimal",aria_label:"Practice loop in seconds",disabled:!loaded,value:view.start.clone(),oninput:move|event|stage(draft,Some(event.value()),None)}}
                    button{class:"instrument-toggle",disabled:!loaded,title:"Use current playhead as staged loop in point",onclick:move|_|stage(draft,Some((studio.playhead)().to_string()),None),"In here"}
                    label{class:"preview-snap-selector","Out (s)" input{class:"field-control",style:"width:88px;height:28px;border-radius:4px;padding:4px 8px;",r#type:"text",inputmode:"decimal",aria_label:"Practice loop out seconds",disabled:!loaded,value:view.end.clone(),oninput:move|event|stage(draft,None,Some(event.value()))}}
                    button{class:"instrument-toggle",disabled:!loaded,title:"Use current playhead as staged loop out point",onclick:move|_|stage(draft,None,Some((studio.playhead)().to_string())),"Out here"}
                    button{class:"instrument-toggle",disabled:!valid_draft,title:"Apply these exact-second loop points",onclick:move|_|commit(studio,draft),"Apply loop"}
                    button{class:"instrument-toggle",disabled:!loop_on,onclick:move|_|{studio.dispatch(Action::SetLoop{range:None});draft.with_mut(|draft|{draft.changed_elsewhere=false;draft.error=None;});},"Clear loop"}
                    if view.changed_elsewhere{span{class:"field-hint","Loop changed elsewhere; Apply commits your staged points."}}
                    if let Some(error)=error{span{class:"field-hint",role:"alert",style:"color:var(--danger-color);","{error}"}}
                }
            }
                {children}
              }
            }
        }
    }
}
