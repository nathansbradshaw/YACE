use super::audio_decode::{self, Pcm};

fn ready<T>(future: impl std::future::Future<Output = T>) -> T {
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    match std::pin::pin!(future).as_mut().poll(&mut context) {
        std::task::Poll::Ready(value) => value,
        std::task::Poll::Pending => panic!("native audio decoding should complete synchronously"),
    }
}

#[test]
fn playback_wave_keeps_source_clock_stereo_and_sample_polarity() {
    let stereo = (0..48_000)
        .flat_map(|n| {
            let sample = (n as f32 * 0.04).sin() * 0.8;
            [sample, -sample]
        })
        .collect();
    let pcm = Pcm {
        stereo,
        rate: 48_000,
        channels: 2,
    };
    let decoded = ready(audio_decode::decode(&pcm.wave().unwrap())).unwrap();
    assert_eq!(decoded.rate, pcm.rate);
    assert_eq!(decoded.stereo.len(), pcm.stereo.len());
    assert_eq!(decoded.duration(), 1.);
    assert!(
        decoded
            .stereo
            .iter()
            .zip(&pcm.stereo)
            .all(|(a, b)| (a - b).abs() < 0.00005)
    );
}

#[test]
fn corrupt_audio_is_rejected() {
    assert!(ready(audio_decode::decode(b"not an audio recording")).is_err());
}
#[test]
fn vorbis_decoding_preserves_float_headroom_without_moving_the_source_clock() {
    let pcm = ready(audio_decode::decode(include_bytes!(
        "fixtures/headroom.ogg"
    )))
    .unwrap();
    assert_eq!(
        (pcm.rate, pcm.channels, pcm.stereo.len()),
        (44100, 2, 5120 * 2)
    );
    let peak = pcm
        .stereo
        .chunks_exact(2)
        .map(|s| s[0].abs())
        .fold(0f32, f32::max);
    assert!(
        peak > 1.1,
        "Vorbis float output was silently clipped: {peak}"
    );
    assert!(
        pcm.stereo
            .chunks_exact(2)
            .map(|s| s[1].abs())
            .fold(0f32, f32::max)
            < 0.3
    );
}

#[test]
fn processing_export_preserves_float_headroom_and_exact_stereo_samples() {
    let pcm = Pcm {
        stereo: vec![1.25, -1.5, 0.000_000_1, -0.000_000_1, 0., -0.25],
        rate: 44100,
        channels: 2,
    };
    let decoded = ready(audio_decode::decode(&pcm.float_wave().unwrap())).unwrap();
    assert_eq!(decoded.stereo, pcm.stereo);
    assert_eq!((decoded.rate, decoded.channels), (44100, 2));
}

#[test]
fn sample_conversion_preserves_duration_dc_and_stereo_polarity() {
    let pcm = Pcm {
        stereo: (0..4800).flat_map(|_| [0.25, -0.5]).collect(),
        rate: 48000,
        channels: 2,
    };
    let converted = pcm.samples(44100, 2).unwrap();
    assert_eq!(converted.len(), 4410 * 2);
    assert!(
        converted
            .chunks_exact(2)
            .all(|s| (s[0] - 0.25).abs() < 1e-6 && (s[1] + 0.5).abs() < 1e-6)
    );
    assert!(
        pcm.samples(22050, 1)
            .unwrap()
            .iter()
            .all(|s| (*s + 0.125).abs() < 1e-6)
    );
    assert_eq!(pcm.samples(48000, 2).unwrap(), pcm.stereo);
}

#[test]
fn downsampling_filters_frequencies_above_the_analysis_nyquist_limit() {
    let rms = |hz: f32| {
        let pcm = Pcm {
            stereo: (0..4410)
                .flat_map(|n| {
                    let v = (n as f32 * hz * std::f32::consts::TAU / 44100.).sin();
                    [v, v]
                })
                .collect(),
            rate: 44100,
            channels: 2,
        };
        let converted = pcm.samples(22050, 1).unwrap();
        (converted[200..2000]
            .iter()
            .map(|v| (*v as f64).powi(2))
            .sum::<f64>()
            / 1800.)
            .sqrt()
    };
    assert!(rms(1000.) > 0.69);
    assert!(
        rms(15000.) < 0.001,
        "analysis must not alias out-of-band audio into attacks"
    );
}
