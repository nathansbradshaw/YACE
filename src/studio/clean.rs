//! Local word review and clean-audio export in the shared tool sheet.
//! Matching and rendering stay in the model-free clean core and CLI contract.
use super::dialogs as rfd;
use super::state::Studio;
use crate::{
    clean::{
        self, CleanMode, CleanPlan, CleanPolicy, CleanReceipt, MuteRange, RenderRequest,
        StemAlignment, WordTimings,
    },
    clean_recognition::{self, Control, Recognition},
    editor::{Action, TimeRange},
    models,
};
use anyhow::{Context, Result, ensure};
use dioxus::prelude::*;
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

const CSS: &str = include_str!("../../assets/clean.css");

#[derive(Clone)]
struct View {
    source: Option<PathBuf>,
    epoch: u64,
    words: Option<WordTimings>,
    recognition: Option<Recognition>,
    recognition_control: Option<Arc<Control>>,
    word_path: Option<PathBuf>,
    plan: Option<CleanPlan>,
    name: String,
    terms: String,
    padding_before: f64,
    padding_after: f64,
    fade: f64,
    mode: CleanMode,
    stem: Option<PathBuf>,
    stem_hashes: Option<(String, String)>,
    evidence: String,
    output_parent: Option<PathBuf>,
    output_name: String,
    busy: Option<String>,
    status: String,
    error: Option<String>,
    receipt: Option<(CleanReceipt, PathBuf)>,
}
impl Default for View {
    fn default() -> Self {
        let policy = CleanPolicy::default();
        Self {
            source: None,
            epoch: 0,
            words: None,
            recognition: None,
            recognition_control: None,
            word_path: None,
            plan: None,
            name: policy.name,
            terms: policy.terms.join("\n"),
            padding_before: policy.padding_before_seconds,
            padding_after: policy.padding_after_seconds,
            fade: policy.fade_seconds,
            mode: CleanMode::VocalOnly,
            stem: None,
            stem_hashes: None,
            evidence: String::new(),
            output_parent: rfd::output_parent(),
            output_name: "clean-copy".into(),
            busy: None,
            status: String::new(),
            error: None,
            receipt: None,
        }
    }
}
impl View {
    fn policy(&self) -> CleanPolicy {
        CleanPolicy {
            version: 1,
            name: self.name.clone(),
            terms: self
                .terms
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect(),
            padding_before_seconds: self.padding_before,
            padding_after_seconds: self.padding_after,
            fade_seconds: self.fade,
        }
    }
    fn adopt(&mut self, plan: CleanPlan) {
        self.name = plan.policy.name.clone();
        self.terms = plan.policy.terms.join("\n");
        self.padding_before = plan.policy.padding_before_seconds;
        self.padding_after = plan.policy.padding_after_seconds;
        self.fade = plan.policy.fade_seconds;
        self.mode = plan.mode;
        self.plan = Some(plan);
    }
    fn output(&self) -> Option<PathBuf> {
        let name = Path::new(self.output_name.trim());
        if matches!(
            name.components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        ) {
            self.output_parent.as_ref().map(|parent| parent.join(name))
        } else {
            None
        }
    }
}
enum Update {
    Words(WordTimings, PathBuf),
    Plan(CleanPlan, PathBuf),
    Built(CleanPlan),
    Saved(PathBuf),
    Stem(PathBuf, String, String),
    Rendered(CleanReceipt, PathBuf),
}

fn current(data: Signal<View>, source: &Path, epoch: u64) -> bool {
    let view = data.read();
    view.source.as_deref() == Some(source) && view.epoch == epoch
}
fn check_source(path: &Path, hash: &str) -> Result<()> {
    ensure!(
        models::hash_file(path)?.eq_ignore_ascii_case(hash),
        "This document belongs to different source audio. Load word timings or a plan for the current recording."
    );
    Ok(())
}
fn save_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary
        .persist_noclobber(path)
        .map_err(|error| error.error)
        .context("Choose a new filename; existing plans are preserved")?;
    Ok(())
}

