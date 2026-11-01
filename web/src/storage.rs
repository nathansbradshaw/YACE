//! A synchronous Rust working set; durable snapshots are committed by the client.
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
thread_local! { static FILES: RefCell<BTreeMap<PathBuf, Arc<Vec<u8>>>> = RefCell::default(); }
thread_local! { static DIRECTORIES: RefCell<BTreeSet<PathBuf>> = RefCell::default(); }
thread_local! { static DIRTY: RefCell<BTreeSet<PathBuf>> = RefCell::default(); }
thread_local! { static PROJECTS: RefCell<BTreeMap<PathBuf,Arc<crate::editor::Project>>> = RefCell::default(); }
thread_local! { static UNENCODED: RefCell<BTreeSet<PathBuf>> = RefCell::default(); }
pub fn cached_project(root: &Path) -> Option<Arc<crate::editor::Project>> {
    PROJECTS.with(|projects| projects.borrow().get(&root.join("project.json")).cloned())
}
pub fn remember_project(root: &Path, project: &crate::editor::Project) {
    PROJECTS.with(|projects| {
        projects
            .borrow_mut()
            .insert(root.join("project.json"), Arc::new(project.clone()));
    });
}
pub fn save_project(root: &Path, project: &crate::editor::Project) {
    remember_project(root, project);
    let path = root.join("project.json");
    UNENCODED.with(|paths| {
        paths.borrow_mut().insert(path.clone());
    });
    DIRTY.with(|paths| {
        paths.borrow_mut().insert(path);
    });
}
fn materialize(path: &Path) -> io::Result<()> {
    if !UNENCODED.with(|paths| paths.borrow().contains(path)) {
        return Ok(());
    }
    let project = PROJECTS
        .with(|projects| projects.borrow().get(path).cloned())
        .ok_or_else(|| io::Error::other("Project cache is missing"))?;
    let bytes = serde_json::to_vec(project.as_ref()).map_err(io::Error::other)?;
    FILES.with(|files| {
        files.borrow_mut().insert(path.to_owned(), Arc::new(bytes));
    });
    UNENCODED.with(|paths| {
        paths.borrow_mut().remove(path);
    });
    Ok(())
}
fn invalidate(path: &Path) -> io::Result<()> {
    let keys: Vec<_> = PROJECTS.with(|projects| {
        projects
            .borrow()
            .keys()
            .filter(|key| {
                key.as_path() == path || path.starts_with(key.parent().unwrap().join("source"))
            })
            .cloned()
            .collect()
    });
    for key in keys {
        if key != path {
            materialize(&key)?;
        }
        PROJECTS.with(|projects| {
            projects.borrow_mut().remove(&key);
        });
        UNENCODED.with(|paths| {
            paths.borrow_mut().remove(&key);
        });
    }
    Ok(())
}
pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    materialize(path.as_ref())?;
    FILES.with(|files| {
        files
            .borrow()
            .get(path.as_ref())
            .map(|bytes| bytes.as_ref().clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Project file is missing"))
    })
}
pub fn shared(path: impl AsRef<Path>) -> io::Result<Arc<Vec<u8>>> {
    materialize(path.as_ref())?;
    FILES.with(|files| {
        files
            .borrow()
            .get(path.as_ref())
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Project file is missing"))
    })
}
pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    write_owned(path, bytes.as_ref().to_vec())
}
pub fn write_owned(path: impl AsRef<Path>, bytes: Vec<u8>) -> io::Result<()> {
    invalidate(path.as_ref())?;
    DIRTY.with(|dirty| {
        dirty.borrow_mut().insert(path.as_ref().to_owned());
    });
    FILES.with(|files| {
        files
            .borrow_mut()
            .insert(path.as_ref().to_owned(), Arc::new(bytes));
    });
    Ok(())
}
pub fn exists(path: impl AsRef<Path>) -> bool {
    FILES.with(|files| {
        files
            .borrow()
            .keys()
            .any(|key| key == path.as_ref() || key.starts_with(path.as_ref()))
    }) || DIRECTORIES.with(|dirs| {
        dirs.borrow()
            .iter()
            .any(|dir| dir == path.as_ref() || dir.starts_with(path.as_ref()))
    })
}
pub fn is_file(path: impl AsRef<Path>) -> bool {
    FILES.with(|files| files.borrow().contains_key(path.as_ref()))
        || PROJECTS.with(|projects| projects.borrow().contains_key(path.as_ref()))
}
pub fn is_dir(path: impl AsRef<Path>) -> bool {
    exists(&path) && !is_file(path)
}
pub fn list(root: &Path) -> io::Result<Vec<(PathBuf, Arc<Vec<u8>>)>> {
    let pending: Vec<_> = UNENCODED.with(|paths| {
        paths
            .borrow()
            .iter()
            .filter(|path| path.starts_with(root))
            .cloned()
            .collect()
    });
    for path in pending {
        materialize(&path)?;
    }
    Ok(FILES.with(|files| {
        files
            .borrow()
            .iter()
            .filter(|(path, _)| path.starts_with(root))
            .map(|(path, bytes)| (path.clone(), bytes.clone()))
            .collect()
    }))
}
pub fn clear(root: &Path) {
    DIRECTORIES.with(|dirs| dirs.borrow_mut().retain(|path| !path.starts_with(root)));
    crate::part_analysis::forget_root(root);
    PROJECTS.with(|projects| {
        projects
            .borrow_mut()
            .retain(|path, _| !path.starts_with(root))
    });
    UNENCODED.with(|paths| paths.borrow_mut().retain(|path| !path.starts_with(root)));
    FILES.with(|files| files.borrow_mut().retain(|path, _| !path.starts_with(root)));
    DIRTY.with(|dirty| dirty.borrow_mut().retain(|path| !path.starts_with(root)));
}
pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    if exists(&path) {
        Ok(path.as_ref().to_owned())
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Project path is missing",
        ))
    }
}
pub fn is_absolute(path: impl AsRef<Path>) -> bool {
    let text = path.as_ref().to_string_lossy();
    let bytes = text.as_bytes();
    text.starts_with('/')
        || (bytes.len() > 2
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'))
}
pub fn create_dir(path: impl AsRef<Path>) -> io::Result<()> {
    if exists(&path) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Directory already exists",
        ));
    }
    DIRECTORIES.with(|dirs| dirs.borrow_mut().insert(path.as_ref().to_owned()));
    Ok(())
}
pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    if is_file(&path) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Path is a file",
        ));
    }
    DIRECTORIES.with(|dirs| {
        let mut dirs = dirs.borrow_mut();
        for parent in path
            .as_ref()
            .ancestors()
            .filter(|p| !p.as_os_str().is_empty())
        {
            dirs.insert(parent.to_owned());
        }
    });
    Ok(())
}
pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    if !is_file(&path) {
        return Err(io::Error::new(io::ErrorKind::NotFound, "File is missing"));
    }
    clear(path.as_ref());
    Ok(())
}
pub fn remove_dir(path: impl AsRef<Path>) -> io::Result<()> {
    if !list(path.as_ref())?.is_empty()
        || DIRECTORIES.with(|dirs| {
            dirs.borrow()
                .iter()
                .any(|dir| dir != path.as_ref() && dir.starts_with(path.as_ref()))
        })
    {
        return Err(io::Error::other("Directory is not empty"));
    }
    DIRECTORIES.with(|dirs| dirs.borrow_mut().remove(path.as_ref()));
    Ok(())
}
pub fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<u64> {
    invalidate(to.as_ref())?;
    DIRTY.with(|dirty| {
        dirty.borrow_mut().insert(to.as_ref().to_owned());
    });
    let bytes = shared(from)?;
    let len = bytes.len() as u64;
    FILES.with(|files| {
        files.borrow_mut().insert(to.as_ref().to_owned(), bytes);
    });
    Ok(len)
}
pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    let items = list(from.as_ref())?;
    let directories: Vec<_> = DIRECTORIES.with(|dirs| {
        dirs.borrow()
            .iter()
            .filter(|path| path.starts_with(from.as_ref()))
            .cloned()
            .collect()
    });
    if items.is_empty() && directories.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Project source is missing",
        ));
    }
    clear(from.as_ref());
    for path in directories {
        DIRECTORIES.with(|dirs| {
            dirs.borrow_mut()
                .insert(to.as_ref().join(path.strip_prefix(from.as_ref()).unwrap()))
        });
    }
    for (path, bytes) in items {
        let relative = path.strip_prefix(from.as_ref()).unwrap();
        let target = if relative.as_os_str().is_empty() {
            to.as_ref().to_owned()
        } else {
            to.as_ref().join(relative)
        };
        invalidate(&target)?;
        DIRTY.with(|dirty| {
            dirty.borrow_mut().insert(target.clone());
        });
        FILES.with(|files| {
            files.borrow_mut().insert(target, bytes);
        });
    }
    Ok(())
}
pub fn pending(root: &Path) -> io::Result<Vec<(PathBuf, Arc<Vec<u8>>)>> {
    DIRTY.with(|dirty| {
        dirty
            .borrow()
            .iter()
            .filter(|path| path.starts_with(root))
            .map(|path| shared(path).map(|bytes| (path.clone(), bytes)))
            .collect()
    })
}
pub fn acknowledge(files: Vec<(PathBuf, Arc<Vec<u8>>)>) {
    DIRTY.with(|dirty| {
        let mut dirty = dirty.borrow_mut();
        for (path, bytes) in files {
            if shared(&path).is_ok_and(|current| Arc::ptr_eq(&current, &bytes)) {
                dirty.remove(&path);
            }
        }
    });
}
pub fn write_new(path: impl AsRef<Path>, bytes: &[u8]) -> io::Result<()> {
    if is_file(&path) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "This file already exists",
        ));
    }
    write(path, bytes)
}
pub fn unique_id() -> String {
    crate::files::unique_root()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

// These read-only filesystem views let the same saved-kit validator run on
// browser imports. The working set cannot contain symlinks or device files.
pub struct Metadata {
    file: bool,
    length: u64,
}
impl Metadata {
    pub fn is_symlink(&self) -> bool {
        false
    }
    pub fn is_file(&self) -> bool {
        self.file
    }
    pub fn is_dir(&self) -> bool {
        !self.file
    }
    pub fn len(&self) -> u64 {
        self.length
    }
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
    pub fn modified(&self) -> io::Result<std::time::SystemTime> {
        Ok(std::time::UNIX_EPOCH)
    }
}
pub fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    let path = path.as_ref();
    if is_file(path) {
        Ok(Metadata {
            file: true,
            length: shared(path)?.len() as u64,
        })
    } else if exists(path) {
        Ok(Metadata {
            file: false,
            length: 0,
        })
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Project path is missing",
        ))
    }
}
pub struct DirEntry(PathBuf);
impl DirEntry {
    pub fn path(&self) -> PathBuf {
        self.0.clone()
    }
    pub fn file_name(&self) -> std::ffi::OsString {
        self.0.file_name().unwrap_or_default().to_owned()
    }
    pub fn file_type(&self) -> io::Result<Metadata> {
        symlink_metadata(&self.0)
    }
}
pub fn read_dir(root: impl AsRef<Path>) -> io::Result<std::vec::IntoIter<io::Result<DirEntry>>> {
    let root = root.as_ref();
    if !exists(root) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Project folder is missing",
        ));
    }
    let mut children: BTreeSet<_> = list(root)?
        .iter()
        .filter_map(|(path, _)| {
            path.strip_prefix(root)
                .ok()?
                .components()
                .next()
                .map(|part| root.join(part.as_os_str()))
        })
        .collect();
    DIRECTORIES.with(|dirs| {
        for path in dirs.borrow().iter() {
            if let Ok(relative) = path.strip_prefix(root)
                && let Some(part) = relative.components().next()
            {
                children.insert(root.join(part.as_os_str()));
            }
        }
    });
    Ok(children
        .into_iter()
        .map(|path| Ok(DirEntry(path)))
        .collect::<Vec<_>>()
        .into_iter())
}
pub struct File {
    bytes: Arc<Vec<u8>>,
    position: usize,
}
impl File {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            bytes: shared(path)?,
            position: 0,
        })
    }
}
impl std::io::Read for File {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = buffer.len().min(self.bytes.len() - self.position);
        buffer[..count].copy_from_slice(&self.bytes[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}
