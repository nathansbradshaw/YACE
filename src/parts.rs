//! Add a part without changing the recording or the chart's timing.
use crate::editor::Project;
use crate::storage as fs;
use crate::{
    editor::{Content, EditorNote, Store},
    timing::TempoMap,
};
use crate::{generation, models};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use {
    sha2::{Digest, Sha256},
    std::path::Path,
};

pub const PARTS: [(&str, &str, &str); 6] = [
    ("Guitar", "Single", "guitar"),
    ("Bass", "DoubleBass", "bass"),
    ("Drums", "Drums", "drums"),
    ("Piano (Keys)", "Keyboard", "piano"),
    ("Pro Piano (Pro Keys)", "ProKeys", "pro_piano"),
    ("Vocals", "Vocals", "vocals"),
];

/// Keyboard review may retain a distinct piano separation. Other parts use
/// the primary audio; never substitute a different instrument silently.
pub fn existing_stems(content: &Content, track: &str) -> Option<crate::stem_review::StemManifest> {
    if crate::keys::is_keys(track) {
        content
            .keyboard_stems
            .as_ref()
            .or(content.stems.as_ref())
            .cloned()
    } else {
        content.stems.clone()
    }
}
pub fn instrument(track: &str) -> Result<&'static str> {
    if track == "Vocals" {
        return Ok("vocals");
    }
    let suffix = ["Expert", "Hard", "Medium", "Easy"]
        .into_iter()
        .find_map(|tier| track.strip_prefix(tier))
        .context("Choose a supported difficulty")?;
    PARTS
        .iter()
        .find(|(_, part, _)| *part == suffix && suffix != "Vocals")
        .map(|(_, _, instrument)| *instrument)
        .context("Unsupported chart part")
}
fn digest(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Origin {
    Authored,
    Generated { job: PathBuf, chart_sha256: String },
}

pub fn create(project: &mut Project, track: &str) -> Result<()> {
    instrument(track)?;
    if track == "Vocals" {
        ensure!(
            project.content.vocals.is_none() && !project.content.track_origins.contains_key(track),
            "Vocals already exist"
        );
        // The first note creates its scoring phrase; an empty lane has no chart yet.
    } else {
        ensure!(
            !project.content.tracks.contains_key(track),
            "This track already exists"
        );
        project.content.tracks.insert(track.into(), vec![]);
    }
    project
        .content
        .track_origins
        .insert(track.into(), Origin::Authored);
    project.selected_track = track.into();
    project.selected_note = None;
    project.selected_region = None;
    Ok(())
}

pub use portable::*;
mod portable {
    use super::*;
    use crate::{editor::TimeRange, reanalysis};
    use std::collections::BTreeMap;
    fn fingerprint(content: &Content, track: &str) -> Result<String> {
        if track == "Vocals" {
            digest(&(&content.vocals, &content.vocal_review))
        } else {
            // A link change can alter the target even if its visible notes match.
            digest(&(content.tracks.get(track), &content.difficulty_links))
        }
    }
    fn current_fingerprint(content: &Content, track: &str) -> Result<String> {
        digest(&(
            fingerprint(content, track)?,
            content.analysis_sources.get(track),
            if track == "Vocals" {
                content.vocal_pitch_analysis.as_ref()
            } else {
                None
            },
            &content.stems,
            &content.keyboard_stems,
        ))
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Target {
        pub version: u32,
        pub project: PathBuf,
        pub track: String,
        pub audio_sha256: String,
        pub source_chart_sha256: String,
        pub timing: TempoMap,
        pub fingerprint: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub requested_range: Option<TimeRange>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub effective_range: Option<TimeRange>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        pub affected_fingerprints: BTreeMap<String, String>,
    }
    impl Target {
        pub fn capture(store: &Store, project: &Project, track: &str) -> Result<Self> {
            ensure!(
                !track.ends_with("Drums")
                    || project.drum_layout() != crate::drum_layout::Layout::FiveLane,
                "Drum generation currently produces four-lane charts; keep this five-lane part or generate into a separate project"
            );
            instrument(track)?;
            Ok(Self {
                version: 2,
                project: fs::canonicalize(&store.root)?,
                track: track.into(),
                audio_sha256: project.audio_sha256.clone(),
                source_chart_sha256: project.source_chart_sha256.clone(),
                timing: project.timing(store)?,
                fingerprint: fingerprint(&project.content, track)?,
                requested_range: None,
                effective_range: None,
                affected_fingerprints: reanalysis::affected(&project.content, track)
                    .into_iter()
                    .map(|track| {
                        Ok((
                            track.clone(),
                            current_fingerprint(&project.content, &track)?,
                        ))
                    })
                    .collect::<Result<_>>()?,
            })
        }
        pub fn with_range(mut self, project: &Project, range: Option<TimeRange>) -> Result<Self> {
            self.requested_range = range;
            self.effective_range = range
                .map(|range| {
                    reanalysis::expand(
                        range,
                        &[&project.content],
                        &self
                            .affected_fingerprints
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>(),
                        &self.timing,
                        project.duration,
                    )
                })
                .transpose()?;
            Ok(self)
        }
        pub fn validate(&self, store: &Store, project: &Project) -> Result<()> {
            ensure!(
                !self.track.ends_with("Drums")
                    || project.drum_layout() != crate::drum_layout::Layout::FiveLane,
                "A four-lane generated draft cannot replace this five-lane drum part; both versions have been kept"
            );
            instrument(&self.track)?;
            if self.version >= 2 {
                ensure!(
                    self.affected_fingerprints
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        == reanalysis::affected(&project.content, &self.track),
                    "Affected difficulties changed; analyze again"
                );
                ensure!(
                    self.requested_range.is_some() == self.effective_range.is_some(),
                    "Incomplete saved analysis range"
                );
                if let (Some(requested), Some(effective)) =
                    (self.requested_range, self.effective_range)
                {
                    ensure!(
                        effective.start <= requested.start && effective.end >= requested.end,
                        "Effective range does not cover the requested loop"
                    );
                }
            }
            ensure!(
                matches!(self.version, 1 | 2)
                    && self.project == fs::canonicalize(&store.root)?
                    && self.audio_sha256 == project.audio_sha256
                    && self.source_chart_sha256 == project.source_chart_sha256
                    && self.timing == project.timing(store)?,
                "This draft belongs to a different project or recording"
            );
            if let Some(range) = self.requested_range {
                reanalysis::validate_range(range, project.duration)?;
            }
            if let Some(range) = self.effective_range {
                reanalysis::validate_range(range, project.duration)?;
            }
            for (track, expected) in &self.affected_fingerprints {
                ensure!(
                    current_fingerprint(&project.content, track)? == *expected,
                    "An affected difficulty or sound source changed. Both versions were kept; analyze again."
                );
            }
            ensure!(
                fingerprint(&project.content, &self.track)? == self.fingerprint,
                "This part changed during generation. Your edits and the generated draft were kept; start a new draft to replace this part."
            );
            Ok(())
        }
    }

    pub fn target(job: &Path) -> Result<Option<Target>> {
        let path = job.join("editor-target.json");
        if fs::exists(&path) {
            let mut target: Target = serde_json::from_slice(&fs::read(path)?)?;
            if target.project.is_relative() {
                target.project = fs::canonicalize(job.join(&target.project))?;
            }
            Ok(Some(target))
        } else {
            Ok(None)
        }
    }
    pub fn timing(job: &Path, bpm: f64) -> Result<TempoMap> {
        if let Some(target) = target(job)? {
            let store = Store::new(&target.project);
            let project = store.load()?;
            target.validate(&store, &project)?;
            Ok(target.timing)
        } else {
            TempoMap::constant(bpm)
        }
    }
    pub fn save_review(job: &Path) -> Result<()> {
        let target = target(job)?.context("Missing part target")?;
        let store = Store::new(&target.project);
        let current = store.load()?;
        let prepared = Prepared::load(&store, &current, job)?;
        let proposed = prepared.preview(&current)?;
        let request = generation::read_request(job)?;
        generation::write_json(
            &job.join("reanalysis-review.json"),
            &serde_json::json!({"version":1,"policy":crate::detection_settings::POLICY,"detection":request.detection,"requested_range":target.requested_range,"effective_range":prepared.range(),"changes":reanalysis::changes(&current,&proposed,&reanalysis::affected(&current.content,&target.track)),"musical_acceptance":"unverified"}),
        )
    }

    pub fn create_job(
        mut request: generation::Request,
        output: &Path,
        target: &Target,
    ) -> Result<PathBuf> {
        let store = Store::new(&target.project);
        let project = store.load()?;
        target.validate(&store, &project)?;
        if target.track == "Vocals"
            && let Some(chart) = &project.content.vocals
        {
            let mut text = String::new();
            let mut join = false;
            for note in &chart.notes {
                if let crate::vocals::Syllable::Text {
                    text: syllable,
                    join_next,
                } = &note.lyric
                {
                    if !text.is_empty() && !join {
                        text.push(' ');
                    }
                    text.push_str(syllable);
                    join = *join_next;
                }
            }
            if !text.is_empty() {
                request
                    .song_info
                    .get_or_insert_with(Default::default)
                    .lyrics = Some(
                    serde_json::json!({"plainLyrics":text,"source":"Existing authored vocal text; reanalysis changes detection only"}),
                );
            }
        }
        let generation::Source::Audio { path } = &request.source;
        ensure!(
            models::hash_file(path)? == target.audio_sha256,
            "Recording changed before generation"
        );
        ensure!(
            request.instruments == [instrument(&target.track)?.to_owned()],
            "Generate only the requested part"
        );
        ensure!(
            target.version >= 2
                || instrument(&target.track)? != "guitar"
                || request.guitar_source == store.guitar_source(&project)?,
            "Keep the chart's existing Guitar sound source"
        );
        let job = generation::create_job(request, output)?;
        let mut saved_target = target.clone();
        let project = fs::canonicalize(&target.project)?;
        let job_parts = job.components().collect::<Vec<_>>();
        let project_parts = project.components().collect::<Vec<_>>();
        let shared = job_parts
            .iter()
            .zip(&project_parts)
            .take_while(|(left, right)| left == right)
            .count();
        let mut relative = PathBuf::new();
        for _ in shared..job_parts.len() {
            relative.push("..");
        }
        for component in &project_parts[shared..] {
            relative.push(component.as_os_str());
        }
        saved_target.project = relative;
        generation::write_json(&job.join("editor-target.json"), &saved_target)?;
        Ok(job)
    }

    #[derive(Clone)]
    pub struct Prepared {
        target: Target,
        generated: Project,
        origin: Origin,
        source: crate::mapping::GuitarSource,
    }
    impl Prepared {
        pub fn load(store: &Store, project: &Project, job: &Path) -> Result<Self> {
            ensure!(
                generation::status(job)?.state == generation::State::Complete,
                "Draft is not finished"
            );
            let mut target = target(job)?.context("This job was not created for an open chart")?;
            target.validate(store, project)?;
            ensure!(
                models::hash_file(&store.audio_path(project)?)? == target.audio_sha256,
                "Recording changed"
            );
            let request: generation::Request =
                serde_json::from_slice(&fs::read(job.join("request.json"))?)?;
            ensure!(
                models::hash_file(&job.join("request.json"))?
                    == generation::status(job)?.request_sha256,
                "Saved generation settings changed"
            );
            ensure!(
                request.instruments == [instrument(&target.track)?.to_owned()],
                "Draft names a different part"
            );
            ensure!(
                target.version >= 2
                    || instrument(&target.track)? != "guitar"
                    || request.guitar_source == store.guitar_source(project)?,
                "Draft uses a different Guitar sound source"
            );
            let generated_store = Store::new(&generation::project_path(job));
            let generated = generated_store.load()?;
            if let Some(range) = target.effective_range {
                target.effective_range = Some(reanalysis::expand(
                    range,
                    &[&project.content, &generated.content],
                    &reanalysis::affected(&project.content, &target.track),
                    &target.timing,
                    project.duration,
                )?);
            }
            ensure!(
                generated.audio_sha256 == target.audio_sha256
                    && generated.timing(&generated_store)? == target.timing
                    && (generated.duration - project.duration).abs() <= 0.1,
                "Generated part does not match the recording and timing"
            );
            ensure!(
                models::hash_file(&generated_store.audio_path(&generated)?)? == target.audio_sha256,
                "Generated recording snapshot changed"
            );
            if target.track == "Vocals" {
                ensure!(
                    generated
                        .content
                        .vocals
                        .as_ref()
                        .is_some_and(|chart| chart.notes.iter().any(|note| target
                            .effective_range
                            .is_none_or(|range| reanalysis::contains(
                                range,
                                target.timing.seconds(note.start)
                            )))),
                    "No vocal notes were detected; the current part was kept"
                );
            } else {
                ensure!(
                    generated
                        .content
                        .tracks
                        .get(&target.track)
                        .is_some_and(|notes| notes.iter().any(|note| crate::editor::playable(
                            &target.track,
                            note.note.lane
                        ) && target
                            .effective_range
                            .is_none_or(|range| reanalysis::contains(
                                range,
                                target.timing.seconds(note.note.tick)
                            )))),
                    "No notes were detected for this difficulty; the current part was kept"
                );
            }
            let origin = Origin::Generated {
                job: fs::canonicalize(job)?,
                chart_sha256: generated.chart_hash(&generated_store)?,
            };
            let mut prepared = Self {
                target,
                generated,
                origin,
                source: request.guitar_source,
            };
            while let Some(range) = prepared.target.effective_range {
                let preview = prepared.preview(project)?;
                let expanded = reanalysis::include_linked_changes(
                    range,
                    &project.content,
                    &preview.content,
                    &reanalysis::affected(&project.content, &prepared.target.track),
                    &prepared.target.timing,
                    project.duration,
                )?;
                if expanded == range {
                    break;
                }
                prepared.target.effective_range = Some(expanded);
            }
            Ok(prepared)
        }
        pub fn range(&self) -> Option<TimeRange> {
            self.target.effective_range
        }
        pub fn preview(&self, project: &Project) -> Result<Project> {
            let mut preview = project.clone();
            if preview.content.stems.is_none() {
                preview.content.stems = self.generated.content.stems.clone();
            }
            if preview.content.keyboard_stems.is_none() {
                preview.content.keyboard_stems = self.generated.content.keyboard_stems.clone();
            }
            self.clone().apply_content(&mut preview)?;
            crate::difficulty_links::after_note_edit(
                &mut preview.content,
                &project.content,
                &self.target.timing,
                project.duration,
            )?;
            Ok(preview)
        }
        pub fn apply(self, store: &Store, project: &mut Project) -> Result<()> {
            self.target.validate(store, project)?;
            let track = self.target.track.clone();
            let audio = store.audio_path(project)?;
            let mut attached = false;
            let mut retain =
                |existing: &mut Option<crate::stem_review::StemManifest>,
                 candidate: Option<&crate::stem_review::StemManifest>| {
                    if existing.is_some() {
                        return;
                    }
                    let Some(candidate) = candidate else {
                        return;
                    };
                    match candidate.validate_source(&audio, &project.audio_sha256, project.duration)
                    .and_then(|manifest| crate::stem_review::game_audio::retain(&store.root, &manifest)) {
                    Ok(manifest) => { *existing = Some(manifest); attached = true; }
                    Err(error) => project.review.coverage_limits.push(format!("Generated stems were not attached: {error:#}. Existing recording and part audio were kept.")),
                }
                };
            // Keep established audition sources. A newly generated part may
            // supply the first primary set, but never replaces another set.
            retain(
                &mut project.content.stems,
                self.generated.content.stems.as_ref(),
            );
            if crate::keys::is_keys(&track) {
                retain(
                    &mut project.content.keyboard_stems,
                    self.generated.content.keyboard_stems.as_ref(),
                );
            }
            if attached {
                project.content.stem_mix = Default::default();
            }
            self.apply_content(project)
        }
        fn apply_content(mut self, project: &mut Project) -> Result<()> {
            let track = self.target.track.clone();
            let range = self.target.effective_range;
            let in_scope = |tick| {
                range.is_none_or(|range| {
                    reanalysis::contains(range, self.target.timing.seconds(tick))
                })
            };
            if track == "Vocals" {
                if range.is_some() {
                    let mut generated = self
                        .generated
                        .content
                        .vocals
                        .take()
                        .context("Missing vocal draft")?;
                    generated.notes.retain(|note| in_scope(note.start));
                    generated.phrases.retain(|phrase| in_scope(phrase.start));
                    if let Some(old) = &project.content.vocals {
                        generated.notes.extend(
                            old.notes
                                .iter()
                                .filter(|note| !in_scope(note.start))
                                .cloned(),
                        );
                        generated.phrases.extend(
                            old.phrases
                                .iter()
                                .filter(|phrase| !in_scope(phrase.start))
                                .cloned(),
                        );
                    }
                    generated.notes.sort_by_key(|note| note.start);
                    generated.phrases.sort_by_key(|phrase| phrase.start);
                    generated.validate()?;
                    project.content.vocals = Some(generated);
                    // Historical evidence uses exact note timing and pitch; changed
                    // notes cannot inherit an unrelated per-note model vote.
                    project.content.vocal_pitch_analysis = None;
                } else {
                    project.content.vocals = self.generated.content.vocals;
                    project.content.vocal_review = self.generated.content.vocal_review;
                    project.content.vocal_pitch_analysis =
                        self.generated.content.vocal_pitch_analysis;
                }
            } else {
                let notes = self
                    .generated
                    .content
                    .tracks
                    .remove(&track)
                    .context("Generated part is missing")?;
                let mut imported = Vec::with_capacity(notes.len());
                for item in notes {
                    if !in_scope(item.note.tick) {
                        continue;
                    }
                    let id = format!("edit-note-{}", project.next_id);
                    project.next_id = project.next_id.checked_add(1).context("Note ID overflow")?;
                    imported.push(EditorNote {
                        midi: None,
                        id,
                        note: item.note,
                    });
                }
                if range.is_some() {
                    imported.extend(
                        project
                            .content
                            .tracks
                            .get(&track)
                            .into_iter()
                            .flatten()
                            .filter(|note| !in_scope(note.note.tick))
                            .cloned(),
                    );
                }
                imported.sort_by(|a, b| a.note.cmp(&b.note).then(a.id.cmp(&b.id)));
                project.content.tracks.insert(track.clone(), imported);
            }
            if self.target.version >= 2 && instrument(&track)? == "guitar" {
                let range = range.unwrap_or(TimeRange {
                    start: 0.,
                    end: project.duration,
                });
                for affected in reanalysis::affected(&project.content, &track) {
                    reanalysis::replace_source(&mut project.content, &affected, range, self.source);
                }
            }
            project
                .content
                .track_origins
                .insert(track.clone(), self.origin);
            project.selected_track = track;
            project.selected_note = None;
            project.selected_region = None;
            Ok(())
        }
    }

    /// Vocal-only jobs need an editable song container, but no instrumental inference.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn vocal_song(
        job: &Path,
        audio: &Path,
        request: &generation::Request,
        timing: &TempoMap,
    ) -> Result<()> {
        let stage = tempfile::tempdir_in(job)?;
        crate::song_layout::prepare(stage.path())?;
        let extension = audio
            .extension()
            .and_then(|value| value.to_str())
            .context("Missing audio extension")?;
        ensure!(
            ["wav", "mp3", "ogg", "opus"].contains(&extension),
            "Expected prepared recording"
        );
        fs::copy(audio, stage.path().join(format!("song.{extension}")))?;
        fs::write(
            stage.path().join("notes.chart"),
            crate::chart::new_chart(
                &request.title,
                &request.artist,
                timing,
                &BTreeMap::from([("ExpertSingle".into(), vec![])]),
            ),
        )?;
        fs::write(
            stage.path().join("song.ini"),
            format!(
                "[song]\nname = {}\nartist = {}\n",
                request.title, request.artist
            ),
        )?;
        if job.join("isolation.json").exists() {
            let manifest = crate::stem_review::StemManifest::read(&job.join("isolation.json"))?;
            generation::write_song_manifest(
                job,
                stage.path(),
                "chart-engine-isolation.json",
                &manifest,
            )?;
        }
        let supplied = request
            .song_info
            .as_ref()
            .and_then(|info| info.lyrics.clone());
        let lyrics = if supplied.is_some() || !request.lyrics {
            supplied
        } else {
            let mut meta = crate::pipeline::metadata(audio)?;
            meta.name = request.title.clone();
            meta.artist = request.artist.clone();
            crate::pipeline::lyrics(&meta)?
        };
        if let Some(lyrics) = lyrics {
            generation::write_json(
                &crate::song_layout::path(stage.path(), "chart-engine-lyrics.json"),
                &lyrics,
            )?;
        }
        crate::pipeline::publish(stage.path(), &job.join("song"))
    }
}
