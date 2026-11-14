//! Independent observations exercise note correction, persistence and stale jobs.
#[path = "support/assistant_project.rs"]
mod fixture;
use chart_engine::{
    editor::{Action, Actor, ActorKind, Request, Store, TimeRange},
    stem_review::{AlignmentEvidence, StemAsset, StemId, StemManifest},
    timing::TempoMap,
    vocal_generation::{self as vg, PitchFrame, PitchSupport},
    vocal_pitch::{Evidence, Frame},
    vocal_pitch_analysis as analysis, vocal_pitch_tracking as tracking,
    vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};
use std::{fs, path::Path};
fn setup(root: &Path) -> (Store, vg::Generation) {
    let store = fixture::fixture(root);
    let mut p = store.load().unwrap();
    let stem = root.join("voice.wav");
    fs::write(&stem, b"bound independent source fixture").unwrap();
    let hash = chart_engine::models::hash_file(&stem).unwrap();
    p.content.stems = Some(StemManifest {
        version: 1,
        source_audio_sha256: p.audio_sha256.clone(),
        sample_rate: 44100,
        channels: 2,
        frames: 176400,
        conventional: None,
        generation: None,
        alignment: AlignmentEvidence {
            method: "fixture".into(),
            details: "source identity, not an accuracy claim".into(),
        },
        stems: vec![StemAsset {
            id: StemId::Vocals,
            path: stem,
            sha256: hash.clone(),
        }],
    });
    p.content.vocals = Some(VocalChart {
        version: 1,
        resolution: 480,
        source: "independent test intervals".into(),
        phrases: vec![VocalPhrase {
            midi: None,
            start: 0,
            end: 3840,
            player: 1,
        }],
        notes: vec![
            VocalNote {
                start: 480,
                end: 960,
                tone: VocalTone::Pitched { midi: 60 },
                lyric: Syllable::Text {
                    text: "niño".into(),
                    join_next: false,
                },
                midi: None,
            },
            VocalNote {
                start: 1440,
                end: 1920,
                tone: VocalTone::Pitched { midi: 64 },
                lyric: Syllable::Missing,
                midi: None,
            },
            VocalNote {
                start: 2400,
                end: 2880,
                tone: VocalTone::Talkie { lenient: true },
                lyric: Syllable::Text {
                    text: "rap".into(),
                    join_next: false,
                },
                midi: None,
            },
        ],
    });
    fixture::save(&store, &p);
    let timing = p.timing(&store).unwrap();
    let mut generation = vg::generate_pcm(
        &vec![0.; 88200],
        &p.audio_sha256,
        &hash,
        4.,
        &timing,
        None,
        None,
        &vg::Policy::default(),
    )
    .unwrap();
    generation.algorithm = vg::COMPARISON_ALGORITHM.into();
    generation.policy.analysis_mode = vg::AnalysisMode::HighAccuracy;
    generation.rmvpe_pitch = Some(Evidence {
        version: 1,
        model_id: chart_engine::vocal_rmvpe::MODEL_ID.into(),
        model_revision: chart_engine::vocal_rmvpe::REVISION.into(),
        model_sha256: chart_engine::vocal_rmvpe::SHA256.into(),
        implementation_sha256: chart_engine::vocal_rmvpe::implementation_hash(),
        audio_sha256: p.audio_sha256.clone(),
        vocal_stem_sha256: hash,
        pcm_sha256: "f".repeat(64),
        duration_seconds: 4.,
        sample_rate: 16000,
        hop_samples: 160,
        frames: (0..401)
            .map(|i| Frame {
                time_seconds: i as f64 * 0.01,
                frequency_hz: (i < 120).then_some(220.),
                confidence: if i < 120 { 0.9 } else { 0. },
                rms: 0.1,
            })
            .collect(),
    });
    (store, generation)
}
fn apply(store: &Store, action: Action) -> anyhow::Result<chart_engine::editor::Applied> {
    store.apply(Request {
        expected_revision: store.load()?.revision,
        actor: Actor {
            name: "pitch-test".into(),
            kind: ActorKind::Human,
        },
        action,
    })
}

