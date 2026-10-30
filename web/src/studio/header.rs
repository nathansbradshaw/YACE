use super::state::Studio;
use dioxus::prelude::*;
#[component]
pub fn Header(mut studio: Studio) -> Element {
    let loaded = studio.snapshot().is_some();
    let title = studio.metadata_value("name");
    let artist = studio.metadata_value("artist");
    let title = if title.is_empty() {
        "YACE".to_owned()
    } else {
        title
    };
    rsx! {header{class:"project-bar",onkeydown:super::shortcuts::keep_space_local,
    button{class:"project-menu-trigger","popovertarget":"project-menu",aria_label:"Project actions",super::brand::BrandMark{size:28}strong{class:"yace-wordmark","YACE"}}div{class:"project-identity",strong{"{title}"}span{"{artist}"}}
       nav{class:"project-actions",aria_label:"Chart workflow",button{disabled:true,title:"The current model backends cannot run in-browser","Generate"}button{disabled:!loaded,onclick:move |_|studio.review_open.set(true),"Review"}button{disabled:!loaded,onclick:move |_|studio.export_dialog(),"Export"}}
       button{disabled:!loaded,onclick:move |_|studio.save_project(),"Save project ZIP"}button{disabled:!loaded,onclick:move |_|studio.collaboration_open.set(true),"Collaborate"}}
       div{id:"project-menu","popover":"auto",class:"command-popover project-popover",
       button{"popovertarget":"project-menu","popovertargetaction":"hide",onclick:move |_|studio.open_dialog(),"Open saved project…"}
       button{"popovertarget":"project-menu","popovertargetaction":"hide",onclick:move |_|studio.import(false),"Import song ZIP…"}
       button{"popovertarget":"project-menu","popovertargetaction":"hide",onclick:move |_|studio.create_dialog(),"Open song folder…"}
       }}
}
#[component]
pub fn Welcome(studio: Studio) -> Element {
    rsx! {main{class:"studio-welcome",aria_label:"Start a chart project",div{class:"welcome-content",h1{"Start a chart project"}p{class:"welcome-intro","Chart locally in your browser. No chart server or account is required."}div{class:"welcome-actions",button{class:"welcome-action",onclick:move |_|studio.import(false),super::icons::Icon{name:"music",size:20}div{strong{"Import song ZIP"}span{"Open an existing chart and its recording."}}}button{class:"welcome-action",onclick:move |_|studio.create_dialog(),super::icons::Icon{name:"folder-open",size:20}div{strong{"Open song folder"}span{"Choose a local chart folder."}}}button{class:"welcome-action",onclick:move |_|studio.open_dialog(),super::icons::Icon{name:"folder-open",size:20}div{strong{"Open saved project"}span{"Resume a project ZIP with undo and review history."}}}}p{class:"welcome-hint","Generation is unavailable in this browser build. Existing charts and manual authoring work locally."}}}}
}
