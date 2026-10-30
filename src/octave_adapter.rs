//! Present editor data using OCTAVE's named lanes.
//! Saving and export still use the editor's normal format.
use crate::{
    editor::{self, Decision, Project, Transport},
    review_regions::Region,
    timing::{Tempo, TempoMap},
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Instrument {
    Guitar,
    Bass,
    Drums,
    Keys,
    ProKeys,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
    Expert,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Flags {
    /// .chart's forced toggle; this is not an absolute HOPO classification.
    pub forced: bool,
    pub tap: bool,
    pub cymbal: bool,
    pub accent: bool,
    pub ghost: bool,
    pub double_kick: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Note {
    pub id: String,
    pub tick: u64,
    pub duration: u64,
    pub seconds: f64,
    pub end_seconds: f64,
    pub lane_name: String,
    pub lane_number: u8,
    /// The source .chart has no measured velocity field.
    pub velocity: Option<u8>,
    pub flags: Flags,
    pub editable: bool,
    pub read_only_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerKind {
    Forced,
    Tap,
    Cymbal,
    Accent,
    Ghost,
}

#[derive(Clone, Debug, Serialize)]
pub struct Marker {
    pub id: String,
    pub tick: u64,
    pub lane: u8,
    pub duration: u64,
    pub kind: MarkerKind,
    pub target_note_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PreservedEvent {
    pub id: String,
    pub tick: u64,
    pub lane: u8,
    pub duration: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Track {
    pub key: String,
    pub instrument: Instrument,
    pub difficulty: Difficulty,
    pub notes: Vec<Note>,
    pub markers: Vec<Marker>,
    pub preserved_unknown_events: Vec<PreservedEvent>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReviewScope {
    pub track: String,
    pub content_sha256: String,
    pub current_decision: Option<Decision>,
    pub hidden: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReviewRegion {
    pub region: Region,
    /// Decisions apply independently to each instrument/difficulty in a region.
    pub scopes: Vec<ReviewScope>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Selection {
    pub active_track: String,
    pub note_id: Option<String>,
    pub region_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Capabilities {
    pub note_add: bool,
    pub note_update: bool,
    pub note_delete: bool,
    pub cymbal_edit: bool,
    pub review_decisions: bool,
    pub flag_visibility: bool,
    pub transport: bool,
    pub export: bool,
    pub undo_available: bool,
    pub redo_available: bool,
    pub multi_selection: bool,
    pub bulk_edit: bool,
    pub tempo_edit: bool,
    pub metadata_edit: bool,
    pub forced_edit: bool,
    pub tap_edit: bool,
    pub articulation_edit: bool,
    pub vocal_edit: bool,
    pub stem_mixer: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub version: u32,
    pub title: String,
    pub revision: u64,
    pub source_chart_sha256: String,
    pub audio_sha256: String,
    pub resolution: u32,
    pub offset_seconds: f64,
    pub tempo_events: Vec<Tempo>,
    pub duration_seconds: f64,
    pub tracks: Vec<Track>,
    pub selection: Selection,
    /// Durable transport baseline; the active audio clock remains authoritative.
    pub transport: Transport,
    pub review_regions: Vec<ReviewRegion>,
    pub capabilities: Capabilities,
}

fn track_kind(track: &str) -> Result<(Instrument, Difficulty)> {
    for (prefix, difficulty) in [
        ("Easy", Difficulty::Easy),
        ("Medium", Difficulty::Medium),
        ("Hard", Difficulty::Hard),
        ("Expert", Difficulty::Expert),
    ] {
        if let Some(suffix) = track.strip_prefix(prefix) {
            let instrument = match suffix {
                "Single" => Instrument::Guitar,
                "DoubleBass" => Instrument::Bass,
                "Drums" => Instrument::Drums,
                "Keyboard" => Instrument::Keys,
                "ProKeys" => Instrument::ProKeys,
                _ => continue,
            };
            return Ok((instrument, difficulty));
        }
    }
    anyhow::bail!("Unsupported editor track {track}")
}

fn marker_kind(drums: bool, lane: u8) -> Option<MarkerKind> {
    match (drums, lane) {
        (false, 5) => Some(MarkerKind::Forced),
        (false, 6) => Some(MarkerKind::Tap),
        (true, 66..=68) => Some(MarkerKind::Cymbal),
        (true, 34..=38) => Some(MarkerKind::Accent),
        (true, 40..=44) => Some(MarkerKind::Ghost),
        _ => None,
    }
}

pub(crate) fn lane_name(drums: bool, lane: u8, cymbal: bool) -> Option<&'static str> {
    if drums {
        match (lane, cymbal) {
            (0, _) => Some("kick"),
            (1, _) => Some("snare"),
            (2, false) => Some("yellowTom"),
            (2, true) => Some("yellowCymbal"),
            (3, false) => Some("blueTom"),
            (3, true) => Some("blueCymbal"),
            (4, false) => Some("greenTom"),
            (4, true) => Some("greenCymbal"),
            (32, _) => Some("doubleKick"),
            _ => None,
        }
    } else {
        match lane {
            0 => Some("green"),
            1 => Some("red"),
            2 => Some("yellow"),
            3 => Some("blue"),
            4 => Some("orange"),
            7 => Some("open"),
            _ => None,
        }
    }
}

/// Project a validated editor project for a Rust port of OCTAVE's panels.
/// Existing IDs/PPQ/offsets are preserved. All writes still use editor::Request.
pub fn snapshot(project: &Project, timing: &TempoMap) -> Result<Snapshot> {
    ensure!(project.version == 1, "Unsupported editor project version");
    ensure!(
        project.duration.is_finite() && project.duration > 0.0,
        "Invalid project duration"
    );
    ensure!(
        timing.resolution > 0
            && timing.offset.is_finite()
            && timing.tempos.first().is_some_and(|tempo| tempo.tick == 0)
            && timing
                .tempos
                .iter()
                .all(|tempo| tempo.bpm.is_finite() && (1.0..=1000.0).contains(&tempo.bpm))
            && timing
                .tempos
                .windows(2)
                .all(|pair| pair[0].tick < pair[1].tick),
        "Invalid editor timing"
    );
    let mut tracks = Vec::new();
    for (key, events) in &project.content.tracks {
        let (instrument, difficulty) = track_kind(key)?;
        let drums = instrument == Instrument::Drums;
        let positions: BTreeSet<_> = events
            .iter()
            .map(|event| (event.note.tick, event.note.lane))
            .collect();
        let has = |tick, lane| positions.contains(&(tick, lane));
        let mut track = Track {
            key: key.clone(),
            instrument,
            difficulty,
            notes: vec![],
            markers: vec![],
            preserved_unknown_events: vec![],
        };
        for event in events {
            let raw = &event.note;
            let end = raw
                .tick
                .checked_add(raw.sustain)
                .context("Note tick overflow")?;
            if let Some(kind) = (!crate::keys::is_keys(key))
                .then(|| marker_kind(drums, raw.lane))
                .flatten()
            {
                track.markers.push(Marker {
                    id: event.id.clone(),
                    tick: raw.tick,
                    lane: raw.lane,
                    duration: raw.sustain,
                    kind,
                    target_note_ids: vec![],
                });
                continue;
            }
            let flags = if crate::keys::is_keys(key) {
                Flags::default()
            } else if drums {
                Flags {
                    cymbal: (2..=4).contains(&raw.lane) && has(raw.tick, raw.lane + 64),
                    accent: (1..=4).contains(&raw.lane) && has(raw.tick, raw.lane + 33),
                    ghost: (1..=4).contains(&raw.lane) && has(raw.tick, raw.lane + 39),
                    double_kick: raw.lane == 32,
                    ..Flags::default()
                }
            } else {
                Flags {
                    forced: has(raw.tick, 5),
                    tap: has(raw.tick, 6),
                    ..Flags::default()
                }
            };
            let projected_lane = if crate::keys::is_pro(key) {
                editor::playable(key, raw.lane).then(|| crate::keys::pitch_name(raw.lane))
            } else {
                lane_name(drums, raw.lane, flags.cymbal)
            };
            let Some(lane_name) = projected_lane else {
                track.preserved_unknown_events.push(PreservedEvent {
                    id: event.id.clone(),
                    tick: raw.tick,
                    lane: raw.lane,
                    duration: raw.sustain,
                });
                continue;
            };
            let read_only_reason = if !editor::playable(key, raw.lane) {
                Some("This preserved lane is not editable by the current command engine".into())
            } else {
                None
            };
            track.notes.push(Note {
                id: event.id.clone(),
                tick: raw.tick,
                duration: raw.sustain,
                seconds: timing.seconds(raw.tick),
                end_seconds: timing.seconds(end),
                lane_name: lane_name.into(),
                lane_number: raw.lane,
                velocity: None,
                flags,
                editable: read_only_reason.is_none(),
                read_only_reason,
            });
        }
        for marker in &mut track.markers {
            let lane = match marker.kind {
                MarkerKind::Cymbal => Some(marker.lane - 64),
                MarkerKind::Accent => Some(marker.lane - 33),
                MarkerKind::Ghost => Some(marker.lane - 39),
                MarkerKind::Forced | MarkerKind::Tap => None,
            };
            marker.target_note_ids = track
                .notes
                .iter()
                .filter(|note| {
                    note.tick == marker.tick && lane.is_none_or(|lane| lane == note.lane_number)
                })
                .map(|note| note.id.clone())
                .collect();
            marker.target_note_ids.sort();
        }
        track
            .notes
            .sort_by(|a, b| (a.tick, a.lane_number, &a.id).cmp(&(b.tick, b.lane_number, &b.id)));
        track
            .markers
            .sort_by(|a, b| (a.tick, a.lane, &a.id).cmp(&(b.tick, b.lane, &b.id)));
        track
            .preserved_unknown_events
            .sort_by(|a, b| (a.tick, a.lane, &a.id).cmp(&(b.tick, b.lane, &b.id)));
        tracks.push(track);
    }
    let review_regions = project
        .review
        .regions
        .iter()
        .map(|region| {
            let scopes = region
                .tracks
                .iter()
                .map(|track| {
                    Ok(ReviewScope {
                        track: track.clone(),
                        content_sha256: project.region_fingerprint(&region.id, track, timing)?,
                        current_decision: project.decision(&region.id, track, timing).cloned(),
                        hidden: project.is_flag_hidden(&region.id, track),
                    })
                })
                .collect::<Result<_>>()?;
            Ok(ReviewRegion {
                region: region.clone(),
                scopes,
            })
        })
        .collect::<Result<_>>()?;
    Ok(Snapshot {
        version: 1,
        title: project.title.clone(),
        revision: project.revision,
        source_chart_sha256: project.source_chart_sha256.clone(),
        audio_sha256: project.audio_sha256.clone(),
        resolution: timing.resolution,
        offset_seconds: timing.offset,
        tempo_events: timing.tempos.clone(),
        duration_seconds: project.duration,
        tracks,
        selection: Selection {
            active_track: project.selected_track.clone(),
            note_id: project.selected_note.clone(),
            region_id: project.selected_region.clone(),
        },
        transport: project.transport.clone(),
        review_regions,
        capabilities: Capabilities {
            note_add: true,
            note_update: true,
            note_delete: true,
            cymbal_edit: true,
            review_decisions: true,
            flag_visibility: true,
            transport: true,
            export: true,
            undo_available: !project.undo.is_empty(),
            redo_available: !project.redo.is_empty(),
            multi_selection: false,
            bulk_edit: false,
            tempo_edit: false,
            metadata_edit: false,
            forced_edit: false,
            tap_edit: false,
            articulation_edit: false,
            vocal_edit: false,
            stem_mixer: true,
        },
    })
}
