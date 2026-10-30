use chart_engine::{
    chart::Tracks,
    clean::{AlignedWord, WordTimings},
    midi,
    timing::{Tempo, TempoMap},
    vocal_generation::{self, LyricSource, Policy, Reason, Status},
    vocals::{self, Syllable, VocalTone},
};

const RATE: usize = 22050;
fn identity() -> String {
    "a".repeat(64)
}
fn voice(duration: f64, tones: &[(f64, f64, f64)]) -> Vec<f32> {
    (0..(duration * RATE as f64).round() as usize)
        .map(|index| {
            let time = index as f64 / RATE as f64;
            tones
                .iter()
                .filter(|(start, end, _)| *start <= time && time < *end)
                .map(|(start, end, frequency)| {
                    let envelope = ((time - start) / 0.008)
                        .min((end - time) / 0.008)
                        .clamp(0., 1.);
                    let phase = std::f64::consts::TAU * frequency * time;
                    (envelope * (0.20 * phase.sin() + 0.06 * (2. * phase).sin())) as f32
                })
                .sum()
        })
        .collect()
}
fn words(duration: f64, values: &[(&str, f64, f64)]) -> WordTimings {
    WordTimings {
        version: 1,
        audio_sha256: identity(),
        duration_seconds: duration,
        source: "Synthetic, explicitly supplied original words and known boundaries".into(),
        words: values
            .iter()
            .map(|(text, start, end)| AlignedWord {
                text: Some((*text).into()),
                start: Some(*start),
                end: Some(*end),
            })
            .collect(),
    }
}
#[test]
fn model_free_voice_pitch_and_timed_lyrics_make_real_vocal_midi() {
    let duration = 3.5;
    let signal = voice(
        duration,
        &[
            (0.25, 0.85, 220.),
            (1.05, 1.65, 246.94165),
            (2.6, 3.2, 261.625565),
        ],
    );
    let words = words(
        duration,
        &[("hold", 0.25, 0.85), ("me", 1.05, 1.65), ("near", 2.6, 3.2)],
    );
    let timing = TempoMap::constant(120.).unwrap();
    let generated = vocal_generation::generate_pcm(
        &signal,
        &identity(),
        &"b".repeat(64),
        duration,
        &timing,
        Some(&words),
        None,
        &Policy::default(),
    )
    .unwrap();
    assert_eq!(generated.status, Status::ReviewRequired);
    assert_eq!(
        serde_json::to_value(&generated.timing).unwrap(),
        serde_json::to_value(Some(&timing)).unwrap()
    );
    let chart = generated.chart.unwrap();
    let named: Vec<_> = chart
        .notes
        .iter()
        .filter(|note| note.lyric != Syllable::Missing)
        .collect();
    assert_eq!(named.len(), 3);
    assert_eq!(chart.phrases.len(), 2);
    assert_eq!(
        named.iter().map(|n| n.tone.clone()).collect::<Vec<_>>(),
        vec![
            VocalTone::Pitched { midi: 57 },
            VocalTone::Pitched { midi: 59 },
            VocalTone::Pitched { midi: 60 }
        ]
    );
    assert!((timing.seconds(named[0].start) - 0.25).abs() < 0.035);
    assert!((timing.seconds(named[0].end) - 0.85).abs() < 0.035);
    let original = midi::new_midi(&timing, &Tracks::new()).unwrap();
    let exported = vocals::append_midi(&original, &chart).unwrap();
    let roundtrip = vocals::import_midi(&exported).unwrap();
    assert_eq!(roundtrip.notes, chart.notes);
    assert_eq!(&exported[14..original.len()], &original[14..]);
}

