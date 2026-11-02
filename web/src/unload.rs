//! Ask the browser before abandoning edits that have not reached IndexedDB.
use dioxus::prelude::*;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{JsCast, closure::Closure};

struct Listener {
    window: web_sys::Window,
    callback: Closure<dyn FnMut(web_sys::BeforeUnloadEvent)>,
}
impl Listener {
    fn install() -> Option<Self> {
        let window = web_sys::window()?;
        let callback = Closure::new(|event: web_sys::BeforeUnloadEvent| {
            event.prevent_default();
            // Older browsers also require a nonempty returnValue. Modern
            // browsers choose their own confirmation text.
            event.set_return_value("YACE has unsaved changes.");
        });
        if let Err(error) = window
            .add_event_listener_with_callback("beforeunload", callback.as_ref().unchecked_ref())
        {
            web_sys::console::error_2(&"Could not protect unsaved YACE changes".into(), &error);
            return None;
        }
        Some(Self { window, callback })
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.window.remove_event_listener_with_callback(
            "beforeunload",
            self.callback.as_ref().unchecked_ref(),
        );
    }
}

/// Subscribe to save state, attaching only while changes are pending or failed.
/// A page-unload handler cannot save asynchronously or protect against a crash.
pub fn use_unsaved_changes(mut pending: impl FnMut() -> bool + 'static) {
    let listener = use_hook(|| Rc::new(RefCell::new(None::<Listener>)));
    use_effect(move || {
        if pending() {
            if listener.borrow().is_none() {
                *listener.borrow_mut() = Listener::install();
            }
        } else {
            listener.borrow_mut().take();
        }
    });
}
