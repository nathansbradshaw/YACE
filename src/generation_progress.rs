//! Lightweight worker progress, separate from recoverable generation state.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Progress {
    pub stage: String,
    pub detail: String,
    pub completed: Option<u64>,
    pub total: Option<u64>,
    pub started_ms: u64,
    pub updated_ms: u64,
}

struct Reporter {
    root: PathBuf,
    progress: Progress,
}

thread_local! {
    static REPORTER: RefCell<Option<Reporter>> = const { RefCell::new(None) };
}

pub(crate) struct Scope(Option<Reporter>);

impl Drop for Scope {
    fn drop(&mut self) {
        REPORTER.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

pub(crate) fn begin(root: &Path) -> Scope {
    let now = now_ms();
    let reporter = Reporter {
        root: root.to_owned(),
        progress: Progress {
            stage: "starting".into(),
            detail: "Starting generation worker".into(),
            completed: None,
            total: None,
            started_ms: now,
            updated_ms: now,
        },
    };
    Scope(REPORTER.with(|slot| slot.replace(Some(reporter))))
}

pub fn read(root: &Path) -> Result<Progress> {
    Ok(serde_json::from_slice(&std::fs::read(
        root.join("progress.json"),
    )?)?)
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn emit(change: impl FnOnce(&mut Progress)) {
    REPORTER.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(reporter) = slot.as_mut() else {
            return;
        };
        change(&mut reporter.progress);
        reporter.progress.updated_ms = now_ms();
        // Progress is advisory. A full disk must not replace the real worker error.
        if let Err(error) =
            crate::generation::write_json(&reporter.root.join("progress.json"), &reporter.progress)
        {
            eprintln!("Could not publish generation progress: {error:#}");
        }
    });
}

pub(crate) fn stage(stage: &str, detail: &str) {
    emit(|progress| {
        progress.stage = stage.into();
        progress.detail = detail.into();
        progress.completed = None;
        progress.total = None;
    });
}

pub(crate) fn detail(detail: impl Into<String>) {
    emit(|progress| {
        progress.detail = detail.into();
        progress.completed = None;
        progress.total = None;
    });
}

pub(crate) fn units(detail: impl Into<String>, completed: usize, total: usize) {
    emit(|progress| {
        progress.detail = detail.into();
        progress.completed = Some(completed.min(total) as u64);
        progress.total = (total > 0).then_some(total as u64);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_real_work_and_restores_outer_job_after_nested_scope() {
        let outer = tempfile::tempdir().unwrap();
        let inner = tempfile::tempdir().unwrap();
        let _scope = begin(outer.path());
        stage("separation", "Loading separator");
        units("Separating chunk 2 of 8", 1, 8);
        let progress = read(outer.path()).unwrap();
        assert_eq!(progress.stage, "separation");
        assert_eq!((progress.completed, progress.total), (Some(1), Some(8)));
        assert!(progress.updated_ms >= progress.started_ms);
        {
            let _scope = begin(inner.path());
            stage("transcription", "Drums");
        }
        detail("Validating all six stems");
        assert_eq!(read(outer.path()).unwrap().total, None);
        assert_eq!(read(inner.path()).unwrap().detail, "Drums");
    }

    #[test]
    fn stage_transition_clears_previous_chunk_counts_and_legacy_jobs_need_no_progress() {
        let root = tempfile::tempdir().unwrap();
        assert!(read(root.path()).is_err());
        let _scope = begin(root.path());
        units("Final chunk", 9, 9);
        stage("editor", "Preparing editing project");
        let progress = read(root.path()).unwrap();
        assert_eq!(progress.completed, None);
        assert_eq!(progress.total, None);
        assert_eq!(progress.stage, "editor");
    }
}