#[test]
fn legacy_generation_report_without_tempo_metadata_is_readable_but_not_timing_attested() {
    let signal = voice(2., &[(0.25, 1.5, 220.)]);
    let timing = TempoMap::constant(125.).unwrap();
    let generated = vocal_generation::generate_pcm(
        &signal,
        &identity(),
        &"b".repeat(64),
        2.,
        &timing,
        Some(&words(2., &[("stay", 0.25, 1.5)])),
        None,
        &Policy::default(),
    )
    .unwrap();
    let mut value = serde_json::to_value(&generated).unwrap();
    value.as_object_mut().unwrap().remove("timing");
    value.as_object_mut().unwrap().remove("neural_pitch");
    let legacy: vocal_generation::Generation = serde_json::from_value(value).unwrap();
    assert!(legacy.timing.is_none() && legacy.neural_pitch.is_none());
    assert_eq!(legacy.chart, generated.chart);
}
#[test]
fn no_words_does_not_fabricate_playable_lyrics_and_silence_is_empty() {
    let timing = TempoMap::constant(120.).unwrap();
    let lyric_source = LyricSource {
        synced_lyrics: None,
        text: "A full line cannot supply individual word times".into(),
        source: "Supplied test text".into(),
    };
    let signal = voice(1.0, &[(0.1, 0.9, 220.)]);
    let generated = vocal_generation::generate_pcm(
        &signal,
        &identity(),
        &identity(),
        1.0,
        &timing,
        None,
        Some(&lyric_source),
        &Policy::default(),
    )
    .unwrap();
    assert_eq!(generated.status, Status::NeedsLyrics);
    let chart = generated.chart.as_ref().unwrap();
    assert!(!chart.notes.is_empty());
    assert!(
        chart
            .notes
            .iter()
            .all(|note| note.lyric == Syllable::Missing)
    );
    let source = midi::new_midi(&timing, &Tracks::new()).unwrap();
    assert_eq!(
        vocals::import_midi(&vocals::append_midi(&source, chart).unwrap())
            .unwrap()
            .notes,
        chart.notes
    );
    assert!(!generated.pitch_candidates.is_empty());
    assert!(
        generated
            .issues
            .iter()
            .any(|i| i.reason == Reason::MissingLyrics)
    );
    assert_eq!(generated.lyrics.unwrap().text, lyric_source.text);
    let empty = vocal_generation::generate_pcm(
        &vec![0.; RATE],
        &identity(),
        &identity(),
        1.0,
        &timing,
        None,
        None,
        &Policy::default(),
    )
    .unwrap();
    assert_eq!(empty.status, Status::NoVoicedPitch);
    assert!(empty.chart.is_none() && empty.pitch_candidates.is_empty());
}
#[test]
fn words_and_multiple_pitches_keep_word_ownership_and_tempo_changes() {
    let signal = voice(2.0, &[(0.2, 0.7, 220.), (0.7, 1.3, 261.625565)]);
    let words = words(2.0, &[("stay", 0.2, 1.3)]);
    let timing = TempoMap::new(
        192,
        0.1,
        vec![
            Tempo { tick: 0, bpm: 120. },
            Tempo {
                tick: 192,
                bpm: 90.,
            },
        ],
    )
    .unwrap();
    let generated = vocal_generation::generate_pcm(
        &signal,
        &identity(),
        &identity(),
        2.0,
        &timing,
        Some(&words),
        None,
        &Policy::default(),
    )
    .unwrap();
    let chart = generated.chart.unwrap();
    assert_eq!(chart.notes.len(), 2);
    assert!(matches!(&chart.notes[0].lyric, Syllable::Text { text, .. } if text == "stay"));
    // Rejected transition frames do not establish a connecting pitch slide.
    let gap = chart.notes[1].start - chart.notes[0].end;
    assert_eq!(
        chart.notes[1].lyric,
        if gap <= 1 {
            Syllable::Continue
        } else {
            Syllable::ContinueWord
        }
    );
    if gap > 1 {
        assert!(
            generated
                .issues
                .iter()
                .any(|i| i.reason == Reason::UnsupportedPitchGap)
        );
    }
    assert!((timing.seconds(chart.notes[1].start) - 0.7).abs() < 0.05);
    assert!((timing.seconds(chart.notes[1].end) - 1.3).abs() < 0.05);
    assert!(
        generated
            .issues
            .iter()
            .any(|i| i.reason == Reason::WordLevelAlignment)
    );
}
#[test]
fn exact_lyrics_reconciliation_preserves_unknown_boundaries() {
    let recognized = words(
        3.0,
        &[
            ("I", 0.1, 0.3),
            ("can", 0.4, 0.6),
            ("sing", 0.7, 1.0),
            ("now", 1.2, 1.5),
        ],
    );
    let lyrics = LyricSource {
        synced_lyrics: None,
        text: "I can really sing now".into(),
        source: "Supplied corrected words".into(),
    };
    let result = vocal_generation::reconcile_lyrics(&recognized, &lyrics).unwrap();
    assert_eq!(result.words.len(), 5);
    assert_eq!(result.words[2].text.as_deref(), Some("really"));
    assert!(result.words[2].start.is_none() && result.words[2].end.is_none());
    assert_eq!(result.words[3].start, Some(0.7));
    assert_eq!(result.words[4].end, Some(1.5));
}

