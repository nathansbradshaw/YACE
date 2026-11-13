//! Ephemeral action feedback. Never part of the authored chart or undo history.
use super::state::Studio;
use dioxus::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    Open,
    Import,
    Recording,
    Backup,
}
impl FileAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Opening project…",
            Self::Import => "Importing song…",
            Self::Recording => "Choosing recording…",
            Self::Backup => "Preparing project backup…",
        }
    }
}

/// Acquire before spawning, so two clicks in the same render cannot start two
/// pickers. Dropping even an unpolled/cancelled future releases its operation.
pub struct FileGuard {
    signal: Signal<Option<FileAction>>,
    status: Signal<String>,
    previous: String,
    action: FileAction,
}
impl FileGuard {
    pub fn start(
        mut signal: Signal<Option<FileAction>>,
        mut status: Signal<String>,
        action: FileAction,
    ) -> Option<Self> {
        let mut value = signal.write();
        if value.is_some() {
            return None;
        }
        *value = Some(action);
        drop(value);
        let previous = status.replace(action.label().into());
        Some(Self {
            signal,
            status,
            previous,
            action,
        })
    }
}
impl Drop for FileGuard {
    fn drop(&mut self) {
        if let Ok(mut value) = self.signal.try_write() {
            *value = None;
        }
        if let Ok(mut status) = self.status.try_write()
            && status.as_str() == self.action.label()
        {
            *status = self.previous.clone();
        }
    }
}

/// Also protects the native backup's save dialog, before compression starts.
pub struct BusyGuard(Signal<bool>);
impl BusyGuard {
    pub fn start(mut signal: Signal<bool>) -> Option<Self> {
        let mut value = signal.write();
        if *value {
            return None;
        }
        *value = true;
        drop(value);
        Some(Self(signal))
    }
}
impl Drop for BusyGuard {
    fn drop(&mut self) {
        if let Ok(mut value) = self.0.try_write() {
            *value = false;
        }
    }
}

#[component]
pub fn BusyLabel(busy: bool, label: String, active: String) -> Element {
    rsx! {
        if busy {span {class: "activity-spinner", aria_hidden: "true"}}
        if busy {"{active}"} else {"{label}"}
    }
}

#[component]
pub fn Status(studio: Studio, title: String) -> Element {
    let action = (studio.file_activity)();
    let busy = action.is_some()
        || (studio.library_busy)()
        || (studio.export_busy)()
        || (studio.group_busy)();
    let text = (studio.status)();
    rsx! {
        style { {include_str!("../../assets/activity.css")} }
        span {class: "activity-status", role: "status", aria_live: "polite", aria_atomic: "true", title,
            if busy {span {class: "activity-spinner", aria_hidden: "true"}}
            span {"{text}"}
        }
    }
}

#[component]
pub fn JobNotice(
    busy: bool,
    state: Option<crate::generation::State>,
    failed: bool,
    detail: String,
    onopen: EventHandler,
) -> Element {
    let mut dismissed = use_signal(|| false);
    use_effect(move || {
        if busy {
            dismissed.set(false);
        }
    });
    if dismissed() {
        return rsx! {};
    }
    let label = job_label(busy, state, failed);
    rsx! {
        aside {class: "generation-running", aria_label: "Background analysis",
            button {class: "generation-summary-open", onclick: move |_| onopen.call(()), aria_label: "Show analysis details",
                strong {BusyLabel {busy,label:label,active:label}}
                span {role: "status", aria_live: "polite", "{detail}"}
            }
            if !busy {button {class: "generation-summary-dismiss", aria_label: "Dismiss analysis status", onclick: move |_| dismissed.set(true), "×"}}
        }
    }
}

pub fn job_label(
    busy: bool,
    state: Option<crate::generation::State>,
    failed: bool,
) -> &'static str {
    use crate::generation::State;
    if busy {
        "Analysis in progress"
    } else {
        match state {
            Some(State::Cancelled) => "Analysis cancelled · saved stages retained",
            _ if failed => "Analysis needs attention",
            Some(State::Complete) => "Draft ready to review",
            Some(State::Failed) => "Analysis failed · review or retry",
            _ => "Analysis saved · resume when ready",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_guards_block_duplicates_and_release_on_cancelled_future() {
        fn app() -> Element {
            use_hook(|| {
                let file = Signal::new(None);
                let busy = Signal::new(false);
                let mut status = Signal::new("Ready".to_owned());
                let guard = FileGuard::start(file, status, FileAction::Open).unwrap();
                assert_eq!(status.peek().as_str(), "Opening project…");
                assert!(FileGuard::start(file, status, FileAction::Import).is_none());
                let unpolled = async move {
                    let _guard = guard;
                };
                drop(unpolled);
                assert_eq!(*file.peek(), None);
                assert_eq!(status.peek().as_str(), "Ready");
                let next = FileGuard::start(file, status, FileAction::Recording).unwrap();
                status.set("Recording ready".into());
                let package = BusyGuard::start(busy).unwrap();
                assert!(BusyGuard::start(busy).is_none());
                drop(package);
                assert!(!*busy.peek());
                assert!(BusyGuard::start(busy).is_some());
                drop(next);
                assert_eq!(*file.peek(), None);
                assert_eq!(status.peek().as_str(), "Recording ready");
            });
            rsx! {}
        }
        VirtualDom::new(app).rebuild_in_place();
    }

    #[test]
    fn terminal_feedback_distinguishes_saved_failed_cancelled_and_complete() {
        use crate::generation::State;
        assert_eq!(
            job_label(true, Some(State::Failed), true),
            "Analysis in progress"
        );
        assert_eq!(
            job_label(false, Some(State::Cancelled), true),
            "Analysis cancelled · saved stages retained"
        );
        assert_eq!(
            job_label(false, Some(State::Complete), false),
            "Draft ready to review"
        );
        assert_eq!(
            job_label(false, Some(State::Complete), true),
            "Analysis needs attention"
        );
        assert_eq!(
            job_label(false, Some(State::Queued), false),
            "Analysis saved · resume when ready"
        );
        assert_eq!(
            job_label(false, Some(State::Failed), false),
            "Analysis failed · review or retry"
        );
    }
}
