//! One native startup route, shared by band and standalone Dance projects.
use super::state::Studio;
use crate::recent_projects::History;
use anyhow::Result;
use dioxus::prelude::WritableExt;
use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
enum Target {
    Requested(PathBuf),
    Recent(PathBuf),
    LegacyDance,
}

fn target(requested: Option<PathBuf>, history: impl FnOnce() -> Result<History>) -> Result<Target> {
    // An explicit open must work even if the unrelated history is unavailable.
    if let Some(path) = requested {
        return Ok(Target::Requested(path));
    }
    Ok(history()?
        .entries
        .into_iter()
        .next()
        .map_or(Target::LegacyDance, |entry| Target::Recent(entry.path)))
}

pub async fn open(mut studio: Studio, requested: Option<PathBuf>) {
    match target(requested, History::load) {
        Ok(Target::Requested(path)) => studio.open_library(path).await,
        Ok(Target::Recent(path)) => studio.open_recent(path).await,
        Ok(Target::LegacyDance) => super::dance::restore_legacy().await,
        Err(error) => studio.error.set(Some(format!(
            "Could not restore the last project: {error:#}. Open a project explicitly; existing projects and recovery files have been kept."
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recent_projects::Entry;

    #[test]
    fn explicit_open_ignores_unavailable_history() {
        let path = PathBuf::from("/explicit/song");
        assert_eq!(
            target(Some(path.clone()), || panic!("must not read history")).unwrap(),
            Target::Requested(path)
        );
    }

    #[test]
    fn latest_project_wins_without_trying_another_family_or_older_entry() {
        for newest in ["/projects/band", "/projects/dance", "/missing/project"] {
            let history = History {
                entries: [newest, "/older/dance"]
                    .map(|path| Entry {
                        path: path.into(),
                        title: "Previously saved project".into(),
                    })
                    .into(),
            };
            assert_eq!(
                target(None, || Ok(history)).unwrap(),
                Target::Recent(newest.into())
            );
        }
    }

    #[test]
    fn legacy_recovery_is_only_used_when_history_is_valid_and_empty() {
        assert_eq!(
            target(None, || Ok(History::default())).unwrap(),
            Target::LegacyDance
        );
        assert!(target(None, || anyhow::bail!("Invalid history document")).is_err());
    }
}
