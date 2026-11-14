//! Shared local percussion controls and recording-clock authoring preview.
use super::state::Studio;
use crate::{
    editor::{Action, TimeRange},
    timing::TempoMap,
    vocal_percussion::{Edit, Hit, Part, PercussionChart, Phrase},
    vocals::VocalPhrase,
};
use dioxus::prelude::*;
use std::{path::PathBuf, sync::Arc};
const PAGE: usize = 64;
fn label(part: Part) -> &'static str {
    match part {
        Part::Lead => "Lead vocals",
        Part::Harm1 => "Harmonies",
    }
}
fn witness_index<T: PartialEq>(items: &[T], selection: Option<&(usize, T)>) -> Option<usize> {
    let (index, expected) = selection?;
    (items.get(*index) == Some(expected)).then_some(*index)
}
#[derive(Clone, PartialEq)]
struct Binding {
    root: Option<PathBuf>,
    audio: String,
    chart: String,
}
fn current(studio: Studio) -> Result<Option<PercussionChart>, String> {
    let doc = studio.snapshot().ok_or("No project is open")?;
    studio
        .store
        .read()
        .as_ref()
        .ok_or("No project is open")?
        .vocal_percussion(&doc)
        .map_err(|error| format!("{error:#}"))
}
fn commit(mut studio: Studio, binding: &Binding, action: Action) -> bool {
    let Some(doc) = studio.snapshot() else {
        return false;
    };
    if studio.store.read().as_ref().map(|store| store.root.clone()) != binding.root
        || doc.audio_sha256 != binding.audio
        || doc.source_chart_sha256 != binding.chart
    {
        studio.error.set(Some(
            "The project changed. Select a current percussion hit or phrase.".into(),
        ));
        return false;
    }
    studio.try_dispatch(action)
}
#[derive(Clone)]
struct Draft {
    start: String,
    end: String,
    displayed: bool,
    phrase_start: String,
    phrase_end: String,
    player: String,
}
impl Draft {
    fn new(hit: Option<&Hit>, range: Option<&Phrase>, tick: u64, resolution: u32) -> Self {
        Self {
            start: hit.map_or(tick, |hit| hit.start).to_string(),
            end: hit.map_or(tick, |hit| hit.end).to_string(),
            displayed: hit.is_none_or(|hit| hit.displayed),
            phrase_start: range.map_or(tick, |phrase| phrase.range.start).to_string(),
            phrase_end: range
                .map_or(tick.saturating_add(u64::from(resolution)), |phrase| {
                    phrase.range.end
                })
                .to_string(),
            player: range.map_or(1, |phrase| phrase.range.player).to_string(),
        }
    }
    fn hit(&self, before: Option<&Hit>) -> Result<Hit, String> {
        let start = self
            .start
            .parse::<u64>()
            .map_err(|_| "Hit start must be a whole tick.")?;
        let end = self
            .end
            .parse::<u64>()
            .map_err(|_| "Hit end must be a whole tick.")?;
        if end < start {
            return Err("Hit end must be at or after its start.".into());
        }
        let mut midi = before.and_then(|hit| hit.midi.clone());
        if let Some(attributes) = &mut midi {
            attributes.zero_length = start == end;
        }
        Ok(Hit {
            start,
            end,
            displayed: self.displayed,
            midi,
        })
    }
    fn phrase(&self, before: Option<&Phrase>) -> Result<Phrase, String> {
        Ok(Phrase {
            range: VocalPhrase {
                midi: None,
                start: self
                    .phrase_start
                    .parse()
                    .map_err(|_| "Phrase start must be a whole tick.")?,
                end: self
                    .phrase_end
                    .parse()
                    .map_err(|_| "Phrase end must be a whole tick.")?,
                player: self
                    .player
                    .parse()
                    .map_err(|_| "Phrase player must be 1 or 2.")?,
            },
            midi: before.and_then(|phrase| phrase.midi.clone()),
        })
    }
}
#[derive(Debug, PartialEq)]
struct Dot {
    index: usize,
    x: f64,
    end: f64,
    displayed: bool,
    active: bool,
}
fn visible(hits: &[Hit], timing: &TempoMap, position: f64, ahead: f64) -> Vec<Dot> {
    let left = position - 1.;
    let right = position + ahead;
    let x = |seconds: f64| 92. + (seconds.max(left).min(right) - left) / (right - left) * 890.;
    let first = hits.partition_point(|hit| timing.seconds(hit.end) < left);
    hits.iter()
        .enumerate()
        .skip(first)
        .take_while(|(_, hit)| timing.seconds(hit.start) <= right)
        .map(|(index, hit)| {
            let start = timing.seconds(hit.start);
            let end = timing.seconds(hit.end);
            Dot {
                index,
                x: x(start),
                end: x(end),
                displayed: hit.displayed,
                active: start <= position && position < end.max(start + 0.1),
            }
        })
        .collect()
}

