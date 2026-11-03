//! Portable backups deduplicate bytes and offer listening or exact stem copies.
//! The manifest describes original paths, lengths and hashes, never edits them.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Seek, Write},
    path::Path,
};

pub const MANIFEST: &str = "yace-backup.json";
pub const MAX_TOTAL: u64 = 32 * 1024 * 1024 * 1024;
pub const MAX_FILE: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_FILES: usize = 16384;
const BLOCK: usize = 4096;
#[path = "compact_backup/listening.rs"]
mod listening;
pub const TRANSCODES: &str = "yace-portable-audio.json";
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StemQuality {
    Listening,
    #[default]
    Exact,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Options {
    pub include_generated_stems: bool,
    #[serde(default)]
    pub stem_quality: StemQuality,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            include_generated_stems: true,
            stem_quality: StemQuality::Exact,
        }
    }
}
#[derive(Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub generated_stems_included: bool,
    pub files: Vec<Entry>,
    #[serde(default)]
    pub transcodes: BTreeMap<String, String>,
}
#[derive(Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub blob: String,
    pub bytes: u64,
    pub sha256: String,
    pub float: Option<FloatLayout>,
    #[serde(default)]
    pub listening: Option<listening::Listening>,
    #[serde(default)]
    pub restored_sha256: Option<String>,
    #[serde(default)]
    pub packed_bytes: Option<u64>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FloatLayout {
    pub offset: usize,
    pub bytes: usize,
    pub channels: usize,
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn generated(path: &str) -> bool {
    let path = format!("/{path}");
    path.contains("/.yace/audio/stems/")
        || path.contains("/.yace/audio/keyboard-stems/")
        || path.contains("/.yace/audio/conventional/")
        || path.starts_with("/assets/stems/")
        || path.starts_with("/stems/")
        || path.starts_with("/keyboard-stems/")
        || (path.starts_with("/analysis/kit-")
            && path.contains("/parts/")
            && path.ends_with(".wav"))
}
pub fn safe_path(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['\\', '\0', ':'])
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && Path::new(name)
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}
fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}
pub fn float_layout(bytes: &[u8]) -> Option<FloatLayout> {
    let rf64 = bytes.get(..4)? == b"RF64";
    if (!rf64 && bytes.get(..4)? != b"RIFF") || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut channels = None;
    let mut large_data = None;
    let mut at = 12usize;
    while at.checked_add(8)? <= bytes.len() {
        let declared = u32_at(bytes, at + 4)?;
        let len = if rf64 && &bytes[at..at + 4] == b"data" && declared == u32::MAX {
            usize::try_from(large_data?).ok()?
        } else {
            declared as usize
        };
        let start = at + 8;
        let end = start.checked_add(len)?;
        if end > bytes.len() {
            return None;
        }
        match &bytes[at..at + 4] {
            b"ds64" if rf64 && len >= 28 => {
                large_data = Some(u64::from_le_bytes(
                    bytes[start + 8..start + 16].try_into().ok()?,
                ));
            }
            b"fmt " if len >= 16 => {
                let format = u16::from_le_bytes(bytes[start..start + 2].try_into().ok()?);
                let count =
                    u16::from_le_bytes(bytes[start + 2..start + 4].try_into().ok()?) as usize;
                let bits = u16::from_le_bytes(bytes[start + 14..start + 16].try_into().ok()?);
                let float_format = format == 3
                    || (format == 0xfffe
                        && len >= 40
                        && bytes[start + 24..start + 40]
                            == [3, 0, 0, 0, 0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113]);
                if !float_format || bits != 32 || !(1..=8).contains(&count) {
                    return None;
                }
                channels = Some(count);
            }
            b"data" => {
                let channels = channels?;
                if !len.is_multiple_of(channels * 4) {
                    return None;
                }
                return Some(FloatLayout {
                    offset: start,
                    bytes: len,
                    channels,
                });
            }
            _ => {}
        }
        at = end.checked_add(len % 2)?;
    }
    None
}
/// XOR prediction and byte planes improve general compression without rounding,
/// clipping, changing headers or changing even NaN/negative-zero bit patterns.
fn write_float<W: Write>(writer: &mut W, bytes: &[u8], layout: FloatLayout) -> Result<()> {
    writer.write_all(&bytes[..layout.offset])?;
    let mut buffer = vec![0u8; BLOCK * 4];
    for block in bytes[layout.offset..layout.offset + layout.bytes].chunks(BLOCK * 4) {
        let count = block.len() / 4;
        let mut previous = [0u32; 8];
        for (i, sample) in block.chunks_exact(4).enumerate() {
            let value = u32::from_le_bytes(sample.try_into()?);
            let channel = i % layout.channels;
            let delta = (value ^ previous[channel]).to_le_bytes();
            previous[channel] = value;
            for plane in 0..4 {
                buffer[plane * count + i] = delta[plane];
            }
        }
        writer.write_all(&buffer[..block.len()])?;
    }
    writer.write_all(&bytes[layout.offset + layout.bytes..])?;
    Ok(())
}
pub fn restore_float(bytes: &mut [u8], layout: FloatLayout) -> Result<()> {
    ensure!(
        (1..=8).contains(&layout.channels)
            && layout.bytes.is_multiple_of(layout.channels * 4)
            && layout
                .offset
                .checked_add(layout.bytes)
                .is_some_and(|end| end <= bytes.len()),
        "Invalid packed WAV layout"
    );
    let mut buffer = vec![0u8; BLOCK * 4];
    for block in bytes[layout.offset..layout.offset + layout.bytes].chunks_mut(BLOCK * 4) {
        buffer[..block.len()].copy_from_slice(block);
        let count = block.len() / 4;
        let mut previous = [0u32; 8];
        for (i, sample) in block.chunks_exact_mut(4).enumerate() {
            let delta = u32::from_le_bytes(std::array::from_fn(|p| buffer[p * count + i]));
            let channel = i % layout.channels;
            previous[channel] ^= delta;
            sample.copy_from_slice(&previous[channel].to_le_bytes());
        }
    }
    ensure!(
        float_layout(bytes).is_some_and(|actual| actual.offset == layout.offset
            && actual.bytes == layout.bytes
            && actual.channels == layout.channels),
        "Packed WAV header differs from its manifest"
    );
    Ok(())
}
pub fn encode_into<'a, W: Write + Seek>(
    files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    writer: W,
    options: Options,
) -> Result<W> {
    let files = files
        .into_iter()
        .map(|(path, bytes)| (path, bytes, digest(bytes)))
        .collect::<Vec<_>>();
    // Never quantize an asset also used as a source recording or authored file.
    let protected = files
        .iter()
        .filter(|(path, _, _)| !generated(path))
        .map(|(_, _, hash)| hash.clone())
        .collect::<BTreeSet<_>>();
    let mut zip = zip::ZipWriter::new(writer);
    let mut manifest = Manifest {
        version: 1,
        generated_stems_included: options.include_generated_stems,
        files: Vec::new(),
        transcodes: BTreeMap::new(),
    };
    let mut names = BTreeSet::new();
    let mut blobs = BTreeSet::new();
    let mut total = 0u64;
    for (path, bytes, sha256) in files {
        ensure!(
            safe_path(path)
                && path != MANIFEST
                && names.insert(path.to_owned())
                && names.len() <= MAX_FILES,
            "Invalid or duplicate backup path"
        );
        if !options.include_generated_stems && generated(path) {
            continue;
        }
        if path == TRANSCODES {
            let previous: BTreeMap<String, String> = serde_json::from_slice(bytes)?;
            manifest.transcodes.extend(previous);
            continue;
        }
        total = total
            .checked_add(bytes.len() as u64)
            .context("Backup size overflow")?;
        ensure!(
            total <= MAX_TOTAL && bytes.len() as u64 <= MAX_FILE,
            "Backup exceeds supported size"
        );
        let blob = format!("yace-assets/{sha256}");
        // Identical files always choose the same representation, regardless of path.
        let float = float_layout(bytes);
        let copy = if options.stem_quality == StemQuality::Listening && !protected.contains(&sha256)
        {
            float.and_then(|layout| listening::prepare(bytes, layout))
        } else {
            None
        };
        let listening = copy.as_ref().map(|(copy, _, _)| *copy);
        let restored_sha256 = copy.as_ref().map(|(_, hash, _)| hash.clone());
        let packed_bytes = copy.as_ref().map(|(_, _, size)| *size);
        if let Some(hash) = &restored_sha256
            && hash != &sha256
        {
            manifest.transcodes.insert(sha256.clone(), hash.clone());
        }
        if blobs.insert(blob.clone()) {
            zip.start_file(
                &blob,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
                    .compression_level(Some(6)),
            )?;
            if let Some(copy) = listening {
                listening::write(&mut zip, bytes, copy)?;
            } else if let Some(layout) = float {
                write_float(&mut zip, bytes, layout)?;
            } else {
                zip.write_all(bytes)?;
            }
        }
        manifest.files.push(Entry {
            path: path.into(),
            blob,
            bytes: bytes.len() as u64,
            sha256,
            float,
            listening,
            restored_sha256,
            packed_bytes,
        });
    }
    ensure!(!manifest.files.is_empty(), "No project files to back up");
    zip.start_file(
        MANIFEST,
        zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated),
    )?;
    serde_json::to_writer(&mut zip, &manifest)?;
    Ok(zip.finish()?)
}
pub fn manifest<R: Read + Seek>(zip: &mut zip::ZipArchive<R>) -> Result<Option<Manifest>> {
    if !zip
        .file_names()
        .any(|name| name.is_ok_and(|name| name == MANIFEST))
    {
        return Ok(None);
    }
    let mut file = zip.by_name(MANIFEST)?;
    ensure!(
        file.size() <= 16 * 1024 * 1024,
        "Backup manifest is too large"
    );
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    ensure!(
        manifest.version == 1 && !manifest.files.is_empty() && manifest.files.len() <= MAX_FILES,
        "Unsupported backup version or entry count"
    );
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    let mut blobs = BTreeMap::new();
    for entry in &manifest.files {
        ensure!(
            safe_path(&entry.path)
                && names.insert(&entry.path)
                && entry.sha256.len() == 64
                && entry.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                && entry.blob == format!("yace-assets/{}", entry.sha256),
            "Invalid backup manifest path or hash"
        );
        total = total
            .checked_add(entry.bytes)
            .context("Backup size overflow")?;
        ensure!(
            entry.bytes <= MAX_FILE && total <= MAX_TOTAL,
            "Expanded backup exceeds supported size"
        );
        ensure!(
            entry
                .restored_sha256
                .as_ref()
                .is_none_or(|hash| valid_hash(hash)),
            "Invalid restored audio checksum"
        );
        ensure!(
            entry.listening.is_some() == entry.restored_sha256.is_some()
                && entry.listening.is_some() == entry.packed_bytes.is_some(),
            "Incomplete listening-stem description"
        );
        if let Some(copy) = entry.listening {
            ensure!(
                Some(copy.layout) == entry.float,
                "Conflicting listening-stem WAV layouts"
            );
            if entry.restored_sha256.as_ref() != Some(&entry.sha256) {
                ensure!(
                    manifest.transcodes.get(&entry.sha256) == entry.restored_sha256.as_ref(),
                    "Missing listening-stem provenance map"
                );
            }
        }
        let identity = serde_json::to_vec(&(
            entry.bytes,
            entry.float,
            entry.listening,
            entry.restored_sha256.as_ref(),
            entry.packed_bytes,
        ))?;
        if let Some(previous) = blobs.insert(&entry.blob, identity.clone()) {
            ensure!(
                previous == identity,
                "Conflicting backup asset descriptions"
            );
        }
    }
    ensure!(
        !names.contains(&TRANSCODES.to_owned())
            && manifest.transcodes.len() <= MAX_FILES
            && manifest
                .transcodes
                .iter()
                .all(|(old, new)| valid_hash(old) && valid_hash(new) && old != new),
        "Invalid portable audio identity map"
    );
    Ok(Some(manifest))
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}
pub fn restore_entry(bytes: Vec<u8>, entry: &Entry) -> Result<Vec<u8>> {
    if let Some(copy) = entry.listening {
        listening::restore(bytes, copy, entry.bytes)
    } else {
        let mut bytes = bytes;
        if let Some(layout) = entry.float {
            restore_float(&mut bytes, layout)?;
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    pub(super) fn wave() -> Vec<u8> {
        let mut out=b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x03\0\x02\0\x44\xac\0\0\x20\x62\x05\0\x08\0\x20\0data\0\0\0\0".to_vec();
        for i in 0..20000 {
            for value in [(i as f32 * 0.03).sin(), (i as f32 * 0.05).cos()] {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        for bits in [0x80000000u32, 0x7fc00001, 0x7f800000, 0xff800000] {
            out.extend_from_slice(&bits.to_le_bytes());
        }
        let length = out.len() as u32;
        out[4..8].copy_from_slice(&(length - 8).to_le_bytes());
        out[40..44].copy_from_slice(&(length - 44).to_le_bytes());
        out
    }
    #[test]
    fn float_packing_is_bit_exact_across_blocks_and_special_values() {
        let original = wave();
        let layout = float_layout(&original).unwrap();
        let mut packed = Vec::new();
        write_float(&mut packed, &original, layout).unwrap();
        assert_ne!(packed, original);
        restore_float(&mut packed, layout).unwrap();
        assert_eq!(packed, original);
        assert!(
            restore_float(
                &mut packed,
                FloatLayout {
                    bytes: usize::MAX,
                    ..layout
                }
            )
            .is_err()
        );
    }
    #[test]
    fn streamed_rf64_stems_keep_their_complete_header_and_sample_bits() {
        let original = wave();
        let data = original.len() - 44;
        let mut rf = b"RF64\xff\xff\xff\xffWAVEds64".to_vec();
        rf.extend_from_slice(&28u32.to_le_bytes());
        rf.extend_from_slice(&(original.len() as u64 + 28).to_le_bytes());
        rf.extend_from_slice(&(data as u64).to_le_bytes());
        rf.extend_from_slice(&((data / 8) as u64).to_le_bytes());
        rf.extend_from_slice(&0u32.to_le_bytes());
        rf.extend_from_slice(&original[12..]);
        rf[76..80].copy_from_slice(&u32::MAX.to_le_bytes());
        let layout = float_layout(&rf).unwrap();
        assert_eq!(layout.offset, 80);
        assert_eq!(layout.bytes, data);
        let mut packed = Vec::new();
        write_float(&mut packed, &rf, layout).unwrap();
        restore_float(&mut packed, layout).unwrap();
        assert_eq!(packed, rf);
    }
    #[test]
    fn manifest_stores_repeated_audio_once_and_keeps_history() {
        let audio = wave();
        let files = [
            ("source/song.wav", audio.as_slice()),
            ("dance/source/song.wav", audio.as_slice()),
            ("checkpoints/history.json", b"history".as_slice()),
        ];
        let bytes = encode_into(
            files,
            Cursor::new(Vec::new()),
            Options {
                stem_quality: StemQuality::Exact,
                ..Default::default()
            },
        )
        .unwrap()
        .into_inner();
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let m = manifest(&mut zip).unwrap().unwrap();
        assert_eq!(m.files.len(), 3);
        assert_eq!(zip.len(), 3);
        let first = &m.files[0];
        let mut packed = Vec::new();
        zip.by_name(&first.blob)
            .unwrap()
            .read_to_end(&mut packed)
            .unwrap();
        restore_float(&mut packed, first.float.unwrap()).unwrap();
        assert_eq!(packed, audio);
    }
    #[test]
    fn omit_stems_preserves_recording_and_checkpoints() {
        let files = [
            ("source/song.ogg", b"recording".as_slice()),
            (
                "source/.yace/audio/stems/key/guitar.wav",
                b"generated".as_slice(),
            ),
            ("analysis/kit-one/parts/kick.wav", b"kit".as_slice()),
            ("checkpoints/state.json", b"history".as_slice()),
            ("source/extras/take.wav", b"authored".as_slice()),
        ];
        let bytes = encode_into(
            files,
            Cursor::new(Vec::new()),
            Options {
                include_generated_stems: false,
                ..Default::default()
            },
        )
        .unwrap()
        .into_inner();
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let m = manifest(&mut zip).unwrap().unwrap();
        assert_eq!(
            m.files.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
            vec![
                "source/song.ogg",
                "checkpoints/state.json",
                "source/extras/take.wav"
            ]
        );
    }
    #[test]
    fn real_stem_compression_and_exact_restore_when_requested() {
        let Some(path) = std::env::var_os("YACE_BACKUP_MEASURE") else {
            return;
        };
        let mut source = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let mut old = 0u64;
        let mut new = 0usize;
        let mut count = 0;
        let quality = if std::env::var_os("YACE_BACKUP_LISTENING").is_some() {
            StemQuality::Listening
        } else {
            StemQuality::Exact
        };
        let mut min_snr = f64::INFINITY;
        for i in 0..source.len() {
            let mut entry = source.by_index(i).unwrap();
            if !generated(&entry.name().unwrap()) || !entry.name().unwrap().ends_with(".wav") {
                continue;
            }
            let name = entry.name().unwrap().into_owned();
            old += entry.compressed_size();
            let mut original = Vec::new();
            entry.read_to_end(&mut original).unwrap();
            let packed = encode_into(
                [(name.as_str(), original.as_slice())],
                Cursor::new(Vec::new()),
                Options {
                    stem_quality: quality,
                    ..Default::default()
                },
            )
            .unwrap()
            .into_inner();
            new += packed.len();
            let mut zip = zip::ZipArchive::new(Cursor::new(packed)).unwrap();
            let m = manifest(&mut zip).unwrap().unwrap();
            let mut restored = Vec::new();
            zip.by_name(&m.files[0].blob)
                .unwrap()
                .read_to_end(&mut restored)
                .unwrap();
            let restored = restore_entry(restored, &m.files[0]).unwrap();
            assert_eq!(
                digest(&restored),
                *m.files[0]
                    .restored_sha256
                    .as_ref()
                    .unwrap_or(&m.files[0].sha256)
            );
            if quality == StemQuality::Exact {
                assert_eq!(digest(&original), digest(&restored));
            } else {
                let layout = m.files[0].float.unwrap();
                let mut signal = 0f64;
                let mut noise = 0f64;
                for (a, b) in original[layout.offset..layout.offset + layout.bytes]
                    .chunks_exact(4)
                    .zip(restored[layout.offset..layout.offset + layout.bytes].chunks_exact(4))
                {
                    let a = f64::from(f32::from_le_bytes(a.try_into().unwrap()));
                    let b = f64::from(f32::from_le_bytes(b.try_into().unwrap()));
                    signal += a * a;
                    noise += (a - b).powi(2);
                }
                if noise > 0. {
                    min_snr = min_snr.min(10. * (signal / noise).log10());
                }
            }
            count += 1;
        }
        eprintln!(
            "{count} real stems: previous ZIP {old} bytes, compact {new} bytes; mode {quality:?}, minimum SNR {min_snr:.1} dB"
        );
        assert!(count > 0 && (new as u64) < old);
        assert!(min_snr > 115.);
    }
}
