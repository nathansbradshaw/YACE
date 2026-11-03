use super::audio_decode::{self, Pcm};

#[test]
#[ignore = "Requires actual YACE_BROWSER_CHANNELS_ZIP download and YACE_CHANNELS_SOURCE real channel fixture with shorter song.wav"]
fn actual_browser_game_channels_preserve_raw_files_and_complete_preview() {
    use sha2::{Digest, Sha256};
    use std::{
        collections::BTreeSet,
        io::{Cursor, Read},
        path::PathBuf,
    };
    fn member(zip: &mut zip::ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        zip.by_name(name).unwrap().read_to_end(&mut bytes).unwrap();
        bytes
    }
    let source = PathBuf::from(
        std::env::var_os("YACE_CHANNELS_SOURCE").expect("Explicit real source required"),
    );
    let mut zip = zip::ZipArchive::new(Cursor::new(
        std::fs::read(
            std::env::var_os("YACE_BROWSER_CHANNELS_ZIP")
                .expect("Actual browser download required"),
        )
        .unwrap(),
    ))
    .unwrap();
    let mut names = BTreeSet::new();
    let mut expected = Vec::<f64>::new();
    let mut backing = 0;
    for entry in std::fs::read_dir(source).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "wav") {
            continue;
        }
        let name = path.file_name().unwrap().to_str().unwrap();
        let original = std::fs::read(&path).unwrap();
        assert_eq!(
            member(&mut zip, name),
            original,
            "Raw game channel changed: {name}"
        );
        let pcm = ready(audio_decode::decode(&original)).unwrap();
        assert_eq!((pcm.rate, pcm.channels), (44100, 2));
        if name == "song.wav" {
            backing = pcm.stereo.len();
        }
        expected.resize(expected.len().max(pcm.stereo.len()), 0.);
        for (out, value) in expected.iter_mut().zip(pcm.stereo) {
            *out += f64::from(value);
        }
        names.insert(name.to_owned());
    }
    assert!(
        backing > 0 && expected.len() > backing,
        "Fixture must exercise shorter backing audio"
    );
    let exported: BTreeSet<_> = zip
        .file_names()
        .map(|name| name.unwrap().into_owned())
        .filter(|name| !name.contains('/') && name.ends_with(".wav"))
        .collect();
    assert_eq!(exported, names, "Game export added or omitted a channel");
    let receipt: serde_json::Value = serde_json::from_slice(&member(
        &mut zip,
        ".yace/audio/chart-engine-conventional-audio.json",
    ))
    .unwrap();
    let raw = member(&mut zip, ".yace/audio/conventional/complete.wav");
    let complete = ready(audio_decode::decode(&raw)).unwrap();
    let gain = receipt["manifest"]["conventional"]["headroom"]
        .as_f64()
        .unwrap();
    assert!(gain > 0. && gain <= 1.);
    assert_eq!((complete.rate, complete.channels), (44100, 2));
    assert_eq!(complete.stereo.len(), expected.len());
    assert_eq!(
        receipt["manifest"]["frames"].as_u64(),
        Some((expected.len() / 2) as u64)
    );
    assert_eq!(
        receipt["manifest"]["source_audio_sha256"].as_str(),
        Some(format!("{:x}", Sha256::digest(raw)).as_str())
    );
    let mut maximum = 0_f64;
    for (actual, expected) in complete.stereo.iter().zip(expected) {
        assert!(actual.is_finite() && actual.abs() <= 1.000001);
        maximum = maximum.max((f64::from(*actual) - expected * gain).abs());
    }
    assert!(maximum < 0.000001, "Complete mix differs by {maximum}");
    eprintln!(
        "{} raw game files unchanged; {} full-clock frames; maximum complete-mix sample delta {maximum}",
        names.len(),
        complete.stereo.len() / 2
    );
}

