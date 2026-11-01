//! Bounded native validation proofs. Every reuse hashes current bytes and checks
//! file/tool identity; a timestamp alone never establishes unchanged audio.
use super::{SAMPLE_RATE, StemManifest, validate_stereo_format};
use crate::models;
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

const MAX_PROOFS: usize = 64;
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    length: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}
impl Stamp {
    fn read(path: &Path) -> Result<Self> {
        let meta = fs::metadata(path)?;
        ensure!(meta.is_file(), "Audio/tool must be a regular file");
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            length: meta.len(),
            modified: meta.modified().ok(),
            created: meta.created().ok(),
            #[cfg(unix)]
            identity: (meta.dev(), meta.ino(), meta.ctime(), meta.ctime_nsec()),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Tool {
    path: PathBuf,
    stamp: Stamp,
}
impl Tool {
    fn find(name: &str) -> Result<Self> {
        let path = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|dir| {
                dir.join(if cfg!(windows) {
                    format!("{name}.exe")
                } else {
                    name.to_owned()
                })
            })
            .find(|path| {
                let Ok(meta) = fs::metadata(path) else {
                    return false;
                };
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    meta.is_file() && meta.permissions().mode() & 0o111 != 0
                }
                #[cfg(not(unix))]
                {
                    meta.is_file()
                }
            })
            .with_context(|| format!("{name} is required on PATH"))?
            .canonicalize()?;
        Ok(Self {
            stamp: Stamp::read(&path)?,
            path,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    path: PathBuf,
    sha256: String,
    stamp: Stamp,
    decoder: Tool,
    probe: Option<Tool>,
}
impl Key {
    fn read(path: &Path, expected: &str, native: bool) -> Result<Self> {
        let path = path.canonicalize().context("Attached audio is missing")?;
        let stamp = Stamp::read(&path)?;
        let sha256 = models::hash_file(&path)?;
        ensure!(
            sha256.eq_ignore_ascii_case(expected),
            "Attached audio changed; checksum mismatch"
        );
        ensure!(Stamp::read(&path)? == stamp, "Audio changed while hashing");
        Ok(Self {
            path,
            sha256,
            stamp,
            decoder: Tool::find("ffmpeg")?,
            probe: native.then(|| Tool::find("ffprobe")).transpose()?,
        })
    }
}
#[derive(Clone)]
struct Proof {
    key: Key,
    frames: u64,
}
static PROOFS: OnceLock<Mutex<VecDeque<Proof>>> = OnceLock::new();

fn verified_with(
    path: &Path,
    expected: &str,
    native: bool,
    inspect: impl FnOnce() -> Result<u64>,
) -> Result<u64> {
    let key = Key::read(path, expected, native)?;
    let cache = PROOFS.get_or_init(|| Mutex::new(VecDeque::new()));
    if let Ok(mut proofs) = cache.lock()
        && let Some(index) = proofs.iter().position(|proof| proof.key == key)
    {
        let proof = proofs.remove(index).unwrap();
        let frames = proof.frames;
        proofs.push_back(proof);
        return Ok(frames);
    }
    // Never hold the proof lock during a decoder/probe or file I/O.
    let frames = inspect()?;
    ensure!(
        Key::read(path, expected, native)? == key,
        "Audio or decoder changed during validation"
    );
    if let Ok(mut proofs) = cache.lock() {
        proofs.retain(|proof| proof.key.path != key.path || proof.key.probe.is_some() != native);
        proofs.push_back(Proof { key, frames });
        while proofs.len() > MAX_PROOFS {
            proofs.pop_front();
        }
    }
    Ok(frames)
}

pub(super) fn frames(path: &Path, expected: &str, native: bool) -> Result<u64> {
    verified_with(path, expected, native, || {
        if native {
            validate_stereo_format(path)?;
        }
        let pcm = models::decode(path, SAMPLE_RATE, 2)?;
        ensure!(
            pcm.len().is_multiple_of(2),
            "Decoded stereo frame is incomplete"
        );
        Ok((pcm.len() / 2) as u64)
    })
}
pub(super) fn validate(manifest: &StemManifest, source: &Path) -> Result<()> {
    let source_path = source.canonicalize()?;
    ensure!(
        frames(source, &manifest.source_audio_sha256, false)? == manifest.frames,
        "Manifest frame count does not match decoded source audio"
    );
    let mut resolved = BTreeSet::new();
    for asset in &manifest.stems {
        let path = asset
            .path
            .canonicalize()
            .context("Attached stem is missing")?;
        ensure!(
            path != source_path && resolved.insert(path.clone()),
            "Stem files must be distinct from the original and one another"
        );
        ensure!(
            frames(&path, &asset.sha256, true)? == manifest.frames,
            "{} stem must have exactly the source's decoded frame count",
            asset.id.as_str()
        );
    }
    ensure!(
        models::hash_file(source)?.eq_ignore_ascii_case(&manifest.source_audio_sha256),
        "Source changed while validating stems"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    #[test]
    fn proofs_require_fresh_hashes_and_file_identity_and_never_cache_failures() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("audio");
        fs::write(&file, b"original").unwrap();
        let expected = models::hash_file(&file).unwrap();
        let calls = Cell::new(0);
        let inspect = || {
            calls.set(calls.get() + 1);
            Ok(100)
        };
        assert_eq!(
            verified_with(&file, &expected, false, inspect).unwrap(),
            100
        );
        assert_eq!(
            verified_with(&file, &expected, false, inspect).unwrap(),
            100
        );
        assert_eq!(calls.get(), 1);
        let modified = fs::metadata(&file).unwrap().modified().unwrap();
        fs::write(&file, b"tampered").unwrap();
        fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert!(verified_with(&file, &expected, false, inspect).is_err());
        let replacement = root.path().join("replacement");
        fs::write(&replacement, b"original").unwrap();
        fs::rename(replacement, &file).unwrap();
        assert_eq!(
            verified_with(&file, &expected, false, inspect).unwrap(),
            100
        );
        assert_eq!(calls.get(), 2);
        assert!(
            verified_with(&file, &expected, true, || anyhow::bail!(
                "invalid native format"
            ))
            .is_err()
        );
        assert!(
            verified_with(&file, &expected, true, || anyhow::bail!(
                "invalid native format"
            ))
            .is_err()
        );
    }
}
