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
                    button {class:"welcome-action",onclick:move |_|studio.new_chart(),Icon{name:"music",size:20}div{strong{"New chart from audio"}span{"Start a blank chart from a recording."}}}
                    button { class: "welcome-action", title:"Generate a draft from local audio",onclick: move |_| studio.generation_open.set(true),
                        Icon { name: "music", size: 20 }
                        div { strong { "Import audio" } span { "Generate a chart draft from a recording." } }
                    }
                    button { class: "welcome-action", disabled: (studio.library_busy)(), "data-browser-pick": "folder", onclick: move |_| studio.create_dialog(),
                        Icon { name: "folder-open", size: 20 }
                        div { strong { "Open charting folder" } span { "Find charts up to 3 folders deep and switch between them." } }
                    }
                    button {class:"welcome-action",onclick:move |_|studio.import_song(),Icon{name:"folder-open",size:20}div{strong{"Import song ZIP"}span{"Open an existing chart and its recording."}}}
                    button { class: "welcome-action", "data-browser-pick": "file", "data-browser-accept":".zip", onclick: move |_| studio.open_dialog(),
                        Icon { name: "folder-open", size: 20 }
                        div { strong { "Open saved project" } span { if cfg!(feature="browser") {"Open a saved project ZIP."} else {"Continue editing a YACE project."} } }
                    }
                }
                if !cfg!(feature = "browser") || cfg!(target_arch = "wasm32") {
                    if let Some(library) = (studio.library)() {
                        div { class: "welcome-library", aria_label: "Charts in selected folder",
                            p { "{library.entries.len()} charts in {library.root.display()}" }
                            div { class: "welcome-chart-list",
                                for (index, item) in library.entries.iter().enumerate() {
                                    button { class: "welcome-action", title: "Open chart {item.title}", disabled: (studio.library_busy)(),
                                        onclick: move |_| {
                                            #[cfg(any(not(feature = "browser"), target_arch = "wasm32"))]
                                            dioxus::core::spawn_forever(async move { studio.open_library_chart(index).await; });
                                            #[cfg(all(feature = "browser", not(target_arch = "wasm32")))]
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

#[cfg(all(test, not(feature = "browser")))]
mod tests {
    use super::*;
    use crate::{
        chart_library::{Entry, Kind, Library},
        editor::Store,
    };
    use dioxus::core::{AttributeValue, Mutation};
    use std::{any::Any, cell::RefCell, rc::Rc};
    mod fixture {
        use crate as chart_engine;
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/assistant_project.rs"
        ));
    }
    type Slot = Rc<RefCell<Option<Studio>>>;
    fn app() -> Element {
        let mut studio = super::super::state::use_studio();
        let (store, slot) = use_context::<(Store, Slot)>();
        use_hook(move || {
            studio.library.set(Some(Library {
                root: store.root.clone(),
                entries: vec![Entry {
                    path: store.root,
                    title: "Lifecycle fixture".into(),
                    kind: Kind::Project,
                }],
            }));
            *slot.borrow_mut() = Some(studio);
        });
        if studio.snapshot().is_some() {
            rsx! { div { "Loaded chart" } }
        } else {
            rsx! { Welcome { studio } }
        }
    }

    #[tokio::test]
    async fn chart_open_finishes_after_the_welcome_component_unmounts() {
        dioxus::html::set_event_converter(Box::new(dioxus::html::SerializedHtmlEventConverter));
        let root = tempfile::tempdir().unwrap();
        let store = fixture::fixture(&root.path().canonicalize().unwrap());
        let slot = Rc::new(RefCell::new(None::<Studio>));
        let mut dom = VirtualDom::new(app);
        dom.provide_root_context((store, slot.clone()));
        let initial = dom.rebuild_to_vec();
        let button = initial
            .edits
            .iter()
            .find_map(|edit| match edit {
                Mutation::SetAttribute {
                    name: "title",
                    value: AttributeValue::Text(value),
                    id,
                    ..
                } if value == "Open chart Lifecycle fixture" => Some(*id),
                _ => None,
            })
            .unwrap();
        dom.runtime().handle_event(
            "click",
            Event::new(
                Rc::new(dioxus::html::PlatformEventData::new(Box::<
                    dioxus::html::SerializedMouseData,
                >::default(
                ))) as Rc<dyn Any>,
                true,
            ),
            button,
        );
        let studio = slot.borrow().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                dom.render_immediate_to_vec();
                if studio.doc.peek().is_some() && !*studio.library_busy.peek() {
                    break;
                }
                tokio::select! {
                    _ = dom.wait_for_work() => {},
                    _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {},
                }
            }
        })
        .await
        .expect("Opening must finish after the welcome screen disappears");
        assert_eq!(*studio.library_selected.peek(), Some(0));
        // The opaque fixture deliberately fails decoding. Its error must arrive
        // after unmount rather than leaving a permanently busy library.
        assert!(studio.error.peek().is_some());
    }
}
