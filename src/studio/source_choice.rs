//! Explicit source selection shared by folder and ZIP imports on both clients.
use super::{modal::Modal, state::Studio};
use crate::{
    song_files::source_choice::{self, Source},
    storage as fs,
};
use anyhow::{Result, ensure};
use dioxus::prelude::*;
use std::{cell::Cell, path::Path, rc::Rc};

#[derive(Clone, Copy)]
enum Answer {
    Use(Source),
    Cancel,
}
#[derive(Clone)]
pub struct Prompt {
    answer: Rc<Cell<Option<Answer>>>,
}
impl Prompt {
    fn respond(&self, answer: Answer) {
        if self.answer.get().is_none() {
            self.answer.set(Some(answer));
        }
    }
}
impl Studio {
    pub async fn choose_chart_source(mut self, song: &Path) -> Result<Option<Source>> {
        if !source_choice::ambiguous(song) {
            source_choice::selected_file(song, None)?;
            return Ok(Some(if fs::is_file(song.join("notes.chart")) {
                Source::Chart
            } else {
                Source::Midi
            }));
        }
        ensure!(
            self.source_choice.peek().is_none(),
            "Finish the current source choice first"
        );
        let answer = Rc::new(Cell::new(None));
        self.source_choice.set(Some(Prompt {
            answer: answer.clone(),
        }));
        let response = loop {
            if let Some(response) = answer.get() {
                break response;
            }
            if self
                .source_choice
                .peek()
                .as_ref()
                .is_none_or(|p| !Rc::ptr_eq(&p.answer, &answer))
            {
                return Ok(None);
            }
            #[cfg(target_arch = "wasm32")]
            gloo_timers::future::TimeoutFuture::new(20).await;
            #[cfg(not(target_arch = "wasm32"))]
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        self.source_choice.set(None);
        match response {
            Answer::Use(source) => Ok(Some(source)),
            Answer::Cancel => {
                self.status.set("Source selection canceled".into());
                Ok(None)
            }
        }
    }
}
#[component]
pub fn SourceChoice(studio: Studio) -> Element {
    let Some(prompt) = (studio.source_choice)() else {
        return rsx! {};
    };
    let cancel = prompt.clone();
    let chart = prompt.clone();
    let midi = prompt.clone();
    let cancel_title = "Cancel source selection".to_owned();
    rsx! {
        Modal { id:"chart-source-choice", label:"Choose chart source", onclose:move |_|cancel.respond(Answer::Cancel),
            section { class:"sections-dialog", h2 { "Choose the chart to edit" }
                p { "This song contains both notes.chart and notes.mid." }
                p { "Choose one source for this editing project. Both originals will be kept, and the other source will stay out of the playable export." }
                div { class:"welcome-actions",
                    button { class:"welcome-action", title:format!("Use {} as the editable source", Source::Chart.filename()), onclick:move |_|chart.respond(Answer::Use(Source::Chart)), "Edit notes.chart" }
                    button { class:"welcome-action", title:format!("Use {} as the editable source", Source::Midi.filename()), onclick:move |_|midi.respond(Answer::Use(Source::Midi)), "Edit notes.mid" }
                }
                button { title:cancel_title, onclick:move |_|prompt.respond(Answer::Cancel), "Cancel" }
            }
        }
    }
}

#[cfg(all(test, not(feature = "browser")))]
mod tests {
    use super::*;
    use dioxus::core::{AttributeValue, Mutation};
    use std::{any::Any, cell::RefCell, path::PathBuf};
    type Outcome = Rc<Cell<Option<Option<Source>>>>;
    type Slot = Rc<RefCell<Option<Studio>>>;
    fn app() -> Element {
        let studio = super::super::state::use_studio();
        let (path, outcome, slot) = use_context::<(PathBuf, Outcome, Slot)>();
        use_hook(move || {
            *slot.borrow_mut() = Some(studio);
            dioxus::core::spawn_forever(async move {
                outcome.set(Some(studio.choose_chart_source(&path).await.unwrap()));
            });
        });
        rsx! {SourceChoice {studio}}
    }
    #[tokio::test]
    async fn chart_midi_and_cancel_buttons_resolve_only_after_an_explicit_click() {
        dioxus::html::set_event_converter(Box::new(dioxus::html::SerializedHtmlEventConverter));
        for (title, expected) in [
            (
                "Use notes.chart as the editable source",
                Some(Source::Chart),
            ),
            ("Use notes.mid as the editable source", Some(Source::Midi)),
            ("Cancel source selection", None),
        ] {
            let root = tempfile::tempdir().unwrap();
            std::fs::write(root.path().join("notes.chart"), b"untouched chart").unwrap();
            std::fs::write(root.path().join("notes.mid"), b"untouched MIDI").unwrap();
            let outcome = Outcome::default();
            let slot = Slot::default();
            let mut dom = VirtualDom::new(app);
            dom.provide_root_context((root.path().to_owned(), outcome.clone(), slot.clone()));
            dom.rebuild_to_vec();
            let button = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let changes = dom.render_immediate_to_vec();
                    if let Some(button) = changes.edits.iter().find_map(|edit| match edit {
                        Mutation::SetAttribute {name:"title",value:AttributeValue::Text(value),id,..} if value == title => Some(*id),
                        _ => None,
                    }) { break button; }
                    tokio::select! { _ = dom.wait_for_work() => {}, _ = tokio::time::sleep(std::time::Duration::from_millis(5)) => {} }
                }
            }).await.unwrap();
            assert_eq!(outcome.get(), None, "No source may be selected by default");
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
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while outcome.get().is_none() {
                    dom.render_immediate_to_vec();
                    tokio::select! { _ = dom.wait_for_work() => {}, _ = tokio::time::sleep(std::time::Duration::from_millis(5)) => {} }
                }
            }).await.unwrap();
            assert_eq!(outcome.get(), Some(expected));
            assert!(slot.borrow().unwrap().source_choice.peek().is_none());
            assert_eq!(
                std::fs::read(root.path().join("notes.chart")).unwrap(),
                b"untouched chart"
            );
            assert_eq!(
                std::fs::read(root.path().join("notes.mid")).unwrap(),
                b"untouched MIDI"
            );
        }
    }
}
