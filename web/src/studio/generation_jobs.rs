//! Recoverable local generation with the same guarded part application as native.
use super::{
    modal::Modal,
    state::{DownloadKind, Studio},
};
use crate::generation::{self, State, Status};
use anyhow::{Context, Result, ensure};
use dioxus::prelude::*;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

#[derive(Clone)]
struct Job {
    root: PathBuf,
    revision: u64,
}
#[derive(Clone)]
struct View {
    title: String,
    requested: String,
    status: Status,
    summary: generation::Summary,
    part: Option<String>,
}
fn view(root: &std::path::Path) -> Result<View> {
    let request = generation::read_request(root)?;
    let target = root.join("editor-target.json");
    let part = if crate::storage::is_file(&target) {
        Some(serde_json::from_slice::<crate::parts::Target>(&crate::storage::read(target)?)?.track)
    } else {
        None
    };
    Ok(View {
        title: format!("{} — {}", request.artist, request.title),
        requested: request.instruments.join(", "),
        status: generation::status(root)?,
        summary: generation::summary(root)?,
        part,
    })
}
type Stop = Rc<(
    Cell<bool>,
    RefCell<Option<futures_channel::oneshot::Sender<()>>>,
)>;
#[derive(Clone, Copy)]
struct Runner {
    job: Signal<Option<Job>>,
    view: Signal<Option<View>>,
    busy: Signal<bool>,
    status: Signal<String>,
    error: Signal<Option<String>>,
    stop: Signal<Option<Stop>>,
    refresh: Signal<u64>,
}
impl Runner {
    fn start(mut self, mut studio: Studio, mut job: Job) {
        let (sender, receiver) = futures_channel::oneshot::channel();
        let stop = Rc::new((Cell::new(false), RefCell::new(Some(sender))));
        self.stop.set(Some(stop.clone()));
        self.busy.set(true);
        self.error.set(None);
        dioxus::core::spawn_forever(async move {
            let result = async {
                let mut receiver = Some(Box::pin(receiver));
                while let Some(stage) = generation::next(&job.root)? {
                    if stop.0.get() {
                        break;
                    }
                    generation::update(
                        &job.root,
                        State::Running,
                        stage,
                        stage.label().into(),
                        None,
                    )?;
                    self.status.set(format!(
                        "Saving job before {}…",
                        stage.label().to_ascii_lowercase()
                    ));
                    job.revision = generation::checkpoint(&job.root, Some(job.revision)).await?;
                    self.job.set(Some(job.clone()));
                    let roots = [job.root.clone()];
                    let progress = Rc::new(move |label: String| {
                        let mut runner = self;
                        runner.status.set(label.clone());
                        let mut studio = studio;
                        studio.status.set(label);
                    });
                    let task = Box::pin(crate::worker::run_with_progress::<Status>(
                        crate::worker::Task::GenerationStage {
                            root: job.root.clone(),
                            stage,
                        },
                        &roots,
                        progress,
                    ));
                    let cancellation = receiver
                        .take()
                        .context("Missing job cancellation receiver")?;
                    let result = match futures_util::future::select(task, cancellation).await {
                        futures_util::future::Either::Left((result, remaining)) => {
                            receiver = Some(remaining);
                            result
                        }
                        futures_util::future::Either::Right(_) => Err(anyhow::anyhow!(
                            "Generation cancelled; completed stages and the current chart were kept"
                        )),
                    };
                    if let Err(error) = result {
                        generation::update(
                            &job.root,
                            if stop.0.get() {
                                State::Cancelled
                            } else {
                                State::Failed
                            },
                            stage,
                            "Retained completed stages".into(),
                            Some(format!("{error:#}")),
                        )?;
                        job.revision =
                            generation::checkpoint(&job.root, Some(job.revision)).await?;
                        self.job.set(Some(job.clone()));
                        self.view.set(Some(view(&job.root)?));
                        return Err(error);
                    }
                    self.status
                        .set("Saving the completed stage locally…".into());
                    job.revision = generation::checkpoint(&job.root, Some(job.revision)).await?;
                    self.job.set(Some(job.clone()));
                    self.view.set(Some(view(&job.root)?));
                    if stop.0.get() && generation::status(&job.root)?.state != State::Complete {
                        generation::update(
                            &job.root,
                            State::Cancelled,
                            stage,
                            "Completed stage saved; generation cancelled".into(),
                            None,
                        )?;
                        job.revision =
                            generation::checkpoint(&job.root, Some(job.revision)).await?;
                        self.job.set(Some(job.clone()));
                        self.view.set(Some(view(&job.root)?));
                        break;
                    }
                }
                self.status.set(
                    if generation::status(&job.root)?.state == State::Complete {
                        "Draft ready to review"
                    } else {
                        "Completed stages saved; resume when ready"
                    }
                    .into(),
                );
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if let Err(error) = result {
                self.error.set(Some(format!("{error:#}")));
            }
            self.busy.set(false);
            self.stop.set(None);
            self.refresh += 1;
            studio.status.set((self.status)());
        });
    }
}

#[component]
pub fn GenerationPanel(mut studio: Studio) -> Element {
    let mut job = use_signal(|| None::<Job>);
    let mut current = use_signal(|| None::<View>);
    let mut busy = use_signal(|| false);
    let mut status = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let stop = use_signal(|| None::<Stop>);
    let refresh = use_signal(|| 0u64);
    let mut requested =
        use_signal(|| vec!["guitar".to_owned(), "bass".to_owned(), "drums".to_owned()]);
    let mut language = use_signal(|| crate::lyric_language::Language::English);
    let mut keyboard_sound = use_signal(|| crate::keyboard_detection::KeyboardSound::Piano);
    let mut piano_quality = use_signal(|| crate::keyboard_detection::PianoQuality::Standard);
    let mut standard_drums = use_signal(|| false);
    let mut full_mix_drums = use_signal(|| false);
    let mut supplied_lyrics = use_signal(|| None::<crate::vocal_generation::LyricSource>);
    let saved = use_resource(move || {
        let open = (studio.generation_open)();
        let _ = refresh();
        async move {
            if open {
                crate::persistence::jobs().await
            } else {
                Ok(Vec::new())
            }
        }
    });
    use_effect(move || {
        if let Some(track) = (studio.generation_target)()
            && let Ok(id) = crate::parts::instrument(&track)
        {
            requested.set(vec![id.into()]);
        }
    });
    use_drop(move || {
        if let Some(control) = stop.peek().as_ref() {
            control.0.set(true);
            if let Some(sender) = control.1.borrow_mut().take() {
                let _ = sender.send(());
            }
        }
    });
    let runner = Runner {
        job,
        view: current,
        busy,
        status,
        error,
        stop,
        refresh,
    };
    if !(studio.generation_open)() {
        return rsx! {};
    }
    let saved = saved
        .read()
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .cloned()
        .unwrap_or_default();
    rsx! {Modal{id:"browser-generation",label:"Generate charts",onclose:move |_|{studio.generation_open.set(false);studio.generation_target.set(None);},
        section{class:"sections-dialog",
            h2{"Generate charts"}
            p{"Audio stays in this browser. Completed stages save locally and can be resumed."}
            button{disabled:busy(),onclick:move |_|{busy.set(true);error.set(None);spawn(async move{
                let root=PathBuf::from(format!("/model-import/{}",crate::storage::unique_id()));
                let result=async{let Some(files)=crate::files::pick(false,".onnx,.safetensors,.bin").await?else{return Ok(None)};crate::files::mount(&root,files)?;
                    let path=crate::storage::list(&root)?.first().map(|(path,_)|path.clone()).context("No model selected")?;
                    status.set("Verifying the selected model checkpoint…".into());
                    Ok::<_,anyhow::Error>(Some(crate::worker::run::<String>(crate::worker::Task::CacheModel{path},std::slice::from_ref(&root)).await?))
                }.await;
                crate::storage::clear(&root);busy.set(false);match result{Ok(Some(id))=>status.set(format!("{id} model verified and cached locally")),Ok(None)=>{},Err(failure)=>error.set(Some(format!("{failure:#}")))}
            });},"Use a downloaded model…"}
            if studio.snapshot().is_some(){
                fieldset{disabled:busy(),legend{"Chart parts"}
                    for(label,_,id)in crate::parts::PARTS{label{input{r#type:"checkbox",checked:requested.read().iter().any(|part|part==id),disabled:(studio.generation_target)().is_some(),onchange:move|event|{let mut values=requested.write();values.retain(|part|part!=id);if event.checked(){values.push(id.into());}}},"{label}"}}
                }
                if requested.read().iter().any(|part|part=="vocals") {
                    button{disabled:busy(),onclick:move |_|{error.set(None);spawn(async move{
                        let result=async{let Some(files)=crate::files::pick(false,".txt,.json").await?else{return Ok(None)};ensure!(files.len()==1,"Choose one lyric file");let(name,bytes)=&files[0];ensure!(bytes.len()<=1024*1024,"Lyric text exceeds 1 MiB");
                            let lyrics=if name.ends_with(".json"){serde_json::from_slice(bytes)?}else{crate::vocal_generation::LyricSource{text:String::from_utf8(bytes.as_ref().clone())?,source:format!("User supplied local file {name}; text requires review"),synced_lyrics:None}};
                            ensure!(!lyrics.text.trim().is_empty()&&!lyrics.source.trim().is_empty(),"Lyrics require text and provenance");Ok::<_,anyhow::Error>(Some(lyrics))}.await;
                        match result{Ok(Some(lyrics))=>supplied_lyrics.set(Some(lyrics)),Ok(None)=>{},Err(failure)=>error.set(Some(format!("{failure:#}")))}
                    });},"Use supplied lyrics…"}
                    if let Some(lyrics)=supplied_lyrics(){p{"Supplied lyrics: {lyrics.source}"}button{disabled:busy(),onclick:move |_|supplied_lyrics.set(None),"Clear supplied lyrics"}}
                    label{"Lyric language"
                        select{aria_label:"Lyric language",disabled:busy(),value:match language(){crate::lyric_language::Language::English=>"en",crate::lyric_language::Language::Spanish=>"es",crate::lyric_language::Language::Auto=>"auto"},onchange:move|event|language.set(match event.value().as_str(){"es"=>crate::lyric_language::Language::Spanish,"auto"=>crate::lyric_language::Language::Auto,_=>crate::lyric_language::Language::English}),
                            option{value:"en","English"}option{value:"es","Spanish"}option{value:"auto","Detect language"}
                        }
                    }
                }
                if requested.read().iter().any(|part|part=="drums") {
                    label{"Drum mapping"
                        select{aria_label:"Drum mapping",disabled:busy(),value:if standard_drums(){"standard"}else{"pro"},onchange:move|event|standard_drums.set(event.value()=="standard"),
                            option{value:"pro","Pro Drums"}option{value:"standard","Standard drums"}
                        }
                    }
                    if requested.read().iter().all(|part|part=="drums") {
                        label{input{r#type:"checkbox",disabled:busy(),checked:full_mix_drums(),onchange:move|event|full_mix_drums.set(event.checked())},"Detect drums from the full recording"}
                    }
                }
                if requested.read().iter().any(|part|matches!(part.as_str(),"piano"|"pro_piano")) {
                    label{"Keyboard sound"
                        select{aria_label:"Keyboard sound",disabled:busy(),value:if keyboard_sound()==crate::keyboard_detection::KeyboardSound::Synth{"synth"}else{"piano"},onchange:move|event|keyboard_sound.set(if event.value()=="synth"{crate::keyboard_detection::KeyboardSound::Synth}else{crate::keyboard_detection::KeyboardSound::Piano}),
                            option{value:"piano","Piano"}option{value:"synth","Synth (experimental)"}
                        }
                    }
                    if keyboard_sound()==crate::keyboard_detection::KeyboardSound::Piano {
                        label{"Piano separation"
                            select{aria_label:"Piano separation",disabled:busy(),value:if piano_quality()==crate::keyboard_detection::PianoQuality::Standard{"standard"}else{"higher"},onchange:move|event|piano_quality.set(if event.value()=="higher"{crate::keyboard_detection::PianoQuality::HighQuality}else{crate::keyboard_detection::PianoQuality::Standard}),
                                option{value:"standard","Standard"}option{value:"higher",disabled:true,"Higher quality (desktop only)"}
                            }
                        }
                        p{"Standard piano runs locally. Higher quality piano separation is currently available in the desktop app."}
                    }
                }
                if let Some(target)=(studio.generation_target)(){p{"Build {target} for this chart. Apply the finished draft when ready; other parts stay in place."}}
                button{disabled:busy()||requested.read().is_empty(),onclick:move |_|{
                    let instruments=requested();busy.set(true);error.set(None);let mut runner=runner;
                    dioxus::core::spawn_forever(async move{
                        let result=async{
                            let store=(studio.store)().context("Open a source chart first")?;let doc=studio.snapshot().context("Open a source chart first")?;let timing=doc.timing(&store)?;
                            let metadata=crate::pipeline::Metadata{name:studio.metadata_value("name"),artist:studio.metadata_value("artist"),album:studio.metadata_value("album"),year:studio.metadata_value("year"),genre:studio.metadata_value("genre"),duration:doc.duration};
                            let full_mix=full_mix_drums()&&instruments.iter().all(|part|part=="drums");
                            let request=generation::Request{source:generation::Source::Audio{path:store.audio_path(&doc)?},bpm:timing.tempos[0].bpm,tempo_map:Some(timing),title:metadata.name.clone(),artist:metadata.artist.clone(),instruments,guitar_source:store.guitar_source(&doc)?,keyboard_sound:keyboard_sound(),piano_quality:piano_quality(),vocal_language:language(),model_dir:"/models".into(),fetch_models:true,full_mix_drums:full_mix,standard_drums:standard_drums(),lyrics:false,song_info:None};
                            let root=PathBuf::from(format!("/generation-jobs/{}",crate::storage::unique_id()));
                            let created=async{
                            if let Some(track)=(studio.generation_target)(){let target=crate::parts::Target::capture(&store,&doc,&track)?;crate::parts::create_job(request,&root,&target)?;}else{generation::create_job(request,&root)?;}
                            let reference=crate::song_layout::resolve(&store.root.join("source"),"chart-engine-lyrics.json");
                            let lyrics=if let Some(lyrics)=supplied_lyrics(){Some(lyrics)}else if crate::storage::is_file(&reference){
                                let value:serde_json::Value=serde_json::from_slice(&crate::storage::read(reference)?)?;
                                value["plainLyrics"].as_str().filter(|text|!text.trim().is_empty()).map(|text|crate::vocal_generation::LyricSource{text:text.into(),source:value["source"].as_str().filter(|source|!source.trim().is_empty()).map(str::to_owned).unwrap_or_else(||value["id"].as_u64().map_or_else(||"LRCLIB matched recording".into(),|id|format!("https://lrclib.net/api/get/{id}"))),synced_lyrics:value["syncedLyrics"].as_str().filter(|text|!text.trim().is_empty()).map(str::to_owned)})
                            }else{None};
                            if let Some(lyrics)=lyrics{generation::write_json(&root.join("supplied-lyrics.json"),&lyrics)?;}
                            generation::freeze(&root,metadata)?;
                            if let Some(manifest)=&doc.content.stems{generation::reuse_stems(&root,manifest)?;}
                            let revision=generation::checkpoint(&root,None).await?;
                            Ok::<_,anyhow::Error>(Job{root:root.clone(),revision})
                            }.await;
                            if created.is_err(){crate::storage::clear(&root);}
                            created
                        }.await;
                        match result{Ok(created)=>{runner.job.set(Some(created.clone()));runner.view.set(view(&created.root).ok());runner.start(studio,created);},Err(failure)=>{error.set(Some(format!("{failure:#}")));busy.set(false);}}
                    });
                },"Generate selected parts"}
            }else{button{disabled:busy(),onclick:move |_|{studio.generation_after_import.set(true);studio.generation_open.set(false);studio.new_chart();},"Choose recording…"}}
            if busy()&&stop.read().is_some(){button{onclick:move |_|{if let Some(control)=stop.read().as_ref(){control.0.set(true);if let Some(sender)=control.1.borrow_mut().take(){let _=sender.send(());}}},"Cancel generation"}}
            if !status().is_empty(){p{role:"status","{status}"}}
            if let Some(failure)=error(){p{role:"alert","{failure}"}}
            if let (Some(handle),Some(result))=(job(),current()){
                h3{"{result.title}"}
                p{"Saved job parts: {result.requested}"}
                p{"{result.status.detail}"}
                if let Some(frames)=result.summary.frames{p{"{frames} stereo frames · source checksums verified"}}
                ul{for(part,count)in &result.summary.instruments{li{"{part}: {count} detections"}}}
                if result.summary.vocal_notes>0||result.summary.vocal_issues>0{p{"{result.summary.vocal_notes} vocal notes · {result.summary.vocal_issues} review issues"}}
                if result.status.state!=State::Complete{button{disabled:busy(),onclick:{let handle=handle.clone();move |_|runner.start(studio,handle.clone())},"Resume remaining stages"}}
                if result.status.state==State::Complete{
                    if let Some(part)=result.part{
                        button{disabled:busy(),onclick:{let root=handle.root.clone();move |_|{
                            error.set(None);
                            let result=async_apply(studio,root.clone());let mut runner=runner;busy.set(true);
                            spawn(async move{match result.await{Ok(())=>{studio.generation_open.set(false);studio.generation_target.set(None);},Err(failure)=>runner.error.set(Some(format!("{failure:#}")))}runner.busy.set(false);});
                        }},"Apply generated {part}"}
                    }
                    button{disabled:busy(),onclick:{let root=handle.root.clone();move |_|{let root=root.clone();busy.set(true);spawn(async move{
                        let result=async{studio.persist_current().await?;let copy=crate::files::unique_root();crate::pipeline::copy_folder(&generation::project_path(&root),&copy)?;studio.open_project(copy).await}.await;
                        match result{Ok(())=>{studio.detach_library();studio.generation_open.set(false);studio.generation_target.set(None);},Err(failure)=>error.set(Some(format!("{failure:#}")))}busy.set(false);
                    });}},"Open as separate project"}
                }
                button{disabled:busy()||!crate::storage::is_file(handle.root.join("stems/stem-manifest.json")),onclick:{let root=handle.root.clone();move |_|{let root=root.clone();spawn(async move{let folder=root.join("stems");let path=download_path("separated-parts.zip");let result=crate::worker::run::<serde_json::Value>(crate::worker::Task::Archive{root:folder.clone(),output:path.clone()},&[folder]).await;match result{Ok(_)=>offer_download(studio,path,"separated-parts.zip",DownloadKind::File),Err(failure)=>error.set(Some(format!("{failure:#}")))}});}},"Save separated parts…"}
                if result.status.state==State::Complete{button{disabled:busy(),onclick:{let root=handle.root.clone();move |_|{let root=generation::project_path(&root);spawn(async move{let path=download_path("generated-project.zip");let result=crate::worker::run::<serde_json::Value>(crate::worker::Task::Backup{root:root.clone(),output:path.clone()},&[root]).await;match result{Ok(_)=>offer_download(studio,path,"generated-project.zip",DownloadKind::Project),Err(failure)=>error.set(Some(format!("{failure:#}")))}});}},"Save generated project ZIP…"}}
                for(relative,name)in[("transcription.json","transcription.json"),("vocals/generation.json","vocal-generation.json"),("vocals/recognition.json","recognition.json"),("vocals/alignment.json","alignment.json")]{
                    if crate::storage::is_file(handle.root.join(relative)){button{onclick:{let source=handle.root.join(relative);move |_|{let path=download_path(name);match crate::storage::copy(&source,&path){Ok(_)=>offer_download(studio,path,name,DownloadKind::File),Err(failure)=>error.set(Some(format!("{failure:#}")))}}},"Save {name}…"}}
                }
            }
            if !saved.is_empty(){details{summary{"Saved generation jobs"}for(root,title)in saved{button{disabled:busy(),onclick:move |_|{let root=root.clone();busy.set(true);error.set(None);spawn(async move{let result=async{let(_,revision)=crate::persistence::load(&root).await?.context("Saved job is missing")?;generation::validate_saved(&root)?;Ok::<_,anyhow::Error>(Job{root,revision})}.await;match result{Ok(loaded)=>{current.set(view(&loaded.root).ok());job.set(Some(loaded));status.set("Saved generation job opened".into());},Err(failure)=>error.set(Some(format!("{failure:#}")))}busy.set(false);});},"{title}"}}}}
            button{onclick:move |_|{studio.generation_open.set(false);studio.generation_target.set(None);},"Close"}
        }
    }}
}

async fn async_apply(studio: Studio, root: PathBuf) -> Result<()> {
    let store = (studio.store)().context("Open the target chart before applying this part")?;
    let doc = studio.snapshot().context("Open the target chart first")?;
    ensure!(
        generation::status(&root)?.state == State::Complete,
        "This generation job is not complete"
    );
    let generated_store = crate::editor::Store::new(&generation::project_path(&root));
    let generated = generated_store.load()?;
    for manifest in generated
        .content
        .stems
        .iter()
        .chain(generated.content.keyboard_stems.iter())
    {
        manifest.validate_binding(&doc.audio_sha256, doc.duration)?;
        crate::models::remember_stems(manifest)?;
    }
    crate::parts::Prepared::load(&store, &doc, &root)?;
    let previous = doc.revision;
    studio.dispatch(crate::editor::Action::ApplyGeneratedPart { job: root });
    ensure!(
        studio
            .snapshot()
            .is_some_and(|current| current.revision > previous),
        "The generated part was not applied; your current chart and draft were kept"
    );
    Ok(())
}
fn download_path(name: &str) -> PathBuf {
    PathBuf::from(format!(
        "/downloads/{}/{}",
        crate::storage::unique_id(),
        name
    ))
}
fn offer_download(mut studio: Studio, path: PathBuf, name: &str, kind: DownloadKind) {
    if let Some((old, _, _)) = studio.export_ready.replace(Some((path, name.into(), kind))) {
        crate::storage::clear(&old);
    }
}
