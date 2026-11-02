//! Project actions stay separate from note editing and playback.
use super::{brand::BrandMark, icons::Icon, shortcuts::keep_space_local, state::Studio};
use dioxus::prelude::*;

#[component]
pub(super) fn ExportSummary(studio: Studio) -> Element {
    if studio.snapshot().is_none() {
        return rsx! {};
    }
    let state = (studio.export_state)();
    let text = state.summary();
    let title = state
        .error
        .clone()
        .or_else(|| {
            state
                .receipt
                .as_ref()
                .map(|r| format!("{} · {}", text, r.destination))
        })
        .unwrap_or_else(|| {
            "Project autosave keeps editing state. Export a song copy separately.".into()
        });
    rsx! {span{role:"status",aria_label:"Song export status",title,style:"overflow:hidden;text-overflow:ellipsis;white-space:nowrap;max-width:55%","{text}"}}
}

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
            if !cfg!(feature = "browser") || cfg!(target_arch = "wasm32") {
                if let Some(library) = (studio.library)() {
                    label { class: "chart-library-picker", title: "{library.root.display()}",
                        span { "Charts " }
                        select {
                            aria_label: "Switch chart in charting folder",
                            disabled: (studio.library_busy)(),
                            value: (studio.library_selected)().map_or(String::new(), |index| index.to_string()),
                            onchange: move |event| {
                                #[cfg(any(not(feature = "browser"), target_arch = "wasm32"))]
                                if let Ok(index) = event.value().parse::<usize>() {
                                    dioxus::core::spawn_forever(async move { studio.open_library_chart(index).await; });
                                }
                                #[cfg(all(feature = "browser", not(target_arch = "wasm32")))]
                                let _ = event;
                            },
                            option { value: "", disabled: true, "Choose a chart ({library.entries.len()})" }
                            for (index, item) in library.entries.iter().enumerate() {
                                option { value: "{index}", "{item.title} · {item.path.strip_prefix(&library.root).unwrap_or(&item.path).display()}" }
                            }
                        }
                    }
                }
            }
            nav { class: "project-actions", aria_label: "Chart workflow",
                button { class: "project-generate", onclick: move |_| studio.generation_open.set(true), title: "Generate charts from local audio", Icon { name: "music", size: 15 } "Generate" }
                button { disabled: !loaded, onclick: move |_| studio.review_open.set(true), title: "Review timed chart flags", Icon { name: "flag", size: 15 } "Review" }
                button { disabled: !loaded || (studio.export_busy)(), onclick: move |_| studio.export_dialog(), title: "Choose tracks, format and audio for a separate song copy", Icon { name: "export", size: 15 } "Export playable song…" }
            }
            button { "popovertarget": "project-tools", aria_label: "More tools", "Tools" Icon { name: "chevron-down", size: 12 } }
        }
        div { id: "project-actions", class: "command-popover project-popover", "popover": "auto", onkeydown: keep_space_local,
            button{"popovertarget":"project-actions","popovertargetaction":"hide",onclick:move |_|studio.new_blank_chart(),Icon{name:"file-plus",size:15}"New blank chart…"}
            button { "popovertarget":"project-actions","popovertargetaction":"hide",onclick:move |_|studio.new_chart(),Icon{name:"music",size:15}"New chart from audio…"}
            button { disabled: (studio.library_busy)(), "popovertarget": "project-actions", "popovertargetaction": "hide", "data-browser-pick": "file", "data-browser-accept":".zip", onclick: move |_| studio.open_dialog(), Icon { name: "folder-open", size: 15 } if cfg!(feature="browser") { "Open saved project…" } else { "Open project or charting folder…" } }
            if cfg!(feature="browser") {button { disabled:(studio.library_busy)(),"popovertarget": "project-actions", "popovertargetaction": "hide", "data-browser-pick": "folder", onclick: move |_| studio.create_dialog(), Icon { name: "file-plus", size: 15 } "Open charting folder…" }}
            if !cfg!(target_arch="wasm32") {button{"popovertarget":"project-actions","popovertargetaction":"hide","data-browser-pick":"file","data-browser-accept":".zip",onclick:move |_|studio.open_zip(),"Import project ZIP…"}}
            button { "popovertarget":"project-actions","popovertargetaction":"hide","data-browser-pick":"file","data-browser-accept":".zip",onclick:move |_|studio.import_song(),"Import song ZIP…" }
            button {disabled:!loaded || (studio.export_busy)(),"popovertarget":"project-actions","popovertargetaction":"hide",title:"Complete editable project ZIP with source files and undo history",onclick:move |_|studio.save_project(),if cfg!(target_arch="wasm32"){"Download project backup…"}else{"Save project backup…"}}
            if cfg!(target_arch="wasm32") {button {"popovertarget":"project-actions","popovertargetaction":"hide",onclick:move |_| {
                #[cfg(target_arch="wasm32")]
                studio.open_saved_projects();
            },"Saved in this browser…"}}
        }
        div { id: "project-tools", class: "command-popover tools-popover", "popover": "auto", onkeydown: keep_space_local,
            button { title: if cfg!(target_arch="wasm32"){ "The current assistant backends require native runtimes".to_owned() }else{ format!("Assistant for {title}") }, aria_label: "Chart assistant", disabled: !loaded || cfg!(target_arch="wasm32"), "popovertarget": "project-tools", "popovertargetaction": "hide", onclick: move |_| studio.assistant_open.set(true), "Chart assistant" }
            button { disabled: document.as_ref().is_none_or(|doc|!doc.has_recording()),title:"Attach a recording before cleaning audio", "popovertarget": "project-tools", "popovertargetaction": "hide", onclick: move |_| studio.clean_open.set(true), "Clean audio" }
            button { disabled: !loaded, "popovertarget": "project-tools", "popovertargetaction": "hide", onclick: move |_| studio.collaboration_open.set(true), "Collaborate" }
        }
    }
}
