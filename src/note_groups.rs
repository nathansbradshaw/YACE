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
const MAX_NOTES: usize = 10_000;

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
            "Select 1–10,000 notes"
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
                .filter(|item| crate::editor::playable(&selected.track, item.note.lane))
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

fn owns_flag(track: &str, lane: u8, flag: u8) -> bool {
    if track.ends_with("Drums") {
        ((2..=4).contains(&lane) && flag == lane + 64)
            || ((1..=4).contains(&lane) && [lane + 33, lane + 39].contains(&flag))
    } else {
        !crate::keys::is_pro(track) && (5..=6).contains(&flag)
    }
}
fn gems(project: &Project, snapshot: &Snapshot) -> Vec<Gem> {
    let notes = &project.content.tracks[&snapshot.track];
    let mut flags: BTreeMap<u64, Vec<u8>> = BTreeMap::new();
    for item in notes
        .iter()
        .filter(|item| !crate::editor::playable(&snapshot.track, item.note.lane))
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
            flags: flags
                .get(&item.note.tick)
                .into_iter()
                .flatten()
                .copied()
                .filter(|flag| owns_flag(&snapshot.track, item.note.lane, *flag))
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
                let owned: Vec<_> = notes
                    .iter()
                    .filter(|note| phrase.start <= note.start && note.end <= phrase.end)
                    .collect();
                Some(VocalPhrase {
                    start: owned.first()?.start,
                    end: owned.last()?.end,
                    player: phrase.player,
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
            family: family(&selection.track).into(),
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
        Ok(format!("{PREFIX}{}", serde_json::to_string(self)?))
    }
    pub fn decode(text: &str) -> Result<Self> {
        ensure!(
            text.len() <= 4 * 1024 * 1024,
            "Clipboard selection is too large"
        );
        let value: Self = serde_json::from_str(
            text.strip_prefix(PREFIX)
                .context("Copy notes in YACE before pasting")?,
        )?;
        ensure!(
            value.version == 1 && value.resolution > 0,
            "Unsupported note clipboard"
        );
        match &value.payload {
            Payload::Instrument { family, notes } => {
                ensure!(
                    ["frets", "drums", "keys", "pro_keys"].contains(&family.as_str())
                        && !notes.is_empty()
                        && notes.len() <= MAX_NOTES,
                    "Invalid note clipboard"
                );
                ensure!(
                    notes.iter().all(|gem| gem.flags.len() <= 4),
                    "Invalid note flags"
                );
            }
            Payload::Vocals { chart } => {
                ensure!(
                    chart.notes.len() <= MAX_NOTES && chart.resolution == value.resolution,
                    "Invalid vocal clipboard"
                );
                chart.validate()?;
            }
        }
        Ok(value)
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
    let notes = project.content.tracks.get_mut(track).unwrap();
    notes.retain(|item| !selection.ids.contains(&item.id));
    for gem in chosen {
        let shared = notes.iter().any(|item| {
            item.note.tick == gem.note.tick && crate::editor::playable(track, item.note.lane)
        });
        notes.retain(|item| {
            !(item.note.tick == gem.note.tick
                && owns_flag(track, gem.note.lane, item.note.lane)
                && (track.ends_with("Drums") || !shared))
        });
    }
}
fn insert_gem(project: &mut Project, track: &str, gem: Gem, id: Option<String>) -> Result<String> {
    ensure!(
        crate::editor::playable(track, gem.note.lane),
        "This note does not fit the destination track"
    );
    ensure!(
        gem.flags
            .iter()
            .all(|flag| owns_flag(track, gem.note.lane, *flag)),
        "This flag does not fit the destination lane"
    );
    let existing = project
        .content
        .tracks
        .get(track)
        .context("Select an existing destination track")?;
    ensure!(
        !existing.iter().any(|item| item.note.tick == gem.note.tick
            && crate::editor::playable(track, item.note.lane)
            && (item.note.lane == gem.note.lane
                || (!track.ends_with("Drums") && ((item.note.lane == 7) != (gem.note.lane == 7))))),
        "Notes already occupy the destination. Nothing was changed."
    );
    let id = if let Some(id) = id {
        id
    } else {
        let id = format!("edit-note-{}", project.next_id);
        project.next_id = project.next_id.checked_add(1).context("Note ID overflow")?;
        id
    };
    let notes = project.content.tracks.get_mut(track).unwrap();
    for lane in gem.flags {
        if !notes
            .iter()
            .any(|item| item.note.tick == gem.note.tick && item.note.lane == lane)
        {
            let marker = format!("edit-note-{}", project.next_id);
            project.next_id = project.next_id.checked_add(1).context("Note ID overflow")?;
            notes.push(EditorNote {
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
                    let lanes = crate::timeline::lane_rows(&selection.track);
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
                        source == family(track) && track != "Vocals",
                        "Choose a compatible track before pasting"
                    );
                    let mut first = None;
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
                        let id = insert_gem(project, track, gem.clone(), None)?;
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
                last.player == phrase.player,
                "Vocal phrases belong to different players"
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
        for (i, note) in chart
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| phrase.start <= n.start && n.end <= phrase.end)
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
    let mut phrases = Vec::new();
    for phrase in &original.phrases {
        let owned: Vec<_> = notes
            .iter()
            .filter(|(i, _)| {
                phrase.start <= original.notes[*i].start && original.notes[*i].end <= phrase.end
            })
            .collect();
        if !owned.is_empty() {
            phrases.push(VocalPhrase {
                start: owned.iter().map(|(_, n)| n.start).min().unwrap(),
                end: owned.iter().map(|(_, n)| n.end).max().unwrap(),
                player: phrase.player,
            });
        }
    }
    notes.sort_by_key(|(_, note)| note.start);
    let tags: Vec<_> = notes.iter().map(|(i, _)| *i).collect();
    let expected = word_links(original)
        .into_iter()
        .filter(|(a, b, _)| tags.contains(a) && tags.contains(b))
        .collect();
    let chart = VocalChart {
        notes: notes.into_iter().map(|(_, note)| note).collect(),
        phrases: merge_phrases(phrases)?,
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
