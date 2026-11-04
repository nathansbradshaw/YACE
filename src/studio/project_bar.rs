//! Project actions stay separate from note editing and playback.
use super::{brand::BrandMark, icons::Icon, shortcuts::keep_space_local, state::Studio};
use dioxus::prelude::*;

#[component]
pub(super) fn ExportSummary(studio: Studio) -> Element {
    if super::dance::active() || studio.snapshot().is_none() {
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
    let mut backup_open = use_signal(|| false);
    let mut backup_stems = use_signal(|| true);
    let mut backup_quality = use_signal(|| crate::compact_backup::StemQuality::Listening);
    let document = studio.snapshot();
    #[cfg(target_arch = "wasm32")]
    let direct_folders = crate::project_folder::supported();
    #[cfg(target_arch = "wasm32")]
    let linked = super::dance::standalone_folder()
        .or_else(|| studio.store.read().as_ref().map(|store| store.root.clone()))
        .is_some_and(|root| crate::project_folder::linked(&root));
    let loaded = document.is_some() || super::dance::active();
    let band = !super::dance::active();
    let title = studio.metadata_value("name");
    let title = if title.is_empty() {
        document
            .as_ref()
            .map_or("No project".into(), |doc| doc.title.clone())
    } else {
        title
    };
    let artist = studio.metadata_value("artist");
    let (title, artist) = super::dance::identity().unwrap_or((title, artist));
    let file_action = (studio.file_activity)();
    let file_busy = file_action.is_some() || (studio.library_busy)();
    let package_busy = (studio.export_busy)();
    let generation_busy = (studio.generation_busy)();
    let package_label = if file_action == Some(super::activity::FileAction::Backup) {
        "Preparing backup…"
    } else {
        "Preparing song…"
    };
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
                            disabled: file_busy,
                            aria_busy: file_busy,
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
                button { class: "project-generate", disabled:!loaded && file_busy, aria_busy:generation_busy, onclick: move |_| super::dance::generate(studio), title: "Generate charts from local audio or reopen analysis details", Icon { name: "music", size: 15 } super::activity::BusyLabel{busy:generation_busy,label:"Generate",active:"Analysis running…"} }
                button { disabled: !loaded, onclick: move |_| super::dance::review(studio), title: "Review timed chart flags", Icon { name: "flag", size: 15 } "Review" }
                button { disabled: !loaded || package_busy, aria_busy:package_busy, onclick: move |_| super::dance::export(studio), title: "Choose tracks, format and audio for a separate song copy", Icon { name: "export", size: 15 } super::activity::BusyLabel{busy:package_busy,label:"Export playable song…",active:package_label} }
            }
            button { "popovertarget": "project-tools", aria_label: "More tools", "Tools" Icon { name: "chevron-down", size: 12 } }
        }
        div { id: "project-actions", class: "command-popover project-popover", "popover": "auto", onkeydown: keep_space_local,
            button{disabled:file_busy,"popovertarget":"project-actions","popovertargetaction":"hide",onclick:move |_|studio.new_blank_chart(),Icon{name:"file-plus",size:15}"New blank chart…"}
            button { disabled:file_busy,aria_busy:file_action==Some(super::activity::FileAction::Recording),"popovertarget":"project-actions","popovertargetaction":"hide",onclick:move |_|super::dance::new_recording(studio,false),Icon{name:"music",size:15}super::activity::BusyLabel{busy:file_action==Some(super::activity::FileAction::Recording),label:"New chart from audio…",active:"Choosing recording…"}}
            button { disabled: file_busy, aria_busy:file_action==Some(super::activity::FileAction::Open), "popovertarget": "project-actions", "popovertargetaction": "hide", "data-browser-pick": "file", "data-browser-accept":".zip", onclick: move |_| studio.open_dialog(), Icon { name: "folder-open", size: 15 } super::activity::BusyLabel{busy:file_action==Some(super::activity::FileAction::Open),label:if cfg!(feature="browser"){ "Open project ZIP…" }else{"Open project or charting folder…"},active:"Opening project…"} }
            {
                #[cfg(target_arch = "wasm32")]
                {rsx! {
                    if direct_folders {
                        button { disabled:file_busy, "popovertarget":"project-actions","popovertargetaction":"hide", onclick: move |_| studio.open_project_folder(), Icon { name: "folder-open", size: 15 } "Open project folder…" }
                        if loaded && !linked {
                            button { disabled:file_busy || package_busy, "popovertarget":"project-actions","popovertargetaction":"hide", title:"Choose an empty folder for the complete editable project", onclick: move |_| studio.save_to_project_folder(), "Save to project folder…" }
                        }
                    }
                }}
                #[cfg(not(target_arch = "wasm32"))]
                {rsx! {}}
            }
            if cfg!(feature="browser") {button { disabled:file_busy,aria_busy:file_action==Some(super::activity::FileAction::Open),"popovertarget": "project-actions", "popovertargetaction": "hide", "data-browser-pick": "folder", onclick: move |_| studio.create_dialog(), Icon { name: "file-plus", size: 15 } "Open charting folder…" }}
            if !cfg!(target_arch="wasm32") {button{disabled:file_busy,"popovertarget":"project-actions","popovertargetaction":"hide","data-browser-pick":"file","data-browser-accept":".zip",onclick:move |_|studio.open_zip(),"Import project ZIP…"}}
            button { disabled:file_busy,aria_busy:file_action==Some(super::activity::FileAction::Import),"popovertarget":"project-actions","popovertargetaction":"hide","data-browser-pick":"file","data-browser-accept":".zip",onclick:move |_|studio.import_song(),super::activity::BusyLabel{busy:file_action==Some(super::activity::FileAction::Import),label:"Import song ZIP…",active:"Importing song…"} }
            button {disabled:!loaded || package_busy || file_busy,aria_busy:file_action==Some(super::activity::FileAction::Backup),"popovertarget":"project-actions","popovertargetaction":"hide",title:"Editable ZIP with original audio, exact stems and undo history",onclick:move |_|super::dance::save_with_options(studio, crate::compact_backup::Options::default()),super::activity::BusyLabel{busy:file_action==Some(super::activity::FileAction::Backup),label:if cfg!(target_arch="wasm32"){"Download complete project…"}else{"Save complete project ZIP…"},active:"Preparing project ZIP…"}}
            if cfg!(target_arch="wasm32") && band {button {disabled:studio.doc.read().is_none() || package_busy,"popovertarget":"project-actions","popovertargetaction":"hide",title:"Preserve the open editor and available files even when normal backup fails",onclick:move |_| {
                #[cfg(target_arch="wasm32")]
                studio.recover_open_project();
            },"Download recovery copy…"}}
            if cfg!(target_arch="wasm32") {button {"popovertarget":"project-actions","popovertargetaction":"hide",onclick:move |_| {
                #[cfg(target_arch="wasm32")]
                studio.open_saved_projects();
            },"Saved in this browser…"}}
        }
        div { id: "project-tools", class: "command-popover tools-popover", "popover": "auto", onkeydown: keep_space_local,
            button { disabled:!loaded || package_busy || file_busy,"popovertarget":"project-tools","popovertargetaction":"hide",onclick:move |_|backup_open.set(true),"Prepare smaller project backup…" }
            button { title: if !band {"Chart assistant is unavailable for this chart type".to_owned()}else if cfg!(target_arch="wasm32"){ "The current assistant backends require native runtimes".to_owned() }else{ format!("Assistant for {title}") }, aria_label: "Chart assistant", disabled: !band || !loaded || cfg!(target_arch="wasm32"), "popovertarget": "project-tools", "popovertargetaction": "hide", onclick: move |_| studio.assistant_open.set(true), "Chart assistant" }
            button { disabled: !band || document.as_ref().is_none_or(|doc|!doc.has_recording()),title:"Attach a recording before cleaning audio", "popovertarget": "project-tools", "popovertargetaction": "hide", onclick: move |_| studio.clean_open.set(true), "Clean audio" }
            button { disabled: !band || !loaded, "popovertarget": "project-tools", "popovertargetaction": "hide", onclick: move |_| studio.collaboration_open.set(true), "Collaborate" }
        }
        if backup_open() {
            super::sheet::ToolSheet {
                id: "project-backup-options", title: "Compact project backup",
                description: "Keep your original recording, charts and complete editing history.",
                close_label: "Close backup options", class: "export-sheet",
                onclose: move |_| backup_open.set(false),
                actions: rsx! {
                    button { onclick: move |_| backup_open.set(false), "Cancel" }
                    button { disabled: !loaded || file_busy || package_busy, onclick: move |_| {
                        backup_open.set(false);
                        super::dance::save_with_options(studio, crate::compact_backup::Options { include_generated_stems: backup_stems(),stem_quality:backup_quality() });
                    }, "Prepare backup" }
                },
                label { class: "checkbox-row",
                    input { r#type: "checkbox", checked: backup_stems(), onchange: move |event| backup_stems.set(event.checked()) }
                    "Include generated stems"
                }
                if backup_stems() {
                    label { "Stem quality"
                        select { value:if backup_quality()==crate::compact_backup::StemQuality::Listening {"listening"}else{"exact"},onchange:move |event|backup_quality.set(if event.value()=="exact"{crate::compact_backup::StemQuality::Exact}else{crate::compact_backup::StemQuality::Listening}),
                            option {value:"listening","High quality · 24-bit listening copies"}
                            option {value:"exact","Exact · original generated samples"}
                        }
                    }
                    if backup_quality()==crate::compact_backup::StemQuality::Listening {
                        p { "Smaller isolated parts for charting. Uses 24-bit precision with preserved headroom, stereo and timing. Tiny rounding differences are recorded; your original recording stays exact." }
                        p { class:"field-hint", "Saved audio checks may need reanalysis after restoring listening copies. Choose Exact to keep their original samples and hashes." }
                    } else {p { "Preserves every generated sample and file checksum. Exact backups are larger." }}
                } else {
                    p { "Stems will be unavailable after restoring this backup until you regenerate or reattach them. The original recording and all chart edits and undo history are still included." }
                }
                p { class: "field-hint", "Repeated assets are stored once. Open this backup in an updated YACE; older versions cannot read the compact format." }
            }
        }
    }
}
