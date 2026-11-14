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
                    SnapControls {studio}
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

const SNAP_PRESETS: &[(u32, &str)] = &[
    (0, "Off"),
    (1, "1/4"),
    (2, "1/8"),
    (3, "1/12"),
    (4, "1/16"),
    (6, "1/24"),
    (8, "1/32"),
    (12, "1/48"),
    (16, "1/64"),
    (24, "1/96"),
    (48, "1/192"),
    (64, "1/256"),
    (96, "1/384"),
    (192, "1/768"),
    (256, "1/1024"),
];

#[component]
fn SnapControls(mut studio: Studio) -> Element {
    let mut open = use_signal(|| false);
    let mut custom = use_signal(|| "7".to_string());
    let mut error = use_signal(|| None::<String>);
    let value = (studio.snap)();
    rsx! {div{class:"snap-controls",
        label{class:"edit-snap","Snap"
            select{aria_label:"Snap",value:"{value}",onkeydown:move|e|e.stop_propagation(),onchange:move|e|{
                if let Ok(value)=e.value().parse::<u32>() && (value==0||crate::snap_grid::Grid::new(1,value).is_ok()){
                    studio.snap.set(value);super::dance::set_snap(value);
                }
            },
                for &(n,label) in SNAP_PRESETS {option{value:"{n}",selected:n==value,"{label}"}}
                if !SNAP_PRESETS.iter().any(|(n,_)|*n==value){option{value:"{value}",selected:true,"Custom · {value}/beat"}}
            }
        }
        button{class:"studio-tool",aria_label:"Custom snap grid",aria_expanded:open().to_string(),onclick:move|_|open.toggle(),"Custom…"}
        super::timing_repair::Controls {studio}
        super::text_events_editor::Controls {studio}
        if open(){super::modal::Modal{id:"snap-grid-dialog",label:"Custom snap settings",onclose:move|_|open.set(false),div{class:"snap-custom-panel",
            h2{"Custom snap grid"}
            label{"Steps per beat" input{aria_label:"Snap steps per beat",r#type:"number",min:"1",max:"1024",step:"1",value:"{custom}",oninput:move|e|{custom.set(e.value());error.set(None);},onkeydown:move|e|e.stop_propagation()}}
            p{class:"field-hint","3 gives eighth-note triplets; 7 divides each beat into seven steps. Fine grids round to the chart's ticks. Zoom in to reveal dense grid lines."}
            if super::dance::active(){p{class:"field-hint","Dance files have 48 rows per beat. Finer steps share the same row."}}
            if let Some(message)=error(){p{role:"alert","{message}"}}
            button{class:"action-primary",onclick:move|_|{
                match custom().parse::<u32>().ok().filter(|n|crate::snap_grid::Grid::new(1,*n).is_ok()){
                    Some(n)=>{studio.snap.set(n);super::dance::set_snap(n);studio.status.set(format!("Snap: {n} steps per beat"));error.set(None);open.set(false);},
                    None=>error.set(Some("Use a whole number from 1 to 1024 steps per beat".into())),
                }
            },"Apply snap grid"}
            button{onclick:move|_|open.set(false),"Close snap settings"}
        }}}
    }}
}
