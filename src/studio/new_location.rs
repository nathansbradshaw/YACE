//! Destination choice shared by the blank and recording setup dialogs.
use dioxus::prelude::*;

#[cfg(target_arch = "wasm32")]
pub use crate::project_destination::Choice;

// Desktop already asks for a project folder in its native creation save dialog.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Default)]
pub struct Choice;
#[cfg(not(target_arch = "wasm32"))]
impl Choice {
    pub fn ready(&self) -> bool {
        true
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[component]
pub fn Prompt(choice: Signal<Choice>, busy: bool) -> Element {
    let _ = (choice, busy);
    rsx! {p {class:"field-hint", "After Create chart, choose a project folder in the save dialog."}}
}

#[cfg(target_arch = "wasm32")]
#[component]
pub fn Prompt(mut choice: Signal<Choice>, busy: bool) -> Element {
    let mut picking = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let supported = crate::project_folder::supported();
    let selected = choice();
    rsx! {
        fieldset {class:"project-location", disabled:busy || picking(),
            legend {"Where should this project be saved?"}
            if supported {
                p {class:"field-hint", "Choose an empty project folder. Edits autosave there, and desktop YACE can open the same folder."}
                button {r#type:"button", aria_pressed:matches!(selected,Choice::Folder(_)), onclick:move |_|{
                    if busy || *picking.peek() {return;}
                    // Start the native picker in this click, before any await.
                    let picker = crate::project_folder::pick();
                    let previous = choice();
                    choice.set(Choice::Unchosen);
                    picking.set(true);
                    error.set(None);
                    spawn(async move {
                        match async {crate::project_folder::selected(picker?).await}.await {
                            Ok(Some(directory)) => choice.set(Choice::Folder(directory)),
                            Ok(None) => choice.set(previous),
                            Err(problem) => {choice.set(previous);error.set(Some(format!("{problem:#}")));}
                        }
                        picking.set(false);
                    });
                }, if picking(){"Choosing folder…"}else{"Choose project folder…"}}
            } else {
                p {class:"field-hint", "This browser cannot save directly to a project folder. Firefox does not support this feature. Use desktop YACE, Chrome or Edge for folder saving, or explicitly choose browser storage below."}
            }
            p {class:"field-hint", "Browser storage stays in this browser and profile. To move to desktop or another device, download a complete project ZIP from the YACE menu."}
            button {r#type:"button", aria_pressed:matches!(selected,Choice::Browser), onclick:move |_|{choice.set(Choice::Browser);error.set(None);}, "Use browser storage"}
            match selected {
                Choice::Unchosen => rsx! {p {role:"status", "Choose a save location to enable Create chart."}},
                Choice::Browser => rsx! {p {role:"status", "Browser storage selected. This project will not autosave to a desktop folder."}},
                Choice::Folder(ref directory) => rsx! {p {role:"status", "Project folder selected: {directory.name()}"}},
            }
            if let Some(message)=error(){p {role:"alert", "{message}"}}
        }
    }
}
