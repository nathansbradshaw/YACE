//! Shared solo inspector and timeline ranges; all edits use project commands.
use super::state::Studio;
use crate::{
    editor::{Action, TimeRange},
    solos::{Edit, Phrase},
};
use dioxus::prelude::*;
use std::path::PathBuf;
const PAGE: usize = 64;
#[derive(Clone, PartialEq)]
struct Binding {
    root: Option<PathBuf>,
    audio: String,
    chart: String,
}
fn binding(studio: Studio) -> Option<Binding> {
    let doc = studio.snapshot()?;
    Some(Binding {
        root: studio.store.read().as_ref().map(|s| s.root.clone()),
        audio: doc.audio_sha256.clone(),
        chart: doc.source_chart_sha256.clone(),
    })
}
fn commit(mut studio: Studio, expected: &Option<Binding>, track: &str, action: Action) -> bool {
    if binding(studio) != *expected
        || studio
            .snapshot()
            .is_none_or(|doc| doc.selected_track != track)
    {
        studio.error.set(Some(
            "The project or track changed. Select the current solo again.".into(),
        ));
        return false;
    }
    studio.try_dispatch(action)
}
pub(super) fn current(studio: Studio, track: &str) -> Option<Vec<Phrase>> {
    let doc = studio.doc.read();
    let doc = doc.as_ref()?;
    if let Some(phrases) = doc.content.solos.get(track) {
        return Some(phrases.clone());
    }
    studio
        .source_solos
        .read()
        .get(track)
        .cloned()
        .unwrap_or_else(|| Some(Vec::new()))
}
fn step(studio: Studio) -> u64 {
    let division = (studio.snap)();
    (studio.timing)().map_or(1, |timing| {
        if division == 0 {
            1
        } else {
            (u64::from(timing.resolution) / u64::from(division)).max(1)
        }
    })
}
fn at_cursor(studio: Studio) -> Phrase {
    let Some(timing) = (studio.timing)() else {
        return Phrase { start: 0, end: 1 };
    };
    let last = studio
        .snapshot()
        .and_then(|doc| timing.tick(doc.duration))
        .unwrap_or(1)
        .max(1);
    let start = timing
        .tick(*studio.playhead.peek())
        .unwrap_or(0)
        .min(last - 1);
    Phrase {
        start,
        end: start
            .saturating_add(u64::from(timing.resolution) * 4)
            .min(last),
    }
}
#[derive(Clone)]
struct Draft {
    start: String,
    end: String,
}
impl From<Phrase> for Draft {
    fn from(value: Phrase) -> Self {
        Self {
            start: value.start.to_string(),
            end: value.end.to_string(),
        }
    }
}
impl Draft {
    fn parse(&self) -> Result<Phrase, String> {
        Ok(Phrase {
            start: self
                .start
                .parse()
                .map_err(|_| "Solo start must be a whole tick.")?,
            end: self
                .end
                .parse()
                .map_err(|_| "Solo end must be a whole tick.")?,
        })
    }
}
#[component]
pub fn Controls(mut studio: Studio, track: String) -> Element {
    let mut previous = use_signal(|| None::<Phrase>);
    let mut draft = use_signal(|| Draft::from(at_cursor(studio)));
    let mut page = use_signal(|| 0usize);
    let mut error = use_signal(|| None::<String>);
    let bound = use_hook(|| binding(studio));
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let Some(phrases) = current(studio, &track) else {
        return rsx! {details {class:"context-fold",summary {"Solos"}p {class:"field-hint","This source has unsupported solo boundaries. Its original events remain read-only; note editing is still available."}}};
    };
    let pages = phrases.len().div_ceil(PAGE).max(1);
    let at = page().min(pages - 1);
    let value = draft.read().clone();
    let chosen = previous();
    let stale = chosen
        .as_ref()
        .is_some_and(|phrase| !phrases.contains(phrase));
    let has_selection = studio.selection().track == track && !studio.selection().ids.is_empty();
    rsx! {details {class:"context-fold solo-controls",onkeydown:move|event|event.stop_propagation(),
        summary {"Solos · {phrases.len()}"}
        p {class:"field-hint","Create a solo from selected notes, the current loop or tick bounds. Changing its start and end moves or resizes it. Undo and Redo retain each edit."}
        div {class:"vocal-syllables",role:"group",aria_label:"Solo ranges",
            for phrase in phrases.iter().skip(at*PAGE).take(PAGE) {{let target=phrase.clone();rsx!{button {class:if chosen.as_ref()==Some(phrase){"studio-tool active"}else{"studio-tool"},aria_pressed:chosen.as_ref()==Some(phrase),onclick:move |_|{previous.set(Some(target.clone()));draft.set(Draft::from(target.clone()));error.set(None);},"{phrase.start}–{phrase.end} ticks"}}}}
        }
        if pages>1 {div {class:"field-row",button {class:"studio-tool",disabled:at==0,onclick:move |_|page.set(at-1),"Previous solos"}span {"Page {at+1} / {pages}"}button {class:"studio-tool",disabled:at+1>=pages,onclick:move |_|page.set(at+1),"Next solos"}}}
        if stale {p {role:"status",class:"field-hint","This solo changed. Your draft is kept; select a current solo before applying it."}}
        div {class:"field-row",
            button {class:"studio-tool",onclick:move |_|{previous.set(None);draft.set(Draft::from(at_cursor(studio)));error.set(None);},"New solo at playhead"}
            button {class:"studio-tool",disabled:!has_selection,onclick:{let track=track.clone();move |_|{let Some(doc)=studio.snapshot() else{return};let selection=studio.selection();if selection.track!=track {return}match crate::solos::selected(&doc,&selection,step(studio)) {Ok(range)=>{draft.set(Draft::from(range));error.set(None);},Err(message)=>error.set(Some(message.to_string()))}}},"Use selected notes"}
            button {class:"studio-tool",disabled:doc.transport.loop_range.is_none(),onclick:move |_|{let Some(doc)=studio.snapshot() else{return};let Some(range)=doc.transport.loop_range else{return};let Some(timing)=(studio.timing)() else{return};if let (Some(start),Some(end))=(timing.tick(range.start),timing.tick(range.end)){draft.set(Draft::from(Phrase{start,end}));error.set(None);}},"Use current loop"}
        }
        div {class:"field-row",
            label {class:"field-stack field-half",span {class:"field-label","Start tick"}input {class:"field-control",r#type:"number",min:"0",value:value.start,aria_label:"Solo start tick",oninput:move|event|draft.write().start=event.value()}}
            label {class:"field-stack field-half",span {class:"field-label","End tick"}input {class:"field-control",r#type:"number",min:"1",value:value.end,aria_label:"Solo end tick",oninput:move|event|draft.write().end=event.value()}}
        }
        if let Some(message)=error(){p {class:"vocal-edit-error",role:"alert","{message}"}}
        button {class:"field-button",onclick:{let bound=bound.clone();let track=track.clone();move |_|{
            match draft.read().parse() {Err(message)=>error.set(Some(message)),Ok(phrase)=>{
                if commit(studio,&bound,&track,Action::EditSolo {edit:Edit::Put {track:track.clone(),previous:previous(),phrase:phrase.clone()}}){previous.set(Some(phrase));error.set(None);}else{error.set((studio.error)());}
            }}
        }},if chosen.is_some(){"Apply solo"}else{"Add solo"}}
        if let Some(phrase)=chosen {div {class:"field-row",
            button {class:"studio-tool",onclick:{let bound=bound.clone();let track=track.clone();let expected=phrase.clone();move |_|{if commit(studio,&bound,&track,Action::EditSolo {edit:Edit::Remove {track:track.clone(),phrase:expected.clone()}}){previous.set(None);error.set(None);}else{error.set((studio.error)());}}},"Remove solo"}
            button {class:"studio-tool",disabled:!doc.has_recording(),onclick:{let bound=bound.clone();let track=track.clone();move |_|{
                let Some(doc)=studio.snapshot() else{return};let Some(timing)=(studio.timing)() else{return};
                if !current(studio,&track).is_some_and(|ranges|ranges.contains(&phrase)){error.set(Some("Selected solo changed. Select it again.".into()));return;}
                let range=TimeRange {start:timing.seconds(phrase.start).clamp(0.,doc.duration),end:timing.seconds(phrase.end).clamp(0.,doc.duration)};
                if range.end<=range.start {error.set(Some("This solo is outside the recording.".into()));return;}
                if commit(studio,&bound,&track,Action::SetLoop {range:Some(range)}){studio.seek(range.start);studio.dispatch(Action::Playback {playing:true});}
            }},"Loop recording solo"}
        }}
        div {class:"field-row",
            button {class:"studio-tool",disabled:doc.undo.is_empty(),onclick:{let bound=bound.clone();let track=track.clone();move |_|{if !commit(studio,&bound,&track,Action::Undo){error.set((studio.error)());}}},"Undo"}
            button {class:"studio-tool",disabled:doc.redo.is_empty(),onclick:move |_|{if !commit(studio,&bound,&track,Action::Redo){error.set((studio.error)());}},"Redo"}
        }
    }}
}
#[component]
pub fn Lane(studio: Studio, track: String) -> Element {
    let phrases = current(studio, &track).unwrap_or_default();
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let start = (studio.view_start)();
    let span = (studio.span)();
    if phrases.is_empty() || !span.is_finite() || span <= 0. {
        return rsx! {};
    };
    let first = phrases.partition_point(|phrase| timing.seconds(phrase.end) <= start);
    rsx! {div {class:"power-lane",span {class:"power-lane-label","Solos"}
        svg {class:"power-lane-plot",view_box:"0 0 1000 16",preserve_aspect_ratio:"none",role:"img","aria-label":"Solo ranges on the song timeline",
            for phrase in phrases.iter().skip(first).take_while(|phrase|timing.seconds(phrase.start)<start+span) {
                {let a=((timing.seconds(phrase.start)-start)/span*1000.).max(0.);let b=((timing.seconds(phrase.end)-start)/span*1000.).min(1000.);rsx!{if b>a {rect {x:"{a}",y:"2",width:"{b-a}",height:"12",rx:"2",fill:"#315f94",stroke:"#87b9f5","data-solo-start":"{phrase.start}","data-solo-end":"{phrase.end}",title {"Solo · {phrase.start}–{phrase.end} ticks"}}}}}
            }
        }
    }}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn solo_numeric_draft_keeps_invalid_input_until_explicit_apply() {
        let draft = Draft {
            start: "120.5".into(),
            end: "480".into(),
        };
        assert!(draft.parse().is_err());
        assert_eq!(draft.start, "120.5");
        assert_eq!(
            Draft::from(Phrase {
                start: 120,
                end: 480
            })
            .parse()
            .unwrap(),
            Phrase {
                start: 120,
                end: 480
            }
        );
    }
}
