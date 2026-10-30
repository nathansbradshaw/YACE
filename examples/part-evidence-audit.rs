//! Independent evidence comparison only. Reference positions never enter detection.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    benchmark::{self, Instrument, Parsed},
    guitar_attacks, larsnet,
    mapping::{GuitarSource, Transcription, Voice},
    models, pipeline, review,
    stem_review::{StemId, StemManifest},
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    prepared_case: PathBuf,
    #[arg(long)]
    events: PathBuf,
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    kit: Option<PathBuf>,
    #[arg(long)]
    output: PathBuf,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    sha256: String,
    bytes: u64,
}
#[derive(Deserialize)]
struct Reference {
    version: u32,
    source_archive_sha256: String,
    song_ini_sha256: String,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Mix {
    version: u32,
    source_archive_sha256: String,
    output_sha256: String,
    sample_rate: u32,
    channels: u16,
    frames: u64,
    ini_delay_milliseconds: Option<f64>,
    included_stems: Vec<IncludedStem>,
}
#[derive(Deserialize)]
struct IncludedStem {
    stem: String,
    sha256: String,
    frames: u64,
    starts_at_source_frame: u64,
}
#[derive(Deserialize)]
struct PreparedRole {
    role: StemId,
    status: String,
    output_sha256: Option<String>,
    original_sha256: Option<String>,
}
#[derive(Deserialize)]
struct Preparation {
    version: u32,
    source_sha256: String,
    source_archive_sha256: String,
    reference_receipt_sha256: String,
    mix_receipt_sha256: String,
    reference_notes_sha256: String,
    frames: u64,
    roles: Vec<PreparedRole>,
}
#[derive(Serialize)]
struct Metrics {
    tolerance_ms: u32,
    reference_attacks: usize,
    detected_attacks: usize,
    matched: usize,
    precision: Option<f64>,
    recall: Option<f64>,
    f1: Option<f64>,
}
#[derive(Serialize)]
struct Agreement {
    tolerance_ms: u32,
    overlap: usize,
    independent_only: usize,
    independent_only_reference_hits: usize,
    independent_only_extras: usize,
    newly_covered_reference_attacks: usize,
}
#[derive(Serialize)]
struct Comparison {
    retained: Vec<Metrics>,
    independent: Vec<Metrics>,
    agreement: Vec<Agreement>,
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 64 * 1024 * 1024,
        "Diagnostic input exceeds 64 MiB"
    );
    Ok(serde_json::from_slice(&bytes)?)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn unchanged(witnesses: &[(PathBuf, String)]) -> Result<()> {
    for (path, expected) in witnesses {
        ensure!(
            models::hash_file(path)? == *expected,
            "An audit witness changed during analysis"
        );
    }
    Ok(())
}
fn times(values: impl Iterator<Item = f64>) -> Vec<f64> {
    let mut values: Vec<_> = values.collect();
    values.sort_by(f64::total_cmp);
    values.dedup();
    values
}
fn metrics(reference: &[f64], detected: &[f64], tolerance_ms: u32) -> Result<Metrics> {
    let matched = benchmark::match_attacks(reference, detected, tolerance_ms as f64 / 1000.)?.len();
    Ok(Metrics {
        tolerance_ms,
        reference_attacks: reference.len(),
        detected_attacks: detected.len(),
        matched,
        precision: (!detected.is_empty()).then(|| matched as f64 / detected.len() as f64),
        recall: (!reference.is_empty()).then(|| matched as f64 / reference.len() as f64),
        f1: (!(reference.is_empty() && detected.is_empty()))
            .then(|| 2. * matched as f64 / (reference.len() + detected.len()) as f64),
    })
}
fn comparison(reference: &[f64], retained: &[f64], independent: &[f64]) -> Result<Comparison> {
    let mut result = Comparison {
        retained: vec![],
        independent: vec![],
        agreement: vec![],
    };
    for tolerance_ms in [25, 50, 100] {
        let tolerance = tolerance_ms as f64 / 1000.;
        let retained_hits: BTreeSet<_> = benchmark::match_attacks(reference, retained, tolerance)?
            .into_iter()
            .map(|(r, _)| r)
            .collect();
        let independent_matches = benchmark::match_attacks(reference, independent, tolerance)?;
        let overlapping: BTreeSet<_> = benchmark::match_attacks(retained, independent, tolerance)?
            .into_iter()
            .map(|(_, i)| i)
            .collect();
        let hit_candidates: BTreeSet<_> = independent_matches.iter().map(|(_, i)| *i).collect();
        result
            .retained
            .push(metrics(reference, retained, tolerance_ms)?);
        result
            .independent
            .push(metrics(reference, independent, tolerance_ms)?);
        result.agreement.push(Agreement {
            tolerance_ms,
            overlap: overlapping.len(),
            independent_only: independent.len() - overlapping.len(),
            independent_only_reference_hits: hit_candidates.difference(&overlapping).count(),
            independent_only_extras: (0..independent.len())
                .filter(|i| !overlapping.contains(i) && !hit_candidates.contains(i))
                .count(),
            newly_covered_reference_attacks: independent_matches
                .iter()
                .filter(|(r, _)| !retained_hits.contains(r))
                .count(),
        });
    }
    Ok(result)
}
fn family(voice: Voice) -> &'static str {
    match voice {
        Voice::Kick => "kick",
        Voice::Snare => "snare",
        Voice::HiHat => "hihat",
        Voice::HighTom | Voice::MidTom | Voice::LowTom | Voice::Tom => "toms",
        Voice::Crash | Voice::Ride | Voice::Cymbal => "cymbals",
    }
}
fn reference_family(lane: Option<u8>, cymbal: Option<bool>) -> Result<&'static str> {
    Ok(match (lane, cymbal) {
        (Some(0), _) => "kick",
        (Some(1), _) => "snare",
        (Some(2), Some(true)) => "hihat",
        (Some(3 | 4), Some(true)) => "cymbals",
        (Some(2..=4), Some(false)) => "toms",
        _ => anyhow::bail!("Reference does not provide a supported four-lane Pro drum family"),
    })
}
fn reference_families(parsed: &Parsed) -> Result<BTreeMap<&'static str, Vec<f64>>> {
    let mut groups: BTreeMap<_, Vec<_>> = larsnet::MODELS
        .into_iter()
        .map(|(name, _)| (name, vec![]))
        .collect();
    for note in &parsed.notes {
        groups
            .get_mut(reference_family(note.lane, note.cymbal)?)
            .context("Unknown reference family")?
            .push(note.start);
    }
    for values in groups.values_mut() {
        *values = times(values.iter().copied());
    }
    Ok(groups)
}
fn bind_manifest(events: &Transcription, manifest: &StemManifest) -> Result<()> {
    ensure!(
        events.guitar_source == GuitarSource::Guitar,
        "Guitar role reassignment is outside this study"
    );
    let saved = events
        .isolation
        .as_ref()
        .context("Events have no source-bound stem provenance")?;
    let fingerprints = |value: &StemManifest| {
        value
            .stems
            .iter()
            .map(|s| (s.id, s.sha256.to_ascii_lowercase()))
            .collect::<BTreeMap<_, _>>()
    };
    ensure!(
        saved
            .source_audio_sha256
            .eq_ignore_ascii_case(&manifest.source_audio_sha256)
            && saved.frames == manifest.frames
            && saved.sample_rate == manifest.sample_rate
            && saved.channels == manifest.channels
            && fingerprints(saved) == fingerprints(manifest),
        "Events and supplied source/stem clocks or hashes differ"
    );
    Ok(())
}
fn stem(manifest: &StemManifest, id: StemId) -> Result<&chart_engine::stem_review::StemAsset> {
    manifest
        .stems
        .iter()
        .find(|s| s.id == id)
        .context("Required direct instrument stem is unavailable")
}
fn bind_direct_roles(
    preparation: &Preparation,
    manifest: &StemManifest,
    reference: &Reference,
    reference_hash: &str,
    mix_hash: &str,
    notes_hash: &str,
) -> Result<()> {
    ensure!(
        preparation.version == 1
            && preparation.source_sha256 == manifest.source_audio_sha256
            && preparation.source_archive_sha256 == reference.source_archive_sha256
            && preparation.reference_receipt_sha256 == reference_hash
            && preparation.mix_receipt_sha256 == mix_hash
            && preparation.reference_notes_sha256 == notes_hash
            && preparation.frames == manifest.frames,
        "Direct-role preparation does not bind this source/reference"
    );
    for id in [StemId::Guitar, StemId::Drums] {
        let roles: Vec<_> = preparation.roles.iter().filter(|r| r.role == id).collect();
        ensure!(
            roles.len() == 1,
            "Duplicate or missing direct-role preparation witness"
        );
        let role = roles[0];
        ensure!(
            role.status == "prepared"
                && role.output_sha256.as_ref() == Some(&stem(manifest, id)?.sha256)
                && role
                    .original_sha256
                    .as_ref()
                    .is_some_and(|h| reference.assets.iter().any(|a| &a.sha256 == h
                        && Path::new(&a.name).file_stem().is_some_and(|n| n
                            .to_string_lossy()
                            .eq_ignore_ascii_case(id.as_str())))),
            "Instrument stem is not a witnessed direct authored role"
        );
    }
    Ok(())
}
fn bind_mix_roles(preparation: &Preparation, mix: &Mix) -> Result<()> {
    for id in [StemId::Guitar, StemId::Drums] {
        let role = preparation
            .roles
            .iter()
            .find(|r| r.role == id)
            .context("Direct role witness missing")?;
        let included: Vec<_> = mix
            .included_stems
            .iter()
            .filter(|s| s.stem.eq_ignore_ascii_case(id.as_str()))
            .collect();
        ensure!(
            included.len() == 1,
            "Direct role missing or duplicated in source mix"
        );
        let included = included[0];
        ensure!(
            role.original_sha256
                .as_ref()
                .is_some_and(|h| h.eq_ignore_ascii_case(&included.sha256))
                && included.starts_at_source_frame == 0
                && included.frames > 0
                && included.frames <= mix.frames,
            "Direct role original hash or source-zero clock differs from source mix"
        );
    }
    Ok(())
}
fn mono(path: &Path, rate: u32, duration: f64) -> Result<Vec<f32>> {
    let audio = models::decode(path, rate, 1)?;
    ensure!(
        audio.iter().all(|v| v.is_finite())
            && (audio.len() as f64 / rate as f64 - duration).abs() <= 2. / rate as f64,
        "Evidence audio has invalid samples or another source clock"
    );
    Ok(audio)
}
fn pcm_hash(audio: &[f32]) -> String {
    let mut digest = Sha256::new();
    for sample in audio {
        digest.update(sample.to_le_bytes());
    }
    format!("{:x}", digest.finalize())
}
fn kit_attacks(
    directory: &Path,
    manifest: &StemManifest,
) -> Result<(BTreeMap<String, Vec<f64>>, larsnet::SeparationReceipt)> {
    let receipt_hash = models::hash_file(&directory.join(larsnet::RECEIPT_FILENAME))?;
    let receipt: larsnet::SeparationReceipt =
        read_json(&directory.join(larsnet::RECEIPT_FILENAME))?;
    let drums = stem(manifest, StemId::Drums)?;
    let expected: BTreeMap<String, String> = larsnet::MODELS
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
    ensure!(
        receipt.version == 1
            && receipt.algorithm == larsnet::ALGORITHM
            && receipt.implementation_sha256 == larsnet::implementation_hash()
            && receipt.source_audio_sha256.as_ref() == Some(&drums.sha256)
            && receipt.models == expected
            && receipt.sample_rate == 44100
            && receipt.channels == 2
            && receipt.frames == manifest.frames
            && receipt.outputs.len() == 5,
        "Kit receipt source/model/clock differs from the supplied drum stem"
    );
    let source = models::decode(&drums.path, 44100, 2)?;
    ensure!(
        source.len() as u64 == manifest.frames * 2
            && source.iter().all(|v| v.is_finite())
            && pcm_hash(&source) == receipt.source_pcm_sha256,
        "Kit source PCM hash differs"
    );
    drop(source);
    let mut result = BTreeMap::new();
    for output in &receipt.outputs {
        ensure!(
            expected.contains_key(&output.part)
                && output.filename == format!("{}.wav", output.part)
                && output.frames == manifest.frames
                && output.rms.is_finite()
                && output.peak.is_finite(),
            "Invalid kit output receipt"
        );
        let path = directory.join(&output.filename);
        ensure!(
            output.wav_sha256.as_ref() == Some(&models::hash_file(&path)?),
            "Kit WAV hash differs"
        );
        let native = models::decode(&path, 44100, 2)?;
        ensure!(
            native.len() as u64 == manifest.frames * 2
                && native.iter().all(|v| v.is_finite())
                && pcm_hash(&native) == output.pcm_sha256,
            "Kit output PCM hash differs"
        );
        drop(native);
        let audio = mono(&path, 22050, manifest.frames as f64 / 44100.)?;
        let attacks = times(review::attacks(&audio).iter().map(|a| a.time));
        ensure!(
            result.insert(output.part.clone(), attacks).is_none(),
            "Duplicate kit output family"
        );
        ensure!(
            output.wav_sha256.as_ref() == Some(&models::hash_file(&path)?),
            "Kit WAV changed during analysis"
        );
    }
    ensure!(
        models::hash_file(&drums.path)? == drums.sha256,
        "Drum stem changed during kit analysis"
    );
    ensure!(
        models::hash_file(&directory.join(larsnet::RECEIPT_FILENAME))? == receipt_hash,
        "Kit receipt changed during analysis"
    );
    Ok((result, receipt))
}
fn run(args: Args) -> Result<()> {
    ensure!(
        !args.output.exists(),
        "Refusing an existing diagnostic output"
    );
    let root = args.prepared_case.canonicalize()?;
    let reference_path = root.join("reference-source.json");
    let mix_path = root.join("source.mix.json");
    let reference_hash = models::hash_file(&reference_path)?;
    let mix_hash = models::hash_file(&mix_path)?;
    let reference: Reference = read_json(&reference_path)?;
    let mix: Mix = read_json(&mix_path)?;
    ensure!(
        reference.version == 1
            && mix.version == 1
            && valid_hash(&reference.source_archive_sha256)
            && reference.source_archive_sha256 == mix.source_archive_sha256
            && mix.sample_rate == 44100
            && mix.channels == 2
            && (1..=44100 * 3600).contains(&mix.frames),
        "Invalid prepared source witnesses"
    );
    ensure!(
        reference.assets.len() <= 10000
            && reference.assets.iter().all(|a| valid_hash(&a.sha256)
                && !a.name.contains(['/', '\\'])
                && Path::new(&a.name).components().count() == 1),
        "Invalid original asset witnesses"
    );
    let source = root.join("source.wav");
    ensure!(
        models::hash_file(&source)? == mix.output_sha256
            && models::hash_file(&root.join("song.ini"))? == reference.song_ini_sha256,
        "Canonical audio or INI changed"
    );
    let delay = pipeline::ini_fields(&fs::read_to_string(root.join("song.ini"))?)
        .get("delay")
        .map(|s| s.parse::<f64>())
        .transpose()?
        .unwrap_or(0.);
    ensure!(
        delay.is_finite()
            && delay.abs() <= 60000.
            && (delay - mix.ini_delay_milliseconds.unwrap_or(0.)).abs() < 1e-6,
        "Reference delay witnesses differ"
    );
    let notes: Vec<_> = reference
        .assets
        .iter()
        .filter(|a| {
            matches!(
                a.name.to_ascii_lowercase().as_str(),
                "notes.mid" | "notes.chart"
            )
        })
        .collect();
    ensure!(notes.len() == 1, "Expected one reference note file");
    let notes = notes[0];
    let notes_path = root.join(&notes.name);
    ensure!(
        fs::metadata(&notes_path)?.len() == notes.bytes
            && notes.bytes <= 64 * 1024 * 1024
            && models::hash_file(&notes_path)? == notes.sha256,
        "Reference notes changed"
    );
    let manifest_hash = models::hash_file(&args.manifest)?;
    let events_hash = models::hash_file(&args.events)?;
    let manifest = pipeline::read_isolation(&source, &args.manifest)?;
    let events: Transcription = read_json(&args.events)?;
    events.validate()?;
    bind_manifest(&events, &manifest)?;
    let preparation_path = args
        .manifest
        .parent()
        .context("Manifest has no parent")?
        .join("preparation.json");
    let preparation_hash = models::hash_file(&preparation_path)?;
    let preparation: Preparation = read_json(&preparation_path)?;
    bind_direct_roles(
        &preparation,
        &manifest,
        &reference,
        &reference_hash,
        &mix_hash,
        &notes.sha256,
    )?;
    bind_mix_roles(&preparation, &mix)?;
    let mut witnesses = vec![
        (reference_path, reference_hash),
        (mix_path, mix_hash),
        (preparation_path, preparation_hash.clone()),
        (root.join("song.ini"), reference.song_ini_sha256.clone()),
    ];
    for role in &preparation.roles {
        if matches!(role.role, StemId::Guitar | StemId::Drums) {
            let asset = reference
                .assets
                .iter()
                .find(|a| {
                    role.original_sha256.as_ref() == Some(&a.sha256)
                        && Path::new(&a.name).file_stem().is_some_and(|s| {
                            s.to_string_lossy().eq_ignore_ascii_case(role.role.as_str())
                        })
                })
                .context("Direct role original witness missing")?;
            let path = root.join(&asset.name);
            ensure!(
                fs::metadata(&path)?.len() == asset.bytes
                    && models::hash_file(&path)? == asset.sha256,
                "Direct authored original asset changed"
            );
            witnesses.push((path, asset.sha256.clone()));
        }
    }
    let duration = manifest.frames as f64 / 44100.;
    ensure!(
        events
            .guitar
            .iter()
            .all(|n| n.start >= 0. && n.end <= duration + 0.1)
            && events
                .drums
                .iter()
                .all(|h| h.time >= 0. && h.time <= duration),
        "Events exceed supplied source clock"
    );
    let guitar_audio = mono(&stem(&manifest, StemId::Guitar)?.path, 22050, duration)?;
    let independent = times(
        guitar_attacks::detect(&guitar_audio, 22050)?
            .into_iter()
            .map(|a| a.seconds),
    );
    let retained = times(events.guitar.iter().map(|n| n.start));
    // Detection is finished before reading the authored note positions.
    let kit = args
        .kit
        .as_ref()
        .map(|dir| kit_attacks(dir, &manifest))
        .transpose()?;
    if let (Some(directory), Some((_, receipt))) = (&args.kit, &kit) {
        witnesses.push((
            directory.join(larsnet::RECEIPT_FILENAME),
            models::hash_file(&directory.join(larsnet::RECEIPT_FILENAME))?,
        ));
        for output in &receipt.outputs {
            witnesses.push((
                directory.join(&output.filename),
                output.wav_sha256.clone().context("Kit WAV hash missing")?,
            ));
        }
    }
    let bytes = fs::read(&notes_path)?;
    let reference_guitar =
        benchmark::attack_times(&benchmark::parse(&bytes, Instrument::Guitar, delay)?);
    let guitar = comparison(&reference_guitar, &retained, &independent)?;
    let drum_report = if let Some((parts, receipt)) = &kit {
        let families = reference_families(&benchmark::parse(&bytes, Instrument::ProDrums, delay)?)?;
        let mut reports = BTreeMap::new();
        for (name, _) in larsnet::MODELS {
            let retained = times(
                events
                    .drums
                    .iter()
                    .filter(|h| {
                        family(h.voice) == name
                            && !events.drum_recovered_hits.iter().any(|r| {
                                r.time == h.time
                                    && r.voice == h.voice
                                    && r.confidence == h.confidence
                            })
                    })
                    .map(|h| h.time),
            );
            reports.insert(name, comparison(&families[name], &retained, &parts[name])?);
        }
        Some(
            serde_json::json!({"families":reports,"algorithm":receipt.algorithm,"implementation_sha256":receipt.implementation_sha256,"models":receipt.models,
            "source_pcm_sha256":receipt.source_pcm_sha256,"outputs":receipt.outputs.iter().map(|o|serde_json::json!({"family":o.part,"wav_sha256":o.wav_sha256,"pcm_sha256":o.pcm_sha256,"frames":o.frames})).collect::<Vec<_>>(),
            "scope":"Five merged families only; no ride/crash or high/mid/low tom articulation result"}),
        )
    } else {
        None
    };
    ensure!(
        models::hash_file(&args.events)? == events_hash
            && models::hash_file(&args.manifest)? == manifest_hash
            && models::hash_file(&source)? == mix.output_sha256
            && models::hash_file(&notes_path)? == notes.sha256,
        "Inputs changed during audit"
    );
    for asset in &manifest.stems {
        ensure!(
            models::hash_file(&asset.path)? == asset.sha256,
            "Attached stem changed during audit"
        );
    }
    unchanged(&witnesses)?;
    let report = serde_json::json!({"version":1,"experiment":"independent-part-evidence-v1","source_sha256":mix.output_sha256,"source_archive_sha256":reference.source_archive_sha256,
        "reference_notes_sha256":notes.sha256,"reference_delay_ms":delay,"events_sha256":events_hash,"manifest_sha256":manifest_hash,"preparation_sha256":preparation_hash,
        "frames":manifest.frames,"sample_rate":44100,"stems":manifest.stems.iter().map(|s|serde_json::json!({"role":s.id,"sha256":s.sha256})).collect::<Vec<_>>(),
        "implementation_sha256":hash(include_bytes!("part-evidence-audit.rs")),"guitar_detector_sha256":hash(include_bytes!("../src/guitar_attacks.rs")),"drum_detector_sha256":hash(include_bytes!("../src/review.rs")),
        "expected_historical_models_not_inference_proof":models::assets().into_iter().filter(|m|["pitch","drums"].contains(&m.id.as_str())).map(|m|serde_json::json!({"id":m.id,"sha256":m.sha256})).collect::<Vec<_>>(),
        "guitar":{"algorithm":guitar_attacks::ALGORITHM,"comparison":guitar},"drums":drum_report,
        "limitations":["Diagnostic evidence comparison only; no generator change or actual chart improvement claim.","Reference notes are scoring targets only; no timing offset was fitted.","Guitar attacks are grouped starts, not fret palette, pitch accuracy, sustain quality, or fun.","Retained guitar events include normal decoder/recovery output; drums exclude recorded recovered hits for the raw ADTOF comparison.","Saved events bind source/stem hashes but do not independently prove historical model execution; expected registry model identities are labeled separately.","Five-family drum audio activity is not Pro articulation truth; masked bleed and bright/noisy guitar transients can be false evidence.","Independent-only overlap statistics use separate optimal one-to-one assignments; they are not a proposed union chart."]});
    let parent = args
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut stage = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut stage, &report)?;
    stage.flush()?;
    stage.as_file().sync_all()?;
    stage.persist_noclobber(&args.output).map_err(|e| e.error)?;
    Ok(())
}
fn main() -> Result<()> {
    run(Args::parse())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> StemManifest {
        StemManifest {
            version: 1,
            source_audio_sha256: "a".repeat(64),
            sample_rate: 44100,
            channels: 2,
            frames: 44100,
            alignment: chart_engine::stem_review::AlignmentEvidence {
                method: "authored".into(),
                details: "control".into(),
            },
            generation: None,
            stems: [StemId::Guitar, StemId::Drums]
                .into_iter()
                .enumerate()
                .map(|(i, id)| chart_engine::stem_review::StemAsset {
                    id,
                    path: PathBuf::from(format!("{i}.wav")),
                    sha256: (if i == 0 { "b" } else { "c" }).repeat(64),
                })
                .collect(),
        }
    }
    #[test]
    fn source_clock_role_swaps_and_changed_witnesses_fail_closed() {
        let manifest = manifest();
        let mut events = Transcription {
            isolation: Some(manifest.clone()),
            ..Default::default()
        };
        bind_manifest(&events, &manifest).unwrap();
        events.isolation.as_mut().unwrap().frames += 1;
        assert!(bind_manifest(&events, &manifest).is_err());
        events.isolation = Some(manifest.clone());
        events.isolation.as_mut().unwrap().stems.swap(0, 1);
        bind_manifest(&events, &manifest).unwrap(); // Order is not identity.
        events.isolation.as_mut().unwrap().stems[0].sha256 = "d".repeat(64);
        assert!(bind_manifest(&events, &manifest).is_err());
        events.isolation = Some(manifest.clone());
        events.guitar_source = GuitarSource::Other;
        assert!(bind_manifest(&events, &manifest).is_err());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("witness");
        fs::write(&path, b"original").unwrap();
        let witnesses = vec![(path.clone(), models::hash_file(&path).unwrap())];
        unchanged(&witnesses).unwrap();
        fs::write(path, b"changed").unwrap();
        assert!(unchanged(&witnesses).is_err());
    }
    #[test]
    fn direct_role_receipt_cannot_reassign_another_source_or_stem() {
        let manifest = manifest();
        let reference = Reference {
            version: 1,
            source_archive_sha256: "d".repeat(64),
            song_ini_sha256: "e".repeat(64),
            assets: vec![
                Asset {
                    name: "guitar.opus".into(),
                    sha256: "f".repeat(64),
                    bytes: 1,
                },
                Asset {
                    name: "drums.opus".into(),
                    sha256: "0".repeat(64),
                    bytes: 1,
                },
            ],
        };
        let mut prep = Preparation {
            version: 1,
            source_sha256: manifest.source_audio_sha256.clone(),
            source_archive_sha256: reference.source_archive_sha256.clone(),
            reference_receipt_sha256: "1".repeat(64),
            mix_receipt_sha256: "2".repeat(64),
            reference_notes_sha256: "3".repeat(64),
            frames: manifest.frames,
            roles: manifest
                .stems
                .iter()
                .enumerate()
                .map(|(i, s)| PreparedRole {
                    role: s.id,
                    status: "prepared".into(),
                    output_sha256: Some(s.sha256.clone()),
                    original_sha256: Some(reference.assets[i].sha256.clone()),
                })
                .collect(),
        };
        let check = |p: &Preparation| {
            bind_direct_roles(
                p,
                &manifest,
                &reference,
                &"1".repeat(64),
                &"2".repeat(64),
                &"3".repeat(64),
            )
        };
        check(&prep).unwrap();
        let mut mix = Mix {
            version: 1,
            source_archive_sha256: reference.source_archive_sha256.clone(),
            output_sha256: manifest.source_audio_sha256.clone(),
            sample_rate: 44100,
            channels: 2,
            frames: manifest.frames,
            ini_delay_milliseconds: None,
            included_stems: prep
                .roles
                .iter()
                .map(|r| IncludedStem {
                    stem: r.role.as_str().into(),
                    sha256: r.original_sha256.clone().unwrap(),
                    frames: manifest.frames,
                    starts_at_source_frame: 0,
                })
                .collect(),
        };
        bind_mix_roles(&prep, &mix).unwrap();
        mix.included_stems[0].starts_at_source_frame = 1;
        assert!(bind_mix_roles(&prep, &mix).is_err());
        mix.included_stems[0].starts_at_source_frame = 0;
        mix.included_stems[0].sha256 = mix.included_stems[1].sha256.clone();
        assert!(bind_mix_roles(&prep, &mix).is_err());
        prep.roles[0].output_sha256 = prep.roles[1].output_sha256.clone();
        assert!(check(&prep).is_err());
        prep.roles[0].output_sha256 = Some(manifest.stems[0].sha256.clone());
        prep.source_sha256 = "9".repeat(64);
        assert!(check(&prep).is_err());
    }
    #[test]
    fn dense_candidates_cannot_claim_duplicate_hits_and_empty_outputs_keep_denominators() {
        let result = comparison(&[1., 2.], &[1.], &[0.99, 1.01, 2., 3.]).unwrap();
        let metric = &result.independent[1];
        assert_eq!(
            (
                metric.reference_attacks,
                metric.detected_attacks,
                metric.matched
            ),
            (2, 4, 2)
        );
        assert_eq!(
            (metric.precision, metric.recall, metric.f1),
            (Some(0.5), Some(1.), Some(2. / 3.))
        );
        assert_eq!(result.agreement[1].newly_covered_reference_attacks, 1);
        let empty = comparison(&[1., 2.], &[], &[]).unwrap();
        assert_eq!(
            (
                empty.independent[1].precision,
                empty.independent[1].recall,
                empty.independent[1].f1
            ),
            (None, Some(0.), Some(0.))
        );
        assert_eq!(metrics(&[], &[1.], 50).unwrap().f1, Some(0.));
        assert_eq!(metrics(&[], &[], 50).unwrap().f1, None);
    }
    #[test]
    fn pro_reference_collapses_articulations_without_guessing_missing_markers() {
        assert_eq!(reference_family(Some(0), Some(false)).unwrap(), "kick");
        assert_eq!(reference_family(Some(2), Some(true)).unwrap(), "hihat");
        for lane in [3, 4] {
            assert_eq!(reference_family(Some(lane), Some(true)).unwrap(), "cymbals");
        }
        for lane in [2, 3, 4] {
            assert_eq!(reference_family(Some(lane), Some(false)).unwrap(), "toms");
        }
        assert!(reference_family(Some(2), None).is_err());
        assert!(reference_family(Some(5), Some(true)).is_err());
    }
}
