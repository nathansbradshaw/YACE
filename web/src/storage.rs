//! A synchronous Rust working set; durable snapshots are committed by the client.
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
thread_local! { static FILES: RefCell<BTreeMap<PathBuf, Arc<Vec<u8>>>> = RefCell::default(); }
thread_local! { static DIRTY: RefCell<BTreeSet<PathBuf>> = RefCell::default(); }
pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    FILES.with(|files| {
        files
            .borrow()
            .get(path.as_ref())
            .map(|bytes| bytes.as_ref().clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Project file is missing"))
    })
}
pub fn shared(path: impl AsRef<Path>) -> io::Result<Arc<Vec<u8>>> {
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
    DIRTY.with(|dirty| {
        dirty.borrow_mut().insert(path.as_ref().to_owned());
    });
    FILES.with(|files| {
        files
            .borrow_mut()
            .insert(path.as_ref().to_owned(), Arc::new(bytes.as_ref().to_vec()));
    });
    Ok(())
}
pub fn exists(path: impl AsRef<Path>) -> bool {
    FILES.with(|files| {
        files
            .borrow()
            .keys()
            .any(|key| key == path.as_ref() || key.starts_with(path.as_ref()))
    })
}
pub fn is_file(path: impl AsRef<Path>) -> bool {
    FILES.with(|files| files.borrow().contains_key(path.as_ref()))
}
pub fn list(root: &Path) -> Vec<(PathBuf, Arc<Vec<u8>>)> {
    FILES.with(|files| {
        files
            .borrow()
            .iter()
            .filter(|(path, _)| path.starts_with(root))
            .map(|(path, bytes)| (path.clone(), bytes.clone()))
            .collect()
    })
}
pub fn clear(root: &Path) {
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
pub fn create_dir(_path: impl AsRef<Path>) -> io::Result<()> {
    Ok(())
}
pub fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<u64> {
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
    let items = list(from.as_ref());
    if items.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Project source is missing",
        ));
    }
    FILES.with(|files| {
        let mut files = files.borrow_mut();
        for (path, bytes) in items {
            let relative = path.strip_prefix(from.as_ref()).unwrap();
            let target = if relative.as_os_str().is_empty() {
                to.as_ref().to_owned()
            } else {
                to.as_ref().join(relative)
            };
            files.remove(&path);
            files.insert(target, bytes);
        }
    });
    Ok(())
}
pub fn pending(root: &Path) -> Vec<(PathBuf, Arc<Vec<u8>>)> {
    DIRTY.with(|dirty| {
        dirty
            .borrow()
            .iter()
            .filter(|path| path.starts_with(root))
            .filter_map(|path| shared(path).ok().map(|bytes| (path.clone(), bytes)))
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