fn observations(g: &mut vg::Generation, spans: &[(f64, f64, f64)], acoustic: bool) {
    let raw = g.rmvpe_pitch.as_mut().unwrap();
    for f in &mut raw.frames {
        let midi = spans
            .iter()
            .find(|(a, b, _)| *a <= f.time_seconds && f.time_seconds < *b)
            .map(|(_, _, p)| *p);
        f.frequency_hz = midi.map(|p| 440. * 2_f64.powf((p - 69.) / 12.));
        f.confidence = if midi.is_some() { 0.9 } else { 0. };
        f.rms = if midi.is_some() { 0.1 } else { 0. };
    }
    g.pitch_frames = raw
        .frames
        .iter()
        .map(|f| {
            let midi = f.frequency_hz.map(|hz| 69. + 12. * (hz / 440.).log2());
            PitchFrame {
                time_seconds: f.time_seconds,
                midi,
                yin_midi: midi.filter(|_| acoustic),
                mcleod_midi: midi.filter(|_| acoustic),
                // Production High caps this displayed value after refinement.
                clarity: if acoustic { 0.59 } else { 0. },
                rms: f.rms,
                local_rms: Some(f.rms),
                local_energy_active: Some(f.rms > 0.),
                support: PitchSupport::Consensus,
            }
        })
        .collect();
}

#[test]
fn section_repairs_recover_short_gaps_extend_split_and_preserve_talkies_with_one_undo() {
    use chart_engine::vocal_section_analysis::RepairKind;
    let temp = tempfile::tempdir().unwrap();
    let (store, mut generation) = setup(temp.path());
    let before = store.load().unwrap();
    let timing = before.timing(&store).unwrap();
    observations(
        &mut generation,
        &[
            (0.4, 1.1, 60.),
            (1.5, 1.75, 64.),
            (1.75, 2., 67.),
            (2.1, 2.2, 69.),
            (2.6, 2.8, 72.),
        ],
        true,
    );
    let result = analysis::propose(&before, &timing, &generation, None).unwrap();
    assert_eq!(result.section.repairs.len(), 3);
    assert_eq!(result.section.repairs[0].kind, RepairKind::Boundary);
    assert_eq!(result.section.repairs[1].kind, RepairKind::Split);
    assert_eq!(result.section.repairs[2].kind, RepairKind::Missing);
    assert!(result.section.uncharted_seconds > 0.3);
    apply(&store, Action::SetVocalPitchAnalysis { analysis: result }).unwrap();
    apply(
        &store,
        Action::ApplyVocalSectionRepairs {
            indices: vec![0, 1, 2],
            include_pitch: false,
        },
    )
    .unwrap();
    let doc = store.load().unwrap();
    let chart = doc.content.vocals.as_ref().unwrap();
    assert_eq!(chart.notes.len(), 5);
    assert_eq!(
        chart.notes[0].lyric,
        before.content.vocals.as_ref().unwrap().notes[0].lyric
    );
    assert_eq!(
        chart.notes.last().unwrap(),
        before
            .content
            .vocals
            .as_ref()
            .unwrap()
            .notes
            .last()
            .unwrap()
    );
    assert_eq!(chart.notes[3].lyric, Syllable::Missing);
    let original_midi = chart_engine::midi::new_midi(&timing, &Default::default()).unwrap();
    let exported = chart_engine::vocals::append_midi(&original_midi, chart).unwrap();
    let mut imported = chart_engine::vocals::import_midi(&exported).unwrap();
    imported.source = chart.source.clone();
    assert_eq!(&imported, chart);
    assert!(chart.notes[..4].iter().all(|n| {
        analysis::current_evidence(&doc, &timing, n)
            .unwrap()
            .uncertain
    }));
    assert!(
        apply(
            &store,
            Action::ApplyVocalSectionRepairs {
                indices: vec![2],
                include_pitch: false
            }
        )
        .is_err()
    );
    apply(&store, Action::Undo).unwrap();
    assert_eq!(store.load().unwrap().content.vocals, before.content.vocals);
    apply(&store, Action::Redo).unwrap();
    assert_eq!(store.load().unwrap().content.vocals, doc.content.vocals);
    // A later check retains structural review flags just as it retains retuning.
    let next = analysis::propose(
        &doc,
        &timing,
        &generation,
        Some(TimeRange {
            start: 0.,
            end: 0.2,
        }),
    )
    .unwrap();
    apply(&store, Action::SetVocalPitchAnalysis { analysis: next }).unwrap();
    let reopened = Store::new(temp.path()).load().unwrap();
    assert!(
        analysis::current_evidence(&reopened, &timing, &chart.notes[3])
            .unwrap()
            .uncertain
    );
}

