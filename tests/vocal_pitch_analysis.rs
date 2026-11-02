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
