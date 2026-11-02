use super::*;
use crate::{
    editor::{Action, Actor, ActorKind, Request, Store},
    stem_review::{MixMode, StemChannel, StemMix},
};

const CHART: &str = "[Song]\n{\nResolution = 480\n}\n[SyncTrack]\n{\n0 = B 120000\n}\n[ExpertSingle]\n{\n240 = N 0 120\n}\n";
fn song(root: &Path) -> PathBuf {
    let song = root.join("song");
    fs::create_dir(&song).unwrap();
    fs::write(song.join("notes.chart"), CHART).unwrap();
    fs::write(
        song.join("song.ini"),
        "[song]\nname = Supplied channels\ncustom = keep\n",
    )
    .unwrap();
    song
}
fn channel(song: &Path, name: &str, frames: usize, left: f32, right: f32) {
    models::write_audio(&song.join(name), &vec![left; frames], &vec![right; frames]).unwrap();
}
fn mono(song: &Path, name: &str, rate: u32, frames: u32, value: i16) {
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((36 + frames * 2).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(rate.to_le_bytes());
    bytes.extend((rate * 2).to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend((frames * 2).to_le_bytes());
    for _ in 0..frames {
        bytes.extend(value.to_le_bytes());
    }
    fs::write(song.join(name), bytes).unwrap();
}
fn apply(store: &Store, action: Action) {
    store
        .apply(Request {
            expected_revision: store.load().unwrap().revision,
            actor: Actor {
                kind: ActorKind::Human,
                name: "Supplied-channel fixture".into(),
            },
            action,
        })
        .unwrap();
}
fn close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 2e-6, "{actual} != {expected}");
}

#[test]
fn complete_mix_groups_split_channels_pads_tails_and_preserves_stereo_balance() {
    let root = tempfile::tempdir().unwrap();
    let input = song(root.path());
    channel(&input, "song.wav", 88200, 0.4, 0.1);
    channel(&input, "guitar.wav", 88200, 0.3, 0.2);
    channel(&input, "drums_1.wav", 44100, 0.2, 0.3);
    channel(&input, "drums_2.wav", 88200, 0.4, 0.1);
    mono(&input, "vocals_1.wav", 22050, 22050, 8192);
    channel(&input, "vocals_2.wav", 88200, 0.1, 0.15);
    let store = Store::create(&input, None, &root.path().join("project")).unwrap();
    let doc = store.load().unwrap();
    assert_eq!(doc.audio_filename, PREVIEW);
    let manifest = doc.content.stems.as_ref().unwrap();
    assert_eq!(manifest.frames, 88200);
    assert_eq!(
        manifest.ids(),
        [
            StemId::Backing,
            StemId::Guitar,
            StemId::Drums,
            StemId::Vocals
        ]
        .into_iter()
        .collect()
    );
    let gain = manifest.conventional.as_ref().unwrap().headroom.value();
    // The shared decoder uses FFmpeg's -3 dB mono speaker matrix.
    let mono_level = 0.25 * std::f32::consts::FRAC_1_SQRT_2;
    close(gain, 1. / (1.4 + mono_level));
    let loaded = manifest.load(&store.audio_path(&doc).unwrap()).unwrap();
    let mixed = loaded
        .render(&StemMix {
            mode: MixMode::Stems,
            ..Default::default()
        })
        .unwrap();
    for (actual, expected) in mixed.iter().zip(loaded.original.iter()) {
        close(*actual, *expected);
    }
    close(mixed[2000], (1.4 + mono_level) * gain);
    close(mixed[2001], (0.85 + mono_level) * gain);
    close(mixed[140000], 1.2 * gain);
    close(mixed[140001], 0.55 * gain);
    assert!(
        manifest
            .stems
            .iter()
            .find(|a| a.id == StemId::Guitar)
            .unwrap()
            .path
            .ends_with("guitar.wav")
    );
    assert!(
        manifest
            .stems
            .iter()
            .find(|a| a.id == StemId::Drums)
            .unwrap()
            .path
            .to_string_lossy()
            .contains("conventional/")
    );
    assert!(!fs::exists(input.join(".yace")));
}