#[test]
fn section_repairs_need_independent_evidence_and_cannot_remove_silent_notes_or_cross_selection() {
    let temp = tempfile::tempdir().unwrap();
    let (store, mut g) = setup(temp.path());
    let doc = store.load().unwrap();
    let timing = doc.timing(&store).unwrap();
    observations(&mut g, &[(0.4, 1.1, 60.), (2.1, 2.2, 69.)], false);
    let result = analysis::propose(&doc, &timing, &g, None).unwrap();
    assert!(result.section.repairs.is_empty());
    assert!(!result.section.findings.is_empty());
    observations(&mut g, &[], true);
    let result = analysis::propose(&doc, &timing, &g, None).unwrap();
    assert!(result.section.repairs.is_empty());
    assert!(
        result
            .section
            .findings
            .iter()
            .any(|f| f.detail.contains("extra note"))
    );
    observations(&mut g, &[(0.4, 1.1, 60.), (2.1, 2.2, 69.)], true);
    let result = analysis::propose(
        &doc,
        &timing,
        &g,
        Some(TimeRange {
            start: 0.6,
            end: 2.05,
        }),
    )
    .unwrap();
    assert!(result.section.repairs.is_empty());
    let mut result = analysis::propose(&doc, &timing, &g, None).unwrap();
    result.section.repairs[0].after[0].lyric = Syllable::Missing;
    assert!(apply(&store, Action::SetVocalPitchAnalysis { analysis: result }).is_err());
}

#[test]
fn section_subset_selection_bounds_phrase_creation_and_stale_batch_are_guarded() {
    let temp = tempfile::tempdir().unwrap();
    let (store, mut g) = setup(temp.path());
    let mut doc = store.load().unwrap();
    doc.content.vocals.as_mut().unwrap().phrases[0].end = 2880;
    fixture::save(&store, &doc);
    let timing = doc.timing(&store).unwrap();
    observations(&mut g, &[(0.4, 1.1, 60.), (3.2, 3.35, 69.)], true);
    let result = analysis::propose(&doc, &timing, &g, None).unwrap();
    assert_eq!(result.section.repairs.len(), 2);
    apply(&store, Action::SetVocalPitchAnalysis { analysis: result }).unwrap();
    assert!(
        apply(
            &store,
            Action::ApplyVocalSectionRepairs {
                indices: vec![1, 1],
                include_pitch: false
            }
        )
        .is_err()
    );
    assert!(
        apply(
            &store,
            Action::ApplyVocalSectionRepairs {
                indices: vec![999],
                include_pitch: false
            }
        )
        .is_err()
    );
    apply(
        &store,
        Action::ApplyVocalSectionRepairs {
            indices: vec![1],
            include_pitch: false,
        },
    )
    .unwrap();
    let doc = store.load().unwrap();
    let chart = doc.content.vocals.as_ref().unwrap();
    assert_eq!(chart.phrases.len(), 2);
    assert_eq!(chart.notes[0].start, 480);
    assert_eq!(chart.notes.last().unwrap().lyric, Syllable::Missing);
    // Legacy reports did not contain the optional section object.
    let mut old = serde_json::to_value(doc.content.vocal_pitch_analysis.as_ref().unwrap()).unwrap();
    old.as_object_mut().unwrap().remove("section");
    let old: analysis::Analysis = serde_json::from_value(old).unwrap();
    assert!(old.section.applied_pitch);
}

