use chart_engine::{editor::Transport, models};
#[test]
fn legacy_transport_defaults_to_normal_rate_and_scaled_clock_keeps_source_seconds() {
    let mut transport: Transport = serde_json::from_str(
        r#"{"position":1.0,"playing":true,"anchor_ms":1000,"loop_range":{"start":1.0,"end":3.0}}"#,
    )
    .unwrap();
    assert_eq!(transport.playback_rate, 1.0);
    transport.playback_rate = 0.5;
    assert_eq!(transport.position_at(3000, 10.0), 2.0);
    assert_eq!(transport.position_at(6000, 10.0), 1.5);
    transport.playing = false;
    assert_eq!(transport.position_at(9000, 10.0), 1.0);
}
#[test]
fn pitch_preserving_practice_decode_changes_duration_preserves_stereo_and_source() {
    let temp = tempfile::tempdir().unwrap();
    let audio = temp.path().join("tone.wav");
    let rate = 44100;
    let left: Vec<_> = (0..rate * 3)
        .map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / rate as f32).sin() * 0.2)
        .collect();
    let right: Vec<_> = (0..rate * 3)
        .map(|i| (std::f32::consts::TAU * 220.0 * i as f32 / rate as f32).sin() * 0.2)
        .collect();
    models::write_audio(&audio, &left, &right).unwrap();
    let original = models::hash_file(&audio).unwrap();
    for speed in [0.5, 0.75, 1.0] {
        let data = models::decode_at_tempo(&audio, rate, 2, speed).unwrap();
        let duration = data.len() as f64 / 2.0 / rate as f64;
        assert!(
            (duration - 3.0 / speed).abs() < 0.08,
            "speed={speed} duration={duration}"
        );
        for (channel, expected) in [(0, 440.0), (1, 220.0)] {
            let signal: Vec<_> = data
                .chunks_exact(2)
                .skip(rate as usize / 2)
                .take(rate as usize)
                .map(|c| c[channel])
                .collect();
            let crossings = signal
                .windows(2)
                .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
                .count();
            assert!(
                (crossings as f64 - expected).abs() < 3.0,
                "{speed}: channel{channel} frequency{crossings}"
            );
        }
    }
    assert_eq!(models::hash_file(&audio).unwrap(), original);
    assert!(models::decode_at_tempo(&audio, rate, 2, 0.0).is_err());
}
