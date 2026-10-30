//! Project actions stay separate from note editing and playback.
use super::{brand::BrandMark, icons::Icon, shortcuts::keep_space_local, state::Studio};
use dioxus::prelude::*;

#[component]
pub fn ProjectBar(mut studio: Studio) -> Element {
    let document = studio.snapshot();
    let loaded = document.is_some();
    let title = studio.metadata_value("name");
    let title = if title.is_empty() {
        document
            .as_ref()
            .map_or("No project".into(), |doc| doc.title.clone())
    } else {
        title
    };
    let artist = studio.metadata_value("artist");
    rsx! {
        header { class: "project-bar", onkeydown: keep_space_local,
            button { class: "project-menu-trigger", "popovertarget": "project-actions", aria_label: "Project actions",
                BrandMark { size: 25 }
                strong { class: "yace-wordmark", "YACE" }
                Icon { name: "chevron-down", size: 12 }
            }
            div { class: "project-identity",
                strong { title: "{title}", "{title}" }
                if !artist.is_empty() { span { title: "{artist}", "{artist}" } }
            }
            nav { class: "project-actions", aria_label: "Chart workflow",
                button { class: "project-generate", onclick: move |_| studio.generation_open.set(true), title: "Generate charts from local audio", Icon { name: "music", size: 15 } "Generate" }
                button { disabled: !loaded, onclick: move |_| studio.review_open.set(true), title: "Review timed chart flags", Icon { name: "flag", size: 15 } "Review" }
                button { disabled: !loaded, onclick: move |_| studio.export_dialog(), title: "Export a separate song folder", Icon { name: "export", size: 15 } "Export" }
            }
            button { "popovertarget": "project-tools", aria_label: "More tools", "Tools" Icon { name: "chevron-down", size: 12 } }
        }
        div { id: "project-actions", class: "command-popover project-popover", "popover": "auto", onkeydown: keep_space_local,
            button { "popovertarget": "project-actions", "popovertargetaction": "hide", "data-browser-pick": "file", "data-browser-accept":".zip", onclick: move |_| studio.open_dialog(), Icon { name: "folder-open", size: 15 } "Open saved project…" }
            button { "popovertarget": "project-actions", "popovertargetaction": "hide", "data-browser-pick": "folder", onclick: move |_| studio.create_dialog(), Icon { name: "file-plus", size: 15 } "Create from song folder…" }
            if cfg!(feature="browser") {
                button { "popovertarget":"project-actions","popovertargetaction":"hide","data-browser-pick":"file","data-browser-accept":".zip",onclick:move |_|studio.create_dialog(),"Import song ZIP…" }
                button {disabled:!loaded,"popovertarget":"project-actions","popovertargetaction":"hide",onclick:move |_|{if let Some(store)=studio.store.read().clone(){spawn(async move{super::dialogs::download(&store.root).await;});}},"Save project ZIP…"}
            }
        }
        div { id: "project-tools", class: "command-popover tools-popover", "popover": "auto", onkeydown: keep_space_local,
            button { title: "Assistant for {title}", aria_label: "Chart assistant", disabled: !loaded, "popovertarget": "project-tools", "popovertargetaction": "hide", onclick: move |_| studio.assistant_open.set(true), "Chart assistant" }
            button { disabled: !loaded, "popovertarget": "project-tools", "popovertargetaction": "hide", onclick: move |_| studio.clean_open.set(true), "Clean audio" }
            button { disabled: !loaded, "popovertarget": "project-tools", "popovertargetaction": "hide", onclick: move |_| studio.collaboration_open.set(true), "Collaborate" }
        }
    }
}
