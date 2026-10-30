//! Tempo and meter controls share the note editor's clock and undo history.
use super::{
    icons::Icon,
    state::{Studio, clock},
};
use crate::{
    editor::Action,
    sync_track::{Edit, Meter, SyncTrack},
    timing::Tempo,
};
use anyhow::{Context, Result};
use dioxus::prelude::*;
use std::collections::BTreeMap;

pub(super) fn current(studio: Studio) -> Option<SyncTrack> {
    let doc = studio.doc.read();
    let source = studio.source_sync.read();
    let sync = doc.as_ref()?.content.sync.as_ref().or(source.as_ref())?;
    Some(SyncTrack {
        tempos: sync.tempos.clone(),
        meters: sync.meters.clone(),
        archived_vocal_review: None,
    })
}
pub(super) fn meters(studio: Studio) -> Vec<Meter> {
    let doc = studio.doc.read();
    let source = studio.source_sync.read();
    doc.as_ref()
        .and_then(|d| d.content.sync.as_ref())
        .or(source.as_ref())
        .map_or_else(Vec::new, |sync| sync.meters.clone())
}
#[derive(Clone, PartialEq)]
enum Selected {
    Tempo(Tempo),
    Meter(Meter),
}
impl Selected {
    fn tick(&self) -> u64 {
        match self {
            Self::Tempo(t) => t.tick,
            Self::Meter(m) => m.tick,
        }
    }
    fn label(&self) -> String {
        match self {
            Self::Tempo(t) => format!("{} BPM", t.bpm),
            Self::Meter(m) => format!("{}/{}", m.numerator, m.denominator),
        }
    }
    fn remove(&self) -> Edit {
        match self {
            Self::Tempo(t) => Edit::RemoveTempo { tempo: t.clone() },
            Self::Meter(m) => Edit::RemoveMeter { meter: m.clone() },
        }
    }
}
#[derive(Clone)]
struct Draft {
    tempo: bool,
    tick: String,
    bpm: String,
    numerator: String,
    denominator: String,
}
impl Draft {
    fn at(studio: Studio, sync: &SyncTrack) -> Self {
        let tick = super::note_canvas::snap(studio, (studio.playhead)()).unwrap_or(0);
        let bpm = sync
            .tempos
            .iter()
            .rev()
            .find(|t| t.tick <= tick)
            .map_or(120., |t| t.bpm);
        let meter = sync.meters.iter().rev().find(|m| m.tick <= tick);
        Self {
            tempo: true,
            tick: tick.to_string(),
            bpm: bpm.to_string(),
            numerator: meter.map_or(4, |m| m.numerator).to_string(),
            denominator: meter.map_or(4, |m| m.denominator).to_string(),
        }
    }
    fn select(&mut self, selected: &Selected) {
        self.tick = selected.tick().to_string();
        match selected {
            Selected::Tempo(t) => {
                self.tempo = true;
                self.bpm = t.bpm.to_string();
            }
            Selected::Meter(m) => {
                self.tempo = false;
                self.numerator = m.numerator.to_string();
                self.denominator = m.denominator.to_string();
            }
        }
    }
    fn edit(&self, selected: Option<Selected>) -> Result<Edit> {
        let tick = self.tick.parse().context("Enter a whole tick number")?;
        if self.tempo {
            Ok(Edit::PutTempo {
                previous: match selected {
                    Some(Selected::Tempo(t)) => Some(t),
                    _ => None,
                },
                tempo: Tempo {
                    tick,
                    bpm: self.bpm.parse().context("Enter a BPM from 1 to 1000")?,
                },
            })
        } else {
            Ok(Edit::PutMeter {
                previous: match selected {
                    Some(Selected::Meter(m)) => Some(m),
                    _ => None,
                },
                meter: Meter {
                    tick,
                    numerator: self
                        .numerator
                        .parse()
                        .context("Enter a numerator from 1 to 255")?,
                    denominator: self
                        .denominator
                        .parse()
                        .context("Choose a time-signature denominator")?,
                },
            })
        }
    }
}

