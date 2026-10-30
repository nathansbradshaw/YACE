use chart_engine::{
    chart::Chart,
    editor::{Actor, ActorKind, Content, Decision, DecisionStatus, EditorNote, Project, Transport},
    octave_adapter::{self, Difficulty, Instrument, MarkerKind},
    review_regions::{Issue, Policy, Reason, ReviewMap},
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

fn fixture() -> (Project, Chart) {
    let raw = "[Song]\n{\n Resolution = 192\n Offset = -0.25\n}\n[SyncTrack]\n{\n 0 = B 120000\n 384 = B 60000\n}\n[ExpertDrums]\n{\n 96 = N 2 0\n 96 = N 66 0\n 96 = N 35 0\n 96 = N 41 0\n 192 = N 32 0\n 240 = N 9 12\n 288 = N 3 0\n 288 = N 67 0\n}\n[ExpertSingle]\n{\n 96 = N 0 0\n 96 = N 1 0\n 96 = N 5 0\n 96 = N 6 0\n 384 = N 7 192\n}\n";
    let chart = Chart::parse(raw.into()).unwrap();
    let source_hash = format!("{:x}", Sha256::digest(raw.as_bytes()));
    let audio_hash = "a".repeat(64);
    let tracks = ["ExpertDrums", "ExpertSingle"]
        .into_iter()
        .map(|track| {
            (
                track.into(),
                chart
                    .notes(track)
                    .unwrap()
                    .into_iter()
                    .map(|note| EditorNote {
                        id: format!("{track}-{}-{}", note.lane, note.tick),
                        note,
                    })
                    .collect(),
            )
        })
        .collect();
    let review = ReviewMap::build(
        3.0,
        Policy::default(),
        BTreeMap::from([
            ("chart".into(), source_hash.clone()),
            ("original_audio".into(), audio_hash.clone()),
        ]),
        Some(BTreeSet::from([
            "ExpertDrums".into(),
            "ExpertSingle".into(),
        ])),
        BTreeSet::from([Reason::FastRepeat]),
        vec![
            Issue::at(0.25, "ExpertDrums", Reason::FastRepeat, &[]),
            Issue::at(0.25, "ExpertSingle", Reason::FastRepeat, &[]),
        ],
    )
    .unwrap();
    let project = Project {
        version: 1,
        revision: 2,
        title: "Adapter fixture".into(),
        original_folder: "/fixture".into(),
        source_chart_sha256: source_hash,
        audio_filename: "song.ogg".into(),
        audio_sha256: audio_hash,
        duration: 3.0,
        review,
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
            hidden_flags: Default::default(),
            stems: None,
            keyboard_stems: None,
            stem_mix: Default::default(),
        },
        selected_track: "ExpertSingle".into(),
        selected_region: None,
        selected_note: Some("ExpertSingle-7-384".into()),
        kit_audition: None,
        transport: Transport {
            playback_rate: 1.0,
            position: 0.5,
            playing: false,
            anchor_ms: 0,
            loop_range: None,
        },
        next_id: 1,
        undo: vec!["existing-checkpoint".into()],
        redo: vec![],
        audit: vec![],
    };
    (project, chart)
}

#[test]
fn projects_named_lanes_flags_ids_and_preserved_unknown_events_without_mutation() {
    let (project, chart) = fixture();
    let before = serde_json::to_vec(&project).unwrap();
    let view = octave_adapter::snapshot(&project, &chart.timing).unwrap();
    assert_eq!(serde_json::to_vec(&project).unwrap(), before);
    let drums = view
        .tracks
        .iter()
        .find(|track| track.instrument == Instrument::Drums)
        .unwrap();
    assert_eq!(drums.difficulty, Difficulty::Expert);
    let yellow = drums
        .notes
        .iter()
        .find(|note| note.lane_number == 2)
        .unwrap();
    assert_eq!(yellow.id, "ExpertDrums-2-96");
    assert_eq!(yellow.lane_name, "yellowCymbal");
    assert!(yellow.flags.cymbal && yellow.flags.accent && yellow.flags.ghost);
    assert!(yellow.editable);
    assert!(yellow.read_only_reason.is_none());
    assert_eq!(yellow.velocity, None);
    let blue = drums
        .notes
        .iter()
        .find(|note| note.lane_number == 3)
        .unwrap();
    assert_eq!(blue.lane_name, "blueCymbal");
    assert!(blue.editable);
    let double_kick = drums
        .notes
        .iter()
        .find(|note| note.lane_number == 32)
        .unwrap();
    assert_eq!(double_kick.lane_name, "doubleKick");
    assert!(double_kick.flags.double_kick);
    assert!(!double_kick.editable);
    let marker = drums
        .markers
        .iter()
        .find(|marker| marker.kind == MarkerKind::Cymbal && marker.lane == 66)
        .unwrap();
    assert_eq!(marker.id, "ExpertDrums-66-96");
    assert_eq!(
        marker.target_note_ids.as_slice(),
        std::slice::from_ref(&yellow.id)
    );
    assert_eq!(drums.preserved_unknown_events.len(), 1);
    assert_eq!(drums.preserved_unknown_events[0].lane, 9);
    assert_eq!(drums.preserved_unknown_events[0].duration, 12);
}

