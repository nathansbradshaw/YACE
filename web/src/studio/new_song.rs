use super::{modal::Modal, state::Studio};
use dioxus::prelude::*;
use std::path::PathBuf;

#[component]
pub fn Setup(mut studio: Studio, root: PathBuf, suggested_title: String) -> Element {
    let mut title = use_signal(|| suggested_title);
    let mut artist = use_signal(String::new);
    let mut tempo = use_signal(|| "120".to_owned());
    let mut use_detected = use_signal(|| true);
    let mut busy = use_signal(|| false);
    let analysis_root = root.clone();
    let analysis = use_resource(move || {
        let root = analysis_root.clone();
        async move {
            let result = async {
                let audio = crate::pipeline::audio_file(&root)?;
                super::media::analyze_tempo(crate::storage::shared(&audio)?.as_slice()).await
            }
            .await;
            if let Ok(estimate) = &result
                && let Some(bpm) = estimate.bpm
            {
                tempo.set(bpm.to_string());
            }
            result.map_err(|error: anyhow::Error| error.to_string())
        }
    });
    let analyzing = analysis.read().is_none();
    let estimate = analysis
        .read()
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .cloned();
    let cancel_root = root.clone();
    let close = use_callback(move |_: ()| {
        if !busy() {
            crate::storage::clear(&cancel_root);
            studio.new_song.set(None);
            studio.error.set(None);
        }
    });
    let valid = !title().trim().is_empty()
        && !artist().trim().is_empty()
        && tempo()
            .parse::<f64>()
            .is_ok_and(|bpm| bpm.is_finite() && (1. ..=1000.).contains(&bpm));
    rsx! {
        Modal { id:"new-song-dialog", label:"New chart", onclose:close,
            section { class:"sections-dialog",
                h2 { "New chart" }
                p { "Start with an empty Guitar chart. Add other parts in the editor." }
                label { class:"field-stack", "Title", input { class:"field-control", aria_label:"Song title", value:title, oninput:move |event|title.set(event.value()), disabled:busy() } }
                label { class:"field-stack", "Artist", input { class:"field-control", aria_label:"Song artist", value:artist, oninput:move |event|artist.set(event.value()), disabled:busy() } }
                label { class:"field-stack", "Starting tempo (BPM)", input { class:"field-control", aria_label:"Starting tempo BPM", r#type:"number", min:"1", max:"1000", step:"0.001", value:tempo, oninput:move |event|{tempo.set(event.value());use_detected.set(false);}, disabled:busy() || analyzing } }
                if analyzing {p {class:"field-hint", "Analyzing tempo throughout the recording…"}}
                if let Some(estimate)=&estimate {
                    p {class:"field-hint", "{estimate.note}"}
                    if !estimate.segments.is_empty() {
                        label {class:"form-check",input {r#type:"checkbox",checked:use_detected(),disabled:busy(),onchange:move |event|{use_detected.set(event.checked());if event.checked() && let Some(Ok(estimate))=analysis.read().as_ref() && let Some(bpm)=estimate.bpm {tempo.set(bpm.to_string());}}},"Use detected tempo map"}
                        details {summary {"Detected tempos"}
                            for segment in &estimate.segments {p {class:"field-hint","{segment.start_seconds:.1}s · {segment.bpm:.3} BPM"}}
                        }
                    }
                }
                if let Some(Err(error))=analysis.read().as_ref() {p {class:"field-hint", "Tempo analysis unavailable: {error}. Enter a starting BPM."}}
                p { class:"field-hint", "You can change tempo and time signatures later with Timing." }
                if let Some(error)=(studio.error)(){p{role:"alert","{error}"}}
                button { disabled:busy(), onclick:move |_|close.call(()), "Cancel" }
                button { disabled:busy() || analyzing || !valid, onclick:move |_|{
                    let root=root.clone();let title=title();let artist=artist();let tempo=tempo().parse().unwrap_or(120.);
                    let timing = if use_detected() {analysis.read().as_ref().and_then(|result|result.as_ref().ok()).filter(|estimate|!estimate.segments.is_empty()).map(|estimate|estimate.tempo_map()).unwrap_or_else(||crate::timing::TempoMap::constant(tempo))} else {crate::timing::TempoMap::constant(tempo)};
                    let timing = match timing {Ok(timing)=>timing,Err(error)=>{studio.error.set(Some(error.to_string()));return;}};
                    busy.set(true);
                    spawn(async move{if let Err(error)=studio.finish_new_chart(root,title,artist,timing).await{studio.error.set(Some(format!("{error:#}")));busy.set(false);}});
                }, if busy(){"Preparing…"}else{"Create chart"} }
            }
        }
    }
}
