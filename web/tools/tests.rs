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
