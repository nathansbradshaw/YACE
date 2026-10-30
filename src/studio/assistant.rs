use super::state::{Studio, clock};
use crate::{
    assistant::{self, Cancellation, Connection, Provider, Review},
    editor::Action,
};
use dioxus::prelude::*;

#[derive(Clone)]
struct Turn {
    question: String,
    review: Review,
    applied: bool,
}

#[component]
pub fn Assistant(mut studio: Studio) -> Element {
    let mut provider = use_signal(|| Provider::Codex);
    let mut model = use_signal(String::new);
    let mut key = use_signal(String::new);
    let mut settings = use_signal(|| false);
    let mut question = use_signal(String::new);
    let mut turns = use_signal(Vec::<Turn>::new);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut models = use_signal(Vec::<String>::new);
    let mut cancellation = use_signal(Cancellation::default);
    let mut epoch = use_signal(|| 0u64);
    let mut project = use_signal(|| None::<std::path::PathBuf>);
    let fingerprint = use_memo(move || {
        if !(studio.assistant_open)() || turns.read().is_empty() {
            return None;
        }
        studio
            .doc
            .read()
            .as_ref()
            .and_then(|doc| assistant::content_fingerprint(doc).ok())
    });
    use_effect(move || {
        let current = studio.store.read().as_ref().map(|s| s.root.clone());
        if *project.peek() != current {
            cancellation.peek().cancel();
            epoch.set(epoch() + 1);
            busy.set(false);
            turns.set(Vec::new());
            error.set(None);
            project.set(current);
        }
    });
    use_drop(move || cancellation.peek().cancel());
    let mut send = move |prompt: String| {
        if busy() || prompt.trim().is_empty() {
            return;
        }
        let Some(store) = studio.store.read().clone() else {
            return;
        };
        let connection = Connection {
            provider: provider(),
            model: model(),
            api_key: key(),
        };
        let history = turns
            .read()
            .iter()
            .map(|t| (t.question.clone(), t.review.reply.answer.clone()))
            .collect::<Vec<_>>();
        let cancel = Cancellation::default();
        cancellation.set(cancel.clone());
        epoch.set(epoch() + 1);
        let id = epoch();
        busy.set(true);
        error.set(None);
        spawn(async move {
            let asked = prompt.clone();
            let result = tokio::task::spawn_blocking(move || {
                assistant::ask(&store, &connection, &asked, &history, &cancel)
            })
            .await;
            if epoch() != id {
                return;
            }
            busy.set(false);
            match result {
                Ok(Ok(review)) => {
                    let mut list = turns.write();
                    if list.len() >= 8 {
                        list.remove(0);
                    }
                    list.push(Turn {
                        question: prompt,
                        review,
                        applied: false,
                    });
                    question.set(String::new());
                    settings.set(false);
                }
                Ok(Err(e)) => error.set(Some(format!("{e:#}"))),
                Err(_) => error.set(Some("Assistant stopped unexpectedly. Try again.".into())),
            }
        });
    };
    let context = studio.doc.read().as_ref().map(|doc| {
        (
            studio
                .metadata
                .read()
                .get("name")
                .filter(|name| !name.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| doc.title.clone()),
            doc.selected_track.clone(),
        )
    });
    let loaded = context.is_some();
    rsx! {
        if (studio.assistant_open)() {
            aside { class:"ai-assistant", aria_label:"AI charting assistant", onkeydown:move |event|event.stop_propagation(),
                header {class:"ai-assistant-header", strong{"Charting assistant"}
                    button {class:"field-button",onclick:move |_|settings.set(!settings()),"Connection"}
                    button {class:"field-button",aria_label:"Close assistant",onclick:move |_|studio.assistant_open.set(false),"Close"}
                }
                div {class:"ai-assistant-body",
                p {class:"ai-assistant-caption","Ask about this chart. You choose which edits to apply."}
                if !loaded {
                    p {"Open a project in the editor to use AI help."}
                }
                if let Some((title,track))=context {p {class:"ai-assistant-context","Using loaded project: {title}"} p {class:"ai-assistant-caption","{track} · near {clock((studio.playhead)())}"}}
                if settings() {
                    div {class:"ai-assistant-settings",
                        label {r#for:"ai-provider","Connect with"}
                        select {id:"ai-provider",class:"field-control",disabled:busy(),value:"{provider().label()}",onchange:move |e|{
                            if let Some((p,_))=Provider::ALL.iter().find(|(_,label)|*label==e.value()) {provider.set(*p);model.set(String::new());key.set(String::new());models.set(Vec::new());}
                        }, for (_,label) in Provider::ALL {option {value:label,"{label}"}} }
                        if provider().needs_key() {
                            label {r#for:"ai-key","API key (this session only)"}
                            input {id:"ai-key",class:"field-control",r#type:"password",autocomplete:"off",disabled:busy(),value:"{key}",oninput:move |e|key.set(e.value())}
                        }
                        label {r#for:"ai-model",if matches!(provider(),Provider::Codex|Provider::Claude) {"Model (optional; CLI default)"} else {"Model name"}}
                        input {id:"ai-model",class:"field-control",disabled:busy(),value:"{model}",placeholder:"Enter a model available to your account",oninput:move |e|model.set(e.value())}
                        if provider()==Provider::Ollama {
                            button {class:"field-button",disabled:busy(),onclick:move |_|{spawn(async move {
                                match tokio::task::spawn_blocking(assistant::local_models).await {
                                    Ok(Ok(list))=>{if list.is_empty(){error.set(Some("No models installed. Install an Ollama model and refresh.".into()));} models.set(list);},
                                    Ok(Err(e))=>error.set(Some(format!("{e:#}"))),Err(_)=>error.set(Some("Could not list models".into()))
                                }
                            });},"Refresh installed models"}
                            for name in models() {button {class:"field-button",disabled:busy(),onclick:move |_|model.set(name.clone()),"{name}"}}
                            p {class:"ai-assistant-caption","Start Ollama and choose an installed model. Ollama controls whether that model uses a cloud service."}
                        } else if matches!(provider(),Provider::Codex|Provider::Claude) {
                            p {class:"ai-assistant-caption",if provider()==Provider::Codex {"Install Codex, then run codex login in a terminal to sign in with ChatGPT. YACE uses that CLI session."} else {"Install Claude Code, then run claude in a terminal and sign in. YACE uses that CLI session."}}
                        }
                        p {class:"ai-assistant-caption","Sends your question and chart context to this provider. Audio stays local; keys aren’t saved."}
                    }
                }
                div {class:"ai-assistant-conversation",aria_live:"polite",
                    if turns.read().is_empty() {p {"Try “Explain this pattern” or “Simplify this part”."}}
                    for (turn_index,turn) in turns().into_iter().enumerate() {
                        {let stale=fingerprint.read().as_deref()!=Some(&turn.review.content_sha256) || studio.store.read().as_ref().is_none_or(|s|s.root!=turn.review.packet.project);
                        let track_caption=turn.review.packet.options.track.as_deref().unwrap_or("All tracks");
                        rsx! {article {class:"ai-assistant-turn",
                            p {class:"ai-assistant-question","{turn.question}"}
                            p {class:"ai-assistant-answer","{turn.review.reply.answer}"}
                            p {class:"ai-assistant-caption","{turn.review.provider.label()} · {track_caption} · revision {turn.review.packet.revision} · {turn.review.packet.sections.len()} sections supplied · {turn.review.packet.remaining} more flags"}
                            for section in &turn.review.packet.sections {
                                {let start=section.start; let end=section.end;
                                rsx! {button {class:"field-button",disabled:busy()||stale||!(studio.audio_ready)(),onclick:move |_|{studio.seek(start);studio.dispatch(Action::SetLoop{range:Some(crate::editor::TimeRange{start,end})});studio.dispatch(Action::Playback{playing:true});},"Listen / loop {clock(start)}–{clock(end)}"}}}
                            }
                            for (index,suggestion) in turn.review.reply.suggestions.iter().enumerate() {
                                div {class:"ai-assistant-suggestion",
                                    p {"{suggestion.reason}"}
                                    p {class:"ai-assistant-caption", "{describe(&turn.review,index)}"}
                                    button {class:"field-button",disabled:busy()||stale||turn.applied,onclick:move |_|{
                                        let Some(store)=studio.store.read().clone() else{return;};
                                        let Some(turn)=turns.read().get(turn_index).cloned() else{return;};
                                        match assistant::apply(&store,&turn.review,index) {
                                            Ok(_)=>{turns.write()[turn_index].applied=true;studio.reload(false);error.set(None);studio.status.set("Suggestion applied. Undo is beside the editing tools.".into());},
                                            Err(e)=>error.set(Some(format!("{e:#}"))),
                                        }
                                    },"Apply this edit"}
                                }
                            }
                            if turn.applied {p {class:"ai-assistant-caption","Applied. Undo is beside the editing tools."}}
                            else if stale && !turn.review.reply.suggestions.is_empty() {p {class:"ai-assistant-caption","Chart changed. Ask again before applying these suggestions."}}
                        }}}
                    }
                }
                }
                if let Some(message)=error() {p {role:"alert",class:"ai-assistant-error","{message}"}}
                if busy() {div {role:"status","Thinking… You can keep charting." button {class:"field-button",onclick:move |_|{cancellation.peek().cancel();epoch.set(epoch()+1);busy.set(false);},"Cancel"}}}
                div {class:"ai-assistant-compose",
                    button {class:"field-button",disabled:busy()||!loaded,onclick:move |_|send("Review the next flagged sections of this track. Explain the evidence, what I should listen for, and propose only clear, justified note fixes.".into()),"Review next flags"}
                    textarea {aria_label:"Ask the charting assistant",class:"field-control",rows:"3",maxlength:8000,disabled:busy(),value:"{question}",placeholder:"Ask about timing, patterns, difficulty…",oninput:move |e|question.set(e.value())}
                    button {class:"field-button",disabled:busy()||!loaded||question().trim().is_empty(),onclick:move |_|send(question()),"Ask"}
                }
            }
        }
    }
}
fn describe(review: &Review, index: usize) -> String {
    let action = &review.reply.suggestions[index].action;
    let id = match action {
        Action::UpdateNote { note_id, .. } | Action::DeleteNote { note_id } => note_id,
        _ => return "Unsupported action".into(),
    };
    let old = review
        .packet
        .sections
        .iter()
        .flat_map(|s| &s.notes)
        .find(|n| &n.id == id);
    let Some(old) = old else {
        return "Note unavailable".into();
    };
    let before = format!(
        "tick {}, lane {}, sustain {}, cymbal {}",
        old.tick,
        old.lane.unwrap_or(0),
        old.end_tick.saturating_sub(old.tick),
        old.cymbal.unwrap_or(false)
    );
    let after = match action {
        Action::UpdateNote {
            tick,
            lane,
            sustain,
            cymbal,
            ..
        } => format!("tick {tick}, lane {lane}, sustain {sustain}, cymbal {cymbal}"),
        _ => "delete note".into(),
    };
    format!(
        "{} · {before} → {after}. {} linked lower-difficulty notes may change.",
        clock(old.start),
        old.linked_children.len() + old.omitted_children
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::core::Mutation;
    use std::{any::Any, cell::RefCell, rc::Rc};
    mod fixture {
        use crate as chart_engine;
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/assistant_project.rs"
        ));
    }
    fn loaded_editor() -> Element {
        let mut studio = super::super::state::use_studio();
        let (store, slot) = use_context::<(crate::editor::Store, Rc<RefCell<Option<Studio>>>)>();
        use_hook(move || {
            studio
                .doc
                .set(Some(std::sync::Arc::new(store.load().unwrap())));
            studio.store.set(Some(store));
            *slot.borrow_mut() = Some(studio);
        });
        rsx! { super::super::project_bar::ProjectBar {studio} Assistant {studio} }
    }
    #[test]
    fn opening_ai_after_the_editor_has_loaded_a_project_keeps_that_project() {
        dioxus::html::set_event_converter(Box::new(dioxus::html::SerializedHtmlEventConverter));
        let root = tempfile::tempdir().unwrap();
        let store = fixture::fixture(root.path());
        let slot = Rc::new(RefCell::new(None::<Studio>));
        let mut dom = VirtualDom::new(loaded_editor);
        dom.provide_root_context((store.clone(), slot.clone()));
        let initial = dom.rebuild_to_vec();
        dom.render_immediate_to_vec();
        let mut studio = slot.borrow().unwrap();
        let button = initial
            .edits
            .iter()
            .find_map(|edit| match edit {
                Mutation::SetAttribute {
                    name: "title",
                    value: dioxus::core::AttributeValue::Text(value),
                    id,
                    ..
                } if value == "Assistant for Visibility fixture" => Some(*id),
                _ => None,
            })
            .expect("Loaded editor exposes its chart assistant");
        let click = || {
            Event::new(
                Rc::new(dioxus::html::PlatformEventData::new(Box::<
                    dioxus::html::SerializedMouseData,
                >::default(
                ))) as Rc<dyn Any>,
                true,
            )
        };
        dom.runtime().handle_event("click", click(), button);
        let opened = dom.render_immediate_to_vec();
        assert!(opened.edits.iter().any(|edit| matches!(edit, Mutation::CreateTextNode {value,..} if value=="Using loaded project: Visibility fixture")), "{opened:?}");
        let close = opened
            .edits
            .iter()
            .filter_map(|edit| match edit {
                Mutation::NewEventListener { name, id } if name == "click" => Some(*id),
                _ => None,
            })
            .nth(1)
            .unwrap();
        dom.runtime().handle_event("click", click(), close);
        dom.render_immediate_to_vec();
        assert!(!*studio.assistant_open.peek());
        assert_eq!(studio.store.peek().as_ref().unwrap().root, store.root);
        assert_eq!(
            studio.doc.peek().as_ref().unwrap().title,
            "Visibility fixture"
        );
        assert_eq!(store.load().unwrap().revision, 0);

        // A normal project/track switch updates the same panel; it never owns
        // a separate project selector or keeps the former chart as its target.
        let second_root = tempfile::tempdir().unwrap();
        let second = fixture::fixture(second_root.path());
        second
            .apply(crate::editor::Request {
                expected_revision: 0,
                actor: crate::editor::Actor {
                    kind: crate::editor::ActorKind::Human,
                    name: "UI test".into(),
                },
                action: Action::SelectTrack {
                    track: "ExpertDrums".into(),
                },
            })
            .unwrap();
        studio.store.set(Some(second.clone()));
        studio
            .doc
            .set(Some(std::sync::Arc::new(second.load().unwrap())));
        studio.metadata.set(std::collections::BTreeMap::from([(
            "name".into(),
            "Second loaded song".into(),
        )]));
        dom.render_immediate_to_vec();
        dom.runtime().handle_event("click", click(), button);
        let switched = dom.render_immediate_to_vec();
        assert!(switched.edits.iter().any(|edit| matches!(edit, Mutation::CreateTextNode {value,..} if value=="Using loaded project: Second loaded song")), "{switched:?}");
        assert!(switched.edits.iter().any(|edit| matches!(edit, Mutation::CreateTextNode {value,..} if value.starts_with("ExpertDrums · near"))));
        assert_eq!(studio.store.peek().as_ref().unwrap().root, second.root);
        assert_eq!(store.load().unwrap().revision, 0);
    }
}