fn job(
    mut studio: Studio,
    mut data: Signal<View>,
    source: PathBuf,
    epoch: u64,
    label: &str,
    task: impl FnOnce() -> Result<Update> + Send + 'static,
) {
    if !current(data, &source, epoch) || data.read().busy.is_some() {
        return;
    }
    data.with_mut(|view| {
        view.busy = Some(label.into());
        view.error = None;
    });
    spawn(async move {
        let result = match tokio::task::spawn_blocking(task).await {
            Ok(result) => result,
            Err(error) => Err(error.into()),
        };
        if !current(data, &source, epoch) {
            return;
        }
        let download = match &result {
            Ok(Update::Saved(path) | Update::Rendered(_, path)) => Some(path.clone()),
            _ => None,
        };
        data.with_mut(|view|{
            view.busy=None;
            match result {
                Err(error)=>view.error=Some(format!("{error:#}")),
                Ok(Update::Words(words,path))=>{view.status=format!("Loaded {} supplied words. Review the explicit markers and build candidate ranges.",words.words.len());view.words=Some(words);view.word_path=Some(path);view.recognition=None;},
                Ok(Update::Plan(plan,path))=>{view.adopt(plan);view.recognition=None;view.status=format!("Loaded plan: {}",path.display());},
                Ok(Update::Built(plan))=>{view.status=format!("Built {} exact matches and {} editable ranges. Unknown words/times were not guessed.",plan.matches.len(),plan.ranges.len());view.plan=Some(plan);},
                Ok(Update::Saved(path))=>view.status=format!("Saved new plan revision: {}",path.display()),
                Ok(Update::Stem(path,audio_hash,stem_hash))=>{view.status="Selected stem is bound to these source files. Describe its alignment below.".into();view.stem=Some(path);view.stem_hashes=Some((audio_hash,stem_hash));view.evidence.clear();},
                Ok(Update::Rendered(receipt,path))=>{view.status=format!("Rendered separate candidate: {}. Listen before accepting it.",path.display());view.receipt=Some((receipt,path));},
            }
        });
        if let Some(error) = data.read().error.clone() {
            studio.error.set(Some(error));
        } else {
            studio.status.set(data.read().status.clone());
            if let Some(path) = download {
                rfd::download(&path).await;
            }
        }
    });
}

