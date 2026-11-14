//! Read-only projection of the current editable part into audition attacks.
use super::*;
use crate::{
    editor::{Project, Store},
    vocal_harmonies::Part,
    vocals::VocalTone,
};

impl Plan {
    pub fn for_project(store: &Store, doc: &Project, settings: &Settings) -> Result<Self> {
        settings.validate()?;
        if !settings.enabled() {
            return Self::new(Vec::new(), doc.duration);
        }
        let timing = doc.timing(store)?;
        let mut notes = Vec::new();
        let selected = |class| settings.classes.contains(&class);
        if settings.claps {
            if doc.selected_track == "Vocals" {
                let voice = match settings.singer {
                    Singer::Lead => doc.content.vocals.as_ref().map(|c| &c.notes),
                    singer => doc
                        .content
                        .harmonies
                        .as_ref()
                        .and_then(|c| {
                            c.voices.get(&match singer {
                                Singer::Harm1 => Part::Harm1,
                                Singer::Harm2 => Part::Harm2,
                                Singer::Harm3 => Part::Harm3,
                                Singer::Lead => unreachable!(),
                            })
                        })
                        .map(|v| &v.notes),
                };
                for note in voice.into_iter().flatten() {
                    let class = match note.tone {
                        VocalTone::Pitched { .. } => NoteClass::PitchedVocal,
                        VocalTone::Talkie { .. } => NoteClass::Talkie,
                    };
                    if selected(class) {
                        notes.push(note.start);
                    }
                }
                if selected(NoteClass::VocalPercussion)
                    && let Some(chart) = store.vocal_percussion(doc)?
                {
                    let owner = if settings.singer == Singer::Lead {
                        crate::vocal_percussion::Part::Lead
                    } else {
                        crate::vocal_percussion::Part::Harm1
                    };
                    if let Some(track) = chart.tracks.get(&owner) {
                        notes.extend(track.hits.iter().map(|hit| hit.start));
                    }
                }
            } else if let Some(track) = doc.content.tracks.get(&doc.selected_track) {
                for note in track {
                    let lane = note.note.lane;
                    if !doc.playable(&doc.selected_track, lane) {
                        continue;
                    }
                    let class = if doc.selected_track.ends_with("Drums") {
                        if matches!(lane, 0 | 32) {
                            NoteClass::Kick
                        } else {
                            NoteClass::DrumPad
                        }
                    } else if lane == 7
                        && !crate::pro_guitar::is_track(&doc.selected_track)
                        && !crate::keys::is_pro(&doc.selected_track)
                    {
                        NoteClass::Open
                    } else {
                        NoteClass::Instrument
                    };
                    if selected(class) {
                        notes.push(note.note.tick);
                    }
                }
            }
        }
        notes.sort_unstable();
        notes.dedup();
        if settings.metronome {
            let imported = store.source_sync(doc)?;
            let meters = doc
                .content
                .sync
                .as_ref()
                .map_or(&imported.meters, |s| &s.meters);
            Self::from_ticks(&timing, meters, &notes, doc.duration)
        } else {
            let clock = Clock::new(&timing)?;
            Self::new(
                notes
                    .into_iter()
                    .map(|tick| Event {
                        seconds: clock.seconds(tick as f64),
                        kind: Kind::Clap,
                    })
                    .collect(),
                doc.duration,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Store, Project) {
        let d = tempfile::tempdir().unwrap();
        let source = d.path().join("song");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(
            source.join("song.ini"),
            "[song]\nname = Click projection\nsong_length = 10000\n",
        )
        .unwrap();
        std::fs::write(source.join("notes.chart"),"[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n[ExpertSingle]\n{\n 0 = N 0 0\n 0 = N 1 0\n 120 = N 5 0\n 240 = N 2 0\n 240 = N 6 0\n 480 = N 7 0\n}\n[HardSingle]\n{\n 960 = N 4 0\n}\n[ExpertDrums]\n{\n 0 = N 0 0\n 0 = N 1 0\n 240 = N 32 0\n 240 = N 66 0\n 480 = N 4 0\n}\n").unwrap();
        let store = Store::create(&source, None, &d.path().join("project")).unwrap();
        let doc = store.load().unwrap();
        (d, store, doc)
    }
    fn times(plan: Plan) -> Vec<f64> {
        assert!(plan.events().iter().all(|e| e.kind == Kind::Clap));
        plan.events().iter().map(|e| e.seconds).collect()
    }
    #[test]
    fn selected_part_filters_markers_and_groups_chord_attacks_without_writing() {
        let (_d, store, mut doc) = fixture();
        doc.selected_track = "ExpertSingle".into();
        let before = serde_json::to_vec(&doc).unwrap();
        let source = std::fs::read(store.root.join("source/notes.chart")).unwrap();
        let mut s = Settings {
            claps: true,
            classes: vec![NoteClass::Instrument],
            ..Settings::default()
        };
        assert_eq!(
            times(Plan::for_project(&store, &doc, &s).unwrap()),
            vec![0., 0.25]
        );
        s.classes = vec![NoteClass::Open];
        assert_eq!(
            times(Plan::for_project(&store, &doc, &s).unwrap()),
            vec![0.5]
        );
        s.classes = vec![NoteClass::Open, NoteClass::Instrument];
        assert_eq!(
            times(Plan::for_project(&store, &doc, &s).unwrap()),
            vec![0., 0.25, 0.5]
        );
        assert_eq!(serde_json::to_vec(&doc).unwrap(), before);
        assert_eq!(
            std::fs::read(store.root.join("source/notes.chart")).unwrap(),
            source
        );
        doc.selected_track = "ExpertDrums".into();
        s.classes = vec![NoteClass::Kick];
        assert_eq!(
            times(Plan::for_project(&store, &doc, &s).unwrap()),
            vec![0., 0.25]
        );
        s.classes = vec![NoteClass::DrumPad];
        assert_eq!(
            times(Plan::for_project(&store, &doc, &s).unwrap()),
            vec![0., 0.5]
        );
        s.classes.clear();
        assert!(
            Plan::for_project(&store, &doc, &s)
                .unwrap()
                .events()
                .is_empty()
        );
    }
    #[test]
    fn current_authored_sync_controls_audition_without_changing_source_offset() {
        let (_d, store, mut doc) = fixture();
        doc.selected_track = "ExpertSingle".into();
        let mut sync = store.source_sync(&doc).unwrap();
        sync.offset = Some(0.25);
        sync.tempos[0].bpm = 60.;
        doc.content.sync = Some(sync);
        let s = Settings {
            claps: true,
            classes: vec![NoteClass::Open],
            ..Settings::default()
        };
        assert_eq!(
            times(Plan::for_project(&store, &doc, &s).unwrap()),
            vec![1.25]
        );
        let source = crate::chart::Chart::parse(
            std::fs::read_to_string(store.root.join("source/notes.chart")).unwrap(),
        )
        .unwrap();
        assert_eq!(source.timing.offset, 0.);
        assert_eq!(doc.content.sync.as_ref().unwrap().offset, Some(0.25));
    }
}