#[test]
fn raw_slide_is_reviewed_without_inventing_steps_and_tempo_changes_keep_audio_clock() {
    let temp = tempfile::tempdir().unwrap();
    let (store, mut g) = setup(temp.path());
    let doc = store.load().unwrap();
    let timing = doc.timing(&store).unwrap();
    observations(&mut g, &[(2.05, 2.45, 65.)], true);
    for (f, dsp) in g
        .rmvpe_pitch
        .as_mut()
        .unwrap()
        .frames
        .iter_mut()
        .zip(&mut g.pitch_frames)
    {
        if f.frequency_hz.is_some() {
            let p = 65. + (f.time_seconds - 2.05) * 7.5;
            f.frequency_hz = Some(440. * 2_f64.powf((p - 69.) / 12.));
            dsp.midi = Some(p);
            dsp.yin_midi = Some(p);
            dsp.mcleod_midi = Some(p);
        }
    }
    let result = analysis::propose(&doc, &timing, &g, None).unwrap();
    assert!(result.section.repairs.is_empty());
    assert!(!result.section.findings.is_empty());
    let timing = TempoMap::new(
        480,
        0.,
        vec![
            chart_engine::timing::Tempo { tick: 0, bpm: 120. },
            chart_engine::timing::Tempo {
                tick: 960,
                bpm: 240.,
            },
        ],
    )
    .unwrap();
    g.timing = Some(timing.clone());
    observations(&mut g, &[(3.1, 3.25, 69.)], true);
    let result = analysis::propose(
        &doc,
        &timing,
        &g,
        Some(TimeRange {
            start: 3.05,
            end: 3.3,
        }),
    )
    .unwrap();
    assert_eq!(result.section.repairs.len(), 1);
    let note = &result.section.repairs[0].after[0];
    assert!((timing.seconds(note.start) - 3.1).abs() < 0.001);
    assert!((timing.seconds(note.end) - 3.25).abs() < 0.001);
}
#[test]
fn independent_dense_contour_retunes_only_supported_intervals_and_bulk_undo_persists() {
    let temp = tempfile::tempdir().unwrap();
    let (store, generation) = setup(temp.path());
    let before = store.load().unwrap();
    let timing = before.timing(&store).unwrap();
    // DSP detected nothing: section checking must still use the independent model.
    assert!(generation.pitch_frames.iter().all(|f| f.midi.is_none()));
    let result = analysis::propose(&before, &timing, &generation, None).unwrap();
    assert_eq!(
        (
            result.inspected,
            result.unresolved,
            result.corrections.len()
        ),
        (2, 1, 1)
    );
    assert_eq!(
        (result.corrections[0].before, result.corrections[0].after),
        (60, 57)
    );
    apply(&store, Action::SetVocalPitchAnalysis { analysis: result }).unwrap();
    apply(&store, Action::ApplyVocalPitchCorrections).unwrap();
    let corrected = store.load().unwrap();
    let evidence = analysis::current_evidence(
        &corrected,
        &timing,
        &corrected.content.vocals.as_ref().unwrap().notes[0],
    )
    .unwrap();
    assert!(evidence.uncertain);
    assert_eq!(evidence.selected_midi, 57);
    assert_eq!(evidence.alternatives, vec![60]);
    let mut expected = before.content.vocals.clone().unwrap();
    expected.notes[0].tone = VocalTone::Pitched { midi: 57 };
    assert_eq!(corrected.content.vocals.as_ref().unwrap(), &expected);
    assert!(
        corrected
            .content
            .vocal_pitch_analysis
            .as_ref()
            .unwrap()
            .applied
    );
    assert!(apply(&store, Action::ApplyVocalPitchCorrections).is_err());
    apply(&store, Action::Undo).unwrap();
    assert_eq!(store.load().unwrap().content.vocals, before.content.vocals);
    apply(&store, Action::Redo).unwrap();
    assert_eq!(store.load().unwrap().content.vocals.unwrap(), expected);
    let midi = chart_engine::midi::new_midi(&timing, &Default::default()).unwrap();
    let midi = chart_engine::vocals::append_midi(&midi, &expected).unwrap();
    let restored = chart_engine::vocals::import_midi(&midi).unwrap();
    assert_eq!(
        restored
            .notes
            .iter()
            .map(|n| (&n.tone, &n.lyric, n.start, n.end))
            .collect::<Vec<_>>(),
        expected
            .notes
            .iter()
            .map(|n| (&n.tone, &n.lyric, n.start, n.end))
            .collect::<Vec<_>>()
    );
}
#[test]
fn comparison_rejects_stale_pitch_lyrics_tempo_stem_and_recording() {
    let temp = tempfile::tempdir().unwrap();
    let (store, generation) = setup(temp.path());
    let p = store.load().unwrap();
    let timing = p.timing(&store).unwrap();
    let result = analysis::propose(&p, &timing, &generation, None).unwrap();
    for kind in 0..5 {
        let mut stale = p.clone();
        match kind {
            0 => {
                stale.content.vocals.as_mut().unwrap().notes[0].tone =
                    VocalTone::Pitched { midi: 62 }
            }
            1 => stale.content.vocals.as_mut().unwrap().notes[0].lyric = Syllable::Missing,
            2 => stale.audio_sha256 = "d".repeat(64),
            3 => stale.content.stems.as_mut().unwrap().stems[0].sha256 = "e".repeat(64),
            _ => {
                assert!(
                    result
                        .validate(&p, &TempoMap::constant(125.).unwrap())
                        .is_err()
                );
                continue;
            }
        }
        assert!(result.validate(&stale, &timing).is_err());
    }
    apply(&store, Action::SetVocalPitchAnalysis { analysis: result }).unwrap();
    fs::write(temp.path().join("voice.wav"), b"source changed").unwrap();
    let revision = store.load().unwrap().revision;
    assert!(apply(&store, Action::ApplyVocalPitchCorrections).is_err());
    assert_eq!(store.load().unwrap().revision, revision);
    assert_eq!(store.load().unwrap().content.vocals, p.content.vocals);
}
#[test]
fn partial_note_selection_no_model_evidence_and_weak_salience_are_not_corrected() {
    let temp = tempfile::tempdir().unwrap();
    let (store, mut generation) = setup(temp.path());
    let p = store.load().unwrap();
    let timing = p.timing(&store).unwrap();
    let result = analysis::propose(
        &p,
        &timing,
        &generation,
        Some(TimeRange {
            start: 0.6,
            end: 4.,
        }),
    )
    .unwrap();
    assert!(result.corrections.is_empty());
    for f in &mut generation.rmvpe_pitch.as_mut().unwrap().frames {
        f.confidence = 0.08;
    }
    assert!(
        analysis::propose(&p, &timing, &generation, None)
            .unwrap()
            .corrections
            .is_empty()
    );
    generation.rmvpe_pitch = None;
    assert!(analysis::propose(&p, &timing, &generation, None).is_err());
}
fn contour(pitches: &[Option<f64>]) -> Vec<PitchFrame> {
    pitches
        .iter()
        .enumerate()
        .map(|(i, &midi)| PitchFrame {
            time_seconds: i as f64 * 0.01,
            midi,
            yin_midi: midi,
            mcleod_midi: midi,
            clarity: 0.9,
            rms: 0.1,
            local_rms: None,
            local_energy_active: None,
            support: PitchSupport::Consensus,
        })
        .collect()
}
#[test]
fn tracking_handles_vibrato_slides_real_short_notes_gaps_and_cancellation() {
    let mut f = contour(
        &(0..100)
            .map(|i| Some(60. + 0.4 * (i as f64 * 0.4).sin()))
            .collect::<Vec<_>>(),
    );
    tracking::note_path(&mut f, 3.2, &|| false).unwrap();
    assert!(f.iter().all(|f| f.midi == Some(60.)));
    let mut pitches = vec![Some(60.); 20];
    pitches.extend((0..30).map(|i| Some(60. + 4. * i as f64 / 29.)));
    pitches.extend(vec![Some(64.); 20]);
    pitches.extend(vec![None; 10]);
    pitches.extend(vec![Some(76.); 10]);
    let mut f = contour(&pitches);
    tracking::note_path(&mut f, 3.2, &|| false).unwrap();
    assert!(f[70..80].iter().all(|f| f.midi.is_none()));
    assert!(f[80..].iter().all(|f| f.midi == Some(76.)));
    assert!(f[..70].windows(2).all(|w| w[0].midi <= w[1].midi));
    let before = f.clone();
    assert!(tracking::note_path(&mut f, 3.2, &|| true).is_err());
    assert_eq!(
        serde_json::to_value(&f).unwrap(),
        serde_json::to_value(before).unwrap()
    );
}