#[component]
pub fn TimingStrip(mut studio: Studio) -> Element {
    let mut width = use_signal(|| 1000.);
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let sync = current(studio);
    let mut markers: BTreeMap<u64, (String, bool)> = BTreeMap::new();
    if let Some(sync) = &sync {
        for tempo in &sync.tempos {
            markers.insert(tempo.tick, (format!("{} BPM", tempo.bpm), true));
        }
        for meter in &sync.meters {
            let label = format!("{}/{}", meter.numerator, meter.denominator);
            markers
                .entry(meter.tick)
                .and_modify(|(text, _)| {
                    text.push_str(" · ");
                    text.push_str(&label);
                })
                .or_insert((label, false));
        }
    }
    let markers: Vec<_> = markers.into_iter().collect();
    let start = (studio.view_start)();
    let span = (studio.span)();
    rsx! {
        div {class:"section-strip timing-strip",aria_label:"Tempo and time signatures",
            button {class:"section-strip-label",title:"Edit tempo and time signatures",onclick:move |_|{studio.timing_target.set(None);studio.timing_open.set(true);},Icon {name:"sliders",size:12}"Timing"}
            div {class:"section-strip-markers",onresize:move |e|{if let Ok(size)=e.get_content_box_size(){width.set(size.width.max(1.));}},
                for (index,(tick,(label,tempo))) in markers.iter().enumerate() {
                    {let x=(timing.seconds(*tick)-start)/span*width();let end=markers.get(index+1).map_or(width(),|(next,_)|(timing.seconds(*next)-start)/span*width());let length=(end-x-3.).min(width()-x).clamp(2.,210.);let target=(*tempo,*tick);
                    rsx! {if x>=0. && x<width() {button {class:"section-marker",style:"left:{x}px;max-width:{length}px",aria_label:"Edit timing at tick {tick}: {label}",title:"{label} · {clock(timing.seconds(*tick))}",onclick:move |_|{studio.timing_target.set(Some(target));studio.timing_open.set(true);},"{label}"}}}}
                }
            }
        }
        if (studio.timing_open)() {TimingDialog {studio}}
    }
}