#[test]
fn provider_omissions_keep_every_recognized_timestamp_and_unknown_provider_word() {
    let recognized = words(
        4.0,
        &[
            ("news", 0.1, 0.4),
            ("sonríe", 0.6, 1.0),
            ("life", 1.2, 1.5),
            ("life", 2.0, 2.3),
            ("outro", 3.0, 3.4),
        ],
    );
    let lyrics = LyricSource {
        text: "sonríe at life".into(),
        source: "Different lyric arrangement".into(),
        synced_lyrics: None,
    };
    let reconciled = vocal_generation::reconcile_lyrics(&recognized, &lyrics).unwrap();
    let timed: Vec<_> = reconciled
        .words
        .iter()
        .filter(|w| w.start.is_some())
        .collect();
    assert_eq!(timed, recognized.words.iter().collect::<Vec<_>>());
    let supplied = reconciled
        .words
        .iter()
        .find(|w| w.text.as_deref() == Some("at"))
        .unwrap();
    assert_eq!((supplied.start, supplied.end), (None, None));
    // Reconciliation never moves the later repetition onto an earlier word.
    assert_eq!(timed[3].start, Some(2.0));
}

#[test]
fn lyric_disagreement_preserves_measured_notes_and_requires_review() {
    let duration = 2.0;
    let signal = voice(
        duration,
        &[
            (0.2, 0.6, 220.0),
            (0.8, 1.2, 246.94165),
            (1.4, 1.8, 261.625565),
        ],
    );
    let recognized = words(
        duration,
        &[("one", 0.2, 0.6), ("extra", 0.8, 1.2), ("three", 1.4, 1.8)],
    );
    let lyrics = LyricSource {
        text: "one three".into(),
        source: "Supplied lyric text".into(),
        synced_lyrics: None,
    };
    let timing = TempoMap::constant(120.0).unwrap();
    let report = vocal_generation::generate_pcm(
        &signal,
        &identity(),
        &"b".repeat(64),
        duration,
        &timing,
        Some(&recognized),
        Some(&lyrics),
        &Policy::default(),
    )
    .unwrap();
    assert_eq!(report.status, Status::ReviewRequired);
    let review = chart_engine::vocal_review::VocalReviewMap::from_generation(&report).unwrap();
    assert!(
        review
            .regions
            .iter()
            .flat_map(|region| &region.issues)
            .any(|issue| issue.reason == Reason::LyricDisagreement)
    );
    let chart = report.chart.unwrap();
    let named: Vec<_> = chart
        .notes
        .iter()
        .filter(|note| note.lyric != Syllable::Missing)
        .collect();
    assert_eq!(named.len(), 3);
    assert!(matches!(&named[1].lyric,Syllable::Text{text,..} if text=="extra"));
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.reason == Reason::LyricDisagreement
                && issue.start_seconds == 0.8
                && issue.end_seconds == 1.2)
    );
}

#[test]
fn differing_spelling_never_borrows_a_recognized_timestamp() {
    let recognized = words(1.0, &[("sonríe", 0.2, 0.7)]);
    let lyrics = LyricSource {
        text: "sonrie".into(),
        source: "Unverified spelling".into(),
        synced_lyrics: None,
    };
    let result = vocal_generation::reconcile_lyrics(&recognized, &lyrics).unwrap();
    assert_eq!(
        result
            .words
            .iter()
            .find(|w| w.text.as_deref() == Some("sonríe"))
            .unwrap()
            .start,
        Some(0.2)
    );
    assert!(
        result
            .words
            .iter()
            .find(|w| w.text.as_deref() == Some("sonrie"))
            .unwrap()
            .start
            .is_none()
    );
}

