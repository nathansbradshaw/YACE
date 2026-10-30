use chart_engine::{
    chart::{self, Note},
    editor::Store,
    mapping::{PitchNote, Transcription},
    models,
    part_analysis::{self, FindingStage},
    stem_review::{AlignmentEvidence, StemId, StemManifest},
    timing::TempoMap,
};
use std::{collections::BTreeMap, fs, path::Path};

fn wave(path: &Path) {
    let rate = 44100u32;
    let frames = rate * 3;
    let mut bytes = Vec::new();
    bytes.extend(b"RIFF");
    bytes.extend((frames * 8 + 36).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(3u16.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(rate.to_le_bytes());
    bytes.extend((rate * 8).to_le_bytes());
    bytes.extend(8u16.to_le_bytes());
    bytes.extend(32u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend((frames * 8).to_le_bytes());
    for i in 0..frames {
        let t = i as f64 / rate as f64;
        let elapsed = t - 1.;
        let v = if (0.0..0.4).contains(&elapsed) {
            (std::f64::consts::TAU * 220. * t).sin() * (-elapsed * 12.).exp() * 0.3
        } else {
            0.
        } as f32;
        bytes.extend(v.to_le_bytes());
        bytes.extend(v.to_le_bytes());
    }
    fs::write(path, bytes).unwrap();
}
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    wave(&root.path().join("song.wav"));
    fs::copy(root.path().join("song.wav"), root.path().join("guitar.wav")).unwrap();
    let manifest = StemManifest::build(
        &root.path().join("song.wav"),
        BTreeMap::from([(StemId::Guitar, root.path().join("guitar.wav"))]),
        AlignmentEvidence {
            method: "synthetic".into(),
            details: "Exact original PCM copy on the same clock".into(),
        },
    )
    .unwrap();
    fs::write(
        root.path().join("stems.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let chart = chart::new_chart(
        "Fixture",
        "YACE",
        &TempoMap::constant(120.).unwrap(),
        &BTreeMap::from([(
            "ExpertSingle".into(),
            vec![
                Note {
                    tick: 960,
                    lane: 0,
                    sustain: 0,
                },
                Note {
                    tick: 960,
                    lane: 1,
                    sustain: 0,
                },
            ],
        )]),
    );
    fs::write(root.path().join("notes.chart"), chart).unwrap();
    fs::write(
        root.path().join("song.ini"),
        "[song]\nname=Fixture\nartist=YACE\n",
    )
    .unwrap();
    root
}

#[test]
fn unavailable_evidence_is_not_zero_and_chords_are_grouped() {
    let f = fixture();
    let p = f.path();
    let r = part_analysis::analyze_files(&p.join("song.wav"), &p.join("notes.chart"), None, None)
        .unwrap();
    let guitar = &r.parts[0];
    assert!(!guitar.source_available);
    assert_eq!(guitar.counts.chart_attacks, 1);
    assert_eq!(guitar.counts.audio_attacks, None);
    assert_eq!(guitar.counts.saved_hypotheses, None);
    assert_eq!(guitar.counts.retained_attacks, None);
    assert!(guitar.sections.iter().all(|s| s.findings.is_empty()));
}

#[test]
fn authored_section_at_zero_is_retained_and_boundaries_do_not_double_count() {
    let f = fixture();
    let p = f.path();
    let mut chart = fs::read_to_string(p.join("notes.chart")).unwrap();
    let parsed = chart::Chart::parse(chart.clone()).unwrap();
    let section = parsed.sections.iter().find(|s| s.name == "Events").unwrap();
    chart.replace_range(
        section.body_start..section.body_end,
        "\n  0 = E \"section Intro\"\n  960 = E \"section Verse\"\n",
    );
    fs::write(p.join("notes.chart"), chart).unwrap();
    let report =
        part_analysis::analyze_files(&p.join("song.wav"), &p.join("notes.chart"), None, None)
            .unwrap();
    let rows = &report.parts[0].sections;
    assert_eq!(rows[0].label, "Intro");
    assert_eq!(rows[1].label, "Verse");
    assert_eq!(rows[0].counts.chart_attacks, 0);
    assert_eq!(rows[1].counts.chart_attacks, 1);
}

#[test]
fn current_chart_and_retained_event_differences_are_local_read_only_prompts() {
    let f = fixture();
    let p = f.path();
    let events = Transcription {
        guitar: vec![
            PitchNote {
                start: 1.,
                end: 1.1,
                pitch: 60,
                confidence: 0.8,
            },
            PitchNote {
                start: 2.,
                end: 2.1,
                pitch: 64,
                confidence: 0.8,
            },
        ],
        ..Default::default()
    };
    let event_path = p.join("events.json");
    fs::write(&event_path, serde_json::to_vec(&events).unwrap()).unwrap();
    let before = models::hash_file(&p.join("notes.chart")).unwrap();
    let r = part_analysis::analyze_files(
        &p.join("song.wav"),
        &p.join("notes.chart"),
        Some(&p.join("stems.json")),
        Some(&event_path),
    )
    .unwrap();
    let guitar = &r.parts[0];
    assert!(guitar.source_available);
    assert_eq!(guitar.counts.retained_attacks, Some(2));
    assert_eq!(
        guitar.counts.saved_hypotheses, None,
        "retained notes cannot impersonate raw detector observations"
    );
    assert!(
        guitar
            .sections
            .iter()
            .flat_map(|s| &s.findings)
            .any(|f| f.stage == FindingStage::Mapping)
    );
    assert_eq!(models::hash_file(&p.join("notes.chart")).unwrap(), before);
    assert_eq!(
        guitar
            .sections
            .iter()
            .map(|s| s.counts.chart_attacks)
            .sum::<usize>(),
        guitar.counts.chart_attacks
    );
    assert!(
        r.parts[2].counts.audio_attacks.is_none(),
        "no drum stem is unknown even when another instrument is attached"
    );
}

#[test]
fn changed_stems_and_foreign_recording_receipts_fail_closed() {
    let f = fixture();
    let p = f.path();
    let mut manifest = StemManifest::read(&p.join("stems.json")).unwrap();
    manifest.source_audio_sha256 = "f".repeat(64);
    let events = Transcription {
        isolation: Some(manifest),
        ..Default::default()
    };
    fs::write(p.join("events.json"), serde_json::to_vec(&events).unwrap()).unwrap();
    assert!(
        part_analysis::analyze_files(
            &p.join("song.wav"),
            &p.join("notes.chart"),
            Some(&p.join("stems.json")),
            Some(&p.join("events.json"))
        )
        .is_err()
    );
    fs::write(p.join("guitar.wav"), "changed").unwrap();
    assert!(
        part_analysis::analyze_files(
            &p.join("song.wav"),
            &p.join("notes.chart"),
            Some(&p.join("stems.json")),
            None
        )
        .is_err()
    );
}

fn controlled_articulation_report(
    hits: Vec<chart_engine::mapping::DrumHit>,
    proposal_index: usize,
    remove_ride_head: bool,
) -> part_analysis::AnalysisReport {
    use chart_engine::{drum_articulation, mapping};
    let fixture = fixture();
    let path = fixture.path();
    let manifest = StemManifest::build(
        &path.join("song.wav"),
        BTreeMap::from([(StemId::Drums, path.join("guitar.wav"))]),
        AlignmentEvidence {
            method: "controlled-projection".into(),
            details:
                "Synthetic ownership/collision fixture, not model or separation quality evidence"
                    .into(),
        },
    )
    .unwrap();
    let mut evidence = drum_articulation::analyze(&vec![0.; 22050 * 3], &hits).unwrap();
    let proposal = evidence
        .iter_mut()
        .find(|e| e.hit_index == proposal_index)
        .unwrap();
    proposal.proposed = Some(mapping::Voice::Ride);
    proposal.map_proposal = true;
    proposal.reason = "synthetic_owned_projection_test".into();
    let source = drum_articulation::Source::from_manifest(&manifest).unwrap();
    let events = Transcription {
        drums: hits,
        drum_articulations: evidence,
        drum_articulation_source: Some(source),
        isolation: Some(manifest.clone()),
        ..Default::default()
    };
    events.validate().unwrap();
    let timing = TempoMap::constant(120.).unwrap();
    let mut tracks = mapping::tracks(&events, &timing, true).unwrap();
    if remove_ride_head {
        tracks
            .get_mut("ExpertDrums")
            .unwrap()
            .retain(|n| n.lane != 3 && n.lane != 67);
    }
    fs::write(
        path.join("notes.chart"),
        chart::new_chart("Fixture", "YACE", &timing, &tracks),
    )
    .unwrap();
    fs::write(
        path.join("stems.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(
        path.join("events.json"),
        serde_json::to_vec(&events).unwrap(),
    )
    .unwrap();
    part_analysis::analyze_files(
        &path.join("song.wav"),
        &path.join("notes.chart"),
        Some(&path.join("stems.json")),
        Some(&path.join("events.json")),
    )
    .unwrap()
}

#[test]
fn unresolved_cymbals_follow_applied_raw_ownership_and_actual_chart_heads() {
    use chart_engine::mapping::{DrumHit, Voice};
    let hit = |voice, confidence| DrumHit {
        time: 1.,
        voice,
        confidence,
    };
    let unresolved = |report: &part_analysis::AnalysisReport| {
        report.parts[2]
            .sections
            .iter()
            .flat_map(|s| &s.findings)
            .filter(|f| {
                f.stage == FindingStage::Articulation && f.detail.starts_with("Generic cymbal")
            })
            .map(|f| f.id.clone())
            .collect::<std::collections::BTreeSet<_>>()
    };
    let applied = controlled_articulation_report(vec![hit(Voice::Cymbal, 0.8)], 0, false);
    assert!(
        unresolved(&applied).is_empty(),
        "Played, owned, charted Ride proposal is applied"
    );
    let edited = controlled_articulation_report(vec![hit(Voice::Cymbal, 0.8)], 0, true);
    assert_eq!(
        unresolved(&edited).len(),
        1,
        "An authored head removal cannot be reported as applied"
    );
    let fallback = controlled_articulation_report(
        vec![
            hit(Voice::Snare, 0.95),
            hit(Voice::Tom, 0.6),
            hit(Voice::Cymbal, 0.8),
        ],
        2,
        false,
    );
    assert_eq!(
        unresolved(&fallback).len(),
        1,
        "Hand-cap/raw-survivor fallback keeps cymbal unresolved alongside the simultaneous tom"
    );
    let discarded = controlled_articulation_report(
        vec![hit(Voice::Cymbal, 0.95), hit(Voice::Cymbal, 0.8)],
        1,
        false,
    );
    assert_eq!(
        unresolved(&discarded).len(),
        2,
        "Same-time evidence for another discarded raw index cannot resolve either generic cymbal"
    );
}

#[test]
fn editor_snapshot_identity_ignores_transport_and_detects_note_edits() {
    let f = fixture();
    let p = f.path();
    let dest = tempfile::tempdir().unwrap();
    let output = dest.path().join("project");
    let store = Store::create(p, None, &output).unwrap();
    let doc = store.load().unwrap();
    let original = part_analysis::project_key(&doc).unwrap();
    let mut view = doc.clone();
    view.transport.position = 1.;
    view.selected_track = "ExpertDoubleBass".into();
    assert_eq!(original, part_analysis::project_key(&view).unwrap());
    let before = fs::read(store.root.join("project.json")).unwrap();
    let report = part_analysis::analyze_project(&store, &doc).unwrap();
    assert_eq!(report.project_key.as_ref(), Some(&original));
    assert_eq!(before, fs::read(store.root.join("project.json")).unwrap());
    view.content.tracks.get_mut("ExpertSingle").unwrap()[0]
        .note
        .tick += 10;
    assert_ne!(original, part_analysis::project_key(&view).unwrap());
}

#[test]
fn kit_audition_is_bounded_source_bound_and_survives_note_edits() {
    use chart_engine::{
        larsnet::{self, OutputReceipt, SeparationReceipt},
        part_analysis::{DrumPartFile, DrumPartSet},
        stem_review::StemAsset,
    };
    let f = fixture();
    let p = f.path();
    let dest = tempfile::tempdir().unwrap();
    let store = Store::create(p, None, &dest.path().join("project")).unwrap();
    let mut doc = store.load().unwrap();
    let mut manifest = StemManifest::read(&p.join("stems.json")).unwrap();
    let drum_hash = models::hash_file(&p.join("guitar.wav")).unwrap();
    manifest.stems.push(StemAsset {
        id: StemId::Drums,
        path: p.join("guitar.wav"),
        sha256: drum_hash.clone(),
    });
    doc.content.stems = Some(manifest);
    let directory = dest.path().join("kit");
    fs::create_dir(&directory).unwrap();
    fs::copy(p.join("guitar.wav"), directory.join("kick.wav")).unwrap();
    let output = OutputReceipt {
        part: "kick".into(),
        filename: "kick.wav".into(),
        frames: 132300,
        pcm_sha256: "0".repeat(64),
        wav_sha256: Some(drum_hash.clone()),
        rms: 0.1,
        peak: 0.3,
        mask_numeric: larsnet::MaskNumeric {
            raw_min: 0.,
            raw_max: 1.,
            clipped_values: 0,
        },
    };
    let mut set = DrumPartSet {
        source_key: part_analysis::kit_source_key(&doc).unwrap(),
        original_audio_sha256: doc.audio_sha256.clone(),
        drum_audio_sha256: drum_hash.clone(),
        directory: directory.clone(),
        parts: vec![DrumPartFile {
            id: "kick".into(),
            label: "Kick".into(),
            filename: "kick.wav".into(),
            sha256: drum_hash.clone(),
            frames: 132300,
        }],
        receipt: SeparationReceipt {
            version: 1,
            algorithm: larsnet::ALGORITHM.into(),
            implementation_sha256: larsnet::implementation_hash(),
            source_audio_sha256: Some(drum_hash),
            source_pcm_sha256: "0".repeat(64),
            sample_rate: 44100,
            channels: 2,
            frames: 132300,
            models: larsnet::MODELS
                .into_iter()
                .map(|(a, b)| (a.into(), b.into()))
                .collect(),
            weight_license: larsnet::WEIGHT_LICENSE.into(),
            outputs: vec![output],
            elapsed_seconds: 0.,
            warnings: vec!["Synthetic clip fixture; not a model-generated component".into()],
        },
    };
    let before = fs::read(store.root.join("project.json")).unwrap();
    doc.content.tracks.get_mut("ExpertSingle").unwrap()[0]
        .note
        .tick += 10;
    let clip = part_analysis::audition_clip(&store, &doc, &set, "kick", 0.95, 1.05).unwrap();
    fs::write(dest.path().join("clip.wav"), clip.wav).unwrap();
    let samples = models::decode(&dest.path().join("clip.wav"), 44100, 2).unwrap();
    assert_eq!(samples.len(), 8820);
    assert!(samples.iter().any(|v| v.abs() > 0.1));
    assert_eq!(before, fs::read(store.root.join("project.json")).unwrap());
    assert!(part_analysis::audition_clip(&store, &doc, &set, "kick", -1., 1.).is_err());
    assert!(part_analysis::audition_clip(&store, &doc, &set, "kick", 0., 21.).is_err());
    set.receipt.outputs[0].frames -= 1;
    assert!(part_analysis::audition_clip(&store, &doc, &set, "kick", 0.95, 1.05).is_err());
    set.receipt.outputs[0].frames += 1;
    let full_set = set.clone();
    let silence = vec![0.; 44100];
    let shortened = directory.join("shortened-fixture.wav");
    models::write_audio(&shortened, &silence, &silence).unwrap();
    // Explicit test-only corruption, preserving write_audio's production
    // refusal to overwrite an existing file.
    fs::rename(&shortened, directory.join("kick.wav")).unwrap();
    let short_hash = models::hash_file(&directory.join("kick.wav")).unwrap();
    set.parts[0].frames = 44100;
    set.parts[0].sha256 = short_hash.clone();
    set.receipt.frames = 44100;
    set.receipt.outputs[0].frames = 44100;
    set.receipt.outputs[0].wav_sha256 = Some(short_hash);
    let error = part_analysis::audition_clip(&store, &doc, &set, "kick", 0.95, 1.05)
        .err()
        .unwrap();
    assert!(
        error.to_string().contains("attached recording clock"),
        "Internally consistent short component must not silently truncate an original-clock clip"
    );
    set = full_set;
    fs::write(directory.join("kick.wav"), "stale").unwrap();
    assert!(part_analysis::audition_clip(&store, &doc, &set, "kick", 0.95, 1.05).is_err());
}

fn save_controlled_kit_cache(
    store: &Store,
    doc: &chart_engine::editor::Project,
    source: &Path,
    name: &str,
    modified: u64,
) -> part_analysis::DrumPartSet {
    use chart_engine::{
        larsnet,
        part_analysis::{DrumPartFile, DrumPartSet},
    };
    let cache = store.root.join("analysis").join(format!("kit-{name}"));
    let directory = cache.join("parts");
    fs::create_dir_all(&directory).unwrap();
    let directory = directory.canonicalize().unwrap();
    let frames = doc.content.stems.as_ref().unwrap().frames;
    let hash = models::hash_file(source).unwrap();
    let mut parts = Vec::new();
    let mut outputs = Vec::new();
    for (id, label) in [
        ("kick", "Kick"),
        ("snare", "Snare"),
        ("toms", "Toms"),
        ("hihat", "Hi-hat"),
        ("cymbals", "Cymbals"),
    ] {
        let filename = format!("{id}.wav");
        fs::copy(source, directory.join(&filename)).unwrap();
        parts.push(DrumPartFile {
            id: id.into(),
            label: label.into(),
            filename: filename.clone(),
            sha256: hash.clone(),
            frames,
        });
        outputs.push(larsnet::OutputReceipt {
            part: id.into(),
            filename,
            frames,
            pcm_sha256: "0".repeat(64),
            wav_sha256: Some(hash.clone()),
            rms: 0.1,
            peak: 0.3,
            mask_numeric: larsnet::MaskNumeric {
                raw_min: 0.,
                raw_max: 1.,
                clipped_values: 0,
            },
        });
    }
    let set = DrumPartSet {
        source_key: part_analysis::kit_source_key(doc).unwrap(),
        original_audio_sha256: doc.audio_sha256.clone(),
        drum_audio_sha256: hash.clone(),
        directory,
        parts,
        receipt: larsnet::SeparationReceipt {
            version: 1,
            algorithm: larsnet::ALGORITHM.into(),
            implementation_sha256: larsnet::implementation_hash(),
            source_audio_sha256: Some(hash),
            source_pcm_sha256: "0".repeat(64),
            sample_rate: 44100,
            channels: 2,
            frames,
            models: larsnet::MODELS
                .into_iter()
                .map(|(a, b)| (a.into(), b.into()))
                .collect(),
            weight_license: larsnet::WEIGHT_LICENSE.into(),
            outputs,
            elapsed_seconds: 0.,
            warnings: vec![
                "Controlled cache fixture, not neural separation quality evidence".into(),
            ],
        },
    };
    fs::write(
        set.directory.join(larsnet::RECEIPT_FILENAME),
        serde_json::to_vec(&set.receipt).unwrap(),
    )
    .unwrap();
    let json = cache.join("kit-parts.json");
    fs::write(&json, serde_json::to_vec(&set).unwrap()).unwrap();
    fs::File::options()
        .write(true)
        .open(json)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(modified)),
        )
        .unwrap();
    set
}

#[test]
fn durable_kit_cache_restores_newest_valid_and_falls_back_without_models_or_writes() {
    let f = fixture();
    let dest = tempfile::tempdir().unwrap();
    let store = Store::create(f.path(), None, &dest.path().join("project")).unwrap();
    let mut doc = store.load().unwrap();
    assert!(
        part_analysis::load_project_drum_parts(&store, &doc)
            .unwrap()
            .is_none()
    );
    let source = f.path().join("guitar.wav");
    doc.content.stems = Some(
        StemManifest::build(
            &store.audio_path(&doc).unwrap(),
            BTreeMap::from([(StemId::Drums, source.clone())]),
            AlignmentEvidence {
                method: "controlled-cache".into(),
                details: "Exact synthetic source-clock copy; no installed model required".into(),
            },
        )
        .unwrap(),
    );
    assert!(
        part_analysis::load_project_drum_parts(&store, &doc)
            .unwrap()
            .is_none()
    );
    let before = fs::read(store.root.join("project.json")).unwrap();
    let old = save_controlled_kit_cache(&store, &doc, &source, "old", 10);
    let newest = save_controlled_kit_cache(&store, &doc, &source, "new", 20);
    let stale = save_controlled_kit_cache(&store, &doc, &source, "stale", 30);
    let stale_json = stale.directory.parent().unwrap().join("kit-parts.json");
    let mut wrong = stale.clone();
    wrong.receipt.implementation_sha256 = "f".repeat(64);
    fs::write(&stale_json, serde_json::to_vec(&wrong).unwrap()).unwrap();
    let corrupt = store.root.join("analysis/kit-corrupt");
    fs::create_dir(&corrupt).unwrap();
    fs::write(corrupt.join("kit-parts.json"), b"{").unwrap();
    let large = store.root.join("analysis/kit-oversized");
    fs::create_dir(&large).unwrap();
    fs::write(large.join("kit-parts.json"), vec![b' '; 256 * 1024 + 1]).unwrap();
    // A newer internally consistent cached part must still match the complete
    // source clock/header, even if its hashes have been updated after truncation.
    let mut short = save_controlled_kit_cache(&store, &doc, &source, "short", 40);
    let kick = short.directory.join("kick.wav");
    let mut bytes = fs::read(&kick).unwrap();
    bytes.truncate(44 + 44100 * 8);
    bytes[4..8].copy_from_slice(&(36 + 44100u32 * 8).to_le_bytes());
    bytes[40..44].copy_from_slice(&(44100u32 * 8).to_le_bytes());
    fs::write(&kick, bytes).unwrap();
    let hash = models::hash_file(&kick).unwrap();
    short.parts[0].sha256 = hash.clone();
    short.receipt.outputs[0].wav_sha256 = Some(hash);
    fs::write(
        short
            .directory
            .join(chart_engine::larsnet::RECEIPT_FILENAME),
        serde_json::to_vec(&short.receipt).unwrap(),
    )
    .unwrap();
    fs::write(
        short.directory.parent().unwrap().join("kit-parts.json"),
        serde_json::to_vec(&short).unwrap(),
    )
    .unwrap();
    // Notes and transport do not change source-bound cached audio identity.
    doc.content.tracks.get_mut("ExpertSingle").unwrap()[0]
        .note
        .tick += 10;
    doc.transport.position = 1.;
    let restored = part_analysis::load_project_drum_parts(&store, &doc)
        .unwrap()
        .unwrap();
    assert_eq!(restored.directory, newest.directory);
    assert_eq!(restored.parts.len(), 5);
    assert_eq!(before, fs::read(store.root.join("project.json")).unwrap());
    // Any component (not only the currently auditioned kick) invalidates a set.
    fs::write(newest.directory.join("hihat.wav"), b"changed").unwrap();
    assert_eq!(
        part_analysis::load_project_drum_parts(&store, &doc)
            .unwrap()
            .unwrap()
            .directory,
        old.directory
    );
    fs::write(old.directory.join("cymbals.wav"), b"changed").unwrap();
    assert!(
        part_analysis::load_project_drum_parts(&store, &doc)
            .unwrap()
            .is_none()
    );
    fs::write(&source, b"current drum source changed").unwrap();
    let error = part_analysis::load_project_drum_parts(&store, &doc).unwrap_err();
    assert!(error.to_string().contains("Attached drum audio changed"));
}

fn kit_transport_fixture() -> (tempfile::TempDir, tempfile::TempDir, Store) {
    let source = fixture();
    let destination = tempfile::tempdir().unwrap();
    let store = Store::create(source.path(), None, &destination.path().join("project")).unwrap();
    let mut doc = store.load().unwrap();
    let drums = source.path().join("guitar.wav");
    doc.content.stems = Some(
        StemManifest::build(
            &store.audio_path(&doc).unwrap(),
            BTreeMap::from([(StemId::Drums, drums.clone())]),
            AlignmentEvidence {
                method: "controlled-kit-transport".into(),
                details: "Exact synthetic copy for shared source-clock controls".into(),
            },
        )
        .unwrap(),
    );
    fs::write(
        store.root.join("project.json"),
        serde_json::to_vec(&doc).unwrap(),
    )
    .unwrap();
    save_controlled_kit_cache(&store, &doc, &drums, "transport", 50);
    (source, destination, store)
}

#[test]
fn kit_selection_is_a_shared_persistent_listening_action_and_not_a_note_edit() {
    use chart_engine::editor::{Action, Actor, ActorKind, Request, TimeRange};
    let (_source, _destination, store) = kit_transport_fixture();
    let before = store.load().unwrap();
    let content = serde_json::to_value(&before.content).unwrap();
    let chart = before.chart_hash(&store).unwrap();
    let undo = before.undo.clone();
    let apply = |action| {
        let doc = store.load().unwrap();
        store
            .apply(Request {
                expected_revision: doc.revision,
                actor: Actor {
                    kind: ActorKind::Ai,
                    name: "kit transport test".into(),
                },
                action,
            })
            .unwrap();
        store.load().unwrap()
    };
    let selected = apply(Action::SetKitAudition {
        part: Some("kick".into()),
    });
    let kit = selected.kit_audition.clone().unwrap();
    assert_eq!(kit.part, "kick");
    assert_eq!(serde_json::to_value(&selected.content).unwrap(), content);
    assert_eq!(selected.chart_hash(&store).unwrap(), chart);
    assert_eq!(selected.undo, undo);
    assert_eq!(
        part_analysis::project_key(&selected).unwrap(),
        part_analysis::project_key(&before).unwrap()
    );
    let path = part_analysis::validated_kit_audition_path(&store, &selected, &kit).unwrap();
    assert!(path.ends_with("kick.wav"));
    let (_, pcm) = part_analysis::load_kit_audition(&store, &selected, &kit).unwrap();
    assert_eq!(pcm.len() as u64, kit.frames * 2);
    apply(Action::SetLoop {
        range: Some(TimeRange { start: 1., end: 2. }),
    });
    apply(Action::Seek { seconds: 1.25 });
    let slowed = apply(Action::SetPlaybackRate { rate: 0.5 });
    assert_eq!(slowed.kit_audition.as_ref(), Some(&kit));
    let playing = apply(Action::Playback { playing: true });
    assert_eq!(
        playing
            .transport
            .position_at(playing.transport.anchor_ms + 1000, playing.duration),
        1.75
    );
    assert_eq!(
        playing
            .transport
            .position_at(playing.transport.anchor_ms + 2000, playing.duration),
        1.25
    );
    let cleared = apply(Action::SetStemMix {
        mix: Default::default(),
    });
    assert!(cleared.kit_audition.is_none());
    assert_eq!(serde_json::to_value(&cleared.content).unwrap(), content);
    let old_revision = selected.revision;
    assert!(
        store
            .apply(Request {
                expected_revision: old_revision,
                actor: Actor {
                    kind: ActorKind::Ai,
                    name: "stale request".into()
                },
                action: Action::SetKitAudition {
                    part: Some("snare".into())
                }
            })
            .is_err()
    );
    assert!(
        store.load().unwrap().kit_audition.is_none(),
        "stale activation cannot replace the newer Original choice"
    );
    let selected = apply(Action::SetKitAudition {
        part: Some("snare".into()),
    });
    let kit = selected.kit_audition.clone().unwrap();
    let edited = apply(Action::AddNote {
        track: "ExpertSingle".into(),
        tick: 1920,
        lane: 2,
        sustain: 0,
        cymbal: false,
    });
    assert_eq!(edited.kit_audition.as_ref(), Some(&kit));
    assert!(part_analysis::validated_kit_audition_path(&store, &edited, &kit).is_ok());
    assert!(
        apply(Action::SetKitAudition { part: None })
            .kit_audition
            .is_none()
    );
}

#[test]
fn kit_audio_matches_the_same_source_clock_at_each_practice_rate_and_rejects_substitution() {
    let (_source, _destination, store) = kit_transport_fixture();
    let doc = store.load().unwrap();
    let selected = part_analysis::select_kit_audition(&store, &doc, "kick").unwrap();
    for rate in [0.5, 1., 1.5] {
        // The controlled component is a byte-identical copy of this original.
        // This exercises real atempo execution, not a fabricated duration field.
        let reference =
            models::decode_at_tempo(&store.audio_path(&doc).unwrap(), 44100, 2, rate).unwrap();
        let prepared =
            part_analysis::prepare_kit_audition(&store, &doc, &selected, rate, reference.len())
                .unwrap();
        assert_eq!(
            prepared, reference,
            "shared source-time PCM differs at {rate}x"
        );
    }
    assert!(
        part_analysis::prepare_kit_audition(
            &store,
            &doc,
            &selected,
            1.,
            selected.frames as usize * 2 + 2
        )
        .is_err()
    );
    assert!(part_analysis::prepare_kit_audition(&store, &doc, &selected, 0.5, 20).is_err());
    let mut fake = selected.clone();
    fake.audio_sha256 = "f".repeat(64);
    assert!(part_analysis::validated_kit_audition_path(&store, &doc, &fake).is_err());
    fake = selected.clone();
    fake.kit = store.root.parent().unwrap().join("kit-parts.json");
    fs::write(&fake.kit, fs::read(&selected.kit).unwrap()).unwrap();
    assert!(part_analysis::validated_kit_audition_path(&store, &doc, &fake).is_err());
    let path = part_analysis::validated_kit_audition_path(&store, &doc, &selected).unwrap();
    fs::write(path, b"replaced after selection").unwrap();
    assert!(part_analysis::load_kit_audition(&store, &doc, &selected).is_err());
}

#[test]
fn ai_kit_selection_uses_the_shared_action_and_binds_the_actual_listening_source() {
    use chart_engine::ai_harness::{self, ApplyRequest, Options};
    use chart_engine::editor::{Action, ActorKind};
    let (_source, _destination, store) = kit_transport_fixture();
    let original = store.load().unwrap();
    let content = serde_json::to_value(&original.content).unwrap();
    let apply = |packet: &ai_harness::Packet, action| {
        ai_harness::apply(
            &store,
            packet,
            ApplyRequest {
                packet_sha256: packet.packet_sha256.clone(),
                expected_revision: packet.revision,
                actor_name: "kit reviewer".into(),
                action,
            },
        )
    };
    let before = ai_harness::review(&store, Options::default()).unwrap();
    assert!(before.kit_audition.is_none());
    apply(
        &before,
        Action::SetKitAudition {
            part: Some("kick".into()),
        },
    )
    .unwrap();
    let packet = ai_harness::review(&store, Options::default()).unwrap();
    let kit = packet.kit_audition.as_ref().unwrap();
    assert_eq!(kit.selected.part, "kick");
    assert_eq!(kit.audio.instrument, "drums:kick");
    assert_eq!(kit.sample_rate, 44100);
    assert_eq!(
        kit.audio.sha256,
        models::hash_file(&kit.audio.path).unwrap()
    );
    assert_eq!(
        kit.selected.frames,
        original.content.stems.as_ref().unwrap().frames
    );
    let selected = store.load().unwrap();
    assert_eq!(serde_json::to_value(&selected.content).unwrap(), content);
    assert_eq!(selected.audit.last().unwrap().actor.kind, ActorKind::Ai);
    assert!(apply(&before, Action::Seek { seconds: 1. }).is_err());
    apply(&packet, Action::SetKitAudition { part: None }).unwrap();
    let cleared = ai_harness::review(&store, Options::default()).unwrap();
    assert!(cleared.kit_audition.is_none());
    apply(
        &cleared,
        Action::SetKitAudition {
            part: Some("snare".into()),
        },
    )
    .unwrap();
    let packet = ai_harness::review(&store, Options::default()).unwrap();
    let saved = fs::read(store.root.join("project.json")).unwrap();
    fs::write(
        &packet.kit_audition.as_ref().unwrap().audio.path,
        b"changed kit audio",
    )
    .unwrap();
    assert!(ai_harness::review(&store, Options::default()).is_err());
    assert!(apply(&packet, Action::Seek { seconds: 1. }).is_err());
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), saved);
}

#[test]
fn cli_and_human_core_share_exact_read_only_report_and_refuse_overwrite() {
    let f = fixture();
    let p = f.path();
    let output = p.join("analysis.json");
    let expected =
        part_analysis::analyze_files(&p.join("song.wav"), &p.join("notes.chart"), None, None)
            .unwrap();
    let invoke = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_chart-engine"))
            .args(["parts", "files", "--audio"])
            .arg(p.join("song.wav"))
            .arg("--chart")
            .arg(p.join("notes.chart"))
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap()
    };
    let command = invoke();
    assert!(
        command.status.success(),
        "{}",
        String::from_utf8_lossy(&command.stderr)
    );
    let actual: serde_json::Value = serde_json::from_slice(&command.stdout).unwrap();
    assert_eq!(actual, serde_json::to_value(expected).unwrap());
    assert_eq!(
        actual,
        serde_json::from_slice::<serde_json::Value>(&fs::read(&output).unwrap()).unwrap()
    );
    let first = fs::read(&output).unwrap();
    assert!(!invoke().status.success());
    assert_eq!(first, fs::read(&output).unwrap());
}