#[test]
fn comparison_refinement_preserves_every_voicing_bit_and_raw_detector_alternatives() {
    let temp = tempfile::tempdir().unwrap();
    let (_, generation) = setup(temp.path());
    let evidence = generation.rmvpe_pitch.unwrap();
    let mut pitches = vec![Some(69.); 100];
    pitches[20..28].fill(None); // context can bridge internally, never fill output
    pitches[50..59].fill(None); // longer gap resets context
    let mut f = contour(&pitches);
    tracking::refine(&mut f, &evidence, &|| false).unwrap();
    for (frame, before) in f.iter().zip(pitches) {
        assert_eq!(frame.midi.is_some(), before.is_some());
        assert_eq!(frame.yin_midi, before);
        assert_eq!(frame.mcleod_midi, before);
        if before.is_some() {
            assert_eq!(frame.midi, Some(57.));
            assert_eq!(frame.support, PitchSupport::ContourPath);
            assert!(frame.clarity <= 0.59);
        }
    }
    assert!(tracking::refine(&mut f, &evidence, &|| true).is_err());
}

#[test]
fn checking_another_phrase_keeps_prior_corrected_notes_flagged() {
    let temp = tempfile::tempdir().unwrap();
    let (store, generation) = setup(temp.path());
    let original = store.load().unwrap();
    let timing = original.timing(&store).unwrap();
    let first = analysis::propose(&original, &timing, &generation, None).unwrap();
    apply(&store, Action::SetVocalPitchAnalysis { analysis: first }).unwrap();
    apply(&store, Action::ApplyVocalPitchCorrections).unwrap();
    let corrected = store.load().unwrap();
    let mut next = analysis::propose(
        &corrected,
        &timing,
        &generation,
        Some(TimeRange {
            start: 1.5,
            end: 2.,
        }),
    )
    .unwrap();
    next.retained_evidence.push(vg::NoteEvidence {
        models: vec![],
        start: 1440,
        end: 1920,
        selected_midi: 64,
        alternatives: vec![65],
        support: vec![PitchSupport::ContourPath],
        uncertain: true,
        reasons: vec!["Untrusted worker witness".into()],
    });
    apply(&store, Action::SetVocalPitchAnalysis { analysis: next }).unwrap();
    let checked = store.load().unwrap();
    assert_eq!(checked.content.vocals, corrected.content.vocals);
    let note = &checked.content.vocals.as_ref().unwrap().notes[0];
    let evidence = analysis::current_evidence(&checked, &timing, note)
        .expect("A later check must not erase an applied pitch's review evidence");
    assert!(evidence.uncertain);
    assert_eq!(evidence.selected_midi, 57);
    assert_eq!(evidence.alternatives, vec![60]);
    assert!(
        analysis::current_evidence(
            &checked,
            &timing,
            &checked.content.vocals.as_ref().unwrap().notes[1]
        )
        .is_none()
    );
    assert_eq!(
        checked
            .content
            .vocal_pitch_analysis
            .as_ref()
            .unwrap()
            .retained_evidence
            .len(),
        1
    );
    let next = analysis::propose(
        &checked,
        &timing,
        &generation,
        Some(TimeRange {
            start: 1.5,
            end: 2.,
        }),
    )
    .unwrap();
    apply(&store, Action::SetVocalPitchAnalysis { analysis: next }).unwrap();
    let checked = store.load().unwrap();
    assert_eq!(
        checked
            .content
            .vocal_pitch_analysis
            .as_ref()
            .unwrap()
            .retained_evidence
            .len(),
        1
    );
    let mut changed = checked.clone();
    changed.audio_sha256 = "0".repeat(64);
    assert!(
        analysis::current_evidence(
            &changed,
            &timing,
            &changed.content.vocals.as_ref().unwrap().notes[0]
        )
        .is_none()
    );
    changed = checked.clone();
    changed.content.vocals.as_mut().unwrap().notes[0].tone = VocalTone::Pitched { midi: 59 };
    assert!(
        analysis::current_evidence(
            &changed,
            &timing,
            &changed.content.vocals.as_ref().unwrap().notes[0]
        )
        .is_none()
    );
}

