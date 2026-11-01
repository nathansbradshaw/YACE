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
    rsx! {Modal{id:"browser-generation",label:"Generate charts",onclose:move |_|studio.generation_open.set(false),
        section{class:"sections-dialog",
            h2{"Generate charts"}
            p{"Process the recording locally. Models are verified and cached in this browser."}
            if let Some(source)=source {
                p{"{source.file_name().unwrap_or_default().to_string_lossy()}"}
                button{disabled:busy(),onclick:move |_|{
                    let audio=source.clone();
                    let output=PathBuf::from(format!("/generated/{}",crate::storage::unique_id()));
                    let (sender,receiver)=futures_channel::oneshot::channel();
                    cancel.set(Some(Rc::new(RefCell::new(Some(sender)))));
                    if let Some((previous,_))=ready.take(){crate::storage::clear(&previous);}
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
            } else {
                p{"Choose a recording to start."}
                button{onclick:move |_|{studio.generation_open.set(false);studio.new_chart();},"Choose recording…"}
            }
            if busy(){button{onclick:move |_|{if let Some(cancel)=cancel.read().as_ref()&&let Some(sender)=cancel.borrow_mut().take(){let _=sender.send(());}},"Cancel generation"}}
            if !status().is_empty(){p{role:"status","{status}"}}
            if let Some(error)=failure(){p{role:"alert","{error}"}}
            if let Some((root,manifest))=ready(){
                p{"{manifest.frames} stereo frames · 44.1 kHz · source checksums verified"}
                ul{for asset in &manifest.stems {li{"{asset.id.as_str()}"}}}
                button{onclick:move |_|{
                    let root=root.clone();spawn(async move{
                        // This explicit export is small for the initial backend
                        // check; the final generator will archive on its worker.
                        let result=crate::files::archive(&root).and_then(|bytes|{
                            let path=root.with_extension("zip");crate::storage::write_owned(&path,bytes)?;
                            if let Some((old,_,_))=studio.export_ready.replace(Some((path,"separated-parts.zip".into(),DownloadKind::File))){crate::storage::clear(&old);}Ok(())
                        });
                        if let Err(error)=result{failure.set(Some(format!("{error:#}")));}
                    });
                },"Save separated parts…"}
            }
            button{onclick:move |_|studio.generation_open.set(false),"Close"}
        }
    }}
}
