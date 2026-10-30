use chart_engine::{
    chart::Chart,
    editor::{
        Action, Actor, ActorKind, Content, DecisionStatus, EditorNote, FlagScope, Project, Request,
        Store, Transport,
    },
    review_regions::{Issue, Policy, Reason, ReviewMap},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

const SOURCE: &str = "[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n[Events]\n{\n}\n[ExpertSingle]\n{\n 480 = N 0 0\n 2400 = N 1 0\n}\n[ExpertDrums]\n{\n 480 = N 1 0\n}\n";

fn fixture(root: &Path) -> Store {
    fs::create_dir(root.join("source")).unwrap();
    fs::create_dir(root.join("checkpoints")).unwrap();
    fs::write(root.join("source/notes.chart"), SOURCE).unwrap();
    let audio = b"opaque recording fixture; visibility uses no decoder";
    fs::write(root.join("source/song.ogg"), audio).unwrap();
    let source_hash = format!("{:x}", Sha256::digest(SOURCE.as_bytes()));
    let audio_hash = format!("{:x}", Sha256::digest(audio));
    let chart = Chart::parse(SOURCE.into()).unwrap();
    let tracks = ["ExpertSingle", "ExpertDrums"]
        .into_iter()
        .map(|track| {
            (
                track.into(),
                chart
                    .notes(track)
                    .unwrap()
                    .into_iter()
                    .map(|note| EditorNote {
                        id: format!("{track}-{}", note.tick),
                        note,
                    })
                    .collect(),
            )
        })
        .collect();
    let project = Project {
        version: 1,
        revision: 0,
        title: "Visibility fixture".into(),
        original_folder: root.join("original"),
        source_chart_sha256: source_hash.clone(),
        audio_filename: "song.ogg".into(),
        audio_sha256: audio_hash.clone(),
        duration: 4.0,
        review: ReviewMap::build(
            4.0,
            Policy {
                window_seconds: 2.0,
                ..Policy::default()
            },
            BTreeMap::from([
                ("chart".into(), source_hash),
                ("original_audio".into(), audio_hash),
            ]),
            None,
            BTreeSet::from([Reason::FastRepeat]),
            vec![
                Issue::at(0.5, "ExpertSingle", Reason::FastRepeat, &[]),
                Issue::at(0.5, "ExpertDrums", Reason::FastRepeat, &[]),
                Issue::at(2.5, "ExpertSingle", Reason::FastRepeat, &[]),
            ],
        )
        .unwrap(),
        content: Content {
            track_origins: Default::default(),
            difficulty_links: Default::default(),
            vocals: None,
            vocal_review: None,
            tracks,
            decisions: vec![],
            hidden_flags: BTreeSet::new(),
            stems: None,
            keyboard_stems: None,
            stem_mix: Default::default(),
        },
        selected_track: "ExpertSingle".into(),
        selected_region: None,
        selected_note: None,
        kit_audition: None,
        transport: Transport {
            position: 0.0,
            playing: false,
            anchor_ms: 0,
            loop_range: None,
            playback_rate: 1.0,
        },
        next_id: 1,
        undo: vec![],
        redo: vec![],
        audit: vec![],
    };
    save(&Store::new(root), &project);
    Store::new(root)
}

fn save(store: &Store, project: &Project) {
    fs::write(
        store.root.join("project.json"),
        serde_json::to_vec_pretty(project).unwrap(),
    )
    .unwrap();
}

fn command(store: &Store, action: Action) {
    store
        .apply(Request {
            expected_revision: store.load().unwrap().revision,
            actor: Actor {
                kind: ActorKind::Ai,
                name: "Visibility regression".into(),
            },
            action,
        })
        .unwrap();
}

fn visibility(region: &str, track: &str, hidden: bool) -> Action {
    Action::SetFlagVisibility {
        region_id: region.into(),
        track: track.into(),
        hidden,
    }
}

#[test]
fn hiding_is_scoped_durable_undoable_and_never_approval() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let before = store.load().unwrap();
    let region = before.review.regions[0].id.clone();
    let chart_hash = before.chart_hash(&store).unwrap();
    command(&store, visibility(&region, "ExpertSingle", true));
    let project = store.load().unwrap();
    assert!(project.is_flag_hidden(&region, "ExpertSingle"));
    assert!(!project.is_flag_hidden(&region, "ExpertDrums"));
    assert!(project.content.decisions.is_empty());
    assert_eq!(project.chart_hash(&store).unwrap(), chart_hash);
    assert_eq!(project.content.hidden_flags.len(), 1);
    assert!(matches!(
        project.audit.last().unwrap().action,
        Action::SetFlagVisibility { hidden: true, .. }
    ));
    assert_eq!(project.audit.last().unwrap().actor.kind, ActorKind::Ai);
    let inspection = store.inspect().unwrap();
    let current = inspection["current_decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|scope| scope["region_id"] == region && scope["track"] == "ExpertSingle")
        .unwrap();
    assert_eq!(current["hidden"], true);
    assert!(current["decision"].is_null());
    let scopes = inspection["octave_view"]["review_regions"][0]["scopes"]
        .as_array()
        .unwrap();
    assert_eq!(
        scopes
            .iter()
            .find(|scope| scope["track"] == "ExpertSingle")
            .unwrap()["hidden"],
        true
    );
    assert_eq!(
        scopes
            .iter()
            .find(|scope| scope["track"] == "ExpertDrums")
            .unwrap()["hidden"],
        false
    );
    let reopened = Store::new(root.path());
    assert!(
        reopened
            .load()
            .unwrap()
            .is_flag_hidden(&region, "ExpertSingle")
    );
    command(&reopened, Action::Undo);
    assert!(
        !reopened
            .load()
            .unwrap()
            .is_flag_hidden(&region, "ExpertSingle")
    );
    command(&reopened, Action::Redo);
    assert!(
        reopened
            .load()
            .unwrap()
            .is_flag_hidden(&region, "ExpertSingle")
    );
    command(&reopened, visibility(&region, "ExpertSingle", false));
    assert!(
        !reopened
            .load()
            .unwrap()
            .is_flag_hidden(&region, "ExpertSingle")
    );
    assert!(reopened.load().unwrap().content.decisions.is_empty());
    assert_eq!(
        fs::read_to_string(root.path().join("source/notes.chart")).unwrap(),
        SOURCE
    );
}

#[test]
fn hidden_state_does_not_change_approval_or_reappear_after_note_edits() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let region = store.load().unwrap().review.regions[0].id.clone();
    command(
        &store,
        Action::Decide {
            region_id: region.clone(),
            track: "ExpertSingle".into(),
            status: DecisionStatus::Reviewed,
            comment: "Compared the current guitar phrase against its recording".into(),
        },
    );
    command(&store, visibility(&region, "ExpertSingle", true));
    let project = store.load().unwrap();
    let timing = project.timing(&store).unwrap();
    assert_eq!(project.content.decisions.len(), 1);
    assert!(project.decision(&region, "ExpertSingle", &timing).is_some());
    command(
        &store,
        Action::UpdateNote {
            note_id: "ExpertSingle-480".into(),
            tick: 600,
            lane: 0,
            sustain: 0,
            cymbal: false,
        },
    );
    let project = store.load().unwrap();
    assert!(project.decision(&region, "ExpertSingle", &timing).is_none());
    assert!(project.is_flag_hidden(&region, "ExpertSingle"));
    assert!(!project.is_flag_hidden(&region, "ExpertDrums"));
    command(&store, Action::Undo);
    assert!(
        store
            .load()
            .unwrap()
            .is_flag_hidden(&region, "ExpertSingle")
    );
    let output = root.path().join("export");
    command(
        &store,
        Action::Export {
            output: output.clone(),
        },
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("editor-review.json")).unwrap()).unwrap();
    assert_eq!(report["hidden_flags"].as_array().unwrap().len(), 1);
    assert_eq!(report["musical_acceptance"], "unverified");
}

