use super::{dialogs as rfd, modal::Modal, state::Studio};
use dioxus::prelude::*;
use std::path::PathBuf;

#[component]
pub fn Setup(mut studio: Studio, root: PathBuf, suggested_title: String) -> Element {
    let original_title = suggested_title.clone();
    let mut title = use_signal(|| suggested_title);
    let mut artist = use_signal(String::new);
    let mut tempo = use_signal(|| "120".to_owned());
    let mut info = use_signal(crate::song_import::SongInfo::default);
    let mut duration = use_signal(|| 0.);
    let mut use_detected = use_signal(|| true);
    let mut busy = use_signal(|| false);
    let mut lookup_busy = use_signal(|| false);
    let mut lyrics = use_signal(|| false);
    let analysis_root = root.clone();
    let analysis = use_resource(move || {
        let root = analysis_root.clone();
        let original_title = original_title.clone();
        async move {
            let result = studio.prepare_new_chart(root).await;
            match result {
                Ok(recording) => {
                    if *title.peek() == original_title {
                        if !recording.metadata.name.trim().is_empty()
                            && recording.metadata.name != "song"
                        {
                            title.set(recording.metadata.name.clone());
                        } else if let Some((a, t)) = original_title.split_once(" - ") {
                            title.set(t.trim().into());
                            if artist.peek().is_empty() {
                                artist.set(a.trim().into());
                            }
                        }
                    }
                    if artist.peek().is_empty() {
                        artist.set(recording.metadata.artist.clone());
                    }
                    duration.set(recording.metadata.duration);
                    let estimate = recording.info.tempo.clone();
                    if let Some(bpm) = estimate.bpm {
                        tempo.set(bpm.to_string());
                    }
                    info.set(recording.info);
                    Ok(estimate)
                }
                Err(error) => Err(format!("{error:#}")),
            }
        }
    });
    let analyzing = analysis.read().is_none();
    let estimate = analysis
        .read()
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .cloned();
    let art = use_memo(move || info.read().artwork.clone());
    let preview = use_memo(move || {
        art().and_then(|path| {
            use base64::Engine;
            let bytes = crate::storage::read(path).ok()?;
            let extension = crate::song_metadata::cover_extension(&bytes).ok()?;
            let mime = if extension == "png" {
                "image/png"
            } else {
                "image/jpeg"
            };
            Some(format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ))
        })
    });
    let cancel_root = root.clone();
    let close = use_callback(move |_: ()| {
        if !busy() {
            studio.cancel_new_chart(&cancel_root);
            studio.error.set(None);
        }
    });
    let valid = !title().trim().is_empty()
        && !artist().trim().is_empty()
        && tempo()
            .parse::<f64>()
            .is_ok_and(|bpm| bpm.is_finite() && (1. ..=1000.).contains(&bpm));
    let lookup_root = root.clone();
    let cover_root = root.clone();
    rsx! {Modal{id:"new-song-dialog",label:"New chart",onclose:close,
        section{class:"sections-dialog",
            h2{"New chart"}
            p{"Start with an empty Guitar chart. Add other parts in the editor."}
            label{class:"field-stack","Title",input{class:"field-control",aria_label:"Song title",value:title,disabled:busy()||lookup_busy(),oninput:move|e|title.set(e.value())}}
            label{class:"field-stack","Artist",input{class:"field-control",aria_label:"Song artist",value:artist,disabled:busy()||lookup_busy(),oninput:move|e|artist.set(e.value())}}
            if analyzing {p{class:"field-hint","Reading recording tags and analyzing tempo throughout the recording…"}}
            details{open:true,summary{"Song information and cover"}
                label{class:"field-stack","Album",input{class:"field-control",aria_label:"Song album",value:"{info.read().album}",disabled:busy()||lookup_busy(),oninput:move|e|info.with_mut(|info|info.album=e.value())}}
                label{class:"field-stack","Year",input{class:"field-control",aria_label:"Song year",value:"{info.read().year}",disabled:busy()||lookup_busy(),oninput:move|e|info.with_mut(|info|info.year=e.value())}}
                label{class:"field-stack","Genre",input{class:"field-control",aria_label:"Song genre",value:"{info.read().genre}",disabled:busy()||lookup_busy(),oninput:move|e|info.with_mut(|info|info.genre=e.value())}}
                if let Some(preview)=preview(){img{src:preview,alt:"Selected cover artwork",style:"width:120px;height:120px;object-fit:contain"}}
                button{disabled:busy()||lookup_busy(),"data-browser-pick":"file","data-browser-accept":".png,.jpg,.jpeg",onclick:move|_|{
                    let root=cover_root.clone();spawn(async move{
                        if let Some(file)=rfd::AsyncFileDialog::new().add_filter("Cover image",&["png","jpg","jpeg"]).pick_file().await {
                            match studio.stage_new_cover(&root,file.path()){Ok(path)=>{info.with_mut(|info|{info.artwork=Some(path);info.artwork_source="Chosen local cover image".into();});studio.error.set(None);},Err(error)=>studio.error.set(Some(format!("{error:#}")))}
                        }
                    });
                },"Choose cover…"}
                button{disabled:busy()||lookup_busy()||info.read().artwork.is_none(),onclick:move|_|info.with_mut(|info|{info.artwork=None;info.artwork_source.clear();}),"Clear cover"}
                label{class:"form-check",input{r#type:"checkbox",checked:lyrics(),disabled:busy()||lookup_busy(),onchange:move|e|lyrics.set(e.checked())},"Look up lyric reference"}
                button{disabled:busy()||lookup_busy()||analyzing||title().trim().is_empty()||duration()<=0.,onclick:move|_|{
                    let root=lookup_root.clone();let current=info();let metadata=crate::pipeline::Metadata{name:title(),artist:artist(),album:current.album.clone(),year:current.year.clone(),genre:current.genre.clone(),duration:duration()};let lyrics=lyrics();lookup_busy.set(true);
                    spawn(async move{match studio.lookup_new_chart(root,metadata,current,lyrics).await{Ok(found)=>{title.set(found.metadata.name);artist.set(found.metadata.artist);info.set(found.info);studio.error.set(None);},Err(error)=>studio.error.set(Some(format!("{error:#}")))}lookup_busy.set(false);});
                },if lookup_busy(){"Looking up song information…"}else{"Look up song info & cover art"}}
                p{class:"field-hint","Lookup shares title, artist and duration with public catalogues. Audio stays local. Suggestions remain editable."}
                if !info.read().artwork_source.is_empty(){p{class:"field-hint",style:"overflow-wrap:anywhere","{info.read().artwork_source}"}}
                if !info.read().notes.is_empty(){details{summary{"Import notes"}for note in &info.read().notes{p{class:"field-hint","{note}"}}}}
            }
            label{class:"field-stack","Starting tempo (BPM)",input{class:"field-control",aria_label:"Starting tempo BPM",r#type:"number",min:"1",max:"1000",step:"0.001",value:tempo,disabled:busy()||analyzing,oninput:move|e|{tempo.set(e.value());use_detected.set(false);}}}
            if let Some(estimate)=&estimate {
                p{class:"field-hint","{estimate.note}"}
                if !estimate.segments.is_empty(){label{class:"form-check",input{r#type:"checkbox",checked:use_detected(),disabled:busy(),onchange:move|e|{use_detected.set(e.checked());if e.checked()&&let Some(Ok(estimate))=analysis.read().as_ref()&&let Some(bpm)=estimate.bpm{tempo.set(bpm.to_string());}}},"Use detected tempo map"}
                    details{summary{"Detected tempos"}for segment in &estimate.segments{p{class:"field-hint","{segment.start_seconds:.1}s · {segment.bpm:.3} BPM"}}}
                }
            }
            if let Some(Err(error))=analysis.read().as_ref(){p{class:"field-hint","Recording preparation unavailable: {error}. Check the recording before creating the chart."}}
            p{class:"field-hint","You can change tempo and time signatures later with Timing."}
            if let Some(error)=(studio.error)(){p{role:"alert","{error}"}}
            button{disabled:busy(),onclick:move|_|close.call(()),"Cancel"}
            button{disabled:busy()||lookup_busy()||analyzing||!valid,onclick:move|_|{
                let root=root.clone();let title=title();let artist=artist();let tempo=tempo().parse().unwrap_or(120.);let info=info();
                let timing=if use_detected(){analysis.read().as_ref().and_then(|r|r.as_ref().ok()).filter(|e|!e.segments.is_empty()).map(|e|e.tempo_map()).unwrap_or_else(||crate::timing::TempoMap::constant(tempo))}else{crate::timing::TempoMap::constant(tempo)};
                let timing=match timing{Ok(timing)=>timing,Err(error)=>{studio.error.set(Some(error.to_string()));return;}};busy.set(true);
                spawn(async move{if let Err(error)=studio.finish_new_chart(root,title,artist,timing,info).await{studio.error.set(Some(format!("{error:#}")));busy.set(false);}});
            },if busy(){"Preparing…"}else{"Create chart"}}
        }
    }}
}
