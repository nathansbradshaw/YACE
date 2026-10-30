//! Read-only diagnostics. Note edits and review decisions remain in the editor.
use super::dialogs as rfd;
use super::{icons::Icon, state::Studio};
use crate::{
    editor::{Action, Project, Store, TimeRange},
    part_analysis::{self, AnalysisReport, FindingStage, PartReport, SectionReport},
    stem_review::{MixMode, StemId, StemMix},
};
use dioxus::prelude::*;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, PartialEq, Eq)]
struct RequestKey {
    root: PathBuf,
    content: String,
}
#[derive(Clone)]
struct Cached {
    key: RequestKey,
    report: Arc<AnalysisReport>,
}
#[derive(Clone)]
struct KitCache {
    key: RequestKey,
    set: Arc<part_analysis::DrumPartSet>,
}
#[derive(Clone, Copy)]
struct KitUi {
    cached: Signal<Option<KitCache>>,
    busy: Signal<bool>,
    error: Signal<Option<(RequestKey, String)>>,
    split_epoch: Signal<u64>,
    restore_attempt: Signal<Option<(RequestKey, u64)>>,
    restoring: Signal<Option<(RequestKey, u64)>>,
}
fn restore_kit(studio: Studio, mut kit: KitUi, key: RequestKey, epoch: u64) {
    let Some(store) = studio.store.read().clone() else {
        return;
    };
    let Some(project) = studio.snapshot() else {
        return;
    };
    kit.restore_attempt.set(Some((key.clone(), epoch)));
    kit.restoring.set(Some((key.clone(), epoch)));
    spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            part_analysis::load_project_drum_parts(&store, &project)
                .map_err(|error| format!("{error:#}"))
        })
        .await;
        if kit.restoring.peek().as_ref() == Some(&(key.clone(), epoch)) {
            kit.restoring.set(None);
        }
        // A deliberate split wins, even if it starts and finishes while this
        // disk-only lookup is running. Never replace its newer cached receipt.
        if *kit.split_epoch.peek() != epoch
            || *kit.busy.peek()
            || kit_key(studio).as_ref() != Some(&key)
            || kit
                .cached
                .peek()
                .as_ref()
                .is_some_and(|cache| cache.key == key)
        {
            return;
        }
        match result {
            Ok(Ok(Some(set))) if set.source_key == key.content => {
                kit.cached.set(Some(KitCache {
                    key,
                    set: Arc::new(set),
                }));
            }
            Ok(Ok(Some(_))) => kit
                .error
                .set(Some((key, "Saved kit belongs to different audio.".into()))),
            Ok(Ok(None)) => {}
            Ok(Err(message)) => kit.error.set(Some((key, message))),
            Err(message) => kit.error.set(Some((key, message.to_string()))),
        }
    });
}
fn kit_key(studio: Studio) -> Option<RequestKey> {
    let root = studio.store.read().as_ref()?.root.clone();
    let project = studio.doc.read();
    let content = part_analysis::kit_source_key(project.as_ref()?).ok()?;
    Some(RequestKey { root, content })
}
/// Picker hint only. The worker verifies every model before any inference.
fn kit_model_directory() -> Option<PathBuf> {
    let mut candidates = vec![crate::generation::default_model_dir().join("larsnet")];
    if let Ok(executable) = std::env::current_exe()
        && let Some(parent) = executable.parent()
    {
        candidates.extend(parent.ancestors().map(|path| path.join("models/larsnet")));
    }
    candidates.into_iter().find(|directory| {
        directory.is_dir()
            && crate::larsnet::MODELS
                .iter()
                .all(|(part, _)| directory.join(format!("{part}.onnx")).is_file())
    })
}
fn split_kit(studio: Studio, mut kit: KitUi) {
    let Some(key) = kit_key(studio) else { return };
    let Some(store) = studio.store.read().clone() else {
        return;
    };
    let Some(project) = studio.snapshot() else {
        return;
    };
    if (kit.busy)() || studio.kit_audition_busy.read().is_some() {
        return;
    }
    kit.busy.set(true);
    kit.split_epoch.set((kit.split_epoch)().wrapping_add(1));
    kit.error.set(None);
    spawn(async move {
        let directory = tokio::task::spawn_blocking(kit_model_directory)
            .await
            .ok()
            .flatten();
        let mut picker = rfd::AsyncFileDialog::new()
            .set_title("Choose local LarsNet ONNX models (CC-BY-NC-4.0)");
        if let Some(directory) = directory {
            // Show the model folder itself so it can be selected immediately.
            picker = picker.set_directory(directory.parent().unwrap_or(&directory));
        }
        let Some(folder) = picker.pick_folder().await else {
            kit.busy.set(false);
            return;
        };
        if kit_key(studio).as_ref() != Some(&key) {
            kit.busy.set(false);
            return;
        }
        let model_dir = folder.path().to_owned();
        let result = tokio::task::spawn_blocking(move || {
            part_analysis::prepare_project_drum_parts(&store, &project, &model_dir)
                .map_err(|e| format!("{e:#}"))
        })
        .await;
        kit.busy.set(false);
        if kit_key(studio).as_ref() != Some(&key) {
            return;
        }
        match result {
            Ok(Ok(set)) if set.source_key == key.content => {
                kit.cached.set(Some(KitCache {
                    key,
                    set: Arc::new(set),
                }));
            }
            Ok(Ok(_)) => kit
                .error
                .set(Some((key, "Drum source changed. Split again.".into()))),
            Ok(Err(message)) => kit.error.set(Some((key, message))),
            Err(message) => kit.error.set(Some((key, message.to_string()))),
        }
    });
}
fn kit_panel(studio: Studio, kit: KitUi) -> Element {
    let key = kit_key(studio);
    let cache = kit
        .cached
        .read()
        .as_ref()
        .filter(|c| Some(&c.key) == key.as_ref())
        .cloned();
    let project = studio.snapshot();
    let selected = project.as_ref().and_then(|p| p.kit_audition.as_ref());
    let playing = project.as_ref().is_some_and(|p| p.transport.playing) && (studio.audio_ready)();
    let pending = (studio.kit_audition_busy)();
    let failed = studio.error.read().is_some() && !(studio.audio_ready)();
    let error = kit
        .error
        .read()
        .as_ref()
        .filter(|(k, _)| Some(k) == key.as_ref())
        .map(|(_, e)| e.clone());
    let source_available = studio
        .doc
        .read()
        .as_ref()
        .and_then(|p| p.content.stems.as_ref())
        .is_some_and(|m| m.stems.iter().any(|s| s.id == StemId::Drums));
    let has_cache = cache.is_some();
    let restoring = kit
        .restoring
        .read()
        .as_ref()
        .is_some_and(|(k, _)| Some(k) == key.as_ref());
    rsx! {details{class:"part-analysis-kit",open:has_cache || (kit.busy)(),
        summary{"Kit isolation"}
        div{class:"part-analysis-kit-heading",
            span{class:"part-analysis-caption",if restoring && !(kit.busy)(){"Loading saved kit…"}else{"Five estimated parts"}}
            button{class:"studio-tool",disabled:!source_available || (kit.busy)() || pending.is_some(),title:"Choose your local LarsNet ONNX models. CC-BY-NC-4.0 weights; no download or note changes.",onclick:move |_|split_kit(studio,kit),if (kit.busy)(){"Splitting…"}else if has_cache{"Split again…"}else{"Split kit…"}}
        }
        p{class:"part-analysis-caption","Local LarsNet models · CC-BY-NC-4.0. Audio only; notes stay unchanged."}
        if !source_available{p{class:"part-analysis-caption","Attach aligned drum audio first."}}
        if let Some(error)=error{p{class:"part-analysis-kit-error",role:"alert","{error}"}}
        if let Some(cache)=cache{
            for part in &cache.set.parts{
                {let id=part.id.clone();let active=selected.is_some_and(|s|s.part==id);let failed=active && failed && pending.is_none();let loading=pending.as_deref()==Some(id.as_str()) || (active && !(studio.audio_ready)() && !failed);let expected=cache.key.clone();let icon=if active && playing{"pause"}else{"play"};let action=if loading{"Loading…"}else if failed{"Retry"}else if active && playing{"Pause"}else if active{"Play"}else{"Listen"};
                rsx!{div{key:"{part.id}",class:"part-analysis-kit-row",
                    span{"{part.label}"}
                    button{class:if active{"studio-tool active"}else{"studio-tool"},aria_label:"{action} {part.label}",aria_pressed:active.to_string(),disabled:(kit.busy)() || pending.is_some() || (active && !(studio.audio_ready)() && !failed),title:"Play this component on the shared chart clock; seek, loop and speed controls stay synchronized",onclick:move |_|{if kit_key(studio).as_ref()==Some(&expected){if active && (studio.audio_ready)(){studio.toggle_playback();}else{studio.audition_kit_part(id.clone(),None);}}},Icon{name:icon,size:12}"{action}"}
                }}}
            }
            if let Some(selected)=selected{
                div{class:"part-analysis-kit-transport",role:"status",span{"{selected.label} · shared playback"}KitTransportClock{studio}}
            }
            p{class:"part-analysis-caption","Estimates may bleed. Toms stay grouped; ride/crash stay combined."}
            details{class:"part-analysis-details",summary{"Audio provenance"}
                p{"Recording: {cache.set.original_audio_sha256}"}p{"Drums: {cache.set.drum_audio_sha256}"}
                p{"Saved audio: {cache.set.directory.display()}"}
                for warning in &cache.set.receipt.warnings{p{"{warning}"}}
            }
        }
    }}
}

