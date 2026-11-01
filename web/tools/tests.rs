use super::audio_decode::{self, Pcm};

#[test]
#[ignore = "Requires YACE_AUDIO_EXPORT_ZIP from actual browser generation"]
fn actual_float_audio_archive_compression_profile() {
    use std::io::{Cursor, Read, Write};
    let input =
        std::fs::File::open(std::env::var_os("YACE_AUDIO_EXPORT_ZIP").expect("actual export"))
            .unwrap();
    let mut archive = zip::ZipArchive::new(input).unwrap();
    let mut pcm = Vec::new();
    archive
        .by_name("drums.wav")
        .unwrap()
        .read_to_end(&mut pcm)
        .unwrap();
    for level in [1, 3, 6] {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(
            "drums.wav",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .compression_level(Some(level)),
        )
        .unwrap();
        zip.write_all(&pcm).unwrap();
        let result = zip.finish().unwrap().into_inner();
        eprintln!("level {level}: {} -> {} bytes", pcm.len(), result.len());
    }
}

#[test]
fn imported_game_archive_shares_its_identical_recovery_audio() {
    use super::archive_codec;
    let audio = vec![19u8; 65536];
    let bytes = archive_codec::encode([
        ("song.ini", b"[song]\nname=test\n".as_slice()),
        ("guitar.ogg", audio.as_slice()),
        (".yace/audio/stem.ogg", audio.as_slice()),
    ])
    .unwrap();
    let files = archive_codec::decode_shared(&bytes).unwrap();
    assert!(std::sync::Arc::ptr_eq(&files[1].1, &files[2].1));
    assert_eq!(*files[1].1, audio);
    assert_eq!(
        archive_codec::strip_wrapper(archive_codec::decode(&bytes).unwrap(), "song.ini")
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn shared_archive_import_checks_the_complete_entry_crc() {
    use super::archive_codec;
    let mut bytes = archive_codec::encode([("recording.ogg", [19u8; 64].as_slice())]).unwrap();
    let offset = {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        zip.by_index(0).unwrap().data_start().unwrap() as usize
    };
    bytes[offset] ^= 1;
    assert!(archive_codec::decode_shared(&bytes).is_err());
}

#[test]
fn shared_archive_import_rejects_paths_outside_the_project() {
    use std::io::{Cursor, Write};
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("../outside", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"no").unwrap();
    assert!(super::archive_codec::decode_shared(&zip.finish().unwrap().into_inner()).is_err());
}

#[test]
#[ignore = "Requires YACE_BROWSER_TRANSCRIPTION actual app download and YACE_NATIVE_TRANSCRIPTION reference"]
fn actual_browser_transcription_matches_native_notes_and_source_clocks() {
    use serde_json::Value;
    use std::fs;
    let read = |key| -> Value {
        serde_json::from_slice(
            &fs::read(std::env::var_os(key).expect("Explicit real model output required")).unwrap(),
        )
        .unwrap()
    };
    let browser = read("YACE_BROWSER_TRANSCRIPTION");
    let native = read("YACE_NATIVE_TRANSCRIPTION");
    assert_eq!(
        browser["isolation"]["source_audio_sha256"],
        native["isolation"]["source_audio_sha256"]
    );
    for instrument in ["drums", "guitar", "bass"] {
        let ours = browser[instrument].as_array().unwrap();
        let theirs = native[instrument].as_array().unwrap();
        assert_eq!(ours.len(), theirs.len(), "{instrument} detection count");
        let mut max_time = 0f64;
        let mut max_score = 0f64;
        for (a, b) in ours.iter().zip(theirs) {
            for field in if instrument == "drums" {
                vec!["time"]
            } else {
                vec!["start", "end"]
            } {
                let delta = (a[field].as_f64().unwrap() - b[field].as_f64().unwrap()).abs();
                max_time = max_time.max(delta);
                assert!(delta < 1e-9, "{instrument} {field} differs: {a} / {b}");
            }
            let identity = if instrument == "drums" {
                "voice"
            } else {
                "pitch"
            };
            assert_eq!(a[identity], b[identity], "{instrument} note identity");
            let delta =
                (a["confidence"].as_f64().unwrap() - b["confidence"].as_f64().unwrap()).abs();
            max_score = max_score.max(delta);
            assert!(delta < 0.002, "{instrument} score differs");
        }
        eprintln!(
            "{instrument}: {} notes, maximum time delta {max_time}, maximum score delta {max_score}",
            ours.len()
        );
    }
}

#[test]
fn analysis_speaker_matrix_matches_the_native_float_decoder() {
    use std::{fs, process::Command};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stereo.wav");
    let pcm = Pcm {
        stereo: (0..4410)
            .flat_map(|i| {
                [
                    ((i as f32) * 0.03).sin() * 0.8,
                    ((i as f32) * 0.07).sin() * 0.4,
                ]
            })
            .collect(),
        rate: 44100,
        channels: 2,
    };
    fs::write(&path, pcm.float_wave().unwrap()).unwrap();
    let result = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&path)
        .args(["-ac", "1", "-f", "f32le", "pipe:1"])
        .output()
        .unwrap();
    assert!(result.status.success());
    let native: Vec<_> = result
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let ours = pcm.samples(44100, 1).unwrap();
    assert_eq!(ours.len(), native.len());
    assert!(
        ours.iter()
            .zip(native)
            .all(|(a, b)| (a - b).abs() < 0.000001)
    );
    let mono = Pcm {
        stereo: vec![0.25; 200],
        rate: 44100,
        channels: 1,
    };
    assert!(mono.samples(44100, 1).unwrap().iter().all(|s| *s == 0.25));
    assert!(
        mono.samples(44100, 2)
            .unwrap()
            .iter()
            .all(|s| (*s - 0.25 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-8)
    );
}

#[test]
#[ignore = "Requires explicit YACE_NATIVE_STEMS and the actual browser download YACE_BROWSER_STEMS_ZIP"]
fn actual_browser_separation_matches_the_native_model_and_source_clock() {
    use sha2::{Digest, Sha256};
    use std::{fs, io::Read, path::PathBuf};
    let native =
        PathBuf::from(std::env::var_os("YACE_NATIVE_STEMS").expect("native reference folder"));
    let browser =
        std::env::var_os("YACE_BROWSER_STEMS_ZIP").expect("actual browser-generated download");
    let mut zip = zip::ZipArchive::new(fs::File::open(browser).unwrap()).unwrap();
    let manifest = if zip
        .file_names()
        .any(|name| name.is_ok_and(|name| name == "stem-manifest.json"))
    {
        "stem-manifest.json"
    } else {
        "manifest.json"
    };
    let mut read = |name: &str| {
        let mut bytes = Vec::new();
        zip.by_name(name).unwrap().read_to_end(&mut bytes).unwrap();
        bytes
    };
    let ours: serde_json::Value = serde_json::from_slice(&read(manifest)).unwrap();
    let theirs: serde_json::Value =
        serde_json::from_slice(&fs::read(native.join("stem-manifest.json")).unwrap()).unwrap();
    assert_eq!(ours["source_audio_sha256"], theirs["source_audio_sha256"]);
    assert_eq!(
        ours["generation"]["model_sha256"],
        theirs["generation"]["model_sha256"]
    );
    assert_eq!(ours["frames"], theirs["frames"]);
    assert_eq!(ours["sample_rate"], 44100);
    assert_eq!(ours["channels"], 2);
    assert_eq!(ours["stems"].as_array().unwrap().len(), 6);
    for name in ["drums", "bass", "vocals", "guitar", "piano", "other"] {
        let bytes = read(&format!("{name}.wav"));
        let asset = ours["stems"]
            .as_array()
            .unwrap()
            .iter()
            .find(|asset| asset["id"] == name)
            .unwrap();
        assert_eq!(
            asset["sha256"].as_str().unwrap(),
            format!("{:x}", Sha256::digest(&bytes))
        );
        let ours = ready(audio_decode::decode(&bytes)).unwrap();
        let reference = theirs["stems"]
            .as_array()
            .unwrap()
            .iter()
            .find(|asset| asset["id"] == name)
            .unwrap()["path"]
            .as_str()
            .unwrap();
        let path = PathBuf::from(reference);
        let path = if path.is_absolute() {
            path
        } else {
            native.join(path)
        };
        let theirs = ready(audio_decode::decode(&fs::read(path).unwrap())).unwrap();
        assert_eq!(ours.rate, theirs.rate);
        assert_eq!(ours.stereo.len(), theirs.stereo.len());
        assert!(ours.stereo.iter().all(|sample| sample.is_finite()));
        let maximum = ours
            .stereo
            .iter()
            .zip(&theirs.stereo)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        let square: f64 = ours
            .stereo
            .iter()
            .zip(&theirs.stereo)
            .map(|(a, b)| ((*a - *b) as f64).powi(2))
            .sum();
        let energy: f64 = theirs
            .stereo
            .iter()
            .map(|sample| (*sample as f64).powi(2))
            .sum();
        let normalized = (square / energy.max(1e-20)).sqrt();
        eprintln!(
            "{name}: {} frames; max delta {maximum}; normalized RMS delta {normalized}",
            ours.stereo.len() / 2
        );
        assert!(
            maximum < 0.0002 && normalized < 0.001,
            "Browser/native separation differs for {name}"
        );
    }
}

#[test]
fn real_mp3_tags_and_embedded_cover_are_read_without_a_native_probe() {
    use std::{fs, process::Command};
    let root = tempfile::tempdir().unwrap();
    let wav = root.path().join("tone.wav");
    let cover = root.path().join("cover.png");
    let mp3 = root.path().join("tagged.mp3");
    let pcm = Pcm {
        stereo: (0..44100 * 2)
            .flat_map(|n| {
                let v = (n as f32 * 0.07).sin() * 1.3;
                [v, -v]
            })
            .collect(),
        rate: 44100,
        channels: 2,
    };
    fs::write(&wav, pcm.float_wave().unwrap()).unwrap();
    let picture = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=32x32",
            "-frames:v",
            "1",
            "-y",
        ])
        .arg(&cover)
        .output()
        .unwrap();
    assert!(
        picture.status.success(),
        "{}",
        String::from_utf8_lossy(&picture.stderr)
    );
    let encoded = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&wav)
        .arg("-i")
        .arg(&cover)
        .args([
            "-map",
            "0:a",
            "-map",
            "1:v",
            "-c:a",
            "libmp3lame",
            "-q:a",
            "4",
            "-c:v",
            "copy",
            "-disposition:v",
            "attached_pic",
            "-id3v2_version",
            "3",
            "-metadata",
            "title=Tagged recording fixture",
            "-metadata",
            "artist=YACE engineering fixture",
            "-metadata",
            "album=Synthetic album",
            "-metadata",
            "date=2024",
            "-metadata",
            "genre=Test",
            "-y",
        ])
        .arg(&mp3)
        .output()
        .unwrap();
    assert!(
        encoded.status.success(),
        "{}",
        String::from_utf8_lossy(&encoded.stderr)
    );
    let bytes = fs::read(&mp3).unwrap();
    let tags = super::recording_tags::read(&bytes).unwrap();
    assert_eq!(
        (
            tags.metadata.name.as_str(),
            tags.metadata.artist.as_str(),
            tags.metadata.album.as_str(),
            tags.metadata.year.as_str(),
            tags.metadata.genre.as_str()
        ),
        (
            "Tagged recording fixture",
            "YACE engineering fixture",
            "Synthetic album",
            "2024",
            "Test"
        )
    );
    assert_eq!(tags.artwork.unwrap(), fs::read(&cover).unwrap());
    let decoded = ready(audio_decode::decode(&bytes)).unwrap();
    let peak = decoded
        .stereo
        .iter()
        .copied()
        .map(f32::abs)
        .fold(0., f32::max);
    assert!(peak > 1.1, "MP3 float output was silently clipped: {peak}");
    let native = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&mp3)
        .args(["-f", "f32le", "-ac", "2", "-ar", "44100", "-"])
        .output()
        .unwrap();
    assert!(native.status.success());
    let reference: Vec<_> = native
        .stdout
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect();
    assert_eq!(reference.len(), decoded.stereo.len());
    let difference = reference
        .iter()
        .zip(&decoded.stereo)
        .map(|(a, b)| (a - b).abs())
        .fold(0., f32::max);
    assert!(
        difference < 0.0002,
        "MP3 native/browser sample difference: {difference}"
    );
    assert_eq!(
        decoded.stereo.len(),
        44100 * 2 * 2,
        "MP3 priming/padding must not change the source clock"
    );
    assert_eq!(
        fs::read(&mp3).unwrap(),
        bytes,
        "Reading tags/audio must preserve the source file"
    );
    if let Some(output) = std::env::var_os("YACE_TAG_FIXTURE_OUT") {
        let output = std::path::PathBuf::from(output);
        fs::create_dir_all(&output).unwrap();
        fs::copy(mp3, output.join("Tagged recording fixture.mp3")).unwrap();
        fs::copy(cover, output.join("tagged-cover.png")).unwrap();
    }
}

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
            .all(|s| (*s + 0.125 * std::f32::consts::SQRT_2).abs() < 1e-6)
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
