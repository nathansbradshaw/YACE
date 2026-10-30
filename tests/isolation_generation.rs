//! Engineering acceptance for source-bound caches; synthetic impulses are not
//! a benchmark of Demucs separation quality or musical note transcription.
use chart_engine::{
    chart::Chart,
    mapping::{DrumHit, Transcription, Voice},
    models,
    pipeline::{self, Build, IsolationOptions, Metadata},
    stem_review::{AlignmentEvidence, SeparationIdentity, StemId, StemManifest},
    timing::TempoMap,
};
use std::{collections::BTreeMap, fs, io::Write, path::Path, process::Command};

const FRAMES: usize = 8820;

struct Fixture {
    root: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut left = vec![0.0; FRAMES];
        let mut right = vec![0.0; FRAMES];
        for index in 0..6 {
            left[100 + index * 200] = 0.125;
            right[199 + index * 200] = -0.125;
        }
        models::write_audio(&root.path().join("song.wav"), &left, &right).unwrap();
        fs::write(
            root.path().join("model.weights"),
            b"synthetic model identity fixture",
        )
        .unwrap();
        Self { root }
    }
    fn source(&self) -> std::path::PathBuf {
        self.root.path().join("song.wav")
    }
    fn model(&self) -> std::path::PathBuf {
        self.root.path().join("model.weights")
    }
    fn cache(&self) -> std::path::PathBuf {
        self.root.path().join("cache")
    }
    fn identity(&self) -> SeparationIdentity {
        SeparationIdentity {
            model_id: "impulse-fixture".into(),
            model_sha256: models::hash_file(&self.model()).unwrap(),
            backend: "deterministic-six-stem-test-fixture".into(),
            parameters: BTreeMap::from([("fixture".into(), "impulses-v1".into())]),
            cache_key: String::new(),
        }
    }
    fn write_stems(stage: &Path) -> anyhow::Result<()> {
        for (index, id) in StemId::ALL.into_iter().enumerate() {
            let mut left = vec![0.0; FRAMES];
            let mut right = vec![0.0; FRAMES];
            left[100 + index * 200] = 0.125;
            right[199 + index * 200] = -0.125;
            models::write_audio(&stage.join(format!("{}.wav", id.as_str())), &left, &right)?;
        }
        Ok(())
    }
    fn prepare(&self) -> StemManifest {
        pipeline::isolation_cached_with(
            &self.source(),
            &self.model(),
            &self.cache(),
            self.identity(),
            |_, _, stage| Self::write_stems(stage),
        )
        .unwrap()
    }
}

#[test]
fn cache_reuses_content_after_moves_and_preserves_all_six_impulse_coordinates() {
    let f = Fixture::new();
    let source_hash = models::hash_file(&f.source()).unwrap();
    let manifest = f.prepare();
    let identity = manifest.generation.as_ref().unwrap();
    let directory = f.cache().join(&identity.cache_key);
    let wire: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join(pipeline::STEM_MANIFEST)).unwrap())
            .unwrap();
    assert_eq!(wire["stems"].as_array().unwrap().len(), 6);
    for asset in wire["stems"].as_array().unwrap() {
        assert!(Path::new(asset["path"].as_str().unwrap()).is_relative());
    }
    for (index, id) in StemId::ALL.into_iter().enumerate() {
        let asset = manifest.stems.iter().find(|asset| asset.id == id).unwrap();
        assert!(asset.path.starts_with(directory.canonicalize().unwrap()));
        let pcm = models::decode(&asset.path, 44100, 2).unwrap();
        assert_eq!(pcm.len(), FRAMES * 2);
        assert_eq!(pcm[2 * (100 + index * 200)], 0.125);
        assert_eq!(pcm[2 * (199 + index * 200) + 1], -0.125);
        assert_eq!(pcm.iter().filter(|v| **v != 0.0).count(), 2);
    }
    let moved_audio = f.root.path().join("same-recording-new-name.wav");
    let moved_model = f.root.path().join("same-model-new-name.weights");
    fs::copy(f.source(), &moved_audio).unwrap();
    fs::copy(f.model(), &moved_model).unwrap();
    let reused = pipeline::isolation_cached_with(
        &moved_audio,
        &moved_model,
        &f.cache(),
        f.identity(),
        |_, _, _| panic!("content-equivalent files should reuse the complete cache"),
    )
    .unwrap();
    assert_eq!(reused, manifest);
    assert_eq!(models::hash_file(&f.source()).unwrap(), source_hash);
    let mut changed = f.identity();
    changed.parameters.insert("fixture".into(), "v2".into());
    assert_ne!(
        changed.calculated_cache_key(&source_hash).unwrap(),
        identity.cache_key
    );
    changed = f.identity();
    changed.model_sha256 = "ab".repeat(32);
    assert_ne!(
        changed.calculated_cache_key(&source_hash).unwrap(),
        identity.cache_key
    );
    assert_ne!(
        f.identity().calculated_cache_key(&"cd".repeat(32)).unwrap(),
        identity.cache_key
    );
}

