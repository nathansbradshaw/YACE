//! Shared native dialogs keep focus and editor shortcuts apart.
use dioxus::prelude::*;

#[component]
pub fn Modal(
    id: &'static str,
    label: &'static str,
    onclose: EventHandler,
    children: Element,
) -> Element {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        let trigger = use_hook(|| {
            web_sys::window()?
                .document()?
                .active_element()?
                .dyn_into::<web_sys::HtmlElement>()
                .ok()
        });
        use_drop(move || {
            if let Some(trigger) = trigger
                && trigger.is_connected()
            {
                let _ = trigger.focus();
            }
        });
    }
    rsx! {
        dialog {
            id,
            class: "studio-modal-portal",
            aria_label: label,
            aria_modal: "true",
            onmounted: move |_| async move {
                #[cfg(target_arch="wasm32")]
                {use wasm_bindgen::JsCast;if let Some(element)=web_sys::window().and_then(|window|window.document()).and_then(|document|document.get_element_by_id(id))&& let Ok(dialog)=element.dyn_into::<web_sys::HtmlDialogElement>()&& let Err(error)=dialog.show_modal(){web_sys::console::error_1(&error);}}
                #[cfg(not(target_arch="wasm32"))]
                {
                // Only the WebView exposes showModal. Its native focus trap also
                // keeps Tab and screen readers out of the editor behind it.
                let id = serde_json::to_string(id).expect("static dialog ID");
                let script = format!(r#"
                    const dialog = document.getElementById({id});
                    const trigger = document.activeElement;
                    dialog.showModal();
                    const observer = new MutationObserver(() => {{
                        if (!dialog.isConnected) {{
                            observer.disconnect();
                            if (trigger?.isConnected) trigger.focus();
                        }}
                    }});
                    observer.observe(dialog.parentNode, {{ childList: true }});
                "#);
                if let Err(error) = dioxus_document::eval(&script).await {
                    eprintln!("Cannot open {id}: {error}");
                }
                }
            },
            oncancel: move |event| {
                event.prevent_default();
                onclose.call(());
            },
            onkeydown: move |event| event.stop_propagation(),
            {children}
        }
    }
}