#[test]
fn model_identity_clock_and_nonfinite_evidence_are_explicit_errors() {
    let temp = tempfile::tempdir().unwrap();
    let (store, generation) = setup(temp.path());
    let p = store.load().unwrap();
    let timing = p.timing(&store).unwrap();
    for kind in 0..5 {
        let mut bad = generation.clone();
        let e = bad.rmvpe_pitch.as_mut().unwrap();
        match kind {
            0 => e.model_sha256 = "0".repeat(64),
            1 => e.implementation_sha256 = "1".repeat(64),
            2 => e.audio_sha256 = "2".repeat(64),
            3 => e.frames[10].time_seconds += 0.003,
            _ => e.frames[10].frequency_hz = Some(f64::NAN),
        }
        assert!(analysis::propose(&p, &timing, &bad, None).is_err());
    }
    assert!(
        chart_engine::vocal_rmvpe::prepare(&temp.path().join("absent"), false, &|| false).is_err()
    );
    let model_dir = temp.path().join("bad-model");
    fs::create_dir(&model_dir).unwrap();
    fs::write(model_dir.join("rmvpe.onnx"), b"incorrect weight bytes").unwrap();
    assert!(chart_engine::vocal_rmvpe::prepare(&model_dir, false, &|| false).is_err());
}