#[test]
fn optional_backend_cache_is_distinct_and_cannot_replace_standard_stems() {
    let f = Fixture::new();
    let primary = f.prepare();
    let before = serde_json::to_vec(&primary).unwrap();
    let optional_model = f.root.path().join("optional-model.weights");
    fs::write(&optional_model, b"distinct optional checkpoint fixture").unwrap();
    let mut identity = f.identity();
    identity.model_id = chart_engine::keyboard_separation::MODEL_ID.into();
    identity.model_sha256 = models::hash_file(&optional_model).unwrap();
    identity.backend = "synthetic-secondary-separator-fixture".into();
    let analysis = pipeline::isolation_cached_with(
        &f.source(),
        &optional_model,
        &f.cache(),
        identity.clone(),
        |_, _, stage| {
            Fixture::write_stems(stage)?;
            fs::remove_file(stage.join("piano.wav"))?;
            let mut pcm = vec![0.; FRAMES];
            pcm[777] = 0.25;
            models::write_audio(&stage.join("piano.wav"), &pcm, &pcm)
        },
    )
    .unwrap();
    assert_ne!(
        primary.generation.as_ref().unwrap().cache_key,
        analysis.generation.as_ref().unwrap().cache_key
    );
    assert_eq!(primary.source_audio_sha256, analysis.source_audio_sha256);
    assert_eq!(primary.frames, analysis.frames);
    let piano = |manifest: &StemManifest| {
        manifest
            .stems
            .iter()
            .find(|s| s.id == StemId::Piano)
            .unwrap()
            .sha256
            .clone()
    };
    assert_ne!(piano(&primary), piano(&analysis));
    assert_eq!(serde_json::to_vec(&f.prepare()).unwrap(), before);
    let reused = pipeline::isolation_cached_with(
        &f.source(),
        &optional_model,
        &f.cache(),
        identity,
        |_, _, _| panic!("optional cache should be reused"),
    )
    .unwrap();
    assert_eq!(reused, analysis);
}

#[test]
fn source_model_and_stem_changes_fail_closed_before_inference_or_cache_replacement() {
    let f = Fixture::new();
    let manifest = f.prepare();
    let directory = f
        .cache()
        .join(&manifest.generation.as_ref().unwrap().cache_key);
    let manifest_path = directory.join(pipeline::STEM_MANIFEST);
    let other = f.root.path().join("other.wav");
    models::write_audio(&other, &vec![0.0; FRAMES], &vec![0.0; FRAMES]).unwrap();
    assert!(pipeline::read_isolation(&other, &manifest_path).is_err());
    let mut wrong_model = f.identity();
    wrong_model.model_sha256 = "aa".repeat(32);
    assert!(
        pipeline::isolation_cached_with(
            &f.source(),
            &f.model(),
            &f.cache(),
            wrong_model,
            |_, _, _| panic!("model identity must fail first")
        )
        .is_err()
    );
    let guitar = manifest
        .stems
        .iter()
        .find(|s| s.id == StemId::Guitar)
        .unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&guitar.path)
        .unwrap()
        .write_all(b"corrupt")
        .unwrap();
    let error = pipeline::transcribe_with_isolation(
        &f.source(),
        &f.root.path().join("no-models"),
        IsolationOptions {
            manifest: Some(&manifest_path),
            ..Default::default()
        },
        &["drums".into(), "guitar".into(), "bass".into()],
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("guitar stem changed"),
        "{error:#}"
    );
    assert!(
        pipeline::isolation_cached_with(
            &f.source(),
            &f.model(),
            &f.cache(),
            f.identity(),
            |_, _, _| panic!("corrupt existing cache must not silently be regenerated")
        )
        .is_err()
    );
    assert_eq!(fs::read_dir(f.cache()).unwrap().count(), 1);
}