fn load_words(studio: Studio, data: Signal<View>, source: PathBuf, epoch: u64) {
    spawn(async move {
        let Some(file) = rfd::AsyncFileDialog::new()
            .set_title("Load supplied word timings")
            .add_filter("Word timing JSON", &["json"])
            .pick_file()
            .await
        else {
            return;
        };
        let path = file.path().to_owned();
        let audio = source.clone();
        job(
            studio,
            data,
            source,
            epoch,
            "Loading word timings…",
            move || {
                let words: WordTimings = serde_json::from_slice(&fs::read(&path)?)?;
                words.validate()?;
                check_source(&audio, &words.audio_sha256)?;
                Ok(Update::Words(words, path))
            },
        );
    });
}
fn load_plan(studio: Studio, data: Signal<View>, source: PathBuf, epoch: u64) {
    spawn(async move {
        let Some(file) = rfd::AsyncFileDialog::new()
            .set_title("Load a clean plan")
            .add_filter("Clean plan JSON", &["json"])
            .pick_file()
            .await
        else {
            return;
        };
        let path = file.path().to_owned();
        let audio = source.clone();
        job(
            studio,
            data,
            source,
            epoch,
            "Loading clean plan…",
            move || {
                let plan: CleanPlan = serde_json::from_slice(&fs::read(&path)?)?;
                plan.validate()?;
                check_source(&audio, &plan.audio_sha256)?;
                Ok(Update::Plan(plan, path))
            },
        );
    });
}
fn save_plan(studio: Studio, data: Signal<View>, source: PathBuf, epoch: u64) {
    let Some(plan) = data.read().plan.clone() else {
        return;
    };
    spawn(async move {
        let Some(file) = rfd::AsyncFileDialog::new()
            .set_title("Save a new clean-plan revision")
            .set_file_name("clean-plan.json")
            .add_filter("Clean plan JSON", &["json"])
            .save_file()
            .await
        else {
            return;
        };
        let path = file.path().to_owned();
        let audio = source.clone();
        job(
            studio,
            data,
            source,
            epoch,
            "Saving clean plan…",
            move || {
                plan.validate()?;
                check_source(&audio, &plan.audio_sha256)?;
                save_new(&path, &serde_json::to_vec_pretty(&plan)?)?;
                Ok(Update::Saved(path))
            },
        );
    });
}
fn choose_stem(studio: Studio, data: Signal<View>, source: PathBuf, epoch: u64) {
    spawn(async move {
        let Some(file) = rfd::AsyncFileDialog::new()
            .set_title("Choose this recording's aligned vocal stem")
            .add_filter("Audio", &["wav", "flac", "ogg", "mp3", "m4a"])
            .pick_file()
            .await
        else {
            return;
        };
        let path = file.path().to_owned();
        let audio = source.clone();
        job(
            studio,
            data,
            source,
            epoch,
            "Binding selected stem/source hashes…",
            move || {
                let audio_hash = models::hash_file(&audio)?;
                let stem_hash = models::hash_file(&path)?;
                Ok(Update::Stem(path, audio_hash, stem_hash))
            },
        );
    });
}
fn choose_parent(mut data: Signal<View>, source: PathBuf, epoch: u64) {
    spawn(async move {
        if let Some(folder) = rfd::AsyncFileDialog::new()
            .set_title("Choose an output parent; a new child folder will be created")
            .pick_folder()
            .await
            && current(data, &source, epoch)
        {
            data.write().output_parent = Some(folder.path().to_owned());
        }
    });
}
fn build(studio: Studio, data: Signal<View>, source: PathBuf, epoch: u64) {
    let view = data.read().clone();
    let policy = view.policy();
    let Some(words) = view.words else { return };
    let mode = view.mode;
    let audio = source.clone();
    job(
        studio,
        data,
        source,
        epoch,
        "Matching supplied words…",
        move || {
            check_source(&audio, &words.audio_sha256)?;
            Ok(Update::Built(clean::plan(&words, &policy, mode)?))
        },
    );
}

