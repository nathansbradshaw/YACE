use super::{icons::Icon, state::Studio};
use dioxus::prelude::*;

#[component]
pub fn Welcome(mut studio: Studio) -> Element {
    rsx! {
        main { class: "studio-welcome", aria_label: "Start a chart project",
            div { class: "welcome-content",
                h1 { "Start a chart project" }
                p { class: "welcome-intro", "Start with audio, a chart, or a saved project." }
                div { class: "welcome-actions", onkeydown: move |event| event.stop_propagation(),
                    button { class: "welcome-action", onclick: move |_| studio.generation_open.set(true),
                        Icon { name: "music", size: 20 }
                        div { strong { "Import audio" } span { "Generate a chart draft from a recording." } }
                    }
                    button { class: "welcome-action", "data-browser-pick": "folder", onclick: move |_| studio.create_dialog(),
                        Icon { name: "folder-open", size: 20 }
                        div { strong { "Open song folder" } span { "Edit an existing chart in a separate copy." } }
                    }
                    button { class: "welcome-action", "data-browser-pick": "file", "data-browser-accept":".zip", onclick: move |_| studio.open_dialog(),
                        Icon { name: "folder-open", size: 20 }
                        div { strong { "Open saved project" } span { if cfg!(feature="browser") {"Open a saved project ZIP."} else {"Continue editing a YACE project."} } }
                    }
                }
                p { class: "welcome-hint", "Edits save automatically. Your original files stay untouched." }
            }
        }
    }
}