#[test]
fn exact_sample_count_rejects_truncation_even_when_new_file_hash_is_attested() {
    let f = Fixture::new();
    let mut manifest = f.prepare();
    let drums = manifest
        .stems
        .iter_mut()
        .find(|s| s.id == StemId::Drums)
        .unwrap();
    fs::remove_file(&drums.path).unwrap();
    models::write_audio(&drums.path, &vec![0.0; FRAMES - 1], &vec![0.0; FRAMES - 1]).unwrap();
    drums.sha256 = models::hash_file(&drums.path).unwrap();
    let error = manifest
        .validate_source(&f.source(), &models::hash_file(&f.source()).unwrap(), 0.2)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("exactly the source's decoded frame count"),
        "{error:#}"
    );
}

#[test]
fn cancellation_partial_output_and_source_mutation_never_publish_a_manifest() {
    let f = Fixture::new();
    let failed = pipeline::isolation_cached_with(
        &f.source(),
        &f.model(),
        &f.cache(),
        f.identity(),
        |_, _, stage| {
            fs::write(stage.join("drums.wav"), b"unfinished")?;
            anyhow::bail!("cancelled fixture operation")
        },
    );
    assert!(failed.unwrap_err().to_string().contains("cancelled"));
    assert_eq!(fs::read_dir(f.cache()).unwrap().count(), 0);
    let failed = pipeline::isolation_cached_with(
        &f.source(),
        &f.model(),
        &f.cache(),
        f.identity(),
        |audio, _, stage| {
            Fixture::write_stems(stage)?;
            fs::OpenOptions::new()
                .append(true)
                .open(audio)?
                .write_all(b"changed source")?;
            Ok(())
        },
    );
    assert!(failed.unwrap_err().to_string().contains("Source changed"));
    assert_eq!(fs::read_dir(f.cache()).unwrap().count(), 0);
}

#[test]
fn a_missing_requested_stem_is_rejected_before_any_transcription_model_lookup() {
    let f = Fixture::new();
    let mut manifest = f.prepare();
    manifest.generation = None;
    manifest.stems.retain(|s| s.id != StemId::Guitar);
    let path = f.root.path().join("partial-manifest.json");
    fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    let error = pipeline::transcribe_with_isolation(
        &f.source(),
        f.root.path(),
        IsolationOptions {
            manifest: Some(&path),
            ..Default::default()
        },
        &["drums".into(), "guitar".into()],
    )
    .unwrap_err();
    assert!(error.to_string().contains("no guitar stem"), "{error:#}");
}

