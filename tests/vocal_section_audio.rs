use chart_engine::{
    editor::TimeRange,
    timing::TempoMap,
    vocal_section_audio::{RATE, wave},
    vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};
fn chart() -> VocalChart {
    VocalChart {
        version: 1,
        resolution: 480,
        source: "controlled comparison".into(),
        phrases: vec![VocalPhrase {
            midi: None,
            start: 0,
            end: 960,
            player: 1,
        }],
        notes: vec![VocalNote {
            start: 192,
            end: 768,
            tone: VocalTone::Pitched { midi: 69 },
            lyric: Syllable::Missing,
            midi: None,
        }],
    }
}
fn sample(bytes: &[u8], frame: usize, channel: usize) -> i16 {
    let i = 44 + frame * 4 + channel * 2;
    i16::from_le_bytes([bytes[i], bytes[i + 1]])
}
#[test]
fn stereo_comparison_places_recording_and_a440_on_one_clock_without_tones_in_gaps() {
    let pcm = vec![0.25; RATE as usize];
    let timing = TempoMap::constant(120.).unwrap();
    let bytes = wave(&pcm, &chart(), &timing, TimeRange { start: 0., end: 1. }).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(&bytes[8..16], b"WAVEfmt ");
    assert_eq!(bytes.len(), 44 + RATE as usize * 4);
    assert_eq!(sample(&bytes, 1000, 0), 8191);
    assert_eq!(sample(&bytes, 1000, 1), 0);
    assert_eq!(sample(&bytes, 20000, 1), 0);
    let first = (0.3 * RATE as f64) as usize;
    let last = (0.7 * RATE as f64) as usize;
    let cycles = (first + 1..last)
        .filter(|i| sample(&bytes, *i - 1, 1) <= 0 && sample(&bytes, *i, 1) > 0)
        .count();
    assert!((175..=177).contains(&cycles));
    let cropped = wave(
        &pcm,
        &chart(),
        &timing,
        TimeRange {
            start: 0.3,
            end: 0.7,
        },
    )
    .unwrap();
    for i in 0..(last - first) {
        assert_eq!(sample(&cropped, i, 1), sample(&bytes, first + i, 1));
    }
}
#[test]
fn preview_rejects_nonfinite_audio_invalid_clocks_and_unbounded_clips() {
    let mut pcm = vec![0.; RATE as usize];
    let timing = TempoMap::constant(120.).unwrap();
    for range in [
        TimeRange {
            start: 0.,
            end: 31.,
        },
        TimeRange {
            start: 0.7,
            end: 0.3,
        },
        TimeRange {
            start: f64::NAN,
            end: 1.,
        },
        TimeRange { start: 0., end: 2. },
    ] {
        assert!(wave(&pcm, &chart(), &timing, range).is_err());
    }
    pcm[1000] = f32::NAN;
    assert!(wave(&pcm, &chart(), &timing, TimeRange { start: 0., end: 1. }).is_err());
    let bad = TempoMap::new(960, 0., timing.tempos).unwrap();
    assert!(
        wave(
            &vec![0.; RATE as usize],
            &chart(),
            &bad,
            TimeRange { start: 0., end: 1. }
        )
        .is_err()
    );
}
