use chart_engine::{
    chart::{Chart, Note},
    editor::{
        Action, Actor, ActorKind, Content, Decision, DecisionStatus, EditorNote, Project, Request,
        Store, TimeRange, Transport,
    },
    editor_validation::validate,
    review_regions::{Issue, Policy, Reason, ReviewMap},
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

type Mutation = Box<dyn Fn(&mut Project)>;

const SOURCE: &str = "[Song]\n{\n Resolution = 480\n Offset = -0.25\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n[ExpertDrums]\n{\n 0 = N 2 0\n 0 = N 66 0\n 480 = N 32 0\n}\n[ExpertSingle]\n{\n 480 = N 7 480\n 480 = N 5 0\n}\n";

fn fixture() -> (Project, Chart) {
    let chart = Chart::parse(SOURCE.into()).unwrap();
    let source_hash = format!("{:x}", Sha256::digest(SOURCE.as_bytes()));
    let audio_hash = "b".repeat(64);
    let mut issue = Issue::at(0.25, "ExpertSingle", Reason::LowCandidateScore, &[]);
    issue.end = 3.5;
    let review = ReviewMap::build(
        4.0,
        Policy {
            window_seconds: 2.0,
            ..Policy::default()
        },
        BTreeMap::from([
            ("chart".into(), source_hash.clone()),
            ("original_audio".into(), audio_hash.clone()),
        ]),
        Some(BTreeSet::from(["ExpertSingle".into()])),
        BTreeSet::from([Reason::LowCandidateScore]),
        vec![issue],
    )
    .unwrap();
    let tracks = ["ExpertDrums", "ExpertSingle"]
        .into_iter()
        .map(|track| {
            (
                track.into(),
                chart
                    .notes(track)
                    .unwrap()
                    .into_iter()
                    .enumerate()
                    .map(|(i, note)| EditorNote {
                        id: format!("{track}-{i}"),
                        note,
                    })
                    .collect(),
            )
        })
        .collect();
    let project = Project {
        version: 1,
        revision: 0,
        title: "Validation fixture".into(),
        original_folder: "/fixture".into(),
        source_chart_sha256: source_hash,
        audio_filename: "song.wav".into(),
        audio_sha256: audio_hash,
        duration: 4.0,
        review,
        content: Content {
            track_origins: Default::default(),
            difficulty_links: Default::default(),
            vocals: None,
            vocal_review: None,
            tracks,
            decisions: vec![],
            hidden_flags: Default::default(),
            stems: None,
            keyboard_stems: None,
            stem_mix: Default::default(),
        },
        selected_track: "ExpertSingle".into(),
        selected_region: None,
        selected_note: Some("ExpertSingle-0".into()),
        kit_audition: None,
        transport: Transport {
            playback_rate: 1.0,
            position: 0.0,
            playing: false,
            anchor_ms: 0,
            loop_range: None,
        },
        next_id: 1,
        undo: vec![],
        redo: vec![],
        audit: vec![],
    };
    (project, chart)
}

#[test]
fn accepts_preserved_controls_negative_offset_and_long_issue_windows() {
    let (project, chart) = fixture();
    validate(&project, &chart).unwrap();
    assert_eq!(project.review.regions.len(), 2);
    assert!(project.review.regions[1].issues[0].start < project.review.regions[1].start);
}

#[test]
fn keyboard_review_keeps_separate_sources_and_rejects_foreign_recordings() {
    use chart_engine::stem_review::{AlignmentEvidence, StemAsset, StemId, StemManifest};
    let (mut project, chart) = fixture();
    let manifest = |id, label: &str| StemManifest {
        version: 1,
        source_audio_sha256: project.audio_sha256.clone(),
        sample_rate: 44100,
        channels: 2,
        frames: 176400,
        alignment: AlignmentEvidence {
            method: label.into(),
            details: "Metadata-only source binding test; no acoustic claim".into(),
        },
        stems: vec![StemAsset {
            id,
            path: format!("/fixture/{label}.wav").into(),
            sha256: "c".repeat(64),
        }],
        generation: None,
    };
    project.content.stems = Some(manifest(StemId::Guitar, "primary"));
    project.content.keyboard_stems = Some(manifest(StemId::Piano, "keyboard"));
    validate(&project, &chart).unwrap();
    assert_eq!(
        project
            .content
            .audition_stems("ExpertSingle")
            .unwrap()
            .alignment
            .method,
        "primary"
    );
    assert_eq!(
        project
            .content
            .audition_stems("ExpertProKeys")
            .unwrap()
            .alignment
            .method,
        "keyboard"
    );
    assert_eq!(
        project.content.stem_ids(),
        BTreeSet::from([StemId::Guitar, StemId::Piano])
    );
    let saved = serde_json::to_value(&project).unwrap();
    let restored: Project = serde_json::from_value(saved.clone()).unwrap();
    validate(&restored, &chart).unwrap();
    let mut legacy = saved;
    legacy["content"]
        .as_object_mut()
        .unwrap()
        .remove("keyboard_stems");
    let legacy: Project = serde_json::from_value(legacy).unwrap();
    assert_eq!(
        legacy
            .content
            .audition_stems("ExpertProKeys")
            .unwrap()
            .alignment
            .method,
        "primary"
    );
    project
        .content
        .keyboard_stems
        .as_mut()
        .unwrap()
        .source_audio_sha256 = "d".repeat(64);
    assert!(validate(&project, &chart).is_err());
}

#[test]
fn supported_keyboard_source_sections_cannot_be_silently_omitted() {
    for (track, lane) in [("ExpertKeyboard", 2), ("ExpertProKeys", 60)] {
        let (mut project, original) = fixture();
        let raw = format!("{}[{track}]\n{{\n 480 = N {lane} 240\n}}\n", original.raw);
        let chart = Chart::parse(raw).unwrap();
        project.source_chart_sha256 = format!("{:x}", Sha256::digest(chart.raw.as_bytes()));
        project
            .review
            .input_sha256
            .insert("chart".into(), project.source_chart_sha256.clone());
        assert!(
            validate(&project, &chart).is_err(),
            "missing {track} must fail"
        );
        project.content.tracks.insert(
            track.into(),
            vec![EditorNote {
                id: format!("{track}-0"),
                note: chart.notes(track).unwrap().remove(0),
            }],
        );
        validate(&project, &chart).unwrap();
    }
}

#[test]
fn rejects_corrupt_transport_source_scope_and_note_identity() {
    let (base, chart) = fixture();
    let mutations: Vec<Mutation> = vec![
        Box::new(|p| p.duration = f64::NAN),
        Box::new(|p| p.transport.position = f64::INFINITY),
        Box::new(|p| p.transport.position = 4.1),
        Box::new(|p| {
            p.transport.loop_range = Some(TimeRange {
                start: 1.0,
                end: 1.0,
            })
        }),
        Box::new(|p| p.source_chart_sha256 = "a".repeat(64)),
        Box::new(|p| p.audio_filename = "..".into()),
        Box::new(|p| p.audio_filename = "/song.wav".into()),
        Box::new(|p| {
            p.review
                .input_sha256
                .insert("original_audio".into(), "c".repeat(64));
        }),
        Box::new(|p| p.selected_track = "ExpertMissing".into()),
        Box::new(|p| p.selected_note = Some("ExpertDrums-1".into())),
        Box::new(|p| {
            p.content.tracks.get_mut("ExpertSingle").unwrap()[0].id = "ExpertDrums-0".into()
        }),
        Box::new(|p| {
            p.content
                .tracks
                .get_mut("ExpertSingle")
                .unwrap()
                .push(EditorNote {
                    id: "new".into(),
                    note: Note {
                        tick: 480,
                        lane: 7,
                        sustain: 0,
                    },
                })
        }),
        Box::new(|p| {
            p.content.tracks.get_mut("ExpertSingle").unwrap()[0].note = Note {
                tick: u64::MAX,
                lane: 7,
                sustain: 1,
            }
        }),
        Box::new(|p| {
            p.content.tracks.remove("ExpertDrums");
        }),
    ];
    for mutate in mutations {
        let mut project = base.clone();
        mutate(&mut project);
        assert!(validate(&project, &chart).is_err());
    }
}

#[test]
fn rejects_invalid_review_ranges_evidence_scope_and_selection() {
    let (base, chart) = fixture();
    let mutations: Vec<Mutation> = vec![
        Box::new(|p| p.review.regions[0].start = -0.1),
        Box::new(|p| p.review.regions[0].end = 4.1),
        Box::new(|p| p.review.regions[0].end = p.review.regions[0].start),
        Box::new(|p| p.review.regions[1].id = p.review.regions[0].id.clone()),
        Box::new(|p| p.review.regions[0].tracks.clear()),
        Box::new(|p| p.review.regions[0].issues.clear()),
        Box::new(|p| p.review.regions[0].issues[0].track = "ExpertDrums".into()),
        Box::new(|p| {
            p.review.regions[0].issues[0]
                .evidence
                .insert("score".into(), f64::NAN);
        }),
        Box::new(|p| {
            p.review.regions[0].issues[0].start = 2.5;
            p.review.regions[0].issues[0].end = 3.0;
        }),
        Box::new(|p| p.selected_region = Some("absent".into())),
        Box::new(|p| {
            p.selected_track = "ExpertDrums".into();
            p.selected_note = None;
            p.selected_region = Some(p.review.regions[0].id.clone());
        }),
    ];
    for mutate in mutations {
        let mut project = base.clone();
        mutate(&mut project);
        assert!(validate(&project, &chart).is_err());
    }
}

fn add_guitar_decision(project: &mut Project, chart: &Chart) -> String {
    let region = project.review.regions[0].id.clone();
    project.content.decisions.push(Decision {
        region_id: region.clone(),
        track: "ExpertSingle".into(),
        status: DecisionStatus::Reviewed,
        actor: Actor {
            kind: ActorKind::Human,
            name: "Fixture reviewer".into(),
        },
        comment: "Checked the guitar figure against its recording".into(),
        chart_sha256: project.source_chart_sha256.clone(),
        region_content_sha256: project
            .region_fingerprint(&region, "ExpertSingle", &chart.timing)
            .unwrap(),
        revision: 1,
        at_ms: 1,
    });
    region
}

#[test]
fn decisions_apply_only_to_the_reviewed_track_and_local_content() {
    let (mut project, chart) = fixture();
    let mut issues = project.review.regions[0].issues.clone();
    issues.push(Issue::at(0.25, "ExpertDrums", Reason::FastRepeat, &[]));
    project.review = ReviewMap::build(
        project.duration,
        project.review.policy.clone(),
        project.review.input_sha256.clone(),
        project.review.generated_tracks.clone(),
        BTreeSet::from([Reason::LowCandidateScore, Reason::FastRepeat]),
        issues,
    )
    .unwrap();
    let region = add_guitar_decision(&mut project, &chart);
    assert!(
        project
            .decision(&region, "ExpertSingle", &chart.timing)
            .is_some()
    );
    assert!(
        project
            .decision(&region, "ExpertDrums", &chart.timing)
            .is_none()
    );

    // A different instrument in the same flagged window must not invalidate
    // the reviewed guitar, or inherit the guitar's decision.
    project.content.tracks.get_mut("ExpertDrums").unwrap()[0]
        .note
        .tick = 96;
    assert!(
        project
            .decision(&region, "ExpertSingle", &chart.timing)
            .is_some()
    );
    assert!(
        project
            .decision(&region, "ExpertDrums", &chart.timing)
            .is_none()
    );

    // A remote edit in the same instrument has independent review scope.
    project
        .content
        .tracks
        .get_mut("ExpertSingle")
        .unwrap()
        .push(EditorNote {
            id: "remote-note".into(),
            note: Note {
                tick: 3000,
                lane: 1,
                sustain: 0,
            },
        });
    assert!(
        project
            .decision(&region, "ExpertSingle", &chart.timing)
            .is_some()
    );
    project.content.tracks.get_mut("ExpertSingle").unwrap()[0]
        .note
        .tick = 600;
    assert!(
        project
            .decision(&region, "ExpertSingle", &chart.timing)
            .is_none()
    );
    assert_eq!(
        project.content.decisions.len(),
        1,
        "history remains available"
    );
}

#[test]
fn export_distinguishes_stale_history_from_current_decisions() {
    let (mut project, chart) = fixture();
    let root = tempfile::tempdir().unwrap();
    let store_path = root.path().join("project");
    std::fs::create_dir_all(store_path.join("source")).unwrap();
    std::fs::create_dir(store_path.join("checkpoints")).unwrap();
    std::fs::write(store_path.join("source/notes.chart"), SOURCE).unwrap();
    // Export verifies bytes and copies the recording; decoding is irrelevant
    // to this decision-history regression, so no audio tool/model is needed.
    let audio_bytes = b"opaque recording bytes for a hash/copy fixture";
    std::fs::write(store_path.join("source/song.wav"), audio_bytes).unwrap();
    project.audio_sha256 = format!("{:x}", Sha256::digest(audio_bytes));
    project
        .review
        .input_sha256
        .insert("original_audio".into(), project.audio_sha256.clone());
    let region = add_guitar_decision(&mut project, &chart);
    project.content.tracks.get_mut("ExpertSingle").unwrap()[0]
        .note
        .tick = 600;
    std::fs::write(
        store_path.join("project.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();
    let store = Store::new(&store_path);
    let output = root.path().join("export");
    store
        .apply(Request {
            expected_revision: 0,
            actor: Actor {
                kind: ActorKind::Ai,
                name: "Export test".into(),
            },
            action: Action::Export {
                output: output.clone(),
            },
        })
        .unwrap();
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output.join("editor-review.json")).unwrap()).unwrap();
    let current = report["current_decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["region_id"] == region && entry["track"] == "ExpertSingle")
        .unwrap();
    assert!(current["decision"].is_null());
    let historical = &report["decision_history"][0];
    assert_eq!(historical["applies_to_current_notes"], false);
    assert_eq!(report["musical_acceptance"], "unverified");
}
