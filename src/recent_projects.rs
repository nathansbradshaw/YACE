//! Small, local history of successfully opened projects. Removing history never
//! removes a project or its browser recovery snapshot.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const LIMIT: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub path: PathBuf,
    pub title: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    pub entries: Vec<Entry>,
}

pub fn title(metadata: &std::collections::BTreeMap<String, String>, path: &Path) -> String {
    let name = metadata
        .get("name")
        .filter(|name| !name.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        });
    metadata
        .get("artist")
        .filter(|artist| !artist.trim().is_empty())
        .map_or_else(|| name.clone(), |artist| format!("{artist} — {name}"))
}

impl History {
    /// Update recents only after the existing history was read successfully.
    /// Opening a project is still allowed when this fails; callers show a
    /// separate warning and preserve the unreadable history for recovery.
    pub fn record_open(path: PathBuf, title: String) -> Result<Self> {
        Self::record_loaded(Self::load(), path, title, Self::save)
    }

    fn record_loaded(
        loaded: Result<Self>,
        path: PathBuf,
        title: String,
        save: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<Self> {
        let mut history = loaded?;
        history.remember(path, title);
        save(&history)?;
        Ok(history)
    }

    pub fn remember(&mut self, path: PathBuf, title: String) {
        self.forget(&path);
        self.entries.insert(0, Entry { path, title });
        self.entries.truncate(LIMIT);
    }

    pub fn forget(&mut self, path: &Path) {
        self.entries.retain(|entry| entry.path != path);
    }

    pub fn load() -> Result<Self> {
        #[cfg(target_arch = "wasm32")]
        {
            let value = browser_storage()?
                .get_item("YACE-recent-projects")
                .map_err(|error| anyhow::anyhow!("Recent projects: {error:?}"))?;
            value.map_or_else(
                || Ok(Self::default()),
                |value| Ok(serde_json::from_str(&value)?),
            )
        }
        #[cfg(all(not(target_arch = "wasm32"), not(feature = "browser")))]
        {
            // UI unit tests use disposable in-memory history. Disk persistence
            // is exercised explicitly with load_from/save_to below.
            if cfg!(test) {
                return Ok(Self::default());
            }
            Self::load_from(&history_path()?)
        }
        // LiveView uploads are temporary and belong to individual sessions.
        #[cfg(all(not(target_arch = "wasm32"), feature = "browser"))]
        Ok(Self::default())
    }

    pub fn save(&self) -> Result<()> {
        #[cfg(target_arch = "wasm32")]
        {
            browser_storage()?
                .set_item("YACE-recent-projects", &serde_json::to_string(self)?)
                .map_err(|error| anyhow::anyhow!("Recent projects: {error:?}"))
        }
        #[cfg(all(not(target_arch = "wasm32"), not(feature = "browser")))]
        {
            if cfg!(test) {
                return Ok(());
            }
            self.save_to(&history_path()?)
        }
        #[cfg(all(not(target_arch = "wasm32"), feature = "browser"))]
        Ok(())
    }

    #[cfg(all(not(target_arch = "wasm32"), not(feature = "browser")))]
    fn load_from(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    #[cfg(all(not(target_arch = "wasm32"), not(feature = "browser")))]
    fn save_to(&self, path: &Path) -> Result<()> {
        use std::io::Write;
        let parent = path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
        file.as_file().sync_all()?;
        file.persist(path)?;
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
fn browser_storage() -> Result<web_sys::Storage> {
    web_sys::window()
        .and_then(|window| window.local_storage().ok().flatten())
        .ok_or_else(|| anyhow::anyhow!("Recent-project storage is unavailable"))
}

#[cfg(all(not(target_arch = "wasm32"), not(feature = "browser")))]
fn history_path() -> Result<PathBuf> {
    use anyhow::Context;
    if let Some(path) = std::env::var_os("YACE_RECENT_PROJECTS") {
        return Ok(path.into());
    }
    #[cfg(target_os = "macos")]
    let root = PathBuf::from(std::env::var_os("HOME").context("Home directory unavailable")?)
        .join("Library/Application Support");
    #[cfg(target_os = "windows")]
    let root =
        PathBuf::from(std::env::var_os("APPDATA").context("App data directory unavailable")?);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let root = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .context("App state directory unavailable")?;
    Ok(root.join("YACE/recent-projects.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(feature = "browser"))]
    #[test]
    fn explicit_open_does_not_replace_unreadable_history() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("recent-projects.json");
        let partial = b"{\"entries\":[{\"path\":\"/recoverable/project\"}";
        std::fs::write(&path, partial).unwrap();
        let result = History::record_loaded(
            History::load_from(&path),
            root.path().join("opened-project"),
            "Explicitly opened".into(),
            |history| history.save_to(&path),
        );
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), partial);
        let mut repaired = History::default();
        repaired.remember("/recovered/dance".into(), "Retained".into());
        repaired.save_to(&path).unwrap();
        let history = History::record_loaded(
            History::load_from(&path),
            root.path().join("opened-project"),
            "Explicitly opened".into(),
            |history| history.save_to(&path),
        )
        .unwrap();
        assert_eq!(history.entries.len(), 2);
        assert_eq!(History::load_from(&path).unwrap(), history);
    }

    #[test]
    fn reopening_moves_to_front_updates_title_and_bounds_history() {
        let mut history = History::default();
        for index in 0..15 {
            history.remember(format!("/charts/{index}").into(), "Song".into());
        }
        history.remember("/charts/9".into(), "Renamed song".into());
        assert_eq!(history.entries.len(), LIMIT);
        assert_eq!(history.entries[0].title, "Renamed song");
        assert_eq!(history.entries[0].path, Path::new("/charts/9"));
        assert_eq!(
            history
                .entries
                .iter()
                .filter(|entry| entry.path == Path::new("/charts/9"))
                .count(),
            1
        );
    }

    #[cfg(not(feature = "browser"))]
    #[test]
    fn history_survives_restart_and_removal_preserves_project_files() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(project.join("project.json"), b"original").unwrap();
        let path = root.path().join("state/recent.json");
        assert_eq!(History::load_from(&path).unwrap(), History::default());
        let mut history = History::default();
        history.remember(project.clone(), "Title".into());
        history.save_to(&path).unwrap();
        let mut restored = History::load_from(&path).unwrap();
        assert_eq!(restored, history);
        restored.forget(&project);
        restored.save_to(&path).unwrap();
        assert!(History::load_from(&path).unwrap().entries.is_empty());
        assert_eq!(
            std::fs::read(project.join("project.json")).unwrap(),
            b"original"
        );
        std::fs::write(&path, b"broken").unwrap();
        assert!(History::load_from(&path).is_err());
    }
}