#[test]
fn invalid_or_stale_visibility_commands_are_atomic_and_idempotent_in_scope() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let first = store.load().unwrap().review.regions[0].id.clone();
    let second = store.load().unwrap().review.regions[1].id.clone();
    command(&store, visibility(&first, "ExpertSingle", true));
    command(&store, visibility(&first, "ExpertSingle", true));
    assert_eq!(store.load().unwrap().content.hidden_flags.len(), 1);
    let before = fs::read(root.path().join("project.json")).unwrap();
    for action in [
        visibility("unknown", "ExpertSingle", true),
        visibility(&second, "ExpertDrums", true),
        visibility(&first, "ExpertDoubleBass", true),
    ] {
        assert!(
            store
                .apply(Request {
                    expected_revision: store.load().unwrap().revision,
                    actor: Actor {
                        kind: ActorKind::Human,
                        name: "Test".into()
                    },
                    action
                })
                .is_err()
        );
        assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
    }
    assert!(
        store
            .apply(Request {
                expected_revision: 0,
                actor: Actor {
                    kind: ActorKind::Ai,
                    name: "Stale client".into()
                },
                action: visibility(&first, "ExpertSingle", false)
            })
            .is_err()
    );
    assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
}

#[test]
fn a_new_region_id_does_not_inherit_an_old_hide() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let old = store.load().unwrap().review.regions[0].id.clone();
    command(&store, visibility(&old, "ExpertSingle", true));
    let mut project = store.load().unwrap();
    let issues = project
        .review
        .regions
        .iter()
        .flat_map(|region| region.issues.clone())
        .collect();
    let mut policy = project.review.policy.clone();
    policy.low_candidate_score_below = 0.4;
    project.review = ReviewMap::build(
        project.duration,
        policy,
        project.review.input_sha256.clone(),
        project.review.generated_tracks.clone(),
        project.review.checks_performed.clone(),
        issues,
    )
    .unwrap();
    let new_id = project.review.regions[0].id.clone();
    assert_ne!(old, new_id);
    save(&store, &project);
    let reopened = store.load().unwrap();
    assert!(!reopened.is_flag_hidden(&new_id, "ExpertSingle"));
    assert!(!reopened.is_flag_hidden(&old, "ExpertSingle"));
    assert!(reopened.content.decisions.is_empty());
}

