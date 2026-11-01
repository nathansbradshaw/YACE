//! Bounded chart discovery and one predictable home for library editing copies.
#[cfg(target_arch = "wasm32")]
use crate::storage as fs;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use sha2::{Digest, Sha256};
#[cfg(not(target_arch = "wasm32"))]
use std::fs;
use std::path::{Path, PathBuf};

fn is_file(path: impl AsRef<Path>) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        path.as_ref().is_file()
    }
    #[cfg(target_arch = "wasm32")]
    {
        fs::is_file(path)
    }
}

pub const SEARCH_DEPTH: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Project,
    Song,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub path: PathBuf,
    pub title: String,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Library {
    pub root: PathBuf,
    pub entries: Vec<Entry>,
}

fn entry(path: &Path) -> Option<Entry> {
    if is_file(path.join("job.json")) {
        let project = crate::song_layout::project_for_job(path);
        if is_file(project.join("project.json")) {
            return entry(&project);
        }
    }
    let kind = if is_file(path.join("project.json")) {
        Kind::Project
    } else if is_file(path.join("notes.chart"))
        || is_file(path.join("notes.mid"))
        || is_file(path.join(".yace/project/project.json"))
    {
        Kind::Song
    } else {
        return None;
    };
    let metadata_path = if kind == Kind::Project {
        path.join("source")
    } else {
        path.to_owned()
    };
    let metadata_path = if is_file(metadata_path.join("song.ini")) {
        metadata_path
    } else if is_file(path.join(".yace/project/source/song.ini")) {
        path.join(".yace/project/source")
    } else {
        metadata_path
    };
    let fields = crate::pipeline::song_ini(&metadata_path)
        .ok()
        .map(|raw| crate::pipeline::ini_fields(&raw));
    let title = fields
        .as_ref()
        .and_then(|fields| fields.get("name"))
        .filter(|name| !name.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        });
    let artist = fields
        .as_ref()
        .and_then(|fields| fields.get("artist"))
        .filter(|artist| !artist.trim().is_empty());
    let title = artist.map_or(title.clone(), |artist| format!("{artist} — {title}"));
    Some(Entry {
        path: path.to_owned(),
        title,
        kind,
    })
}

/// Accept a project file, a song, a containing folder, or a child of a project.
/// Stop at project/song boundaries so copied sources never appear as extra charts.
pub fn discover(selected: &Path) -> Result<Library> {
    let selected = fs::canonicalize(selected).context("Cannot find the selected folder")?;
    let root = if is_file(&selected) {
        selected
            .parent()
            .context("Selected file has no folder")?
            .to_owned()
    } else {
        selected
    };
    if is_file(root.join("project.json"))
        && let Some(found) = entry(&root)
    {
        return Ok(Library {
            root,
            entries: vec![found],
        });
    }
    for ancestor in root.ancestors().skip(1).take(SEARCH_DEPTH) {
        if is_file(ancestor.join("project.json"))
            || is_file(ancestor.join(".yace/project/project.json"))
            || (is_file(ancestor.join("job.json"))
                && is_file(crate::song_layout::project_for_job(ancestor).join("project.json")))
        {
            return Ok(Library {
                root: ancestor.to_owned(),
                entries: vec![entry(ancestor).unwrap()],
            });
        }
    }
    if let Some(found) = entry(&root) {
        return Ok(Library {
            root,
            entries: vec![found],
        });
    }
    for ancestor in root.ancestors().skip(1).take(SEARCH_DEPTH) {
        if let Some(found) = entry(ancestor) {
            return Ok(Library {
                root: ancestor.to_owned(),
                entries: vec![found],
            });
        }
    }
    let mut entries = Vec::new();
    scan(&root, SEARCH_DEPTH, &mut entries)?;
    entries.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then(a.path.cmp(&b.path))
    });
    ensure!(
        !entries.is_empty(),
        "No YACE projects or charts found within {SEARCH_DEPTH} folders of {}",
        root.display()
    );
    Ok(Library { root, entries })
}

fn scan(root: &Path, remaining: usize, out: &mut Vec<Entry>) -> Result<()> {
    if let Some(found) = entry(root) {
        out.push(found);
        return Ok(());
    }
    if remaining == 0 {
        return Ok(());
    }
    let mut children = fs::read_dir(root)?.collect::<std::io::Result<Vec<_>>>()?;
    children.sort_by_key(|child| child.file_name());
    for child in children {
        let name = child.file_name();
        let name = name.to_string_lossy();
        if child.file_type()?.is_dir()
            && !name.starts_with('.')
            && !name.ends_with(".app")
            && !matches!(name.as_ref(), "node_modules" | "target")
        {
            scan(&child.path(), remaining - 1, out)?;
        }
    }
    Ok(())
}