#[test]
fn legacy_import_cli_and_export_share_the_review_manifest_without_invented_model_history() {
    let f = Fixture::new();
    let native = f.prepare();
    let stems = native.stems[0].path.parent().unwrap();
    let path = f.root.path().join("imported.json");
    let result = Command::new(env!("CARGO_BIN_EXE_chart-engine"))
        .args(["import-stems", "--audio"])
        .arg(f.source())
        .arg("--stems")
        .arg(stems)
        .arg("--output")
        .arg(&path)
        .args([
            "--alignment-method",
            "supplied_fixture",
            "--alignment-details",
            "Synthetic, unshifted six-stem impulses; no separation quality claim.",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let imported = pipeline::read_isolation(&f.source(), &path).unwrap();
    assert!(imported.generation.is_none());
    assert_eq!(imported.alignment.method, "supplied_fixture");
    let output = f.root.path().join("portable");
    pipeline::export_isolation(&f.source(), &imported, &output).unwrap();
    let portable =
        pipeline::read_isolation(&f.source(), &output.join(pipeline::STEM_MANIFEST)).unwrap();
    assert_eq!(portable.source_audio_sha256, imported.source_audio_sha256);
    assert_eq!(portable.alignment, imported.alignment);
    assert!(
        portable
            .stems
            .iter()
            .all(|s| s.path.starts_with(output.canonicalize().unwrap()))
    );
    assert!(pipeline::export_isolation(&f.source(), &imported, &output).is_err());
    let rejected = f.root.path().join("unattested.json");
    assert!(
        pipeline::import_isolation(
            &f.source(),
            stems,
            &rejected,
            AlignmentEvidence {
                method: String::new(),
                details: String::new()
            }
        )
        .is_err()
    );
    assert!(!rejected.exists());
}

#[test]
fn build_retains_source_binding_and_authored_notes_with_legacy_events_still_supported() {
    let f = Fixture::new();
    let isolation = f.prepare();
    let source = f.root.path().join("authored");
    fs::create_dir(&source).unwrap();
    let chart = "[Song]\n{\n  Resolution = 192\n}\n[SyncTrack]\n{\n  0 = B 120000\n}\n[Events]\n{\n}\n[ExpertSingle]\n{\n  0 = N 4 0\n  0 = N 5 0\n  12 = N 2 0\n  12 = N 6 0\n}\n";
    fs::write(source.join("notes.chart"), chart).unwrap();
    fs::copy(f.source(), source.join("song.wav")).unwrap();
    fs::write(
        source.join("song.ini"),
        "[song]\nname = Fixture\nartist = Test\n",
    )
    .unwrap();
    let events = Transcription {
        isolation: Some(isolation.clone()),
        drums: vec![DrumHit {
            time: 0.02,
            voice: Voice::Kick,
            confidence: 0.9,
        }],
        ..Default::default()
    };
    let output = f.root.path().join("enhanced");
    let report = pipeline::build(Build {
        audio: &f.source(),
        output: &output,
        existing: Some(&source),
        timing: None,
        transcription: &events,
        meta: Metadata {
            name: "Fixture".into(),
            artist: "Test".into(),
            duration: 0.2,
            ..Default::default()
        },
        pro: true,
        lookup_lyrics: false,
    })
    .unwrap();
    assert_eq!(
        report["isolation"]["source_audio_sha256"],
        isolation.source_audio_sha256
    );
    assert_eq!(report["status"], "automatic_draft");
    let after = Chart::parse(fs::read_to_string(output.join("notes.chart")).unwrap()).unwrap();
    assert_eq!(
        after.notes("ExpertSingle").unwrap(),
        Chart::parse(chart.to_owned())
            .unwrap()
            .notes("ExpertSingle")
            .unwrap()
    );
    assert_eq!(
        fs::read_to_string(source.join("notes.chart")).unwrap(),
        chart
    );
    let sidecar = pipeline::read_isolation(
        &output.join("song.wav"),
        &output.join("chart-engine-isolation.json"),
    )
    .unwrap();
    assert_eq!(sidecar, isolation);
    let review: chart_engine::review_regions::ReviewMap =
        serde_json::from_slice(&fs::read(output.join("chart-engine-review.json")).unwrap())
            .unwrap();
    assert!(
        review
            .coverage_limits
            .iter()
            .any(|limit| limit.contains("isolation purity"))
    );
    assert_eq!(review.musical_acceptance, "unverified");
    let old: Transcription =
        serde_json::from_str("{\"drums\":[],\"guitar\":[],\"bass\":[],\"warnings\":[]}").unwrap();
    assert!(old.isolation.is_none());
    assert_eq!(
        mapping_tracks(&events),
        mapping_tracks(&Transcription {
            isolation: None,
            ..events.clone()
        })
    );

    let wrong_audio = f.root.path().join("wrong.wav");
    models::write_audio(&wrong_audio, &vec![0.0; FRAMES], &vec![0.0; FRAMES]).unwrap();
    let rejected = f.root.path().join("rejected");
    assert!(
        pipeline::build(Build {
            audio: &wrong_audio,
            output: &rejected,
            existing: None,
            timing: Some(TempoMap::constant(120.0).unwrap()),
            transcription: &events,
            meta: Metadata {
                name: "Fixture".into(),
                artist: "Test".into(),
                duration: 0.2,
                ..Default::default()
            },
            pro: true,
            lookup_lyrics: false
        })
        .is_err()
    );
    assert!(!rejected.exists());
}

fn mapping_tracks(events: &Transcription) -> chart_engine::chart::Tracks {
    chart_engine::mapping::tracks(events, &TempoMap::constant(120.0).unwrap(), true).unwrap()
}
