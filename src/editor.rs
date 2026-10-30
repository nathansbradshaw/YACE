//! Saved chart editing shared by the desktop UI and command line.
use crate::{
    chart::{Chart, Note},
    models, pipeline,
    review_regions::ReviewMap,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Unlock explicitly before closing the file. A spawned decoder can briefly
/// inherit the handle and otherwise keep the next editor command locked.
struct CommandLock(std::fs::File);
impl Drop for CommandLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

#[cfg(test)]
mod command_lock_tests {
    use super::CommandLock;
    #[test]
    fn command_unlocks_even_when_an_inherited_description_is_still_open() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("editor.lock");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.try_lock().unwrap();
        let inherited = file.try_clone().unwrap();
        drop(CommandLock(file));
        let next = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        next.try_lock()
            .expect("completed command must release its lock");
        next.unlock().unwrap();
        drop(inherited);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Human,
    Ai,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub kind: ActorKind,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EditorNote {
    pub id: String,
    pub note: Note,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    Pending,
    Reviewed,
    Resolved,
    Dismissed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub region_id: String,
    pub track: String,
    pub status: DecisionStatus,
    pub actor: Actor,
    pub comment: String,
    pub chart_sha256: String,
    pub region_content_sha256: String,
    pub revision: u64,
    pub at_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct FlagScope {
    pub region_id: String,
    pub track: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Content {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub track_origins: BTreeMap<String, crate::parts::Origin>,
    pub tracks: BTreeMap<String, Vec<EditorNote>>,
    #[serde(default)]
    pub vocals: Option<crate::vocals::VocalChart>,
    #[serde(default)]
    pub vocal_review: Option<crate::vocal_review::VocalReviewMap>,
    #[serde(default)]
    pub difficulty_links: crate::difficulty_links::LinkState,
    pub decisions: Vec<Decision>,
    #[serde(default)]
    pub hidden_flags: BTreeSet<FlagScope>,
    #[serde(default)]
    pub stems: Option<crate::stem_review::StemManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_stems: Option<crate::stem_review::StemManifest>,
    #[serde(default)]
    pub stem_mix: crate::stem_review::StemMix,
}
impl Content {
    pub fn audition_stems(&self, track: &str) -> Option<&crate::stem_review::StemManifest> {
        if crate::keys::is_keys(track) {
            self.keyboard_stems.as_ref().or(self.stems.as_ref())
        } else {
            self.stems.as_ref()
        }
    }
    pub fn stem_ids(&self) -> BTreeSet<crate::stem_review::StemId> {
        self.stems
            .iter()
            .chain(self.keyboard_stems.iter())
            .flat_map(|m| m.ids())
            .collect()
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TimeRange {
    pub start: f64,
    pub end: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transport {
    pub position: f64,
    pub playing: bool,
    pub anchor_ms: u64,
    pub loop_range: Option<TimeRange>,
    #[serde(default = "normal_playback_rate")]
    pub playback_rate: f64,
}
fn normal_playback_rate() -> f64 {
    1.0
}
impl Transport {
    pub fn position_at(&self, clock_ms: u64, duration: f64) -> f64 {
        let elapsed = if self.playing {
            clock_ms.saturating_sub(self.anchor_ms) as f64 / 1000.0
        } else {
            0.0
        };
        let mut position = self.position + elapsed * self.playback_rate;
        if self.playing
            && let Some(range) = self.loop_range
            && position >= range.end
        {
            position = range.start + (position - range.start).rem_euclid(range.end - range.start);
        }
        position.clamp(0.0, duration)
    }
    pub fn current_position(&self, duration: f64) -> f64 {
        self.position_at(now_ms(), duration)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    CreateTrack {
        track: String,
    },
    ApplyGeneratedPart {
        job: PathBuf,
    },
    ImportCollaboration {
        bundle: crate::collaboration_exchange::Bundle,
    },
    Collaborate {
        operation: crate::collaboration::Operation,
    },
    Seek {
        seconds: f64,
    },
    Playback {
        playing: bool,
    },
    SetPlaybackRate {
        rate: f64,
    },
    SetLoop {
        range: Option<TimeRange>,
    },
    SelectTrack {
        track: String,
    },
    SelectRegion {
        region_id: String,
    },
    SelectNote {
        note_id: Option<String>,
    },
    AddNote {
        track: String,
        tick: u64,
        lane: u8,
        sustain: u64,
        cymbal: bool,
    },
    UpdateNote {
        note_id: String,
        tick: u64,
        lane: u8,
        sustain: u64,
        cymbal: bool,
    },
    DeleteNote {
        note_id: String,
    },
    EditNoteGroup {
        edit: crate::note_groups::Edit,
    },
    EnableDifficultyLink {
        track: String,
        enabled: bool,
    },
    ClearDifficultyOverrides {
        track: String,
        note_id: Option<String>,
    },
    Decide {
        region_id: String,
        track: String,
        status: DecisionStatus,
        comment: String,
    },
    SetFlagVisibility {
        region_id: String,
        track: String,
        hidden: bool,
    },
    AttachStems {
        manifest: PathBuf,
    },
    AttachKeyboardStems {
        manifest: PathBuf,
    },
    SetStemMix {
        mix: crate::stem_review::StemMix,
    },
    /// Select a validated saved drum component for the shared audio transport.
    SetKitAudition {
        part: Option<String>,
    },
    SetVocals {
        document: crate::vocals::VocalChart,
    },
    DeleteVocalNote {
        index: usize,
    },
    SetVocalGeneration {
        document: crate::vocals::VocalChart,
        review: crate::vocal_review::VocalReviewMap,
    },
    SetVocalReview {
        review: crate::vocal_review::VocalReviewMap,
    },
    Undo,
    Redo,
    Export {
        output: PathBuf,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub expected_revision: u64,
    pub actor: Actor,
    pub action: Action,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditEntry {
    pub revision: u64,
    pub actor: Actor,
    pub action: Action,
    pub at_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub version: u32,
    pub revision: u64,
    pub title: String,
    pub original_folder: PathBuf,
    pub source_chart_sha256: String,
    pub audio_filename: String,
    pub audio_sha256: String,
    pub duration: f64,
    pub review: ReviewMap,
    pub content: Content,
    pub selected_track: String,
    pub selected_region: Option<String>,
    pub selected_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit_audition: Option<crate::part_analysis::KitAudition>,
    pub transport: Transport,
    pub next_id: u64,
    pub undo: Vec<String>,
    pub redo: Vec<String>,
    pub audit: Vec<AuditEntry>,
}

pub fn playable(track: &str, lane: u8) -> bool {
    if crate::keys::is_pro(track) {
        (crate::keys::MIN_PITCH..=crate::keys::MAX_PITCH).contains(&lane)
    } else {
        lane <= 4 || (!track.ends_with("Drums") && !crate::keys::is_keys(track) && lane == 7)
    }
}
fn editable_track(name: &str) -> bool {
    ["Easy", "Medium", "Hard", "Expert"].iter().any(|tier| {
        ["Single", "DoubleBass", "Drums", "Keyboard", "ProKeys"]
            .iter()
            .any(|part| name == format!("{tier}{part}"))
    })
}

impl Project {
    pub fn timing(&self, store: &Store) -> Result<crate::timing::TempoMap> {
        Ok(store.source_chart(self)?.timing)
    }
    pub fn chart_text(&self, store: &Store) -> Result<String> {
        let chart = store.source_chart(self)?;
        let mut text = chart.raw.clone();
        // Replace only N lines in changed tracks. Non-note events and every
        // untouched section remain byte-identical, including original newlines.
        for section in chart.sections.iter().rev() {
            let Some(notes) = self.content.tracks.get(&section.name) else {
                continue;
            };
            let mut current: Vec<_> = notes.iter().map(|n| n.note.clone()).collect();
            current.sort();
            let mut original = chart.notes(&section.name)?;
            original.sort();
            if current == original {
                continue;
            }
            let newline = if chart.raw.contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            let mut body = String::new();
            let mut inserted = false;
            for line in chart.raw[section.body_start..section.body_end].split_inclusive('\n') {
                let is_note = line
                    .split_once('=')
                    .is_some_and(|(_, event)| event.split_whitespace().next() == Some("N"));
                if is_note {
                    if !inserted {
                        body.push_str(&crate::chart::note_lines(&current, newline));
                        inserted = true;
                    }
                } else {
                    body.push_str(line);
                }
            }
            if !inserted {
                body.push_str(&crate::chart::note_lines(&current, newline));
            }
            text.replace_range(section.body_start..section.body_end, &body);
        }
        // Append added parts without touching the existing chart bytes.
        // Skip empty reductions so they do not create unused sections.
        let newline = if chart.raw.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        for (track, notes) in &self.content.tracks {
            if chart.sections.iter().any(|section| section.name == *track) {
                continue;
            }
            ensure!(
                crate::editor_validation::is_link_created_track(self, &chart, track)
                    || self.content.track_origins.contains_key(track),
                "Unrelated new editor track"
            );
            if notes.is_empty() && !self.content.track_origins.contains_key(track) {
                continue;
            }
            if !text.ends_with('\n') {
                text.push_str(newline);
            }
            text.push_str(newline);
            text.push_str(&format!("[{track}]{newline}{{{newline}"));
            let notes: Vec<_> = notes.iter().map(|item| item.note.clone()).collect();
            text.push_str(&crate::chart::note_lines(&notes, newline));
            text.push_str(&format!("}}{newline}"));
        }
        Ok(text)
    }
    pub fn chart_hash(&self, store: &Store) -> Result<String> {
        Ok(hash(self.chart_text(store)?.as_bytes()))
    }
    pub fn region_fingerprint(
        &self,
        region: &str,
        track: &str,
        timing: &crate::timing::TempoMap,
    ) -> Result<String> {
        if track == crate::vocal_review::TRACK {
            return self
                .content
                .vocal_review
                .as_ref()
                .context("No vocal review checks attached")?
                .fingerprint(self, region, timing);
        }
        let region = self
            .review
            .regions
            .iter()
            .find(|r| r.id == region)
            .context("Unknown review region")?;
        let mut notes = BTreeMap::new();
        ensure!(
            region.tracks.contains(track),
            "This track is not part of the review region"
        );
        {
            let mut scoped: Vec<_> = self
                .content
                .tracks
                .get(track)
                .into_iter()
                .flatten()
                .filter(|n| {
                    let start = timing.seconds(n.note.tick);
                    let end = timing.seconds(n.note.tick.saturating_add(n.note.sustain));
                    end >= region.start - 0.1 && start <= region.end + 0.1
                })
                .map(|n| n.note.clone())
                .collect();
            scoped.sort();
            notes.insert(track, scoped);
        }
        Ok(hash(&serde_json::to_vec(&(
            1,
            &region.id,
            track,
            &self.audio_sha256,
            timing,
            notes,
        ))?))
    }
    pub fn decision<'a>(
        &'a self,
        region: &str,
        track: &str,
        timing: &crate::timing::TempoMap,
    ) -> Option<&'a Decision> {
        // Skip note hashing for unreviewed sections. This runs on every repaint.
        if !self
            .content
            .decisions
            .iter()
            .any(|d| d.region_id == region && d.track == track)
        {
            return None;
        }
        let fingerprint = self.region_fingerprint(region, track, timing).ok()?;
        self.content.decisions.iter().rev().find(|d| {
            d.region_id == region && d.track == track && d.region_content_sha256 == fingerprint
        })
    }
    pub fn is_flag_hidden(&self, region_id: &str, track: &str) -> bool {
        (if track == crate::vocal_review::TRACK {
            self.content
                .vocal_review
                .as_ref()
                .is_some_and(|map| map.region(region_id).is_some())
        } else {
            self.review
                .regions
                .iter()
                .any(|region| region.id == region_id && region.tracks.contains(track))
        }) && self.content.hidden_flags.contains(&FlagScope {
            region_id: region_id.into(),
            track: track.into(),
        })
    }
    pub fn find_note(&self, id: &str) -> Option<(&str, &EditorNote)> {
        self.content.tracks.iter().find_map(|(track, notes)| {
            notes
                .iter()
                .find(|n| n.id == id)
                .map(|n| (track.as_str(), n))
        })
    }
    fn current_decisions(&self, timing: &crate::timing::TempoMap) -> Vec<serde_json::Value> {
        let mut decisions:Vec<_>=self.review.regions.iter().flat_map(|r|r.tracks.iter().map(move|track|
            serde_json::json!({"region_id":r.id,"track":track,"region_content_sha256":self.region_fingerprint(&r.id,track,timing).ok(),"decision":self.decision(&r.id,track,timing),"hidden":self.is_flag_hidden(&r.id,track)})
        )).collect();
        if let Some(map) = &self.content.vocal_review {
            for region in &map.regions {
                let track = crate::vocal_review::TRACK;
                decisions.push(serde_json::json!({"region_id":region.id,"track":track,"region_content_sha256":self.region_fingerprint(&region.id,track,timing).ok(),"decision":self.decision(&region.id,track,timing),"hidden":self.is_flag_hidden(&region.id,track)}));
            }
        }
        decisions
    }
    pub fn is_cymbal(&self, track: &str, note: &Note) -> bool {
        track.ends_with("Drums")
            && self.content.tracks[track]
                .iter()
                .any(|n| n.note.tick == note.tick && n.note.lane == note.lane + 64)
    }
    fn next_note(&mut self, note: Note) -> EditorNote {
        let id = format!("edit-note-{}", self.next_id);
        self.next_id += 1;
        EditorNote { id, note }
    }
    fn validate_notes(&self, store: &Store) -> Result<()> {
        let timing = self.timing(store)?;
        let mut ids = BTreeSet::new();
        for (track, notes) in &self.content.tracks {
            ensure!(editable_track(track), "Unsupported editor track");
            let mut positions = BTreeSet::new();
            for n in notes {
                ensure!(ids.insert(&n.id), "Duplicate note ID");
                ensure!(
                    positions.insert((n.note.tick, n.note.lane)),
                    "Duplicate lane at the same tick in {track}"
                );
                let end = n
                    .note
                    .tick
                    .checked_add(n.note.sustain)
                    .context("Note tick overflow")?;
                if playable(track, n.note.lane) {
                    ensure!(
                        timing.seconds(end) <= self.duration + 0.1,
                        "Note extends beyond recording"
                    );
                }
            }
        }
        Ok(())
    }
    fn edit_note(
        &mut self,
        store: &Store,
        track: &str,
        id: Option<&str>,
        note: Option<Note>,
        cymbal: bool,
    ) -> Result<()> {
        ensure!(self.content.tracks.contains_key(track), "Unknown track");
        let old = id
            .map(|id| {
                self.find_note(id)
                    .map(|(_, n)| n.note.clone())
                    .context("Unknown note ID")
            })
            .transpose()?;
        if let Some(old) = &old {
            ensure!(
                playable(track, old.lane),
                "Edit the playable note, not its control marker"
            );
        }
        if let Some(note) = &note {
            ensure!(playable(track, note.lane), "Unsupported lane for {track}");
            ensure!(
                !cymbal || (track.ends_with("Drums") && (2..=4).contains(&note.lane)),
                "Cymbals require drum lane 2, 3 or 4"
            );
            ensure!(
                !track.ends_with("Drums") || note.sustain == 0,
                "Drums cannot sustain"
            );
        }
        let mut carry = Vec::new();
        let mut drum_articulations = Vec::new();
        if let Some(old) = &old {
            let notes = self.content.tracks.get_mut(track).unwrap();
            notes.retain(|n| Some(n.id.as_str()) != id);
            if track.ends_with("Drums") {
                notes.retain(|n| !(n.note.tick == old.tick && n.note.lane == old.lane + 64));
                if (1..=4).contains(&old.lane) {
                    for marker in notes.iter().filter(|n| {
                        n.note.tick == old.tick
                            && [old.lane + 33, old.lane + 39].contains(&n.note.lane)
                    }) {
                        drum_articulations.push((marker.clone(), marker.note.lane - old.lane));
                    }
                    notes.retain(|n| {
                        !(n.note.tick == old.tick
                            && [old.lane + 33, old.lane + 39].contains(&n.note.lane))
                    });
                }
            } else if !notes
                .iter()
                .any(|n| n.note.tick == old.tick && playable(track, n.note.lane))
            {
                for n in notes
                    .iter()
                    .filter(|n| n.note.tick == old.tick && (5..=6).contains(&n.note.lane))
                {
                    carry.push(n.clone());
                }
                notes.retain(|n| !(n.note.tick == old.tick && (5..=6).contains(&n.note.lane)));
            }
        }
        if let Some(note) = note {
            let mut added = if let Some(id) = id {
                EditorNote {
                    id: id.into(),
                    note: note.clone(),
                }
            } else {
                self.next_note(note.clone())
            };
            for mut marker in carry {
                marker.note.tick = note.tick;
                let notes = self.content.tracks.get_mut(track).unwrap();
                if !notes
                    .iter()
                    .any(|n| n.note.tick == note.tick && n.note.lane == marker.note.lane)
                {
                    notes.push(marker);
                }
            }
            if track.ends_with("Drums") && (1..=4).contains(&note.lane) {
                for (mut marker, offset) in drum_articulations {
                    marker.note.tick = note.tick;
                    marker.note.lane = note.lane + offset;
                    let notes = self.content.tracks.get_mut(track).unwrap();
                    ensure!(
                        !notes
                            .iter()
                            .any(|n| n.note.tick == marker.note.tick
                                && n.note.lane == marker.note.lane),
                        "An articulation marker already exists at the destination"
                    );
                    notes.push(marker);
                }
            }
            // Five-fret open notes cannot coincide with colored gems.
            if !track.ends_with("Drums") {
                ensure!(
                    !self.content.tracks[track]
                        .iter()
                        .any(|n| n.note.tick == note.tick
                            && playable(track, n.note.lane)
                            && ((note.lane == 7) != (n.note.lane == 7))),
                    "An open note cannot share a chord with colored gems"
                );
            }
            if cymbal {
                let marker = self.next_note(Note {
                    tick: note.tick,
                    lane: note.lane + 64,
                    sustain: 0,
                });
                self.content.tracks.get_mut(track).unwrap().push(marker);
            }
            added.note = note;
            self.selected_note = Some(added.id.clone());
            self.content.tracks.get_mut(track).unwrap().push(added);
        } else {
            self.selected_note = None;
        }
        self.validate_notes(store)
    }
}

#[derive(Clone)]
pub struct Store {
    pub root: PathBuf,
}
#[derive(Debug, Serialize)]
pub struct Applied {
    pub revision: u64,
    pub chart_sha256: String,
    pub selected_note: Option<String>,
    pub exported_to: Option<PathBuf>,
}

impl Store {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
        }
    }
    fn source_chart(&self, project: &Project) -> Result<Chart> {
        let bytes = fs::read(self.root.join("source/notes.chart"))?;
        ensure!(
            hash(&bytes) == project.source_chart_sha256,
            "Source snapshot changed; refusing to edit against changed inputs"
        );
        Chart::parse(String::from_utf8(bytes)?)
    }
    pub fn audio_path(&self, project: &Project) -> Result<PathBuf> {
        ensure!(
            Path::new(&project.audio_filename).components().count() == 1,
            "Invalid audio filename"
        );
        Ok(self.root.join("source").join(&project.audio_filename))
    }
    pub fn create(song: &Path, markers: Option<&Path>, output: &Path) -> Result<Self> {
        pipeline::destination(output, Some(song))?;
        pipeline::validate_existing(song)?;
        let editable_master = pipeline::editable_chart_file(song)?;
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let stage = tempfile::tempdir_in(parent)?;
        fs::create_dir(stage.path().join("source"))?;
        fs::create_dir(stage.path().join("checkpoints"))?;
        pipeline::copy_folder(song, &stage.path().join("source"))?;
        if editable_master != song.join("notes.chart") {
            fs::rename(
                stage.path().join("source/notes.mid"),
                stage
                    .path()
                    .join("source/chart-engine-working/game-notes.mid"),
            )?;
            fs::copy(&editable_master, stage.path().join("source/notes.chart"))?;
        }
        let chart = Chart::parse(fs::read_to_string(stage.path().join("source/notes.chart"))?)?;
        let source_hash = hash(chart.raw.as_bytes());
        let audio = pipeline::audio_file(&stage.path().join("source"))?;
        let duration = pipeline::metadata(&audio)?.duration;
        let audio_hash = models::hash_file(&audio)?;
        let embedded_markers = song.join("chart-engine-review.json");
        let markers = markers.or_else(|| {
            embedded_markers
                .is_file()
                .then_some(embedded_markers.as_path())
        });
        let mut review = if let Some(path) = markers {
            let review: ReviewMap = serde_json::from_slice(&fs::read(path)?)?;
            ensure!(
                review.version == 1 && review.input_sha256.get("chart") == Some(&source_hash),
                "Review markers do not match these notes"
            );
            ensure!(
                review.input_sha256.get("original_audio") == Some(&audio_hash)
                    && (review.duration_seconds - duration).abs() <= 0.1,
                "Review markers do not match this recording"
            );
            review
        } else {
            crate::review::generation_map(
                &chart,
                &Default::default(),
                duration,
                BTreeMap::from([
                    ("chart".into(), source_hash.clone()),
                    ("original_audio".into(), audio_hash.clone()),
                ]),
                BTreeSet::new(),
            )?
        };
        // Let the original recording open even if attached stems are missing.
        // The audio worker checks stem files before offering playback.
        let stems = match (|| -> Result<Option<crate::stem_review::StemManifest>> {
            let isolation = song.join("chart-engine-isolation.json");
            let transcription = song.join("chart-engine-transcription.json");
            let (mut manifest, base) = if isolation.is_file() {
                (
                    serde_json::from_slice::<crate::stem_review::StemManifest>(&fs::read(
                        &isolation,
                    )?)?,
                    isolation,
                )
            } else if transcription.is_file() {
                let value: serde_json::Value = serde_json::from_slice(&fs::read(&transcription)?)?;
                let Some(value) = value.get("isolation").filter(|value| !value.is_null()) else {
                    return Ok(None);
                };
                (
                    serde_json::from_value::<crate::stem_review::StemManifest>(value.clone())?,
                    transcription,
                )
            } else {
                return Ok(None);
            };
            let base = base
                .parent()
                .context("Isolation sidecar has no parent")?
                .canonicalize()?;
            for asset in &mut manifest.stems {
                let path = if asset.path.is_relative() {
                    base.join(&asset.path)
                } else {
                    asset.path.clone()
                };
                asset.path = path.canonicalize().unwrap_or(path);
            }
            manifest.validate_binding(&audio_hash, duration)?;
            Ok(Some(manifest))
        })() {
            Ok(stems) => stems,
            Err(error) => {
                review.coverage_limits.push(format!("Stem attachment skipped: {error:#}. The original recording remains available; converted or different recordings require a newly source-bound manifest."));
                None
            }
        };
        let keyboard_stems = match (|| -> Result<Option<crate::stem_review::StemManifest>> {
            let sidecar = song.join("chart-engine-keyboard-isolation.json");
            let transcription = song.join("chart-engine-transcription.json");
            let (value, base) = if sidecar.is_file() {
                (serde_json::from_slice(&fs::read(&sidecar)?)?, sidecar)
            } else if transcription.is_file() {
                let saved: serde_json::Value = serde_json::from_slice(&fs::read(&transcription)?)?;
                let Some(value) = saved.get("keyboard_analysis").filter(|v| !v.is_null()) else {
                    return Ok(None);
                };
                (value.clone(), transcription)
            } else {
                return Ok(None);
            };
            let mut manifest: crate::stem_review::StemManifest = serde_json::from_value(value)?;
            let base = base
                .parent()
                .context("Keyboard isolation sidecar has no parent")?
                .canonicalize()?;
            for asset in &mut manifest.stems {
                let path = if asset.path.is_relative() {
                    base.join(&asset.path)
                } else {
                    asset.path.clone()
                };
                asset.path = path.canonicalize().unwrap_or(path);
            }
            manifest.validate_binding(&audio_hash, duration)?;
            Ok(Some(manifest))
        })() {
            Ok(manifest) => manifest,
            Err(error) => {
                review.coverage_limits.push(format!("Keyboard stem attachment skipped: {error:#}. The original recording remains available."));
                None
            }
        };
        let tracks: BTreeMap<_, _> = chart
            .sections
            .iter()
            .filter(|s| editable_track(&s.name))
            .map(|s| {
                let notes = chart
                    .notes(&s.name)?
                    .into_iter()
                    .enumerate()
                    .map(|(i, n)| EditorNote {
                        id: format!(
                            "note-{}",
                            hash(format!("{source_hash}:{}:{i}:{n:?}", s.name).as_bytes())
                        ),
                        note: n,
                    })
                    .collect();
                Ok((s.name.clone(), notes))
            })
            .collect::<Result<_>>()?;
        ensure!(!tracks.is_empty(), "No supported playable tracks");
        let selected_track = [
            "ExpertDrums",
            "ExpertSingle",
            "ExpertDoubleBass",
            "ExpertKeyboard",
            "ExpertProKeys",
        ]
        .into_iter()
        .find(|t| tracks.contains_key(*t))
        .unwrap_or(tracks.keys().next().unwrap())
        .into();
        let mut project = Project {
            version: 1,
            revision: 0,
            title: song
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            original_folder: song.canonicalize()?,
            source_chart_sha256: source_hash,
            audio_filename: audio.file_name().unwrap().to_string_lossy().into(),
            audio_sha256: audio_hash,
            duration,
            review,
            content: Content {
                track_origins: Default::default(),
                tracks,
                vocals: {
                    let path = stage.path().join("source/chart-engine-vocals.json");
                    if path.exists() {
                        Some(serde_json::from_slice(&fs::read(path)?)?)
                    } else {
                        None
                    }
                },
                difficulty_links: Default::default(),
                vocal_review: None,
                decisions: Vec::new(),
                hidden_flags: BTreeSet::new(),
                stems,
                keyboard_stems,
                stem_mix: Default::default(),
            },
            selected_track,
            selected_region: None,
            selected_note: None,
            kit_audition: None,
            transport: Transport {
                position: 0.0,
                playing: false,
                anchor_ms: now_ms(),
                loop_range: None,
                playback_rate: 1.0,
            },
            next_id: 1,
            undo: Vec::new(),
            redo: Vec::new(),
            audit: Vec::new(),
        };
        crate::difficulty_links::enable_generated(
            &mut project.content,
            project.review.generated_tracks.as_ref(),
            &chart.timing,
            duration,
        )?;
        let staged = Self::new(stage.path());
        project.validate_notes(&staged)?;
        crate::editor_validation::validate(&project, &chart)?;
        staged.save(&project)?;
        pipeline::publish(stage.path(), output)?;
        Ok(Self::new(&output.canonicalize()?))
    }
    pub fn load(&self) -> Result<Project> {
        let doc: Project = serde_json::from_slice(&fs::read(self.root.join("project.json"))?)?;
        ensure!(doc.version == 1, "Unsupported editor project version");
        for manifest in doc
            .content
            .stems
            .iter()
            .chain(doc.content.keyboard_stems.iter())
        {
            manifest.validate_binding(&doc.audio_sha256, doc.duration)?;
        }
        doc.content.stem_mix.prepare(doc.content.stem_ids())?;
        let chart = self.source_chart(&doc)?;
        crate::editor_validation::validate(&doc, &chart)?;
        crate::difficulty_links::validate(&doc.content, &chart.timing, doc.duration)?;
        Ok(doc)
    }
    /// Immutable source routing is separate from undoable note content. Legacy
    /// projects infer it from the exact generation evidence copied into source/.
    pub fn guitar_source(&self, doc: &Project) -> Result<crate::mapping::GuitarSource> {
        let binding = self.root.join("source/chart-engine-melody-source.json");
        if binding.is_file() {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Binding {
                version: u32,
                audio_sha256: String,
                guitar_source: crate::mapping::GuitarSource,
            }
            let binding: Binding = serde_json::from_slice(&fs::read(binding)?)?;
            ensure!(
                binding.version == 1 && binding.audio_sha256 == doc.audio_sha256,
                "Melody source belongs to another recording"
            );
            return Ok(binding.guitar_source);
        }
        let path = self.root.join("source/chart-engine-transcription.json");
        if !path.is_file() {
            return Ok(Default::default());
        }
        let bytes = fs::read(path)?;
        #[derive(Deserialize)]
        struct SourceChoice {
            #[serde(default)]
            guitar_source: crate::mapping::GuitarSource,
        }
        let choice: SourceChoice = serde_json::from_slice(&bytes)?;
        if choice.guitar_source != crate::mapping::GuitarSource::Guitar {
            ensure!(
                doc.review.input_sha256.get("transcription") == Some(&hash(&bytes))
                    && doc.review.input_sha256.get("original_audio") == Some(&doc.audio_sha256),
                "Alternative melody source does not match the saved generation/recording witness"
            );
            serde_json::from_slice::<crate::mapping::Transcription>(&bytes)?.validate()?;
        }
        Ok(choice.guitar_source)
    }
    fn save(&self, project: &Project) -> Result<()> {
        let mut tmp = tempfile::NamedTempFile::new_in(&self.root)?;
        tmp.write_all(&serde_json::to_vec_pretty(project)?)?;
        tmp.as_file().sync_all()?;
        tmp.persist(self.root.join("project.json"))?;
        Ok(())
    }
    fn checkpoint(&self, content: &Content) -> Result<String> {
        let bytes = serde_json::to_vec(content)?;
        let id = hash(&bytes);
        let path = self.root.join("checkpoints").join(format!("{id}.json"));
        if !path.exists() {
            let mut file = tempfile::NamedTempFile::new_in(self.root.join("checkpoints"))?;
            file.write_all(&bytes)?;
            file.as_file().sync_all()?;
            file.persist_noclobber(&path)?;
        }
        ensure!(models::hash_file(&path)? == id, "Checkpoint changed");
        Ok(id)
    }
    fn restore(&self, id: &str) -> Result<Content> {
        ensure!(
            id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid checkpoint ID"
        );
        let bytes = fs::read(self.root.join("checkpoints").join(format!("{id}.json")))?;
        ensure!(hash(&bytes) == id, "Checkpoint checksum mismatch");
        Ok(serde_json::from_slice(&bytes)?)
    }
    pub fn apply(&self, request: Request) -> Result<Applied> {
        self.apply_guarded(request, |_| Ok(()))
    }
    /// Run a caller's source/snapshot preconditions under the same command lock
    /// as the existing revision check and action. Persistence stays shared.
    pub fn apply_guarded(
        &self,
        request: Request,
        guard: impl FnOnce(&Project) -> Result<()>,
    ) -> Result<Applied> {
        // Hash kit files outside the editor lock so editing stays responsive.
        // The revision check below rejects a result if the project changed.
        let kit_selection = if let Action::SetKitAudition { part: Some(part) } = &request.action {
            let snapshot = self.load()?;
            ensure!(
                snapshot.revision == request.expected_revision,
                "Revision conflict; reload before selecting kit audio"
            );
            Some(crate::part_analysis::select_kit_audition(
                self, &snapshot, part,
            )?)
        } else {
            None
        };
        let mut generated_part = if let Action::ApplyGeneratedPart { job } = &request.action {
            let snapshot = self.load()?;
            ensure!(
                snapshot.revision == request.expected_revision,
                "Revision conflict; reload before applying a part"
            );
            Some(crate::parts::Prepared::load(self, &snapshot, job)?)
        } else {
            None
        };
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("editor.lock"))?;
        lock.try_lock()
            .context("Another editor command is saving; reload and retry")?;
        let _command_lock = CommandLock(lock);
        let mut doc = self.load()?;
        ensure!(
            doc.revision == request.expected_revision,
            "Revision conflict: expected {}, current {}; reload before editing",
            request.expected_revision,
            doc.revision
        );
        ensure!(
            !request.actor.name.trim().is_empty() && request.actor.name.len() <= 200,
            "Reviewer name is required"
        );
        guard(&doc)?;
        if let Action::ImportCollaboration { bundle } = &request.action
            && !crate::collaboration_exchange::validate_import(&doc, bundle)?
        {
            return Ok(Applied {
                revision: doc.revision,
                chart_sha256: doc.chart_hash(self)?,
                selected_note: doc.selected_note,
                exported_to: None,
            });
        }
        if let Action::Export { output } = &request.action {
            self.export(&doc, output, &request.actor)?;
            return Ok(Applied {
                revision: doc.revision,
                chart_sha256: doc.chart_hash(self)?,
                selected_note: doc.selected_note,
                exported_to: Some(output.clone()),
            });
        }
        let before = doc.content.clone();
        let content_action = matches!(
            request.action,
            Action::Collaborate {
                operation: crate::collaboration::Operation::Merge { .. }
            } | Action::CreateTrack { .. }
                | Action::ApplyGeneratedPart { .. }
                | Action::AddNote { .. }
                | Action::UpdateNote { .. }
                | Action::DeleteNote { .. }
                | Action::EditNoteGroup { .. }
                | Action::EnableDifficultyLink { .. }
                | Action::ClearDifficultyOverrides { .. }
                | Action::Decide { .. }
                | Action::SetFlagVisibility { .. }
                | Action::AttachStems { .. }
                | Action::AttachKeyboardStems { .. }
                | Action::SetStemMix { .. }
                | Action::SetVocals { .. }
                | Action::DeleteVocalNote { .. }
                | Action::SetVocalGeneration { .. }
                | Action::SetVocalReview { .. }
        );
        doc.transport.position = doc.transport.current_position(doc.duration);
        doc.transport.anchor_ms = now_ms();
        match &request.action {
            Action::CreateTrack { track } => {
                crate::parts::create(&mut doc, track)?;
            }
            Action::ApplyGeneratedPart { .. } => generated_part
                .take()
                .context("Missing generated draft")?
                .apply(self, &mut doc)?,
            Action::ImportCollaboration { .. } => (),
            Action::Collaborate { operation } => {
                crate::collaboration::apply(&mut doc, operation, &request.actor)?
            }
            Action::Seek { seconds } => {
                ensure!(
                    seconds.is_finite() && (0.0..=doc.duration).contains(seconds),
                    "Seek is outside recording"
                );
                doc.transport.position = *seconds;
            }
            Action::Playback { playing } => doc.transport.playing = *playing,
            Action::SetPlaybackRate { rate } => {
                ensure!(
                    rate.is_finite() && (0.5..=1.5).contains(rate),
                    "Practice rate must be between 0.5 and 1.5"
                );
                doc.transport.playback_rate = *rate;
                doc.transport.playing = false;
            }
            Action::SetLoop { range } => {
                if let Some(r) = range {
                    ensure!(
                        r.start.is_finite()
                            && r.end.is_finite()
                            && r.start >= 0.0
                            && r.end > r.start
                            && r.end <= doc.duration,
                        "Invalid loop range"
                    );
                }
                doc.transport.loop_range = *range;
                if let Some(r) = range
                    && !(r.start..r.end).contains(&doc.transport.position)
                {
                    doc.transport.position = r.start;
                }
            }
            Action::SelectTrack { track } => {
                ensure!(
                    track == "Vocals" || doc.content.tracks.contains_key(track),
                    "Unknown track"
                );
                doc.selected_track = track.clone();
                doc.selected_note = None;
                if doc.selected_region.as_ref().is_some_and(|id| {
                    !doc.review
                        .regions
                        .iter()
                        .any(|r| &r.id == id && r.tracks.contains(track))
                }) {
                    doc.selected_region = None;
                }
            }
            Action::SelectRegion { region_id } => {
                if let Some(region) = doc
                    .content
                    .vocal_review
                    .as_ref()
                    .and_then(|map| map.region(region_id))
                {
                    doc.transport.position = region.start;
                    doc.transport.loop_range = Some(TimeRange {
                        start: region.start,
                        end: region.end,
                    });
                } else {
                    let r = doc
                        .review
                        .regions
                        .iter()
                        .find(|r| r.id == *region_id)
                        .context("Unknown review region")?;
                    doc.transport.position = r.start;
                    doc.transport.loop_range = Some(TimeRange {
                        start: r.start,
                        end: r.end,
                    });
                    if !r.tracks.contains(&doc.selected_track)
                        && let Some(track) = r
                            .tracks
                            .iter()
                            .find(|t| {
                                t.starts_with("Expert") && doc.content.tracks.contains_key(*t)
                            })
                            .or_else(|| {
                                r.tracks
                                    .iter()
                                    .find(|t| doc.content.tracks.contains_key(*t))
                            })
                    {
                        doc.selected_track = track.clone();
                    }
                }
                doc.selected_note = None;
                doc.selected_region = Some(region_id.clone());
            }
            Action::SelectNote { note_id } => {
                if let Some(id) = note_id {
                    let (track, n) = doc.find_note(id).context("Unknown note ID")?;
                    ensure!(
                        playable(track, n.note.lane),
                        "Select the playable note, not its control marker"
                    );
                    doc.selected_track = track.into();
                }
                doc.selected_note = note_id.clone();
            }
            Action::AddNote {
                track,
                tick,
                lane,
                sustain,
                cymbal,
            } => doc.edit_note(
                self,
                track,
                None,
                Some(Note {
                    tick: *tick,
                    lane: *lane,
                    sustain: *sustain,
                }),
                *cymbal,
            )?,
            Action::UpdateNote {
                note_id,
                tick,
                lane,
                sustain,
                cymbal,
            } => {
                let track = doc
                    .find_note(note_id)
                    .context("Unknown note ID")?
                    .0
                    .to_owned();
                doc.edit_note(
                    self,
                    &track,
                    Some(note_id),
                    Some(Note {
                        tick: *tick,
                        lane: *lane,
                        sustain: *sustain,
                    }),
                    *cymbal,
                )?;
            }
            Action::DeleteNote { note_id } => {
                let track = doc
                    .find_note(note_id)
                    .context("Unknown note ID")?
                    .0
                    .to_owned();
                doc.edit_note(self, &track, Some(note_id), None, false)?;
            }
            Action::EditNoteGroup { edit } => {
                let timing = doc.timing(self)?;
                crate::note_groups::apply(&mut doc, &timing, edit)?;
            }
            Action::EnableDifficultyLink { track, enabled } => {
                let timing = doc.timing(self)?;
                crate::difficulty_links::set_enabled(
                    &mut doc.content,
                    track,
                    *enabled,
                    &timing,
                    doc.duration,
                )?;
            }
            Action::ClearDifficultyOverrides { track, note_id } => {
                let timing = doc.timing(self)?;
                crate::difficulty_links::clear_overrides(
                    &mut doc.content,
                    track,
                    note_id.as_deref(),
                    &timing,
                    doc.duration,
                )?;
            }
            Action::Decide {
                region_id,
                track,
                status,
                comment,
            } => {
                ensure!(
                    doc.review.regions.iter().any(|r| r.id == *region_id)
                        || (track == crate::vocal_review::TRACK
                            && doc
                                .content
                                .vocal_review
                                .as_ref()
                                .is_some_and(|map| map.region(region_id).is_some())),
                    "Unknown review region"
                );
                ensure!(comment.len() <= 10000, "Review comment too long");
                ensure!(
                    *status == DecisionStatus::Pending || !comment.trim().is_empty(),
                    "Record review evidence or a reason for this decision"
                );
                doc.content.decisions.push(Decision {
                    region_id: region_id.clone(),
                    track: track.clone(),
                    status: *status,
                    actor: request.actor.clone(),
                    comment: comment.clone(),
                    chart_sha256: doc.chart_hash(self)?,
                    region_content_sha256: doc.region_fingerprint(
                        region_id,
                        track,
                        &doc.timing(self)?,
                    )?,
                    revision: doc.revision + 1,
                    at_ms: now_ms(),
                });
            }
            Action::SetFlagVisibility {
                region_id,
                track,
                hidden,
            } => {
                if track == crate::vocal_review::TRACK {
                    ensure!(
                        doc.content
                            .vocal_review
                            .as_ref()
                            .is_some_and(|map| map.region(region_id).is_some()),
                        "Unknown vocal review region"
                    );
                } else {
                    let region = doc
                        .review
                        .regions
                        .iter()
                        .find(|region| region.id == *region_id)
                        .context("Unknown review region")?;
                    ensure!(
                        region.tracks.contains(track) && doc.content.tracks.contains_key(track),
                        "This track is not part of the review region"
                    );
                }
                let scope = FlagScope {
                    region_id: region_id.clone(),
                    track: track.clone(),
                };
                if *hidden {
                    doc.content.hidden_flags.insert(scope);
                } else {
                    doc.content.hidden_flags.remove(&scope);
                }
            }
            Action::AttachStems { manifest } | Action::AttachKeyboardStems { manifest } => {
                let manifest = crate::stem_review::StemManifest::read(manifest)?;
                let manifest = Some(manifest.validate_source(
                    &self.audio_path(&doc)?,
                    &doc.audio_sha256,
                    doc.duration,
                )?);
                if matches!(&request.action, Action::AttachKeyboardStems { .. }) {
                    doc.content.keyboard_stems = manifest;
                } else {
                    doc.content.stems = manifest;
                }
                doc.content.stem_mix = Default::default();
                doc.kit_audition = None;
                doc.transport.playing = false;
            }
            Action::SetStemMix { mix } => {
                mix.prepare(doc.content.stem_ids())?;
                doc.content.stem_mix = mix.clone();
                doc.kit_audition = None;
            }
            Action::SetKitAudition { part } => {
                doc.kit_audition = if part.is_some() {
                    Some(kit_selection.context("Kit audition preflight missing")?)
                } else {
                    None
                };
                doc.transport.playing = false;
            }
            Action::SetVocals { document } => {
                document.validate()?;
                doc.content.vocals = Some(document.clone());
            }
            Action::DeleteVocalNote { index } => {
                let chart = doc.content.vocals.as_mut().context("No vocal chart")?;
                ensure!(*index < chart.notes.len(), "Unknown vocal note");
                chart.notes.remove(*index);
                chart.phrases.retain(|phrase| {
                    chart
                        .notes
                        .iter()
                        .any(|note| phrase.start <= note.start && note.end <= phrase.end)
                });
                if chart.notes.is_empty() {
                    doc.content.vocals = None;
                    doc.content.vocal_review = None;
                } else {
                    chart.validate()?;
                }
            }
            Action::SetVocalGeneration { document, review } => {
                document.validate()?;
                review.validate()?;
                review.validate_binding(&doc)?;
                review.validate_timing(&doc.timing(self)?, true)?;
                review.validate_generated_chart(document)?;
                doc.content.vocals = Some(document.clone());
                doc.content.vocal_review = Some(review.clone());
            }
            Action::SetVocalReview { review } => {
                review.validate()?;
                review.validate_binding(&doc)?;
                review.validate_timing(&doc.timing(self)?, false)?;
                ensure!(doc.content.vocals.is_some(), "No vocal chart to review");
                doc.content.vocal_review = Some(review.clone());
            }
            Action::Undo => {
                let prior = doc.undo.pop().context("Nothing to undo")?;
                doc.redo.push(self.checkpoint(&doc.content)?);
                doc.content = self.restore(&prior)?;
                doc.selected_note = None;
            }
            Action::Redo => {
                let next = doc.redo.pop().context("Nothing to redo")?;
                doc.undo.push(self.checkpoint(&doc.content)?);
                doc.content = self.restore(&next)?;
                doc.selected_note = None;
            }
            Action::Export { .. } => unreachable!(),
        }
        let timing = doc.timing(self)?;
        if matches!(
            request.action,
            Action::AddNote { .. }
                | Action::ApplyGeneratedPart { .. }
                | Action::UpdateNote { .. }
                | Action::DeleteNote { .. }
                | Action::EditNoteGroup { .. }
                | Action::Collaborate {
                    operation: crate::collaboration::Operation::Merge { .. }
                }
        ) {
            crate::difficulty_links::after_note_edit(
                &mut doc.content,
                &before,
                &timing,
                doc.duration,
            )?;
        }
        if doc
            .selected_note
            .as_ref()
            .is_some_and(|id| doc.find_note(id).is_none())
        {
            doc.selected_note = None;
        }
        if let Some(id) = &doc.selected_note
            && let Some((track, _)) = doc.find_note(id)
        {
            doc.selected_track = track.into();
        }
        // Undo can remove a newly created lower track after it was selected.
        if doc.selected_track != "Vocals" && !doc.content.tracks.contains_key(&doc.selected_track) {
            let expert = ["Easy", "Medium", "Hard"]
                .into_iter()
                .find_map(|tier| doc.selected_track.strip_prefix(tier))
                .map(|suffix| format!("Expert{suffix}"));
            doc.selected_track = expert
                .filter(|track| doc.content.tracks.contains_key(track))
                .or_else(|| doc.content.tracks.keys().next().cloned())
                .context("Project has no editable tracks")?;
            doc.selected_note = None;
            doc.selected_region = None;
        }
        if doc.selected_region.as_ref().is_some_and(|id| {
            !doc.content
                .vocal_review
                .as_ref()
                .is_some_and(|map| map.region(id).is_some())
                && !doc
                    .review
                    .regions
                    .iter()
                    .any(|r| &r.id == id && r.tracks.contains(&doc.selected_track))
        }) {
            doc.selected_region = None;
        }
        // Undo/redo can restore a different stem attachment without passing
        // through AttachStems. Never retain a component for that former source.
        if doc.kit_audition.as_ref().is_some_and(|selected| {
            crate::part_analysis::kit_source_key(&doc).is_ok_and(|key| key != selected.source_key)
        }) {
            doc.kit_audition = None;
        }
        doc.validate_notes(self)?;
        for manifest in doc
            .content
            .stems
            .iter()
            .chain(doc.content.keyboard_stems.iter())
        {
            manifest.validate_binding(&doc.audio_sha256, doc.duration)?;
        }
        doc.content.stem_mix.prepare(doc.content.stem_ids())?;
        crate::editor_validation::validate(&doc, &self.source_chart(&doc)?)?;
        crate::difficulty_links::validate(&doc.content, &timing, doc.duration)?;
        if content_action {
            doc.undo.push(self.checkpoint(&before)?);
            doc.redo.clear();
        }
        doc.revision = doc.revision.checked_add(1).context("Revision overflow")?;
        doc.audit.push(AuditEntry {
            revision: doc.revision,
            actor: request.actor,
            action: request.action,
            at_ms: now_ms(),
        });
        let result = Applied {
            revision: doc.revision,
            chart_sha256: doc.chart_hash(self)?,
            selected_note: doc.selected_note.clone(),
            exported_to: None,
        };
        self.save(&doc)?;
        Ok(result)
    }
    fn export(&self, doc: &Project, output: &Path, actor: &Actor) -> Result<()> {
        let source = self.root.join("source");
        pipeline::destination(output, Some(&source))?;
        if doc.original_folder.exists() {
            pipeline::destination(output, Some(&doc.original_folder))?;
        }
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let stage = tempfile::tempdir_in(parent)?;
        pipeline::copy_folder(&source, stage.path())?;
        ensure!(
            models::hash_file(&self.audio_path(doc)?)? == doc.audio_sha256,
            "Source audio snapshot changed"
        );
        let text = doc.chart_text(self)?;
        Chart::parse(text.clone())?;
        fs::write(stage.path().join("notes.chart"), &text)?;
        if !doc.content.track_origins.is_empty() {
            let raw = pipeline::song_ini(stage.path())?;
            let fields = pipeline::ini_fields(&raw);
            let mut updates = BTreeMap::new();
            for (track, notes) in &doc.content.tracks {
                if !doc.content.track_origins.contains_key(track)
                    || !notes.iter().any(|note| playable(track, note.note.lane))
                {
                    continue;
                }
                let key = if track.ends_with("ProKeys") {
                    "diff_keys_real"
                } else if track.ends_with("Keyboard") {
                    "diff_keys"
                } else if track.ends_with("DoubleBass") {
                    "diff_bass"
                } else if track.ends_with("Drums") {
                    "diff_drums"
                } else {
                    "diff_guitar"
                };
                if fields
                    .get(key)
                    .and_then(|value| value.parse::<i32>().ok())
                    .is_none_or(|rank| rank < 0)
                {
                    updates.insert(key.into(), "0".into());
                }
            }
            if doc.content.track_origins.contains_key("Vocals")
                && doc
                    .content
                    .vocals
                    .as_ref()
                    .is_some_and(|chart| !chart.notes.is_empty())
                && fields
                    .get("diff_vocals")
                    .and_then(|value| value.parse::<i32>().ok())
                    .is_none_or(|rank| rank < 0)
            {
                updates.insert("diff_vocals".into(), "0".into());
            }
            fs::write(
                stage.path().join("song.ini"),
                pipeline::update_ini(&raw, &updates),
            )?;
            fs::write(
                stage.path().join("chart-engine-part-origins.json"),
                serde_json::to_vec_pretty(&doc.content.track_origins)?,
            )?;
        }
        // Preserve source generation evidence, but don't present it as a fresh
        // audit of edited notes. Export decisions separately with fingerprints.
        let mut archive = stage.path().join("editor-source");
        let mut archive_index = 1;
        while archive.exists() {
            archive = stage.path().join(format!("editor-source-{archive_index}"));
            archive_index += 1;
        }
        fs::create_dir(&archive)?;
        let mut exported_chart_sha256 = hash(text.as_bytes());
        let midi_export = doc.content.vocals.is_some()
            || doc.content.tracks.keys().any(|t| crate::keys::is_pro(t));
        if midi_export {
            let (midi, delay) = crate::game_export::song(
                &Chart::parse(text.clone())?,
                doc.content.vocals.as_ref(),
            )?;
            exported_chart_sha256 = hash(&midi);
            fs::write(stage.path().join("notes.mid"), midi)?;
            fs::rename(
                stage.path().join("notes.chart"),
                archive.join("notes.chart"),
            )?;
            let ini = stage.path().join("song.ini");
            let raw = pipeline::song_ini(stage.path())?;
            fs::write(ini, crate::game_export::ini_with_delay(&raw, delay))?;
            if let Some(vocals) = &doc.content.vocals {
                fs::write(
                    stage.path().join("chart-engine-vocals.json"),
                    serde_json::to_vec_pretty(vocals)?,
                )?;
            }
            if doc.content.tracks.keys().any(|t| crate::keys::is_pro(t)) {
                let working = stage.path().join("chart-engine-working");
                if working.exists() {
                    fs::rename(&working, archive.join("chart-engine-working"))?;
                }
                fs::create_dir(&working)?;
                fs::write(working.join("notes.chart"), &text)?;
                fs::write(
                    working.join("binding.json"),
                    serde_json::to_vec_pretty(
                        &serde_json::json!({"version":1,"master_sha256":hash(text.as_bytes()),"midi_sha256":exported_chart_sha256}),
                    )?,
                )?;
            }
        }
        for name in [
            "chart-engine-review.json",
            "chart-engine-report.json",
            "chart-engine-transcription.json",
            "chart-engine-lyrics.json",
            "chart-engine-isolation.json",
            "chart-engine-keyboard-isolation.json",
            "chart-engine-pro-keys-arrangement.json",
        ] {
            let path = stage.path().join(name);
            if path.exists() {
                fs::rename(path, archive.join(name))?;
            }
        }
        fs::write(
            stage.path().join("chart-engine-melody-source.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"version":1,"audio_sha256":models::hash_file(&pipeline::audio_file(stage.path())?)?,"guitar_source":self.guitar_source(doc)?}),
            )?,
        )?;
        if let Some(manifest) = &doc.content.stems {
            fs::write(
                stage.path().join("chart-engine-isolation.json"),
                serde_json::to_vec_pretty(manifest)?,
            )?;
        }
        if let Some(manifest) = &doc.content.keyboard_stems {
            fs::write(
                stage.path().join("chart-engine-keyboard-isolation.json"),
                serde_json::to_vec_pretty(manifest)?,
            )?;
        }
        let fingerprint = hash(text.as_bytes());
        let timing = doc.timing(self)?;
        let history:Vec<_>=doc.content.decisions.iter().map(|d|serde_json::json!({"decision":d,"applies_to_current_notes":doc.region_fingerprint(&d.region_id,&d.track,&timing).is_ok_and(|hash|hash==d.region_content_sha256)})).collect();
        let report = serde_json::json!({"version":1,"project_revision":doc.revision,"chart_sha256":fingerprint,"audio_sha256":doc.audio_sha256,"exported_by":actor,"source_review_evidence_is_stale":doc.review.input_sha256.get("chart")!=Some(&fingerprint),"source_review":doc.review,"current_decisions":doc.current_decisions(&timing),"decision_history":history,"hidden_flags":doc.content.hidden_flags,"vocal_review":doc.content.vocal_review,"difficulty_links":crate::difficulty_links::snapshot(doc,&timing),"difficulty_link_state":doc.content.difficulty_links,"stems":doc.content.stems,"stem_mix_requested":doc.content.stem_mix,"stem_mix_scope":"Review preference only; exported song audio is unchanged and playback revalidates stem files","musical_acceptance":"unverified"});
        fs::write(
            stage.path().join("editor-review.json"),
            serde_json::to_vec_pretty(&{
                let mut report = report;
                report["keyboard_stems"] = serde_json::to_value(&doc.content.keyboard_stems)?;
                report
            })?,
        )?;
        fs::write(
            stage.path().join("game-export.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "version":1,"source_chart_sha256":fingerprint,"exported_chart_sha256":exported_chart_sha256,
                "format":if midi_export{"midi"}else{"chart"},"vocals":doc.content.vocals.is_some(),
                "pro_keys":doc.content.tracks.keys().any(|t|crate::keys::is_pro(t)),
                "pro_keys_ranges":doc.content.tracks.iter().filter(|(t,_)|crate::keys::is_pro(t)).map(|(t,notes)| Ok((t.clone(),crate::key_ranges::plan(&notes.iter().map(|n|n.note.clone()).collect::<Vec<_>>())?))).collect::<Result<BTreeMap<_,_>>>()?,
                "musical_acceptance":"unverified"
            }))?,
        )?;
        pipeline::publish(stage.path(), output)
    }
    pub fn inspect(&self) -> Result<serde_json::Value> {
        let doc = self.load()?;
        let current = doc.chart_hash(self)?;
        let timing = doc.timing(self)?;
        let decisions = doc.current_decisions(&timing);
        let octave_view = crate::octave_adapter::snapshot(&doc, &timing)?;
        let live: Option<serde_json::Value> = fs::read(self.root.join("live-state.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let live = live.filter(|v| {
            v["at_ms"]
                .as_u64()
                .is_some_and(|at| now_ms().saturating_sub(at) < 2000)
        });
        Ok(
            serde_json::json!({"playhead_seconds":doc.transport.current_position(doc.duration),"live_transport":live,"timing":doc.timing(self)?,"chart_sha256":current,"review_evidence_is_stale":doc.review.input_sha256.get("chart")!=Some(&current),"current_decisions":decisions,"difficulty_links":crate::difficulty_links::snapshot(&doc,&timing),"octave_view":octave_view,"project":doc}),
        )
    }

    #[cfg(feature = "editor")]
    pub(crate) fn report_live(
        &self,
        revision: u64,
        position: f64,
        playing: bool,
        audio_ready: bool,
        audition: serde_json::Value,
    ) -> Result<()> {
        let value = serde_json::json!({"at_ms":now_ms(),"revision":revision,"playhead_seconds":position,"playing":playing,"audio_output_ready":audio_ready,"audition":audition});
        let mut file = tempfile::NamedTempFile::new_in(&self.root)?;
        serde_json::to_writer(&mut file, &value)?;
        file.persist(self.root.join("live-state.json"))?;
        Ok(())
    }
}
