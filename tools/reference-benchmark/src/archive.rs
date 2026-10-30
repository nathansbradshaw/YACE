// Rust adaptation of YARG.Core YARGSongFileStream.cs and SngHandler/*.cs.
// Copyright YARG contributors; LGPL-3.0-only. See NOTICE.md and LICENSE.
// This optional standalone process is not linked into the MIT YACE application.
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Component, Path},
};
const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
#[derive(Clone, Serialize)]
pub struct Entry {
    pub name: String,
    pub length: usize,
    pub position: usize,
}
pub struct Archive {
    pub bytes: Vec<u8>,
    pub keys: [u8; 16],
    pub metadata: BTreeMap<String, String>,
    pub entries: Vec<Entry>,
    pub sha256: String,
}
fn take<'a>(bytes: &'a [u8], p: &mut usize, n: usize) -> Result<&'a [u8]> {
    let end = p.checked_add(n).context("length overflow")?;
    ensure!(end <= bytes.len(), "truncated archive");
    let out = &bytes[*p..end];
    *p = end;
    Ok(out)
}
fn u64le(bytes: &[u8], p: &mut usize) -> Result<u64> {
    Ok(u64::from_le_bytes(take(bytes, p, 8)?.try_into()?))
}
fn usizele(bytes: &[u8], p: &mut usize) -> Result<usize> {
    Ok(usize::try_from(u64le(bytes, p)?)?)
}
fn string(bytes: &[u8], p: &mut usize) -> Result<String> {
    let n = u32::from_le_bytes(take(bytes, p, 4)?.try_into()?) as usize;
    ensure!(n <= 4 * 1024 * 1024, "metadata string too large");
    Ok(String::from_utf8(take(bytes, p, n)?.to_vec())?)
}
pub fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.contains(['/', '\\', ':'])
        && !name.chars().any(char::is_control)
        && Path::new(name)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        && Path::new(name).components().count() == 1
}
fn safe_metadata_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 255
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
pub fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
impl Archive {
    pub fn open(path: &Path) -> Result<Self> {
        let meta = std::fs::metadata(path)?;
        ensure!(
            meta.is_file() && meta.len() <= MAX_ARCHIVE,
            "archive exceeds512MiB limit"
        );
        let original = std::fs::read(path)?;
        ensure!(
            original.len() as u64 <= MAX_ARCHIVE,
            "archive grew beyond size limit"
        );
        let sha256 = sha(&original);
        let mut bytes = if original.starts_with(b"YARGSONG") {
            ensure!(original.len() >= 24, "truncated YARGSONG header");
            let z = i32::from(original[8]) + 1679;
            let w = (z ^ 4) - z * 2;
            let n = 25 * w - 5;
            let mut x = (w + (z << 1)) ^ 4;
            let l = (n + 73)
                .wrapping_mul(n + 23)
                .wrapping_sub(n.wrapping_mul(n).wrapping_add(96i32.wrapping_mul(n)));
            x = (-l).wrapping_add(n).wrapping_add(x).wrapping_sub(w * 25);
            x = (x + 5) % 255;
            ensure!(x >= 0, "invalid cipher parameters");
            let set = &original[9..24];
            let (mut a, mut b, mut c) = (0i32, 0i32, 0i32);
            for i in 0..24 {
                let j = i * x;
                let v = i32::from(set[(j % 15) as usize]);
                a = a.wrapping_add((v + i * 3298 + 88903) as u8 as i32);
                b = b.wrapping_sub(i32::from(set[((j + 7001) % 15) as usize]));
                c = c.wrapping_add(v);
            }
            original[24..]
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    (i32::from(*v)
                        .wrapping_sub((i as i32).wrapping_mul(c))
                        .wrapping_sub(b)
                        ^ a) as u8
                })
                .collect::<Vec<_>>()
        } else {
            original
        };
        ensure!(
            bytes.starts_with(b"SNGPKG") || bytes.starts_with(b"RB3CON"),
            "unsupported archive signature"
        );
        let mut p = 6;
        let version = u32::from_le_bytes(take(&bytes, &mut p, 4)?.try_into()?);
        ensure!(version == 1, "unsupported SNG version{version}");
        let keys: [u8; 16] = take(&bytes, &mut p, 16)?.try_into()?;
        let metadata_len = usizele(&bytes, &mut p)?;
        ensure!(metadata_len <= 16 * 1024 * 1024, "metadata too large");
        let metadata_bytes = take(&bytes, &mut p, metadata_len)?;
        let mut q = 0;
        let pairs = usizele(metadata_bytes, &mut q)?;
        ensure!(pairs <= 10000, "too many metadata pairs");
        let mut metadata = BTreeMap::new();
        for _ in 0..pairs {
            let k = string(metadata_bytes, &mut q)?;
            let v = string(metadata_bytes, &mut q)?;
            ensure!(safe_metadata_key(&k), "unsafe INI metadata key");
            ensure!(metadata.insert(k, v).is_none(), "duplicate metadata key");
        }
        ensure!(
            q == metadata_bytes.len(),
            "unexpected metadata trailing bytes"
        );
        let listings_len = usizele(&bytes, &mut p)?;
        ensure!(listings_len <= 16 * 1024 * 1024, "listings too large");
        let listings = take(&bytes, &mut p, listings_len)?;
        let mut q = 0;
        let count = usizele(listings, &mut q)?;
        ensure!(count <= 10000, "too many files");
        let mut entries = Vec::new();
        let mut names = std::collections::BTreeSet::new();
        for _ in 0..count {
            let n = take(listings, &mut q, 1)?[0] as usize;
            let name = String::from_utf8(take(listings, &mut q, n)?.to_vec())?;
            ensure!(safe_name(&name), "unsafe archive entry{name:?}");
            ensure!(names.insert(name.to_lowercase()), "duplicate archive path");
            let length = usizele(listings, &mut q)?;
            let position = usizele(listings, &mut q)?;
            ensure!(
                position
                    .checked_add(length)
                    .is_some_and(|e| e <= bytes.len()),
                "entry outside archive"
            );
            entries.push(Entry {
                name,
                length,
                position,
            });
        }
        ensure!(q == listings.len(), "unexpected listing bytes");
        let mut ranges = entries
            .iter()
            .map(|e| (e.position, e.position + e.length))
            .collect::<Vec<_>>();
        ranges.sort();
        ensure!(ranges.iter().all(|r| r.0 >= p), "entry points into header");
        ensure!(
            ranges.windows(2).all(|r| r[0].1 <= r[1].0),
            "overlapping entries"
        );
        // Owned bytes are immutable after parsing; all entry access is bounds checked.
        bytes.shrink_to_fit();
        Ok(Self {
            bytes,
            keys,
            metadata,
            entries,
            sha256,
        })
    }
    pub fn data(&self, entry: &Entry) -> Vec<u8> {
        self.bytes[entry.position..entry.position + entry.length]
            .iter()
            .enumerate()
            .map(|(i, v)| v ^ self.keys[i % 16] ^ (i as u8))
            .collect()
    }
    pub fn get(&self, name: &str) -> Option<Vec<u8>> {
        self.entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))
            .map(|e| self.data(e))
    }
    pub fn extract(&self, destination: &Path) -> Result<()> {
        if destination.exists() {
            bail!("destination already exists; refusing overwrite")
        }
        std::fs::create_dir_all(destination)?;
        for entry in &self.entries {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            use std::io::Write;
            options
                .open(destination.join(&entry.name))?
                .write_all(&self.data(entry))?;
        }
        let ini = std::iter::once("[song]".to_owned())
            .chain(
                self.metadata
                    .iter()
                    .map(|(k, v)| format!("{k} = {}", v.replace(['\r', '\n'], " "))),
            )
            .collect::<Vec<_>>()
            .join("\n");
        if !self
            .entries
            .iter()
            .any(|e| e.name.eq_ignore_ascii_case("song.ini"))
        {
            std::fs::write(destination.join("song.ini"), ini)?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_traversal() {
        for path in [
            "../notes.mid",
            "/notes.mid",
            "a/../b",
            "a\\b",
            "C:note",
            "notes.mid/",
            "notes\n.mid",
            ".",
            "",
        ] {
            assert!(!safe_name(path), "{path}");
        }
        assert!(safe_name("notes.mid"));
    }
    #[test]
    fn rejects_ini_metadata_injection() {
        for key in ["", "artist\n[song]", "a=b", "[song]", "artist\r"] {
            assert!(!safe_metadata_key(key));
        }
        assert!(safe_metadata_key("diff_bass_real"));
    }
}

#[cfg(test)]
mod archive_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    fn fixture(names: &[&str]) -> Fixture {
        let keys = std::array::from_fn::<_, 16, _>(|i| i as u8 * 3);
        let mut bytes = b"SNGPKG".to_vec();
        bytes.extend(1u32.to_le_bytes());
        bytes.extend(keys);
        let mut meta = 1u64.to_le_bytes().to_vec();
        for s in ["artist", "Reference fixture"] {
            meta.extend((s.len() as u32).to_le_bytes());
            meta.extend(s.as_bytes());
        }
        bytes.extend((meta.len() as u64).to_le_bytes());
        bytes.extend(meta);
        let listing_len = 8 + names.iter().map(|n| 1 + n.len() + 16).sum::<usize>();
        let start = bytes.len() + 8 + listing_len;
        let mut listings = (names.len() as u64).to_le_bytes().to_vec();
        let payload = (0..32u8).collect::<Vec<_>>();
        for (i, name) in names.iter().enumerate() {
            listings.push(name.len() as u8);
            listings.extend(name.as_bytes());
            listings.extend((payload.len() as u64).to_le_bytes());
            listings.extend(((start + i * payload.len()) as u64).to_le_bytes());
        }
        bytes.extend((listing_len as u64).to_le_bytes());
        bytes.extend(listings);
        for _ in names {
            bytes.extend(
                payload
                    .iter()
                    .enumerate()
                    .map(|(i, v)| v ^ keys[i % 16] ^ (i as u8)),
            );
        }
        let path = std::env::temp_dir().join(format!(
            "yace-reference-{}-{}.sng",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::write(&path, bytes).unwrap();
        Fixture(path)
    }
    #[test]
    fn decrypts_standard_sng_entries_with_file_local_mask() {
        let f = fixture(&["notes.mid", "song.opus"]);
        let a = Archive::open(&f.0).unwrap();
        assert_eq!(a.metadata["artist"], "Reference fixture");
        for name in ["notes.mid", "song.opus"] {
            assert_eq!(a.get(name).unwrap(), (0..32u8).collect::<Vec<_>>());
        }
    }
    #[test]
    fn rejects_duplicate_casefolded_paths() {
        let f = fixture(&["notes.mid", "NOTES.MID"]);
        assert!(Archive::open(&f.0).is_err());
    }
    #[test]
    fn rejects_unsafe_archive_path_before_extraction() {
        let f = fixture(&["../evil"]);
        assert!(Archive::open(&f.0).is_err());
    }
    #[test]
    fn rejects_truncated_payload() {
        let f = fixture(&["notes.mid"]);
        let mut data = std::fs::read(&f.0).unwrap();
        data.pop();
        std::fs::write(&f.0, data).unwrap();
        assert!(Archive::open(&f.0).is_err());
    }
    #[test]
    fn rejects_unsupported_version_before_extracting() {
        let f = fixture(&["notes.mid"]);
        let mut bytes = std::fs::read(&f.0).unwrap();
        bytes[6..10].copy_from_slice(&2u32.to_le_bytes());
        std::fs::write(&f.0, bytes).unwrap();
        assert!(Archive::open(&f.0).is_err());
    }
    #[test]
    fn rejects_unbounded_metadata_length_without_allocating_it() {
        let f = fixture(&["notes.mid"]);
        let mut bytes = std::fs::read(&f.0).unwrap();
        bytes[26..34].copy_from_slice(&u64::MAX.to_le_bytes());
        std::fs::write(&f.0, bytes).unwrap();
        assert!(Archive::open(&f.0).is_err());
    }
    #[test]
    fn rejects_entry_pointing_into_header() {
        let f = fixture(&["notes.mid"]);
        let mut bytes = std::fs::read(&f.0).unwrap();
        let metadata_length = u64::from_le_bytes(bytes[26..34].try_into().unwrap()) as usize;
        let entry_position = 34 + metadata_length + 8 + 8 + 1 + "notes.mid".len() + 8;
        bytes[entry_position..entry_position + 8].copy_from_slice(&0u64.to_le_bytes());
        std::fs::write(&f.0, bytes).unwrap();
        assert!(Archive::open(&f.0).is_err());
    }
    #[test]
    fn refuses_existing_output_without_writing() {
        let f = fixture(&["notes.mid"]);
        let a = Archive::open(&f.0).unwrap();
        let before = std::fs::read(&f.0).unwrap();
        assert!(a.extract(&f.0).is_err());
        assert_eq!(before, std::fs::read(&f.0).unwrap());
    }
}
