//! One phrase/all-vocals comparison and batch correction flow on both clients.
use super::{icons::Icon, state::Studio};
use crate::editor::{Action, TimeRange};
use dioxus::prelude::*;
#[cfg(not(target_arch = "wasm32"))]
type Stop = std::sync::Arc<crate::clean_recognition::Control>;
#[cfg(target_arch = "wasm32")]
type Stop = std::rc::Rc<std::cell::RefCell<Option<futures_channel::oneshot::Sender<()>>>>;
fn cancel(stop: &Stop) {
    #[cfg(not(target_arch = "wasm32"))]
    stop.cancel();
    #[cfg(target_arch = "wasm32")]
    if let Some(sender) = stop.borrow_mut().take() {
        let _ = sender.send(());
    }
}
#[component]
pub fn VocalPitchAnalyzer(mut studio: Studio, range: Option<TimeRange>) -> Element {
    let busy = use_signal(|| false);
    let mut status = use_signal(String::new);
    let stop = use_signal(|| None::<Stop>);
    let chosen = use_signal(|| (String::new(), std::collections::BTreeSet::<usize>::new()));
    let include_pitch = use_signal(|| false);
    let mut selection_start = use_signal(|| "0".to_string());
    let mut selection_end = use_signal(String::new);
    let mut range_open = use_signal(|| false);
    let mut adaptive = use_signal(|| false);
    use_drop(move || {
        if let Some(stop) = stop.read().as_ref() {
            cancel(stop);
        }
    });
    #[cfg(not(target_arch = "wasm32"))]
    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            if let Some(c) = stop.read().as_ref() {
                status.set(c.message());
            }
        }
    });
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let ready = doc.content.stems.as_ref().is_some_and(|m| {
        m.stems
            .iter()
            .any(|s| s.id == crate::stem_review::StemId::Vocals)
    });
    let timing = (studio.timing)();
    let custom_range = selection_start()
        .parse::<f64>()
        .ok()
        .zip(if selection_end().is_empty() {
            Some(doc.duration)
        } else {
            selection_end().parse::<f64>().ok()
        })
        .filter(|(a, b)| a.is_finite() && b.is_finite() && *a >= 0. && a < b && *b <= doc.duration)
        .map(|(start, end)| TimeRange { start, end });
    let analysis = doc.content.vocal_pitch_analysis.as_ref();
    let changes = analysis.map_or(0, |a| a.corrections.len());
    let change_label = if changes == 1 {
        "correction"
    } else {
        "corrections"
    };
    let valid = analysis
        .zip(timing.as_ref())
        .is_some_and(|(a, t)| a.validate(&doc, t).is_ok());
    rsx! {section{class:"vocal-pitch-analysis",aria_label:"Vocal pitch comparison",
        div{class:"form-heading","Check pitches against the recording"}
        p{class:"field-hint","Compare pitch, gaps, short sounds and note lengths. Preview section repairs and choose which to apply. New notes keep unknown lyrics."}
        p{class:"field-hint","Uses the High accuracy model (362 MB, downloaded once). Phrase checks use the full recording for context."}
        label{class:"field-hint",input{r#type:"checkbox",checked:adaptive(),disabled:busy(),onchange:move|e|adaptive.set(e.checked())} "Deep rescan gaps and disagreements (experimental, slower)"}
        if adaptive(){p{class:"field-hint","Rechecks smaller overlapping sections against vocal audio, without lyrics. Supported additions remain draft notes for review. Sound coverage does not guarantee correct pitches or singer identity."}}
        div{class:"vocal-file-actions",
            button{class:"studio-tool",disabled:busy()||!ready||range.is_none(),onclick:move |_|start(studio,range,busy,status,stop,adaptive()),Icon{name:"music",size:13}"Analyze phrase"}
            button{class:"studio-tool",disabled:busy()||!ready||doc.transport.loop_range.is_none(),onclick:{let loop_range=doc.transport.loop_range;move |_|start(studio,loop_range,busy,status,stop,adaptive())},"Analyze loop"}
            button{class:"studio-tool",disabled:busy()||!ready,onclick:move |_|start(studio,None,busy,status,stop,adaptive()),"Analyze all vocals"}
            if busy(){button{class:"studio-tool",onclick:move |_|{if let Some(s)=stop.read().as_ref(){cancel(s);}status.set("Cancelling comparison…".into());},"Cancel"}}
        }
        button{class:"studio-tool",aria_expanded:range_open(),onclick:move |_|range_open.set(!range_open()),"Choose time range"}
        if range_open(){div{class:"review-evidence",onkeydown:move|e|e.stop_propagation(),
            label{span{"Start (seconds)"}input{r#type:"number",min:"0",step:"0.01",aria_label:"Vocal analysis start seconds",value:selection_start(),oninput:move|e|selection_start.set(e.value())}}
            label{span{"End (seconds)"}input{r#type:"number",min:"0",step:"0.01",aria_label:"Vocal analysis end seconds",placeholder:"{doc.duration:.2}",value:selection_end(),oninput:move|e|selection_end.set(e.value())}}
            button{class:"studio-tool",disabled:busy()||!ready||custom_range.is_none(),onclick:move |_|start(studio,custom_range,busy,status,stop,adaptive()),"Analyze time range"}
            if custom_range.is_none(){p{class:"field-hint","Choose a start before the end, within the recording."}}
        }}
        if !ready{p{class:"field-hint","Attach a vocal stem to compare these notes with the recording."}}
        if busy(){p{role:"status","{status}"}}
        if let Some(a)=analysis {
            if let Some(deep)=&a.adaptive{p{role:"status","Deep review: {deep.passes.len()} scans · {deep.initial_residual_frames as f64*0.01:.2}s initial disagreement · {deep.remaining_residual_frames as f64*0.01:.2}s unresolved"}
                if deep.budget_exhausted{p{class:"field-hint","Processing limit reached. Unresolved sections remain flagged; select a shorter range to continue."}}
                if !deep.alternatives.is_empty(){details{class:"review-evidence",summary{"Pitch alternatives from rescans"}
                    p{class:"field-hint","Showing up to 100 of {deep.alternatives.len()} retained alternatives; {deep.omitted_alternatives} additional intervals exceeded the report limit."}
                    for alternative in deep.alternatives.iter().take(100){
                        {let before=alternative.before_midi.map(crate::vocal_pitch_analysis::note_name).unwrap_or_else(||"no supported pitch".into());let after=alternative.after_midi.map(crate::vocal_pitch_analysis::note_name).unwrap_or_else(||"no supported pitch".into());
                        rsx!{p{class:"field-hint","{alternative.start:.2}–{alternative.end:.2}s: {before} → {after}"}}}
                    }
                    p{class:"field-hint","Rescans use the same model in different contexts. Disagreement remains review evidence."}
                }}
            }
            if a.applied {p{role:"status","Applied vocal changes · {a.section.applied_repairs.len()} section repairs. Undo restores the previous notes."}}
            else if valid {
                p{role:"status","{a.inspected} notes checked · {changes} {change_label} · {a.unresolved} unresolved"}
                if !a.corrections.is_empty(){button{class:"studio-tool",disabled:busy(),onclick:move |_|studio.dispatch(Action::ApplyVocalPitchCorrections),"Apply supported corrections"}}
            }else{p{class:"field-hint","The recording, timing or vocal notes changed. Analyze again to refresh these proposals."}}
            if !a.corrections.is_empty(){details{class:"review-evidence",summary{"Pitch comparisons"}
                for c in &a.corrections {
                    {let number=c.index+1;let before=crate::vocal_pitch_analysis::note_name(c.before);let after=crate::vocal_pitch_analysis::note_name(c.after);let support=(c.support_fraction*100.).round()as usize;
                    let target=doc.content.vocals.as_ref().and_then(|chart|chart.notes.iter().position(|n|n.start==c.start&&n.end==c.end&&n.tone==crate::vocals::VocalTone::Pitched{midi:if a.applied{c.after}else{c.before}}));
                    rsx!{button{class:"vocal-check-row",disabled:target.is_none()||(!valid&&!a.applied),title:"{support}% of supported contour frames agree; model agreement is not a calibrated correctness probability",onclick:move |_|{if let Some(index)=target{studio.select_vocal(index,false,false);}},"Note {number}: {before} → {after} · {support}% contour support"}}}
                }
            }}
            {section_review(studio,a,timing.as_ref(),valid,busy(),chosen,include_pitch)}
        }
    }}
}

#[allow(clippy::too_many_arguments)]
fn section_review(
    studio: Studio,
    a: &crate::vocal_pitch_analysis::Analysis,
    timing: Option<&crate::timing::TempoMap>,
    valid: bool,
    busy: bool,
    mut chosen: Signal<(String, std::collections::BTreeSet<usize>)>,
    mut include_pitch: Signal<bool>,
) -> Element {
    let Some(timing) = timing else {
        return rsx! {};
    };
    if a.section.algorithm.is_empty() {
        return rsx! {};
    }
    let key = format!(
        "{}:{}:{}:{}:{}",
        a.generation_sha256, a.chart_sha256, a.range.start, a.range.end, a.section.algorithm
    );
    let selected = if chosen.read().0 == key {
        chosen.read().1.clone()
    } else {
        Default::default()
    };
    let voiced = format!("{:.2}", a.section.voiced_seconds);
    let gaps = format!("{:.2}", a.section.uncharted_seconds);
    let indices: Vec<_> = selected.iter().copied().collect();
    let can_apply =
        valid && !busy && (!indices.is_empty() || include_pitch() && !a.corrections.is_empty());
    rsx! {div{class:"vocal-section-review",
        div{class:"form-heading","Section audio comparison"}
        p{class:"field-hint","{voiced}s of model-supported voice · {gaps}s outside existing notes. Voice may include backing singers or leakage."}
        {contour_plot(studio,a,timing,&selected)}
        super::vocal_section_audio::VocalSectionAudio{studio,selected:indices.clone(),include_pitch:include_pitch()}
        p{class:"field-hint","Blue: detected vocal pitch · gray: current notes · orange: selected repairs. Play the vocal stem to check singer identity. Shortening and deletion remain review findings to protect missed or quiet singing."}
        if !a.applied&&valid {
            label{class:"form-check",input{r#type:"checkbox",checked:include_pitch(),onchange:move|e|include_pitch.set(e.checked())},"Include supported pitch corrections"}
            button{class:"studio-tool",disabled:!can_apply,onclick:move |_|studio.dispatch(Action::ApplyVocalSectionRepairs{indices:indices.clone(),include_pitch:include_pitch()}),"Apply selected repairs"}
        }
        if a.section.repairs.is_empty(){p{class:"field-hint","No section repairs meet the independent acoustic support rules. This does not certify every sung note."}}
        for (i,r) in a.section.repairs.iter().enumerate() {
            {let (start,end)=r.bounds();let start=timing.seconds(start);let end=timing.seconds(end);let label=r.kind.label();let time=format!("{start:.2}–{end:.2}s");
            let before=if r.before.is_empty(){"gap".into()}else{r.before.iter().filter_map(|n|match n.tone{crate::vocals::VocalTone::Pitched{midi}=>Some(crate::vocal_pitch_analysis::note_name(midi)),_=>None}).collect::<Vec<_>>().join(", ")};
            let after=r.after.iter().filter_map(|n|match n.tone{crate::vocals::VocalTone::Pitched{midi}=>Some(format!("{} ({:.2}s)",crate::vocal_pitch_analysis::note_name(midi),timing.seconds(n.end)-timing.seconds(n.start))),_=>None}).collect::<Vec<_>>().join(", ");
            let checked=selected.contains(&i);let key=key.clone();
            rsx!{div{class:"review-evidence",key:"repair-{i}",
                if !a.applied{label{class:"form-check",input{r#type:"checkbox",aria_label:"Select {label} at {time}",checked,disabled:!valid||busy,onchange:move|e|{let mut value=chosen.write();if value.0!=key{*value=(key.clone(),Default::default());}
                    if e.checked(){value.1.insert(i);}else{value.1.remove(&i);}}},"{label} · {time}"}}
                else{strong{"{label} · {time}"}if a.section.applied_repairs.contains(&i){span{" · Applied"}}else{span{" · Not applied"}}}
                p{"{before} → {after}"}
                p{class:"field-hint","{r.detail} · {r.support_frames} model frames / {r.acoustic_frames} agreeing acoustic frames"}
                button{class:"studio-tool",disabled:busy,onclick:move |_|{studio.dispatch(Action::SetLoop{range:Some(TimeRange{start,end})});studio.seek(start);},"Loop and inspect"}
            }}}
        }
        if !a.section.findings.is_empty(){details{class:"review-evidence",summary{"{a.section.findings.len()} sounds to inspect"},
            for (i,f) in a.section.findings.iter().take(200).enumerate(){
                {let start=f.start;let end=f.end;let time=format!("{start:.2}–{end:.2}s");
                rsx!{div{key:"finding-{i}",p{class:"field-hint","{time}: {f.detail}"}button{class:"studio-tool",disabled:busy,onclick:move |_|{studio.dispatch(Action::SetLoop{range:Some(TimeRange{start,end})});studio.seek(start);},"Inspect sound"}}}
            }}
            if a.section.findings.len()>200 {p{"Showing the first 200 findings. Analyze a shorter loop to focus the review."}}
        }}
    }}
}

fn contour_plot(
    studio: Studio,
    a: &crate::vocal_pitch_analysis::Analysis,
    timing: &crate::timing::TempoMap,
    selected: &std::collections::BTreeSet<usize>,
) -> Element {
    let Some(doc) = studio.snapshot() else {
        return rsx! {};
    };
    let Some(chart) = &doc.content.vocals else {
        return rsx! {};
    };
    let notes: Vec<_> = chart
        .notes
        .iter()
        .filter(|n| timing.seconds(n.start) < a.range.end && timing.seconds(n.end) > a.range.start)
        .take(1000)
        .collect();
    let pitches: Vec<f64> = a
        .section
        .contour
        .iter()
        .filter_map(|p| p.midi)
        .chain(notes.iter().filter_map(|n| match n.tone {
            crate::vocals::VocalTone::Pitched { midi } => Some(midi as f64),
            _ => None,
        }))
        .collect();
    let low = pitches.iter().copied().fold(60., f64::min).floor().max(34.) - 1.;
    let high = pitches.iter().copied().fold(72., f64::max).ceil().min(86.) + 1.;
    let x =
        |t: f64| 50. + 820. * ((t - a.range.start) / (a.range.end - a.range.start)).clamp(0., 1.);
    let y = |p: f64| 155. - 135. * ((p - low) / (high - low)).clamp(0., 1.);
    let mut path = String::new();
    let mut open = false;
    for p in &a.section.contour {
        if let Some(midi) = p.midi {
            path.push_str(&format!(
                "{}{:.2},{:.2} ",
                if open { "L" } else { "M" },
                x(p.seconds),
                y(midi)
            ));
            open = true;
        } else {
            open = false;
        }
    }
    rsx! {svg{view_box:"0 0 900 180",width:"100%",role:"img","aria-label":"Vocal pitch contour compared with current notes and selected section repairs",
        title{"Vocal pitch in the selected audio interval"}
        rect{x:"0",y:"0",width:"900",height:"180",fill:"#151922"}
        for step in 0..=4 {
            {let pitch=(low+(high-low)*step as f64/4.).round() as u8;let py=y(pitch as f64).to_string();let label=crate::vocal_pitch_analysis::note_name(pitch);
            rsx!{g{line{x1:"50",x2:"870",y1:py.clone(),y2:py.clone(),stroke:"#39404c"}text{x:"4",y:py,fill:"#cbd1df",font_size:"12","{label}"}}}}
        }
        for n in notes {if let crate::vocals::VocalTone::Pitched{midi}=n.tone {
            rect{x:x(timing.seconds(n.start)).to_string(),y:(y(midi as f64)-3.).to_string(),width:(x(timing.seconds(n.end))-x(timing.seconds(n.start))).max(1.).to_string(),height:"6",fill:"#adb5c4"}
        }}
        for i in selected {if let Some(r)=a.section.repairs.get(*i){for n in &r.after{if let crate::vocals::VocalTone::Pitched{midi}=n.tone {
            rect{x:x(timing.seconds(n.start)).to_string(),y:(y(midi as f64)-4.).to_string(),width:(x(timing.seconds(n.end))-x(timing.seconds(n.start))).max(1.).to_string(),height:"8",fill:"#f7aa38"}
        }}}}
        path{d:path,fill:"none",stroke:"#61c7ff",stroke_width:"1.5"}
        text{x:"50",y:"175",fill:"#cbd1df",font_size:"12","{a.range.start:.2}s"}
        text{x:"800",y:"175",fill:"#cbd1df",font_size:"12","{a.range.end:.2}s"}
    }}
}
fn start(
    mut studio: Studio,
    range: Option<TimeRange>,
    mut busy: Signal<bool>,
    mut status: Signal<String>,
    mut stop: Signal<Option<Stop>>,
    adaptive: bool,
) {
    let Some(project) = studio.snapshot() else {
        return;
    };
    let range = range.map(|r| TimeRange {
        start: r.start,
        end: r.end.min(project.duration),
    });
    let Some(store) = studio.store.read().clone() else {
        return;
    };
    let Some(_timing) = (studio.timing)() else {
        return;
    };
    let audio = match store.audio_path(&project) {
        Ok(p) => p,
        Err(e) => {
            studio.error.set(Some(e.to_string()));
            return;
        }
    };
    let expected_root = store.root.clone();
    busy.set(true);
    status.set("Preparing independent vocal pitch comparison…".into());
    studio.error.set(None);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let control = std::sync::Arc::new(crate::clean_recognition::Control::default());
        stop.set(Some(control.clone()));
        spawn(async move {
            let models = crate::generation::default_model_dir();
            let worker = control.clone();
            let result = tokio::task::spawn_blocking(move || {
                crate::vocal_pitch_analysis::analyze_configured(
                    &project, &audio, &_timing, range, &models, worker, adaptive,
                )
            })
            .await;
            if !control.is_cancelled()
                && studio
                    .store
                    .read()
                    .as_ref()
                    .is_some_and(|s| s.root == expected_root)
            {
                match result {
                    Ok(Ok(analysis)) => studio.dispatch(Action::SetVocalPitchAnalysis { analysis }),
                    Ok(Err(e)) => studio.error.set(Some(format!("{e:#}"))),
                    Err(e) => studio.error.set(Some(e.to_string())),
                }
            }
            busy.set(false);
            stop.set(None);
        });
    }
    #[cfg(target_arch = "wasm32")]
    {
        let (sender, receiver) = futures_channel::oneshot::channel();
        stop.set(Some(std::rc::Rc::new(std::cell::RefCell::new(Some(
            sender,
        )))));
        spawn(async move {
            let callback = std::rc::Rc::new(move |message: String| {
                let mut s = status;
                s.set(message);
            });
            let roots = [expected_root.clone()];
            let task = Box::pin(crate::worker::run_with_progress::<
                crate::vocal_pitch_analysis::Analysis,
            >(
                crate::worker::Task::CompareVocalPitch {
                    root: expected_root.clone(),
                    project: Box::new((*project).clone()),
                    audio,
                    range,
                    adaptive,
                },
                &roots,
                callback,
            ));
            let result = match futures_util::future::select(task, Box::pin(receiver)).await {
                futures_util::future::Either::Left((result, _)) => result,
                futures_util::future::Either::Right(_) => Err(anyhow::anyhow!(
                    "Vocal comparison cancelled; current notes were kept"
                )),
            };
            if studio
                .store
                .read()
                .as_ref()
                .is_some_and(|s| s.root == expected_root)
            {
                match result {
                    Ok(analysis) => studio.dispatch(Action::SetVocalPitchAnalysis { analysis }),
                    Err(e) => studio.error.set(Some(format!("{e:#}"))),
                }
            }
            busy.set(false);
            stop.set(None);
        });
    }
}
