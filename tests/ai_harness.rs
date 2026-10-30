use chart_engine::{
    chart::Chart,
    editor::{
        Action, Actor, ActorKind, Content, DecisionStatus, EditorNote, Project, Request, Store,
        Transport,
    },
    review_regions::{Issue, Policy, Reason, ReviewMap},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

const SOURCE: &str = "[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n[Events]\n{\n}\n[ExpertSingle]\n{\n 480 = N 0 0\n 500 = N 0 0\n 2400 = N 1 0\n}\n[ExpertDrums]\n{\n 480 = N 1 0\n}\n";

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
            Some(BTreeSet::from([
                "ExpertSingle".into(),
                "ExpertDrums".into(),
            ])),
            BTreeSet::from([Reason::FastRepeat]),
            vec![
                Issue::at(0.5, "ExpertSingle", Reason::FastRepeat, &[]),
                Issue::at(0.5, "ExpertDrums", Reason::FastRepeat, &[]),
                Issue::at(2.5, "ExpertSingle", Reason::FastRepeat, &[]),
            ],
        )
        .unwrap(),
        content: Content {
            sections: None,
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

use chart_engine::ai_harness::{self, ApplyRequest, Options, Packet};
fn packet(s: &Store) -> Packet {
    ai_harness::review(s, Options::default()).unwrap()
}
fn apply(s: &Store, p: &Packet, action: Action) -> anyhow::Result<chart_engine::editor::Applied> {
    ai_harness::apply(
        s,
        p,
        ApplyRequest {
            packet_sha256: p.packet_sha256.clone(),
            expected_revision: p.revision,
            actor_name: "AI reviewer".into(),
            action,
        },
    )
}
#[test]
fn deterministic_bounded_packets_include_all_tracks_and_current_rules() {
    let root = tempfile::tempdir().unwrap();
    let s = fixture(root.path());
    let before = fs::read(s.root.join("project.json")).unwrap();
    let a = packet(&s);
    assert_eq!(
        serde_json::to_vec(&a).unwrap(),
        serde_json::to_vec(&packet(&s)).unwrap()
    );
    assert_eq!(a.sections.len(), 3);
    let expert = ai_harness::review(
        &s,
        Options {
            track: Some("ExpertSingle".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(expert.total_eligible, 2);
    assert!(
        expert
            .sections
            .iter()
            .all(|section| section.track == "ExpertSingle")
    );
    assert!(
        ai_harness::review(
            &s,
            Options {
                track: Some("ExpertBass".into()),
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(a.sections.iter().any(|x| x.track == "ExpertDrums"));
    assert!(a.sections.iter().any(|x| !x.rule_findings.is_empty()));
    assert_eq!(before, fs::read(s.root.join("project.json")).unwrap());
    let first = ai_harness::review(
        &s,
        Options {
            limit: 1,
            note_limit: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(first.remaining, 2);
    let second = ai_harness::review(
        &s,
        Options {
            limit: 1,
            note_limit: 1,
            after: first.next_after,
            ..Default::default()
        },
    )
    .unwrap();
    assert_ne!(first.sections[0].id, second.sections[0].id);
    assert!(second.sections[0].omitted_notes > 0);
    assert!(serde_json::to_vec(&second).unwrap().len() < 16384);
    assert!(
        ai_harness::review(
            &s,
            Options {
                limit: 9,
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(
        ai_harness::review(
            &s,
            Options {
                context_seconds: f64::NAN,
                ..Default::default()
            }
        )
        .is_err()
    );
}
#[test]
fn reviewed_hidden_and_stale_flags_follow_shared_ledger() {
    let root = tempfile::tempdir().unwrap();
    let s = fixture(root.path());
    let p = packet(&s);
    let id = p
        .sections
        .iter()
        .find(|x| x.track == "ExpertSingle" && x.start < 1.)
        .unwrap()
        .region_id
        .clone();
    command(
        &s,
        Action::Decide {
            region_id: id.clone(),
            track: "ExpertSingle".into(),
            status: DecisionStatus::Reviewed,
            comment: "Listened to region".into(),
        },
    );
    assert_eq!(packet(&s).sections.len(), 2);
    assert_eq!(
        ai_harness::review(
            &s,
            Options {
                include_approved: true,
                ..Default::default()
            }
        )
        .unwrap()
        .sections
        .len(),
        3
    );
    command(
        &s,
        Action::UpdateNote {
            note_id: "ExpertSingle-500".into(),
            tick: 510,
            lane: 1,
            sustain: 0,
            cymbal: false,
        },
    );
    assert!(
        packet(&s)
            .sections
            .iter()
            .any(|x| x.region_id == id && x.track == "ExpertSingle" && x.stale_decision)
    );
    command(
        &s,
        Action::SetFlagVisibility {
            region_id: id.clone(),
            track: "ExpertSingle".into(),
            hidden: true,
        },
    );
    assert!(
        !packet(&s)
            .sections
            .iter()
            .any(|x| x.region_id == id && x.track == "ExpertSingle")
    );
    assert!(
        ai_harness::review(
            &s,
            Options {
                include_hidden: true,
                ..Default::default()
            }
        )
        .unwrap()
        .sections
        .iter()
        .any(|x| x.region_id == id && x.hidden)
    );
}
#[test]
fn ai_edit_uses_existing_audit_undo_and_rejects_old_or_forged_packets() {
    let root = tempfile::tempdir().unwrap();
    let s = fixture(root.path());
    let initial = s.load().unwrap().content.tracks;
    let p = packet(&s);
    apply(
        &s,
        &p,
        Action::UpdateNote {
            note_id: "ExpertSingle-500".into(),
            tick: 520,
            lane: 1,
            sustain: 0,
            cymbal: false,
        },
    )
    .unwrap();
    let doc = s.load().unwrap();
    assert_eq!(doc.audit.last().unwrap().actor.kind, ActorKind::Ai);
    assert!(!doc.undo.is_empty());
    assert!(apply(&s, &p, Action::Undo).is_err());
    apply(&s, &packet(&s), Action::Undo).unwrap();
    assert_eq!(s.load().unwrap().content.tracks, initial);
    let mut forged = packet(&s);
    forged.sections[0].start += 0.01;
    assert!(apply(&s, &forged, Action::Undo).is_err());
    // A checksum alone is not an authorization: even a deliberately rehashed
    // false packet must fail reconstruction under the editor lock.
    forged.packet_sha256.clear();
    forged.packet_sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(&forged).unwrap()));
    forged.validate().unwrap();
    assert!(apply(&s, &forged, Action::Undo).is_err());
    let mut r = serde_json::to_value(ApplyRequest {
        packet_sha256: "x".into(),
        expected_revision: 0,
        actor_name: "AI".into(),
        action: Action::Undo,
    })
    .unwrap();
    r["actor"] = serde_json::json!({"kind":"human","name":"Impersonated"});
    assert!(serde_json::from_value::<ApplyRequest>(r).is_err());
}
#[test]
fn changed_state_or_audio_at_same_revision_cannot_export_or_mutate() {
    let root = tempfile::tempdir().unwrap();
    let s = fixture(root.path());
    let p = packet(&s);
    let mut doc = s.load().unwrap();
    doc.review
        .coverage_limits
        .push("Changed same-revision evidence".into());
    save(&s, &doc);
    let before = fs::read(s.root.join("project.json")).unwrap();
    let output = root.path().join("not-exported");
    assert!(
        apply(
            &s,
            &p,
            Action::Export {
                output: output.clone()
            }
        )
        .is_err()
    );
    assert!(!output.exists());
    assert_eq!(before, fs::read(s.root.join("project.json")).unwrap());
    assert_eq!(fs::read_dir(s.root.join("checkpoints")).unwrap().count(), 0);
    let fresh = packet(&s);
    fs::write(s.root.join("source/song.ogg"), b"changed recording").unwrap();
    assert!(ai_harness::review(&s, Options::default()).is_err());
    assert!(apply(&s, &fresh, Action::Undo).is_err());
    assert_eq!(before, fs::read(s.root.join("project.json")).unwrap());
}
#[test]
fn direct_editor_guard_failure_has_no_audit_checkpoint_or_export() {
    let root = tempfile::tempdir().unwrap();
    let s = fixture(root.path());
    let before = fs::read(s.root.join("project.json")).unwrap();
    let output = root.path().join("guarded-export");
    assert!(
        s.apply_guarded(
            Request {
                expected_revision: 0,
                actor: Actor {
                    kind: ActorKind::Ai,
                    name: "AI".into()
                },
                action: Action::Export {
                    output: output.clone()
                }
            },
            |_| anyhow::bail!("Rejected witness")
        )
        .is_err()
    );
    assert!(!output.exists());
    assert_eq!(before, fs::read(s.root.join("project.json")).unwrap());
    assert_eq!(fs::read_dir(s.root.join("checkpoints")).unwrap().count(), 0);
}
#[test]
fn source_bound_stem_paths_are_checksum_checked_before_action() {
    use chart_engine::stem_review::*;
    let root = tempfile::tempdir().unwrap();
    let s = fixture(root.path());
    let stem = root.path().join("bass.bin");
    fs::write(&stem, b"opaque bound stem").unwrap();
    let mut doc = s.load().unwrap();
    doc.content.stems = Some(StemManifest {
        version: 1,
        source_audio_sha256: doc.audio_sha256.clone(),
        sample_rate: 44100,
        channels: 2,
        frames: 176400,
        alignment: AlignmentEvidence {
            method: "Checksum fixture".into(),
            details: "Metadata/hash control only, no decoded audio claim".into(),
        },
        stems: vec![StemAsset {
            id: StemId::Bass,
            path: stem.clone(),
            sha256: format!("{:x}", Sha256::digest(b"opaque bound stem")),
        }],
        generation: None,
    });
    save(&s, &doc);
    let p = packet(&s);
    assert_eq!(p.audio.stems[0].instrument, "bass");
    assert!(p.audio.stems[0].path.is_absolute());
    fs::write(stem, b"changed stem").unwrap();
    assert!(apply(&s, &p, Action::Undo).is_err());
}

fn split_audio_fixture(root: &Path) -> Store {
    use chart_engine::stem_review::*;
    let store = fixture(root);
    let mut doc = store.load().unwrap();
    let raw = format!(
        "{SOURCE}[ExpertKeyboard]\n{{\n 480 = N 2 240\n}}\n[ExpertProKeys]\n{{\n 480 = N 60 240\n}}\n"
    );
    fs::write(store.root.join("source/notes.chart"), &raw).unwrap();
    let chart = Chart::parse(raw.clone()).unwrap();
    doc.source_chart_sha256 = format!("{:x}", Sha256::digest(raw.as_bytes()));
    for track in ["ExpertKeyboard", "ExpertProKeys"] {
        doc.content.tracks.insert(
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
        );
    }
    let mut inputs = doc.review.input_sha256.clone();
    inputs.insert("chart".into(), doc.source_chart_sha256.clone());
    let mut generated = doc.review.generated_tracks.clone().unwrap();
    generated.extend(["ExpertKeyboard".into(), "ExpertProKeys".into()]);
    let mut issues: Vec<_> = doc
        .review
        .regions
        .iter()
        .flat_map(|r| r.issues.clone())
        .collect();
    issues.extend([
        Issue::at(0.5, "ExpertKeyboard", Reason::PianoSourceReview, &[]),
        Issue::at(0.5, "ExpertProKeys", Reason::PianoSourceReview, &[]),
    ]);
    doc.review = ReviewMap::build(
        4.,
        doc.review.policy.clone(),
        inputs,
        Some(generated),
        BTreeSet::from([Reason::FastRepeat, Reason::PianoSourceReview]),
        issues,
    )
    .unwrap();
    let manifest = |prefix: &str, ids: &[StemId]| StemManifest {
        version: 1,
        source_audio_sha256: doc.audio_sha256.clone(),
        sample_rate: 44100,
        channels: 2,
        frames: 176400,
        alignment: AlignmentEvidence {
            method: format!("{prefix} checksum fixture"),
            details:
                "Opaque bytes test file identity only; no decoded audio or separation quality claim"
                    .into(),
        },
        stems: ids
            .iter()
            .map(|&id| {
                let payload = format!("{prefix} {} bound bytes", id.as_str());
                let path = root.join(format!("{prefix}-{}.bin", id.as_str()));
                fs::write(&path, payload.as_bytes()).unwrap();
                StemAsset {
                    id,
                    path,
                    sha256: format!("{:x}", Sha256::digest(payload.as_bytes())),
                }
            })
            .collect(),
        generation: None,
    };
    doc.content.stems = Some(manifest("primary", &[StemId::Bass, StemId::Piano]));
    doc.content.keyboard_stems = Some(manifest("keyboard", &StemId::ALL));
    save(&store, &doc);
    store.load().unwrap();
    store
}

#[test]
fn split_keyboard_audio_packet_retains_primary_context_and_exact_section_track_scope() {
    let root = tempfile::tempdir().unwrap();
    let store = split_audio_fixture(root.path());
    let p = ai_harness::review(
        &store,
        Options {
            limit: 8,
            ..Default::default()
        },
    )
    .unwrap();
    let keyboard = p.keyboard_audio.as_ref().unwrap();
    assert_eq!(p.audio.mix.sha256, keyboard.mix.sha256);
    assert_eq!(p.audio.mix.path, keyboard.mix.path);
    assert_eq!(p.audio.frames, keyboard.frames);
    assert_eq!(p.audio.stems.len(), 2);
    assert_eq!(keyboard.stems.len(), 6);
    let primary_piano = p
        .audio
        .stems
        .iter()
        .find(|s| s.instrument == "piano")
        .unwrap();
    let actual_piano = keyboard
        .stems
        .iter()
        .find(|s| s.instrument == "piano")
        .unwrap();
    assert_ne!(primary_piano.path, actual_piano.path);
    assert_ne!(primary_piano.sha256, actual_piano.sha256);
    assert!(keyboard.scope.contains("Piano/Pro Piano"));
    assert!(p.sections.iter().any(|s| s.track == "ExpertKeyboard"));
    assert!(p.sections.iter().any(|s| s.track == "ExpertProKeys"));
    assert!(p.sections.iter().any(|s| s.track == "ExpertSingle"));
    assert_eq!(
        serde_json::to_vec(&p).unwrap(),
        serde_json::to_vec(
            &ai_harness::review(
                &store,
                Options {
                    limit: 8,
                    ..Default::default()
                }
            )
            .unwrap()
        )
        .unwrap()
    );
    let roundtrip: Packet = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
    roundtrip.validate().unwrap();
}

#[test]
fn split_keyboard_audio_stale_bytes_and_same_revision_rebinding_fail_before_action() {
    for target in ["keyboard", "primary", "recording", "rebound-keyboard"] {
        let root = tempfile::tempdir().unwrap();
        let store = split_audio_fixture(root.path());
        let p = packet(&store);
        let mut doc = store.load().unwrap();
        let path = match target {
            "primary" => doc.content.stems.as_ref().unwrap().stems[0].path.clone(),
            "recording" => store.audio_path(&doc).unwrap(),
            _ => doc.content.keyboard_stems.as_ref().unwrap().stems[0]
                .path
                .clone(),
        };
        fs::write(&path, b"changed audio context").unwrap();
        if target == "rebound-keyboard" {
            doc.content.keyboard_stems.as_mut().unwrap().stems[0].sha256 =
                format!("{:x}", Sha256::digest(b"changed audio context"));
            save(&store, &doc);
            assert!(packet(&store).keyboard_audio.is_some());
        } else {
            assert!(
                ai_harness::review(&store, Options::default()).is_err(),
                "changed {target}"
            );
        }
        let before = fs::read(store.root.join("project.json")).unwrap();
        assert!(
            apply(&store, &p, Action::Seek { seconds: 0.75 }).is_err(),
            "stale {target}"
        );
        assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
        assert_eq!(
            fs::read_dir(store.root.join("checkpoints"))
                .unwrap()
                .count(),
            0
        );
    }
}

#[test]
fn forged_rehashed_keyboard_context_cannot_substitute_primary_stems() {
    let root = tempfile::tempdir().unwrap();
    let store = split_audio_fixture(root.path());
    let mut p = packet(&store);
    p.keyboard_audio.as_mut().unwrap().stems = p.audio.stems.clone();
    p.packet_sha256.clear();
    p.packet_sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(&p).unwrap()));
    p.validate().unwrap();
    let before = fs::read(store.root.join("project.json")).unwrap();
    assert!(apply(&store, &p, Action::Seek { seconds: 0.75 }).is_err());
    assert_eq!(fs::read(store.root.join("project.json")).unwrap(), before);
}

#[test]
fn packets_without_keyboard_audio_keep_legacy_schema_checksum_and_apply_contract() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let p = packet(&store);
    assert!(p.keyboard_audio.is_none());
    let legacy = serde_json::to_value(&p).unwrap();
    assert!(legacy.get("keyboard_audio").is_none());
    let restored: Packet = serde_json::from_value(legacy).unwrap();
    assert_eq!(restored.packet_sha256, p.packet_sha256);
    assert_eq!(
        serde_json::to_vec(&restored).unwrap(),
        serde_json::to_vec(&p).unwrap()
    );
    restored.validate().unwrap();
    apply(&store, &restored, Action::Seek { seconds: 0.75 }).unwrap();
    assert_eq!(store.load().unwrap().transport.position, 0.75);
}
#[test]
fn vocal_flags_use_same_pending_approval_hide_and_stale_edit_queue() {
    use chart_engine::{vocal_generation as vg, vocal_review::VocalReviewMap, vocals::*};
    let root = tempfile::tempdir().unwrap();
    let s = fixture(root.path());
    let mut doc = s.load().unwrap();
    let chart = VocalChart {
        version: 1,
        resolution: 480,
        source: "Known fixture".into(),
        phrases: vec![VocalPhrase {
            start: 480,
            end: 1440,
            player: 1,
        }],
        notes: vec![
            VocalNote {
                start: 480,
                end: 800,
                tone: VocalTone::Pitched { midi: 60 },
                lyric: Syllable::Text {
                    text: "test".into(),
                    join_next: false,
                },
            },
            VocalNote {
                start: 1000,
                end: 1440,
                tone: VocalTone::Pitched { midi: 62 },
                lyric: Syllable::ContinueWord,
            },
        ],
    };
    let report = vg::Generation {
        version: 1,
        audio_sha256: doc.audio_sha256.clone(),
        vocal_stem_sha256: "b".repeat(64),
        duration_seconds: 4.,
        algorithm: "fixture".into(),
        timing: Some(doc.timing(&s).unwrap()),
        policy: Default::default(),
        status: vg::Status::ReviewRequired,
        chart: Some(chart.clone()),
        pitch_candidates: vec![],
        pitch_frames: vec![],
        words: None,
        lyrics: None,
        issues: vec![vg::Issue {
            start_seconds: 0.5,
            end_seconds: 1.5,
            reason: vg::Reason::PitchDisagreement,
            detail: "Review original recording".into(),
        }],
        warnings: vec![],
        neural_pitch: None,
        vocal_pitch: None,
    };
    doc.content.vocals = Some(chart.clone());
    doc.content.vocal_review = Some(VocalReviewMap::from_generation(&report).unwrap());
    save(&s, &doc);
    let p = ai_harness::review(
        &s,
        Options {
            limit: 8,
            ..Default::default()
        },
    )
    .unwrap();
    let vocal = p.sections.iter().find(|x| x.track == "Vocals").unwrap();
    assert_eq!(vocal.notes[0].tone, Some(VocalTone::Pitched { midi: 60 }));
    assert_eq!(
        vocal.notes[0].syllable_kind,
        Some(ai_harness::VocalSyllableKind::Text)
    );
    assert_eq!(
        vocal.notes[1].syllable_kind,
        Some(ai_harness::VocalSyllableKind::ContinueWord)
    );
    assert_eq!(vocal.notes[1].lyric.as_deref(), Some(""));
    assert_eq!(vocal.vocal_phrases.len(), 1);
    let id = vocal.region_id.clone();
    apply(
        &s,
        &p,
        Action::Decide {
            region_id: id.clone(),
            track: "Vocals".into(),
            status: DecisionStatus::Reviewed,
            comment: "Checked fixture phrase".into(),
        },
    )
    .unwrap();
    assert!(
        !ai_harness::review(
            &s,
            Options {
                limit: 8,
                ..Default::default()
            }
        )
        .unwrap()
        .sections
        .iter()
        .any(|x| x.track == "Vocals")
    );
    let mut edited = chart;
    edited.notes[0].tone = VocalTone::Pitched { midi: 62 };
    command(&s, Action::SetVocals { document: edited });
    assert!(
        ai_harness::review(
            &s,
            Options {
                limit: 8,
                ..Default::default()
            }
        )
        .unwrap()
        .sections
        .iter()
        .any(|x| x.track == "Vocals" && x.stale_decision)
    );
    command(
        &s,
        Action::SetFlagVisibility {
            region_id: id,
            track: "Vocals".into(),
            hidden: true,
        },
    );
    assert!(
        !ai_harness::review(
            &s,
            Options {
                limit: 8,
                ..Default::default()
            }
        )
        .unwrap()
        .sections
        .iter()
        .any(|x| x.track == "Vocals")
    );
    // Valid source text may be long; typed context carries a bounded kind,
    // rather than cloning that text back into an otherwise compact packet.
    let mut huge = s.load().unwrap().content.vocals.unwrap();
    huge.notes[0].lyric = Syllable::Text {
        text: "a".repeat(600_000),
        join_next: false,
    };
    command(&s, Action::SetVocals { document: huge });
    let bounded = ai_harness::review(
        &s,
        Options {
            track: Some("Vocals".into()),
            include_hidden: true,
            ..Default::default()
        },
    )
    .unwrap();
    let vocal = &bounded.sections[0];
    assert_eq!(vocal.notes[0].lyric.as_ref().unwrap().len(), 128);
    assert_eq!(
        vocal.notes[0].syllable_kind,
        Some(ai_harness::VocalSyllableKind::Text)
    );
    assert!(serde_json::to_vec_pretty(&bounded).unwrap().len() < 512 * 1024);
}

#[test]
fn expert_context_shows_linked_children_and_local_overrides_survive_expert_edit() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    command(
        &store,
        Action::EnableDifficultyLink {
            track: "EasySingle".into(),
            enabled: true,
        },
    );
    let initial = packet(&store);
    let parent = initial
        .sections
        .iter()
        .filter(|s| s.track == "ExpertSingle")
        .flat_map(|s| &s.notes)
        .find(|n| n.id == "ExpertSingle-2400")
        .unwrap();
    let child = parent
        .linked_children
        .iter()
        .find(|c| c.track == "EasySingle")
        .unwrap();
    let child_id = child.note_id.clone();
    let original = store
        .load()
        .unwrap()
        .find_note(&child_id)
        .unwrap()
        .1
        .note
        .clone();
    command(
        &store,
        Action::UpdateNote {
            note_id: child_id.clone(),
            tick: original.tick,
            lane: 2,
            sustain: original.sustain,
            cymbal: false,
        },
    );
    let p = packet(&store);
    assert!(
        p.sections
            .iter()
            .flat_map(|s| &s.notes)
            .filter(|n| n.id == "ExpertSingle-2400")
            .flat_map(|n| &n.linked_children)
            .any(|c| c.note_id == child_id && c.manual_override)
    );
    apply(
        &store,
        &p,
        Action::UpdateNote {
            note_id: "ExpertSingle-2400".into(),
            tick: 2500,
            lane: 3,
            sustain: 0,
            cymbal: false,
        },
    )
    .unwrap();
    let after = store.load().unwrap();
    assert_eq!(after.find_note(&child_id).unwrap().1.note.lane, 2);
    assert_eq!(
        after.find_note(&child_id).unwrap().1.note.tick,
        original.tick
    );
    assert!(
        after.content.difficulty_links.links["EasySingle"]
            .overrides
            .contains_key(&child_id)
    );
}

#[test]
fn cli_review_next_and_apply_use_the_same_guarded_contract() {
    use std::process::Command;
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let bin = env!("CARGO_BIN_EXE_chart-engine");
    let path = root.path().join("packet.json");
    let result = Command::new(bin)
        .args(["ai", "next"])
        .arg(&store.root)
        .arg("--output")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let p: Packet = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(p.sections.len(), 1);
    let request = root.path().join("request.json");
    fs::write(
        &request,
        serde_json::to_vec(&ApplyRequest {
            packet_sha256: p.packet_sha256,
            expected_revision: p.revision,
            actor_name: "CLI AI".into(),
            action: Action::UpdateNote {
                note_id: "ExpertSingle-500".into(),
                tick: 520,
                lane: 1,
                sustain: 0,
                cymbal: false,
            },
        })
        .unwrap(),
    )
    .unwrap();
    let result = Command::new(bin)
        .args(["ai", "apply"])
        .arg(&store.root)
        .arg("--packet")
        .arg(&path)
        .arg("--request")
        .arg(&request)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        store.load().unwrap().audit.last().unwrap().actor.kind,
        ActorKind::Ai
    );
    let old = Command::new(bin)
        .args(["ai", "apply"])
        .arg(&store.root)
        .arg("--packet")
        .arg(path)
        .arg("--request")
        .arg(request)
        .output()
        .unwrap();
    assert!(!old.status.success());
    assert_eq!(store.load().unwrap().revision, 1);
}

#[test]
fn pretty_packet_bound_matches_own_file_roundtrip() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let mut p = packet(&store);
    p.musical_acceptance.clear();
    let base = serde_json::to_vec_pretty(&p).unwrap().len();
    p.musical_acceptance = "x".repeat(512 * 1024 - base + 1);
    fn rehash(p: &mut Packet) {
        p.packet_sha256.clear();
        p.packet_sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(p).unwrap()));
    }
    rehash(&mut p);
    assert!(serde_json::to_vec(&p).unwrap().len() < 512 * 1024);
    assert!(serde_json::to_vec_pretty(&p).unwrap().len() > 512 * 1024);
    assert!(p.validate().unwrap_err().to_string().contains("bounds"));
    p.musical_acceptance.pop();
    rehash(&mut p);
    let own_file = serde_json::to_vec_pretty(&p).unwrap();
    assert_eq!(own_file.len(), 512 * 1024);
    let loaded: Packet = serde_json::from_slice(&own_file).unwrap();
    loaded.validate().unwrap();
}

#[test]
fn replacing_aligned_vocal_stem_keeps_stale_checks_inspectable_hideable_and_navigable() {
    use chart_engine::{
        models, stem_review::*, vocal_generation as vg, vocal_review::VocalReviewMap, vocals::*,
    };
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let tone = |hz: f32| {
        (0..176400)
            .map(|i| 0.1 * (std::f32::consts::TAU * hz * i as f32 / 44100.).sin())
            .collect::<Vec<_>>()
    };
    let a = tone(220.);
    let b = tone(330.);
    let source = root.path().join("source/song.wav");
    let voice_a = root.path().join("voice-a.wav");
    let voice_b = root.path().join("voice-b.wav");
    models::write_audio(&source, &a, &a).unwrap();
    models::write_audio(&voice_a, &a, &a).unwrap();
    models::write_audio(&voice_b, &b, &b).unwrap();
    let mut doc = store.load().unwrap();
    doc.audio_filename = "song.wav".into();
    doc.audio_sha256 = models::hash_file(&source).unwrap();
    let mut inputs = doc.review.input_sha256.clone();
    inputs.insert("original_audio".into(), doc.audio_sha256.clone());
    doc.review = ReviewMap::build(
        4.,
        doc.review.policy.clone(),
        inputs,
        doc.review.generated_tracks.clone(),
        doc.review.checks_performed.clone(),
        doc.review
            .regions
            .iter()
            .flat_map(|r| r.issues.clone())
            .collect(),
    )
    .unwrap();
    save(&store, &doc);
    let attach = |voice: &Path, name: &str| {
        let manifest = StemManifest::build(
            &source,
            BTreeMap::from([(StemId::Vocals, voice.to_owned())]),
            AlignmentEvidence {
                method: "Synthetic synchronized sample clock".into(),
                details:
                    "Four seconds at44100Hz; source and both synthetic stems begin on sample zero"
                        .into(),
            },
        )
        .unwrap();
        let path = root.path().join(name);
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        command(&store, Action::AttachStems { manifest: path });
    };
    attach(&voice_a, "a.json");
    let chart = VocalChart {
        version: 1,
        resolution: 480,
        source: "Synthetic source-bound phrase".into(),
        phrases: vec![VocalPhrase {
            start: 480,
            end: 1440,
            player: 1,
        }],
        notes: vec![VocalNote {
            start: 480,
            end: 1440,
            tone: VocalTone::Pitched { midi: 57 },
            lyric: Syllable::Text {
                text: "test".into(),
                join_next: false,
            },
        }],
    };
    let doc = store.load().unwrap();
    let timing = doc.timing(&store).unwrap();
    let report = vg::Generation {
        version: 1,
        audio_sha256: doc.audio_sha256,
        vocal_stem_sha256: models::hash_file(&voice_a).unwrap(),
        duration_seconds: 4.,
        timing: Some(timing),
        algorithm: "source-binding fixture".into(),
        policy: Default::default(),
        status: vg::Status::ReviewRequired,
        chart: Some(chart.clone()),
        pitch_candidates: vec![],
        pitch_frames: vec![],
        words: None,
        lyrics: None,
        issues: vec![vg::Issue {
            start_seconds: 0.5,
            end_seconds: 1.5,
            reason: vg::Reason::PitchDisagreement,
            detail: "Review source-bound synthetic contour".into(),
        }],
        warnings: vec![],
        neural_pitch: None,
        vocal_pitch: None,
    };
    command(
        &store,
        Action::SetVocalGeneration {
            document: chart,
            review: VocalReviewMap::from_generation(&report).unwrap(),
        },
    );
    let options = || Options {
        track: Some("Vocals".into()),
        ..Default::default()
    };
    assert!(ai_harness::review(&store, options()).unwrap().sections[0].approval_allowed);
    attach(&voice_b, "b.json");
    let p = ai_harness::review(&store, options()).unwrap();
    let section = &p.sections[0];
    let id = section.region_id.clone();
    assert!(!section.approval_allowed);
    assert!(section.region_content_sha256.is_none());
    assert!(
        section
            .source_stale_reason
            .as_ref()
            .unwrap()
            .contains("different vocal stem")
    );
    assert_ne!(
        p.vocal_review_stem_sha256.as_ref().unwrap(),
        &p.audio.stems[0].sha256
    );
    let before = fs::read(store.root.join("project.json")).unwrap();
    assert!(
        apply(
            &store,
            &p,
            Action::Decide {
                region_id: id.clone(),
                track: "Vocals".into(),
                status: DecisionStatus::Reviewed,
                comment: "Cannot approve stale source".into()
            }
        )
        .is_err()
    );
    assert_eq!(before, fs::read(store.root.join("project.json")).unwrap());
    apply(
        &store,
        &p,
        Action::SetFlagVisibility {
            region_id: id,
            track: "Vocals".into(),
            hidden: true,
        },
    )
    .unwrap();
    assert!(
        ai_harness::review(&store, options())
            .unwrap()
            .sections
            .is_empty()
    );
    let hidden = ai_harness::review(
        &store,
        Options {
            include_hidden: true,
            ..options()
        },
    )
    .unwrap();
    assert!(hidden.sections[0].hidden && !hidden.sections[0].approval_allowed);
    apply(&store, &hidden, Action::Seek { seconds: 0.5 }).unwrap();
    assert_eq!(store.load().unwrap().transport.position, 0.5);
}
