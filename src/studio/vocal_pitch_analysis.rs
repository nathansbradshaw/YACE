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
        p{class:"field-hint","Find supported pitch corrections automatically. Lyrics and note timing are preserved."}
        p{class:"field-hint","Uses the High accuracy model (362 MB, downloaded once). Phrase checks use the full recording for context."}
        div{class:"vocal-file-actions",
            button{class:"studio-tool",disabled:busy()||!ready||range.is_none(),onclick:move |_|start(studio,range,busy,status,stop),Icon{name:"music",size:13}"Analyze phrase"}
            button{class:"studio-tool",disabled:busy()||!ready,onclick:move |_|start(studio,None,busy,status,stop),"Analyze all vocals"}
            if busy(){button{class:"studio-tool",onclick:move |_|{if let Some(s)=stop.read().as_ref(){cancel(s);}status.set("Cancelling comparison…".into());},"Cancel"}}
        }
        if !ready{p{class:"field-hint","Attach a vocal stem to compare these notes with the recording."}}
        if busy(){p{role:"status","{status}"}}
        if let Some(a)=analysis {
            if a.applied {p{role:"status","Applied {changes} pitch {change_label}. Undo restores the previous pitches."}}
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
        }
    }}
}
fn start(
    mut studio: Studio,
    range: Option<TimeRange>,
    mut busy: Signal<bool>,
    mut status: Signal<String>,
    mut stop: Signal<Option<Stop>>,
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
                crate::vocal_pitch_analysis::analyze(
                    &project, &audio, &_timing, range, &models, worker,
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