#[test]
fn stem_only_song_supports_gain_mute_solo_undo_and_redo() {
    let root = tempfile::tempdir().unwrap();
    let input = song(root.path());
    channel(&input, "bass.wav", 44100, 0.2, 0.1);
    channel(&input, "rhythm.wav", 44100, 0.3, 0.4);
    channel(&input, "keys.wav", 22050, 0.1, 0.05);
    channel(&input, "crowd.wav", 44100, 0.1, 0.1);
    let store = Store::create(&input, None, &root.path().join("project")).unwrap();
    let doc = store.load().unwrap();
    let manifest = doc.content.stems.as_ref().unwrap();
    assert_eq!(
        manifest.ids(),
        [StemId::Bass, StemId::Rhythm, StemId::Piano, StemId::Crowd]
            .into_iter()
            .collect()
    );
    let loaded = manifest.load(&store.audio_path(&doc).unwrap()).unwrap();
    let mut mix = StemMix {
        mode: MixMode::Stems,
        ..Default::default()
    };
    mix.channels.insert(
        StemId::Bass,
        StemChannel {
            gain: 0.5,
            mute: false,
            solo: true,
        },
    );
    apply(&store, Action::SetStemMix { mix: mix.clone() });
    let result = loaded
        .mix_frame(100, &store.load().unwrap().content.stem_mix)
        .unwrap();
    close(result[0], 0.1);
    close(result[1], 0.05);
    mix.channels.get_mut(&StemId::Bass).unwrap().mute = true;
    apply(&store, Action::SetStemMix { mix });
    assert_eq!(
        loaded
            .mix_frame(100, &store.load().unwrap().content.stem_mix)
            .unwrap(),
        [0., 0.]
    );
    apply(&store, Action::Undo);
    close(
        loaded
            .mix_frame(100, &store.load().unwrap().content.stem_mix)
            .unwrap()[0],
        0.1,
    );
    apply(&store, Action::Redo);
    assert_eq!(
        loaded
            .mix_frame(100, &store.load().unwrap().content.stem_mix)
            .unwrap(),
        [0., 0.]
    );
}