#[component]
fn KitTransportClock(studio: Studio) -> Element {
    // Keep frame-clock updates out of the diagnostic report's render scope.
    rsx! {time{"{super::state::clock((studio.playhead)())}"}}
}

fn snapshot(studio: Studio) -> Option<(Store, std::sync::Arc<Project>, RequestKey)> {
    let store = studio.store.read().clone()?;
    let project = studio.snapshot()?;
    let content = part_analysis::project_key(&project).ok()?;
    let key = RequestKey {
        root: store.root.clone(),
        content,
    };
    Some((store, project, key))
}
fn start(
    studio: Studio,
    mut busy: Signal<bool>,
    mut cached: Signal<Option<Cached>>,
    mut error: Signal<Option<String>>,
) {
    let Some((store, project, requested)) = snapshot(studio) else {
        return;
    };
    if busy() {
        return;
    }
    busy.set(true);
    error.set(None);
    spawn(async move {
        let expected = requested.clone();
        let result = tokio::task::spawn_blocking(move || {
            part_analysis::analyze_project(&store, &project).map_err(|e| format!("{e:#}"))
        })
        .await;
        busy.set(false);
        // Navigation and listening can continue while analysis runs.
        // Reject the result after edits, stem replacement or a project switch.
        if snapshot(studio).is_none_or(|(_, _, key)| key != requested) {
            return;
        }
        match result {
            Ok(Ok(report)) if report.project_key.as_deref() == Some(expected.content.as_str()) => {
                cached.set(Some(Cached {
                    key: requested,
                    report: Arc::new(report),
                }));
            }
            Ok(Ok(_)) => error.set(Some("Analysis snapshot changed. Recheck.".into())),
            Ok(Err(message)) => error.set(Some(message)),
            Err(message) => error.set(Some(message.to_string())),
        }
    });
}
fn count(value: Option<usize>) -> String {
    value.map_or_else(|| "—".into(), |value| value.to_string())
}
fn timestamp(seconds: f64) -> String {
    let value = seconds.max(0.).floor() as u64;
    format!("{}:{:02}", value / 60, value % 60)
}
fn precise_timestamp(seconds: f64) -> String {
    let tenths = (seconds.max(0.) * 10.).round() as u64;
    format!("{}:{:02}.{}", tenths / 600, tenths / 10 % 60, tenths % 10)
}
fn stage_label(stage: &FindingStage) -> &'static str {
    match stage {
        FindingStage::AudioOnly => "Audio",
        FindingStage::Filtered => "Filtered",
        FindingStage::Mapping => "Mapping",
        FindingStage::Articulation => "Kit",
    }
}
fn band_label(id: &str) -> &str {
    match id {
        "low_30_120_hz" => "Low",
        "body_120_500_hz" => "Body",
        "attack_2000_6000_hz" => "Attack",
        "metal_6000_16000_hz" => "Metal",
        _ => id,
    }
}
fn jump(mut studio: Studio, track: String, start: f64, end: f64, looping: bool) {
    if studio
        .snapshot()
        .is_some_and(|p| p.content.tracks.contains_key(&track))
    {
        studio.dispatch(Action::SelectTrack { track });
    }
    if looping && end > start {
        studio.dispatch(Action::SetLoop {
            range: Some(TimeRange { start, end }),
        });
    } else if studio.snapshot().is_some_and(|project| {
        project
            .transport
            .loop_range
            .is_some_and(|range| start < range.start || end > range.end)
    }) {
        // A previous section's loop must not pull this jump back out of view.
        studio.dispatch(Action::SetLoop { range: None });
    }
    studio.seek(start);
    let duration = studio.snapshot().map_or(end, |p| p.duration);
    let span = (end - start + 2.).clamp(4., 120.);
    studio.span.set(span);
    studio
        .view_start
        .set((start - 0.5).max(0.).min((duration - span).max(0.)));
}
fn audition(studio: Studio, id: StemId, isolated: bool) {
    let Some(project) = studio.snapshot() else {
        return;
    };
    let mut mix = if isolated {
        StemMix {
            mode: MixMode::Stems,
            ..Default::default()
        }
    } else {
        project.content.stem_mix.clone()
    };
    if isolated {
        let Some(stems) = project.content.stems.as_ref() else {
            return;
        };
        if !stems.stems.iter().any(|s| s.id == id) {
            return;
        }
        for stem in &stems.stems {
            let channel = mix.channels.entry(stem.id).or_default();
            channel.gain = if stem.id == id { 1. } else { 0. };
            channel.solo = false;
            channel.mute = false;
        }
    } else {
        mix.mode = MixMode::Original;
    }
    studio.dispatch(Action::SetStemMix { mix });
}
fn save_report(
    studio: Studio,
    expected: RequestKey,
    report: Arc<AnalysisReport>,
    mut error: Signal<Option<String>>,
) {
    spawn(async move {
        let Some(file) = rfd::AsyncFileDialog::new()
            .set_title("Save YACE part analysis")
            .set_file_name("part-analysis.json")
            .add_filter("JSON", &["json"])
            .save_file()
            .await
        else {
            return;
        };
        if snapshot(studio).is_none_or(|(_, _, current)| current != expected) {
            error.set(Some(
                "Chart or source changed. Recheck before saving.".into(),
            ));
            return;
        }
        let path = file.path().to_owned();
        match tokio::task::spawn_blocking(move || {
            crate::generation::write_json(&path, report.as_ref())
        })
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(message)) => error.set(Some(format!("{message:#}"))),
            Err(message) => error.set(Some(message.to_string())),
        }
    });
}

