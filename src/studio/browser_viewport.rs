//! Keep the desktop workspace intact when a browser window is too small.
use dioxus::prelude::*;

#[cfg(feature = "browser")]
#[path = "viewport_policy.rs"]
mod policy;
#[cfg(feature = "browser")]
const DISMISSED: &str = "YACE-desktop-window-notice-dismissed";

#[component]
pub fn BrowserViewport(children: Element) -> Element {
    #[cfg(not(feature = "browser"))]
    {
        children
    }
    #[cfg(feature = "browser")]
    {
        let (small, mut dismissed) = use_viewport();
        rsx! {
            style { {include_str!("../../assets/browser-viewport.css")} }
            div {class: "browser-viewport",
                if small() && !dismissed() {
                    aside {class: "desktop-space-notice", aria_label: "Desktop editing space",
                        span {"YACE is designed for desktop use. Enlarge this window for more editing space."}
                        button {r#type:"button",aria_label:"Dismiss desktop window notice",onclick:move |_| {
                            dismissed.set(true);
                            remember_dismissal();
                        },"Dismiss"}
                    }
                }
                div {class: "browser-desktop-scroll", {children}}
            }
        }
    }
}

#[cfg(all(feature = "browser", target_arch = "wasm32"))]
fn use_viewport() -> (Signal<bool>, Signal<bool>) {
    use wasm_bindgen::{JsCast, closure::Closure};
    fn small() -> bool {
        web_sys::window().is_some_and(|window| {
            let width = window
                .inner_width()
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or_default();
            let height = window
                .inner_height()
                .ok()
                .and_then(|v| v.as_f64())
                .unwrap_or_default();
            policy::cramped(width, height)
        })
    }
    struct Listener {
        window: web_sys::Window,
        callback: Closure<dyn FnMut()>,
    }
    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = self.window.remove_event_listener_with_callback(
                "resize",
                self.callback.as_ref().unchecked_ref(),
            );
        }
    }
    let mut cramped = use_signal(small);
    let dismissed = use_signal(|| {
        web_sys::window()
            .and_then(|w| w.session_storage().ok().flatten())
            .and_then(|s| s.get_item(DISMISSED).ok().flatten())
            .is_some_and(|v| v == "1")
    });
    use_hook(move || {
        web_sys::window().and_then(|window| {
            let callback = Closure::<dyn FnMut()>::new(move || {
                let value = small();
                if *cramped.peek() != value {
                    cramped.set(value);
                }
            });
            window
                .add_event_listener_with_callback("resize", callback.as_ref().unchecked_ref())
                .ok()?;
            Some(std::rc::Rc::new(Listener { window, callback }))
        })
    });
    (cramped, dismissed)
}

#[cfg(all(feature = "browser", target_arch = "wasm32"))]
fn remember_dismissal() {
    if let Some(storage) = web_sys::window().and_then(|w| w.session_storage().ok().flatten()) {
        let _ = storage.set_item(DISMISSED, "1");
    }
}

#[cfg(all(feature = "browser", not(target_arch = "wasm32")))]
fn use_viewport() -> (Signal<bool>, Signal<bool>) {
    #[derive(serde::Deserialize)]
    struct Viewport {
        width: f64,
        height: f64,
        dismissed: bool,
    }
    let mut small = use_signal(|| false);
    let mut dismissed = use_signal(|| false);
    let mut bridge = use_signal(|| None::<dioxus_document::Eval>);
    use_future(move || async move {
        // Hosted Rust cannot access the browser's window or sessionStorage.
        // This bridge reports dimensions only; Rust owns thresholds and UI.
        let mut eval = dioxus_document::eval(&format!(
            r#"
            const key={key};
            let closed;
            const stop=new Promise(resolve=>{{closed=()=>resolve('stop');}});
            const report=()=>{{let dismissed=false;try{{dismissed=sessionStorage.getItem(key)==='1';}}catch(_){{}}
                dioxus.send({{width:innerWidth,height:innerHeight,dismissed}});}};
            const socket=window.ipc?.ws;
            window.addEventListener('resize',report);
            socket?.addEventListener('close',closed,{{once:true}});
            try{{report();for(;;){{const message=await Promise.race([dioxus.recv(),stop]);
                if(message==='stop')break;
            }}}}finally{{window.removeEventListener('resize',report);socket?.removeEventListener('close',closed);}}
        "#,
            key = serde_json::to_string(DISMISSED).unwrap()
        ));
        bridge.set(Some(eval));
        while let Ok(view) = eval.recv::<Viewport>().await {
            let value = policy::cramped(view.width, view.height);
            if *small.peek() != value {
                small.set(value);
            }
            if view.dismissed && !*dismissed.peek() {
                dismissed.set(true);
            }
        }
    });
    use_drop(move || {
        if let Some(eval) = *bridge.peek() {
            let _ = eval.send("stop");
        }
    });
    (small, dismissed)
}

#[cfg(all(feature = "browser", not(target_arch = "wasm32")))]
fn remember_dismissal() {
    // Only the hosted client needs a JavaScript storage binding.
    spawn(async {
        let _ = dioxus_document::eval(&format!(
            "try{{sessionStorage.setItem({},'1');}}catch(_){{}}",
            serde_json::to_string(DISMISSED).unwrap()
        ))
        .await;
    });
}
