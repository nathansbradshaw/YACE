// Result actions are available only after generation has produced a draft.
use dioxus::prelude::*;

#[component]
pub fn GenerationActions(
    busy: bool,
    saved: bool,
    complete: bool,
    source: bool,
    can_generate: bool,
    stopping: bool,
    #[props(default = true)] can_stop: bool,
    onclose: EventHandler,
    onstop: EventHandler,
    onopen: EventHandler,
    onresume: EventHandler,
    onstart: EventHandler,
    onchoose: EventHandler,
) -> Element {
    let close = if busy {
        "Continue in background"
    } else if saved {
        "Keep draft & close"
    } else {
        "Close"
    };
    let primary = if busy {
        if !can_stop {
            "Preparing…"
        } else if stopping {
            "Finishing this step…"
        } else {
            "Stop after this step"
        }
    } else if saved {
        if complete {
            "Open generated chart"
        } else {
            "Resume generation"
        }
    } else if source {
        "Generate selected parts"
    } else {
        "Choose recording…"
    };
    rsx! {
        button { class:"action-secondary", onclick:move |_|onclose.call(()), "{close}" }
        button {
            class:if busy{"action-secondary"}else{"action-primary"},
            disabled:if busy{stopping||!can_stop}else{!saved && source && !can_generate},
            onclick:move |_|{
                if busy{onstop.call(())}else if saved{if complete{onopen.call(())}else{onresume.call(())}}else if source{onstart.call(())}else{onchoose.call(())}
            },
            "{primary}"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::core::{AttributeValue, Mutation};
    use std::{any::Any, cell::Cell, rc::Rc};
    #[derive(Clone, Copy)]
    struct Case {
        busy: bool,
        saved: bool,
        complete: bool,
        source: bool,
        stopping: bool,
    }
    fn harness() -> Element {
        let c = use_context::<Case>();
        let action = use_context::<Rc<Cell<&'static str>>>();
        let close = action.clone();
        let stop = action.clone();
        let open = action.clone();
        let resume = action.clone();
        let start = action.clone();
        rsx! {GenerationActions{busy:c.busy,saved:c.saved,complete:c.complete,source:c.source,stopping:c.stopping,can_generate:true,onclose:move |_|close.set("close"),onstop:move |_|stop.set("stop"),onopen:move |_|open.set("open"),onresume:move |_|resume.set("resume"),onstart:move |_|start.set("start"),onchoose:move |_|action.set("choose")}}
    }
    #[test]
    fn actions_follow_available_work_and_never_offer_cancel_for_a_finished_draft() {
        dioxus::html::set_event_converter(Box::new(dioxus::html::SerializedHtmlEventConverter));
        for (case, close, primary) in [
            (
                Case {
                    busy: false,
                    saved: false,
                    complete: false,
                    source: false,
                    stopping: false,
                },
                "Close",
                "Choose recording…",
            ),
            (
                Case {
                    busy: false,
                    saved: false,
                    complete: false,
                    source: true,
                    stopping: false,
                },
                "Close",
                "Generate selected parts",
            ),
            (
                Case {
                    busy: true,
                    saved: true,
                    complete: false,
                    source: true,
                    stopping: false,
                },
                "Continue in background",
                "Stop after this step",
            ),
            (
                Case {
                    busy: true,
                    saved: true,
                    complete: false,
                    source: true,
                    stopping: true,
                },
                "Continue in background",
                "Finishing this step…",
            ),
            (
                Case {
                    busy: false,
                    saved: true,
                    complete: false,
                    source: true,
                    stopping: false,
                },
                "Keep draft & close",
                "Resume generation",
            ),
            (
                Case {
                    busy: false,
                    saved: true,
                    complete: true,
                    source: true,
                    stopping: false,
                },
                "Keep draft & close",
                "Open generated chart",
            ),
        ] {
            let mut dom = VirtualDom::new(harness);
            dom.provide_root_context(case);
            let action = Rc::new(Cell::new(""));
            dom.provide_root_context(action.clone());
            let edits = dom.rebuild_to_vec().edits;
            let labels = edits
                .iter()
                .filter_map(|edit| match edit {
                    Mutation::CreateTextNode { value, .. } => Some(value.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(labels, vec![close, primary]);
            let buttons = edits
                .iter()
                .filter_map(|edit| match edit {
                    Mutation::NewEventListener { name, id } if name == "click" => Some(*id),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let click = || {
                Event::new(
                    Rc::new(dioxus::html::PlatformEventData::new(Box::<
                        dioxus::html::SerializedMouseData,
                    >::default(
                    ))) as Rc<dyn Any>,
                    true,
                )
            };
            dom.runtime().handle_event("click", click(), buttons[0]);
            assert_eq!(action.get(), "close");
            if !case.stopping {
                dom.runtime().handle_event("click", click(), buttons[1]);
                assert_eq!(
                    action.get(),
                    if case.busy {
                        "stop"
                    } else if case.saved {
                        if case.complete { "open" } else { "resume" }
                    } else if case.source {
                        "start"
                    } else {
                        "choose"
                    }
                );
            }
            if case.stopping {
                assert!(edits.iter().any(|edit| matches!(
                    edit,
                    Mutation::SetAttribute {
                        name: "disabled",
                        value: AttributeValue::Bool(true),
                        ..
                    }
                )));
            }
        }
    }
}
