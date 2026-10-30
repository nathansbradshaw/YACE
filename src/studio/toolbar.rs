//! File, playback and chart tools. Component ancestry: THIRD_PARTY.md.
use super::{icons::Icon, state::Studio};
use crate::editor::Action;
use dioxus::prelude::*;
#[component]
pub fn Toolbar(mut studio: Studio) -> Element {
    let doc = studio.snapshot();
    let loaded = doc.is_some();
    let playing = doc.as_ref().is_some_and(|d| d.transport.playing);
    let can_undo = doc.as_ref().is_some_and(|d| !d.undo.is_empty());
    let can_redo = doc.as_ref().is_some_and(|d| !d.redo.is_empty());
    let name = studio.metadata_value("name");
    let name = if name.is_empty() {
        doc.as_ref().map(|d| d.title.clone()).unwrap_or_default()
    } else {
        name
    };
    let artist = studio.metadata_value("artist");
    let play_icon = if playing { "pause" } else { "play" };
    let volume = (studio.volume)();
    rsx! {
        div { class: "toolbar", onkeydown: move |event: KeyboardEvent| {
            if event.key() == Key::Character(" ".into()) { event.stop_propagation(); }
        },
            div { class: "toolbar-wear", aria_hidden: "true", dangerous_inner_html: include_str!("../../assets/toolbar-wear.svg") }
            span { class: "toolbar-brand", "YACE" }
            div { class: "toolbar-group toolbar-file",
                button { class: "toolbar-button", title: "Open saved review project", aria_label: "Open saved review project", onclick: move |_| studio.open_dialog(),
                    span { class: "toolbar-icon", Icon { name: "folder" } } span { class: "toolbar-label", "Open" }
                }
                button { class: "toolbar-button", title: "Create a review project from a song folder", aria_label: "Create a review project from a song folder", onclick: move |_| studio.create_dialog(),
                    span { class: "toolbar-icon", Icon { name: "file-plus" } } span { class: "toolbar-label", "New" }
                }
                if loaded { span { class: "toolbar-save-status", title: "Chart edits save automatically after each successful change", Icon { name: "check", size: 15 } span { "Saved" } } }
                button { class: "toolbar-button", title: "Export a separate song folder", aria_label: "Export a separate song folder", disabled: !loaded, onclick: move |_| studio.export_dialog(),
                    span { class: "toolbar-icon", Icon { name: "export" } } span { class: "toolbar-label", "Export" }
                }
            }
            div { class: "toolbar-separator" }
            div { class: "toolbar-group toolbar-edit",
                button { class: "toolbar-button", title: "Undo (Command/Ctrl+Z)", aria_label: "Undo (Command/Ctrl+Z)", disabled: !can_undo, onclick: move |_| studio.dispatch(Action::Undo), span { class: "toolbar-icon", Icon { name: "undo" } } }
                button { class: "toolbar-button", title: "Redo (Command/Ctrl+Shift+Z)", aria_label: "Redo (Command/Ctrl+Shift+Z)", disabled: !can_redo, onclick: move |_| studio.dispatch(Action::Redo), span { class: "toolbar-icon", Icon { name: "redo" } } }
            }
            div { class: "toolbar-separator" }
            div { class: "toolbar-group toolbar-playback",
                button { class: "toolbar-button toolbar-button-play", title: "Play / Pause (Space)", aria_label: "Play / Pause (Space)", disabled: !loaded || !(studio.audio_ready)(), onclick: move |_| studio.toggle_playback(), span { class: "toolbar-icon", Icon { name: play_icon } } }
                button { class: "toolbar-button", title: "Stop and return to beginning", aria_label: "Stop and return to beginning", disabled: !loaded, onclick: move |_| { studio.dispatch(Action::Playback { playing: false }); studio.seek(0.0); }, span { class: "toolbar-icon", Icon { name: "square" } } }
            }
            div { class: "toolbar-group toolbar-volume",
                span { class: "toolbar-icon toolbar-volume-icon", Icon { name: "volume" } }
                input { onkeydown: move |event| event.stop_propagation(), class: "toolbar-volume-slider", r#type: "range", min: "0", max: "1", step: "0.01", value: "{volume}",
                    style: "--slider-fill: {volume * 100.0}%;", title: "Recording volume", disabled: !loaded,
                    oninput: move |event| { if let Ok(value) = event.value().parse() { studio.update_volume(value); } }
                }
            }
            div { class: "toolbar-separator" }
            div { class: "toolbar-song-info",
                if loaded {
                    span { class: "toolbar-song-name", title: "{name}", "{name}" }
                    span { class: "toolbar-song-artist", "{artist}" }
                } else { span { class: "toolbar-no-song", "No song loaded" } }
            }
            div { class: "toolbar-spacer" }
            div { class: "toolbar-group toolbar-autochart",
                button { class: "toolbar-button toolbar-button-accent", title: "Import local audio and generate charts", aria_label: "Import local audio and generate charts", onclick: move |_| studio.generation_open.set(true),
                    span { class: "toolbar-icon", Icon { name: "music" } } span { class: "toolbar-label", "Auto-Chart" }
                }
            }
            div { class: "toolbar-group toolbar-validate",
                button { class: "toolbar-button", disabled: !loaded, title: if loaded { format!("AI help for {name}") } else { "Open a project to use the charting assistant".into() }, aria_label: "AI charting assistant", onclick: move |_| studio.assistant_open.set(true),
                    span { class: "toolbar-icon", Icon { name: "comment" } } span { class: "toolbar-label", "Assistant" }
                }
                super::collaboration::Collaboration { studio }
                button { class: "toolbar-button", disabled: !loaded, title: "Review timed generation and pattern flags", aria_label: "Review timed generation and pattern flags", onclick: move |_| studio.review_open.set(true),
                    span { class: "toolbar-icon", Icon { name: "check" } } span { class: "toolbar-label", "Review" }
                }
                button { class: "toolbar-button", disabled: !loaded, title: "Find explicit lyrics automatically and review clean-audio edits", aria_label: "Find explicit lyrics automatically and review clean-audio edits", onclick: move |_| studio.clean_open.set(true),
                    span { class: "toolbar-icon", Icon { name: "music" } } span { class: "toolbar-label", "Clean audio" }
                }
            }
        }
    }
}