#[component]
pub fn PartAnalysisButton(studio: Studio) -> Element {
    let mut open = use_signal(|| false);
    let busy = use_signal(|| false);
    let cached = use_signal(|| None::<Cached>);
    let error = use_signal(|| None::<String>);
    let mut selected = use_signal(|| StemId::Guitar);
    let mut section_id = use_signal(|| None::<String>);
    let kit = KitUi {
        cached: use_signal(|| None),
        busy: use_signal(|| false),
        error: use_signal(|| None),
        split_epoch: use_signal(|| 0),
        restore_attempt: use_signal(|| None),
        restoring: use_signal(|| None),
    };
    let kit_context = use_memo(move || {
        if open() && selected() == StemId::Drums {
            kit_key(studio)
        } else {
            None
        }
    });
    use_effect(move || {
        // Subscribe only to source, tab, open and split changes.
        // Peek at restore progress so an empty result cannot trigger endless retries.
        let context = kit_context();
        let splitting = (kit.busy)();
        let epoch = (kit.split_epoch)();
        let Some(key) = context else { return };
        if splitting
            || kit
                .cached
                .peek()
                .as_ref()
                .is_some_and(|cache| cache.key == key)
        {
            return;
        }
        if kit.restore_attempt.peek().as_ref() == Some(&(key.clone(), epoch)) {
            return;
        }
        restore_kit(studio, kit, key, epoch);
    });
    let current = use_memo(move || {
        if open() {
            snapshot(studio).map(|(_, _, key)| key)
        } else {
            None
        }
    });
    let cache = cached.read().clone();
    let fresh = cache
        .as_ref()
        .is_some_and(|c| current().as_ref() == Some(&c.key));
    let report = cache.as_ref().filter(|_| fresh).map(|c| c.report.clone());
    let part = report
        .as_ref()
        .and_then(|r| {
            r.parts
                .iter()
                .find(|p| p.id == selected())
                .or_else(|| r.parts.first())
        })
        .cloned();
    let active = part
        .as_ref()
        .and_then(|p| {
            p.sections
                .iter()
                .find(|s| Some(&s.id) == section_id.read().as_ref())
        })
        .cloned();
    let button_disabled = studio.store.read().is_none() || studio.doc.read().is_none();
    let playback_icon = if studio.snapshot().is_some_and(|p| p.transport.playing) {
        "pause"
    } else {
        "play"
    };
    rsx! {
        button {class:if open(){"studio-tool active"}else{"studio-tool"},disabled:button_disabled,
            title:"Find where source attacks, saved detections and chart notes differ",aria_expanded:open().to_string(),
            onclick:move |_|{open.set(!open());if open(){
                if let Some(project)=studio.snapshot(){
                    let instrument=if project.selected_track.ends_with("Drums"){Some(StemId::Drums)}else if project.selected_track.ends_with("DoubleBass"){Some(StemId::Bass)}else if project.selected_track.ends_with("Single"){Some(StemId::Guitar)}else{None};
                    if let Some(instrument)=instrument && instrument!=selected(){selected.set(instrument);section_id.set(None);}
                }
                let valid=cached.read().as_ref().is_some_and(|c|snapshot(studio).is_some_and(|(_,_,key)|key==c.key));if !valid{start(studio,busy,cached,error);}
            }},
            Icon{name:"search",size:13} "Part analysis"
        }
        if open() {
            aside {class:"part-analysis-drawer",role:"dialog",aria_label:"Part analysis",
                onkeydown:move|event:KeyboardEvent|{if event.key()==Key::Escape{open.set(false);}event.stop_propagation();},
                div {class:"part-analysis-header",
                    div {class:"part-analysis-heading",Icon{name:"search",size:16}"Part analysis"}
                    button {class:"studio-tool",disabled:busy(),title:"Recheck the current saved chart and bound source",onclick:move |_|start(studio,busy,cached,error),if busy(){"Checking…"}else{"Recheck"}}
                    button {class:"part-analysis-icon-button",disabled:!fresh,aria_label:"Save analysis JSON",title:"Save the same source-bound report available to AI and CLI tools",onclick:move |_|{if let Some(c)=cached.read().as_ref(){save_report(studio,c.key.clone(),c.report.clone(),error);}},Icon{name:"export",size:16}}
                    button {class:"part-analysis-icon-button",aria_label:"Close part analysis",title:"Close",onclick:move |_|open.set(false),Icon{name:"close",size:16}}
                }
                if let Some(message)=error(){div{class:"part-analysis-notice error",role:"alert",span{"{message}"}}}
                if cache.is_some() && !fresh {div{class:"part-analysis-notice",span{"Chart or source changed."}}}
                if let Some(report)=report {
                    div {class:"part-analysis-tabs",role:"tablist",aria_label:"Instrument",
                        for item in &report.parts {
                            {let id=item.id;let active=part.as_ref().is_some_and(|p|p.id==id);let icon=match id{StemId::Bass=>"bass",StemId::Drums=>"drums",_=>"guitar"};
                            rsx!{button{key:"{id:?}",role:"tab",aria_selected:active.to_string(),class:if active{"studio-tool active"}else{"studio-tool"},onclick:move |_|{selected.set(id);section_id.set(None);},Icon{name:icon,size:14}"{item.label}"}}}
                        }
                    }
                    if let Some(part)=part {
                        {let audition_id=studio.guitar_source.read().audition_stem(&part.track,false);
                        let audition_available=studio.doc.read().as_ref().and_then(|p|p.content.stems.as_ref()).is_some_and(|m|m.stems.iter().any(|s|s.id==audition_id));
                        rsx!{
                        div{class:"part-analysis-body",
                            {stage_row(report.duration,&part)}
                            div{class:"part-analysis-audition",span{class:"part-analysis-audio-label","Audition"}
                                button{class:"studio-tool",onclick:move |_|audition(studio,audition_id,false),"Original"}
                                button{class:"studio-tool",disabled:!audition_available,title:"Listen to the actual selected source stem",onclick:move |_|audition(studio,audition_id,true),"{audition_id:?} only"}
                                button{class:"part-analysis-icon-button",disabled:!(studio.audio_ready)(),aria_label:"Play or pause",title:"Play or pause",onclick:move |_|studio.toggle_playback(),Icon{name:playback_icon,size:15}}
                            }
                            if part.id==StemId::Drums{{kit_panel(studio,kit)}}
                            if let Some(section)=&active {
                                {section_findings(studio,part.track.clone(),section)}
                            }
                            div{class:"part-analysis-section-header",span{"Section"}span{title:"Independent source attack estimates","Audio"}span{title:"Available saved hypotheses; not the complete neural output","Saved"}span{title:"Retained detected attacks","Kept"}span{title:"Current Expert chart attacks","Chart"}span{title:"Diagnostic findings","Checks"}}
                            for section in &part.sections {
                                {let id=section.id.clone();let track=part.track.clone();let start=section.start;let end=section.end;let selected_row=section_id.read().as_ref()==Some(&id);
                                rsx!{button{key:"{section.id}",class:if selected_row{"part-analysis-section-row selected"}else{"part-analysis-section-row"},title:"Jump to {section.label}",onclick:move |_|{section_id.set(Some(id.clone()));jump(studio,track.clone(),start,end,false);},
                                    span{class:"part-analysis-section-name",strong{"{section.label}"}time{"{timestamp(start)} – {timestamp(end)}"}}
                                    span{class:"part-analysis-number","{count(section.counts.audio_attacks)}"}
                                    span{class:"part-analysis-number","{count(section.counts.saved_hypotheses)}"}
                                    span{class:"part-analysis-number","{count(section.counts.retained_attacks)}"}
                                    span{class:"part-analysis-number","{section.counts.chart_attacks}"}
                                    if !section.findings.is_empty(){span{class:"part-analysis-finding-count","{section.findings.len()}"}}else{span{}}
                                }}}
                            }
                            if part.sections.is_empty(){div{class:"part-analysis-section-empty","No sections available."}}
                            if !part.subparts.is_empty(){details{class:"part-analysis-subparts",summary{"Drum frequency views"}
                                p{class:"part-analysis-caption","Frequency bands, not isolated kit pieces."}
                                for band in &part.subparts {div{class:"part-analysis-band",title:"{band.label}: {band.interpretation}",span{"{band_label(&band.label)}"}span{"{band.attacks} attacks · RMS {band.rms:.4}"}}}
                            }}
                            details{class:"part-analysis-details",summary{"Evidence & limits"}
                                p{"Missing evidence is shown as —, not zero. Counts locate review candidates; they do not prove missing or correct notes."}
                                for limit in part.limits.iter().chain(&report.limits){p{"{limit}"}}
                                p{"Analysis: {report.algorithm} · {report.implementation_sha256}"}
                                p{"Recording: {report.source_audio_sha256}"}p{"Chart: {report.chart_sha256}"}
                                if let Some(hash)=&report.transcription_sha256{p{"Saved detections: {hash}"}}
                                for (id,hash) in &report.stem_sha256 {p{"{id}: {hash}"}}
                            }
                        }
                        }}
                    }
                } else {div{class:"part-analysis-empty",if busy(){"Checking source and chart…"}else{"Recheck to inspect this chart."}}}
            }
        }
    }
}

