//! Per-tab workspace identity, independent of shared project revision storage.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Component, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Band,
    Stepmania,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    version: u8,
    pub family: Family,
    pub root: PathBuf,
}
impl Target {
    pub fn new(family: Family, root: PathBuf) -> Self {
        Self {
            version: 1,
            family,
            root,
        }
    }
    pub(super) fn decode(raw: &str) -> Result<Self> {
        let target: Self =
            serde_json::from_str(raw).context("Invalid workspace recovery target")?;
        ensure!(
            target.version == 1,
            "Unsupported workspace recovery version"
        );
        let root = target.root.to_string_lossy();
        ensure!(
            target.root.is_absolute()
                && target.root.parent().is_some()
                && root.len() <= 4096
                && !root.contains('\0')
                && !target
                    .root
                    .components()
                    .any(|c| matches!(c, Component::ParentDir | Component::CurDir)),
            "Invalid workspace recovery path"
        );
        Ok(target)
    }
}

/// A corrupt tab target must never silently choose another tab's project.
pub(super) fn select(
    tab: Option<&str>,
    default: impl FnOnce() -> Result<Option<String>>,
) -> Result<Option<Target>> {
    match tab {
        Some(raw) => Target::decode(raw).map(Some),
        None => default()?.as_deref().map(Target::decode).transpose(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(family: Family, root: &str) -> String {
        serde_json::to_string(&Target::new(family, root.into())).unwrap()
    }

    #[test]
    fn each_tab_keeps_its_workspace_despite_a_different_default() {
        let band = encoded(Family::Band, "/projects/song-a");
        let dance = encoded(Family::Stepmania, "/projects/song-b");
        for (tab, default, family) in [
            (&band, &dance, Family::Band),
            (&dance, &band, Family::Stepmania),
        ] {
            let target = select(Some(tab), || Ok(Some(default.clone())))
                .unwrap()
                .unwrap();
            assert_eq!(target.family, family);
            assert_eq!(serde_json::to_string(&target).unwrap(), *tab);
        }
    }

    #[test]
    fn a_bound_tab_never_reads_shared_preferences() {
        let tab = encoded(Family::Band, "/projects/song-a");
        assert!(
            select(Some(&tab), || anyhow::bail!("unavailable shared storage"))
                .unwrap()
                .is_some()
        );
        assert!(
            select(Some("broken"), || panic!(
                "must not choose a different project"
            ))
            .is_err()
        );
    }

    #[test]
    fn fresh_tabs_use_the_default_or_allow_legacy_recovery() {
        let dance = encoded(Family::Stepmania, "/projects/song-b");
        assert_eq!(
            select(None, || Ok(Some(dance))).unwrap().unwrap().family,
            Family::Stepmania
        );
        assert!(select(None, || Ok(None)).unwrap().is_none());
    }

    #[test]
    fn unsupported_or_unsafe_targets_do_not_fall_back() {
        for raw in [
            r#"{"version":2,"family":"band","root":"/projects/a"}"#,
            r#"{"version":1,"family":"unknown","root":"/projects/a"}"#,
            r#"{"version":1,"family":"band","root":"relative"}"#,
            r#"{"version":1,"family":"band","root":"/projects/../a"}"#,
            r#"{"version":1,"family":"band","root":"/"}"#,
        ] {
            assert!(
                select(Some(raw), || panic!("must retain the failed target")).is_err(),
                "{raw}"
            );
        }
    }
}