#[test]
fn retains_source_resolution_piecewise_timing_open_rows_and_forced_toggle_meaning() {
    let (project, chart) = fixture();
    let view = octave_adapter::snapshot(&project, &chart.timing).unwrap();
    assert_eq!(view.resolution, 192);
    assert_eq!(view.offset_seconds, -0.25);
    assert_eq!(view.tempo_events.len(), 2);
    assert_eq!(view.tempo_events[1].tick, 384);
    let guitar = view
        .tracks
        .iter()
        .find(|track| track.instrument == Instrument::Guitar)
        .unwrap();
    assert_eq!(
        guitar.notes.len(),
        3,
        "control markers are not playable gems"
    );
    let open = guitar
        .notes
        .iter()
        .find(|note| note.lane_name == "open")
        .unwrap();
    assert_eq!(open.id, "ExpertSingle-7-384");
    assert_eq!(open.tick, 384);
    assert_eq!(open.duration, 192);
    assert!((open.seconds - 0.75).abs() < 1e-9);
    assert!((open.end_seconds - 1.75).abs() < 1e-9);
    assert!(open.editable);
    for note in guitar.notes.iter().filter(|note| note.tick == 96) {
        assert!(note.flags.forced && note.flags.tap);
    }
    let force = guitar
        .markers
        .iter()
        .find(|marker| marker.kind == MarkerKind::Forced)
        .unwrap();
    assert_eq!(force.target_note_ids.len(), 2);
    assert_eq!(view.selection.note_id.as_deref(), Some(open.id.as_str()));
    assert!(view.capabilities.undo_available);
    assert!(!view.capabilities.redo_available);
    assert!(
        !view.capabilities.forced_edit
            && !view.capabilities.tempo_edit
            && !view.capabilities.vocal_edit
    );
    let serialized = serde_json::to_string(&view).unwrap();
    assert!(
        !serialized.contains("hopo"),
        "forced is a toggle, not absolute HOPO"
    );
}

#[test]
fn exposes_only_current_decisions_for_each_review_track() {
    let (mut project, chart) = fixture();
    let region = project.review.regions[0].id.clone();
    project.content.decisions.push(Decision {
        region_id: region.clone(),
        track: "ExpertSingle".into(),
        status: DecisionStatus::Reviewed,
        actor: Actor {
            kind: ActorKind::Ai,
            name: "Adapter test".into(),
        },
        comment: "Guitar-specific review".into(),
        chart_sha256: project.source_chart_sha256.clone(),
        region_content_sha256: project
            .region_fingerprint(&region, "ExpertSingle", &chart.timing)
            .unwrap(),
        revision: 2,
        at_ms: 1,
    });
    let view = octave_adapter::snapshot(&project, &chart.timing).unwrap();
    let scopes = &view.review_regions[0].scopes;
    assert!(
        scopes
            .iter()
            .find(|scope| scope.track == "ExpertSingle")
            .unwrap()
            .current_decision
            .is_some()
    );
    assert!(
        scopes
            .iter()
            .find(|scope| scope.track == "ExpertDrums")
            .unwrap()
            .current_decision
            .is_none()
    );
    project.content.tracks.get_mut("ExpertDrums").unwrap()[0]
        .note
        .tick = 100;
    assert!(
        octave_adapter::snapshot(&project, &chart.timing)
            .unwrap()
            .review_regions[0]
            .scopes
            .iter()
            .find(|scope| scope.track == "ExpertSingle")
            .unwrap()
            .current_decision
            .is_some()
    );
    project.content.tracks.get_mut("ExpertSingle").unwrap()[0]
        .note
        .tick = 100;
    assert!(
        octave_adapter::snapshot(&project, &chart.timing)
            .unwrap()
            .review_regions[0]
            .scopes
            .iter()
            .all(|scope| scope.current_decision.is_none())
    );
}

#[test]
fn refuses_invalid_time_and_tick_overflow_without_using_a_fallback_resolution() {
    let (mut project, mut chart) = fixture();
    chart.timing.resolution = 0;
    assert!(octave_adapter::snapshot(&project, &chart.timing).is_err());
    chart.timing.resolution = 192;
    project.content.tracks.get_mut("ExpertSingle").unwrap()[0]
        .note
        .tick = u64::MAX;
    project.content.tracks.get_mut("ExpertSingle").unwrap()[0]
        .note
        .sustain = 1;
    assert!(octave_adapter::snapshot(&project, &chart.timing).is_err());
}