#[test]
fn repeated_words_rearticulate_without_turning_vibrato_into_extra_notes() {
    let signal: Vec<_> = (0..RATE)
        .map(|index| {
            let time = index as f64 / RATE as f64;
            if !(0.1..0.9).contains(&time) {
                return 0.;
            }
            // Frequency modulation of +/-40 cents around A3, integrated phase.
            let phase = std::f64::consts::TAU * 220. * time
                - 0.40 * (std::f64::consts::TAU * 5. * time).cos();
            (0.2 * phase.sin() + 0.06 * (2. * phase).sin()) as f32
        })
        .collect();
    let words = words(1.0, &[("stay", 0.1, 0.5), ("stay", 0.5, 0.9)]);
    let generated = vocal_generation::generate_pcm(
        &signal,
        &identity(),
        &identity(),
        1.0,
        &TempoMap::constant(120.).unwrap(),
        Some(&words),
        None,
        &Policy::default(),
    )
    .unwrap();
    let chart = generated.chart.unwrap();
    let named: Vec<_> = chart
        .notes
        .iter()
        .filter(|note| note.lyric != Syllable::Missing)
        .collect();
    assert_eq!(named.len(), 2);
    assert!(
        named
            .iter()
            .all(|note| matches!(&note.lyric, Syllable::Text { text, .. } if text == "stay"))
    );
    assert!(
        chart
            .notes
            .iter()
            .all(|note| note.tone == VocalTone::Pitched { midi: 57 })
    );
}

#[test]
fn cancelled_vocal_request_never_reads_or_publishes_a_chart() {
    use chart_engine::clean::StemAlignment;
    let missing = std::path::Path::new("a-recording-that-is-not-opened.ogg");
    let binding = StemAlignment {
        audio_sha256: identity(),
        stem_sha256: identity(),
        evidence: "test".into(),
    };
    let timing = TempoMap::constant(120.).unwrap();
    let result = vocal_generation::generate_cancellable(
        vocal_generation::Request {
            audio: missing,
            vocal_stem: missing,
            alignment: &binding,
            timing: &timing,
            words: None,
            lyrics: None,
            policy: &Policy::default(),
            neural_pitch: None,
            vocal_pitch: None,
        },
        &|| true,
    );
    assert!(result.unwrap_err().to_string().contains("cancelled"));
}
#[test]
fn mismatched_sources_clocks_and_unvoiced_words_are_not_accepted() {
    let timing = TempoMap::constant(120.).unwrap();
    let signal = voice(1.0, &[(0.1, 0.9, 220.)]);
    let mut wrong = words(1.0, &[("hey", 0.1, 0.9)]);
    wrong.audio_sha256 = "c".repeat(64);
    assert!(
        vocal_generation::generate_pcm(
            &signal,
            &identity(),
            &identity(),
            1.0,
            &timing,
            Some(&wrong),
            None,
            &Policy::default()
        )
        .is_err()
    );
    assert!(
        vocal_generation::generate_pcm(
            &signal,
            &identity(),
            &identity(),
            1.1,
            &timing,
            None,
            None,
            &Policy::default()
        )
        .is_err()
    );
    let empty = vocal_generation::generate_pcm(
        &vec![0.; RATE],
        &identity(),
        &identity(),
        1.0,
        &timing,
        Some(&words(1.0, &[("hey", 0.1, 0.9)])),
        None,
        &Policy::default(),
    )
    .unwrap();
    assert!(empty.chart.is_none());
    assert!(
        empty
            .issues
            .iter()
            .any(|i| i.reason == Reason::WordWithoutPitch)
    );
    let mut invalid = signal;
    invalid[5] = f32::NAN;
    assert!(
        vocal_generation::generate_pcm(
            &invalid,
            &identity(),
            &identity(),
            1.0,
            &timing,
            None,
            None,
            &Policy::default()
        )
        .is_err()
    );
}
#[test]
fn out_of_range_and_conflicting_word_ownership_are_reviewed_not_transposed() {
    let timing = TempoMap::constant(120.).unwrap();
    let high = voice(1.0, &[(0.1, 0.9, 1200.)]);
    let result = vocal_generation::generate_pcm(
        &high,
        &identity(),
        &identity(),
        1.0,
        &timing,
        Some(&words(1.0, &[("high", 0.1, 0.9)])),
        None,
        &Policy::default(),
    )
    .unwrap();
    assert!(result.chart.is_none());
    assert!(
        result
            .issues
            .iter()
            .any(|i| i.reason == Reason::OutOfRangePitch)
    );
    let normal = voice(1.0, &[(0.1, 0.9, 220.)]);
    let overlapping = words(1.0, &[("one", 0.1, 0.9), ("two", 0.2, 0.8)]);
    let result = vocal_generation::generate_pcm(
        &normal,
        &identity(),
        &identity(),
        1.0,
        &timing,
        Some(&overlapping),
        None,
        &Policy::default(),
    )
    .unwrap();
    assert!(
        result
            .chart
            .unwrap()
            .notes
            .iter()
            .all(|note| note.lyric == Syllable::Missing)
    );
}