#[test]
fn adaptive_feedback_recovers_supported_gaps_without_words_and_preserves_authored_notes() {
    let temp = tempfile::tempdir().unwrap();
    let (store, mut generation) = setup(temp.path());
    let before = store.load().unwrap();
    let timing = before.timing(&store).unwrap();
    observations(
        &mut generation,
        &[(0.1, 0.45, 57.), (3.25, 3.65, 57.)],
        false,
    );
    let pcm = (0..64000)
        .map(|i| {
            let t = i as f64 / 16000.;
            if (0.1..0.45).contains(&t) || (3.25..3.65).contains(&t) {
                (0.2 * (std::f64::consts::TAU * 220. * t).sin()) as f32
            } else {
                0.
            }
        })
        .collect::<Vec<_>>();
    let old = generation.rmvpe_pitch.take().unwrap();
    let mut raw = chart_engine::vocal_rmvpe::finish_pcm(&pcm, old.frames).unwrap();
    raw.audio_sha256 = old.audio_sha256;
    raw.vocal_stem_sha256 = old.vocal_stem_sha256;
    generation.rmvpe_pitch = Some(raw);
    let stereo = pcm.iter().flat_map(|s| [*s, *s]).collect::<Vec<_>>();
    let base = analysis::propose(&before, &timing, &generation, None).unwrap();
    assert!(base.section.repairs.is_empty());
    let mut review = chart_engine::vocal_adaptive::Review::new(
        &before,
        &timing,
        &generation,
        None,
        pcm,
        &stereo,
        &stereo,
    )
    .unwrap();
    let mut calls = 0;
    while let Some(window) = review.next_window().unwrap() {
        let frames =
            generation.rmvpe_pitch.as_ref().unwrap().frames[window.start..window.end].to_vec();
        // A caller cannot inject a different source-clock window.
        if calls == 0 {
            let mut bad = window.clone();
            bad.end += 1;
            assert!(review.complete(&bad, frames.clone()).is_err());
        }
        review.complete(&window, frames).unwrap();
        calls += 1;
        assert!(calls <= 100);
    }
    let result = review.finish().unwrap();
    assert!(calls > 1);
    assert!(!result.section.repairs.is_empty());
    assert!(result.adaptive.as_ref().unwrap().remaining_residual_frames > 0);
    let mut forged = result.clone();
    forged.adaptive.as_mut().unwrap().implementation_sha256 = "0".repeat(64);
    assert!(forged.validate(&before, &timing).is_err());
    let restored: analysis::Analysis =
        serde_json::from_slice(&serde_json::to_vec(&result).unwrap()).unwrap();
    restored.validate(&before, &timing).unwrap();
    let indices = (0..result.section.repairs.len()).collect();
    apply(&store, Action::SetVocalPitchAnalysis { analysis: result }).unwrap();
    apply(
        &store,
        Action::ApplyVocalSectionRepairs {
            indices,
            include_pitch: false,
        },
    )
    .unwrap();
    let after = store.load().unwrap();
    let chart = after.content.vocals.as_ref().unwrap();
    for authored in &before.content.vocals.as_ref().unwrap().notes {
        assert!(chart.notes.contains(authored));
    }
    assert!(chart.notes.len() > before.content.vocals.as_ref().unwrap().notes.len());
    for note in chart
        .notes
        .iter()
        .filter(|n| !before.content.vocals.as_ref().unwrap().notes.contains(n))
    {
        assert!(
            analysis::current_evidence(&after, &timing, note)
                .is_some_and(|e| e.uncertain && !e.reasons.is_empty())
        );
    }
    let bytes = chart_engine::vocals::append_midi(
        &chart_engine::midi::new_midi(&timing, &Default::default()).unwrap(),
        chart,
    )
    .unwrap();
    let imported = chart_engine::vocals::import_midi(&bytes).unwrap();
    assert_eq!(imported.notes, chart.notes);
    apply(&store, Action::Undo).unwrap();
    assert_eq!(store.load().unwrap().content.vocals, before.content.vocals);
    apply(&store, Action::Redo).unwrap();
    assert_eq!(store.load().unwrap().content.vocals, after.content.vocals);
    // Updating the adaptive recipe must not make previously added drafts
    // appear certain. Old proposals cannot be applied, but their witnesses
    // remain visible and survive a subsequent current check.
    let mut earlier = after.clone();
    earlier
        .content
        .vocal_pitch_analysis
        .as_mut()
        .unwrap()
        .adaptive
        .as_mut()
        .unwrap()
        .implementation_sha256 = "a".repeat(64);
    fixture::save(&store, &earlier);
    for note in chart
        .notes
        .iter()
        .filter(|n| !before.content.vocals.as_ref().unwrap().notes.contains(n))
    {
        let e = analysis::current_evidence(&earlier, &timing, note).unwrap();
        assert!(
            e.uncertain
                && e.reasons
                    .iter()
                    .any(|r| r.contains("earlier analysis implementation"))
        );
    }
    // A normal phrase check must not erase uncertainty from deep repairs.
    let next = analysis::propose(
        &after,
        &timing,
        &generation,
        Some(TimeRange {
            start: 1.5,
            end: 2.,
        }),
    )
    .unwrap();
    apply(&store, Action::SetVocalPitchAnalysis { analysis: next }).unwrap();
    let checked = store.load().unwrap();
    for note in chart
        .notes
        .iter()
        .filter(|n| !before.content.vocals.as_ref().unwrap().notes.contains(n))
    {
        assert!(
            analysis::current_evidence(&checked, &timing, note).is_some_and(|e| {
                e.uncertain
                    && e.reasons
                        .iter()
                        .any(|r| r.contains("earlier analysis implementation"))
            })
        );
    }
}

