//! Phrase audition and explicit, undoable lead-vocal edits.
use super::dialogs as rfd;
use super::{
    icons::Icon,
    state::{Studio, clock},
};
use crate::{
    editor::{Action, TimeRange},
    vocals::{Syllable, VocalChart, VocalNote, VocalTone},
};
use dioxus::prelude::*;

fn syllable(note: &VocalNote) -> String {
    match &note.lyric {
        Syllable::Missing => "Missing lyric".into(),
        Syllable::Text { text, join_next } => {
            format!("{text}{}", if *join_next { "-" } else { "" })
        }
        Syllable::Continue => "Pitch slide".into(),
        Syllable::ContinueWord => "Continue word".into(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[component]
pub fn VocalEditor(studio: Studio) -> Element {
    let mut recognize = use_signal(|| true);
    let mut language = use_signal(|| crate::lyric_language::Language::Auto);
    let mut lookup = use_signal(|| true);
    let mut control = use_signal(|| None::<std::sync::Arc<crate::clean_recognition::Control>>);
    let mut progress = use_signal(String::new);
    let mut generation_error = use_signal(|| None::<String>);
    let mut generation_notice = use_signal(|| None::<(String, String)>);
    let mut import_path = use_signal(String::new);
    let mut refresh = use_signal(|| 0u64);
    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            let message = control
                .read()
                .as_ref()
                .map(|control| control.message())
                .unwrap_or_default();
            if progress() != message {
                progress.set(message);
            }
        }
    });
    use_drop(move || {
        if let Some(control) = control.read().as_ref() {
            control.cancel();
        }
    });
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let present = doc.content.vocals.is_some();
    let stem_ready = doc.content.stems.as_ref().is_some_and(|manifest| {
        manifest
            .stems
            .iter()
            .any(|stem| stem.id == crate::stem_review::StemId::Vocals)
    });
    let busy = control.read().is_some();
    let selected = (studio.vocal_note)().and_then(|index| {
        doc.content
            .vocals
            .as_ref()?
            .notes
            .get(index)
            .cloned()
            .map(|note| (index, note))
    });
    let project_root = studio.store.read().as_ref().map(|store| store.root.clone());
    let cancelled = control
        .read()
        .as_ref()
        .is_some_and(|control| control.is_cancelled());
    rsx! {div{class:"vocal-editor",
        if let Some((index,note))=selected {
            VocalNoteEditor{key:"{project_root:?}:{doc.audio_sha256}:{index}:{note:?}",studio,index,note:note.clone(),audio_sha256:doc.audio_sha256.clone(),project_root}
        }
        if present{VocalPhrases{studio}}
        div{class:"vocal-generation-actions",
            button{class:"field-button vocal-generate",disabled:busy||!stem_ready,title:"Generate lead-vocal notes and timed lyrics from the attached vocal stem",onclick:move |_|{
                let Some(project)=studio.store.read().as_ref().map(|store|store.root.clone())else{return};
                let models=crate::generation::default_model_dir();let recognize=recognize();let lookup_lyrics=lookup();let language=language();
                let actor=crate::editor::Actor{kind:crate::editor::ActorKind::Human,name:studio.reviewer.read().clone()};
                let worker=std::sync::Arc::new(crate::clean_recognition::Control::default());control.set(Some(worker.clone()));generation_error.set(None);generation_notice.set(None);
                spawn(async move{
                    let expected_root=project.clone();
                    let result=tokio::task::spawn_blocking(move||crate::vocal_workflow::generate_project(crate::vocal_workflow::ProjectRequest{project:&project,model_dir:&models,recognize,language,neural_pitch:recognize,download_model:true,lookup_lyrics,actor},worker)).await;
                    if studio.store.read().as_ref().is_some_and(|store|store.root==expected_root){studio.reload(true);refresh.set(refresh()+1);
                    match result{Ok(Ok(result))=>{if !result.applied {
                        let message=match result.status{crate::vocal_generation::Status::ReviewRequired=>Some("New draft saved; current edits kept"),crate::vocal_generation::Status::NeedsLyrics if studio.snapshot().is_some_and(|doc|doc.content.vocal_review.is_some())=>Some("Draft needs timed lyrics; current vocals kept"),crate::vocal_generation::Status::NoVoicedPitch if studio.snapshot().is_some_and(|doc|doc.content.vocal_review.is_some())=>Some("No stable vocal pitch; current vocals kept"),_=>None};
                        if let Some(message)=message{generation_notice.set(Some((message.into(),result.output.display().to_string())));}
                    }},Ok(Err(error))=>generation_error.set(Some(format!("{error:#}"))),Err(error)=>generation_error.set(Some(error.to_string()))}
                    }
                    control.set(None);
                });
            },Icon{name:"music",size:14}if present{" Regenerate"}else{" Generate vocals"}}
            if busy{button{class:"studio-tool",disabled:cancelled,onclick:move |_|{if let Some(control)=control.read().as_ref(){control.cancel();}},if cancelled{"Cancelling…"}else{"Cancel"}}}
        }
        details{class:"review-evidence vocal-generation-options",summary{"Options"}
            label{class:"generation-field","Vocal language",select{class:"field-control",disabled:busy,value:match language(){crate::lyric_language::Language::Auto=>"auto",crate::lyric_language::Language::English=>"english",crate::lyric_language::Language::Spanish=>"spanish"},onkeydown:move|e|e.stop_propagation(),onchange:move|e|language.set(match e.value().as_str(){"english"=>crate::lyric_language::Language::English,"spanish"=>crate::lyric_language::Language::Spanish,_=>crate::lyric_language::Language::Auto}),option{value:"auto","Detect automatically"}option{value:"spanish","Spanish"}option{value:"english","English"}}}
            label{class:"form-check",title:"Optional local models recognize and align lyrics and add a pitch estimate. Off keeps pitch extraction model-free; supply timed words or import JSON to complete lyrics.",input{r#type:"checkbox",onkeydown:move|event|event.stop_propagation(),checked:recognize(),disabled:busy,onchange:move|event|recognize.set(event.checked())}"Local AI assistance"}
            label{class:"form-check",title:"Look up matching lyrics online; source metadata is sent to the lyrics provider",input{r#type:"checkbox",onkeydown:move|event|event.stop_propagation(),checked:lookup(),disabled:busy,onchange:move|event|lookup.set(event.checked())}"Fetch lyrics"}
        }
        if busy{div{class:"vocal-generation-progress",role:"status",if progress().is_empty(){"Analyzing vocals…"}else{"{progress}"}}}
        if let Some(error)=generation_error(){div{class:"vocal-edit-error",role:"alert","{error}"}}
        if let Some((message,path))=generation_notice(){div{class:"vocal-generation-progress",role:"status",title:"Saved proposal: {path}","{message}"}}
        div{class:"vocal-file-actions",
            button{class:"studio-tool",disabled:busy,title:"Import a model-free vocal chart JSON","data-browser-pick":"file","data-browser-accept":".json",onclick:move |_|import_document(studio),Icon{name:"folder-open",size:13}"Import JSON"}
            button{class:"studio-tool",disabled:!present,title:"Save the editable vocal chart as JSON",onclick:move |_|save_document(studio),Icon{name:"export",size:13}"Save JSON"}
        }
        details{class:"review-evidence vocal-import-path",summary{"Import path"}
            div{class:"vocal-file-actions",
                input{class:"field-control",aria_label:"Vocal JSON file path",placeholder:"/path/to/vocals.json",value:import_path(),onkeydown:move|event|event.stop_propagation(),oninput:move|event|import_path.set(event.value())}
                button{class:"studio-tool",disabled:busy||import_path().trim().is_empty(),onclick:move |_|import_document_path(studio,import_path().trim().into()),"Import"}
            }
        }
        details{class:"review-evidence",summary{"Vocal review checks"}VocalGenerationIssues{studio,refresh:refresh()}}
        if !present && !stem_ready{div{class:"vocal-empty",button{class:"studio-tool",title:"Choose an aligned stem manifest that includes vocals",onclick:move |_|studio.attach_stems_dialog(),Icon{name:"folder-open",size:13}"Attach stems"}}}
        else if !present{div{class:"vocal-empty","No vocal chart yet"}}
    }}
}

#[cfg(not(target_arch = "wasm32"))]
fn import_document(studio: Studio) {
    let Some(original) = studio.snapshot() else {
        return;
    };
    let original_root = studio.store.read().as_ref().map(|store| store.root.clone());
    spawn(async move {
        let dialog = rfd::AsyncFileDialog::new()
            .set_title("Import vocal chart JSON")
            .add_filter("Vocal chart JSON", &["json"]);
        let Some(file) = dialog.pick_file().await else {
            return;
        };
        let path = file.path().to_owned();
        apply_import(studio, path, original, original_root).await;
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn import_document_path(studio: Studio, path: std::path::PathBuf) {
    let Some(original) = studio.snapshot() else {
        return;
    };
    let original_root = studio.store.read().as_ref().map(|store| store.root.clone());
    spawn(async move {
        apply_import(studio, path, original, original_root).await;
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn read_import(path: &std::path::Path) -> anyhow::Result<VocalChart> {
    anyhow::ensure!(
        std::fs::metadata(path)?.len() <= 8 * 1024 * 1024,
        "Vocal JSON exceeds 8 MiB"
    );
    let document: VocalChart = serde_json::from_slice(&std::fs::read(path)?)?;
    document.validate()?;
    Ok(document)
}

#[cfg(not(target_arch = "wasm32"))]
async fn apply_import(
    mut studio: Studio,
    path: std::path::PathBuf,
    original: std::sync::Arc<crate::editor::Project>,
    original_root: Option<std::path::PathBuf>,
) {
    let result = tokio::task::spawn_blocking(move || read_import(&path)).await;
    let Some(current) = studio.snapshot() else {
        return;
    };
    if studio.store.read().as_ref().map(|store| store.root.clone()) != original_root
        || current.audio_sha256 != original.audio_sha256
        || current.content.vocals != original.content.vocals
    {
        studio.error.set(Some(
            "The project or vocals changed while choosing a file. Import again.".into(),
        ));
        return;
    }
    match result {
        Ok(Ok(document)) => studio.dispatch(Action::SetVocals { document }),
        Ok(Err(error)) => studio.error.set(Some(format!("{error:#}"))),
        Err(error) => studio.error.set(Some(error.to_string())),
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn save_document(mut studio: Studio) {
    let Some(document) = studio.snapshot().and_then(|doc| doc.content.vocals.clone()) else {
        return;
    };
    spawn(async move {
        let Some(file) = rfd::AsyncFileDialog::new()
            .set_title("Save editable vocal chart")
            .set_file_name("chart-engine-vocals.json")
            .add_filter("Vocal chart JSON", &["json"])
            .save_file()
            .await
        else {
            return;
        };
        let path = file.path().to_owned();
        let output = path.clone();
        match tokio::task::spawn_blocking(move || crate::generation::write_json(&path, &document))
            .await
        {
            Ok(Ok(())) => rfd::download(&output).await,
            Ok(Err(error)) => studio.error.set(Some(format!("{error:#}"))),
            Err(error) => studio.error.set(Some(error.to_string())),
        }
    });
}

fn issue_label(reason: crate::vocal_generation::Reason) -> &'static str {
    use crate::vocal_generation::Reason::*;
    match reason {
        MissingLyrics => "Missing lyrics",
        UntimedWord => "Untimed word",
        WordWithoutPitch => "Word without pitch",
        PitchDisagreement => "Pitch disagreement",
        OutOfRangePitch => "Pitch out of range",
        WordLevelAlignment => "Check syllable timing",
        PhraseBoundary => "Check phrase boundary",
        UnsupportedPitchGap => "Uncertain pitch gap",
        ShortWordFragment => "Short word fragment",
        CoveragePhraseBoundary => "Coverage at phrase break",
        LyricDisagreement => "Lyric sources differ",
    }
}
type GenerationChecks = (
    crate::vocal_generation::Generation,
    Option<crate::vocal_review::VocalReviewMap>,
);
#[cfg(not(target_arch = "wasm32"))]
#[component]
fn VocalGenerationIssues(studio: Studio, refresh: u64) -> Element {
    let source = use_memo(use_reactive((&refresh,), move |(refresh,)| {
        studio
            .store
            .read()
            .as_ref()
            .zip(studio.doc.read().as_ref())
            .map(|(store, doc)| {
                (
                    store.root.clone(),
                    doc.audio_sha256.clone(),
                    doc.content.vocal_review.is_some(),
                    refresh,
                )
            })
    }));
    let report = use_resource(move || {
        let source = source();
        async move {
            let Some((root, hash, attached, _)) = source else {
                return Ok(None);
            };
            if attached {
                return Ok(None);
            }
            tokio::task::spawn_blocking(move || -> Result<Option<GenerationChecks>, String> {
                let latest = root.join("vocal-generation.json");
                let path = if latest.exists() {
                    latest
                } else {
                    root.join("source/chart-engine-vocal-generation.json")
                };
                if !path.exists() {
                    return Ok(None);
                }
                let metadata = std::fs::metadata(&path).map_err(|error| error.to_string())?;
                if metadata.len() > 64 * 1024 * 1024 {
                    return Err("Vocal review report exceeds 64 MiB".into());
                }
                let report: crate::vocal_generation::Generation = serde_json::from_slice(
                    &std::fs::read(&path).map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
                if !report.audio_sha256.eq_ignore_ascii_case(&hash) {
                    return Err("Vocal review report belongs to another recording".into());
                }
                let review = crate::vocal_review::VocalReviewMap::from_generation(&report).ok();
                Ok(Some((report, review)))
            })
            .await
            .map_err(|error| error.to_string())?
        }
    });
    if studio
        .snapshot()
        .is_some_and(|doc| doc.content.vocal_review.is_some())
    {
        return rsx! {VocalReviewChecks{studio}};
    }
    let report = report.read();
    let Some(result) = report.as_ref() else {
        return rsx! {};
    };
    let (report, attach) = match result {
        Ok(Some(report)) => report,
        Ok(None) => return rsx! {},
        Err(error) => return rsx! {div{class:"vocal-edit-error","{error}"}},
    };
    let stale = studio
        .snapshot()
        .is_some_and(|doc| doc.content.vocals != report.chart);
    let attach = attach.clone();
    rsx! {div{class:"vocal-review-summary",
        if report.status==crate::vocal_generation::Status::NeedsLyrics{div{class:"vocal-review-status","Pitch draft saved; lyrics missing"}}
        if report.status==crate::vocal_generation::Status::NoVoicedPitch{div{class:"vocal-review-status","No stable vocal pitch"}}
        if let Some(review)=attach{button{class:"studio-tool",title:"Attach these source-bound checks without replacing your vocal edits or approving notes",onclick:move |_|studio.dispatch(Action::SetVocalReview{review:review.clone()}),Icon{name:"flag",size:12}"Load review checks"}}
        if !report.issues.is_empty(){details{class:"review-evidence",summary{Icon{name:"flag",size:12}" {report.issues.len()} vocal checks" if stale{span{class:"review-stale-badge",title:"These checks describe the generated draft, before your vocal edits","Draft"}}}
            div{class:"vocal-check-list",for issue in &report.issues{
                {let start=issue.start_seconds;let end=issue.end_seconds;let duration=studio.snapshot().map_or(0.,|doc|doc.duration);let valid=start.is_finite()&&end.is_finite()&&start>=0.&&start<end&&end<=duration;
                 rsx!{button{class:"vocal-check-row",disabled:!valid,title:"{issue.detail}",onclick:move |_|{studio.dispatch(Action::SetLoop{range:Some(TimeRange{start,end})});studio.seek(start);},span{class:"vocal-phrase-time","{clock(start)}"}span{"{issue_label(issue.reason)}"}}}}
            }}
        }}
    }}
}

#[component]
fn VocalReviewChecks(studio: Studio) -> Element {
    let mut show_hidden = use_signal(|| false);
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let Some(review) = doc.content.vocal_review.as_ref() else {
        return rsx! {};
    };
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let source_current = review.validate_binding(&doc).is_ok();
    let hidden_count = review
        .regions
        .iter()
        .filter(|r| doc.is_flag_hidden(&r.id, crate::vocal_review::TRACK))
        .count();
    let pending = review
        .regions
        .iter()
        .filter(|r| {
            !doc.is_flag_hidden(&r.id, crate::vocal_review::TRACK)
                && doc
                    .decision(&r.id, crate::vocal_review::TRACK, &timing)
                    .is_none_or(|d| d.status == crate::editor::DecisionStatus::Pending)
        })
        .count();
    rsx! {div{class:"vocal-review-summary",
        div{class:"vocal-review-heading",Icon{name:"flag",size:12}strong{"{pending} pending"}
            if hidden_count>0{label{class:"form-check",input{r#type:"checkbox",checked:show_hidden(),onchange:move|event|show_hidden.set(event.checked())}"Hidden ({hidden_count})"}}
        }
        if !source_current{span{class:"review-stale-badge",title:"These checks describe a different recording or vocal stem; load matching checks before approving","Source changed"}}
        div{class:"vocal-review-regions",for region in &review.regions{
            if show_hidden()||!doc.is_flag_hidden(&region.id,crate::vocal_review::TRACK){
                {let id=region.id.clone();let approve_id=id.clone();let hide_id=id.clone();let jump_id=id.clone();let start=region.start;let end=region.end;
                let hidden=doc.is_flag_hidden(&id,crate::vocal_review::TRACK);let decision=doc.decision(&id,crate::vocal_review::TRACK,&timing);
                let approved=decision.is_some_and(|d|matches!(d.status,crate::editor::DecisionStatus::Reviewed|crate::editor::DecisionStatus::Resolved));
                let visibility_icon=if hidden{"eye"}else{"eye-off"};
                let changed=decision.is_none()&&doc.content.decisions.iter().any(|d|d.track==crate::vocal_review::TRACK&&d.region_id==id);
                let class=if hidden{"vocal-review-region hidden"}else if approved{"vocal-review-region reviewed"}else if doc.selected_region.as_ref()==Some(&id){"vocal-review-region active"}else{"vocal-review-region"};
                rsx!{div{key:"{id}",class,title:"{region.detail}",
                    button{class:"vocal-review-jump",title:"Loop {clock(start)}–{clock(end)}: {region.detail}",onclick:move |_|{studio.dispatch(Action::SelectRegion{region_id:jump_id.clone()});studio.seek(start);},span{class:"vocal-phrase-time","{clock(start)}–{clock(end)}"}span{"{issue_label(region.reason)}"}}
                    div{class:"vocal-review-actions",
                        span{class:"vocal-review-state",if hidden{"Hidden"}else if approved{"Reviewed"}else if decision.is_some_and(|d|d.status==crate::editor::DecisionStatus::Dismissed){"Dismissed"}else if changed{"Changed"}else{"Pending"}}
                        button{class:"review-approve",disabled:approved||!source_current,title:"Approve current vocal notes and lyrics only in this section",onclick:move |_|studio.dispatch(Action::Decide{region_id:approve_id.clone(),track:crate::vocal_review::TRACK.into(),status:crate::editor::DecisionStatus::Reviewed,comment:"Approved current vocal notes and lyrics in this section".into()}),Icon{name:"check",size:12}if approved{"Approved"}else{"Approve"}}
                        button{class:"review-hide",title:if hidden{"Show this flag without changing its decision"}else{"Hide this flag without approving notes"},onclick:move |_|studio.dispatch(Action::SetFlagVisibility{region_id:hide_id.clone(),track:crate::vocal_review::TRACK.into(),hidden:!hidden}),Icon{name:visibility_icon,size:12}if hidden{"Unhide"}else{"Hide"}}
                    }
                    details{class:"review-evidence",summary{"{region.issues.len()} checks"}
                        div{class:"vocal-review-causes",for (issue_index,issue) in region.issues.iter().enumerate(){div{key:"{issue_index}",class:"vocal-review-cause",span{class:"vocal-phrase-time","{clock(issue.start_seconds)}–{clock(issue.end_seconds)}"}strong{"{issue_label(issue.reason)}"}span{"{issue.detail}"}}}}
                    }
                }}}
            }
        }}
    }}
}

#[component]
fn VocalPhrases(studio: Studio) -> Element {
    let mut phrase_index = use_signal(|| 0usize);
    let selected_note = studio.vocal_note;
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let Some(chart) = doc.content.vocals.as_ref() else {
        return rsx! {};
    };
    let Some(timing) = (studio.timing)() else {
        return rsx! {};
    };
    let index = selected_note()
        .and_then(|index| chart.notes.get(index))
        .and_then(|note| {
            chart
                .phrases
                .iter()
                .position(|phrase| phrase.start <= note.start && note.end <= phrase.end)
        })
        .unwrap_or_else(|| phrase_index().min(chart.phrases.len().saturating_sub(1)));
    let selected = chart.phrases.get(index);
    rsx! {
        div {class:"vocal-editor",
            div {class:"vocal-summary",strong{"Lead vocals"}span{"{chart.phrases.len()} phrases · {chart.notes.len()} notes"}}
            div {class:"vocal-phrase-list",role:"group",aria_label:"Vocal phrases",
                for (index,phrase) in chart.phrases.iter().enumerate() {
                    {let start=timing.seconds(phrase.start);let end=timing.seconds(phrase.end);
                     let words=chart.notes.iter().filter(|note|phrase.start<=note.start && note.end<=phrase.end).map(syllable).collect::<Vec<_>>().join(" ");
                     rsx!{button{class:if selected==Some(phrase){"vocal-phrase-choice active"}else{"vocal-phrase-choice"},title:"Loop vocal phrase at {clock(start)}",onclick:move |_|{phrase_index.set(index);studio.clear_selection();studio.dispatch(Action::SetLoop{range:Some(TimeRange{start,end})});studio.seek(start);},
                        span{class:"vocal-phrase-time","{clock(start)}"}span{class:"vocal-phrase-words","{words}"}
                     }}
                    }
                }
            }
            if let Some(phrase)=selected {
                div{class:"form-heading",style:"margin-top:16px","Syllables"}
                div{class:"vocal-syllables",
                    for (index,note) in chart.notes.iter().enumerate().filter(|(_,note)|phrase.start<=note.start && note.end<=phrase.end){
                        button{class:if selected_note()==Some(index){"studio-tool active"}else{"studio-tool"},title:"Edit vocal note at tick {note.start}",onclick:move |_|studio.select_vocal(index,false,false),"{syllable(note)}"}
                    }
                }
            }
            details{class:"review-evidence",style:"margin-top:16px",summary{"Source"}div{class:"review-evidence-body","{chart.source}"}}
        }
    }
}

#[derive(Clone)]
struct NoteDraft {
    start: String,
    end: String,
    pitch: String,
    text: String,
    mode: String,
    join: bool,
    continuation: bool,
    continue_word: bool,
}
impl NoteDraft {
    fn from_note(note: &VocalNote) -> Self {
        let (pitch, mode) = match note.tone {
            VocalTone::Pitched { midi } => (midi.to_string(), "pitched"),
            VocalTone::Talkie { lenient: false } => ("60".into(), "talkie"),
            VocalTone::Talkie { lenient: true } => ("60".into(), "lenient"),
        };
        let (text, join, continuation, continue_word) = match &note.lyric {
            Syllable::Missing => (String::new(), false, false, false),
            Syllable::Text { text, join_next } => (text.clone(), *join_next, false, false),
            Syllable::Continue => (String::new(), false, true, false),
            Syllable::ContinueWord => (String::new(), false, false, true),
        };
        Self {
            start: note.start.to_string(),
            end: note.end.to_string(),
            pitch,
            text,
            mode: mode.into(),
            join,
            continuation,
            continue_word,
        }
    }
    fn parse(&self) -> Result<VocalNote, String> {
        let start = self
            .start
            .parse()
            .map_err(|_| "Start must be a whole tick.".to_owned())?;
        let end = self
            .end
            .parse()
            .map_err(|_| "End must be a whole tick.".to_owned())?;
        let tone = match self.mode.as_str() {
            "talkie" => VocalTone::Talkie { lenient: false },
            "lenient" => VocalTone::Talkie { lenient: true },
            _ => VocalTone::Pitched {
                midi: self
                    .pitch
                    .parse()
                    .map_err(|_| "Pitch must be MIDI 36–84.".to_owned())?,
            },
        };
        if self.continuation && self.continue_word {
            return Err("Choose either Continue word or Pitch slide.".into());
        }
        let lyric = if self.continue_word {
            Syllable::ContinueWord
        } else if self.continuation {
            Syllable::Continue
        } else if self.text.is_empty() && !self.join {
            Syllable::Missing
        } else {
            Syllable::Text {
                text: self.text.clone(),
                join_next: self.join,
            }
        };
        Ok(VocalNote {
            start,
            end,
            tone,
            lyric,
        })
    }
}

/// Validate a complete replacement, preserving every unselected note/phrase.
fn replacement(
    chart: &VocalChart,
    index: usize,
    expected: &VocalNote,
    draft: &NoteDraft,
) -> Result<VocalChart, String> {
    if chart.notes.get(index) != Some(expected) {
        return Err("This note changed. Select it again before editing.".into());
    }
    let mut updated = chart.clone();
    updated.notes[index] = draft.parse()?;
    updated.validate().map_err(|error| error.to_string())?;
    Ok(updated)
}

#[component]
fn VocalNoteEditor(
    studio: Studio,
    index: usize,
    note: VocalNote,
    audio_sha256: String,
    project_root: Option<std::path::PathBuf>,
) -> Element {
    let mut draft = use_signal(|| NoteDraft::from_note(&note));
    let mut error = use_signal(|| None::<String>);
    let current = draft.read().clone();
    let pitched = current.mode == "pitched";
    rsx! {div{class:"vocal-note-editor",
        div{class:"field-row",
            div{class:"field-stack field-half",label{class:"field-label","Start tick"}input{class:"field-control",onkeydown:move|event|event.stop_propagation(),aria_label:"Vocal note start tick",r#type:"number",min:"0",value:current.start,oninput:move|event|draft.write().start=event.value()}}
            div{class:"field-stack field-half",label{class:"field-label","End tick"}input{class:"field-control",onkeydown:move|event|event.stop_propagation(),aria_label:"Vocal note end tick",r#type:"number",min:"1",value:current.end,oninput:move|event|draft.write().end=event.value()}}
        }
        div{class:"field-stack",label{class:"field-label","Syllable"}input{class:"field-control",onkeydown:move|event|event.stop_propagation(),aria_label:"Vocal syllable text",placeholder:"Missing lyric",value:current.text,disabled:current.continuation || current.continue_word,oninput:move|event|draft.write().text=event.value()}}
        div{class:"field-row",
            div{class:"field-stack field-half",label{class:"field-label","Type"}select{class:"field-control",onkeydown:move|event|event.stop_propagation(),aria_label:"Vocal note type",value:current.mode,onchange:move|event|draft.write().mode=event.value(),option{value:"pitched","Pitched"}option{value:"talkie","Talkie"}option{value:"lenient","Lenient talkie"}}}
            div{class:"field-stack field-half",label{class:"field-label","MIDI pitch"}input{class:"field-control",onkeydown:move|event|event.stop_propagation(),aria_label:"Vocal MIDI pitch",r#type:"number",min:"36",max:"84",value:current.pitch,disabled:!pitched,oninput:move|event|draft.write().pitch=event.value()}}
        }
        div{class:"vocal-note-options",
            label{class:"form-check",input{r#type:"checkbox",onkeydown:move|event|event.stop_propagation(),checked:current.continue_word,onchange:move|event|{let mut value=draft.write();value.continue_word=event.checked();if value.continue_word{value.continuation=false;}}}"Continue word"}
            label{class:"form-check",input{r#type:"checkbox",onkeydown:move|event|event.stop_propagation(),checked:current.continuation,onchange:move|event|{let mut value=draft.write();value.continuation=event.checked();if value.continuation{value.continue_word=false;}}}"Pitch slide"}
            label{class:"form-check",input{r#type:"checkbox",onkeydown:move|event|event.stop_propagation(),checked:current.join,disabled:current.continuation || current.continue_word,onchange:move|event|draft.write().join=event.checked()}"Join next"}
        }
        if let Some(error)=error(){div{class:"vocal-edit-error",role:"alert","{error}"}}
        button{class:"field-button",style:"margin-top:12px",onclick:move |_|{
            let Some(doc)=studio.snapshot()else{return};
            if studio.store.read().as_ref().map(|store|store.root.clone())!=project_root || doc.audio_sha256!=audio_sha256{error.set(Some("The project changed. Select a current vocal note.".into()));return;}
            let Some(chart)=doc.content.vocals.as_ref()else{return};
            match replacement(chart,index,&note,&draft.read()){
                Ok(document)=>{studio.dispatch(Action::SetVocals{document});error.set(None);},
                Err(message)=>error.set(Some(message))
            }
        },Icon{name:"check",size:14}" Apply note"}
    }}
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chart() -> VocalChart {
        VocalChart {
            version: 1,
            resolution: 192,
            source: "supplied MIDI".into(),
            phrases: vec![crate::vocals::VocalPhrase {
                start: 0,
                end: 1000,
                player: 1,
            }],
            notes: vec![
                VocalNote {
                    start: 100,
                    end: 200,
                    tone: VocalTone::Pitched { midi: 60 },
                    lyric: Syllable::Text {
                        text: "hello".into(),
                        join_next: false,
                    },
                },
                VocalNote {
                    start: 300,
                    end: 500,
                    tone: VocalTone::Pitched { midi: 62 },
                    lyric: Syllable::Text {
                        text: "world".into(),
                        join_next: false,
                    },
                },
            ],
        }
    }
    #[test]
    fn staged_vocal_edits_preserve_neighbors_and_reject_stale_notes() {
        let original = chart();
        let mut draft = NoteDraft::from_note(&original.notes[0]);
        draft.pitch = "64".into();
        draft.text = "hey".into();
        let updated = replacement(&original, 0, &original.notes[0], &draft).unwrap();
        assert_eq!(updated.notes[1], original.notes[1]);
        assert_eq!(updated.phrases, original.phrases);
        assert_eq!(updated.notes[0].tone, VocalTone::Pitched { midi: 64 });
        assert!(
            replacement(&updated, 0, &original.notes[0], &draft)
                .unwrap_err()
                .contains("changed")
        );
    }
    #[test]
    fn invalid_vocal_edits_cannot_break_phrase_pitch_or_monophony() {
        let original = chart();
        let mut draft = NoteDraft::from_note(&original.notes[0]);
        draft.end = "400".into();
        assert!(replacement(&original, 0, &original.notes[0], &draft).is_err());
        draft.end = "200".into();
        draft.pitch = "90".into();
        assert!(replacement(&original, 0, &original.notes[0], &draft).is_err());
        draft.pitch = "60".into();
        draft.continuation = true;
        assert!(replacement(&original, 0, &original.notes[0], &draft).is_err());
        draft.continuation = false;
        draft.continue_word = true;
        assert!(replacement(&original, 0, &original.notes[0], &draft).is_err());
        draft.continue_word = false;
        draft.text = "two words".into();
        assert!(replacement(&original, 0, &original.notes[0], &draft).is_err());
    }
    #[test]
    fn independent_word_continuation_and_authored_slide_have_distinct_drafts() {
        let original = chart();
        for lyric in [
            Syllable::Continue,
            Syllable::ContinueWord,
            Syllable::Missing,
        ] {
            let mut note = original.notes[1].clone();
            note.lyric = lyric.clone();
            let draft = NoteDraft::from_note(&note);
            assert_eq!(draft.parse().unwrap(), note);
            assert_eq!(draft.continue_word, lyric == Syllable::ContinueWord);
            assert_eq!(draft.continuation, lyric == Syllable::Continue);
            assert_ne!(syllable(&note), "+");
            let updated = replacement(&original, 1, &original.notes[1], &draft).unwrap();
            assert_eq!(updated.notes[1].lyric, lyric);
            assert_eq!(updated.notes[0], original.notes[0]);
        }
        let mut ambiguous = NoteDraft::from_note(&original.notes[1]);
        ambiguous.continuation = true;
        ambiguous.continue_word = true;
        assert!(ambiguous.parse().is_err());
    }
    #[test]
    fn vocal_import_validates_schema_and_bounds_before_apply() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vocals.json");
        let original = chart();
        std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
        assert_eq!(read_import(&path).unwrap(), original);
        let mut invalid = original;
        invalid.notes[0].tone = VocalTone::Pitched { midi: 100 };
        std::fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(read_import(&path).is_err());
        std::fs::write(&path, b"not JSON").unwrap();
        assert!(read_import(&path).is_err());
        std::fs::File::create(&path)
            .unwrap()
            .set_len(8 * 1024 * 1024 + 1)
            .unwrap();
        assert!(
            read_import(&path)
                .unwrap_err()
                .to_string()
                .contains("8 MiB")
        );
    }
}

#[cfg(target_arch = "wasm32")]
#[component]
pub fn VocalEditor(mut studio: Studio) -> Element {
    rsx! {VocalPhrases{studio}
    div{class:"vocal-file-actions",
    button{class:"studio-tool",onclick:move |_|{spawn(async move{let result=async{let Some(files)=crate::files::pick(false,".json").await?else{return Ok(())};let document:VocalChart=serde_json::from_slice(&files[0].1)?;document.validate()?;studio.dispatch(Action::SetVocals{document});Ok::<_,anyhow::Error>(())}.await;if let Err(error)=result{studio.error.set(Some(format!("{error:#}")));}});},"Import JSON"}
    button{class:"studio-tool",disabled:!studio.snapshot().is_some_and(|doc|doc.content.vocals.is_some()),onclick:move |_|{if let Some(doc)=studio.snapshot(){if let Some(chart)=&doc.content.vocals{let result=serde_json::to_vec_pretty(chart).map_err(anyhow::Error::from).and_then(|bytes|crate::files::download("vocals.json",&bytes));if let Err(error)=result{studio.error.set(Some(format!("{error:#}")));}}}},"Save JSON"}
    }p{class:"field-hint","Place notes in the vocal lane or import a vocal document. Automatic generation is unavailable in this browser build."}}
}
