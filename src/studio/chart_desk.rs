//! The timeline owns track selection, editing tools and edit history.
use super::{
    icons::Icon,
    note_canvas::{Ruler, TrackRoll, VocalRoll, Waveform},
    state::{EditTool, Studio},
    track_picker::TrackPicker,
};
use dioxus::prelude::*;

#[component]
pub fn ChartDesk(mut studio: Studio) -> Element {
    let hidden_flags = use_signal(|| false);
    if super::dance::active() {
        let source = format!(
            "{}:{}",
            super::dance::view_epoch(),
            super::dance::selected_track().unwrap_or_default()
        );
        let track = super::track_views::active(studio);
        return rsx! {section{class:"chart-desk",aria_label:"Note editor",tabindex:"-1",onmounted:move |event|studio.editor_focus.set(Some(event.data())),EditingToolbar{studio,hidden_flags}super::track_views::Controls{studio,pane:super::track_views::Pane::Roll}super::track_views::RollHeading{studio,track:track.clone()}if !super::track_views::collapsed(studio,&track){for key in [source]{super::dance::Roll{key:"{key}",studio}}}super::song_overview::SongOverview{studio}}};
    }
    let Some(_) = studio.snapshot() else {
        return rsx! {};
    };
    let tracks = super::track_views::visible(studio, super::track_views::Pane::Roll);
    rsx! {
        section { class: "chart-desk", aria_label: "Note editor", tabindex: "-1", onmounted: move |event| studio.editor_focus.set(Some(event.data())),
            EditingToolbar { studio,hidden_flags }
            super::track_views::Controls{studio,pane:super::track_views::Pane::Roll}
            div { class: "chart-desk-scroll", onwheel: move |event| super::song_overview::pan(studio,event),
                div { class: "chart-desk-ruler", super::sections::SectionStrip { studio } super::timing_editor::TimingStrip { studio } Ruler { studio }
                    div { class: "midi-waveform-strip", div { class: "midi-waveform-label", "Recording" } Waveform { studio } }
                }
                for track in tracks {
                    section { key: "{track}", class: "chart-desk-track",
                        super::track_views::RollHeading{studio,track:track.clone()}
                        if !super::track_views::collapsed(studio,&track) {
                            if track == "Vocals" { VocalRoll { studio } } else { TrackRoll { studio, track, show_hidden: hidden_flags() } }
                        }
                    }
                }
            }
            super::song_overview::SongOverview {studio}
        }
    }
}

#[component]
fn EditingToolbar(mut studio: Studio, mut hidden_flags: Signal<bool>) -> Element {
    let history = super::session::history(studio);
    rsx! {            div { class: "chart-desk-toolbar", role: "toolbar", aria_label: "Chart editing", onkeydown: super::shortcuts::keep_space_local,
                    TrackPicker { studio }
                button { aria_label: "Add a part", title: "Add a blank part or generate one from the recording", onclick: move |_| studio.parts_open.set(true), Icon { name: "plus", size: 14 } "Add part" }
                    div { class: "edit-tools", role: "group", aria_label: "Editing tools",
                        for (tool, label, icon, key) in [(EditTool::Select,"Select","cursor","1"),(EditTool::Place,"Place","plus","2"),(EditTool::Erase,"Erase","eraser","3")] {
                            button { class: "edit-tool", aria_label: "{label} notes", aria_pressed: ((studio.tool)() == tool).to_string(), title: "{label} notes ({key})",
                                onkeydown: move |event| { if event.key() == Key::Character(" ".into()) { event.stop_propagation(); } },
                                onclick: move |_| studio.tool.set(tool), Icon { name: icon, size: 14 } span { class: "edit-tool-name", "{label}" }
                            }
                        }
                    }
                    div { class: "edit-history", role: "group", aria_label: "Edit history",
                        button { title: "Undo (Command/Ctrl+Z)", aria_label: "Undo", disabled: !history.0, onclick: move |_| super::session::undo(studio,false), Icon { name: "undo", size: 14 } }
                        button { title: "Redo (Command/Ctrl+Shift+Z)", aria_label: "Redo", disabled: !history.1, onclick: move |_| super::session::undo(studio,true), Icon { name: "redo", size: 14 } }
                        button { title: "Copy notes (Command/Ctrl+C)", aria_label: "Copy notes", disabled: !super::session::selected(studio), onclick: move |_| super::session::copy(studio,false), Icon { name: "copy", size: 14 } }
                        button { title: "Paste at playhead (Command/Ctrl+V)", aria_label: "Paste notes", onclick: move |_| super::session::paste(studio), Icon { name: "clipboard", size: 14 } }
                    }
                    label { class: "edit-snap", "Snap"
                        select { aria_label: "Snap", value: "{studio.snap}", onkeydown: move |event| event.stop_propagation(), onchange: move |event| { if let Ok(value) = event.value().parse() { studio.snap.set(value);super::dance::set_snap(value); } },
                        for (value, label) in [(0,"Off"),(1,"1/4"),(2,"1/8"),(3,"1/12"),(4,"1/16"),(6,"1/24"),(8,"1/32"),(12,"1/48"),(16,"1/64"),(24,"1/96"),(48,"1/192")] { option { value: "{value}", selected: value == (studio.snap)(), "{label}" } }
                        }
                    }
                    div { class: "edit-zoom", role: "group", aria_label: "Timeline zoom",
                        button { title: "Zoom out", aria_label: "Zoom out", onclick: move |_| studio.span.set(((studio.span)() * 1.5).min(48.0)), Icon { name: "minus", size: 14 } }
                        output { if super::dance::active(){"{(studio.span)():.1} beats"}else{"{(studio.span)():.1}s"} }
                        button { title: "Zoom in", aria_label: "Zoom in", onclick: move |_| studio.span.set(((studio.span)() / 1.5).max(0.5)), Icon { name: "plus", size: 14 } }
                    }
                    label { class: "toolbar-toggle", input { r#type: "checkbox", checked: (studio.follow)(), onkeydown: move |event| event.stop_propagation(), onchange: move |event| studio.follow.set(event.checked()) } "Follow" }
                    if !super::dance::active(){label { class: "toolbar-toggle", input { r#type: "checkbox", checked: hidden_flags(), onkeydown: move |event| event.stop_propagation(), onchange: move |event| hidden_flags.set(event.checked()) } "Hidden flags" }}
                }
    }
}
