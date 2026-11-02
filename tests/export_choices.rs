use chart_engine::{
    chart::Chart,
    editor::{Actor, ActorKind, Store},
    export_options::{self, Format, Options, Target},
    game_export, models, song_layout,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const CHART: &str = "[Song]\r\n{\r\n Resolution = 480\r\n Offset = 0.125\r\n}\r\n[SyncTrack]\r\n{\r\n 0 = B 120000\r\n}\r\n[Events]\r\n{\r\n 960 = E \"section Chorus\"\r\n}\r\n[ExpertSingle]\r\n{\r\n 480 = N 0 120\r\n}\r\n[HardSingle]\r\n{\r\n 960 = N 1 240\r\n}\r\n[ExpertDrums]\r\n{\r\n 480 = N 1 0\r\n}\r\n";
fn actor() -> Actor {
    Actor {
        kind: ActorKind::Human,
        name: "Export fixture".into(),
    }
}
fn fixture(root: &Path, audio: bool, midi: bool) -> Store {
    let source = root.join("song");
    fs::create_dir(&source).unwrap();
    if midi {
        let chart = Chart::parse(CHART.into()).unwrap();
        let (bytes, delay) = game_export::song_with_ini(&chart, None, &BTreeMap::new()).unwrap();
        let mut smf = midly::Smf::parse(&bytes).unwrap();
        use midly::{MetaMessage as M, MidiMessage as N, TrackEvent, TrackEventKind as K};
        smf.tracks.push(vec![
            TrackEvent {
                delta: 0.into(),
                kind: K::Meta(M::TrackName(b"PART REAL_GUITAR")),
            },
            TrackEvent {
                delta: 480.into(),
                kind: K::Midi {
                    channel: 0.into(),
                    message: N::NoteOn {
                        key: 24.into(),
                        vel: 100.into(),
                    },
                },
            },
            TrackEvent {
                delta: 120.into(),
                kind: K::Midi {
                    channel: 0.into(),
                    message: N::NoteOff {
                        key: 24.into(),
                        vel: 0.into(),
                    },
                },
            },
            TrackEvent {
                delta: 0.into(),
                kind: K::Meta(M::EndOfTrack),
            },
        ]);
        let mut output = Vec::new();
        smf.write_std(&mut output).unwrap();
        fs::write(source.join("notes.mid"), output).unwrap();
        fs::write(source.join("song.ini"), format!("[song]\nname = Export fixture\ndelay = {delay}\ncustom = preserved\ndiff_drums = 3\n")).unwrap();
    } else {
        fs::write(source.join("notes.chart"), CHART).unwrap();
        fs::write(
            source.join("song.ini"),
            "[song]\nname = Export fixture\ncustom = preserved\ndiff_drums = 3\n",
        )
        .unwrap();
    }
    fs::write(source.join("album.png"), b"synthetic artwork").unwrap();
    if audio {
        models::write_audio(
            &source.join("song.wav"),
            &vec![0.; 176400],
            &vec![0.; 176400],
        )
        .unwrap();
    }
    Store::create(&source, None, &root.join("project")).unwrap()
}
fn selection(parts: &[&str]) -> Options {
    Options {
        tracks: Some(parts.iter().map(|p| (*p).into()).collect()),
        ..Default::default()
    }
}
fn files(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(files(&path));
        } else {
            out.push(path);
        }
    }
    out
}
#[test]
fn selected_chart_and_audio_choices_leave_project_and_history_complete() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path(), true, false);
    let before: BTreeMap<_, _> = files(&store.root)
        .into_iter()
        .map(|p| {
            let b = fs::read(&p).unwrap();
            (p, b)
        })
        .collect();
    let mut options = selection(&["HardSingle"]);
    options.include_audio = false;
    options.format = Format::Chart;
    let output = root.path().join("export");
    store
        .export_with_options(&store.load().unwrap(), &output, &actor(), &options)
        .unwrap();
    let text = fs::read_to_string(output.join("notes.chart")).unwrap();
    let chart = Chart::parse(text.clone()).unwrap();
    assert_eq!(chart.notes("HardSingle").unwrap().len(), 1);
    assert!(!text.contains("ExpertSingle"));
    assert!(!text.contains("ExpertDrums"));
    assert!(text.contains("960 = E \"section Chorus\"\r\n"));
    assert_eq!(chart.timing.offset, 0.125);
    assert!(files(&output).iter().all(|p| !export_options::is_audio(p)));
    let ini = fs::read_to_string(output.join("song.ini")).unwrap();
    assert!(ini.contains("custom = preserved"));
    assert!(ini.contains("diff_drums = -1"));
    assert!(output.join("album.png").is_file());
    for (path, bytes) in before {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    assert_eq!(store.load().unwrap().content.tracks.len(), 3);
    #[cfg(any(feature = "editor", feature = "browser"))]
    {
        let backup = root.path().join("backup.zip");
        chart_engine::project_files::backup(&store, &backup).unwrap();
        let restored =
            chart_engine::project_files::import(&backup, &root.path().join("restored")).unwrap();
        assert_eq!(restored.load().unwrap().content.tracks.len(), 3);
        assert!(
            restored
                .audio_path(&restored.load().unwrap())
                .unwrap()
                .is_file()
        );
    }
}
#[test]
fn forced_midi_filters_difficulties_and_preserves_recording_delay() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path(), false, false);
    let mut options = selection(&["HardSingle"]);
    options.format = Format::Midi;
    let output = root.path().join("export");
    store
        .export_with_options(&store.load().unwrap(), &output, &actor(), &options)
        .unwrap();
    let bytes = fs::read(output.join("notes.mid")).unwrap();
    let smf = midly::Smf::parse(&bytes).unwrap();
    let pitches: BTreeSet<_> = smf
        .tracks
        .iter()
        .flatten()
        .filter_map(|e| match e.kind {
            midly::TrackEventKind::Midi {
                message: midly::MidiMessage::NoteOn { key, vel },
                ..
            } if vel.as_int() > 0 => Some(key.as_int()),
            _ => None,
        })
        .collect();
    assert!(pitches.contains(&85));
    assert!(pitches.iter().all(|key| (84..=90).contains(key)));
    assert!(
        fs::read_to_string(output.join("song.ini"))
            .unwrap()
            .contains("delay = 125")
    );
    assert!(!output.join("notes.chart").exists());
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert_eq!(
        export_options::available(&reopened, &reopened.load().unwrap()).unwrap(),
        BTreeSet::from(["HardSingle".into()])
    );
}
#[test]
fn imported_unmapped_midi_parts_are_optional_and_do_not_return_after_reopening() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path(), false, true);
    let doc = store.load().unwrap();
    let available = export_options::available(&store, &doc).unwrap();
    assert!(available.contains("MIDI: PART REAL_GUITAR"));
    let full = root.path().join("full");
    store
        .export_with_options(&doc, &full, &actor(), &Options::default())
        .unwrap();
    assert_eq!(
        fs::read(full.join("notes.mid")).unwrap(),
        fs::read(root.path().join("song/notes.mid")).unwrap()
    );
    let output = root.path().join("export");
    store
        .export_with_options(&doc, &output, &actor(), &selection(&["ExpertSingle"]))
        .unwrap();
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    let available = export_options::available(&reopened, &reopened.load().unwrap()).unwrap();
    assert!(!available.contains("MIDI: PART REAL_GUITAR"));
    let again = root.path().join("again");
    reopened
        .export_with_options(
            &reopened.load().unwrap(),
            &again,
            &actor(),
            &Options::default(),
        )
        .unwrap();
    let bytes = fs::read(again.join("notes.mid")).unwrap();
    let midi = midly::Smf::parse(&bytes).unwrap();
    assert!(!midi.tracks.iter().flatten().any(|e| matches!(
        e.kind,
        midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(b"PART REAL_GUITAR"))
    )));
    assert!(song_layout::path(&output, "game-export.json").is_file());
}
#[test]
fn invalid_choices_fail_before_publishing_an_export() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path(), false, false);
    let doc = store.load().unwrap();
    for options in [selection(&[]), selection(&["ExpertMissing"])] {
        let output = root.path().join("invalid");
        assert!(
            store
                .export_with_options(&doc, &output, &actor(), &options)
                .is_err()
        );
        assert!(!output.exists());
    }
    let midi_root = tempfile::tempdir().unwrap();
    let midi = fixture(midi_root.path(), false, true);
    let mut chart = Options {
        format: Format::Chart,
        ..Default::default()
    };
    assert!(
        chart
            .selected(&midi, &midi.load().unwrap())
            .unwrap_err()
            .to_string()
            .contains("Imported MIDI")
    );
    chart.format = Format::Automatic;
    chart.target = Target::CloneHero;
    assert!(chart.selected(&midi, &midi.load().unwrap()).is_err());
    chart.tracks = Some(BTreeSet::from(["ExpertSingle".into()]));
    assert!(chart.selected(&midi, &midi.load().unwrap()).is_ok());
}

