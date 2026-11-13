//! Shared native/WASM controls for independent singers and shared phrase owners.
use super::{
    state::Studio,
    vocal_editor::{NoteDraft, syllable},
};
use crate::{
    editor::{Action, TimeRange},
    song_files::game_midi::{self, RetainedHarmonyPhrases},
    vocal_harmonies::{Edit, Part, PhraseKind},
    vocals::{Syllable, VocalNote, VocalPhrase, VocalTone},
};
use dioxus::prelude::*;
use std::path::PathBuf;
const PAGE: usize = 64;
#[cfg(test)]
#[path = "harmony_editor_tests.rs"]
mod tests;

#[derive(Clone, PartialEq)]
struct Binding {
    root: Option<PathBuf>,
    audio: String,
    chart: String,
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
            "The project changed. Select a current harmony note or phrase.".into(),
        ));
        return false;
    }
    studio.try_dispatch(action)
}
#[derive(Clone)]
struct PhraseDraft {
    start: String,
    end: String,
    player: String,
}
impl PhraseDraft {
    fn from(phrase: &VocalPhrase) -> Self {
        Self {
            start: phrase.start.to_string(),
            end: phrase.end.to_string(),
            player: phrase.player.to_string(),
        }
    }
    fn parse(&self) -> Result<VocalPhrase, String> {
        Ok(VocalPhrase {
            midi: None,
            start: self
                .start
                .parse()
                .map_err(|_| "Phrase start must be a whole tick.".to_owned())?,
            end: self
                .end
                .parse()
                .map_err(|_| "Phrase end must be a whole tick.".to_owned())?,
            player: self
                .player
                .parse()
                .map_err(|_| "Phrase player must be 1 or 2.".to_owned())?,
        })
    }
}
fn initial(
    part: Part,
    resolution: u32,
    notes: &[VocalNote],
    ranges: &RetainedHarmonyPhrases,
) -> (NoteDraft, PhraseDraft, PhraseDraft) {
    let hint = notes.last().map(|note| note.end).unwrap_or_else(|| {
        if part != Part::Harm1
            && let Some(lyric) = ranges.lyrics.first()
            && let Some(score) = ranges
                .scoring
                .iter()
                .find(|score| score.start < lyric.end && lyric.start < score.end)
        {
            return score.start.max(lyric.start);
        }
        ranges.scoring.first().map_or(0, |phrase| phrase.start)
    });
    let score = ranges
        .scoring
        .iter()
        .find(|p| p.start <= hint && hint < p.end)
        .cloned()
        .unwrap_or(VocalPhrase {
            midi: None,
            start: hint,
            end: hint.saturating_add(u64::from(resolution).max(1)),
            player: 1,
        });
    let lyric = ranges
        .lyrics
        .iter()
        .find(|p| p.start <= hint && hint < p.end)
        .cloned()
        .unwrap_or_else(|| score.clone());
    let mut end = hint
        .saturating_add((u64::from(resolution) / 2).max(1))
        .min(score.end);
    if part != Part::Harm1 {
        end = end.min(lyric.end);
    }
    let note = VocalNote {
        start: hint,
        end,
        tone: VocalTone::Pitched { midi: 60 },
        lyric: Syllable::Missing,
        midi: None,
    };
    (
        NoteDraft::from_note(&note),
        PhraseDraft::from(&score),
        PhraseDraft::from(&lyric),
    )
}
fn first_free_tick(
    mut tick: u64,
    resolution: u32,
    blocked: &[crate::vocal_percussion::Phrase],
) -> u64 {
    for phrase in blocked {
        if tick < phrase.range.end
            && phrase.range.start < tick.saturating_add(u64::from(resolution).max(1))
        {
            tick = phrase.range.end;
        }
    }
    tick
}

