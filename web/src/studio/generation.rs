//! Browser generation stays on local workers and publishes complete results.
use super::{
    modal::Modal,
    state::{DownloadKind, Studio},
};
use crate::stem_review::StemManifest;
use dioxus::prelude::*;
use std::{cell::RefCell, path::PathBuf, rc::Rc};

type Cancel = Rc<RefCell<Option<futures_channel::oneshot::Sender<()>>>>;

#[component]
pub fn GenerationPanel(mut studio: Studio) -> Element {
    let mut busy = use_signal(|| false);
    let mut status = use_signal(String::new);
    let mut failure = use_signal(|| None::<String>);
    let mut ready = use_signal(|| None::<(PathBuf, StemManifest)>);
    let mut transcribed = use_signal(|| None::<(PathBuf, crate::mapping::Transcription)>);
    let mut draft = use_signal(||None::<PathBuf>);
    let mut requested=use_signal(||vec!["drums".to_owned(),"guitar".to_owned(),"bass".to_owned()]);
    let mut vocal_report=use_signal(||None::<(PathBuf,crate::vocal_generation::Generation)>);
    let mut cancel = use_signal(|| None::<Cancel>);
    use_drop(move || {
        if let Some(cancel) = cancel.peek().as_ref()
            && let Some(sender) = cancel.borrow_mut().take()
        {
            let _ = sender.send(());
        }
    });
    if !(studio.generation_open)() {
        return rsx! {};
    }
    let source = (studio.source_audio)();
    let attached=studio.snapshot().and_then(|doc|doc.content.stems.clone()).filter(|manifest|manifest.ids()==crate::stem_review::StemId::ALL.into_iter().collect());
    rsx! {Modal{id:"browser-generation",label:"Generate charts",onclose:move |_|studio.generation_open.set(false),
        section{class:"sections-dialog",
            h2{"Generate charts"}
            p{"Process the recording locally. Models are verified and cached in this browser."}
            button{disabled:busy(),onclick:move |_|{busy.set(true);failure.set(None);spawn(async move{
                let root=PathBuf::from(format!("/model-import/{}",crate::storage::unique_id()));
                let result=async{
                    let Some(files)=crate::files::pick(false,".onnx,.safetensors,.bin").await? else{return Ok(None)};
                    crate::files::mount(&root,files)?;let files=crate::storage::list(&root)?;
                    let path=files.first().map(|(path,_)|path.clone()).ok_or_else(||anyhow::anyhow!("No model selected"))?;
                    busy.set(true);failure.set(None);status.set("Verifying the selected model checkpoint…".into());
                    let id=crate::worker::run::<String>(crate::worker::Task::CacheModel{path},std::slice::from_ref(&root)).await?;
                    Ok::<_,anyhow::Error>(Some(id))
                }.await;
                crate::storage::clear(&root);busy.set(false);
                match result{Ok(Some(id))=>status.set(format!("{id} model verified and cached locally")),Ok(None)=>{},Err(error)=>failure.set(Some(format!("{error:#}")))}
            });},"Use a downloaded model…"}
            fieldset{disabled:busy(),legend{"Chart parts"}
                for(label,_,id)in crate::parts::PARTS.into_iter().filter(|(_,_,id)|*id!="vocals") {
                    label{input{r#type:"checkbox",checked:requested.read().iter().any(|part|part==id),onchange:move |event|{
                        let mut selected=requested.write();selected.retain(|part|part!=id);if event.checked(){selected.push(id.into());}
                    }},"{label}"}
                }
            }
            if let Some(source)=source {
                p{"{source.file_name().unwrap_or_default().to_string_lossy()}"}
                button{disabled:busy(),onclick:move |_|{
                    let audio=source.clone();
                    let output=PathBuf::from(format!("/generated/{}",crate::storage::unique_id()));
                    let (sender,receiver)=futures_channel::oneshot::channel();
                    cancel.set(Some(Rc::new(RefCell::new(Some(sender)))));
                    if let Some((previous,_))=ready.take(){crate::storage::clear(&previous);}
                    if let Some((previous,_))=transcribed.take(){crate::storage::clear(&previous);}
                    draft.set(None);
                    vocal_report.set(None);
                    busy.set(true);failure.set(None);status.set("Preparing local separation…".into());
                    dioxus::core::spawn_forever(async move{
                        let progress=Rc::new(move|label:String|{let mut current=status;current.set(label.clone());let mut global=studio;global.status.set(label);});
                        let task=crate::worker::run_with_progress::<StemManifest>(crate::worker::Task::Separate{audio,output:output.clone()},&[],progress);
                        futures_util::pin_mut!(task,receiver);
                        let result=match futures_util::future::select(task,receiver).await {
                            futures_util::future::Either::Left((result,_))=>result,
                            futures_util::future::Either::Right(_)=>Err(anyhow::anyhow!("Generation cancelled; existing chart and audio were kept")),
                        };
                        match result{Ok(manifest)=>{status.set("Six separated parts are ready".into());studio.status.set("Separated parts ready to review".into());ready.set(Some((output,manifest)));},Err(error)=>{crate::storage::clear(&output);studio.status.set("Generation stopped; existing chart and audio kept".into());failure.set(Some(format!("{error:#}")));}}
                        cancel.set(None);busy.set(false);
                    });
                },"Separate vocals and instruments"}
                if let Some(manifest)=attached {
                    button{disabled:busy(),onclick:move |_|{
                        let manifest=manifest.clone();let Some(audio)=(studio.source_audio)()else{return};
                        let output=PathBuf::from(format!("/generated/{}",crate::storage::unique_id()));
                        busy.set(true);failure.set(None);status.set("Checking the chart’s separated audio…".into());
                        dioxus::core::spawn_forever(async move{
                            let result=async{
                                let mut manifest=crate::worker::run::<StemManifest>(crate::worker::Task::ValidateStems{audio,manifest},&[]).await?;
                                for stem in &mut manifest.stems {let path=output.join(format!("{}.wav",stem.id.as_str()));crate::storage::copy(&stem.path,&path)?;stem.path=path;}
                                crate::models::remember_stems(&manifest)?;
                                crate::storage::write(output.join("manifest.json"),serde_json::to_vec_pretty(&manifest)?)?;
                                Ok::<_,anyhow::Error>(manifest)
                            }.await;
                            match result{Ok(manifest)=>{if let Some((previous,_))=ready.take(){crate::storage::clear(&previous);}ready.set(Some((output,manifest)));status.set("The chart’s separated audio is ready".into());},Err(error)=>{crate::storage::clear(&output);failure.set(Some(format!("{error:#}")));}}
                            busy.set(false);
                        });
                    },"Use this chart’s separated audio"}
                }
            } else {
                p{"Choose a recording to start."}
                button{onclick:move |_|{studio.generation_open.set(false);studio.new_chart();},"Choose recording…"}
            }
            if busy()&&cancel.read().is_some(){button{onclick:move |_|{if let Some(cancel)=cancel.read().as_ref()&&let Some(sender)=cancel.borrow_mut().take(){let _=sender.send(());}},"Cancel generation"}}
            if !status().is_empty(){p{role:"status","{status}"}}
            if let Some(error)=failure(){p{role:"alert","{error}"}}
            if let Some((root,manifest))=ready(){
                p{"{manifest.frames} stereo frames · 44.1 kHz · source checksums verified"}
                ul{for asset in &manifest.stems {li{"{asset.id.as_str()}"}}}
                button{disabled:busy(),onclick:{let root=root.clone();let manifest=manifest.clone();move |_|{
                    let Some(audio)=(studio.source_audio)()else{return};let Some(timing)=(studio.timing)()else{return};let root=root.clone();let manifest=manifest.clone();
                    let output=PathBuf::from(format!("/vocal-draft/{}",crate::storage::unique_id()));
                    let(sender,receiver)=futures_channel::oneshot::channel();cancel.set(Some(Rc::new(RefCell::new(Some(sender)))));
                    busy.set(true);failure.set(None);
                    dioxus::core::spawn_forever(async move{
                        let progress=Rc::new(move|label:String|{let mut current=status;current.set(label.clone());let mut global=studio;global.status.set(label);});
                        let roots=[root];let task=crate::worker::run_with_progress::<crate::vocal_generation::Generation>(crate::worker::Task::Vocals{audio,manifest,timing,output:output.clone()},&roots,progress);
                        futures_util::pin_mut!(task,receiver);
                        let result=match futures_util::future::select(task,receiver).await{futures_util::future::Either::Left((value,_))=>value,futures_util::future::Either::Right(_)=>Err(anyhow::anyhow!("Vocal generation cancelled; existing chart kept"))};
                        match result{Ok(report)=>{status.set("Vocal pitch draft ready; missing lyrics remain marked for review".into());vocal_report.set(Some((output,report)));},Err(error)=>{crate::storage::clear(&output);failure.set(Some(format!("{error:#}")));}}
                        busy.set(false);cancel.set(None);
                    });
                }},"Generate vocal pitches"}
                button{disabled:busy(),onclick:{let root=root.clone();let manifest=manifest.clone();move |_|{
                    let root=root.clone();let manifest=manifest.clone();let Some(audio)=(studio.source_audio)() else{return};let wanted=requested();
                    let output=PathBuf::from(format!("/transcribed/{}",crate::storage::unique_id()));
                    let(sender,receiver)=futures_channel::oneshot::channel();cancel.set(Some(Rc::new(RefCell::new(Some(sender)))));
                    busy.set(true);failure.set(None);transcribed.set(None);
                    dioxus::core::spawn_forever(async move{
                        let progress=Rc::new(move|label:String|{let mut current=status;current.set(label.clone());let mut global=studio;global.status.set(label);});
                        let roots=[root];
                        let task=crate::worker::run_with_progress::<crate::mapping::Transcription>(crate::worker::Task::Transcribe{audio,manifest,wanted,output:output.clone()},&roots,progress);
                        futures_util::pin_mut!(task,receiver);
                        let result=match futures_util::future::select(task,receiver).await{futures_util::future::Either::Left((value,_))=>value,futures_util::future::Either::Right(_)=>Err(anyhow::anyhow!("Generation cancelled; current chart kept"))};
                        match result{Ok(events)=>{status.set("Instrument detections ready for arrangement".into());transcribed.set(Some((output,events)));},Err(error)=>{crate::storage::clear(&output);failure.set(Some(format!("{error:#}")));}}
                        busy.set(false);cancel.set(None);
                    });
                }},"Detect selected parts"}
                button{disabled:busy(),onclick:move |_|{
                    let root=root.clone();spawn(async move{
                        let path=root.with_extension("zip");
                        let result=crate::worker::run::<serde_json::Value>(crate::worker::Task::Archive{root:root.clone(),output:path.clone()},&[root]).await.map(|_|{
                            if let Some((old,_,_))=studio.export_ready.replace(Some((path,"separated-parts.zip".into(),DownloadKind::File))){crate::storage::clear(&old);}
                        });
                        if let Err(error)=result{failure.set(Some(format!("{error:#}")));}
                    });
                },"Save separated parts…"}
            }
            if let Some((root,events))=transcribed(){
                p{"{events.drums.len()} drum hits · {events.guitar.len()} guitar notes · {events.bass.len()} bass notes · {events.piano.len()} Keys · {events.pro_piano.len()} Pro Keys"}
                button{onclick:move |_|{
                    let path=root.join("transcription.json");
                    studio.export_ready.set(Some((path,"transcription.json".into(),DownloadKind::File)));
                },"Save transcription evidence…"}
                button{disabled:busy(),onclick:move |_|{
                    let Some(audio)=(studio.source_audio)() else{return};let Some(timing)=(studio.timing)() else{return};let Some(doc)=studio.snapshot() else{return};
                    let events=events.clone();let roots:Vec<_>=events.isolation.iter().flat_map(|m|m.stems.iter().filter_map(|s|s.path.parent().map(|p|p.to_owned()))).collect();
                    let meta=crate::pipeline::Metadata{name:studio.metadata_value("name"),artist:studio.metadata_value("artist"),album:studio.metadata_value("album"),year:studio.metadata_value("year"),genre:studio.metadata_value("genre"),duration:doc.duration};
                    let output=PathBuf::from(format!("/drafts/{}",crate::storage::unique_id()));
                    let vocals=vocal_report().map(|(_,report)|Box::new(report));
                    let(sender,receiver)=futures_channel::oneshot::channel();cancel.set(Some(Rc::new(RefCell::new(Some(sender)))));
                    busy.set(true);failure.set(None);status.set("Building chart parts and review evidence…".into());
                    dioxus::core::spawn_forever(async move{
                        let task=crate::worker::run::<serde_json::Value>(crate::worker::Task::BuildDraft{audio,events:Box::new(events),vocals,timing,meta,pro:true,output:output.clone()},&roots);
                        futures_util::pin_mut!(task,receiver);
                        let result=match futures_util::future::select(task,receiver).await{futures_util::future::Either::Left((value,_))=>value,futures_util::future::Either::Right(_)=>Err(anyhow::anyhow!("Chart build cancelled; existing project kept"))};
                        match result{Ok(_)=>{draft.set(Some(output));status.set("Chart draft ready to open and review".into());},Err(error)=>{crate::storage::clear(&output);failure.set(Some(format!("{error:#}")));}}
                        busy.set(false);cancel.set(None);
                    });
                },"Build chart draft"}
            }
            if let Some((root,report))=vocal_report(){
                p{"{report.chart.as_ref().map_or(0,|chart|chart.notes.len())} vocal notes · {report.issues.len()} review issues"}
                button{onclick:move |_|studio.export_ready.set(Some((root.join("vocal-generation.json"),"vocal-generation.json".into(),DownloadKind::File))),"Save vocal evidence…"}
            }
            if let Some(path)=draft(){button{disabled:busy(),onclick:move |_|{let path=path.clone();spawn(async move{match async{studio.persist_current().await?;studio.open_project(path).await}.await{Ok(())=>studio.generation_open.set(false),Err(error)=>failure.set(Some(format!("{error:#}")))}});},"Open generated draft"}}
            button{onclick:move |_|studio.generation_open.set(false),"Close"}
        }
    }}
}
