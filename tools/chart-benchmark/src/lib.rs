//! MIT. Standard extracted files only; no dependency on the optional LGPL decoder.
pub mod evaluate;
pub mod paired;
pub mod recoveries;
use anyhow::{Context, Result, ensure};
use chart_engine::{
    benchmark::{self, Instrument},
    models::hash_file,
    stem_review::{AlignmentEvidence, StemAsset, StemId, StemManifest},
};
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};
const RATE: u64 = 44100;
const MAX_FRAMES: u64 = RATE * 3600;
#[derive(Deserialize)]
struct Asset {
    name: String,
    bytes: u64,
    sha256: String,
}
#[derive(Deserialize)]
struct Reference {
    version: u32,
    assets: Vec<Asset>,
    metadata: BTreeMap<String, String>,
    source_archive_sha256: String,
    song_ini_sha256: String,
}
#[derive(Deserialize)]
struct Included {
    stem: String,
    sha256: String,
    frames: u64,
    starts_at_source_frame: u64,
}
#[derive(Deserialize)]
struct Mix {
    version: u32,
    output_sha256: String,
    source_archive_sha256: String,
    sample_rate: u64,
    channels: u16,
    frames: u64,
    duration_seconds: f64,
    included_stems: Vec<Included>,
    ini_delay_milliseconds: Option<f64>,
}
fn hash_valid(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
/// A batch hash alone cannot establish which archive belongs at a case index.
pub fn validate_frozen_case(
    frozen: &serde_json::Value,
    index: usize,
    archive_sha256: &str,
) -> Result<()> {
    let expected = frozen["songs"]
        .as_array()
        .and_then(|songs| songs.get(index))
        .and_then(|song| song["archive_sha256"].as_str())
        .context("Frozen case archive identity missing")?;
    ensure!(
        hash_valid(expected)
            && hash_valid(archive_sha256)
            && expected.eq_ignore_ascii_case(archive_sha256),
        "Prepared case archive differs from frozen selection at index {}",
        index + 1
    );
    Ok(())
}
fn json_file<T: serde::de::DeserializeOwned>(p: &Path) -> Result<T> {
    let mut bytes = vec![];
    fs::File::open(p)?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 16 * 1024 * 1024, "Receipt exceeds16MiB");
    Ok(serde_json::from_slice(&bytes)?)
}
fn flat(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && Path::new(s).components().count() == 1
        && matches!(Path::new(s).components().next(), Some(Component::Normal(_)))
        && !s.contains(['\\', '/'])
}
fn ordinary(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    ensure!(
        m.is_file() && !m.file_type().is_symlink(),
        "Ordinary non-symlink file required: {}",
        path.display()
    );
    Ok(())
}
fn verify_asset(case: &Path, a: &Asset) -> Result<PathBuf> {
    ensure!(
        flat(&a.name) && hash_valid(&a.sha256),
        "Invalid asset identity"
    );
    let path = case.join(&a.name);
    ordinary(&path)?;
    ensure!(
        fs::metadata(&path)?.len() == a.bytes && hash_file(&path)?.eq_ignore_ascii_case(&a.sha256),
        "Original asset checksum/size changed: {}",
        a.name
    );
    Ok(path)
}
struct Case {
    root: PathBuf,
    reference: Reference,
    mix: Mix,
    notes: PathBuf,
    delay: f64,
}
fn load_case(path: &Path) -> Result<Case> {
    let root = path.canonicalize()?;
    let reference: Reference = json_file(&root.join("reference-source.json"))?;
    let mix: Mix = json_file(&root.join("source.mix.json"))?;
    ensure!(
        reference.version == 1 && mix.version == 1,
        "Unsupported source receipt version"
    );
    ensure!(
        hash_valid(&reference.source_archive_sha256)
            && reference.source_archive_sha256 == mix.source_archive_sha256,
        "Archive receipt mismatch"
    );
    ensure!(
        mix.sample_rate == RATE
            && mix.channels == 2
            && mix.frames > 0
            && mix.frames <= MAX_FRAMES
            && mix.duration_seconds.is_finite()
            && (mix.duration_seconds - mix.frames as f64 / RATE as f64).abs() < 1e-6,
        "Invalid canonical source clock"
    );
    let source = root.join("source.wav");
    ordinary(&source)?;
    ensure!(
        hash_valid(&mix.output_sha256) && hash_file(&source)? == mix.output_sha256,
        "Canonical source checksum changed"
    );
    ensure!(
        reference.assets.len() <= 10000 && mix.included_stems.len() <= 64,
        "Asset budget exceeded"
    );
    let mut names = BTreeSet::new();
    for a in &reference.assets {
        ensure!(
            flat(&a.name) && names.insert(a.name.to_lowercase()) && hash_valid(&a.sha256),
            "Invalid/duplicate asset receipt"
        );
    }
    let mut stems = BTreeSet::new();
    for stem in &mix.included_stems {
        ensure!(
            flat(&stem.stem)
                && stems.insert(stem.stem.to_lowercase())
                && hash_valid(&stem.sha256)
                && stem.starts_at_source_frame == 0
                && stem.frames > 0
                && stem.frames <= mix.frames,
            "Invalid mix stem provenance"
        );
    }
    ordinary(&root.join("song.ini"))?;
    ensure!(
        hash_file(&root.join("song.ini"))? == reference.song_ini_sha256,
        "Source song.ini changed"
    );
    let delay = reference
        .metadata
        .get("delay")
        .map(|s| s.parse::<f64>())
        .transpose()?
        .unwrap_or(0.);
    ensure!(
        delay.is_finite() && delay.abs() <= 60000.,
        "Invalid reference delay"
    );
    ensure!(
        (mix.ini_delay_milliseconds.unwrap_or(0.) - delay).abs() < 1e-6,
        "Mix/reference delay witnesses differ"
    );
    let assets: Vec<_> = reference
        .assets
        .iter()
        .filter(|a| {
            matches!(
                a.name.to_ascii_lowercase().as_str(),
                "notes.mid" | "notes.chart"
            )
        })
        .collect();
    ensure!(
        assets.len() == 1,
        "Expected one unambiguous reference notes asset"
    );
    let notes = verify_asset(&root, assets[0])?;
    Ok(Case {
        root,
        reference,
        mix,
        notes,
        delay,
    })
}
fn wave_header(frames: u64) -> Result<Vec<u8>> {
    let size = frames.checked_mul(8).context("PCM size overflow")?;
    ensure!(size <= u32::MAX as u64 - 36, "WAV exceedsRIFFbudget");
    let mut b = vec![];
    b.extend(b"RIFF");
    b.extend((36 + size as u32).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(3u16.to_le_bytes());
    b.extend(2u16.to_le_bytes());
    b.extend((RATE as u32).to_le_bytes());
    b.extend((RATE as u32 * 8).to_le_bytes());
    b.extend(8u16.to_le_bytes());
    b.extend(32u16.to_le_bytes());
    b.extend(b"data");
    b.extend((size as u32).to_le_bytes());
    Ok(b)
}
/// Stream decode without trimming, offset, normalization, or gain. Optional
/// destination is a float WAV padded to the canonical mix's exact frame count.
fn decode(
    path: &Path,
    expected: u64,
    target: u64,
    destination: Option<&Path>,
    log: &Path,
) -> Result<u64> {
    ensure!(
        expected <= target && target <= MAX_FRAMES,
        "Invalid decode clock budget"
    );
    let error = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(log)?;
    let mut command = Command::new("ffmpeg");
    command
        .args([
            "-v",
            "error",
            "-nostdin",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ])
        .arg(path)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-sn",
            "-dn",
            "-ac",
            "2",
            "-ar",
            "44100",
            "-f",
            "f32le",
            "-acodec",
            "pcm_f32le",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::from(error));
    let mut child = command.spawn().context("ffmpeg is required")?;
    let result = (|| -> Result<u64> {
        let mut stream = child.stdout.take().context("No decoder stdout")?;
        let mut writer = destination
            .map(|p| fs::OpenOptions::new().write(true).create_new(true).open(p))
            .transpose()?;
        if let Some(w) = &mut writer {
            w.write_all(&wave_header(target)?)?;
        }
        let (mut total, mut pending) = (0u64, Vec::<u8>::new());
        let mut bytes = [0u8; 65536];
        loop {
            let count = stream.read(&mut bytes)?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(count as u64)
                .context("PCM byte overflow")?;
            ensure!(
                total <= expected * 8,
                "Decoded audio is longer than its receipt clock"
            );
            pending.extend_from_slice(&bytes[..count]);
            let complete = pending.len() / 4 * 4;
            for chunk in pending[..complete].chunks_exact(4) {
                ensure!(
                    f32::from_le_bytes(chunk.try_into()?).is_finite(),
                    "Nonfinite decoded audio"
                );
            }
            pending.drain(..complete);
            if let Some(w) = &mut writer {
                w.write_all(&bytes[..count])?;
            }
        }
        ensure!(
            pending.is_empty() && total == expected * 8,
            "Decoded source frame count differs from receipt (shorter/partial source)"
        );
        if let Some(w) = &mut writer {
            let mut remaining = (target - expected) * 8;
            let zeros = [0u8; 65536];
            while remaining > 0 {
                let n = remaining.min(zeros.len() as u64) as usize;
                w.write_all(&zeros[..n])?;
                remaining -= n as u64;
            }
            w.sync_all()?;
        }
        Ok(total / 8)
    })();
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    ensure!(
        status.success() || result.is_err(),
        "ffmpeg decode failed; see {}",
        log.display()
    );
    result
}
pub fn prepare(case: &Path, output: &Path) -> Result<()> {
    ensure!(!output.exists(), "Refusing existing oracle output");
    let case = load_case(case)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    ensure!(parent.exists(), "Oracle parent must exist");
    let stage = tempfile::Builder::new()
        .prefix(".oracle-prepare-")
        .tempdir_in(parent)?;
    decode(
        &case.root.join("source.wav"),
        case.mix.frames,
        case.mix.frames,
        None,
        &stage.path().join("source-decode.log"),
    )?;
    let mut assets = vec![];
    let mut roles = vec![];
    for id in [StemId::Bass, StemId::Guitar, StemId::Drums, StemId::Vocals] {
        let candidates: Vec<_> = case
            .reference
            .assets
            .iter()
            .filter(|a| {
                let path = Path::new(&a.name);
                path.file_stem()
                    .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(id.as_str()))
                    && path.extension().is_some_and(|e| {
                        matches!(
                            e.to_string_lossy().to_ascii_lowercase().as_str(),
                            "opus" | "ogg" | "flac" | "wav" | "mp3"
                        )
                    })
            })
            .collect();
        ensure!(
            candidates.len() <= 1,
            "Ambiguous duplicate role encodings: {}",
            id.as_str()
        );
        let Some(asset) = candidates.first() else {
            roles.push(json!({"role":id,"status":"unavailable","reason":"No direct authored role; split/backing stems are not guessed or reassigned"}));
            continue;
        };
        let included = case
            .mix
            .included_stems
            .iter()
            .find(|s| s.stem.eq_ignore_ascii_case(id.as_str()))
            .context("Direct oracle role missing from mix provenance")?;
        ensure!(
            included.sha256.eq_ignore_ascii_case(&asset.sha256),
            "Role/mix original asset hashes differ"
        );
        let original = verify_asset(&case.root, asset)?;
        let name = format!("{}.wav", id.as_str());
        let destination = stage.path().join(&name);
        let frames = decode(
            &original,
            included.frames,
            case.mix.frames,
            Some(&destination),
            &stage.path().join(format!("{}-decode.log", id.as_str())),
        )?;
        verify_asset(&case.root, asset)?;
        let sha256 = hash_file(&destination)?;
        assets.push(StemAsset {
            id,
            path: PathBuf::from(name),
            sha256: sha256.clone(),
        });
        roles.push(json!({"role":id,"status":"prepared","original":asset.name,"original_sha256":asset.sha256,"decoded_frames":frames,"padded_frames":case.mix.frames-frames,"output_sha256":sha256}));
    }
    ensure!(!assets.is_empty(), "No supported direct oracle roles");
    let mut manifest=StemManifest{version:1,source_audio_sha256:case.mix.output_sha256.clone(),sample_rate:RATE as u32,channels:2,frames:case.mix.frames,alignment:AlignmentEvidence{method:"authored_archive_sample_zero_with_tail_padding".into(),details:"Direct authored role files from the same checksum-bound archive/mix receipt; ffmpeg44100Hz stereo float decode starts at frame0. Only missing tail frames zero-padded; no trim, offset, gain, normalization, or stem reassignment. Oracle condition; file identity/clock is not isolation purity or musical acceptance.".into()},stems:assets,generation:None};
    for stem in &mut manifest.stems {
        stem.path = stage.path().join(&stem.path).canonicalize()?;
    }
    manifest.validate_binding(&case.mix.output_sha256, case.mix.duration_seconds)?;
    let manifest_path = stage.path().join("stem-manifest.json");
    manifest.write_relative(&manifest_path)?;
    StemManifest::read(&manifest_path)?
        .validate_binding(&case.mix.output_sha256, case.mix.duration_seconds)?;
    let lyrics = benchmark::lyric_source(&fs::read(&case.notes)?);
    let lyric_status = match lyrics {
        Ok(document) => {
            fs::write(
                stage.path().join("lyric-source.json"),
                serde_json::to_vec_pretty(&document)?,
            )?;
            json!({"status":"text_only","path":"lyric-source.json","timing_supplied":false})
        }
        Err(e) => json!({"status":"unavailable","error":format!("{e:#}"),"timing_supplied":false}),
    };
    ensure!(
        hash_file(&case.root.join("source.wav"))? == case.mix.output_sha256,
        "Canonical source changed during preparation"
    );
    let verified = load_case(&case.root)?;
    ensure!(
        verified.mix.output_sha256 == case.mix.output_sha256,
        "Source witness changed"
    );
    let receipt = json!({"version":1,"condition":"oracle authored direct stems; supplied reference lyric TEXT only","case":case.root,"source_sha256":case.mix.output_sha256,"source_archive_sha256":case.reference.source_archive_sha256,"reference_receipt_sha256":hash_file(&case.root.join("reference-source.json"))?,"mix_receipt_sha256":hash_file(&case.root.join("source.mix.json"))?,"reference_notes_sha256":hash_file(&case.notes)?,"reference_delay_ms":case.delay,"generated_delay_ms":0,"sample_rate":RATE,"channels":2,"frames":case.mix.frames,"roles":roles,"lyrics":lyric_status,"targets_enter_generation":false,"limitation":"Reference notes only supply plain lyric text and later comparison targets. Split role files and harmony/lead assignment are unsupported; no source gain copied into stems. Original compressed assets untouched."});
    fs::write(
        stage.path().join("preparation.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    chart_engine::pipeline::publish(stage.path(), output)?;
    Ok(())
}
pub fn compare(case: &Path, generated: &Path, output: &Path) -> Result<()> {
    ensure!(!output.exists(), "Refusing existing comparison report");
    let validation = load_case(case);
    let mut results = vec![];
    for instrument in [
        Instrument::Guitar,
        Instrument::Bass,
        Instrument::Drums,
        Instrument::ProDrums,
        Instrument::Vocals,
    ] {
        let result = match &validation {
            Ok(case) => benchmark::compare(benchmark::Request {
                reference_midi: &case.notes,
                generated_midi: generated,
                reference_audio: &case.root.join("source.wav"),
                generated_audio: &case.root.join("source.wav"),
                reference_delay_ms: case.delay,
                generated_delay_ms: 0.,
                instrument,
            }),
            Err(e) => Err(anyhow::anyhow!("Source validation failed: {e:#}")),
        };
        results.push(match result {
            Ok(report) => json!({"instrument":instrument,"success":true,"report":report}),
            Err(e) => json!({"instrument":instrument,"success":false,"error":format!("{e:#}")}),
        });
    }
    let report = json!({"version":1,"condition":"Expert/Pro only; same hash-bound recording; generated offset0, reference INI delay plus embedded chart offset; no fitted alignment","generated":generated,"generated_sha256":hash_file(generated).ok(),"reference_case":case,"results":results,"limitations":["This compares chart representations, not acoustic instrument truth, listening acceptance, fun, or editing time. Every instrument failure remains visible.","Generation provenance must independently bind generated notes to this canonical source; supplying the same source to both comparator sides verifies reference identity, not arbitrary generated-chart origin."]});
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    file.write_all(&serde_json::to_vec_pretty(&report)?)?;
    file.sync_all()?;
    Ok(())
}