fn stage_row(duration: f64, part: &PartReport) -> Element {
    rsx! {div{class:"part-analysis-stage-row",aria_label:"Generation stages",
        Stage{label:"Recording",value:timestamp(duration),detail:"Original recording duration"}
        Stage{label:"Audio attacks",value:if part.source_available{count(part.counts.audio_attacks)}else{"—".into()},detail:"Independent attack estimates from the bound instrument stem; not note truth"}
        Stage{label:"Saved hypotheses",value:count(part.counts.saved_hypotheses),detail:"Available saved candidate hypotheses; the full network output may not have been retained"}
        Stage{label:"Retained attacks",value:count(part.counts.retained_attacks),detail:"Retained detected attacks before game mapping"}
        Stage{label:"Chart attacks",value:part.counts.chart_attacks.to_string(),detail:"Current Expert chart attack groups"}
    }}
}
#[component]
fn Stage(label: &'static str, value: String, detail: &'static str) -> Element {
    rsx! {div{class:"part-analysis-stage",title:detail,div{class:"part-analysis-stage-label","{label}"}div{class:if value=="—"{"part-analysis-stage-value unavailable"}else{"part-analysis-stage-value"},"{value}"}}}
}
fn section_findings(studio: Studio, track: String, section: &SectionReport) -> Element {
    let start = section.start;
    let end = section.end;
    let loop_track = track.clone();
    rsx! {div{class:"part-analysis-findings",
        div{class:"part-analysis-findings-header",strong{"{section.label}"}button{class:"studio-tool",title:"Loop this section without changing notes",onclick:move |_|jump(studio,loop_track.clone(),start,end,true),Icon{name:"loop",size:12}"Loop"}}
        if section.findings.is_empty(){p{class:"part-analysis-caption","No diagnostic differences flagged."}}
        else{details{key:"{track}:{section.id}",class:"part-analysis-finding-list",
            summary{"{section.findings.len()} checks"}
            div{class:"part-analysis-finding-items",
                for finding in &section.findings {
                    {let track=track.clone();let start=finding.start;let end=finding.end;
                    rsx!{details{key:"{finding.id}",class:"part-analysis-finding",
                        summary{time{class:"part-analysis-finding-time","{precise_timestamp(start)}–{precise_timestamp(end)}"}span{class:"part-analysis-finding-stage","{stage_label(&finding.stage)}"}span{"{finding.summary}"}}
                        div{class:"part-analysis-finding-detail",
                            button{class:"studio-tool",title:"Jump to this evidence window without changing notes",onclick:move |_|jump(studio,track.clone(),start,end,false),Icon{name:"search",size:12}"Jump"}
                            p{"{finding.detail}"}
                        }
                    }}}
                }
            }
        }}
    }}
}
