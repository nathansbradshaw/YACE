use chart_engine::{editor::Store, song_import};
use std::{fs, path::Path, process::Command};

fn run(args: &[&str]) {
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn pulse_file(path: &Path) {
    let rate = 11025_u32;
    let mut pcm = vec![0_i16; rate as usize * 48];
    for (begin, end, bpm) in [(0.0, 24.0, 120.0), (24.0, 48.0, 90.0)] {
        let mut beat = begin;
        while beat < end - 0.1 {
            let start = (beat * rate as f64) as usize;
            for j in 0..600.min(pcm.len() - start) {
                pcm[start + j] = (((j as f32 * 0.7).sin() + (j as f32 * 1.73).sin())
                    * (-(j as f32) / 90.0).exp()
                    * 14000.0) as i16;
            }
            beat += 60.0 / bpm;
        }
    }
    let length = pcm.len() as u32 * 2;
    let mut bytes = Vec::new();
    bytes.extend(b"RIFF");
    bytes.extend((36 + length).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16_u32.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(rate.to_le_bytes());
    bytes.extend((rate * 2).to_le_bytes());
    bytes.extend(2_u16.to_le_bytes());
    bytes.extend(16_u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(length.to_le_bytes());
    for sample in pcm {
        bytes.extend(sample.to_le_bytes());
    }
    fs::write(path, bytes).unwrap();
}

#[test]
fn encoded_mp3_tags_cover_and_tempo_survive_project_move_and_export() {
    let root = tempfile::tempdir().unwrap();
    let wave = root.path().join("pulse.wav");
    pulse_file(&wave);
    let cover = root.path().join("cover.png");
    run(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=blue:s=32x32",
        "-frames:v",
        "1",
        cover.to_str().unwrap(),
    ]);
    let mp3 = root.path().join("input.mp3");
    run(&[
        "-i",
        wave.to_str().unwrap(),
        "-i",
        cover.to_str().unwrap(),
        "-map",
        "0:a",
        "-map",
        "1:v",
        "-c:a",
        "libmp3lame",
        "-q:a",
        "2",
        "-c:v",
        "mjpeg",
        "-disposition:v",
        "attached_pic",
        "-metadata",
        "title=Pulse fixture",
        "-metadata",
        "artist=Test Artist",
        "-metadata",
        "album=Test Album",
        "-metadata",
        "date=2024",
        "-metadata",
        "genre=Test",
        mp3.to_str().unwrap(),
    ]);
    let imported = song_import::local(&mp3).unwrap();
    assert_eq!(imported.metadata.name, "Pulse fixture");
    assert_eq!(imported.metadata.artist, "Test Artist");
    assert_eq!(imported.info.album, "Test Album");
    assert_eq!(imported.info.year, "2024");
    assert!(imported.info.artwork.as_ref().unwrap().is_file());
    assert_eq!(
        imported.info.tempo.segments.len(),
        2,
        "{:?}",
        imported.info.tempo
    );
    for (section, bpm) in imported.info.tempo.segments.iter().zip([120.0, 90.0]) {
        assert!((section.bpm - bpm).abs() < 0.3, "{section:?}");
    }
    let song = root.path().join("Song");
    let project = song_import::create_project(&mp3, &imported, None, &song).unwrap();
    assert_eq!(project, song.canonicalize().unwrap().join(".yace/project"));
    for name in [
        "notes.chart",
        "song.ini",
        "song.wav",
        "album.png",
        ".yace/analysis/import-information.json",
        ".yace/originals/audio/recording.mp3",
    ] {
        assert!(song.join(name).is_file(), "{name}");
    }
    let store = Store::new(&project);
    let doc = store.load().unwrap();
    assert_eq!(doc.title, "Pulse fixture");
    assert_eq!(
        fs::read(project.join("source/.yace/originals/audio/recording.mp3")).unwrap(),
        fs::read(&mp3).unwrap(),
        "The portable editing project must retain the encoded original"
    );
    assert_eq!(doc.timing(&store).unwrap().tempos.len(), 2);
    assert!(song_import::create_project(&mp3, &imported, None, &song).is_err());
    fs::remove_file(&mp3).unwrap();
    fs::remove_file(imported.info.artwork.unwrap()).unwrap();
    let moved = root.path().join("Moved");
    fs::rename(&song, &moved).unwrap();
    let store = Store::new(&moved.join(".yace/project"));
    let doc = store.load().unwrap();
    assert!(store.audio_path(&doc).unwrap().is_file());
    let output = root.path().join("Export");
    store
        .apply(chart_engine::editor::Request {
            expected_revision: doc.revision,
            actor: chart_engine::editor::Actor {
                kind: chart_engine::editor::ActorKind::Human,
                name: "Import test".into(),
            },
            action: chart_engine::editor::Action::Export {
                output: output.clone(),
            },
        })
        .unwrap();
    let ini = fs::read_to_string(output.join("song.ini")).unwrap();
    assert!(ini.contains("album = Test Album"));
    assert!(output.join("album.png").is_file());
}

#[test]
fn untagged_filename_and_manual_bpm_work_and_invalid_metadata_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("Artist - Untagged.wav");
    pulse_file(&path);
    let mut imported = song_import::local(&path).unwrap();
    assert_eq!(imported.metadata.artist, "Artist");
    assert_eq!(imported.metadata.name, "Untagged");
    let song = root.path().join("Manual");
    let project = song_import::create_project(&path, &imported, Some(100.0), &song).unwrap();
    let store = Store::new(&project);
    let timing = store.load().unwrap().timing(&store).unwrap();
    assert_eq!(timing.tempos.len(), 1);
    assert_eq!(timing.tempos[0].bpm, 100.0);
    imported.metadata.name = "Bad\nartist = injected".into();
    assert!(
        song_import::create_project(&path, &imported, None, &root.path().join("Invalid")).is_err()
    );
    assert!(!root.path().join("Invalid").exists());
}

#[test]
#[ignore = "Explicit online catalogue/cover smoke check; shares only fixture title and artist"]
fn live_catalogue_returns_album_year_and_cover() {
    let root = tempfile::tempdir().unwrap();
    let mut metadata = chart_engine::pipeline::Metadata {
        name: "Billie Jean".into(),
        artist: "Michael Jackson".into(),
        duration: 294.0,
        ..Default::default()
    };
    let mut info = song_import::SongInfo::default();
    song_import::lookup(&mut metadata, &mut info, root.path(), true).unwrap();
    eprintln!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({"metadata":metadata,"artwork":info.artwork,"source":info.artwork_source,"notes":info.notes,"lyrics_found":info.lyrics.is_some()}))
            .unwrap()
    );
    assert!(!info.album.is_empty());
    assert!(!info.year.is_empty());
    assert!(info.artwork.as_ref().unwrap().is_file());
}
