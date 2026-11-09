//! Source-pinned runtime for the private macOS vocal helper pack.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
pub const VERSION: &str = "1.24.4";
pub const FILENAME: &str = "libonnxruntime.1.24.4.dylib";
pub const SOURCE_SHA256: &str = "f958c8c8f50a787f64cd13aa5f07710b97075d9869c6983dc0001d2452100eaa";
const LICENSE_SHA256: &str = "2f07c72751aed99790b8a4869cf2311df85a860b22ded05fa22803587a48922c";
const NOTICES_SHA256: &str = "0e07b95f3a8d6230037707c5c4a2b554d12c4cb67369669ac255635528ffcee2";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub version: u32,
    pub runtime_version: String,
    pub platform: String,
    pub filename: String,
    pub source_sha256: String,
    pub packaged_sha256: String,
    pub license_sha256: String,
    pub notices_sha256: String,
}
#[derive(Debug)]
pub struct Runtime {
    pub path: PathBuf,
    pub sha256: String,
    pub source_sha256: String,
    pub version: String,
}
pub fn command(helper: impl AsRef<Path>, runtime: Option<&Runtime>) -> Command {
    let mut command = Command::new(helper.as_ref());
    if let Some(runtime) = runtime {
        command.env("ORT_DYLIB_PATH", &runtime.path);
    }
    command
}
/// A packaged app always uses its sealed runtime, even if a shell set another.
/// Standalone research may explicitly supply a runtime without changing globals.
pub fn resolve(tools: &Path, explicit: Option<OsString>) -> Result<Option<Runtime>> {
    let contents = tools
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "MacOS"))
        .and_then(Path::parent)
        .filter(|p| p.file_name().is_some_and(|n| n == "Contents"));
    if let Some(contents) = contents {
        let resources = contents.join("Resources");
        let receipt: Receipt = serde_json::from_slice(&fs::read(resources.join("vocal-runtime.json"))
            .context("Pinned vocal runtime is missing; install the matching desktop vocal pack. Another analysis mode was not substituted")?)?;
        ensure!(
            receipt.version == 1
                && receipt.runtime_version == VERSION
                && receipt.platform == "macos-arm64"
                && receipt.filename == FILENAME
                && receipt.source_sha256 == SOURCE_SHA256
                && receipt.license_sha256 == LICENSE_SHA256
                && receipt.notices_sha256 == NOTICES_SHA256,
            "Packaged vocal runtime identity differs from its pinned recipe"
        );
        let path = contents.join("Frameworks").join(FILENAME);
        ensure!(
            crate::models::hash_file(&path)? == receipt.packaged_sha256,
            "Packaged vocal runtime checksum differs; another quality mode was not substituted"
        );
        let notices = resources.join("third-party/vocal-runtime");
        ensure!(
            crate::models::hash_file(&notices.join("LICENSE"))? == LICENSE_SHA256
                && crate::models::hash_file(&notices.join("ThirdPartyNotices.txt"))?
                    == NOTICES_SHA256,
            "Packaged vocal runtime notices differ"
        );
        return Ok(Some(Runtime {
            path: path.canonicalize()?,
            sha256: receipt.packaged_sha256,
            source_sha256: receipt.source_sha256,
            version: receipt.runtime_version,
        }));
    }
    explicit
        .map(|path| {
            let path = PathBuf::from(path)
                .canonicalize()
                .context("Explicit ONNX runtime path is unavailable")?;
            let sha256 = crate::models::hash_file(&path)?;
            Ok(Runtime {
                sha256: sha256.clone(),
                source_sha256: sha256,
                path,
                version: "explicit research runtime".into(),
            })
        })
        .transpose()
}
/// The local publisher package is read-only; code signing changes only the copy.
pub fn package(source: &Path, contents: &Path, signing_identity: &str) -> Result<Receipt> {
    let library = source.join("capi").join(FILENAME);
    ensure!(
        crate::models::hash_file(&library)? == SOURCE_SHA256,
        "Vocal runtime must be the pinned ONNX Runtime 1.24.4 macOS arm64 library"
    );
    ensure!(
        cfg!(all(target_os = "macos", target_arch = "aarch64")),
        "The packaged vocal runtime currently supports macOS arm64; another runtime was not substituted"
    );
    for (file, sha) in [
        ("LICENSE", LICENSE_SHA256),
        ("ThirdPartyNotices.txt", NOTICES_SHA256),
    ] {
        ensure!(
            crate::models::hash_file(&source.join(file))? == sha,
            "Vocal runtime notice differs: {file}"
        );
    }
    let frameworks = contents.join("Frameworks");
    fs::create_dir_all(&frameworks)?;
    let target = frameworks.join(FILENAME);
    fs::copy(library, &target)?;
    let status = Command::new("codesign")
        .args(["--force", "--sign", signing_identity])
        .arg(&target)
        .output()?;
    ensure!(
        status.status.success(),
        "Vocal runtime signing failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let notices = contents.join("Resources/third-party/vocal-runtime");
    fs::create_dir_all(&notices)?;
    for file in ["LICENSE", "ThirdPartyNotices.txt"] {
        fs::copy(source.join(file), notices.join(file))?;
    }
    let receipt = Receipt {
        version: 1,
        runtime_version: VERSION.into(),
        platform: "macos-arm64".into(),
        filename: FILENAME.into(),
        source_sha256: SOURCE_SHA256.into(),
        packaged_sha256: crate::models::hash_file(&target)?,
        license_sha256: LICENSE_SHA256.into(),
        notices_sha256: NOTICES_SHA256.into(),
    };
    crate::generation::write_json(&contents.join("Resources/vocal-runtime.json"), &receipt)?;
    Ok(receipt)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packaged_runtime_never_falls_back_to_a_shell_setting() {
        let root = tempfile::tempdir().unwrap();
        let tools = root.path().join("YACE.app/Contents/MacOS/vocal-tools");
        fs::create_dir_all(&tools).unwrap();
        let external = root.path().join("external.dylib");
        fs::write(&external, b"research library").unwrap();
        assert!(
            resolve(&tools, Some(external.into_os_string()))
                .unwrap_err()
                .to_string()
                .contains("Pinned vocal runtime is missing")
        );
    }
    #[test]
    fn explicit_runtime_is_scoped_to_each_child() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runtime");
        fs::write(&path, b"fixture").unwrap();
        let runtime = resolve(root.path(), Some(path.clone().into_os_string()))
            .unwrap()
            .unwrap();
        let child = command("helper", Some(&runtime));
        assert_eq!(
            child
                .get_envs()
                .find(|(k, _)| *k == "ORT_DYLIB_PATH")
                .unwrap()
                .1,
            Some(path.canonicalize().unwrap().as_os_str())
        );
        assert!(command("helper", None).get_envs().next().is_none());
    }
    #[test]
    fn foreign_runtime_cannot_be_packaged_or_change_the_source() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("capi")).unwrap();
        let path = root.path().join("capi").join(FILENAME);
        fs::write(&path, b"wrong runtime").unwrap();
        assert!(package(root.path(), &root.path().join("Contents"), "-").is_err());
        assert_eq!(fs::read(path).unwrap(), b"wrong runtime");
        assert!(!root.path().join("Contents").exists());
    }

    #[test]
    fn a_replaced_packaged_library_fails_before_a_worker_starts() {
        let root = tempfile::tempdir().unwrap();
        let contents = root.path().join("YACE.app/Contents");
        let tools = contents.join("MacOS/vocal-tools");
        fs::create_dir_all(&tools).unwrap();
        fs::create_dir_all(contents.join("Resources")).unwrap();
        fs::create_dir_all(contents.join("Frameworks")).unwrap();
        let receipt = Receipt {
            version: 1,
            runtime_version: VERSION.into(),
            platform: "macos-arm64".into(),
            filename: FILENAME.into(),
            source_sha256: SOURCE_SHA256.into(),
            packaged_sha256: "0".repeat(64),
            license_sha256: LICENSE_SHA256.into(),
            notices_sha256: NOTICES_SHA256.into(),
        };
        fs::write(
            contents.join("Resources/vocal-runtime.json"),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        fs::write(contents.join("Frameworks").join(FILENAME), b"replacement").unwrap();
        assert!(
            resolve(&tools, None)
                .unwrap_err()
                .to_string()
                .contains("checksum differs")
        );
    }
}