#[component]
pub fn HarmonyEditor(mut studio: Studio) -> Element {
    let mut voice = use_signal(|| Part::Harm1);
    let mut selected = use_signal(|| None::<usize>);
    let mut page = use_signal(|| 0usize);
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let binding = Binding {
        root: studio.store.read().as_ref().map(|s| s.root.clone()),
        audio: doc.audio_sha256.clone(),
        chart: doc.source_chart_sha256.clone(),
    };
    let current = voice();
    let label = current.track();
    let chart = doc.content.harmonies.as_ref();
    let notes = chart
        .and_then(|chart| chart.voices.get(&current))
        .map_or(&[][..], |voice| voice.notes.as_slice());
    let (ranges, readonly, warning) = if let Some(chart) = chart {
        (
            RetainedHarmonyPhrases {
                scoring: chart.phrases.clone(),
                lyrics: chart.lyric_phrases.clone(),
            },
            false,
            None,
        )
    } else if let Some(store) = studio.store.read().as_ref() {
        match game_midi::retained_harmony_phrases(&store.root.join("source")).and_then(|ranges| {
            let opaque = game_midi::extra_parts(&store.root.join("source"))?
                .iter()
                .any(|name| {
                    name.strip_prefix("MIDI: ")
                        .and_then(Part::from_midi)
                        .is_some()
                });
            let can_begin = opaque
                && game_midi::can_begin_harmonies(
                    &store.root.join("source"),
                    doc.content.vocal_percussion.as_ref(),
                )?;
            Ok((ranges.unwrap_or_default(), opaque && !can_begin))
        }) {
            Ok((ranges, opaque)) => (ranges, opaque, None),
            Err(error) => (Default::default(), true, Some(format!("{error:#}"))),
        }
    } else {
        (Default::default(), true, None)
    };
    let pages = notes.len().div_ceil(PAGE).max(1);
    let current_page = page().min(pages - 1);
    let note = selected().and_then(|index| notes.get(index).cloned().map(|note| (index, note)));
    let form_key = format!("{label}:{note:?}");
    rsx! {details {class:"review-evidence harmony-editor",open:chart.is_some(),onkeydown:move|event|event.stop_propagation(),
        summary {"Harmony parts"}
        p {class:"field-hint","Each singer has separate notes and lyrics. HARM1 scoring phrases cover all singers; HARM2 lyric ranges also cover HARM3."}
        div {class:"field-row",
            for target in Part::ALL {
                {let count=chart.and_then(|c|c.voices.get(&target)).map_or(0,|v|v.notes.len());rsx! {
                    button {class:if current==target {"studio-tool active"}else{"studio-tool"},aria_pressed:current==target,
                        onclick:move |_|{voice.set(target);selected.set(None);page.set(0);studio.vocal_note.set(None);},"{target.track()} · {count}"}
                }}
            }
        }
        div {class:"field-row",
            button {class:"studio-tool",disabled:doc.undo.is_empty(),onclick:{let binding=binding.clone();move |_|{if commit(studio,&binding,Action::Undo){selected.set(None);}}},"Undo"}
            button {class:"studio-tool",disabled:doc.redo.is_empty(),onclick:{let binding=binding.clone();move |_|{if commit(studio,&binding,Action::Redo){selected.set(None);}}},"Redo"}
            button {class:"studio-tool",disabled:readonly,onclick:move |_|{selected.set(None);studio.vocal_note.set(None);},"New {label} note"}
        }
        if let Some(warning)=warning {p {class:"vocal-edit-error",role:"alert","{warning}"}}
        if readonly {p {class:"field-hint","Original harmony data is preserved read-only. Authoring here requires an editable baseline; see the import warnings."}}
        else {
            div {class:"vocal-syllables",role:"group",aria_label:"{label} notes",
                for (index,note) in notes.iter().enumerate().skip(current_page*PAGE).take(PAGE) {
                    {let text=syllable(note);let pitch=match note.tone {VocalTone::Pitched{midi}=>midi.to_string(),VocalTone::Talkie{..}=>"Talkie".into()};rsx! {
                        button {class:if selected()==Some(index){"studio-tool active"}else{"studio-tool"},
                            aria_label:"Select {label} note {index} at tick {note.start}",
                            onclick:move |_|{selected.set(Some(index));studio.vocal_note.set(None);},
                            "{note.start}–{note.end} · {pitch} · {text}"}
                    }}
                }
            }
            if pages>1 {div {class:"field-row",
                button {class:"studio-tool",disabled:current_page==0,onclick:move |_|page.set(current_page-1),"Previous notes"}
                span {"Page {current_page+1} / {pages}"}
                button {class:"studio-tool",disabled:current_page+1>=pages,onclick:move |_|page.set(current_page+1),"Next notes"}
            }}
            {rsx!{HarmonyNoteForm {key:"{form_key}",studio,binding:binding.clone(),part:current,note,resolution:timing.resolution,ranges,selected,page}}}
            if chart.is_some(){HarmonyPhrases {studio,binding:binding.clone()}}
        }
    }}
}

