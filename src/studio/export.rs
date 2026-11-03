//! The same export choices and validation on desktop and in the browser.
use super::{
    sheet::ToolSheet,
    state::{self, Studio},
};
use crate::export_options::{self, Format, Options, Target};
use dioxus::prelude::*;

#[component]
pub fn ExportDialog(mut studio: Studio, #[props(default)] failure: String) -> Element {
    let context = use_hook(move || {
        let store = studio
            .store
            .read()
            .clone()
            .ok_or_else(|| "Open a project".to_owned())?;
        let doc = studio
            .snapshot()
            .ok_or_else(|| "Open a project".to_owned())?;
        let tracks = export_options::available(&store, &doc).map_err(|e| format!("{e:#}"))?;
        let with_audio = export_options::source_size(&store, &doc, true).ok();
        let without_audio = export_options::source_size(&store, &doc, false).ok();
        Ok::<_, String>((store, doc, tracks, with_audio, without_audio))
    });
    let initial = context
        .as_ref()
        .ok()
        .map(|(_, doc, tracks, _, _)| Options {
            tracks: Some(tracks.clone()),
            include_audio: doc.has_recording(),
            ..Default::default()
        })
        .unwrap_or_default();
    let mut options = use_signal(|| initial);
    let busy = (studio.export_busy)();
    let close = EventHandler::new(move |_: ()| {
        if !(studio.export_busy)() {
            studio.export_open.set(false);
        }
    });
    let (store, doc, tracks, with_audio, without_audio) = match context {
        Ok(context) => context,
        Err(error) => {
            return rsx! {super::modal::Modal {id: "export-options", label: "Export playable song", onclose: close,
            article {class: "tool-sheet", div {class: "tool-sheet-content", p {role: "alert", "{error}"} button {onclick: move |_| close.call(()), "Close"}}}
            }};
        }
    };
    let selected = options();
    let imported_midi = crate::song_files::game_midi::exists(&store.root.join("source"));
    let validation = selected
        .validate(&tracks, imported_midi)
        .err()
        .map(|e| e.to_string());
    let chosen = selected.tracks.as_ref().unwrap_or(&tracks);
    let midi = selected.use_midi(chosen, imported_midi);
    let estimated = if selected.include_audio {
        with_audio
    } else {
        without_audio
    };
    let size = estimated.map(|n| format!("{:.1} MiB", n as f64 / 1_048_576.));
    let root = store.root.clone();
    let revision = doc.revision;
    let all_tracks = tracks.clone();
    rsx! {
        ToolSheet {
            id: "export-options", title: "Export playable song", description: "Choose what goes into a separate song copy. Your complete editing project stays saved.",
            close_label: "Close export", class: "export-sheet", onclose: close,
            actions: rsx! {
                if !failure.is_empty() {
                    p {class: "generation-validation", role: "alert", aria_label: "Song export error", style: "margin:0;flex:1;overflow-wrap:anywhere", "{failure}"}
                }
                button {disabled: busy, onclick: move |_| close.call(()), "Cancel"}
                button {class: "primary", disabled: busy || validation.is_some(), onclick: move |_| studio.export_selected(options(), root.clone(), revision),
                    aria_busy:busy, super::activity::BusyLabel{busy,label:if cfg!(target_arch="wasm32"){"Prepare song ZIP"}else{"Choose export folder…"},active:"Preparing export…"}
                }
            },
            fieldset {disabled: busy,
                legend {"Compatibility"}
                label {r#for: "export-target", "Target"}
                select {id: "export-target", value: match selected.target {Target::Preserve=>"preserve",Target::Yarg=>"yarg",Target::CloneHero=>"clonehero"},
                    onchange: move |event| { options.write().target = match event.value().as_str() {"yarg"=>Target::Yarg,"clonehero"=>Target::CloneHero,_=>Target::Preserve}; },
                    option {value: "preserve", "Preserve all parts (recommended)"}
                    option {value: "yarg", "YARG"}
                    option {value: "clonehero", "Clone Hero"}
                }
                label {r#for: "export-format", "Chart format"}
                select {id: "export-format", value: match selected.format {Format::Automatic=>"auto",Format::Chart=>"chart",Format::Midi=>"midi"},
                    onchange: move |event| { options.write().format = match event.value().as_str() {"chart"=>Format::Chart,"midi"=>Format::Midi,_=>Format::Automatic}; },
                    option {value: "auto", "Automatic (recommended)"}
                    option {value: "chart", ".chart"}
                    option {value: "midi", "MIDI (.mid)"}
                }
                p {class: "field-hint", "Automatic uses MIDI for vocals, Pro Keys and imported MIDI songs. Clone Hero requires you to deselect vocals, Pro Keys and other original MIDI parts."}
            }
            fieldset {disabled: busy,
                legend {"Instruments and difficulties"}
                div {class: "export-selection-actions",
                    button {onclick: move |_| options.write().tracks = Some(all_tracks.clone()), "Select all"}
                    button {onclick: move |_| options.write().tracks = Some(Default::default()), "Clear selection"}
                }
                div {class: "export-tracks",
                    for track in tracks {
                        label {key: "{track}", class: "export-track",
                            input {r#type: "checkbox", checked: selected.contains(&track), onchange: {let track = track.clone(); move |event: FormEvent| {
                                let mut options = options.write();
                                let selected = options.tracks.get_or_insert_with(Default::default);
                                if event.checked() { selected.insert(track.clone()); } else { selected.remove(&track); }
                            }}}
                            if track.starts_with("MIDI: ") {"{track}"} else if track == "Vocals" {"Vocals"} else {
                                "{state::instrument(&track)} · {difficulty(&track)}"
                            }
                        }
                    }
                }
            }
            fieldset {disabled: busy,
                legend {"Audio"}
                label {class: "export-track",
                    input {r#type: "checkbox", disabled: !doc.has_recording(), checked: selected.include_audio, onchange: move |event| options.write().include_audio = event.checked()}
                    "Include recording and stems"
                }
                if !selected.include_audio {p {class: "field-hint", "Chart-only copy. Add matching audio before playing it in a game."}}
            }
            section {class: "export-preview", aria_label: "Package preview",
                h3 {"Package preview"}
                ul {
                    li {if midi {"notes.mid + editable chart in YACE supporting files"} else {"notes.chart"}}
                    li {"song.ini, artwork and other source files"}
                    if selected.include_audio {li {"Recording and available stems"}}
                    li {"YACE supporting files and source history"}
                }
                if let Some(size) = size {p {class: "field-hint", "Source files: approximately {size} before compression. Generated files and audio conversion can change the final size."}}
                p {class: "field-hint", "For undo history and a complete editable backup, use the YACE menu → " if cfg!(target_arch="wasm32") {"Download project backup"} else {"Save project backup"} "."}
            }
            if let Some(error) = validation {p {class: "generation-validation", role: "alert", "{error}"}}
        }
    }
}
fn difficulty(track: &str) -> &str {
    ["Easy", "Medium", "Hard", "Expert"]
        .into_iter()
        .find(|tier| track.starts_with(tier))
        .unwrap_or(track)
}