#[test]
fn adaptive_short_clip_cannot_confirm_itself_by_changing_output_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    let (store, mut generation) = setup(temp.path());
    let mut p = store.load().unwrap();
    p.duration = 0.5;
    let chart = p.content.vocals.as_mut().unwrap();
    chart.notes.truncate(1);
    chart.notes[0].start = 432;
    chart.notes[0].end = 470;
    chart.phrases[0].end = 480;
    p.content.stems.as_mut().unwrap().frames = 22050;
    let timing = p.timing(&store).unwrap();
    generation.duration_seconds = 0.5;
    observations(&mut generation, &[(0.1, 0.4, 57.)], false);
    generation.pitch_frames.truncate(51);
    let pcm = (0..8000)
        .map(|i| {
            let t = i as f64 / 16000.;
            if (0.1..0.4).contains(&t) {
                (0.2 * (std::f64::consts::TAU * 220. * t).sin()) as f32
            } else {
                0.
            }
        })
        .collect::<Vec<_>>();
    let mut old = generation.rmvpe_pitch.take().unwrap();
    old.frames.truncate(51);
    let mut raw = chart_engine::vocal_rmvpe::finish_pcm(&pcm, old.frames).unwrap();
    raw.audio_sha256 = old.audio_sha256;
    raw.vocal_stem_sha256 = old.vocal_stem_sha256;
    generation.rmvpe_pitch = Some(raw);
    let stereo = pcm.iter().flat_map(|s| [*s, *s]).collect::<Vec<_>>();
    let mut review = chart_engine::vocal_adaptive::Review::new(
        &p,
        &timing,
        &generation,
        None,
        pcm,
        &stereo,
        &stereo,
    )
    .unwrap();
    let mut calls = 0;
    while let Some(w) = review.next_window().unwrap() {
        assert_eq!((w.context_start, w.context_end), (0, 51));
        review
            .complete(
                &w,
                generation.rmvpe_pitch.as_ref().unwrap().frames[w.start..w.end].to_vec(),
            )
            .unwrap();
        calls += 1;
    }
    let result = review.finish().unwrap();
    assert!(calls > 1);
    assert!(result.section.repairs.is_empty());
    assert!(result.adaptive.unwrap().remaining_residual_frames > 0);
}