impl Library {
    /// Keep each song's editing state with that song. Earlier collection workspaces
    /// remain authoritative when they already contain saved edits.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn project_path(&self, item: &Entry) -> Result<PathBuf> {
        if item.kind == Kind::Project {
            return Ok(item.path.clone());
        }
        let managed = item.path.join(".yace/project");
        if managed.join("project.json").is_file() {
            return Ok(managed);
        }
        let base = if self.root == item.path {
            self.root
                .parent()
                .context("A song at the filesystem root needs a containing chart folder")?
        } else {
            &self.root
        };
        let path = item.path.canonicalize()?;
        let name: String = item
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .take(64)
            .collect();
        let managed_path = |base: &Path| {
            let identity = path.strip_prefix(base).unwrap_or(&path);
            let digest = format!(
                "{:x}",
                Sha256::digest(identity.to_string_lossy().as_bytes())
            );
            base.join(".yace/projects")
                .join(format!("{name}-{}", &digest[..16]))
        };
        // Opening a nested song directly must find edits created through its
        // containing collection, even when that collection has since moved.
        let preferred = managed_path(base);
        if preferred.join("project.json").is_file() {
            return Ok(preferred);
        }
        for ancestor in path.ancestors().skip(1).take(SEARCH_DEPTH) {
            let candidate = managed_path(ancestor);
            if candidate.join("project.json").is_file() {
                return Ok(candidate);
            }
        }
        Ok(managed)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(&self, item: &Entry) -> Result<PathBuf> {
        ensure!(self.entries.contains(item), "Chart is not in this library");
        let path = self.project_path(item)?;
        if path.join("project.json").is_file() {
            crate::editor::Store::new(&path).load()?;
            return Ok(path);
        }
        ensure!(
            item.kind == Kind::Song,
            "Saved project is missing project.json"
        );
        fs::create_dir_all(path.parent().context("Project has no parent")?)?;
        crate::editor::Store::create(&item.path, None, &path)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"{}").unwrap();
    }
    #[test]
    fn scans_three_levels_and_stops_at_project_and_song_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("one/two/three/notes.chart"));
        touch(&dir.path().join("one/two/three/source/notes.chart"));
        touch(&dir.path().join("deep/a/b/c/notes.chart"));
        touch(&dir.path().join("saved/project.json"));
        touch(&dir.path().join("saved/source/notes.chart"));
        touch(&dir.path().join(".yace/hidden/notes.chart"));
        let found = discover(dir.path()).unwrap();
        assert_eq!(found.entries.len(), 2);
        assert!(
            found
                .entries
                .iter()
                .any(|e| e.path.ends_with("one/two/three"))
        );
    }
    #[test]
    fn accepts_child_and_project_file_without_choosing_an_ambiguous_descendant() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("saved/project.json"));
        touch(&dir.path().join("saved/source/notes.chart"));
        fs::create_dir_all(dir.path().join("saved/checkpoints/child")).unwrap();
        let child = discover(&dir.path().join("saved/checkpoints/child")).unwrap();
        assert_eq!(child.entries[0].kind, Kind::Project);
        assert_eq!(child, discover(&dir.path().join("saved/source")).unwrap());
        assert_eq!(
            child,
            discover(&dir.path().join("saved/project.json")).unwrap()
        );
        touch(&dir.path().join("another/project.json"));
        assert_eq!(discover(dir.path()).unwrap().entries.len(), 2);
    }
    #[test]
    fn generation_destination_opens_its_saved_project_once() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("generation/job.json"));
        touch(&dir.path().join("generation/project/project.json"));
        touch(&dir.path().join("generation/song/notes.chart"));
        touch(&dir.path().join("generation/game-song/notes.chart"));
        fs::create_dir_all(dir.path().join("generation/song/references")).unwrap();
        let library = discover(dir.path()).unwrap();
        assert_eq!(library.entries.len(), 1);
        assert!(library.entries[0].path.ends_with("generation/project"));
        for child in ["song", "game-song", "song/references"] {
            let selected = discover(&dir.path().join("generation").join(child)).unwrap();
            assert_eq!(selected.entries, library.entries);
        }
    }
    #[test]
    fn duplicate_song_names_keep_separate_private_workspaces_inside_each_song() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("artist-a/song/notes.chart"));
        touch(&dir.path().join("artist-b/song/notes.chart"));
        let library = discover(dir.path()).unwrap();
        let a = library.project_path(&library.entries[0]).unwrap();
        let b = library.project_path(&library.entries[1]).unwrap();
        assert_ne!(a, b);
        assert_eq!(
            a,
            discover(dir.path())
                .unwrap()
                .project_path(&library.entries[0])
                .unwrap()
        );
        assert_eq!(a, library.entries[0].path.join(".yace/project"));
        assert_eq!(b, library.entries[1].path.join(".yace/project"));
        let single = discover(&library.entries[0].path).unwrap();
        assert_eq!(single.project_path(&single.entries[0]).unwrap(), a);
    }

    #[test]
    fn earlier_collection_workspaces_are_reused_without_replacing_saved_edits() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("artist/song/notes.chart"));
        let library = discover(dir.path()).unwrap();
        let identity = "artist/song";
        let hash = format!("{:x}", Sha256::digest(identity.as_bytes()));
        let legacy = dir
            .path()
            .canonicalize()
            .unwrap()
            .join(".yace/projects")
            .join(format!("song-{}", &hash[..16]));
        touch(&legacy.join("project.json"));
        assert_eq!(library.project_path(&library.entries[0]).unwrap(), legacy);
        let single = discover(&library.entries[0].path).unwrap();
        assert_eq!(single.project_path(&single.entries[0]).unwrap(), legacy);
    }

    #[test]
    fn private_generated_state_and_visible_song_are_one_library_entry() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("song/notes.chart"));
        touch(&dir.path().join("song/.yace/project/project.json"));
        touch(&dir.path().join("song/.yace/project/source/notes.chart"));
        touch(
            &dir.path()
                .join("song/.yace/jobs/generation/song/notes.chart"),
        );
        let library = discover(dir.path()).unwrap();
        assert_eq!(library.entries.len(), 1);
        assert!(library.entries[0].path.ends_with("song"));
        assert!(
            library
                .project_path(&library.entries[0])
                .unwrap()
                .ends_with("song/.yace/project")
        );
    }
    #[cfg(unix)]
    #[test]
    fn does_not_follow_directory_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        touch(&other.path().join("notes.chart"));
        std::os::unix::fs::symlink(other.path(), dir.path().join("loop")).unwrap();
        assert!(discover(dir.path()).is_err());
    }
}