#[component]
fn PhraseFields(mut draft: Signal<PhraseDraft>, label: &'static str) -> Element {
    let current = draft.read().clone();
    rsx! {div {class:"field-row",
        label {class:"field-stack field-half",span {class:"field-label","{label} start"}input {class:"field-control",r#type:"number",min:"0",aria_label:"{label} start tick",value:current.start,oninput:move|e|draft.write().start=e.value()}}
        label {class:"field-stack field-half",span {class:"field-label","{label} end"}input {class:"field-control",r#type:"number",min:"1",aria_label:"{label} end tick",value:current.end,oninput:move|e|draft.write().end=e.value()}}
        label {class:"field-stack",span {class:"field-label","Player"}select {class:"field-control",aria_label:"{label} player",value:current.player,onchange:move|e|draft.write().player=e.value(),option {value:"1","1"}option {value:"2","2"}}}
    }}
}
#[component]
fn HarmonyNoteForm(
    mut studio: Studio,
    binding: Binding,
    part: Part,
    note: Option<(usize, VocalNote)>,
    resolution: u32,
    ranges: RetainedHarmonyPhrases,
    mut selected: Signal<Option<usize>>,
    mut page: Signal<usize>,
) -> Element {
    let notes = studio
        .snapshot()
        .and_then(|d| {
            d.content
                .harmonies
                .as_ref()?
                .voices
                .get(&part)
                .map(|v| v.notes.clone())
        })
        .unwrap_or_default();
    let (new_draft, new_score, new_lyric) = use_hook(|| {
        let mut initial = initial(part, resolution, &notes, &ranges);
        if notes.is_empty()
            && ranges.scoring.is_empty()
            && let Some(doc) = studio.snapshot()
            && let Some(store) = studio.store.read().as_ref()
            && let Ok(Some(pc)) = store.vocal_percussion(&doc)
            && let Some(track) = pc.tracks.get(&crate::vocal_percussion::Part::Harm1)
            && let Some(timing) = (studio.timing)()
        {
            let tick = first_free_tick(
                timing.tick(*studio.playhead.peek()).unwrap_or(0),
                resolution,
                &track.phrases,
            );
            initial.0.start = tick.to_string();
            initial.0.end = tick
                .saturating_add((u64::from(resolution) / 2).max(1))
                .to_string();
            let phrase = VocalPhrase {
                midi: None,
                start: tick,
                end: tick.saturating_add(u64::from(resolution).max(1)),
                player: 1,
            };
            initial.1 = PhraseDraft::from(&phrase);
            initial.2 = PhraseDraft::from(&phrase);
        }
        initial
    });
    let mut draft = use_signal(|| {
        note.as_ref()
            .map_or(new_draft, |(_, note)| NoteDraft::from_note(note))
    });
    let score = use_signal(|| new_score);
    let lyric = use_signal(|| new_lyric);
    let mut add_ranges = use_signal(|| true);
    let mut error = use_signal(|| None::<String>);
    let fields = draft.read().clone();
    let pitched = fields.mode == "pitched";
    let label = part.track();
    let editing = note.is_some();
    rsx! {div {class:"vocal-note-editor",style:"margin-top:16px",
        div {class:"form-heading",if editing {"Edit {label} note"}else{"Add {label} note"}}
        div {class:"field-row",
            label {class:"field-stack field-half",span {class:"field-label","Start tick"}input {class:"field-control",r#type:"number",min:"0",aria_label:"{label} note start tick",value:fields.start,oninput:move|e|draft.write().start=e.value()}}
            label {class:"field-stack field-half",span {class:"field-label","End tick"}input {class:"field-control",r#type:"number",min:"1",aria_label:"{label} note end tick",value:fields.end,oninput:move|e|draft.write().end=e.value()}}
        }
        label {class:"field-stack",span {class:"field-label","Syllable"}input {class:"field-control",aria_label:"{label} syllable text",placeholder:"Missing lyric",value:fields.text,disabled:fields.continuation||fields.continue_word,oninput:move|e|draft.write().text=e.value()}}
        div {class:"field-row",
            label {class:"field-stack field-half",span {class:"field-label","Type"}select {class:"field-control",aria_label:"{label} note type",value:fields.mode,onchange:move|e|draft.write().mode=e.value(),option {value:"pitched","Pitched"}option {value:"talkie","Talkie"}option {value:"lenient","Lenient talkie"}}}
            label {class:"field-stack field-half",span {class:"field-label","MIDI pitch"}input {class:"field-control",r#type:"number",min:"36",max:"84",aria_label:"{label} MIDI pitch",value:fields.pitch,disabled:!pitched,oninput:move|e|draft.write().pitch=e.value()}}
        }
        div {class:"vocal-note-options",
            label {class:"form-check",input {r#type:"checkbox",checked:fields.continue_word,onchange:move|e|{let mut d=draft.write();d.continue_word=e.checked();if d.continue_word{d.continuation=false;}}}"Continue word"}
            label {class:"form-check",input {r#type:"checkbox",checked:fields.continuation,onchange:move|e|{let mut d=draft.write();d.continuation=e.checked();if d.continuation{d.continue_word=false;}}}"Pitch slide"}
            label {class:"form-check",input {r#type:"checkbox",checked:fields.join,disabled:fields.continuation||fields.continue_word,onchange:move|e|draft.write().join=e.checked()}"Join next"}
        }
        if !editing {
            label {class:"form-check",input {r#type:"checkbox",checked:add_ranges(),onchange:move|e|add_ranges.set(e.checked())}"Include these shared ranges"}
            if add_ranges(){PhraseFields {draft:score,label:"HARM1 scoring"}if part!=Part::Harm1 {PhraseFields {draft:lyric,label:"HARM2 lyrics"}}}
        }
        if let Some(message)=error(){p {class:"vocal-edit-error",role:"alert","{message}"}}
        button {class:"field-button",onclick:{let binding=binding.clone();let original=note.clone();move |_|{
            let result=(||->Result<_,String>{
                let mut updated=draft.read().parse()?;
                if let Some((_,expected))=&original {updated.midi=expected.midi.clone();}
                let scoring_phrase=if original.is_none()&&add_ranges(){Some(score.read().parse()?)}else{None};
                let lyric_phrase=if original.is_none()&&add_ranges()&&part!=Part::Harm1{Some(lyric.read().parse()?)}else{None};
                Ok((updated,scoring_phrase,lyric_phrase))
            })();
            match result {Err(message)=>error.set(Some(message)),Ok((updated,scoring_phrase,lyric_phrase))=>{
                let edit=Edit::PutNote {part,index:original.as_ref().map(|(i,_)|*i),expected:original.as_ref().map(|(_,n)|n.clone()),note:updated.clone(),scoring_phrase,lyric_phrase};
                if commit(studio,&binding,Action::EditHarmonies{edit}) {
                    let index=studio.snapshot().and_then(|doc|doc.content.harmonies.as_ref()?.voices.get(&part)?.notes.iter().position(|note|note==&updated));
                    studio.vocal_note.set(None);selected.set(index);page.set(index.unwrap_or(0)/PAGE);error.set(None);
                }else {error.set(studio.error.read().clone());}
            }}
        }},if editing {"Apply {label} note"}else{"Add {label} note"}}
        if let Some((index,expected))=note.clone() {
            div {class:"field-row",
                button {class:"studio-tool",onclick:{let binding=binding.clone();let expected=expected.clone();move |_|{
                    if commit(studio,&binding,Action::EditHarmonies {edit:Edit::RemoveNote {part,index,expected:expected.clone()}}){selected.set(None);error.set(None);}
                    else{error.set(studio.error.read().clone());}
                }},"Remove {label} note"}
                button {class:"studio-tool",disabled:!studio.snapshot().is_some_and(|doc|doc.has_recording()),title:"Loop the original recording for this note",onclick:{let binding=binding.clone();let expected=expected.clone();move |_|{
                    let Some(doc)=studio.snapshot() else{return};
                    if doc.content.harmonies.as_ref().and_then(|c|c.voices.get(&part)).and_then(|v|v.notes.get(index))!=Some(&expected){error.set(Some("Selected harmony note changed. Select it again.".into()));return;}
                    let Some(timing)=(studio.timing)()else{return};
                    let start=timing.seconds(expected.start).clamp(0.,doc.duration);let end=timing.seconds(expected.end).clamp(0.,doc.duration);
                    if end<=start{error.set(Some("This note is outside the recording.".into()));return;}
                    if commit(studio,&binding,Action::SetLoop {range:Some(TimeRange{start,end})}) {studio.seek(start);studio.dispatch(Action::Playback {playing:true});}
                }},"Loop recording"}
            }
        }
    }}
}

#[component]
fn HarmonyPhrases(studio: Studio, binding: Binding) -> Element {
    let mut kind = use_signal(|| PhraseKind::Scoring);
    let mut selected = use_signal(|| None::<usize>);
    let mut page = use_signal(|| 0usize);
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let Some(chart) = &doc.content.harmonies else {
        return rsx! {};
    };
    let phrases = match kind() {
        PhraseKind::Scoring => &chart.phrases,
        PhraseKind::Lyrics => &chart.lyric_phrases,
    };
    let pages = phrases.len().div_ceil(PAGE).max(1);
    let current_page = page().min(pages - 1);
    let selected_index = selected()
        .filter(|index| *index < phrases.len())
        .or_else(|| (!phrases.is_empty()).then_some(0));
    let phrase = selected_index.and_then(|i| phrases.get(i).cloned().map(|phrase| (i, phrase)));
    let next = selected_index.and_then(|i| phrases.get(i + 1).cloned());
    let key = format!("{:?}:{phrase:?}", kind());
    rsx! {div {class:"vocal-note-editor",style:"margin-top:16px",
        div {class:"form-heading","Shared phrases"}
        div {class:"field-row",
            button {class:if kind()==PhraseKind::Scoring{"studio-tool active"}else{"studio-tool"},onclick:move |_|{kind.set(PhraseKind::Scoring);selected.set(None);page.set(0);},"HARM1 scoring"}
            button {class:if kind()==PhraseKind::Lyrics{"studio-tool active"}else{"studio-tool"},onclick:move |_|{kind.set(PhraseKind::Lyrics);selected.set(None);page.set(0);},"HARM2 lyrics"}
        }
        div {class:"vocal-syllables",
            for (index,phrase) in phrases.iter().enumerate().skip(current_page*PAGE).take(PAGE) {
                button {class:if selected_index==Some(index){"studio-tool active"}else{"studio-tool"},onclick:move |_|selected.set(Some(index)),"{phrase.start}–{phrase.end} · player {phrase.player}"}
            }
        }
        if pages>1{div {class:"field-row",
            button {class:"studio-tool",disabled:current_page==0,onclick:move |_|page.set(current_page-1),"Previous phrases"}
            span {"Page {current_page+1} / {pages}"}
            button {class:"studio-tool",disabled:current_page+1>=pages,onclick:move |_|page.set(current_page+1),"Next phrases"}
        }}
        if let Some(phrase)=phrase{HarmonyPhraseForm {key:"{key}",studio,binding:binding.clone(),kind:kind(),phrase,next,selected,page}}
        else {p {class:"field-hint","Add a note with shared ranges to create phrases. Empty ranges are removed when their notes are deleted."}}
    }}
}
#[component]
fn HarmonyPhraseForm(
    studio: Studio,
    binding: Binding,
    kind: PhraseKind,
    phrase: (usize, VocalPhrase),
    next: Option<VocalPhrase>,
    mut selected: Signal<Option<usize>>,
    mut page: Signal<usize>,
) -> Element {
    let draft = use_signal(|| PhraseDraft::from(&phrase.1));
    let mut split =
        use_signal(|| (phrase.1.start + (phrase.1.end - phrase.1.start) / 2).to_string());
    let mut error = use_signal(|| None::<String>);
    let label = match kind {
        PhraseKind::Scoring => "HARM1 scoring",
        PhraseKind::Lyrics => "HARM2 lyrics",
    };
    rsx! {PhraseFields {draft,label}
        if let Some(error)=error(){p {class:"vocal-edit-error",role:"alert","{error}"}}
        div {class:"field-row",
            button {class:"studio-tool",onclick:{let binding=binding.clone();let original=phrase.clone();move |_|{
                match draft.read().parse() {Err(message)=>error.set(Some(message)),Ok(updated)=>{
                    let edit=Edit::PutPhrase {kind,index:Some(original.0),expected:Some(original.1.clone()),phrase:updated.clone()};
                    if commit(studio,&binding,Action::EditHarmonies{edit}) {
                        let index=studio.snapshot().and_then(|d|{let c=d.content.harmonies.as_ref()?;match kind{PhraseKind::Scoring=>&c.phrases,PhraseKind::Lyrics=>&c.lyric_phrases}.iter().position(|p|p==&updated)});
                        selected.set(index);page.set(index.unwrap_or(0)/PAGE);error.set(None);
                    }else{error.set(studio.error.read().clone());}
                }}
            }},"Apply shared phrase"}
        }
        p {class:"field-hint","Split at a gap between notes, or merge matching-player ranges. Singer notes stay unchanged."}
        div {class:"field-row",
            label {class:"field-stack",span {class:"field-label","Split tick"}input {class:"field-control",r#type:"number",min:"0",aria_label:"{label} split tick",value:split(),oninput:move|e|split.set(e.value())}}
            button {class:"studio-tool",onclick:{let binding=binding.clone();let original=phrase.clone();move |_|{
                let Ok(tick)=split().parse::<u64>()else{error.set(Some("Split tick must be a whole tick.".into()));return};
                if commit(studio,&binding,Action::EditHarmonies{edit:Edit::SplitPhrase{kind,index:original.0,expected:original.1.clone(),tick}}){selected.set(Some(original.0));page.set(original.0/PAGE);error.set(None);}
                else{error.set(studio.error.read().clone());}
            }},"Split phrase"}
            button {class:"studio-tool",disabled:next.is_none(),onclick:{let binding=binding.clone();let phrase=phrase.clone();move |_|{
                let Some(next_expected)=next.clone()else{return};
                if commit(studio,&binding,Action::EditHarmonies{edit:Edit::MergePhrase{kind,index:phrase.0,expected:phrase.1.clone(),next_expected}}){selected.set(Some(phrase.0));page.set(phrase.0/PAGE);error.set(None);}
                else{error.set(studio.error.read().clone());}
            }},"Merge with next"}
        }
    }
}
