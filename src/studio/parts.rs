//! Both authoring paths are available for every supported part.
use super::{icons::Icon, state::Studio};
use crate::editor::Action;
use dioxus::prelude::*;

#[component]
pub fn PartsPanel(mut studio: Studio) -> Element {
    let Some(project) = studio.snapshot() else {
        return rsx! {};
    };
    let mut tier = use_signal(|| {
        ["Expert", "Hard", "Medium", "Easy"]
            .into_iter()
            .find(|tier| project.selected_track.starts_with(tier))
            .unwrap_or("Expert")
            .to_owned()
    });
    rsx! {
        super::sheet::ToolSheet {
            id: "add-part-dialog", title: "Add a part", description: "Start blank or generate a draft from this recording.",
            close_label: "Close add part", class: "parts-sheet", onclose: move |_| studio.parts_open.set(false),
            actions: rsx! { span { class: "field-hint", "Other parts stay in place. Changes can be undone." } },
            label { class: "parts-difficulty", "Difficulty"
                select { aria_label: "New part difficulty", value: "{tier}", onchange: move |event| tier.set(event.value()),
                    for difficulty in ["Expert", "Hard", "Medium", "Easy"] { option { value: difficulty, selected: tier() == difficulty, "{difficulty}" } }
                }
            }
            ul { class: "parts-list",
                for (label, suffix, _) in crate::parts::PARTS {
                    { let track = if suffix == "Vocals" { "Vocals".to_owned() } else { format!("{}{suffix}", tier()) };
                      let exists = if suffix == "Vocals" { project.content.vocals.is_some() || project.content.track_origins.contains_key("Vocals") } else { project.content.tracks.contains_key(&track) };
                      let blank_track = track.clone(); let generated_track = track.clone();
                      rsx! { li {
                          div { strong { "{label}" } if exists { span { "In this chart" } } }
                          button { disabled: exists, title: if exists { "This part already exists. Select it from Track to edit." } else { "Add an empty part and start placing notes" },
                              aria_label: "Add blank {label}", onclick: move |_| {
                                  studio.dispatch(Action::CreateTrack { track: blank_track.clone() });
                                  if studio.snapshot().is_some_and(|doc| doc.selected_track == blank_track) { studio.parts_open.set(false); }
                              }, Icon { name: "plus", size: 14 } "Blank"
                          }
                          button { aria_label: "Generate {label} from recording", onclick: move |_| {
                              studio.generation_target.set(Some(generated_track.clone()));
                              studio.parts_open.set(false); studio.generation_open.set(true);
                          }, Icon { name: "music", size: 14 } "Generate…" }
                      } }
                    }
                }
            }
        }
    }
}
