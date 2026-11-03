//! Shared-transport practice controls. No extra player or playback clock.
use super::icons::Icon;
use super::session;
use super::state::{Studio, clock};
use crate::editor::{Action, TimeRange};
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
fn selected(doc: &session::Transport) -> Option<(&str, TimeRange)> {
    doc.selected
        .as_ref()
        .map(|(id, range)| (id.as_str(), *range))
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
    let Some(doc) = session::transport(studio) else {
        return;
    };
    let current = draft.read().clone();
    if current.source != super::dance::source_identity().or_else(|| (studio.source_audio)()) {
        draft.write().error =
            Some("The recording changed; use the current project's loop points.".into());
        return;
    }
    match parse(&current, doc.duration) {
        Ok(range) => {
            session::set_loop(studio, Some(range));
            if session::transport(studio).is_some_and(|project| project.loop_range == Some(range)) {
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
        session::set_loop(studio, None);
        draft.write().error = None;
        return;
    }
    let Some(doc) = session::transport(studio) else {
        return;
    };
    if !draft.read().dirty {
        let range = selected(&doc).map(|(_, range)| range).unwrap_or_else(|| {
            let start = session::position(studio).clamp(0., (doc.duration - 0.001).max(0.));
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

#[derive(Clone, Default)]
struct RateDraft {
    project: Option<PathBuf>,
    saved: f64,
    text: String,
    dirty: bool,
    error: Option<String>,
}
fn percentage(text: &str) -> Result<f64, &'static str> {
    let percent = text
        .trim()
        .parse::<f64>()
        .map_err(|_| "Enter a speed from 25 to 200%.")?;
    if !percent.is_finite() || !(25. ..=200.).contains(&percent) {
        return Err("Enter a speed from 25 to 200%.");
    }
    Ok(percent / 100.)
}
fn percent_text(rate: f64) -> String {
    let value = format!("{:.6}", rate * 100.);
    value.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod rate_tests {
    use super::*;
    #[test]
    fn exact_percentages_retain_custom_saved_rates_and_reject_invalid_edits() {
        for rate in [0.25, 0.5, 0.83, 1., 1.5, 2., 0.835, 1.234567] {
            assert!((percentage(&percent_text(rate)).unwrap() - rate).abs() < 1e-8);
        }
        for value in ["", "-1", "24.99", "200.01", "NaN", "inf", "speed"] {
            assert!(percentage(value).is_err(), "Accepted {value}");
        }
        assert_eq!(percentage(" 83 ").unwrap(), 0.83);
    }
}
fn stage_rate(studio: Studio, mut draft: Signal<RateDraft>, text: String) {
    if draft.peek().project != session::project_identity(studio) {
        return;
    }
    let saved = draft.peek().saved;
    let parsed = percentage(&text);
    draft.with_mut(|draft| {
        draft.text = text;
        draft.dirty = true;
        draft.error = parsed.as_ref().err().map(|error| (*error).to_owned());
    });
    if let Ok(rate) = parsed {
        session::preview_rate(studio, Some(rate));
    } else {
        // The slider falls back to the saved rate for an invalid draft; the
        // actual player must do the same, even before blur or Enter.
        session::preview_rate(studio, Some(saved));
        session::preview_rate(studio, None);
    }
}
fn commit_rate(studio: Studio, mut draft: Signal<RateDraft>) {
    let current = draft.peek().clone();
    if !current.dirty {
        return;
    }
    if current.project != session::project_identity(studio) {
        return;
    }
    match percentage(&current.text) {
        Ok(rate) => {
            session::rate(studio, rate);
            if session::transport(studio).is_some_and(|doc| (doc.playback_rate - rate).abs() < 1e-9)
            {
                draft.with_mut(|draft| {
                    draft.saved = rate;
                    draft.text = percent_text(rate);
                    draft.dirty = false;
                    draft.error = None;
                });
            } else {
                draft.write().error =
                    Some("Speed was not saved. Check the project error and retry.".into());
            }
        }
        Err(error) => {
            session::preview_rate(studio, Some(current.saved));
            session::preview_rate(studio, None);
            draft.write().error = Some(error.into());
        }
    }
}
#[component]
fn PracticeSpeed(studio: Studio, loaded: bool, rate: f64) -> Element {
    let mut draft = use_signal(RateDraft::default);
    use_drop(move || studio.preview_playback_rate(None));
    use_effect(move || {
        let project = session::project_identity(studio);
        let rate = session::transport(studio).map_or(1., |doc| doc.playback_rate);
        if draft.peek().project != project || (draft.peek().saved - rate).abs() > 1e-9 {
            // A project switch or another client's saved command supersedes a drag.
            studio.preview_playback_rate(None);
            draft.set(RateDraft {
                project,
                saved: rate,
                text: percent_text(rate),
                ..Default::default()
            });
        }
    });
    let view = draft.read().clone();
    let slider = percentage(&view.text).unwrap_or(rate) * 100.;
    rsx! {
        div { class:"practice-speed", role:"group", aria_label:"Pitch-preserving practice speed",
            label { "Speed"
                input { r#type:"range", min:"25", max:"200", step:"1", value:"{slider}", disabled:!loaded,
                    aria_label:"Practice speed slider", title:"Pitch-preserving speed: 25–200%",
                    oninput:move|event|stage_rate(studio,draft,event.value()),
                    onchange:move|_|commit_rate(studio,draft),
                    onblur:move|_|commit_rate(studio,draft),
                    onpointerup:move|_|commit_rate(studio,draft),
                    onpointercancel:move|_|commit_rate(studio,draft),
                }
            }
            label { class:"practice-speed-exact",
                input { r#type:"number", min:"25", max:"200", step:"any", value:view.text.clone(), disabled:!loaded,
                    aria_label:"Practice speed percent", aria_invalid:view.error.is_some().to_string(), title:"Exact pitch-preserving speed: 25–200%",
                    oninput:move|event|stage_rate(studio,draft,event.value()),
                    onchange:move|_|commit_rate(studio,draft),
                    onblur:move|_|commit_rate(studio,draft),
                    onkeydown:move|event:KeyboardEvent| { if event.key()==Key::Enter { event.prevent_default(); commit_rate(studio,draft); } else if event.key()==Key::Escape { event.prevent_default(); let saved=draft.peek().saved; session::preview_rate(studio,Some(saved)); session::preview_rate(studio,None); draft.with_mut(|draft| {draft.text=percent_text(saved);draft.dirty=false;draft.error=None;}); } },
                }
                "%"
            }
            button { class:"instrument-toggle", disabled:!loaded, aria_label:"Reset practice speed to 100 percent", title:"Reset speed to 100%",
                onclick:move|_| { stage_rate(studio,draft,"100".into()); commit_rate(studio,draft); }, "100%" }
            if let Some(error)=view.error.clone() { span { class:"field-hint practice-speed-error", role:"alert", "{error}" } }
        }
    }
}

#[component]
pub fn PracticeControls(studio: Studio, children: Element) -> Element {
    let mut draft = use_signal(LoopDraft::default);
    let mut edit_loop = use_signal(|| false);
    let mut options_open = use_signal(|| false);
    let mut summary_node = use_signal(|| None::<std::rc::Rc<MountedData>>);
    use_effect(move || {
        let source = super::dance::source_identity().or_else(|| (studio.source_audio)());
        let Some(doc) = session::transport(studio) else {
            return;
        };
        let active = doc.loop_range;
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
    let document = session::transport(studio);
    let doc = document.as_ref();
    let loaded = doc.is_some();
    let playing = doc.as_ref().is_some_and(|doc| doc.playing);
    let loop_on = doc.as_ref().is_some_and(|doc| doc.loop_range.is_some());
    let selected_id = doc
        .as_ref()
        .and_then(|doc| selected(doc).map(|(id, _)| id.to_owned()));
    let duration = doc.as_ref().map_or(0., |doc| doc.duration);
    let rate = doc.as_ref().map_or(1., |doc| doc.playback_rate);
    let position = session::position(studio).clamp(0., duration);
    let view = draft.read().clone();
    let valid_draft = loaded && parse(&view, duration).is_ok();
    let error = view
        .error
        .clone()
        .or_else(|| (view.dirty && !valid_draft).then(|| parse(&view, duration).unwrap_err()));
    let source_ready = doc.is_some_and(|doc| doc.ready);
    let playback_icon = if playing { "pause" } else { "play" };
    rsx! {
        div{class:"practice-controls",role:"toolbar",aria_label:"Preview and practice controls",
            onkeydown:move|event:KeyboardEvent|event.stop_propagation(),
            button{class:"instrument-toggle transport-control",aria_pressed:playing.to_string(),disabled:!loaded||!source_ready,aria_label:if playing{"Pause practice recording"}else{"Play practice recording"},title:"Play / pause recording (Space)",onclick:move|_|session::toggle_playback(studio),Icon { name: playback_icon, size: 16 }}
            button{class:"instrument-toggle transport-control",disabled:!loaded,aria_label:"Stop and return to beginning",title:"Stop and return to beginning",onclick:move|_|{session::playback(studio,false);session::seek(studio,0.0);},Icon{name:"square",size:16}}
            span{class:"timeline-time",style:"white-space:nowrap;",title:"{position:.3} / {duration:.3} seconds","{clock(position)} / {clock(duration)}"}
            PracticeSpeed { studio, loaded, rate }
            label{class:"practice-volume",title:"Recording volume",Icon{name:"volume",size:15}
                input{r#type:"range",min:"0",max:"1",step:"0.01",value:"{(studio.volume)()}",disabled:!loaded,aria_label:"Recording volume",oninput:move|event|{if let Ok(volume)=event.value().parse(){session::volume(studio,volume);}}}
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
                  button{class:"instrument-toggle",disabled:!loaded,title:"Seek backward two original-recording seconds",aria_label:"Seek backward 2 seconds",onclick:move|_|session::seek(studio,session::position(studio)-2.),Icon { name: "skip-back", size: 15 } "2 s"}
                  button{class:"instrument-toggle",disabled:!loaded,title:"Seek forward two original-recording seconds",aria_label:"Seek forward 2 seconds",onclick:move|_|session::seek(studio,session::position(studio)+2.),"2 s" Icon { name: "skip-forward", size: 15 }}
                  label{class:"preview-snap-selector",input{r#type:"checkbox",checked:loop_on,disabled:!loaded,onchange:move|event|enable_loop(studio,draft,event.checked())},"Loop"}
                  button{class:"instrument-toggle",disabled:selected_id.is_none(),title:"Loop the selected review region",onclick:move|_|{if let Some(id)=session::transport(studio).and_then(|doc|selected(&doc).map(|(id,_)|id.to_owned())){draft.with_mut(|draft|{draft.dirty=false;draft.changed_elsewhere=false;draft.error=None;});studio.dispatch(Action::SelectRegion{region_id:id});}},"Loop section"}
            button{class:if edit_loop(){"instrument-toggle active"}else{"instrument-toggle"},style:"margin-left:auto;white-space:nowrap;",aria_expanded:edit_loop(),onclick:move|_|edit_loop.set(!edit_loop()),"Edit loop"}
                }
            if edit_loop(){
                div{style:"display:flex;flex:1 0 100%;flex-wrap:wrap;align-items:center;gap:8px;padding-top:4px;",
                    label{class:"preview-snap-selector","In (s)" input{class:"field-control",style:"width:88px;height:28px;border-radius:4px;padding:4px 8px;",r#type:"text",inputmode:"decimal",aria_label:"Practice loop in seconds",disabled:!loaded,value:view.start.clone(),oninput:move|event|stage(draft,Some(event.value()),None)}}
                    button{class:"instrument-toggle",disabled:!loaded,title:"Use current playhead as staged loop in point",onclick:move|_|stage(draft,Some(session::position(studio).to_string()),None),"In here"}
                    label{class:"preview-snap-selector","Out (s)" input{class:"field-control",style:"width:88px;height:28px;border-radius:4px;padding:4px 8px;",r#type:"text",inputmode:"decimal",aria_label:"Practice loop out seconds",disabled:!loaded,value:view.end.clone(),oninput:move|event|stage(draft,None,Some(event.value()))}}
                    button{class:"instrument-toggle",disabled:!loaded,title:"Use current playhead as staged loop out point",onclick:move|_|stage(draft,None,Some(session::position(studio).to_string())),"Out here"}
                    button{class:"instrument-toggle",disabled:!valid_draft,title:"Apply these exact-second loop points",onclick:move|_|commit(studio,draft),"Apply loop"}
                    button{class:"instrument-toggle",disabled:!loop_on,onclick:move|_|{session::set_loop(studio,None);draft.with_mut(|draft|{draft.changed_elsewhere=false;draft.error=None;});},"Clear loop"}
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
