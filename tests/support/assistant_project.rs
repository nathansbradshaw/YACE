use chart_engine::{
    chart::Chart,
    editor::{Content, EditorNote, Project, Store, Transport},
    review_regions::{Issue, Policy, Reason, ReviewMap},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

const SOURCE: &str = "[Song]\n{\n Resolution = 480\n}\n[SyncTrack]\n{\n 0 = B 120000\n}\n[Events]\n{\n}\n[ExpertSingle]\n{\n 480 = N 0 0\n 500 = N 0 0\n 2400 = N 1 0\n}\n[ExpertDrums]\n{\n 480 = N 1 0\n}\n";

pub(crate) fn fixture(root: &Path) -> Store {
    fs::create_dir(root.join("source")).unwrap();
    fs::create_dir(root.join("checkpoints")).unwrap();
    fs::write(root.join("source/notes.chart"), SOURCE).unwrap();
    let audio = b"opaque recording fixture; visibility uses no decoder";
    fs::write(root.join("source/song.ogg"), audio).unwrap();
    let source_hash = format!("{:x}", Sha256::digest(SOURCE.as_bytes()));
    let audio_hash = format!("{:x}", Sha256::digest(audio));
    let chart = Chart::parse(SOURCE.into()).unwrap();
    let tracks = ["ExpertSingle", "ExpertDrums"]
        .into_iter()
        .map(|track| {
            (
                track.into(),
                chart
                    .notes(track)
                    .unwrap()
                    .into_iter()
                    .map(|note| EditorNote {
                        id: format!("{track}-{}", note.tick),
                        note,
                    })
                    .collect(),
            )
        })
        .collect();
    let project = Project {
        version: 1,
        revision: 0,
        title: "Visibility fixture".into(),
        original_folder: root.join("original"),
        source_chart_sha256: source_hash.clone(),
        audio_filename: "song.ogg".into(),
        audio_sha256: audio_hash.clone(),
        duration: 4.0,
        review: ReviewMap::build(
            4.0,
            Policy {
                window_seconds: 2.0,
                ..Policy::default()
            },
            BTreeMap::from([
                ("chart".into(), source_hash),
                ("original_audio".into(), audio_hash),
            ]),
            Some(BTreeSet::from([
                "ExpertSingle".into(),
                "ExpertDrums".into(),
            ])),
            BTreeSet::from([Reason::FastRepeat]),
            vec![
                Issue::at(0.5, "ExpertSingle", Reason::FastRepeat, &[]),
                Issue::at(0.5, "ExpertDrums", Reason::FastRepeat, &[]),
                Issue::at(2.5, "ExpertSingle", Reason::FastRepeat, &[]),
            ],
        )
        .unwrap(),
        content: Content {
            star_power: Default::default(),
            sync: None,
            sections: None,
            track_origins: Default::default(),
            difficulty_links: Default::default(),
            vocals: None,
            vocal_review: None,
            tracks,
            decisions: vec![],
            hidden_flags: BTreeSet::new(),
            stems: None,
            keyboard_stems: None,
            stem_mix: Default::default(),
        },
        selected_track: "ExpertSingle".into(),
        selected_region: None,
        selected_note: None,
        kit_audition: None,
        transport: Transport {
            position: 0.0,
            playing: false,
            anchor_ms: 0,
            loop_range: None,
            playback_rate: 1.0,
        },
        next_id: 1,
        undo: vec![],
        redo: vec![],
        audit: vec![],
    };
    save(&Store::new(root), &project);
    Store::new(root)
}

pub(crate) fn save(store: &Store, project: &Project) {
    fs::write(
        store.root.join("project.json"),
        serde_json::to_vec_pretty(project).unwrap(),
    )
    .unwrap();
}