#[test]
fn old_projects_and_checkpoints_default_to_visible_flags() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let mut value = serde_json::to_value(store.load().unwrap()).unwrap();
    value["content"]
        .as_object_mut()
        .unwrap()
        .remove("hidden_flags");
    let old_content = serde_json::to_vec(&value["content"]).unwrap();
    let checkpoint = format!("{:x}", Sha256::digest(&old_content));
    fs::write(
        root.path()
            .join("checkpoints")
            .join(format!("{checkpoint}.json")),
        old_content,
    )
    .unwrap();
    value["undo"] = serde_json::json!([checkpoint]);
    fs::write(
        root.path().join("project.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert!(store.load().unwrap().content.hidden_flags.is_empty());
    command(&store, Action::Undo);
    assert!(store.load().unwrap().content.hidden_flags.is_empty());
    assert_eq!(store.load().unwrap().chart_text(&store).unwrap(), SOURCE);
    let mut invalid = store.load().unwrap();
    invalid.content.hidden_flags.insert(FlagScope {
        region_id: String::new(),
        track: "ExpertSingle".into(),
    });
    assert!(
        chart_engine::editor_validation::validate(&invalid, &Chart::parse(SOURCE.into()).unwrap())
            .is_err()
    );
}

fn vocal_fixture(root: &Path) -> Store {
    let store = fixture(root);
    let mut doc = store.load().unwrap();
    doc.duration = 26.;
    doc.review.duration_seconds = 26.;
    save(&store, &doc);
    store
}
fn vocal_report(store: &Store) -> chart_engine::vocal_generation::Generation {
    use chart_engine::{vocal_generation as vg, vocals::*};
    vg::Generation {
        version: 1,
        audio_sha256: store.load().unwrap().audio_sha256,
        vocal_stem_sha256: "b".repeat(64),
        duration_seconds: 26.,
        timing: Some(store.load().unwrap().timing(store).unwrap()),
        algorithm: "vocal-review-fixture-v1".into(),
        policy: vg::Policy::default(),
        status: vg::Status::ReviewRequired,
        chart: Some(VocalChart {
            version: 1,
            resolution: 480,
            source: "original test syllables".into(),
            phrases: vec![
                VocalPhrase {
                    start: 0,
                    end: 22080,
                    player: 1,
                },
                VocalPhrase {
                    start: 24000,
                    end: 24960,
                    player: 1,
                },
            ],
            notes: vec![
                VocalNote {
                    start: 0,
                    end: 240,
                    tone: VocalTone::Pitched { midi: 60 },
                    lyric: Syllable::Text {
                        text: "start".into(),
                        join_next: false,
                    },
                },
                VocalNote {
                    start: 10560,
                    end: 10800,
                    tone: VocalTone::Pitched { midi: 60 },
                    lyric: Syllable::Text {
                        text: "word".into(),
                        join_next: false,
                    },
                },
                VocalNote {
                    start: 12480,
                    end: 12960,
                    tone: VocalTone::Pitched { midi: 62 },
                    lyric: Syllable::Continue,
                },
                VocalNote {
                    start: 24000,
                    end: 24480,
                    tone: VocalTone::Pitched { midi: 64 },
                    lyric: Syllable::Text {
                        text: "last".into(),
                        join_next: false,
                    },
                },
            ],
        }),
        pitch_candidates: vec![],
        pitch_frames: vec![],
        neural_pitch: None,
        vocal_pitch: None,
        words: None,
        lyrics: None,
        issues: vec![
            vg::Issue {
                start_seconds: 13.2,
                end_seconds: 13.4,
                reason: vg::Reason::PitchDisagreement,
                detail: "Review continued word".into(),
            },
            vg::Issue {
                start_seconds: 25.,
                end_seconds: 25.5,
                reason: vg::Reason::PhraseBoundary,
                detail: "Review phrase ending".into(),
            },
        ],
        warnings: vec![],
    }
}
fn install_vocal_report(store: &Store, report: &chart_engine::vocal_generation::Generation) {
    command(
        store,
        Action::SetVocalGeneration {
            document: report.chart.clone().unwrap(),
            review: chart_engine::vocal_review::VocalReviewMap::from_generation(report).unwrap(),
        },
    );
}
fn approve_vocal(store: &Store, id: &str) {
    command(
        store,
        Action::Decide {
            region_id: id.into(),
            track: "Vocals".into(),
            status: DecisionStatus::Reviewed,
            comment: "Compared vocal word and pitch against this source section".into(),
        },
    );
}
#[test]
fn vocal_decisions_hide_persist_export_and_undo_independently() {
    let root = tempfile::tempdir().unwrap();
    let store = vocal_fixture(root.path());
    install_vocal_report(&store, &vocal_report(&store));
    let id = store.load().unwrap().content.vocal_review.unwrap().regions[0]
        .id
        .clone();
    command(&store, visibility(&id, "Vocals", true));
    assert!(store.load().unwrap().content.decisions.is_empty());
    approve_vocal(&store, &id);
    let reopened = Store::new(root.path());
    let doc = reopened.load().unwrap();
    let timing = doc.timing(&reopened).unwrap();
    assert!(doc.is_flag_hidden(&id, "Vocals"));
    let decision = doc.decision(&id, "Vocals", &timing).unwrap();
    assert_eq!(decision.actor.kind, ActorKind::Ai);
    assert_eq!(decision.status, DecisionStatus::Reviewed);
    let inspected = reopened.inspect().unwrap();
    let scope = inspected["current_decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|scope| scope["region_id"] == id)
        .unwrap();
    assert_eq!(scope["hidden"], true);
    assert_eq!(scope["decision"]["status"], "reviewed");
    command(&reopened, Action::Undo);
    let doc = reopened.load().unwrap();
    assert!(doc.is_flag_hidden(&id, "Vocals"));
    assert!(doc.decision(&id, "Vocals", &timing).is_none());
    command(&reopened, Action::Redo);
    command(&reopened, visibility(&id, "Vocals", false));
    let doc = reopened.load().unwrap();
    assert!(!doc.is_flag_hidden(&id, "Vocals"));
    assert!(doc.decision(&id, "Vocals", &timing).is_some());
    let output = root.path().join("vocal-export");
    command(
        &reopened,
        Action::Export {
            output: output.clone(),
        },
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("editor-review.json")).unwrap()).unwrap();
    assert_eq!(
        report["vocal_review"]["regions"].as_array().unwrap().len(),
        2
    );
    assert!(
        report["current_decisions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|scope| scope["track"] == "Vocals" && scope["decision"]["status"] == "reviewed")
    );
}
#[test]
fn vocal_approval_tracks_continuation_owner_but_not_unrelated_words() {
    use chart_engine::vocals::Syllable;
    let root = tempfile::tempdir().unwrap();
    let store = vocal_fixture(root.path());
    install_vocal_report(&store, &vocal_report(&store));
    let doc = store.load().unwrap();
    let timing = doc.timing(&store).unwrap();
    let regions = doc.content.vocal_review.unwrap().regions;
    approve_vocal(&store, &regions[0].id);
    approve_vocal(&store, &regions[1].id);
    let mut chart = store.load().unwrap().content.vocals.unwrap();
    chart.notes[0].lyric = Syllable::Text {
        text: "unrelated".into(),
        join_next: false,
    };
    command(
        &store,
        Action::SetVocals {
            document: chart.clone(),
        },
    );
    assert!(
        store
            .load()
            .unwrap()
            .decision(&regions[0].id, "Vocals", &timing)
            .is_some()
    );
    chart.notes[1].lyric = Syllable::Text {
        text: "changed".into(),
        join_next: false,
    };
    command(&store, Action::SetVocals { document: chart });
    let doc = store.load().unwrap();
    assert!(doc.decision(&regions[0].id, "Vocals", &timing).is_none());
    assert!(doc.decision(&regions[1].id, "Vocals", &timing).is_some());
    approve_vocal(&store, &regions[0].id);
    assert!(
        store
            .load()
            .unwrap()
            .decision(&regions[0].id, "Vocals", &timing)
            .is_some()
    );
    command(&store, Action::Undo);
    assert!(
        store
            .load()
            .unwrap()
            .decision(&regions[0].id, "Vocals", &timing)
            .is_none()
    );
    command(&store, Action::Undo);
    assert!(
        store
            .load()
            .unwrap()
            .decision(&regions[0].id, "Vocals", &timing)
            .is_some()
    );
}

#[test]
fn missing_lyrics_do_not_borrow_review_ownership_from_previous_word() {
    use chart_engine::vocals::Syllable;
    let root = tempfile::tempdir().unwrap();
    let store = vocal_fixture(root.path());
    let mut report = vocal_report(&store);
    report.chart.as_mut().unwrap().notes[2].lyric = Syllable::Missing;
    install_vocal_report(&store, &report);
    let project = store.load().unwrap();
    let map = project.content.vocal_review.as_ref().unwrap();
    let timing = project.timing(&store).unwrap();
    let id = &map.regions[0].id;
    let before = map.fingerprint(&project, id, &timing).unwrap();
    let mut changed = project.clone();
    changed.content.vocals.as_mut().unwrap().notes[1].lyric = Syllable::Text {
        text: "unrelated".into(),
        join_next: false,
    };
    assert_eq!(before, map.fingerprint(&changed, id, &timing).unwrap());
    changed.content.vocals.as_mut().unwrap().notes[2].tone =
        chart_engine::vocals::VocalTone::Pitched { midi: 63 };
    assert_ne!(before, map.fingerprint(&changed, id, &timing).unwrap());
}

#[test]
fn independent_word_continuation_keeps_owner_bound_to_review_outside_section() {
    use chart_engine::vocals::Syllable;
    let root = tempfile::tempdir().unwrap();
    let store = vocal_fixture(root.path());
    let mut report = vocal_report(&store);
    report.chart.as_mut().unwrap().notes[2].lyric = Syllable::ContinueWord;
    install_vocal_report(&store, &report);
    let project = store.load().unwrap();
    let map = project.content.vocal_review.as_ref().unwrap();
    let timing = project.timing(&store).unwrap();
    let id = &map.regions[0].id;
    let before = map.fingerprint(&project, id, &timing).unwrap();
    let mut changed = project.clone();
    changed.content.vocals.as_mut().unwrap().notes[0].lyric = Syllable::Text {
        text: "independent".into(),
        join_next: false,
    };
    assert_eq!(before, map.fingerprint(&changed, id, &timing).unwrap());
    changed.content.vocals.as_mut().unwrap().notes[1].lyric = Syllable::Text {
        text: "owner".into(),
        join_next: false,
    };
    assert_ne!(before, map.fingerprint(&changed, id, &timing).unwrap());
}
#[test]
fn vocal_regeneration_does_not_inherit_flags_and_undo_restores_immutable_evidence() {
    let root = tempfile::tempdir().unwrap();
    let store = vocal_fixture(root.path());
    let mut report = vocal_report(&store);
    install_vocal_report(&store, &report);
    let old = store.load().unwrap().content.vocal_review.unwrap().regions[0]
        .id
        .clone();
    approve_vocal(&store, &old);
    command(&store, visibility(&old, "Vocals", true));
    report.algorithm = "vocal-review-fixture-v2".into();
    install_vocal_report(&store, &report);
    let doc = store.load().unwrap();
    let timing = doc.timing(&store).unwrap();
    let new = doc.content.vocal_review.as_ref().unwrap().regions[0]
        .id
        .clone();
    assert_ne!(old, new);
    assert!(!doc.is_flag_hidden(&new, "Vocals"));
    assert!(doc.decision(&new, "Vocals", &timing).is_none());
    assert!(!doc.is_flag_hidden(&old, "Vocals"));
    fs::write(
        root.path().join("vocal-generation.json"),
        b"sidecar replaced by a later worker",
    )
    .unwrap();
    command(&store, Action::Undo);
    let doc = store.load().unwrap();
    assert!(doc.decision(&old, "Vocals", &timing).is_some());
    assert!(doc.is_flag_hidden(&old, "Vocals"));
    command(&store, Action::Redo);
    assert!(
        store
            .load()
            .unwrap()
            .decision(&new, "Vocals", &timing)
            .is_none()
    );
}
#[test]
fn vocal_review_rejects_wrong_source_mutated_issue_stale_revision_and_chart_witness_atomically() {
    let root = tempfile::tempdir().unwrap();
    let store = vocal_fixture(root.path());
    let report = vocal_report(&store);
    install_vocal_report(&store, &report);
    let before = fs::read(root.path().join("project.json")).unwrap();
    let review = chart_engine::vocal_review::VocalReviewMap::from_generation(&report).unwrap();
    let mut wrong_source = review.clone();
    wrong_source.audio_sha256 = "c".repeat(64);
    let mut mutated_issue = review.clone();
    mutated_issue.regions[0].detail = "different issue under same ID".into();
    let mut wrong_chart = report.chart.clone().unwrap();
    wrong_chart.notes[0].tone = chart_engine::vocals::VocalTone::Pitched { midi: 61 };
    for action in [
        Action::SetVocalReview {
            review: wrong_source,
        },
        Action::SetVocalReview {
            review: mutated_issue,
        },
        Action::SetVocalGeneration {
            document: wrong_chart,
            review: review.clone(),
        },
        visibility("unknown", "Vocals", true),
    ] {
        assert!(
            store
                .apply(Request {
                    expected_revision: store.load().unwrap().revision,
                    actor: Actor {
                        kind: ActorKind::Ai,
                        name: "Invalid test".into()
                    },
                    action
                })
                .is_err()
        );
        assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
    }
    assert!(
        store
            .apply(Request {
                expected_revision: 0,
                actor: Actor {
                    kind: ActorKind::Ai,
                    name: "Stale test".into()
                },
                action: Action::SetVocalReview { review }
            })
            .is_err()
    );
    assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
}

#[test]
fn vocal_stale_stem_blocks_approval_but_keeps_visibility_and_jump_independent() {
    use chart_engine::stem_review::{AlignmentEvidence, StemAsset, StemId, StemManifest};
    let root = tempfile::tempdir().unwrap();
    let store = vocal_fixture(root.path());
    install_vocal_report(&store, &vocal_report(&store));
    let mut doc = store.load().unwrap();
    let id = doc.content.vocal_review.as_ref().unwrap().regions[0]
        .id
        .clone();
    doc.content.stems = Some(StemManifest {
        version: 1,
        source_audio_sha256: doc.audio_sha256.clone(),
        sample_rate: 44100,
        channels: 2,
        frames: 26 * 44100,
        alignment: AlignmentEvidence {
            method: "fixture".into(),
            details: "Changed isolated vocal estimate".into(),
        },
        stems: vec![StemAsset {
            id: StemId::Vocals,
            path: root.path().join("replacement-vocals.wav"),
            sha256: "d".repeat(64),
        }],
        generation: None,
    });
    save(&store, &doc);
    let before = fs::read(root.path().join("project.json")).unwrap();
    assert!(
        store
            .apply(Request {
                expected_revision: doc.revision,
                actor: Actor {
                    kind: ActorKind::Ai,
                    name: "Changed stem".into()
                },
                action: Action::Decide {
                    region_id: id.clone(),
                    track: "Vocals".into(),
                    status: DecisionStatus::Reviewed,
                    comment: "Must not approve mismatched source".into()
                }
            })
            .is_err()
    );
    assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
    command(&store, visibility(&id, "Vocals", true));
    assert!(store.load().unwrap().is_flag_hidden(&id, "Vocals"));
    command(&store, visibility(&id, "Vocals", false));
    assert!(!store.load().unwrap().is_flag_hidden(&id, "Vocals"));
    command(
        &store,
        Action::SelectRegion {
            region_id: id.clone(),
        },
    );
    let selected = store.load().unwrap();
    assert_eq!(selected.selected_region, Some(id));
    assert_eq!(selected.selected_track, "ExpertSingle");
    assert_eq!(selected.transport.position, 12.);
    assert_eq!(
        selected.transport.loop_range,
        Some(chart_engine::editor::TimeRange {
            start: 12.,
            end: 24.
        })
    );
    assert!(selected.content.decisions.is_empty());
}

#[test]
fn vocal_sections_bound_review_work_and_retain_every_issue() {
    use chart_engine::vocal_generation::{Issue, Reason};
    let root = tempfile::tempdir().unwrap();
    let store = vocal_fixture(root.path());
    let mut report = vocal_report(&store);
    report.issues.push(Issue {
        start_seconds: 11.,
        end_seconds: 13.,
        reason: Reason::WordLevelAlignment,
        detail: "Cross-window evidence".into(),
    });
    report.issues.push(Issue {
        start_seconds: 24.,
        end_seconds: 24.,
        reason: Reason::PhraseBoundary,
        detail: "Boundary point".into(),
    });
    for index in 0..500 {
        report.issues.push(Issue {
            start_seconds: 13.2,
            end_seconds: 13.4,
            reason: Reason::PitchDisagreement,
            detail: format!("Detector detail {index}"),
        });
    }
    let map = chart_engine::vocal_review::VocalReviewMap::from_generation(&report).unwrap();
    assert_eq!(map.regions.len(), 3);
    assert!(map.regions.iter().all(|r| r.end - r.start <= 12.));
    for issue in &report.issues {
        assert!(
            map.regions
                .iter()
                .any(|r| r.issues.iter().any(|i| i.detail == issue.detail
                    && i.start_seconds == issue.start_seconds
                    && i.end_seconds == issue.end_seconds))
        );
    }
    assert_eq!(
        map.regions
            .iter()
            .filter(|r| r.issues.iter().any(|i| i.detail == "Cross-window evidence"))
            .count(),
        2
    );
    assert_eq!(
        map.regions
            .iter()
            .find(|r| r.issues.iter().any(|i| i.detail == "Boundary point"))
            .unwrap()
            .start,
        24.
    );
    let mut bad = report.clone();
    bad.issues[0].end_seconds = 27.;
    assert!(chart_engine::vocal_review::VocalReviewMap::from_generation(&bad).is_err());
}

#[test]
fn vocal_approval_follows_whole_hyphenated_word_across_section_boundary() {
    use chart_engine::vocals::Syllable;
    let root = tempfile::tempdir().unwrap();
    let store = vocal_fixture(root.path());
    let mut report = vocal_report(&store);
    let chart = report.chart.as_mut().unwrap();
    chart.notes[1].lyric = Syllable::Text {
        text: "own".into(),
        join_next: true,
    };
    chart.notes[2].lyric = Syllable::Text {
        text: "er".into(),
        join_next: false,
    };
    install_vocal_report(&store, &report);
    let doc = store.load().unwrap();
    let timing = doc.timing(&store).unwrap();
    let id = doc.content.vocal_review.unwrap().regions[0].id.clone();
    approve_vocal(&store, &id);
    let mut chart = store.load().unwrap().content.vocals.unwrap();
    chart.notes[0].lyric = Syllable::Text {
        text: "separate".into(),
        join_next: false,
    };
    command(
        &store,
        Action::SetVocals {
            document: chart.clone(),
        },
    );
    assert!(
        store
            .load()
            .unwrap()
            .decision(&id, "Vocals", &timing)
            .is_some()
    );
    chart.notes[1].lyric = Syllable::Text {
        text: "oth".into(),
        join_next: true,
    };
    command(&store, Action::SetVocals { document: chart });
    assert!(
        store
            .load()
            .unwrap()
            .decision(&id, "Vocals", &timing)
            .is_none()
    );
    command(&store, Action::Undo);
    assert!(
        store
            .load()
            .unwrap()
            .decision(&id, "Vocals", &timing)
            .is_some()
    );
}

#[test]
fn vocal_review_binds_exact_tempo_and_offset_but_allows_explicit_legacy_adoption() {
    let root = tempfile::tempdir().unwrap();
    let store = vocal_fixture(root.path());
    let report = vocal_report(&store);
    install_vocal_report(&store, &report);
    let before = fs::read(root.path().join("project.json")).unwrap();
    for wrong_offset in [false, true] {
        let mut wrong = report.clone();
        let timing = wrong.timing.as_mut().unwrap();
        assert_eq!(timing.resolution, 480);
        if wrong_offset {
            timing.offset += 0.02;
        } else {
            timing.tempos[0].bpm = 121.;
        }
        let review = chart_engine::vocal_review::VocalReviewMap::from_generation(&wrong).unwrap();
        for action in [
            Action::SetVocalGeneration {
                document: wrong.chart.clone().unwrap(),
                review: review.clone(),
            },
            Action::SetVocalReview { review },
        ] {
            assert!(
                store
                    .apply(Request {
                        expected_revision: store.load().unwrap().revision,
                        actor: Actor {
                            kind: ActorKind::Ai,
                            name: "Mismatched timing".into()
                        },
                        action
                    })
                    .is_err()
            );
            assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
        }
    }
    let mut legacy = serde_json::to_value(&report).unwrap();
    legacy.as_object_mut().unwrap().remove("timing");
    let legacy: chart_engine::vocal_generation::Generation =
        serde_json::from_value(legacy).unwrap();
    assert!(legacy.timing.is_none());
    let review = chart_engine::vocal_review::VocalReviewMap::from_generation(&legacy).unwrap();
    assert!(
        store
            .apply(Request {
                expected_revision: store.load().unwrap().revision,
                actor: Actor {
                    kind: ActorKind::Ai,
                    name: "Unwitnessed automatic chart".into()
                },
                action: Action::SetVocalGeneration {
                    document: legacy.chart.clone().unwrap(),
                    review: review.clone()
                }
            })
            .is_err()
    );
    assert_eq!(fs::read(root.path().join("project.json")).unwrap(), before);
    command(&store, Action::SetVocalReview { review });
    assert!(
        store
            .load()
            .unwrap()
            .content
            .vocal_review
            .unwrap()
            .timing
            .is_none()
    );
}