fn compare_native_resampling(path: &std::path::Path) {
    let pcm = ready(audio_decode::decode(&std::fs::read(path).unwrap())).unwrap();
    for rate in [11025, 16000, 22050, 44100, 48000] {
        for channels in [1, 2] {
            let native = std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-i"])
                .arg(path)
                .args([
                    "-f",
                    "f32le",
                    "-ac",
                    &channels.to_string(),
                    "-ar",
                    &rate.to_string(),
                    "-",
                ])
                .output()
                .unwrap();
            assert!(native.status.success());
            let native: Vec<_> = native
                .stdout
                .chunks_exact(4)
                .map(|s| f32::from_le_bytes(s.try_into().unwrap()))
                .collect();
            let ours = pcm.samples(rate, channels).unwrap();
            assert_eq!(ours.len(), native.len(), "{rate}Hz/{channels} source clock");
            let max = ours
                .iter()
                .zip(&native)
                .map(|(a, b)| (a - b).abs())
                .fold(0f32, f32::max);
            let rms = (ours
                .iter()
                .zip(&native)
                .map(|(a, b)| (*a as f64 - *b as f64).powi(2))
                .sum::<f64>()
                / ours.len() as f64)
                .sqrt();
            eprintln!("{rate}Hz/{channels}: max {max}; RMS {rms}");
            assert!(
                max < 0.00001 && rms < 0.000001,
                "Native/browser resampling differs"
            );
        }
    }
}
#[test]
fn analysis_resampling_matches_native_filter_edges_and_fractional_source_clock() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("signal.wav");
    for source_rate in [44100, 48000, 96000] {
        let pcm = Pcm {
            rate: source_rate,
            channels: 2,
            stereo: (0..1003)
                .flat_map(|frame| {
                    [
                        if frame == 0 {
                            1.5
                        } else {
                            (frame as f32 * 0.33).sin() * 0.7
                        },
                        (frame as f32 * 1.37).cos() * 0.3,
                    ]
                })
                .collect(),
        };
        std::fs::write(&path, pcm.float_wave().unwrap()).unwrap();
        compare_native_resampling(&path);
    }
}
#[test]
#[ignore = "Requires explicit real YACE_RESAMPLE_INPUT"]
fn actual_recording_resampling_matches_native_model_inputs() {
    compare_native_resampling(std::path::Path::new(
        &std::env::var_os("YACE_RESAMPLE_INPUT").expect("actual audio"),
    ));
}

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
fn compact_backup_restore_retains_source_history_and_shared_assets() {
    use std::{io::Cursor, sync::Arc};
    let audio = vec![31; 65536];
    let files = [
        ("source/song.ogg", audio.as_slice()),
        ("dance/source/song.ogg", audio.as_slice()),
        ("project.json", b"document".as_slice()),
        ("checkpoints/id.json", b"history".as_slice()),
    ];
    let zip =
        super::compact_backup::encode_into(files, Cursor::new(Vec::new()), Default::default())
            .unwrap()
            .into_inner();
    let restored: std::collections::BTreeMap<_, _> = super::archive_codec::decode_shared(&zip)
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(restored["source/song.ogg"].as_slice(), audio);
    assert_eq!(restored["checkpoints/id.json"].as_slice(), b"history");
    assert!(Arc::ptr_eq(
        &restored["source/song.ogg"],
        &restored["dance/source/song.ogg"]
    ));
}

