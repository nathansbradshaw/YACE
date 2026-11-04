//! Restore a refused form edit from the accepted model, retaining keyboard focus.
use dioxus::prelude::*;

#[derive(Clone, Copy)]
pub(super) struct FieldReset {
    epoch: Signal<u64>,
    focus: Signal<bool>,
}

pub(super) fn use_field_reset() -> FieldReset {
    FieldReset {
        epoch: use_signal(|| 0),
        focus: use_signal(|| false),
    }
}

impl FieldReset {
    // Use this key on a dynamic list root: static template keys do not replace
    // a live field whose accepted model value stayed unchanged after refusal.
    pub fn key(self) -> u64 {
        (self.epoch)()
    }

    pub fn rejected(mut self) {
        self.focus.set(true);
        self.epoch.set((self.epoch)().wrapping_add(1));
    }

    pub async fn mounted(mut self, event: MountedEvent) {
        let requested = *self.focus.peek();
        if requested {
            self.focus.set(false);
            let _ = event.set_focus(true).await;
        }
    }
}
