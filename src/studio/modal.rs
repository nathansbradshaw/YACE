//! Shared native dialogs keep focus and editor shortcuts apart.
use dioxus::prelude::*;

#[component]
pub fn Modal(
    id: &'static str,
    label: &'static str,
    onclose: EventHandler,
    children: Element,
) -> Element {
    rsx! {
        dialog {
            id,
            class: "studio-modal-portal",
            aria_label: label,
            aria_modal: "true",
            onmounted: move |_| async move {
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