/// Explicit local integration: no copyrighted recording is part of the suite.
#[test]
#[ignore = "requires explicitly supplied recording, aligned vocal stem and timed words"]
fn local_recording_generates_reviewable_game_vocals() {
    use chart_engine::{chart::Chart, clean::StemAlignment, models};
    use std::{fs, path::PathBuf};
    let song = PathBuf::from(std::env::var("CHART_ENGINE_VOCAL_SONG").unwrap());
    let stem = PathBuf::from(std::env::var("CHART_ENGINE_VOCAL_STEM").unwrap());
    let words_path = PathBuf::from(std::env::var("CHART_ENGINE_VOCAL_WORDS").unwrap());
    let output = PathBuf::from(std::env::var("CHART_ENGINE_VOCAL_OUTPUT").unwrap());
    assert!(
        !output.exists(),
        "integration output must be a new recovery directory"
    );
    let audio = song.join("song.ogg");
    let chart = Chart::parse(fs::read_to_string(song.join("notes.chart")).unwrap()).unwrap();
    let words: WordTimings = serde_json::from_slice(&fs::read(words_path).unwrap()).unwrap();
    let binding = StemAlignment {
        audio_sha256: models::hash_file(&audio).unwrap(), stem_sha256: models::hash_file(&stem).unwrap(),
        evidence: "Previously audited full-length six-stem cache; historical alignment attestation; independent lead-vocal accuracy is not established".into(),
    };
    let generated = vocal_generation::generate(vocal_generation::Request {
        audio: &audio,
        vocal_stem: &stem,
        alignment: &binding,
        timing: &chart.timing,
        words: Some(&words),
        lyrics: None,
        policy: &Policy::default(),
        neural_pitch: None,
        vocal_pitch: None,
    })
    .unwrap();
    let vocals = generated
        .chart
        .as_ref()
        .expect("actual timed words must overlap voiced pitch in this explicit fixture");
    vocals.validate().unwrap();
    fs::create_dir(&output).unwrap();
    fs::write(
        output.join("vocal-generation.json"),
        serde_json::to_vec_pretty(&generated).unwrap(),
    )
    .unwrap();
    fs::write(
        output.join("vocals.json"),
        serde_json::to_vec_pretty(vocals).unwrap(),
    )
    .unwrap();
    if chart.timing.offset == 0. {
        let source = midi::new_midi(&chart.timing, &Tracks::new()).unwrap();
        let exported = vocals::append_midi(&source, vocals).unwrap();
        assert_eq!(vocals::import_midi(&exported).unwrap().notes, vocals.notes);
        fs::write(output.join("notes.mid"), exported).unwrap();
    }
    eprintln!(
        "real vocal integration: {} notes, {} phrases, {} pitch candidates, {} review issues; output {}",
        vocals.notes.len(),
        vocals.phrases.len(),
        generated.pitch_candidates.len(),
        generated.issues.len(),
        output.display()
    );
}