/// Recognition runs off the UI thread. Source changes and Cancel stop inference;
/// no result from a previous recording can replace the current panel state.
fn detect_automatic(mut studio: Studio, mut data: Signal<View>, source: PathBuf, epoch: u64) {
    if !current(data, &source, epoch) || data.read().busy.is_some() {
        return;
    }
    let view = data.read().clone();
    let policy = view.policy();
    if policy.validate().is_err() {
        return;
    }
    let mode = view.mode;
    let parent = studio
        .store
        .read()
        .as_ref()
        .map(|store| store.root.join("clean-analysis"));
    let Some(parent) = parent else {
        return;
    };
    let selected = view
        .stem
        .zip(view.stem_hashes)
        .filter(|_| !view.evidence.trim().is_empty())
        .map(|(path, (audio_sha256, stem_sha256))| {
            (
                path,
                StemAlignment {
                    audio_sha256,
                    stem_sha256,
                    evidence: view.evidence,
                },
            )
        });
    let attached = studio
        .doc
        .read()
        .as_ref()
        .and_then(|doc| doc.content.stems.as_ref())
        .and_then(|manifest| {
            manifest
                .stems
                .iter()
                .find(|stem| stem.id == crate::stem_review::StemId::Vocals)
                .map(|stem| {
                    (
                        stem.path.clone(),
                        StemAlignment {
                            audio_sha256: manifest.source_audio_sha256.clone(),
                            stem_sha256: stem.sha256.clone(),
                            evidence: format!(
                                "{}: {}",
                                manifest.alignment.method, manifest.alignment.details
                            ),
                        },
                    )
                })
        });
    let stem = selected.or(attached);
    let task_stem = stem.clone();
    let control = Arc::new(Control::default());
    data.with_mut(|view| {
        view.busy = Some("Starting automatic lyric analysis…".into());
        view.error = None;
        view.recognition_control = Some(control.clone());
    });
    let task_control = control.clone();
    let audio = source.clone();
    let model_dir = crate::generation::default_model_dir();
    spawn(async move {
        let mut worker = tokio::task::spawn_blocking(move || -> Result<_> {
            let detection = clean_recognition::detect(
                clean_recognition::Request {
                    audio: &audio,
                    model_dir: &model_dir,
                    download_model: true,
                    vocal_stem: task_stem
                        .as_ref()
                        .map(|(path, binding)| (path.as_path(), binding)),
                },
                &policy,
                mode,
                task_control.clone(),
            )?;
            ensure!(
                !task_control.is_cancelled(),
                "Automatic lyric analysis cancelled"
            );
            let saved = clean_recognition::save_run(&parent, &detection)?;
            Ok((detection, saved))
        });
        let mut ticker = tokio::time::interval(std::time::Duration::from_millis(200));
        let result = loop {
            tokio::select! {
                result = &mut worker => break result,
                _ = ticker.tick() => {
                    if current(data, &source, epoch) {
                        let message = if control.is_cancelled() { "Cancelling automatic analysis…".into() } else { control.message() };
                        if !message.is_empty() { data.write().busy=Some(message); }
                    } else { control.cancel(); }
                }
            }
        };
        if !current(data, &source, epoch) {
            return;
        }
        data.with_mut(|view| {
            view.busy=None;
            view.recognition_control=None;
            if control.is_cancelled() { view.status="Automatic analysis cancelled. Existing words and ranges were kept.".into(); return; }
            match result {
                Ok(Ok((detection, saved))) => {
                    view.status=format!("Recognized {} words and found {} explicit matches ({} timed ranges). Analysis saved automatically. Listen and review the edits.", detection.recognition.words.words.len(), detection.plan.matches.len(), detection.plan.ranges.len());
                    view.word_path=Some(saved.join("words.json"));
                    view.words=Some(detection.recognition.words.clone());
                    view.recognition=Some(detection.recognition);
                    view.plan=Some(detection.plan);
                    view.receipt=None;
                    if let Some((path, binding)) = stem { view.stem=Some(path); view.stem_hashes=Some((binding.audio_sha256, binding.stem_sha256)); view.evidence=binding.evidence; }
                },
                Ok(Err(error)) => view.error=Some(format!("{error:#}")),
                Err(error) => view.error=Some(format!("Automatic analysis worker failed: {error}")),
            }
        });
        if let Some(error) = data.read().error.clone() {
            studio.error.set(Some(error));
        } else {
            studio.status.set(data.read().status.clone());
        }
    });
}
fn render(studio: Studio, data: Signal<View>, source: PathBuf, epoch: u64) {
    let view = data.read().clone();
    let Some(output) = view.output() else { return };
    let Some(plan) = view.plan else { return };
    let stem = if plan.mode == CleanMode::VocalOnly {
        view.stem
    } else {
        None
    };
    let alignment = if plan.mode == CleanMode::VocalOnly {
        view.stem_hashes
            .map(|(audio_sha256, stem_sha256)| StemAlignment {
                audio_sha256,
                stem_sha256,
                evidence: view.evidence,
            })
    } else {
        None
    };
    let audio = source.clone();
    job(
        studio,
        data,
        source,
        epoch,
        "Rendering separate clean audio…",
        move || {
            let receipt = clean::render(RenderRequest {
                audio: &audio,
                plan: &plan,
                vocal_stem: stem.as_deref(),
                stem_alignment: alignment.as_ref(),
                output: &output,
            })?;
            Ok(Update::Rendered(receipt, output))
        },
    );
}

