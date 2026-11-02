//! Native file adapter; the simfile implementation itself has no filesystem IO.
use crate::stepmania::{
    Project,
    bundle::{Files, MANIFEST},
};
use anyhow::{Context, Result, ensure};
use std::{fs, path::Path};
fn portable_name(root: &Path, path: &Path) -> Result<String> {
    Ok(path
        .strip_prefix(root)?
        .to_str()
        .context("Source filenames must be valid UTF-8; originals are unchanged")?
        .replace('\\', "/"))
}
pub fn read(root: &Path) -> Result<Files> {
    fn walk(root: &Path, path: &Path, files: &mut Files, total: &mut u64) -> Result<()> {
        let metadata = fs::symlink_metadata(path)?;
        ensure!(!metadata.is_symlink(), "Source symlinks are unsupported");
        if metadata.is_dir() {
            for entry in fs::read_dir(path)? {
                walk(root, &entry?.path(), files, total)?;
            }
        } else {
            ensure!(metadata.is_file(), "Unsupported source file");
            *total = total
                .checked_add(metadata.len())
                .context("Source size overflow")?;
            ensure!(
                *total <= 1024 * 1024 * 1024 && files.len() < 4096,
                "Source folder exceeds limits"
            );
            let name = portable_name(root, &path)?;
            files.insert(name, fs::read(path)?);
        }
        Ok(())
    }
    let root = if root.is_file() {
        root.parent().context("File has no parent")?
    } else {
        root
    };
    let mut files = Files::new();
    walk(root, root, &mut files, &mut 0)?;
    Ok(files)
}
pub fn write_new(root: &Path, files: &Files) -> Result<()> {
    crate::stepmania::bundle::validate_files(files)?;
    ensure!(!root.exists(), "Output already exists");
    let parent = root.parent().context("Output has no parent")?;
    fs::create_dir_all(parent)?;
    let stage = tempfile::tempdir_in(parent)?;
    for (name, bytes) in files {
        let path = stage.path().join(name);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, bytes)?;
    }
    fs::rename(stage.path(), root)?;
    Ok(())
}
pub fn save_document(root: &Path, project: &Project, expected: Option<u64>) -> Result<()> {
    use std::io::Write;
    fs::create_dir_all(root)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(".stepmania-lock"))?;
    lock.try_lock()
        .context("Project is being saved elsewhere")?;
    let destination = root.join(MANIFEST);
    let current = if destination.exists() {
        Some(Project::decode(&fs::read(&destination)?)?.revision)
    } else {
        None
    };
    ensure!(
        current == expected,
        "Project changed elsewhere; save a backup before reopening"
    );
    let mut stage = tempfile::NamedTempFile::new_in(root)?;
    stage.write_all(&project.encode()?)?;
    stage.as_file().sync_all()?;
    stage.persist(&destination)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::stepmania::{Profile, bundle};
    #[test]
    fn saves_are_revision_checked_and_do_not_rewrite_sources() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        let project = Project::blank(
            "Test".into(),
            "YACE".into(),
            String::new(),
            120.,
            0.,
            Profile::Pad,
        )
        .unwrap();
        write_new(&root, &bundle::backup(&project, &Files::new()).unwrap()).unwrap();
        assert!(save_document(&root, &project, None).is_err());
        assert!(save_document(&root, &project, Some(0)).is_ok());
        assert!(write_new(&root, &Files::new()).is_err());
        assert_eq!(
            Project::decode(&fs::read(root.join(MANIFEST)).unwrap())
                .unwrap()
                .revision,
            0
        );
    }
    #[cfg(unix)]
    #[test]
    fn source_symlinks_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("real"), b"data").unwrap();
        std::os::unix::fs::symlink(temp.path().join("real"), temp.path().join("link")).unwrap();
        assert!(read(temp.path()).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn source_names_cannot_be_lossily_renamed() {
        use std::os::unix::ffi::OsStringExt;
        // APFS rejects such names itself. Exercise the conversion boundary
        // directly so the Linux filename case remains covered on macOS too.
        let root = Path::new("source");
        let file = root.join(std::ffi::OsString::from_vec(b"art\xff.png".to_vec()));
        assert!(
            portable_name(root, &file)
                .unwrap_err()
                .to_string()
                .contains("UTF-8")
        );
    }
}
