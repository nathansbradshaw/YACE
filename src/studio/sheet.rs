//! Consistent tool windows with native focus containment and persistent jobs.
use super::icons::Icon;
use dioxus::prelude::*;

#[component]
pub fn ToolSheet(
    id: &'static str,
    title: &'static str,
    description: &'static str,
    close_label: &'static str,
    class: &'static str,
    onclose: EventHandler,
    actions: Element,
    children: Element,
) -> Element {
    rsx! {
        super::modal::Modal { id, label: title, onclose,
            article { class: "tool-sheet {class}",
                header { class: "tool-sheet-heading",
                    div { h2 { "{title}" } p { "{description}" } }
                    button { class: "tool-sheet-close", aria_label: close_label, onclick: move |_| onclose.call(()), Icon { name: "close", size: 18 } }
                }
                div { class: "tool-sheet-content", {children} }
                footer { class: "tool-sheet-actions", {actions} }
            }
        }
    }
}