#[test]
fn omitting_conventional_audio_still_produces_a_reopenable_chart_only_song() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path(), true, false);
    let song = root.path().join("song");
    models::write_audio(&song.join("bass.wav"), &vec![0.; 176400], &vec![0.; 176400]).unwrap();
    let store = Store::create(&song, None, &root.path().join("multichannel")).unwrap();
    assert!(store.load().unwrap().content.stems.is_some());
    let output = root.path().join("export");
    store
        .export_with_options(
            &store.load().unwrap(),
            &output,
            &actor(),
            &Options {
                include_audio: false,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(files(&output).iter().all(|p| !export_options::is_audio(p)));
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert!(!reopened.load().unwrap().has_recording());
}

#[test]
fn deselected_vocals_and_pro_keys_do_not_return_when_a_chart_export_is_reopened() {
    use chart_engine::vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone};
    let root = tempfile::tempdir().unwrap();
    fixture(root.path(), false, false);
    let song = root.path().join("song");
    fs::write(
        song.join("notes.chart"),
        format!("{CHART}[ExpertProKeys]\n{{\n 480 = N 60 120\n}}\n"),
    )
    .unwrap();
    let vocal = VocalChart {
        version: 1,
        resolution: 480,
        source: "Synthetic export fixture".into(),
        phrases: vec![VocalPhrase {
            start: 480,
            end: 960,
            player: 1,
        }],
        notes: vec![VocalNote {
            start: 480,
            end: 600,
            tone: VocalTone::Pitched { midi: 60 },
            lyric: Syllable::Missing,
            midi: None,
        }],
    };
    fs::write(
        song.join("chart-engine-vocals.json"),
        serde_json::to_vec(&vocal).unwrap(),
    )
    .unwrap();
    let store = Store::create(&song, None, &root.path().join("vocal-project")).unwrap();
    let doc = store.load().unwrap();
    assert!(doc.content.vocals.is_some());
    let all = Options {
        format: Format::Chart,
        ..Default::default()
    };
    assert!(all.selected(&store, &doc).is_err());
    let mut options = selection(&["HardSingle"]);
    options.format = Format::Chart;
    let output = root.path().join("export");
    store
        .export_with_options(&doc, &output, &actor(), &options)
        .unwrap();
    let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
    assert!(reopened.load().unwrap().content.vocals.is_none());
    assert_eq!(
        export_options::available(&reopened, &reopened.load().unwrap()).unwrap(),
        BTreeSet::from(["HardSingle".into()])
    );
    assert!(store.load().unwrap().content.vocals.is_some());
}