#[component]
fn TimingDialog(mut studio: Studio) -> Element {
    let Some(sync) = current(studio) else {
        return rsx! {super::modal::Modal {id:"timing-dialog",label:"Timing",onclose:move |_|studio.timing_open.set(false),section {class:"sections-dialog",h2 {"Timing"}p {"This chart has unsupported time-signature data."}button {onclick:move |_|studio.timing_open.set(false),"Close"}}}};
    };
    let mut selected = use_signal(|| {
        (studio.timing_target)().and_then(|(tempo, tick)| {
            if tempo {
                sync.tempos
                    .iter()
                    .find(|t| t.tick == tick)
                    .cloned()
                    .map(Selected::Tempo)
            } else {
                sync.meters
                    .iter()
                    .find(|m| m.tick == tick)
                    .cloned()
                    .map(Selected::Meter)
            }
        })
    });
    let mut draft = use_signal(|| {
        let mut d = Draft::at(studio, &sync);
        if let Some(s) = selected() {
            d.select(&s);
        }
        d
    });
    let mut defaults = use_signal(&*draft);
    let mut epoch = use_signal(|| 0u64);
    let mut error = use_signal(|| None::<String>);
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let mut rows: Vec<_> = sync
        .tempos
        .iter()
        .cloned()
        .map(Selected::Tempo)
        .chain(sync.meters.iter().cloned().map(Selected::Meter))
        .collect();
    rows.sort_by_key(|s| s.tick());
    let archive = studio.doc.read().as_ref().is_some_and(|doc| {
        doc.content.vocal_review.is_none()
            && doc
                .content
                .sync
                .as_ref()
                .is_some_and(|s| s.archived_vocal_review.is_some())
    });
    let mut reset = move |mut new: Draft, target: Option<Selected>| {
        if let Some(s) = &target {
            new.select(s);
        }
        selected.set(target);
        defaults.set(new.clone());
        draft.set(new);
        epoch.set(epoch().wrapping_add(1));
        error.set(None);
    };
    let mut save = move |_| {
        let edit = match draft().edit(selected()) {
            Ok(edit) => edit,
            Err(e) => {
                error.set(Some(e.to_string()));
                return;
            }
        };
        studio.dispatch(Action::EditSync { edit });
        error.set((studio.error)());
        if error().is_none()
            && let Some(sync) = current(studio)
        {
            reset(Draft::at(studio, &sync), None);
        }
    };
    rsx! {
        super::modal::Modal {id:"timing-dialog",label:"Timing",onclose:move |_|studio.timing_open.set(false),
            section {class:"sections-dialog timing-dialog",
                header {h2 {"Timing"}button {aria_label:"Close timing",onclick:move |_|studio.timing_open.set(false),Icon {name:"close",size:16}}}
                p {class:"field-hint","Tempo changes shift note playback. Notes keep their beat positions."}
                ul {class:"sections-list timing-list",
                    for row in rows {
                        {let edit=row.clone();let remove=row.clone();let label=row.label();let kind=match row {Selected::Tempo(_)=>"Tempo",Selected::Meter(_)=>"Time signature"};let tick=row.tick();
                        rsx! {li {
                            button {class:"section-go",aria_label:"Edit {kind} {label} at tick {tick}",onclick:move |_|reset(draft(),Some(edit.clone())),span {"{label}"}time {"{kind} · {clock(timing.seconds(tick))}"}}
                            button {title:"Delete marker (Undo restores it)",aria_label:"Delete {kind} at tick {tick}",disabled:tick==0,onclick:move |_|{studio.dispatch(Action::EditSync {edit:remove.remove()});error.set((studio.error)());if error().is_none()&&selected().as_ref()==Some(&remove){reset(draft(),None);}},Icon {name:"trash",size:14}}
                        }}}
                    }
                }
                for form_key in [epoch()] {
                    form {key:"{form_key}",class:"section-form timing-form",onsubmit:move |event|{event.prevent_default();save(());},
                        label {"Type" select {aria_label:"Timing marker type",value:if draft().tempo {"tempo"}else{"meter"},disabled:selected().is_some(),onchange:move |event|{let mut new=draft();new.tempo=event.value()=="tempo";reset(new,None);},option {value:"tempo","Tempo"}option {value:"meter","Time signature"}}}
                        label {"Start tick" input {aria_label:"Timing start tick",r#type:"number",min:"0",step:"1",initial_value:defaults().tick,oninput:move |e|{draft.write().tick=e.value();}}}
                        button {r#type:"button",onclick:move |_|{if let Some(tick)=super::note_canvas::snap(studio,(studio.playhead)()){let mut new=draft();new.tick=tick.to_string();reset(new,selected());}},"At playhead"}
                        if draft().tempo {
                            label {"BPM" input {aria_label:"Tempo BPM",r#type:"number",min:"1",max:"1000",step:"0.001",initial_value:defaults().bpm,oninput:move |e|{draft.write().bpm=e.value();}}}
                        } else {
                            label {"Beats per bar" input {aria_label:"Time signature numerator",r#type:"number",min:"1",max:"255",step:"1",initial_value:defaults().numerator,oninput:move |e|{draft.write().numerator=e.value();}}}
                            label {"Beat value" select {aria_label:"Time signature denominator",value:draft().denominator,onchange:move |e|{draft.write().denominator=e.value();},for value in [1,2,4,8,16,32,64,128] {option {value:"{value}",selected:draft().denominator==value.to_string(),"{value}"}}}}
                        }
                        div {class:"section-form-actions",
                            if selected().is_some() {button {r#type:"button",onclick:move |_|{if let Some(sync)=current(studio){reset(Draft::at(studio,&sync),None);}},"New marker"}}
                            button {r#type:"submit",if selected().is_some(){"Save"}else{"Add"}}
                        }
                    }
                }
                if archive {p {class:"field-hint","Tempo changed. Earlier vocal checks are kept in history; review this part again."}}
                if let Some(error)=error(){p {role:"alert",class:"section-error","{error}"}}
            }
        }
    }
}