#[test]
fn export_reopen_and_portable_history_retain_raw_routing_without_an_extra_game_mix() {
    let root = tempfile::tempdir().unwrap();
    let input = song(root.path());
    for (name, value) in [
        ("song.wav", 0.1),
        ("guitar.wav", 0.2),
        ("drums_1.wav", 0.3),
        ("drums_2.wav", 0.1),
        ("vocals_1.wav", 0.1),
    ] {
        channel(&input, name, 44100, value, value / 2.);
    }
    let originals = inputs(&input).unwrap();
    let store = Store::create(&input, None, &root.path().join("project")).unwrap();
    apply(
        &store,
        Action::SetStemMix {
            mix: StemMix {
                mode: MixMode::Stems,
                channels: BTreeMap::from([(
                    StemId::Guitar,
                    StemChannel {
                        gain: 0.25,
                        mute: false,
                        solo: true,
                    },
                )]),
            },
        },
    );
    let export = root.path().join("export");
    apply(
        &store,
        Action::Export {
            output: export.clone(),
        },
    );
    assert_eq!(inputs(&export).unwrap(), originals);
    let receipt: serde_json::Value = serde_json::from_slice(
        &fs::read(song_layout::resolve(&export, "yace-audio.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["mode"], "conventional_channels");
    assert!(receipt["original_audio"].is_null());
    assert!(!fs::exists(export.join("drums.wav")));
    assert!(!fs::exists(export.join("vocals.wav")));
    let reopened = Store::create(&export, None, &root.path().join("reopened")).unwrap();
    let doc = reopened.load().unwrap();
    let manifest = doc.content.stems.as_ref().unwrap();
    manifest.load(&reopened.audio_path(&doc).unwrap()).unwrap();
    assert_eq!(
        manifest
            .stems
            .iter()
            .find(|asset| asset.id == StemId::Guitar)
            .unwrap()
            .path,
        reopened.root.join("source/guitar.wav")
    );
    assert_eq!(inputs(&reopened.root.join("source")).unwrap(), originals);
    fn collect(root: &Path, path: &Path, files: &mut crate::project_portability::Files) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                collect(root, &path, files)
            } else {
                files.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    std::fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = crate::project_portability::Files::new();
    collect(&store.root, &store.root, &mut files);
    let moved = root.path().join("moved");
    crate::project_portability::relocate(&mut files, &moved).unwrap();
    for (name, bytes) in files {
        let path = moved.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let moved = Store::new(&moved);
    apply(&moved, Action::Undo);
    apply(&moved, Action::Redo);
    let doc = moved.load().unwrap();
    close(doc.content.stem_mix.channels[&StemId::Guitar].gain, 0.25);
    doc.content
        .stems
        .as_ref()
        .unwrap()
        .load(&moved.audio_path(&doc).unwrap())
        .unwrap();
}

#[test]
fn changed_or_added_source_channels_fail_without_publishing_an_export() {
    let root = tempfile::tempdir().unwrap();
    let input = song(root.path());
    channel(&input, "song.wav", 44100, 0.1, 0.1);
    channel(&input, "bass.wav", 44100, 0.2, 0.2);
    let store = Store::create(&input, None, &root.path().join("project")).unwrap();
    let source = store.root.join("source");
    channel(&source, "crowd.wav", 44100, 0.1, 0.1);
    assert!(
        validate(&source)
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    let output = root.path().join("failed");
    assert!(
        store
            .apply(Request {
                expected_revision: store.load().unwrap().revision,
                actor: Actor {
                    kind: ActorKind::Human,
                    name: "fixture".into()
                },
                action: Action::Export {
                    output: output.clone()
                }
            })
            .is_err()
    );
    assert!(!fs::exists(output));
    fs::remove_file(source.join("crowd.wav")).unwrap();
    fs::write(source.join("bass.wav"), b"damaged audio").unwrap();
    assert!(validate(&source).is_err());
    assert_eq!(inputs(&input).unwrap().len(), 2);
}

#[test]
fn existing_recordings_and_explicit_separation_keep_their_original_semantics() {
    let root = tempfile::tempdir().unwrap();
    let input = song(root.path());
    channel(&input, "song.wav", 44100, 0.1, 0.1);
    assert!(!prepare(&input).unwrap());
    assert!(audio_identity(&input).unwrap().is_none());
    channel(&input, "guitar.wav", 44100, 0.1, 0.1);
    assert!(audio_identity(&input).unwrap().is_some());
    song_layout::prepare(&input).unwrap();
    fs::write(
        song_layout::path(&input, "chart-engine-isolation.json"),
        b"{}",
    )
    .unwrap();
    assert!(!should_prepare(&input).unwrap());
    assert!(decode_inputs(&input).unwrap().is_empty());
    assert!(!prepare(&input).unwrap());
    assert!(!fs::exists(input.join(PREVIEW)));
}

#[test]
fn duplicate_encodings_are_not_summed_and_channel_identity_tracks_all_inputs() {
    let root = tempfile::tempdir().unwrap();
    let input = song(root.path());
    channel(&input, "song.wav", 44100, 0.1, 0.1);
    channel(&input, "guitar.wav", 44100, 0.2, 0.2);
    assert!(
        std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(input.join("guitar.wav"))
            .arg(input.join("guitar.flac"))
            .status()
            .unwrap()
            .success()
    );
    let identity = audio_identity(&input).unwrap();
    assert!(prepare(&input).unwrap());
    assert_eq!(
        inputs(&input)
            .unwrap()
            .iter()
            .find(|i| i.id == StemId::Guitar)
            .unwrap()
            .filename,
        "guitar.flac"
    );
    assert_eq!(warnings(&input).unwrap().len(), 1);
    let manifest = manifest(&input).unwrap().unwrap();
    let loaded = manifest.load(&input.join(PREVIEW)).unwrap();
    close(loaded.original[2000], 0.3);
    let mut legacy = manifest.clone();
    legacy.conventional = None;
    assert!(
        legacy
            .validate_binding(&legacy.source_audio_sha256, 1.)
            .is_err()
    );
    fs::remove_file(input.join("guitar.flac")).unwrap();
    assert_ne!(audio_identity(&input).unwrap(), identity);
    assert!(validate(&input).is_err());
    assert!(!valid_audio_filename("../song.wav"));
    assert!(!valid_audio_filename(".yace/other.wav"));
}
