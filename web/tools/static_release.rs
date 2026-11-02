//! Immutable static releases keep workers in already-open tabs on their engine.
use crate::{Result, copy_source};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const ENTRY: &str = "import init from './yace_web.js';";
fn snapshot(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    fn visit(root: &Path, directory: &Path, output: &mut BTreeMap<PathBuf, Vec<u8>>) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                visit(root, &entry.path(), output)?;
            } else if kind.is_file() {
                output.insert(
                    entry.path().strip_prefix(root)?.to_owned(),
                    Sha256::digest(fs::read(entry.path())?).to_vec(),
                );
            } else {
                return Err("Static releases cannot contain symbolic links".into());
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files)?;
    Ok(files)
}
struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn stage(source: &Path, target: &Path, revision: &str) -> Result<PathBuf> {
    stage_bundle(source, target, revision, false)
}

/// Reuse only a byte-identical retained runtime; existing release URLs stay intact.
pub fn stage_reusing_runtime(source: &Path, target: &Path, revision: &str) -> Result<PathBuf> {
    stage_bundle(source, target, revision, true)
}

fn stage_bundle(
    source: &Path,
    target: &Path,
    revision: &str,
    reuse_runtime: bool,
) -> Result<PathBuf> {
    if revision.len() != 40
        || !revision
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
    {
        return Err("Use the full lowercase Git source revision for a static release".into());
    }
    let entry = fs::read_to_string(source.join("index.html"))?;
    if entry.matches(ENTRY).count() != 1 {
        return Err("Static entry point must import the generated application exactly once".into());
    }
    let original = snapshot(source)?;
    if original.get(Path::new("index.html")) != Some(&Sha256::digest(entry.as_bytes()).to_vec()) {
        return Err("Static entry changed during release staging".into());
    }
    if !original.contains_key(Path::new("yace_web.js"))
        || !original.contains_key(Path::new("yace_web_bg.wasm"))
        || !original.contains_key(Path::new("worker.js"))
    {
        return Err("Build the complete static application before staging a release".into());
    }
    for path in original.keys() {
        if fs::metadata(source.join(path))?.len() > 25 * 1024 * 1024 {
            return Err(format!("Static asset exceeds 25 MiB: {}", path.display()).into());
        }
    }
    let mut expected = original.clone();
    let rewritten = if reuse_runtime {
        let runtime = snapshot(&source.join("runtime"))?;
        if !runtime.contains_key(Path::new("ort.wasm.min.mjs"))
            || runtime != snapshot(&target.join("runtime"))?
        {
            return Err("The retained root runtime differs; use a complete release instead".into());
        }
        let module = fs::read_to_string(source.join("yace_web.js"))?;
        let import = "from './runtime/ort.wasm.min.mjs';";
        if module.matches(import).count() != 1 {
            return Err("Expected exactly one generated inference runtime import".into());
        }
        let module = module.replace(import, "from '../../runtime/ort.wasm.min.mjs';");
        expected.retain(|path, _| !path.starts_with("runtime"));
        expected.insert(
            PathBuf::from("yace_web.js"),
            Sha256::digest(module.as_bytes()).to_vec(),
        );
        Some(module)
    } else {
        None
    };
    fs::create_dir_all(target)?;
    let source_path = source.canonicalize()?;
    let target_path = target.canonicalize()?;
    if source_path.starts_with(&target_path) || target_path.starts_with(&source_path) {
        return Err("Stage releases in a separate hosting directory".into());
    }
    let releases = target.join("releases");
    fs::create_dir_all(&releases)?;
    let release = releases.join(revision);
    if release.exists() {
        if snapshot(&release)? != expected {
            return Err("A different artifact already uses this source revision; immutable releases cannot be overwritten".into());
        }
    } else {
        let temporary = target
            .parent()
            .ok_or("Hosting directory needs a parent")?
            .join(format!(
                ".yace-release-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
            ));
        fs::create_dir(&temporary)?;
        let staging = Staging(temporary);
        copy_source(source, &staging.0)?;
        if snapshot(&staging.0)? != original {
            return Err("Static files changed during release staging".into());
        }
        if let Some(module) = &rewritten {
            fs::write(staging.0.join("yace_web.js"), module)?;
            fs::remove_dir_all(staging.0.join("runtime"))?;
            if snapshot(&staging.0)? != expected
                || snapshot(&source.join("runtime"))? != snapshot(&target.join("runtime"))?
            {
                return Err("Static files changed while reusing the retained runtime".into());
            }
        }
        fs::rename(&staging.0, &release)?;
    }
    // Preserve root assets from the first flat deployment, as well as every
    // immutable release. Open tabs may still need their next worker or model.
    let index = entry.replace(
        ENTRY,
        &format!("import init from './releases/{revision}/yace_web.js';"),
    );
    let temporary = target.join(format!(".index-{}", std::process::id()));
    fs::write(&temporary, index)?;
    fs::rename(temporary, target.join("index.html"))?;
    fs::write(
        target.join("build-info.json"),
        format!(
            "{{\n  \"application\": \"YACE\",\n  \"platform\": \"static-dioxus-web-wasm\",\n  \"source_commit\": \"{revision}\",\n  \"asset_directory\": \"releases/{revision}\",\n  \"repository\": \"https://github.com/nathansbradshaw/YACE\"\n}}\n"
        ),
    )?;
    Ok(release)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bundle(path: &Path, marker: &str) {
        fs::create_dir_all(path.join("runtime")).unwrap();
        for (name, text) in [
            ("index.html", ENTRY),
            ("yace_web.js", marker),
            ("yace_web_bg.wasm", marker),
            ("worker.js", marker),
            ("runtime/engine.wasm", marker),
        ] {
            fs::write(path.join(name), text).unwrap();
        }
    }
    #[test]
    fn publishing_another_release_preserves_every_active_engine_and_legacy_root() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        let host = temp.path().join("host");
        bundle(&first, "first engine");
        bundle(&second, "second engine");
        bundle(&host, "legacy engine");
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        stage(&first, &host, &a).unwrap();
        let retained = snapshot(&host.join("releases").join(&a)).unwrap();
        stage(&second, &host, &b).unwrap();
        assert_eq!(snapshot(&host.join("releases").join(a)).unwrap(), retained);
        assert_eq!(fs::read(host.join("worker.js")).unwrap(), b"legacy engine");
        assert!(
            fs::read_to_string(host.join("index.html"))
                .unwrap()
                .contains(&format!("releases/{b}/yace_web.js"))
        );
        assert_eq!(
            fs::read(host.join("releases").join(b).join("runtime/engine.wasm")).unwrap(),
            b"second engine"
        );
    }
    #[test]
    fn restaging_is_idempotent_but_cannot_replace_a_published_engine() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let host = temp.path().join("host");
        let revision = "c".repeat(40);
        bundle(&source, "original");
        stage(&source, &host, &revision).unwrap();
        let before = snapshot(&host).unwrap();
        stage(&source, &host, &revision).unwrap();
        assert_eq!(snapshot(&host).unwrap(), before);
        fs::write(source.join("worker.js"), b"changed").unwrap();
        assert!(stage(&source, &host, &revision).is_err());
        assert_eq!(snapshot(&host).unwrap(), before);
    }
    #[test]
    fn invalid_revision_or_incomplete_bundle_keeps_the_host_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let host = temp.path().join("host");
        bundle(&source, "source");
        bundle(&host, "host");
        let before = snapshot(&host).unwrap();
        assert!(stage(&source, &host, "../bad").is_err());
        fs::remove_file(source.join("worker.js")).unwrap();
        assert!(stage(&source, &host, &"d".repeat(40)).is_err());
        assert_eq!(snapshot(&host).unwrap(), before);
    }

    #[test]
    fn compact_release_reuses_only_identical_runtime_without_changing_old_engines() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let host = temp.path().join("host");
        bundle(&source, "new engine");
        bundle(&host, "legacy engine");
        fs::write(
            source.join("yace_web.js"),
            "import { env } from './runtime/ort.wasm.min.mjs';",
        )
        .unwrap();
        fs::write(host.join("runtime/engine.wasm"), "new engine").unwrap();
        fs::write(source.join("runtime/ort.wasm.min.mjs"), "shared runtime").unwrap();
        fs::write(host.join("runtime/ort.wasm.min.mjs"), "shared runtime").unwrap();
        let retained = snapshot(&host.join("runtime")).unwrap();
        let revision = "e".repeat(40);
        let release = stage_reusing_runtime(&source, &host, &revision).unwrap();
        assert!(!release.join("runtime").exists());
        assert!(
            fs::read_to_string(release.join("yace_web.js"))
                .unwrap()
                .contains("../../runtime/ort.wasm.min.mjs")
        );
        assert_eq!(snapshot(&host.join("runtime")).unwrap(), retained);
        assert_eq!(fs::read(host.join("worker.js")).unwrap(), b"legacy engine");
        assert_eq!(
            fs::read(release.join("yace_web_bg.wasm")).unwrap(),
            b"new engine"
        );
        let before = snapshot(&host).unwrap();
        stage_reusing_runtime(&source, &host, &revision).unwrap();
        assert_eq!(snapshot(&host).unwrap(), before);
        fs::write(source.join("runtime/engine.wasm"), "changed runtime").unwrap();
        assert!(stage_reusing_runtime(&source, &host, &"f".repeat(40)).is_err());
        assert_eq!(snapshot(&host).unwrap(), before);
    }
}
