use super::state::Studio;
use dioxus::prelude::*;
#[component]
pub fn Collaboration(mut studio: Studio) -> Element {
    rsx! {super::modal::Modal{id:"browser-reviews",label:"Collaborative song reviews",onclose:move |_|studio.collaboration_open.set(false),
    h2{"Collaborative reviews"}p{"Exchange portable review bundles. Audio and source charts stay local."}
    label{"Your name" input{aria_label:"Your name",value:studio.reviewer,oninput:move |event|studio.reviewer.set(event.value())}}
    button{onclick:move |_|{let result=(||->anyhow::Result<_>{let doc=studio.snapshot().ok_or_else(||anyhow::anyhow!("Open a project"))?;let bundle=crate::collaboration_exchange::export_bundle(&doc)?;crate::files::download("reviews.json",&serde_json::to_vec_pretty(&bundle)?)} )();if let Err(error)=result{studio.error.set(Some(format!("{error:#}")));}},"Export complete review bundle…"}
    button{onclick:move |_|{spawn(async move{let result=async{let Some(files)=crate::files::pick(false,".json").await?else{return Ok(())};let bundle=serde_json::from_slice(&files[0].1)?;studio.dispatch(crate::editor::Action::ImportCollaboration{bundle});Ok::<_,anyhow::Error>(())}.await;if let Err(error)=result{studio.error.set(Some(format!("{error:#}")));}});},"Import review bundle…"}
    if let Some(error)=(studio.error)(){p{role:"alert","{error}"}}
    button{onclick:move |_|studio.collaboration_open.set(false),"Close"}
    }}
}
