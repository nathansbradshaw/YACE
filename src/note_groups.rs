//! Atomic note-group edits and a portable, typed text clipboard.
use crate::{
    chart::Note,
    editor::{EditorNote, Project},
    timing::TempoMap,
    vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const PREFIX: &str = "YACE notes v1\n";
pub const MAX_NOTES: usize = 100_000;
pub const MAX_CLIPBOARD_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub track: String,
    pub ids: BTreeSet<String>,
    pub vocals: BTreeSet<usize>,
}
impl Selection {
    pub fn len(&self) -> usize {
        if self.track == "Vocals" {
            self.vocals.len()
        } else {
            self.ids.len()
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gem {
    pub note: Note,
    pub flags: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midi: Option<crate::song_files::game_midi::NoteAttributes>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Payload {
    Instrument { family: String, notes: Vec<Gem> },
    Vocals { chart: VocalChart },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Clipboard {
    pub version: u32,
    pub resolution: u32,
    pub payload: Payload,
}

pub fn family(track: &str) -> &'static str {
    if track == "Vocals" {
        "vocals"
    } else if let Some(limit) = crate::pro_guitar::max_fret(track) {
        if crate::pro_guitar::is_bass(track) && limit == 17 {
            "pro_bass_17"
        } else if crate::pro_guitar::is_bass(track) {
            "pro_bass_22"
        } else if limit == 17 {
            "pro_guitar_17"
        } else {
            "pro_guitar_22"
        }
    } else if crate::six_fret::is_track(track) {
        "six_fret"
    } else if crate::keys::is_pro(track) {
        "pro_keys"
    } else if track.ends_with("Drums") {
        "drums"
    } else if crate::keys::is_keys(track) {
        "keys"
    } else {
        "frets"
    }
}
fn project_family<'a>(project: &Project, track: &'a str) -> &'a str {
    if track.ends_with("Drums") && project.drum_layout() == crate::drum_layout::Layout::FiveLane {
        "five_lane_drums"
    } else {
        family(track)
    }
}
fn digest(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn fingerprint(project: &Project, track: &str) -> Result<String> {
    if track == "Vocals" {
        digest(&(
            &project.source_chart_sha256,
            &project.content.sync,
            &project.content.vocals,
        ))
    } else {
        digest(&(
            &project.source_chart_sha256,
            &project.content.sync,
            project.content.tracks.get(track).context("Unknown track")?,
        ))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub track: String,
    pub fingerprint: String,
    pub ids: BTreeSet<String>,
    pub vocals: BTreeSet<usize>,
}
impl Snapshot {
    pub fn capture(project: &Project, selected: &Selection) -> Result<Self> {
        ensure!(
            !selected.is_empty() && selected.len() <= MAX_NOTES,
            "Select 1–100,000 notes; larger operations must be split"
        );
        if selected.track == "Vocals" {
            let chart = project.content.vocals.as_ref().context("No vocal chart")?;
            ensure!(
                selected.ids.is_empty()
                    && selected
                        .vocals
                        .iter()
                        .all(|&index| index < chart.notes.len()),
                "Vocal selection changed"
            );
        } else {
            let notes = project
                .content
                .tracks
                .get(&selected.track)
                .context("Unknown track")?;
            let playable: BTreeSet<_> = notes
                .iter()
                .filter(|item| project.playable(&selected.track, item.note.lane))
                .map(|item| &item.id)
                .collect();
            ensure!(
                selected.vocals.is_empty() && selected.ids.iter().all(|id| playable.contains(id)),
                "Note selection changed"
            );
        }
        Ok(Self {
            track: selected.track.clone(),
            fingerprint: fingerprint(project, &selected.track)?,
            ids: selected.ids.clone(),
            vocals: selected.vocals.clone(),
        })
    }
    pub(crate) fn validate(&self, project: &Project) -> Result<()> {
        Self::capture(
            project,
            &Selection {
                track: self.track.clone(),
                ids: self.ids.clone(),
                vocals: self.vocals.clone(),
            },
        )?;
        ensure!(
            self.fingerprint == fingerprint(project, &self.track)?,
            "Notes changed during the edit. Select them again."
        );
        Ok(())
    }
}

pub(crate) fn attack_marker(track: &str, flag: u8) -> bool {
    if crate::pro_guitar::is_track(track) {
        flag == crate::pro_guitar::HOPO
    } else {
        !crate::keys::is_pro(track) && (5..=6).contains(&flag)
    }
}
fn owns_flag(layout: crate::drum_layout::Layout, track: &str, lane: u8, flag: u8) -> bool {
    if track.ends_with("Drums") {
        layout.owns_flag(lane, flag)
    } else {
        attack_marker(track, flag)
    }
}
fn gems(project: &Project, snapshot: &Snapshot) -> Vec<Gem> {
    let notes = &project.content.tracks[&snapshot.track];
    let mut flags: BTreeMap<u64, Vec<u8>> = BTreeMap::new();
    for item in notes
        .iter()
        .filter(|item| !project.playable(&snapshot.track, item.note.lane))
    {
        flags
            .entry(item.note.tick)
            .or_default()
            .push(item.note.lane);
    }
    notes
        .iter()
        .filter(|item| snapshot.ids.contains(&item.id))
        .map(|item| Gem {
            note: item.note.clone(),
            midi: item.midi.clone(),
            flags: flags
                .get(&item.note.tick)
                .into_iter()
                .flatten()
                .copied()
                .filter(|flag| {
                    owns_flag(
                        project.drum_layout(),
                        &snapshot.track,
                        item.note.lane,
                        *flag,
                    )
                })
                .collect(),
        })
        .collect()
}
pub fn copy(project: &Project, timing: &TempoMap, selection: &Selection) -> Result<Clipboard> {
    let snapshot = Snapshot::capture(project, selection)?;
    let payload = if selection.track == "Vocals" {
        let original = project.content.vocals.as_ref().context("No vocal chart")?;
        closed_words(original, &selection.vocals)?;
        let notes: Vec<_> = original
            .notes
            .iter()
            .enumerate()
            .filter(|(i, _)| selection.vocals.contains(i))
            .map(|(_, note)| note.clone())
            .collect();
        let phrases = original
            .phrases
            .iter()
            .filter_map(|phrase| {
                let start = notes.partition_point(|n| n.start < phrase.start);
                let end = notes.partition_point(|n| n.end <= phrase.end);
                let owned = notes.get(start..end)?;
                Some(VocalPhrase {
                    start: owned.first()?.start,
                    end: owned.last()?.end,
                    ..phrase.clone()
                })
            })
            .collect();
        let mut chart = VocalChart {
            notes,
            phrases,
            ..original.clone()
        };
        chart.validate().context(
            "Include the owning syllable and the rest of a joined word when copying vocals",
        )?;
        let start = chart.notes[0].start;
        for note in &mut chart.notes {
            note.start -= start;
            note.end -= start;
        }
        for phrase in &mut chart.phrases {
            phrase.start -= start;
            phrase.end -= start;
        }
        Payload::Vocals { chart }
    } else {
        let mut notes = gems(project, &snapshot);
        let start = notes
            .iter()
            .map(|gem| gem.note.tick)
            .min()
            .context("No selected notes")?;
        for gem in &mut notes {
            gem.note.tick -= start;
        }
        Payload::Instrument {
            family: project_family(project, &selection.track).into(),
            notes,
        }
    };
    Ok(Clipboard {
        version: 1,
        resolution: timing.resolution,
        payload,
    })
}
impl Clipboard {
    pub fn encode(&self) -> Result<String> {
        self.validate()?;
        let encoded = format!("{PREFIX}{}", serde_json::to_string(self)?);
        ensure!(
            encoded.len() <= MAX_CLIPBOARD_BYTES,
            "Clipboard selection exceeds 32 MiB; copy fewer notes"
        );
        Ok(encoded)
    }
    pub fn decode(text: &str) -> Result<Self> {
        ensure!(
            text.len() <= MAX_CLIPBOARD_BYTES,
            "Clipboard selection exceeds 32 MiB; copy fewer notes"
        );
        let value: Self = serde_json::from_str(
            text.strip_prefix(PREFIX)
                .context("Copy notes in YACE before pasting")?,
        )?;
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && self.resolution > 0,
            "Unsupported note clipboard"
        );
        match &self.payload {
            Payload::Instrument { family, notes } => {
                ensure!(
                    notes.len() <= MAX_NOTES,
                    "Clipboard contains more than 100,000 notes; split the selection"
                );
                ensure!(
                    [
                        "frets",
                        "six_fret",
                        "drums",
                        "five_lane_drums",
                        "keys",
                        "pro_keys",
                        "pro_guitar_17",
                        "pro_guitar_22",
                        "pro_bass_17",
                        "pro_bass_22"
                    ]
                    .contains(&family.as_str())
                        && !notes.is_empty(),
                    "Invalid note clipboard"
                );
                ensure!(
                    notes.iter().all(|gem| gem.flags.len() <= 4),
                    "Invalid note flags"
                );
            }
            Payload::Vocals { chart } => {
                ensure!(
                    chart.notes.len() <= MAX_NOTES,
                    "Clipboard contains more than 100,000 notes; split the selection"
                );
                ensure!(
                    chart.resolution == self.resolution,
                    "Invalid vocal clipboard"
                );
                chart.validate()?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    Delete {
        selection: Snapshot,
    },
    Move {
        selection: Snapshot,
        ticks: i64,
        rows: i16,
    },
    Paste {
        track: String,
        tick: u64,
        clipboard: Clipboard,
    },
}
fn shifted(tick: u64, delta: i64) -> Result<u64> {
    tick.checked_add_signed(delta)
        .context("The group would start before the chart or overflow")
}
fn remove_gems(project: &mut Project, selection: &Snapshot) {
    let track = &selection.track;
    let chosen = gems(project, selection);
    let layout = project.drum_layout();
    let lanes = project.lane_rows(track);
    let notes = project.content.tracks.get_mut(track).unwrap();
    notes.retain(|item| !selection.ids.contains(&item.id));
    let shared: BTreeSet<_> = notes
        .iter()
        .filter(|n| lanes.contains(&n.note.lane))
        .map(|n| n.note.tick)
        .collect();
    let mut removed_flags = BTreeSet::new();
    for gem in chosen {
        if track.ends_with("Drums") || !shared.contains(&gem.note.tick) {
            removed_flags.extend(
                gem.flags
                    .into_iter()
                    .filter(|flag| owns_flag(layout, track, gem.note.lane, *flag))
                    .map(|flag| (gem.note.tick, flag)),
            );
        }
    }
    notes.retain(|item| !removed_flags.contains(&(item.note.tick, item.note.lane)));
}
struct DestinationIndex {
    occupied: BTreeMap<u64, BTreeSet<u8>>,
    markers: BTreeSet<(u64, u8)>,
}
impl DestinationIndex {
    fn new(project: &Project, track: &str) -> Result<Self> {
        let notes = project
            .content
            .tracks
            .get(track)
            .context("Select an existing destination track")?;
        let mut index = Self {
            occupied: BTreeMap::new(),
            markers: BTreeSet::new(),
        };
        for item in notes {
            index.markers.insert((item.note.tick, item.note.lane));
            if project.playable(track, item.note.lane) {
                index
                    .occupied
                    .entry(item.note.tick)
                    .or_default()
                    .insert(item.note.lane);
            }
        }
        Ok(index)
    }
}
fn insert_gem(
    project: &mut Project,
    track: &str,
    gem: Gem,
    id: Option<String>,
    index: &mut DestinationIndex,
) -> Result<String> {
    if crate::pro_guitar::is_track(track) {
        crate::pro_guitar::validate_attributes(track, gem.midi.as_ref())?;
    }
    if let Some(midi) = &gem.midi {
        midi.validate()?;
    }
    ensure!(
        project.playable(track, gem.note.lane),
        "This note does not fit the destination track"
    );
    ensure!(
        gem.flags
            .iter()
            .all(|flag| owns_flag(project.drum_layout(), track, gem.note.lane, *flag)),
        "This flag does not fit the destination lane"
    );
    ensure!(
        index.occupied.get(&gem.note.tick).is_none_or(|lanes| {
            !lanes.contains(&gem.note.lane)
                && (track.ends_with("Drums")
                    || if gem.note.lane == 7 {
                        lanes.iter().all(|lane| *lane == 7)
                    } else {
                        !lanes.contains(&7)
                    })
        }),
        "Notes already occupy the destination. Nothing was changed."
    );
    index
        .occupied
        .entry(gem.note.tick)
        .or_default()
        .insert(gem.note.lane);
    index.markers.insert((gem.note.tick, gem.note.lane));
    let id = if let Some(id) = id {
        id
    } else {
        let id = format!("edit-note-{}", project.next_id);
        project.next_id = project.next_id.checked_add(1).context("Note ID overflow")?;
        id
    };
    let notes = project.content.tracks.get_mut(track).unwrap();
    for lane in gem.flags {
        if index.markers.insert((gem.note.tick, lane)) {
            let marker = format!("edit-note-{}", project.next_id);
            project.next_id = project.next_id.checked_add(1).context("Note ID overflow")?;
            notes.push(EditorNote {
                midi: None,
                id: marker,
                note: Note {
                    tick: gem.note.tick,
                    lane,
                    sustain: 0,
                },
            });
        }
    }
    notes.push(EditorNote {
        midi: gem.midi,
        id: id.clone(),
        note: gem.note,
    });
    Ok(id)
}
fn scaled(tick: u64, from: u32, to: u32, anchor: u64) -> Result<u64> {
    let scaled = (tick as u128 * to as u128 + from as u128 / 2) / from as u128;
    anchor
        .checked_add(u64::try_from(scaled)?)
        .context("Clipboard timing overflow")
}

pub fn apply(project: &mut Project, timing: &TempoMap, edit: &Edit) -> Result<()> {
    match edit {
        Edit::Delete { selection } | Edit::Move { selection, .. } => {
            selection.validate(project)?;
            if selection.track == "Vocals" {
                edit_vocals(project, selection, edit)?;
            } else {
                let selected: Vec<_> = project.content.tracks[&selection.track]
                    .iter()
                    .filter(|item| selection.ids.contains(&item.id))
                    .cloned()
                    .collect();
                let mut bundles = gems(project, selection);
                remove_gems(project, selection);
                if let Edit::Move { ticks, rows, .. } = edit {
                    let lanes = project.lane_rows(&selection.track);
                    let mut index = DestinationIndex::new(project, &selection.track)?;
                    for (item, gem) in selected.iter().zip(&mut bundles) {
                        let row = lanes
                            .iter()
                            .position(|lane| *lane == item.note.lane)
                            .context("Unknown note lane")? as i16;
                        let lane = lanes
                            .get(
                                usize::try_from(row + rows)
                                    .context("The group would leave the lane grid")?,
                            )
                            .context("The group would leave the lane grid")?;
                        gem.note.tick = shifted(gem.note.tick, *ticks)?;
                        gem.note.lane = *lane;
                        if selection.track.ends_with("Drums") {
                            for flag in &mut gem.flags {
                                *flag = lane
                                    .checked_add(*flag - item.note.lane)
                                    .context("Drum flag overflow")?;
                            }
                        }
                        insert_gem(
                            project,
                            &selection.track,
                            gem.clone(),
                            Some(item.id.clone()),
                            &mut index,
                        )?;
                    }
                }
            }
            project.selected_track = selection.track.clone();
            project.selected_note = None;
        }
        Edit::Paste {
            track,
            tick,
            clipboard,
        } => {
            let clipboard = Clipboard::decode(&clipboard.encode()?)?;
            ensure!(
                timing.seconds(*tick).is_finite() && timing.seconds(*tick) >= 0.,
                "Paste point is outside the recording"
            );
            match clipboard.payload {
                Payload::Instrument {
                    family: source,
                    mut notes,
                } => {
                    ensure!(
                        source == project_family(project, track) && track != "Vocals",
                        "Choose a compatible track before pasting"
                    );
                    let mut first = None;
                    let mut index = DestinationIndex::new(project, track)?;
                    for gem in &mut notes {
                        let end = scaled(
                            gem.note
                                .tick
                                .checked_add(gem.note.sustain)
                                .context("Note timing overflow")?,
                            clipboard.resolution,
                            timing.resolution,
                            *tick,
                        )?;
                        gem.note.tick = scaled(
                            gem.note.tick,
                            clipboard.resolution,
                            timing.resolution,
                            *tick,
                        )?;
                        let sustain = end
                            .checked_sub(gem.note.tick)
                            .context("Invalid note duration")?;
                        ensure!(
                            gem.note.sustain == 0 || sustain > 0,
                            "This chart resolution is too small for the copied sustain"
                        );
                        gem.note.sustain = sustain;
                        let id = insert_gem(project, track, gem.clone(), None, &mut index)?;
                        if first.is_none() {
                            first = Some(id);
                        }
                    }
                    project.selected_note = first;
                }
                Payload::Vocals { chart } => {
                    ensure!(track == "Vocals", "Select Vocals before pasting");
                    paste_vocals(project, timing, *tick, clipboard.resolution, chart)?;
                    project.selected_note = None;
                }
            }
            project.selected_track = track.clone();
        }
    }
    Ok(())
}

fn merge_phrases(mut phrases: Vec<VocalPhrase>) -> Result<Vec<VocalPhrase>> {
    phrases.sort_by_key(|phrase| phrase.start);
    let mut merged: Vec<VocalPhrase> = Vec::new();
    for phrase in phrases {
        if let Some(last) = merged.last_mut()
            && phrase.start < last.end
        {
            ensure!(
                last.player == phrase.player && last.midi == phrase.midi,
                "Vocal phrases have different players or MIDI attributes"
            );
            last.end = last.end.max(phrase.end);
        } else {
            merged.push(phrase);
        }
    }
    Ok(merged)
}
// Keep syllables attached to their original words when notes change order.
fn word_links(chart: &VocalChart) -> BTreeSet<(usize, usize, bool)> {
    let mut links = BTreeSet::new();
    for phrase in &chart.phrases {
        let mut owner = None;
        let mut joined = None;
        let start = chart.notes.partition_point(|n| n.start < phrase.start);
        let end = chart.notes.partition_point(|n| n.end <= phrase.end);
        for (i, note) in chart
            .notes
            .iter()
            .enumerate()
            .skip(start)
            .take(end.saturating_sub(start))
        {
            match &note.lyric {
                Syllable::Text { join_next, .. } => {
                    if let Some(previous) = joined.take() {
                        links.insert((previous, i, true));
                    }
                    owner = Some(i);
                    if *join_next {
                        joined = Some(i);
                    }
                }
                Syllable::Continue | Syllable::ContinueWord => {
                    if let Some(owner) = owner {
                        links.insert((owner, i, false));
                    }
                }
                Syllable::Missing => {
                    owner = None;
                }
            }
        }
    }
    links
}
fn closed_words(chart: &VocalChart, selected: &BTreeSet<usize>) -> Result<()> {
    for (a, b, joined) in word_links(chart) {
        ensure!(
            selected.contains(&a) == selected.contains(&b),
            "{}",
            if joined {
                "Select the whole joined word"
            } else {
                "Select the owning syllable and its continuations together"
            }
        );
    }
    Ok(())
}
fn preserve_words(
    chart: &VocalChart,
    tags: &[usize],
    expected: BTreeSet<(usize, usize, bool)>,
) -> Result<()> {
    let actual = word_links(chart)
        .into_iter()
        .map(|(a, b, kind)| (tags[a], tags[b], kind))
        .collect();
    ensure!(
        expected == actual,
        "This placement would change which word a vocal note belongs to"
    );
    Ok(())
}
fn edit_vocals(project: &mut Project, selection: &Snapshot, edit: &Edit) -> Result<()> {
    let original = project.content.vocals.as_ref().context("No vocal chart")?;
    closed_words(original, &selection.vocals)?;
    let mut notes: Vec<(usize, VocalNote)> = original.notes.iter().cloned().enumerate().collect();
    match edit {
        Edit::Delete { .. } => notes.retain(|(i, _)| !selection.vocals.contains(i)),
        Edit::Move { ticks, rows, .. } => {
            for (i, note) in &mut notes {
                if selection.vocals.contains(i) {
                    note.start = shifted(note.start, *ticks)?;
                    note.end = shifted(note.end, *ticks)?;
                    if let VocalTone::Pitched { midi } = &mut note.tone {
                        let pitch = *midi as i16 - rows;
                        ensure!(
                            (36..=84).contains(&pitch),
                            "The group would leave the vocal pitch range"
                        );
                        *midi = pitch as u8;
                    } else {
                        ensure!(
                            *rows == 0,
                            "Move pitched notes separately from talkies when changing pitch"
                        );
                    }
                }
            }
        }
        Edit::Paste { .. } => unreachable!(),
    }
    if notes.is_empty() {
        project.content.vocals = None;
        project.content.vocal_review = None;
        return Ok(());
    }
    let mut bounds = vec![None::<(u64, u64)>; original.phrases.len()];
    for (i, note) in &notes {
        let index = original
            .phrases
            .partition_point(|p| p.start <= original.notes[*i].start)
            .checked_sub(1)
            .context("Vocal note lost its scoring phrase")?;
        let entry = &mut bounds[index];
        *entry = Some(entry.map_or((note.start, note.end), |(start, end)| {
            (start.min(note.start), end.max(note.end))
        }));
    }
    let mut phrases = Vec::new();
    for (phrase, bound) in original.phrases.iter().zip(bounds) {
        if let Some((start, end)) = bound {
            let first = original.notes.partition_point(|n| n.start < phrase.start);
            let last = original.notes.partition_point(|n| n.start < phrase.end);
            let mut updated = phrase.clone();
            if let Edit::Move { ticks, .. } = edit
                && selection.vocals.range(first..last).count() == last - first
            {
                // Moving the whole phrase carries its authored leading and
                // trailing space, rather than rebuilding it from note edges.
                updated.start = shifted(phrase.start, *ticks)?;
                updated.end = shifted(phrase.end, *ticks)?;
            } else {
                // Retained/unselected phrases keep their exact scoring edges.
                // A partial move extends them only when a note needs room.
                updated.start = phrase.start.min(start);
                updated.end = phrase.end.max(end);
            }
            phrases.push(updated);
        }
    }
    phrases.sort_by_key(|phrase| phrase.start);
    ensure!(
        phrases.windows(2).all(|pair| pair[0].end <= pair[1].start),
        "The group would overlap scoring phrases. Move or resize their boundaries first; nothing was changed."
    );
    notes.sort_by_key(|(_, note)| note.start);
    let tags: Vec<_> = notes.iter().map(|(i, _)| *i).collect();
    let retained: BTreeSet<_> = tags.iter().copied().collect();
    let expected = word_links(original)
        .into_iter()
        .filter(|(a, b, _)| retained.contains(a) && retained.contains(b))
        .collect();
    let chart = VocalChart {
        notes: notes.into_iter().map(|(_, note)| note).collect(),
        phrases,
        ..original.clone()
    };
    chart.validate()?;
    preserve_words(&chart, &tags, expected)?;
    project.content.vocals = Some(chart);
    Ok(())
}
fn paste_vocals(
    project: &mut Project,
    timing: &TempoMap,
    tick: u64,
    resolution: u32,
    mut chart: VocalChart,
) -> Result<()> {
    for note in &mut chart.notes {
        note.start = scaled(note.start, resolution, timing.resolution, tick)?;
        note.end = scaled(note.end, resolution, timing.resolution, tick)?;
    }
    for phrase in &mut chart.phrases {
        phrase.start = scaled(phrase.start, resolution, timing.resolution, tick)?;
        phrase.end = scaled(phrase.end, resolution, timing.resolution, tick)?;
    }
    chart.resolution = timing.resolution;
    let mut expected = word_links(&chart);
    let mut tagged: Vec<_> = chart.notes.into_iter().enumerate().collect();
    if let Some(current) = &project.content.vocals {
        let offset = tagged.len();
        expected.extend(
            word_links(current)
                .into_iter()
                .map(|(a, b, k)| (a + offset, b + offset, k)),
        );
        tagged.extend(
            current
                .notes
                .iter()
                .cloned()
                .enumerate()
                .map(|(i, n)| (i + offset, n)),
        );
        chart.phrases.extend(current.phrases.iter().cloned());
        chart.source = current.source.clone();
    }
    tagged.sort_by_key(|(_, n)| n.start);
    let tags: Vec<_> = tagged.iter().map(|(i, _)| *i).collect();
    chart.notes = tagged.into_iter().map(|(_, n)| n).collect();
    chart.phrases = merge_phrases(chart.phrases)?;
    chart.validate()?;
    preserve_words(&chart, &tags, expected)?;
    project.content.vocals = Some(chart);
    Ok(())
}