#[test]
fn incremental_import_seeks_archives_larger_than_one_gibibyte() {
    use std::io::{Seek, SeekFrom, Write};
    let mut file = tempfile::tempfile().unwrap();
    file.seek(SeekFrom::Start(1024 * 1024 * 1024 + 1)).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    zip.start_file("project.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"project").unwrap();
    let mut file = zip.finish().unwrap();
    assert!(file.metadata().unwrap().len() > 1024 * 1024 * 1024);
    file.rewind().unwrap();
    let files = super::archive_codec::strip_wrapper_shared(
        super::archive_codec::decode_reader(file).unwrap(),
        "project.json",
    )
    .unwrap();
    assert_eq!(files[0].1.as_slice(), b"project");
}

#[test]
fn incremental_import_limits_expansion_before_allocating_an_entry() {
    use std::io::Cursor;
    let bytes = super::archive_codec::encode([("big.wav", [7; 1024].as_slice())]).unwrap();
    let mut delivered = false;
    let result =
        super::archive_codec::decode_into_limited(Cursor::new(bytes), 512, 2048, |_, _| {
            delivered = true;
            Ok(())
        });
    assert!(result.is_err());
    assert!(!delivered);
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
    let instruments =
        std::env::var("YACE_TRANSCRIPTION_PARTS").unwrap_or_else(|_| "drums,guitar,bass".into());
    // Explicit diagnostic mode preserves a recorded CPU/WASM threshold case;
    // strict comparisons remain the default and no inference threshold changes.
    let review_boundaries = std::env::var("YACE_ALLOW_ONSET_BOUNDARY_REVIEW").as_deref() == Ok("1");
    for instrument in instruments.split(',') {
        assert!(["drums", "guitar", "bass", "piano", "pro_piano"].contains(&instrument));
        let ours = browser[instrument].as_array().unwrap();
        let theirs = native[instrument].as_array().unwrap();
        assert_eq!(ours.len(), theirs.len(), "{instrument} detection count");
        let mut max_time = 0f64;
        let mut max_score = 0f64;
        let mut boundary_count = 0usize;
        for (index, (a, b)) in ours.iter().zip(theirs).enumerate() {
            let near_gate = |value: &Value| {
                value
                    .as_f64()
                    .is_some_and(|score| (score - 0.5).abs() <= 64. * f64::from(f32::EPSILON))
            };
            let boundary = review_boundaries
                && instrument == "guitar"
                && a["start"] != b["start"]
                && a["end"] == b["end"]
                && (near_gate(&browser["guitar_candidates"][index]["onset_score"])
                    || near_gate(&native["guitar_candidates"][index]["onset_score"]));
            if boundary {
                boundary_count += 1;
                eprintln!(
                    "Review required: {instrument} onset threshold boundary; browser {a}, native {b}"
                );
            }
            for field in if instrument == "drums" {
                vec!["time"]
            } else {
                vec!["start", "end"]
            } {
                let delta = (a[field].as_f64().unwrap() - b[field].as_f64().unwrap()).abs();
                max_time = max_time.max(delta);
                let tolerance = if matches!(instrument, "piano" | "pro_piano") {
                    // Kong refines event times with float regression outputs.
                    // Different WASM/native kernels retain sub-sample agreement.
                    1e-6
                } else {
                    1e-9
                };
                assert!(
                    delta < tolerance || (boundary && field == "start"),
                    "{instrument} {field} differs: {a} / {b}"
                );
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
            assert!(delta < 0.002 || boundary, "{instrument} score differs");
        }
        eprintln!(
            "{instrument}: {} notes, maximum time delta {max_time}, maximum score delta {max_score}, explicitly reviewed onset boundaries {boundary_count}",
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
#[ignore = "Requires named actual browser/native recognition and vocal-generation downloads"]
fn actual_browser_recognition_preserves_native_words_times_and_vocal_chart() {
    use serde_json::Value;
    let read = |key| -> Value {
        serde_json::from_slice(
            &std::fs::read(std::env::var_os(key).expect("Explicit real model output required"))
                .unwrap(),
        )
        .unwrap()
    };
    let browser = read("YACE_BROWSER_RECOGNITION");
    let native = read("YACE_NATIVE_RECOGNITION");
    for field in ["language", "model_sha256", "recognition_audio_sha256"] {
        assert_eq!(browser[field], native[field], "{field}");
    }
    for field in ["audio_sha256", "duration_seconds", "words"] {
        assert_eq!(browser["words"][field], native["words"][field], "{field}");
    }
    let ours = browser["segments"].as_array().unwrap();
    let theirs = native["segments"].as_array().unwrap();
    assert_eq!(ours.len(), theirs.len());
    let mut maximum = 0f64;
    for (a, b) in ours.iter().zip(theirs) {
        for field in ["text", "start_seconds", "end_seconds"] {
            assert_eq!(a[field], b[field], "{field}");
        }
        for field in ["mean_token_probability", "no_speech_probability"] {
            let a = a[field].as_f64().unwrap();
            let b = b[field].as_f64().unwrap();
            assert!((0. ..=1.).contains(&a) && (0. ..=1.).contains(&b));
            maximum = maximum.max((a - b).abs());
        }
    }
    // CPU/WASM and native Metal confidence scores are not identical. These
    // compare source-bound decisions, not a claim of identical arithmetic.
    eprintln!(
        "{} segments, identical words/times, maximum probability delta {maximum}",
        ours.len()
    );
    let browser = read("YACE_BROWSER_VOCAL_GENERATION");
    let native = read("YACE_NATIVE_VOCAL_GENERATION");
    for field in ["chart", "issues", "status"] {
        assert_eq!(browser[field], native[field], "{field}");
    }
}

#[test]
#[ignore = "Requires YACE_BROWSER_VOCAL_GENERATION and a real native run using the same word evidence"]
fn actual_browser_vocal_chart_matches_native_supplied_word_evidence() {
    use serde_json::Value;
    let read = |key| -> Value {
        serde_json::from_slice(
            &std::fs::read(std::env::var_os(key).expect("Explicit real model output required"))
                .unwrap(),
        )
        .unwrap()
    };
    let browser = read("YACE_BROWSER_VOCAL_GENERATION");
    let native = read("YACE_NATIVE_VOCAL_GENERATION");
    for field in [
        "audio_sha256",
        "vocal_stem_sha256",
        "duration_seconds",
        "algorithm",
        "policy",
        "timing",
        "words",
        "chart",
        "issues",
        "status",
    ] {
        assert_eq!(browser[field], native[field], "{field}");
    }
    eprintln!(
        "{} notes, {} phrases, {} review issues; same words and source clock",
        browser["chart"]["notes"].as_array().unwrap().len(),
        browser["chart"]["phrases"].as_array().unwrap().len(),
        browser["issues"].as_array().unwrap().len()
    );
}

#[test]
#[ignore = "Requires actual downloaded YACE_BROWSER_ALIGNMENT and real YACE_NATIVE_ALIGNMENT from identical supplied lyrics and stems"]
fn actual_browser_acoustic_alignment_preserves_native_word_boundaries() {
    use serde_json::Value;
    let read = |key| -> Value {
        serde_json::from_slice(
            &std::fs::read(std::env::var_os(key).expect("Explicit real alignment output required"))
                .unwrap(),
        )
        .unwrap()
    };
    let mut browser = read("YACE_BROWSER_ALIGNMENT");
    let mut native = read("YACE_NATIVE_ALIGNMENT");
    let left = browser["diagnostics"].as_array_mut().unwrap();
    let right = native["diagnostics"].as_array_mut().unwrap();
    assert_eq!(left.len(), right.len());
    let mut maximum = 0_f64;
    for (a, b) in left.iter_mut().zip(right.iter_mut()) {
        match (
            a["mean_log_probability"].as_f64(),
            b["mean_log_probability"].as_f64(),
        ) {
            (Some(a), Some(b)) => {
                maximum = maximum.max((a - b).abs());
                assert!(
                    (a - b).abs() < 0.001,
                    "Acoustic probability diverged: {a}/{b}"
                );
            }
            (None, None) => {}
            _ => panic!("Acoustic diagnostics lost probability"),
        }
        a.as_object_mut().unwrap().remove("mean_log_probability");
        b.as_object_mut().unwrap().remove("mean_log_probability");
    }
    assert_eq!(
        browser, native,
        "Word boundaries, uncertainty, source binding and selection must match"
    );
    eprintln!(
        "{} aligned words; maximum acoustic log probability delta {maximum}",
        browser["words"]["words"].as_array().unwrap().len()
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
            // Native SWR normalizes the phase bank together and has a small
            // DC ripple. Also compare the complete result with native below.
            .all(|s| (s[0] - 0.25).abs() < 1e-5 && (s[1] + 0.5).abs() < 1e-5)
    );
    assert!(
        pcm.samples(22050, 1)
            .unwrap()
            .iter()
            .all(|s| (*s + 0.125 * std::f32::consts::SQRT_2).abs() < 1e-5)
    );
    assert_eq!(pcm.samples(48000, 2).unwrap(), pcm.stereo);
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("constant.wav");
    std::fs::write(&path, pcm.float_wave().unwrap()).unwrap();
    compare_native_resampling(&path);
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
#[test]
fn optimizer_version_accepts_official_release_and_packaged_132_only() {
    for version in [
        "wasm-opt version 132\n",
        "wasm-opt version 132 (version_132)\n",
    ] {
        assert!(super::compatible_optimizer_version(version), "{version}");
    }
    for version in [
        "wasm-opt version 131",
        "wasm-opt version 133",
        "wasm-opt version 1320",
        "wasm-opt version 132.1",
        "other version 132",
        "",
    ] {
        assert!(!super::compatible_optimizer_version(version), "{version}");
    }
}