fn set_range(mut data: Signal<View>, index: usize, change: impl FnOnce(&mut MuteRange)) {
    if let Some(range) = data
        .write()
        .plan
        .as_mut()
        .and_then(|plan| plan.ranges.get_mut(index))
    {
        change(range);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_starts_with_explicit_markers_and_loaded_plans_keep_their_custom_policy() {
        let mut view = View::default();
        assert_eq!(view.policy(), CleanPolicy::default());
        let words = WordTimings {
            version: 1,
            audio_sha256: "a".repeat(64),
            duration_seconds: 1.,
            source: "Manually timed test word".into(),
            words: vec![clean::AlignedWord {
                text: Some("heck".into()),
                start: Some(0.1),
                end: Some(0.2),
            }],
        };
        let custom = CleanPolicy {
            name: "Saved custom choices".into(),
            terms: vec!["heck".into()],
            ..CleanPolicy::default()
        };
        let plan = clean::plan(&words, &custom, CleanMode::FullMix).unwrap();
        view.adopt(plan.clone());
        assert_eq!(view.policy(), custom);
        assert_eq!(view.plan, Some(plan));
    }
}

/// Keep mounted while closed so unsaved plan state and async completion survive.
#[component]
pub fn CleanPanel(studio: Studio) -> Element {
    let mut data = use_signal(View::default);
    use_effect(move || {
        let source = (studio.source_audio)();
        data.with_mut(|view| {
            if view.source != source {
                if let Some(control) = &view.recognition_control {
                    control.cancel();
                }
                let epoch = view.epoch.saturating_add(1);
                *view = View {
                    source,
                    epoch,
                    ..View::default()
                };
            }
        });
    });
    use_drop(move || {
        if let Some(control) = &data.peek().recognition_control {
            control.cancel();
        }
    });
    use_effect(move || {
        let source = (studio.source_audio)();
        let ranges = {
            let view = data.read();
            if view.source == source {
                view.plan
                    .as_ref()
                    .and_then(|plan| clean::merged_ranges(plan).ok())
                    .unwrap_or_default()
            } else {
                Vec::new()
            }
        };
        let mut shared = studio;
        shared.clean_ranges.set(ranges);
    });
    let view = data.read().clone();
    let policy = view.policy();
    let can_build = view.words.is_some() && policy.validate().is_ok();
    let plan_valid = view
        .plan
        .as_ref()
        .is_some_and(|plan| plan.validate().is_ok());
    let ranges = view
        .plan
        .as_ref()
        .map(|plan| plan.ranges.clone())
        .unwrap_or_default();
    let merged = view
        .plan
        .as_ref()
        .and_then(|plan| clean::merged_ranges(plan).ok())
        .unwrap_or_default();
    let ready = plan_valid
        && !merged.is_empty()
        && view.output().is_some()
        && (view.mode == CleanMode::FullMix
            || (view.stem.is_some()
                && view.stem_hashes.is_some()
                && !view.evidence.trim().is_empty()));
    let next_step = if !plan_valid || merged.is_empty() {
        "Find words and enable at least one valid mute range."
    } else if view.mode == CleanMode::VocalOnly && view.stem_hashes.is_none() {
        "Choose a vocal stem, or switch to full mix."
    } else if view.mode == CleanMode::VocalOnly && view.evidence.trim().is_empty() {
        "Add a note about how you checked the stem timing."
    } else {
        "Choose a save location and a new folder name."
    };
    let source = view.source.clone();
    let epoch = view.epoch;
    let busy = view.busy.is_some();
    if !(studio.clean_open)() {
        return rsx! {};
    }
    let has_loop = studio
        .snapshot()
        .is_some_and(|project| project.transport.loop_range.is_some());
    rsx! {
        style{"{CSS}"}
        super::sheet::ToolSheet {
        id: "clean-dialog", title: "Clean audio", description: "Mute selected words in a separate audio copy.",
        class: "clean-modal", close_label: "Close clean audio", onclose: move |_| { let mut s = studio; s.clean_open.set(false); },
        actions: rsx! {
                    p{class:"field-hint","Clean edits aren’t autosaved. Save a plan or export before quitting."}
                    div{class:"clean-actions",
                        button{class:"action-secondary",disabled:!has_loop,onclick:move|_|studio.dispatch(Action::SetLoop{range:None}),"Clear audition loop"}
                        button{class:"action-secondary",onclick:move|_|{let mut s=studio;s.clean_open.set(false);},"Close"}
                        button{class:"action-primary",disabled:busy||!ready,aria_describedby: if !busy && !ready { Some("clean-next-step") } else { None },onclick:move|_|{if let Some(audio)=(studio.source_audio)(){render(studio,data,audio,epoch);}},"Export clean audio"}
                    }
                 },

                    if let Some(path)=&source{p{class:"clean-path",title:"{path.display()}","{path.file_name().unwrap_or_default().to_string_lossy()}"}}else{p{"Open a song project first."}}
                    if let Some(label)=&view.busy{p{class:"clean-status",role:"status","{label}"}}
                    if let Some(control)=&view.recognition_control{button{class:"action-secondary",disabled:control.is_cancelled(),onclick:{let control=control.clone();move|_|{control.cancel();data.write().busy=Some("Cancelling automatic analysis…".into());}},"Cancel automatic analysis"}}
                    fieldset{class:"clean-fieldset",disabled:busy||source.is_none(),
                        div{class:"form-section",
                            h3{class:"clean-step","1. Find words"}
                            div{class:"clean-actions",
                                button{class:"action-primary",disabled:policy.validate().is_err(),onclick:move|_|{if let Some(audio)=(studio.source_audio)(){detect_automatic(studio,data,audio,epoch);}},"Find explicit words"}
                                if plan_valid { button{class:"action-secondary",onclick:move|_|{if let Some(audio)=(studio.source_audio)(){save_plan(studio,data,audio,epoch);}},"Save plan…"} }
                            }
                            p{class:"field-hint","English only · runs locally · 190 MB model download on first use. Finding again replaces your ranges."}
                            details { summary { "Use saved files" }
                            div { class: "clean-actions",
                                button{"data-browser-pick":"file","data-browser-accept":".json",class:"action-secondary",onclick:move|_|{if let Some(audio)=(studio.source_audio)(){load_words(studio,data,audio,epoch);}},"Load word timings…"}
                                button{"data-browser-pick":"file","data-browser-accept":".json",class:"action-secondary",onclick:move|_|{if let Some(audio)=(studio.source_audio)(){load_plan(studio,data,audio,epoch);}},"Load clean plan…"}
                            }
                            }
                            if let Some(recognition)=&view.recognition{details{summary{"Recognized lyrics · automatic estimates"}p{class:"field-hint","{clean_recognition::RECOGNITION_WARNING}"}p{class:"field-hint","{recognition.words.words.iter().filter_map(|word|word.text.as_deref()).collect::<Vec<_>>().join(\" \")}"}}}
                            if let Some(words)=&view.words{p{class:"field-hint","{words.words.len()} words · {words.words.iter().filter(|word|word.text.is_none()).count()} unknown · {words.words.iter().filter(|word|word.start.is_none()).count()} untimed"}}
                            if let Some(path)=&view.word_path{p{class:"clean-path","{path.display()}"}}
                        }
                        details{class:"form-section clean-options",
                            summary{"Words to mute and timing"}
                            details { summary { "About the word list" } p{class:"field-hint","{clean::DEFAULT_POLICY_NOTE}"} }
                            button{class:"action-secondary",onclick:move|_|data.with_mut(|view|{let policy=CleanPolicy::default();view.name=policy.name;view.terms=policy.terms.join("\n");}),"Use common explicit markers"}
                            div{class:"field-stack",label{class:"field-label",r#for:"clean-policy-name","Policy name"}input{id:"clean-policy-name",class:"field-control",value:view.name.clone(),oninput:move|event|data.write().name=event.value()}}
                            div{class:"field-stack",label{class:"field-label",r#for:"clean-policy-terms","One exact word or phrase per line"}textarea{id:"clean-policy-terms",class:"field-control",rows:"4",value:view.terms.clone(),placeholder:"Add or remove explicit words and phrases for this recording.",oninput:move|event|data.write().terms=event.value()}}
                            div{class:"clean-columns",
                                div{class:"field-stack",label{class:"field-label",r#for:"clean-pad-before","Padding before (s)"}input{id:"clean-pad-before",class:"field-control",r#type:"number",min:"0",step:"0.005",value:view.padding_before.to_string(),oninput:move|event|{if let Ok(value)=event.value().parse(){data.write().padding_before=value;}}}}
                                div{class:"field-stack",label{class:"field-label",r#for:"clean-pad-after","Padding after (s)"}input{id:"clean-pad-after",class:"field-control",r#type:"number",min:"0",step:"0.005",value:view.padding_after.to_string(),oninput:move|event|{if let Ok(value)=event.value().parse(){data.write().padding_after=value;}}}}
                                div{class:"field-stack",label{class:"field-label",r#for:"clean-fade","Fade outside range (s)"}input{id:"clean-fade",class:"field-control",r#type:"number",min:"0",step:"0.005",value:view.fade.to_string(),oninput:move|event|{if let Ok(value)=event.value().parse(){data.with_mut(|view|{view.fade=value;if let Some(plan)=&mut view.plan{plan.policy.fade_seconds=value;}});}}}}
                            }
                            button{class:"action-secondary",disabled:!can_build,onclick:move|_|{if let Some(audio)=(studio.source_audio)(){build(studio,data,audio,epoch);}},"Rebuild mute ranges"}
                            p{class:"field-hint","Rebuilding replaces manual range edits."}
                            if view.plan.as_ref().is_some_and(|plan|plan.policy!=policy){p{class:"clean-warning","Word list or timing changed. Rebuild ranges to apply."}}
                        }
                        div{class:"form-section",
                            div{class:"form-heading","Audio mode"}
                            div{class:"field-flags",
                                label{class:"form-check",input{r#type:"radio",name:"clean-mode",checked:view.mode==CleanMode::VocalOnly,onchange:move|_|data.with_mut(|view|{view.mode=CleanMode::VocalOnly;if let Some(plan)=&mut view.plan{plan.mode=CleanMode::VocalOnly;}})},"Vocal-only · aligned stem"}
                                label{class:"form-check",input{r#type:"radio",name:"clean-mode",checked:view.mode==CleanMode::FullMix,onchange:move|_|data.with_mut(|view|{view.mode=CleanMode::FullMix;if let Some(plan)=&mut view.plan{plan.mode=CleanMode::FullMix;}})},"Full mix · instruments muted too"}
                            }
                            if view.mode==CleanMode::VocalOnly{
                                p{class:"field-hint","Needs an aligned vocal stem. Poor alignment can leave words or damage instruments."}
                                button{"data-browser-pick":"file","data-browser-accept":"audio/*,.wav,.flac,.ogg,.mp3,.m4a",class:"action-secondary",onclick:move|_|{if let Some(audio)=(studio.source_audio)(){choose_stem(studio,data,audio,epoch);}},"Choose aligned vocal stem…"}
                                if let Some(path)=&view.stem{p{class:"clean-path","{path.display()}"}}
                                div{class:"field-stack",label{class:"field-label",r#for:"clean-alignment-notes","How did you check stem timing?"}textarea{id:"clean-alignment-notes",class:"field-control",rows:"2",value:view.evidence.clone(),placeholder:"Stem source and timing check",oninput:move|event|data.write().evidence=event.value()}}
                                details { summary { "Stem requirements" } p{class:"field-hint","Use a full-length, aligned vocal stem. Matching duration alone is not enough. Files are verified by hash. Vocal-only mode never falls back to muting the full mix."} }
                            }else{p{class:"clean-warning","Full-mix mode silences accompaniment and voice together in each selected range."}}
                        }
                        if let Some(plan)=&view.plan{
                            div{class:"form-section",
                                h3{class:"clean-step","2. Listen and adjust"}
                                p{class:"field-hint","{plan.matches.len()} matches · {plan.matches.iter().filter(|found|found.start.is_none()).count()} unresolved matches · {plan.unknown_words} unknown words · {plan.untimed_words} untimed words"}
                                if plan.unknown_words>0||plan.untimed_words>0||plan.matches.iter().any(|found|found.start.is_none()){p{class:"clean-warning","Missing words/times have no invented ranges. Review transcript gaps before accepting any clean version."}}
                                div{class:"clean-range-scroll",table{class:"clean-ranges",
                                    thead{tr{th{"Use"}th{"Start (s)"}th{"End (s)"}th{"Reason / edit"}th{"Actions"}}}
                                    tbody{for (index,range) in ranges.iter().enumerate(){tr{key:"clean-range-{index}",
                                        td{input{r#type:"checkbox",aria_label:"Enable range {index}",checked:range.enabled,onchange:move|event|set_range(data,index,|range|range.enabled=event.checked())}}
                                        td{input{class:"field-control",r#type:"number",aria_label:"Range {index} start seconds",min:"0",step:"0.01",value:range.start.to_string(),oninput:move|event|{if let Ok(value)=event.value().parse(){set_range(data,index,|range|range.start=value);}}}}
                                        td{input{class:"field-control",r#type:"number",aria_label:"Range {index} end seconds",min:"0",step:"0.01",value:range.end.to_string(),oninput:move|event|{if let Ok(value)=event.value().parse(){set_range(data,index,|range|range.end=value);}}}}
                                        td{input{class:"field-control clean-reason",aria_label:"Range {index} reason",value:range.reason.clone(),oninput:move|event|set_range(data,index,|range|range.reason=event.value())}}
                                        td{div{class:"clean-actions",
                                            button{class:"action-secondary",disabled:range.start>=range.end,onclick:move|_|{if let Some(range)=data.read().plan.as_ref().and_then(|plan|plan.ranges.get(index)).cloned(){studio.dispatch(Action::SetLoop{range:Some(TimeRange{start:range.start,end:range.end})});studio.dispatch(Action::Seek{seconds:range.start});studio.dispatch(Action::Playback{playing:true});}},"Listen source"}
                                            button{class:"action-secondary",aria_label:"Remove range {index}",onclick:move|_|{if let Some(plan)=&mut data.write().plan&&index<plan.ranges.len(){plan.ranges.remove(index);}},"Remove"}
                                        }}
                                    }}}
                                }}
                                button{class:"action-secondary",onclick:move|_|{if let Some(plan)=&mut data.write().plan{plan.ranges.push(MuteRange{start:0.,end:plan.duration_seconds.min(0.25),enabled:true,reason:"Manual selection; review timing".into()});}},"Add manual range"}
                                p{class:"field-hint","Listen source loops the original. Check the exported audio separately."}
                                if let Err(error)=plan.validate(){p{class:"clean-error",role:"alert","Fix before saving/exporting: {error}"}}
                                details{summary{"Match provenance and warnings"}for found in &plan.matches{p{class:"field-hint","{found.term:?} · words {found.first_word}..{found.end_word_exclusive} · {found.start:?}–{found.end:?} s"}}for warning in &plan.warnings{p{class:"field-hint","{warning}"}}}
                            }
                        }else{div { class: "form-section", h3 { class: "clean-step", "2. Listen and adjust" } p { class: "field-hint", "Mute ranges appear after finding words." } }}
                        div{class:"form-section",
                            h3{class:"clean-step","3. Export a copy"}
                            div{class:"field-row",
                                if !cfg!(feature="browser") {button{class:"action-secondary",onclick:move|_|{if let Some(audio)=(studio.source_audio)(){choose_parent(data,audio,epoch);}},"Choose save location…"}}
                                div{class:"field-stack field-half",label{class:"field-label",r#for:"clean-output-name","New folder name"}input{id:"clean-output-name",class:"field-control",value:view.output_name.clone(),oninput:move|event|data.write().output_name=event.value()}}
                            }
                            if !cfg!(feature="browser") && let Some(path)=view.output(){p{class:"clean-path","{path.display()}"}}
                            p{class:"field-hint","Choose a new folder. Original files stay untouched."}
                            details { summary { "Export details" } p{class:"field-hint","Saves clean.wav (44.1 kHz stereo float), plan.json and receipt.json. Needs an enabled, valid mute range. Vocal-only also needs a vocal stem and timing notes."} }
                        }
                    }
                    if let Some(error)=&view.error{p{class:"clean-error",role:"alert","{error}"}}
                    if !view.status.is_empty(){p{class:"clean-status",role:"status","{view.status}"}}
                    if !busy && !ready { p { id: "clean-next-step", class: "clean-status", role: "status", "{next_step}" } }
                    if let Some((receipt,path))=&view.receipt{details{summary{"Last export receipt"}p{class:"clean-path","{path.display()}"}p{class:"field-hint","{receipt.rendered_ranges.len()} ranges · {receipt.unresolved_matches} unresolved matches · {receipt.frames} stereo frames · peak {receipt.output_peak}"}for warning in &receipt.warnings{p{class:"field-hint","{warning}"}}}}

    }
    }
}
