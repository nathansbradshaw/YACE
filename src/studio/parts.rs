//! Both authoring paths are available for every supported part.
use super::{icons::Icon, state::Studio};
use crate::editor::Action;
use dioxus::prelude::*;

#[component]
pub fn FirstPart(mut value: Signal<String>, disabled: bool) -> Element {
    rsx! {label{class:"field-stack","First part",select{class:"field-control",aria_label:"First part",value:"{value}",disabled,onchange:move|e|value.set(e.value()),
        for (id,label) in [("band","Guitar"),("pad","Dance"),("keyboard","4-key")] {option{value:id,selected:value()==id,"{label}"}}
    }}}
}

#[component]
pub fn PartsPanel(mut studio: Studio) -> Element {
    let project = studio
        .snapshot()
        .filter(|_| super::dance::band_visible(studio));
    if project.is_none() && !super::dance::active() {
        return rsx! {};
    }
    let mut tier = use_signal(|| {
        ["Expert", "Hard", "Medium", "Easy"]
            .into_iter()
            .find(|tier| {
                project
                    .as_ref()
                    .is_some_and(|project| project.selected_track.starts_with(tier))
            })
            .unwrap_or("Expert")
            .to_owned()
    });
    rsx! {
        super::sheet::ToolSheet {
            id: "add-part-dialog", title: "Add a part", description: "Start blank or generate a draft from this recording.",
            close_label: "Close add part", class: "parts-sheet", onclose: move |_| studio.parts_open.set(false),
            actions: rsx! { span { class: "field-hint", "Other parts stay in place. Changes can be undone." } },
            if project.is_some(){label { class: "parts-difficulty", "Difficulty"
                select { aria_label: "New part difficulty", value: "{tier}", onchange: move |event| tier.set(event.value()),
                    for difficulty in ["Expert", "Hard", "Medium", "Easy"] { option { value: difficulty, selected: tier() == difficulty, "{difficulty}" } }
                }
            }}
            ul { class: "parts-list",
                for (label,profile) in [("Dance",crate::stepmania::Profile::Pad),("4-key",crate::stepmania::Profile::Keyboard)] {
                    li { div { strong { "{label}" } }
                        button { aria_label:"Add blank {label}",onclick:move |_|super::dance::from_recording(studio,profile,false),"Blank" }
                        button { aria_label:"Generate {label} from recording",onclick:move |_|super::dance::from_recording(studio,profile,true),"Generate…" }
                    }
                }
                if let Some(project)=project{for (label, suffix, _) in crate::parts::PARTS {
                    { let track = if suffix == "Vocals" { "Vocals".to_owned() } else { format!("{}{suffix}", tier()) };
                      let exists = if suffix == "Vocals" { project.content.vocals.is_some() || project.content.track_origins.contains_key("Vocals") } else { project.content.tracks.contains_key(&track) };
                      let blank_track = track.clone(); let generated_track = track.clone();
                      rsx! { li {
                          div { strong { "{label}" } if exists { span { "In this chart" } } }
                          button { disabled: exists, title: if exists { "This part already exists. Select it from Track to edit." } else { "Add an empty part and start placing notes" },
                              aria_label: "Add blank {label}", onclick: move |_| {
                                  super::dance::deactivate();
                                  studio.dispatch(Action::CreateTrack { track: blank_track.clone() });
                                  if studio.snapshot().is_some_and(|doc| doc.selected_track == blank_track) { studio.parts_open.set(false); }
                              }, Icon { name: "plus", size: 14 } "Blank"
                          }
                          button { title:"Generate this part from the recording", aria_label: "Generate {label} from recording", onclick: move |_| {
                              super::dance::deactivate();
                              studio.generation_target.set(Some(generated_track.clone()));
                              studio.parts_open.set(false); studio.generation_open.set(true);
                          }, Icon { name: "music", size: 14 } "Generate…" }
                      } }
                    }
                }}
            }
        }
    }
}
