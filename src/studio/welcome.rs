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
                    button { class: "welcome-action", disabled: (studio.library_busy)(), "data-browser-pick": "folder", onclick: move |_| studio.create_dialog(),
                        Icon { name: "folder-open", size: 20 }
                        div { strong { if cfg!(feature="browser") { "Open song folder" } else { "Open charting folder" } } span { if cfg!(feature="browser") { "Edit an existing chart in a separate copy." } else { "Find charts up to 3 folders deep and switch between them." } } }
                    }
                    button { class: "welcome-action", "data-browser-pick": "file", "data-browser-accept":".zip", onclick: move |_| studio.open_dialog(),
                        Icon { name: "folder-open", size: 20 }
                        div { strong { "Open saved project" } span { if cfg!(feature="browser") {"Open a saved project ZIP."} else {"Continue editing a YACE project."} } }
                    }
                }
                if !cfg!(feature = "browser") {
                    if let Some(library) = (studio.library)() {
                        div { class: "welcome-library", aria_label: "Charts in selected folder",
                            p { "{library.entries.len()} charts in {library.root.display()}" }
                            div { class: "welcome-chart-list",
                                for (index, item) in library.entries.iter().enumerate() {
                                    button { class: "welcome-action", disabled: (studio.library_busy)(),
                                        onclick: move |_| {
                                            #[cfg(not(feature = "browser"))]
                                            spawn(async move { studio.open_library_chart(index).await; });
                                            #[cfg(feature = "browser")]
                                            let _ = index;
                                        },
                                        Icon { name: "music", size: 20 }
                                        div { strong { "{item.title}" } span { "{item.path.strip_prefix(&library.root).unwrap_or(&item.path).display()}" } }
                                    }
                                }
                            }
                        }
                    }
                }
                p { class: "welcome-hint", "Edits save automatically. Your original files stay untouched." }
            }
        }
    }
}