#[component]
pub fn PercussionEditor(mut studio: Studio) -> Element {
    let mut part = use_signal(|| Part::Lead);
    let mut selected = use_signal(|| None::<(usize, Hit)>);
    let mut page = use_signal(|| 0usize);
    let mut epoch = use_signal(|| 0u64);
    let view = use_memo(move || current(studio).map(|chart| chart.map(Arc::new)));
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let binding = Binding {
        root: studio.store.read().as_ref().map(|store| store.root.clone()),
        audio: doc.audio_sha256.clone(),
        chart: doc.source_chart_sha256.clone(),
    };
    let (chart, warning) = match view() {
        Ok(chart) => (chart, None),
        Err(error) => (None, Some(error)),
    };
    let track = chart.as_ref().and_then(|chart| chart.tracks.get(&part()));
    let hits = track.map_or(&[][..], |track| track.hits.as_slice());
    let pages = hits.len().div_ceil(PAGE).max(1);
    let current_page = page().min(pages - 1);
    let original = selected();
    let selected_index = witness_index(hits, original.as_ref());
    let key = format!("{:?}:{original:?}:{}", part(), epoch());
    rsx! {details {class:"review-evidence percussion-editor",onkeydown:move|event|event.stop_propagation(),
        summary {"Vocal percussion"}
        p {class:"field-hint","Add displayed or hidden percussion hits in dedicated scoring phrases, separate from singing. Hidden hits remain visible here for editing."}
        if let Some(warning)=warning {p {class:"vocal-edit-error",role:"alert","{warning}"}p {class:"field-hint","Original percussion is preserved read-only. Resolve the import warning before editing it."}}
        else {
            PercussionPreview {studio,chart:chart.clone(),part:part(),selected:selected_index}
            div {class:"field-row",
                for target in [Part::Lead,Part::Harm1] {button {class:if part()==target{"studio-tool active"}else{"studio-tool"},aria_pressed:part()==target,onclick:move |_|{part.set(target);selected.set(None);page.set(0);},"{label(target)}"}}
            }
            div {class:"field-row",
                button {class:"studio-tool",onclick:move |_|{selected.set(None);epoch.set(epoch()+1);},"New hit at cursor"}
                button {class:"studio-tool",disabled:doc.undo.is_empty(),onclick:{let binding=binding.clone();move |_|{if commit(studio,&binding,Action::Undo){selected.set(None);}}},"Undo"}
                button {class:"studio-tool",disabled:doc.redo.is_empty(),onclick:{let binding=binding.clone();move |_|{if commit(studio,&binding,Action::Redo){selected.set(None);}}},"Redo"}
            }
            div {class:"vocal-syllables",role:"group",aria_label:"Percussion hits",
                for (index,hit) in hits.iter().enumerate().skip(current_page*PAGE).take(PAGE) {
                    {let witness=hit.clone();rsx!{button {class:if selected_index==Some(index){"studio-tool active"}else{"studio-tool"},aria_pressed:selected_index==Some(index),aria_label:"Select {label(part())} percussion hit at tick {hit.start}",onclick:move |_|selected.set(Some((index,witness.clone()))),"{hit.start}–{hit.end} · ",if hit.displayed{"Displayed"}else{"Hidden"}}}}
                }
            }
            if pages>1 {div {class:"field-row",button {class:"studio-tool",disabled:current_page==0,onclick:move |_|page.set(current_page-1),"Previous hits"}span {"Page {current_page+1} / {pages}"}button {class:"studio-tool",disabled:current_page+1>=pages,onclick:move |_|page.set(current_page+1),"Next hits"}}}
            if original.is_some() && selected_index.is_none(){p {class:"vocal-edit-error",role:"status","The selected hit changed. Your form is kept; select a current hit before applying."}}
            {rsx!{PercussionHitForm {key:"{key}",studio,binding:binding.clone(),part:part(),original,resolution:timing.resolution,selected,page}}}
            if let Some(track)=track {{rsx!{PercussionPhrases {key:"{part():?}",studio,binding:binding.clone(),part:part(),phrases:track.phrases.clone()}}}}
        }
    }}
}
#[component]
fn PercussionPreview(
    studio: Studio,
    chart: Option<Arc<PercussionChart>>,
    part: Part,
    selected: Option<usize>,
) -> Element {
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let position = (studio.playhead)();
    let ahead = 4.;
    let cursor = 92. + 890. / (ahead + 1.);
    rsx! {svg {view_box:"0 0 1000 104",width:"100%",height:"104",role:"img","aria-label":"Displayed and hidden vocal percussion on the recording clock","data-preview-position":"{position}",
        rect {x:"0",y:"0",width:"1000",height:"104",rx:"8",fill:"#121d29"}
        line {x1:"{cursor}",x2:"{cursor}",y1:"18",y2:"99",stroke:"#d6e1ee",stroke_width:"1"}
        text {x:"{cursor}",y:"13",text_anchor:"middle",fill:"#d6e1ee",font_size:"11","Now"}
        for (owner,y,title) in [(Part::Lead,39.,"Lead"),(Part::Harm1,79.,"Harmony")] {
            text {x:"8",y:"{y+4.}",fill:"#c2d2e2",font_size:"12","{title}"}
            line {x1:"92",x2:"982",y1:"{y}",y2:"{y}",stroke:"#304052"}
            if let Some(track)=chart.as_ref().and_then(|chart|chart.tracks.get(&owner)) {
                for dot in visible(&track.hits,&timing,position,ahead) {
                    {let picked=owner==part&&selected==Some(dot.index);rsx!{g {
                        title {"{title} · hit {dot.index} · ",if dot.displayed{"Displayed"}else{"Hidden"}}
                        if dot.end>dot.x {line {x1:"{dot.x}",x2:"{dot.end}",y1:"{y}",y2:"{y}",stroke:"#c9a65f",stroke_width:"2"}}
                        circle {cx:"{dot.x}",cy:"{y}",r:if picked||dot.active{"7"}else{"5"},fill:if dot.displayed{"#d8b66d"}else{"#121d29"},stroke:if picked{"#ffffff"}else{"#d8b66d"},stroke_width:if picked{"3"}else{"2"},stroke_dasharray:if dot.displayed{"none"}else{"2 2"}}
                    }}}
                }
            }
        }
    }}
}
#[component]
fn PhraseFields(mut draft: Signal<Draft>) -> Element {
    let value = draft.read().clone();
    rsx! {div {class:"field-row",
        label {class:"field-stack field-half",span {class:"field-label","Phrase start tick"}input {class:"field-control",r#type:"number",min:"0",aria_label:"Percussion phrase start tick",value:value.phrase_start,oninput:move|e|draft.write().phrase_start=e.value()}}
        label {class:"field-stack field-half",span {class:"field-label","Phrase end tick"}input {class:"field-control",r#type:"number",min:"1",aria_label:"Percussion phrase end tick",value:value.phrase_end,oninput:move|e|draft.write().phrase_end=e.value()}}
        label {class:"field-stack",span {class:"field-label","Player"}select {class:"field-control",aria_label:"Percussion phrase player",value:value.player,onchange:move|e|draft.write().player=e.value(),option {value:"1","1"}option {value:"2","2"}}}
    }}
}
#[component]
fn PercussionHitForm(
    mut studio: Studio,
    binding: Binding,
    part: Part,
    original: Option<(usize, Hit)>,
    resolution: u32,
    mut selected: Signal<Option<(usize, Hit)>>,
    mut page: Signal<usize>,
) -> Element {
    let tick = (studio.timing)()
        .and_then(|timing| timing.tick(*studio.playhead.peek()))
        .unwrap_or(0);
    let chart = current(studio).ok().flatten();
    let at = original.as_ref().map_or(tick, |(_, hit)| hit.start);
    let range = chart
        .as_ref()
        .and_then(|chart| chart.tracks.get(&part))
        .and_then(|track| {
            track
                .phrases
                .iter()
                .find(|phrase| phrase.range.start <= at && at < phrase.range.end)
        });
    let mut draft = use_signal(|| {
        Draft::new(
            original.as_ref().map(|(_, hit)| hit),
            range,
            tick,
            resolution,
        )
    });
    let mut add_range = use_signal(|| range.is_none());
    let mut error = use_signal(|| None::<String>);
    let value = draft.read().clone();
    let editing = original.is_some();
    rsx! {div {class:"vocal-note-editor",
        div {class:"form-heading",if editing{"Edit percussion hit"}else{"Add percussion hit"}}
        div {class:"field-row",
            label {class:"field-stack field-half",span {class:"field-label","Start tick"}input {class:"field-control",r#type:"number",min:"0",aria_label:"Percussion hit start tick",value:value.start,oninput:move|e|draft.write().start=e.value()}}
            label {class:"field-stack field-half",span {class:"field-label","End tick"}input {class:"field-control",r#type:"number",min:"0",aria_label:"Percussion hit end tick",value:value.end,oninput:move|e|draft.write().end=e.value()}}
        }
        p {class:"field-hint","Use the same start and end tick for a tap. Imported hit lengths and MIDI attributes are retained when editing."}
        label {class:"form-check",input {r#type:"checkbox",checked:value.displayed,onchange:move|e|draft.write().displayed=e.checked()}"Displayed in game"}
        label {class:"form-check",input {r#type:"checkbox",checked:add_range(),onchange:move|e|add_range.set(e.checked())}"Include a new dedicated scoring phrase"}
        if add_range(){PhraseFields {draft}}
        if let Some(message)=error(){p {class:"vocal-edit-error",role:"alert","{message}"}}
        button {class:"field-button",onclick:{let binding=binding.clone();let before=original.clone();move |_|{
            let parsed=(||->Result<_,String>{Ok((draft.read().hit(before.as_ref().map(|(_,hit)|hit))?,if add_range(){Some(draft.read().phrase(None)?)}else{None}))})();
            match parsed {Err(message)=>error.set(Some(message)),Ok((hit,phrase))=>{
                let edit=Edit::PutHit {part,index:before.as_ref().map(|(index,_)|*index),expected:before.as_ref().map(|(_,hit)|hit.clone()),hit:hit.clone(),phrase};
                if commit(studio,&binding,Action::EditVocalPercussion {edit}) {
                    let index=current(studio).ok().flatten().and_then(|chart|chart.tracks.get(&part).and_then(|track|track.hits.iter().position(|item|item==&hit)));
                    selected.set(index.map(|index|(index,hit.clone())));page.set(index.unwrap_or(0)/PAGE);error.set(None);
                }else{error.set(studio.error.read().clone());}
            }}
        }},if editing{"Apply percussion hit"}else{"Add percussion hit"}}
        if let Some((index,expected))=original.clone() {div {class:"field-row",
            button {class:"studio-tool",onclick:{let binding=binding.clone();let expected=expected.clone();move |_|{
                if commit(studio,&binding,Action::EditVocalPercussion {edit:Edit::RemoveHit {part,index,expected:expected.clone()}}){selected.set(None);error.set(None);}else{error.set(studio.error.read().clone());}
            }},"Remove percussion hit"}
            button {class:"studio-tool",disabled:!studio.snapshot().is_some_and(|doc|doc.has_recording()),onclick:{let binding=binding.clone();move |_|{
                let result=(||->Result<_,String>{let chart=current(studio)?.ok_or("No percussion hits")?;let track=chart.tracks.get(&part).ok_or("Percussion part changed")?;
                    if track.hits.get(index)!=Some(&expected){return Err("Selected percussion hit changed. Select it again.".into());}
                    let phrase=track.phrases.iter().find(|phrase|phrase.covers(&expected)).ok_or("Scoring phrase changed")?;
                    let doc=studio.snapshot().ok_or("No project")?;let timing=(studio.timing)().ok_or("No timing")?;
                    let start=timing.seconds(phrase.range.start).clamp(0.,doc.duration);let end=timing.seconds(phrase.range.end).clamp(0.,doc.duration);
                    if end<=start{return Err("This phrase is outside the recording.".into());}Ok(TimeRange {start,end})})();
                match result{Err(message)=>error.set(Some(message)),Ok(range)=>{if commit(studio,&binding,Action::SetLoop {range:Some(range)}){studio.seek(range.start);studio.dispatch(Action::Playback {playing:true});}}}
            }},"Loop recording phrase"}
        }}
    }}
}
#[component]
fn PercussionPhrases(
    studio: Studio,
    binding: Binding,
    part: Part,
    phrases: Vec<Phrase>,
) -> Element {
    let mut selected = use_signal(|| phrases.first().cloned().map(|phrase| (0usize, phrase)));
    let mut page = use_signal(|| 0usize);
    let pages = phrases.len().div_ceil(PAGE).max(1);
    let current_page = page().min(pages - 1);
    let chosen = selected();
    let selected_index = witness_index(&phrases, chosen.as_ref());
    rsx! {details {class:"review-evidence",summary {"Percussion scoring phrases"}
        div {class:"vocal-syllables",for (index,phrase) in phrases.iter().enumerate().skip(current_page*PAGE).take(PAGE){{let witness=phrase.clone();rsx!{button {class:if selected_index==Some(index){"studio-tool active"}else{"studio-tool"},onclick:move |_|selected.set(Some((index,witness.clone()))),"{phrase.range.start}–{phrase.range.end} · player {phrase.range.player}"}}}}}
        if pages>1{div {class:"field-row",button {class:"studio-tool",disabled:current_page==0,onclick:move |_|page.set(current_page-1),"Previous phrases"}span {"Page {current_page+1} / {pages}"}button {class:"studio-tool",disabled:current_page+1>=pages,onclick:move |_|page.set(current_page+1),"Next phrases"}}}
        if chosen.is_some() && selected_index.is_none(){p {class:"vocal-edit-error",role:"status","The selected phrase changed. Your form is kept; select a current phrase before applying."}}
        if let Some((index,phrase))=chosen.clone(){PercussionPhraseForm {key:"{part:?}:{index}:{phrase:?}",studio,binding,part,index,phrase,next:if selected_index.is_some(){phrases.get(index+1).cloned()}else{None},selected,page}}
    }}
}
fn refresh_phrase(
    studio: Studio,
    part: Part,
    index: usize,
    wanted: Option<&Phrase>,
    mut selected: Signal<Option<(usize, Phrase)>>,
    mut page: Signal<usize>,
) {
    let next = current(studio).ok().flatten().and_then(|chart| {
        chart.tracks.get(&part).and_then(|track| {
            let index = match wanted {
                Some(wanted) => track.phrases.iter().position(|phrase| phrase == wanted)?,
                None => index,
            };
            track
                .phrases
                .get(index)
                .cloned()
                .map(|phrase| (index, phrase))
        })
    });
    page.set(next.as_ref().map_or(0, |(index, _)| index / PAGE));
    selected.set(next);
}
#[component]
fn PercussionPhraseForm(
    studio: Studio,
    binding: Binding,
    part: Part,
    index: usize,
    phrase: Phrase,
    next: Option<Phrase>,
    selected: Signal<Option<(usize, Phrase)>>,
    page: Signal<usize>,
) -> Element {
    let draft = use_signal(|| Draft::new(None, Some(&phrase), phrase.range.start, 480));
    let mut split = use_signal(|| {
        (phrase.range.start + (phrase.range.end - phrase.range.start) / 2).to_string()
    });
    let mut error = use_signal(|| None::<String>);
    rsx! {PhraseFields {draft}
        if let Some(message)=error(){p {class:"vocal-edit-error",role:"alert","{message}"}}
        button {class:"studio-tool",onclick:{let binding=binding.clone();let expected=phrase.clone();move |_|{
            match draft.read().phrase(Some(&expected)){Err(message)=>error.set(Some(message)),Ok(phrase)=>{if commit(studio,&binding,Action::EditVocalPercussion {edit:Edit::PutPhrase {part,index,expected:expected.clone(),phrase:phrase.clone()}}){refresh_phrase(studio,part,index,Some(&phrase),selected,page);error.set(None);}else{error.set(studio.error.read().clone());}}}
        }},"Apply percussion phrase"}
        p {class:"field-hint","Split between hits, or merge matching-player phrases. Empty phrases are removed with their final hit."}
        div {class:"field-row",
            label {class:"field-stack",span {class:"field-label","Split tick"}input {class:"field-control",r#type:"number",min:"0",aria_label:"Percussion phrase split tick",value:split(),oninput:move|e|split.set(e.value())}}
            button {class:"studio-tool",onclick:{let binding=binding.clone();let expected=phrase.clone();move |_|{
                let Ok(tick)=split().parse::<u64>()else{error.set(Some("Split tick must be a whole tick.".into()));return};
                if commit(studio,&binding,Action::EditVocalPercussion {edit:Edit::SplitPhrase {part,index,expected:expected.clone(),tick}}){refresh_phrase(studio,part,index,None,selected,page);error.set(None);}else{error.set(studio.error.read().clone());}
            }},"Split percussion phrase"}
            button {class:"studio-tool",disabled:next.is_none(),onclick:move |_|{let Some(next_expected)=next.clone()else{return};
                if commit(studio,&binding,Action::EditVocalPercussion {edit:Edit::MergePhrase {part,index,expected:phrase.clone(),next_expected}}){refresh_phrase(studio,part,index,None,selected,page);error.set(None);}else{error.set(studio.error.read().clone());}
            },"Merge next percussion phrase"}
        }
    }
}

#[cfg(test)]
#[path = "percussion_editor_tests.rs"]
mod tests;
