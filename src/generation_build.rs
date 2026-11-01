//! Shared chart building, evidence, review and game export for both clients.
use crate::{
    chart::{self, Chart},
    mapping::{self, Transcription},
    midi, models,
    pipeline::{
        Metadata, audio_file, chart_file, copy_song_source, destination, ini_fields, lyrics,
        organize_song_stage, publish, song_ini, update_ini, validate_existing,
    },
    song_layout, storage as fs,
    timing::TempoMap,
};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
pub(crate) fn isolation_limits(manifest: &crate::stem_review::StemManifest) -> Vec<String> {
    let mut warnings = vec!["Source-bound stems were validated for hashes, format and full-song sample counts. Isolation can still leak other instruments, omit notes or shift transients; scores remain model outputs, not probabilities of correct playable notes. Review weak attacks, bass pitch and candidate scores; no listening approval is implied.".into()];
    if manifest.generation.is_none() {
        warnings.push("Supplied isolation is an explicit alignment attestation, not a contemporaneous model-bound native separation receipt. Its actual source/model history and phase alignment are not independently established.".into());
    }
    warnings
}

pub(crate) fn higher_quality_manifest(manifest: &crate::stem_review::StemManifest) -> bool {
    manifest.generation.as_ref().is_some_and(|identity| {
        identity.model_id == crate::keyboard_separation::MODEL_ID
            && identity.model_sha256 == crate::keyboard_separation::asset().sha256
    })
}

/// Copy portable song assets and evidence without copying a live editor into
/// its own source snapshot. Unknown song files remain byte-preserving.
/// Archive copied generation sidecars before a new additive pass. This only
/// mutates the unpublished staging copy; source history is never overwritten.
fn archive_generation_sidecars(stage: &Path, source: Option<&Path>) -> Result<Option<PathBuf>> {
    let mut files = BTreeMap::new();
    for name in [
        "chart-engine-report.json",
        "chart-engine-transcription.json",
        "chart-engine-lyrics.json",
        "chart-engine-review.json",
        "chart-engine-isolation.json",
        "chart-engine-keyboard-isolation.json",
        "chart-engine-pro-keys-arrangement.json",
    ] {
        let path = song_layout::resolve(stage, name);
        if fs::exists(&path) {
            ensure!(
                fs::is_file(&path),
                "Generation sidecar must be a file: {name}"
            );
            files.insert(name, models::hash_file(&path)?);
        }
    }
    if files.is_empty() {
        return Ok(None);
    }
    let id = format!("{:x}", Sha256::digest(serde_json::to_vec(&files)?));
    let relative = song_layout::relative("chart-engine-history").join(id);
    let history = stage.join(&relative);
    fs::create_dir_all(history.parent().context("History parent missing")?)?;
    // Reject history-name collisions, even when the main files match.
    // An existing entry may contain extra recovery files.
    ensure!(
        !fs::exists(&history),
        "Generation history already contains this receipt set"
    );
    fs::create_dir(&history).context("Generation history already contains this receipt set")?;
    for name in files.keys() {
        fs::rename(song_layout::resolve(stage, name), history.join(name))?;
    }
    fs::write(
        history.join("archive-sha256.json"),
        serde_json::to_vec_pretty(&files)?,
    )?;
    fs::write(
        history.join("archive-origin.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "source_folder":source.map(fs::canonicalize).transpose()?,
            "relative_asset_base":"Original source_folder; archived sidecars retain their exact original bytes and are historical evidence, not active manifests",
        }))?,
    )?;
    Ok(Some(relative))
}

pub struct Build<'a> {
    pub audio: &'a Path,
    pub output: &'a Path,
    pub existing: Option<&'a Path>,
    pub timing: Option<TempoMap>,
    pub transcription: &'a Transcription,
    pub meta: Metadata,
    pub pro: bool,
    pub lookup_lyrics: bool,
}

/// Attach generated-rule review prompts to the same timed map used by the
/// native queue and automation. Existing pattern flags are retained once.
pub fn with_generated_rule_reviews(
    map: crate::review_regions::ReviewMap,
    rules: &crate::generated_rules::RuleReport,
) -> Result<crate::review_regions::ReviewMap> {
    use crate::{
        generated_rules::{RuleCode, Severity},
        review_regions::{Issue, Reason, ReviewMap},
    };
    ensure!(
        rules.version == 1,
        "Unsupported generated-rule report version"
    );
    ensure!(
        !rules.has_errors(),
        "Cannot publish review markers for structurally rejected generated notes"
    );
    let generated = map
        .generated_tracks
        .as_ref()
        .context("Generated-rule review requires known generated-track provenance")?;
    let mut issues: Vec<_> = map
        .regions
        .iter()
        .flat_map(|region| region.issues.clone())
        .collect();
    let mut checks = map.checks_performed.clone();
    checks.extend([
        Reason::FastRepeat,
        Reason::HandCrowding,
        Reason::SustainOverlap,
        Reason::ReductionDifference,
    ]);
    for finding in rules
        .findings
        .iter()
        .filter(|finding| finding.severity == Severity::Review)
    {
        ensure!(
            generated.contains(&finding.track),
            "Generated-rule finding names a preserved or unknown track"
        );
        let reason = match finding.code {
            RuleCode::FastRepeat => Reason::FastRepeat,
            RuleCode::HandCrowding => Reason::HandCrowding,
            RuleCode::SustainRetriggerOverlap => Reason::SustainOverlap,
            RuleCode::LowerTierAttackAbsentFromExpert => Reason::ReductionDifference,
            _ => continue,
        };
        // The existing hand check deliberately collapses a crowded burst to
        // one prompt. Do not reintroduce one marker for every nearby window.
        let tolerance = if reason == Reason::HandCrowding {
            0.020 + 1e-9
        } else {
            1e-6
        };
        if issues.iter().any(|issue| {
            issue.track == finding.track
                && issue.reason == reason
                && (issue.start - finding.seconds).abs() <= tolerance
        }) {
            continue;
        }
        issues.push(Issue::at(
            finding.seconds,
            &finding.track,
            reason,
            &[("generated_rule_tick", finding.tick as f64)],
        ));
    }
    let mut merged = ReviewMap::build(
        map.duration_seconds,
        map.policy,
        map.input_sha256,
        map.generated_tracks,
        checks,
        issues,
    )?;
    merged.coverage_limits = map.coverage_limits;
    merged.coverage_limits.push("Generated-rule sustain/reduction checks cover only newly emitted tracks. Fast patterns and arrangement differences remain review prompts; the generation receipt retains the exact rule codes and explanations.".into());
    // Keep explicit acceptance language from the originating review map.
    merged.musical_acceptance = map.musical_acceptance;
    merged.unflagged_sections = map.unflagged_sections;
    Ok(merged)
}

pub fn build(request: Build<'_>) -> Result<serde_json::Value> {
    build_with_lyrics(request, None)
}

pub fn build_with_lyrics(
    request: Build<'_>,
    supplied_lyrics: Option<&serde_json::Value>,
) -> Result<serde_json::Value> {
    let Build {
        audio,
        output,
        existing,
        timing,
        transcription,
        mut meta,
        pro,
        lookup_lyrics,
    } = request;
    destination(output, existing)?;
    transcription.validate()?;
    if let Some(isolation) = &transcription.isolation {
        isolation.validate_source(audio, &models::hash_file(audio)?, meta.duration)?;
    }
    if let Some(analysis) = &transcription.keyboard_analysis {
        analysis.validate_source(audio, &models::hash_file(audio)?, meta.duration)?;
    }
    if let Some(source) = existing {
        validate_existing(source)?;
    }
    let source_chart = existing.map(chart_file).transpose()?;
    let chart_doc = source_chart
        .as_ref()
        .filter(|p| p.extension().is_some_and(|e| e == "chart"))
        .map(|p| -> Result<Chart> {
            crate::song_files::timing::apply_ini(
                Chart::parse(fs::read_to_string(p)?)?,
                &song_ini(existing.context("Missing source song")?)?,
            )
        })
        .transpose()?;
    let midi_bytes = source_chart
        .as_ref()
        .filter(|p| p.extension().is_some_and(|e| e == "mid"))
        .map(fs::read)
        .transpose()?;
    let map = if let Some(chart) = &chart_doc {
        chart.timing.clone()
    } else if let Some(bytes) = &midi_bytes {
        let mut map=midi::timing(bytes)?;
        map.offset=crate::song_files::timing::legacy_delay(&song_ini(existing.context("Missing source song")?)?)?.unwrap_or(0.0);
        map
    } else {
        timing.context("New songs require --bpm; automatic beat tracking is not ported yet")?
    };
    let generated = mapping::tracks(transcription, &map, pro)?;
    let authored: crate::chart::Tracks = if let Some(chart) = &chart_doc {
        chart
            .sections
            .iter()
            .filter(|section| {
                ["Easy", "Medium", "Hard", "Expert"].iter().any(|tier| {
                    ["Single", "DoubleBass", "Drums", "Keyboard", "ProKeys"]
                        .iter()
                        .any(|instrument| section.name == format!("{tier}{instrument}"))
                })
            })
            .map(|section| Ok((section.name.clone(), chart.notes(&section.name)?)))
            .collect::<Result<_>>()?
    } else {
        // MIDI enhancement preserves whole existing instrument chunks, so no
        // newly emitted lower row can conflict with its authored Expert part.
        Default::default()
    };
    let (generated, difficulty_reduction) =
        crate::difficulty_links::reduce_generated(&generated, &authored, &map, meta.duration)?;
    ensure!(
        generated.values().any(|v| !v.is_empty()),
        "No playable notes detected; no output created"
    );
    let (mut content, chart_name, added) = if let Some(chart) = &chart_doc {
        let (text, added) = chart.enhance(&generated)?;
        (text.into_bytes(), "notes.chart", added)
    } else if let Some(bytes) = &midi_bytes {
        let (bytes, added) = midi::enhance(bytes, &generated)?;
        (bytes, "notes.mid", added)
    } else {
        (
            chart::new_chart(&meta.name, &meta.artist, &map, &generated).into_bytes(),
            "notes.chart",
            generated.keys().cloned().collect(),
        )
    };
    ensure!(
        !added.is_empty(),
        "No missing tracks to add; no output created"
    );
    // MIDI receipts identify whole PART tracks, while the shared review model
    // uses difficulty names. Normalize without changing receipt semantics.
    let review_generated: BTreeSet<String> = generated
        .keys()
        .filter(|name| {
            if chart_name == "notes.chart" {
                added.contains(name)
            } else {
                [
                    ("Single", "PART GUITAR"),
                    ("DoubleBass", "PART BASS"),
                    ("Drums", "PART DRUMS"),
                    ("Keyboard", "PART KEYS"),
                ]
                .iter()
                .any(|(suffix, label)| name.ends_with(suffix) && added.iter().any(|a| a == label))
                    || crate::keys::midi_track(name)
                        .is_some_and(|label| added.iter().any(|a| a == label))
            }
        })
        .cloned()
        .collect();
    let emitted: crate::chart::Tracks = generated
        .iter()
        .filter(|(name, _)| review_generated.contains(*name))
        .map(|(name, notes)| (name.clone(), notes.clone()))
        .collect();
    let difficulty_reduction: BTreeMap<_, _> = difficulty_reduction
        .into_iter()
        .filter(|(track, _)| review_generated.contains(track))
        .collect();
    let rule_report = crate::generated_rules::assess(&emitted, &map, meta.duration);
    ensure!(
        !rule_report.has_errors(),
        "Generated chart violates structural rules: {}",
        serde_json::to_string(&rule_report)?
    );
    // Older saved inference evidence described the old range-dropping mapper.
    // Preserve those source bytes, but describe this newly generated arrangement
    // with the current rule rather than repeating an obsolete export warning.
    let mut warnings: Vec<_> = transcription
        .warnings
        .iter()
        .filter(|warning| {
            !warning.contains("omitted from the playable25-key arrangement")
                && !warning.contains("Pro Keys preserves MIDI48–72")
        })
        .cloned()
        .collect();
    let pro_arrangement = (!transcription.pro_piano.is_empty())
        .then(|| crate::pro_keys::arrange_transcription(transcription, &map))
        .transpose()?;
    let pro_arrangement_summary = pro_arrangement.as_ref().map(|plan| serde_json::json!({
        "rule_id":plan.rule_id,"source_candidates":plan.decisions.len(),"expert_notes":plan.notes.len(),
        "octave_adapted_candidates":plan.octave_adapted_candidates,
        "merged_simultaneous_candidates":plan.merged_simultaneous_candidates,
        "merged_octave_collisions":plan.merged_octave_collisions,
        "shortened_releases":plan.shortened_releases,"out_of_range_omitted":plan.out_of_range_omitted,
        "evidence":song_layout::relative("chart-engine-pro-keys-arrangement.json")
    }));
    if let Some(plan) = &pro_arrangement {
        warnings.push(format!("Pro Keys register arrangement {}: {} octave adaptations, {} simultaneous unison merges, {} near-simultaneous octave collisions merged, {} shortened source releases; {} candidates omitted for pitch range. Original detections and every arrangement decision are retained for review.", plan.rule_id,plan.octave_adapted_candidates,plan.merged_simultaneous_candidates,plan.merged_octave_collisions,plan.shortened_releases,plan.out_of_range_omitted));
    }
    let pro_keys_ranges: BTreeMap<_, _> = emitted
        .iter()
        .filter(|(track, _)| crate::keys::is_pro(track))
        .map(|(track, notes)| Ok((track.clone(), crate::key_ranges::plan(notes)?)))
        .collect::<Result<_>>()?;
    for (track, plan) in &pro_keys_ranges {
        if !plan.uncovered.is_empty() {
            warnings.push(format!("{track}: {} sounding pitch groups exceed a single YARG display range. MIDI retains every pitch; explicit Pro Keys range review is required.", plan.uncovered.len()));
        }
    }
    if let Some(isolation) = &transcription.isolation {
        for warning in isolation_limits(isolation) {
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
    }
    let stage = staging(output)?;
    let mut ini = if let Some(source) = existing {
        copy_song_source(source, stage.path())?;
        song_ini(source)?
    } else {
        String::new()
    };
    organize_song_stage(stage.path())?;
    song_layout::prepare(stage.path())?;
    if chart_doc.is_some() && crate::song_files::timing::legacy_delay(&ini)?.is_some(){
        crate::song_files::timing::retain_original(stage.path(),&fs::read_to_string(source_chart.as_ref().context("Missing source chart")?)?,&ini)?;
        ini=crate::game_export::ini_with_delay(&ini,0);
    }
    let previous_generation = archive_generation_sidecars(stage.path(), existing)?;
    if let Some(history) = &previous_generation {
        warnings.push(format!(
            "Prior generation receipts retained in {}; source files were not changed",
            history.display()
        ));
    }
    if !ini.is_empty() {
        for (key, value) in ini_fields(&ini) {
            match key.as_str() {
                "name" if meta.name.is_empty() => meta.name = value,
                "artist" if meta.artist.is_empty() => meta.artist = value,
                _ => {}
            }
        }
    }
    ensure!(
        !meta.name.trim().is_empty() && !meta.artist.trim().is_empty(),
        "Title and artist must come from tags or --title/--artist"
    );
    let mut updates = BTreeMap::new();
    if existing.is_none() {
        for (k, v) in [
            ("name", meta.name.clone()),
            ("artist", meta.artist.clone()),
            ("album", meta.album.clone()),
            ("year", meta.year.clone()),
            ("genre", meta.genre.clone()),
            ("charter", "YACE (automatic draft)".into()),
        ] {
            ensure!(!v.contains(['\r', '\n']), "Metadata contains a newline");
            updates.insert(k.into(), v);
        }
        let extension = audio
            .extension()
            .and_then(|e| e.to_str())
            .context("Audio needs a file extension")?;
        if matches!(
            extension.to_ascii_lowercase().as_str(),
            "ogg" | "opus" | "mp3" | "wav"
        ) {
            fs::copy(
                audio,
                stage
                    .path()
                    .join(format!("song.{}", extension.to_ascii_lowercase())),
            )?;
        } else {
            convert_audio(audio, &stage.path().join("song.wav"))?;
            warnings.push("Input audio was converted to song.wav for game compatibility; the input file was preserved.".into());
        }
    }
    for (suffix, part, key) in [
        ("Single", "PART GUITAR", "diff_guitar"),
        ("DoubleBass", "PART BASS", "diff_bass"),
        ("Drums", "PART DRUMS", "diff_drums"),
        ("Keyboard", "PART KEYS", "diff_keys"),
        ("ProKeys", "PART REAL_KEYS_X", "diff_keys_real"),
    ] {
        if added.iter().any(|n| n.ends_with(suffix) || n == part) {
            let count = generated.get(&format!("Expert{suffix}")).map_or(0, |n| {
                n.iter()
                    .filter(|n| crate::editor::playable(&format!("Expert{suffix}"), n.lane))
                    .count()
            });
            let nps = count as f64 / meta.duration.max(1.0);
            let rating = if nps < 1.5 {
                1
            } else if nps < 3.0 {
                2
            } else if nps < 4.5 {
                3
            } else if nps < 6.0 {
                4
            } else if nps < 7.5 {
                5
            } else {
                6
            };
            updates.insert(key.into(), rating.to_string());
            if suffix == "Drums" {
                updates.insert(
                    "pro_drums".into(),
                    if pro { "True" } else { "False" }.into(),
                );
                updates.insert(
                    "diff_drums_real".into(),
                    if pro { rating } else { -1 }.to_string(),
                );
            }
        }
    }
    if lookup_lyrics {
        match supplied_lyrics
            .map(|value| Ok(Some(value.clone())))
            .unwrap_or_else(|| lyrics(&meta))
        {
            Ok(Some(record)) => {
                if let Some(lrc) = record.get("syncedLyrics").and_then(|s| s.as_str())
                    && chart_name == "notes.chart"
                {
                    let raw = String::from_utf8(content.clone())?;
                    let doc = Chart::parse(raw)?;
                    let has_lyrics = doc
                        .sections
                        .iter()
                        .find(|s| s.name == "Events")
                        .is_some_and(|s| doc.raw[s.body_start..s.body_end].contains("lyric "));
                    if !has_lyrics
                        && let Some(section) = doc.sections.iter().find(|s| s.name == "Events")
                    {
                        let lines = lyric_events(lrc, &map, meta.duration);
                        let mut text = doc.raw.clone();
                        text.insert_str(section.body_end, &lines);
                        content = text.into_bytes();
                    }
                }
                fs::write(
                    song_layout::path(stage.path(), "chart-engine-lyrics.json"),
                    serde_json::to_vec_pretty(&record)?,
                )?;
                warnings.push("Lyrics are database text/line timings; word-level alignment is not implemented. Existing lyric events are preserved.".into());
            }
            Ok(None) => warnings.push(
                "No exact artist/title/duration lyrics match; existing lyrics were retained."
                    .into(),
            ),
            Err(e) => warnings.push(format!(
                "Lyrics unavailable ({e}); existing lyrics were retained."
            )),
        }
    }
    fs::write(stage.path().join(chart_name), content)?;
    fs::write(stage.path().join("song.ini"), update_ini(&ini, &updates))?;
    fs::write(
        song_layout::path(stage.path(), "chart-engine-transcription.json"),
        serde_json::to_vec_pretty(transcription)?,
    )?;
    if added
        .iter()
        .any(|name| name.ends_with("Single") || name == "PART GUITAR")
    {
        fs::write(
            song_layout::path(stage.path(), "chart-engine-melody-source.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"version":1,"audio_sha256":models::hash_file(&audio_file(stage.path())?)?,"guitar_source":transcription.guitar_source}),
            )?,
        )?;
    }
    if let Some(plan) = &pro_arrangement {
        fs::write(
            song_layout::path(stage.path(), "chart-engine-pro-keys-arrangement.json"),
            serde_json::to_vec_pretty(&serde_json::json!({"version":1,
                "source_transcription_sha256":models::hash_file(&song_layout::path(stage.path(), "chart-engine-transcription.json"))?,
                "arrangement":plan}))?,
        )?;
    }
    if let Some(isolation) = &transcription.isolation {
        fs::write(
            song_layout::path(stage.path(), "chart-engine-isolation.json"),
            serde_json::to_vec_pretty(isolation)?,
        )?;
    }
    if let Some(analysis) = &transcription.keyboard_analysis {
        fs::write(
            song_layout::path(stage.path(), "chart-engine-keyboard-isolation.json"),
            serde_json::to_vec_pretty(analysis)?,
        )?;
    }
    warnings.push("Instrument intensity is a note-density heuristic. Chart validity is not a listening/playability approval.".into());
    let mut input_hashes = BTreeMap::new();
    input_hashes.insert(audio.display().to_string(), models::hash_file(audio)?);
    if let Some(path) = source_chart {
        input_hashes.insert(path.display().to_string(), models::hash_file(&path)?);
    }
    let pro_keys_output =
        chart_name == "notes.chart" && review_generated.iter().any(|t| crate::keys::is_pro(t));
    let mut report = serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"status":"automatic_draft","output":output,"added":added,"input_sha256":input_hashes,"metadata":meta,"timing":map,"model_catalog":models::assets(),"isolation":transcription.isolation,"isolation_manifest":transcription.isolation.as_ref().map(|_|song_layout::relative("chart-engine-isolation.json")),"difficulty_reduction":difficulty_reduction,"charting_rules":rule_report,"review_markers":song_layout::relative("chart-engine-review.json"),"warnings":warnings,"pro_keys_arrangement":pro_arrangement_summary,"pro_keys_ranges":pro_keys_ranges,"game_chart":if pro_keys_output{"notes.mid"}else{chart_name},"editable_master":if pro_keys_output{Some(song_layout::relative("chart-engine-working").join("notes.chart"))}else{None}});
    if let Some(analysis) = &transcription.keyboard_analysis {
        report["keyboard_analysis"] = serde_json::to_value(analysis)?;
        report["keyboard_analysis_manifest"] = serde_json::json!(song_layout::relative(
            "chart-engine-keyboard-isolation.json"
        ));
    }
    if transcription
        .keyboard_manifest()?
        .is_some_and(higher_quality_manifest)
    {
        report["keyboard_analysis_model"] =
            serde_json::to_value(crate::keyboard_separation::asset())?;
    }
    fs::write(
        song_layout::path(stage.path(), "chart-engine-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    let review_chart = if chart_name == "notes.chart" {
        Chart::parse(fs::read_to_string(stage.path().join(chart_name))?)?
    } else {
        // This representation is diagnostic only; the exported MIDI remains
        // byte-preserving for authored track chunks. Inspect only newly added parts.
        let added_tracks = generated
            .into_iter()
            .filter(|(name, _)| review_generated.contains(name))
            .collect();
        let mut diagnostic = Chart::parse(chart::new_chart(
            &meta.name,
            &meta.artist,
            &map,
            &added_tracks,
        ))?;
        // Keep MIDI's microsecond tempo precision for diagnostics; .chart text
        // rounds BPM to thousandths and is not the actual exported note file.
        diagnostic.timing = map.clone();
        diagnostic
    };
    let review_inputs = BTreeMap::from([
        (
            "chart".into(),
            models::hash_file(&stage.path().join(chart_name))?,
        ),
        (
            "transcription".into(),
            models::hash_file(&song_layout::path(
                stage.path(),
                "chart-engine-transcription.json",
            ))?,
        ),
        (
            "generation_receipt".into(),
            models::hash_file(&song_layout::path(stage.path(), "chart-engine-report.json"))?,
        ),
        (
            "original_audio".into(),
            models::hash_file(&audio_file(stage.path())?)?,
        ),
    ]);
    let review = crate::review::generation_map(
        &review_chart,
        transcription,
        meta.duration,
        review_inputs,
        review_generated,
    )?;
    let mut review = with_generated_rule_reviews(review, &rule_report)?;
    review.coverage_limits.push("Guitar/bass/drum lower difficulties derive from the effective Expert chart (authored Expert takes precedence). Pro Keys lower tiers select subsets of the fitted Expert register arrangement. Five-lane Piano selects source attacks/chord voices independently. Live Expert-master links are not available for Keys/Pro Keys. These reductions do not establish musical acceptance.".into());
    if let Some(isolation) = &transcription.isolation {
        review.coverage_limits.push(format!("Generation used hash-bound instrument stems with alignment method '{}'. Exact source/stem sample counts establish engineering consistency; they do not establish phase alignment, isolation purity, calibrated note confidence or musical acceptance. Existing attack-support, pitch-disagreement and candidate-score flags remain applicable.", isolation.alignment.method));
        if isolation.generation.is_none() {
            review.coverage_limits.push("Isolation was supplied with an explicit provenance attestation; no contemporaneous native model-bound job receipt is available.".into());
        }
    }
    fs::write(
        song_layout::path(stage.path(), "chart-engine-review.json"),
        serde_json::to_vec_pretty(&review)?,
    )?;
    ensure!(
        !fs::exists(output),
        "Output appeared during processing; refusing to overwrite"
    );
    if let Some(isolation) = &transcription.isolation {
        isolation.validate_binding(&models::hash_file(audio)?, meta.duration)?;
    }
    if let Some(analysis) = &transcription.keyboard_analysis {
        analysis.validate_source(audio, &models::hash_file(audio)?, meta.duration)?;
    }
    if pro_keys_output {
        let master = stage.path().join("notes.chart");
        let chart = Chart::parse(fs::read_to_string(&master)?)?;
        let (midi, delay) =
            crate::game_export::song_with_ini(&chart, None, &ini_fields(&song_ini(stage.path())?))?;
        let working = song_layout::path(stage.path(), "chart-engine-working");
        // Existing recovery data must never be overwritten during enhancement.
        ensure!(
            !fs::exists(&working) || fs::read_dir(&working)?.next().is_none(),
            "Existing working-master archive requires opening its YACE project before adding Pro Keys"
        );
        fs::create_dir_all(&working)?;
        fs::write(stage.path().join("notes.mid"), midi)?;
        fs::rename(&master, working.join("notes.chart"))?;
        fs::write(
            working.join("binding.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"version":1,"master_sha256":models::hash_file(&working.join("notes.chart"))?,"midi_sha256":models::hash_file(&stage.path().join("notes.mid"))?}),
            )?,
        )?;
        let ini = stage.path().join("song.ini");
        fs::write(
            &ini,
            crate::game_export::ini_with_delay(&fs::read_to_string(&ini)?, delay),
        )?;
    }
    publish(stage.path(), output)?;
    Ok(report)
}

#[cfg(not(target_arch = "wasm32"))]
fn staging(output: &Path) -> Result<tempfile::TempDir> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(tempfile::Builder::new()
        .prefix(".chart-engine-")
        .tempdir_in(parent)?)
}
#[cfg(target_arch = "wasm32")]
struct Stage(PathBuf);
#[cfg(target_arch = "wasm32")]
impl Stage {
    fn path(&self) -> &Path {
        &self.0
    }
}
#[cfg(target_arch = "wasm32")]
impl Drop for Stage {
    fn drop(&mut self) {
        crate::storage::clear(&self.0);
    }
}
#[cfg(target_arch = "wasm32")]
fn staging(output: &Path) -> Result<Stage> {
    let path = output.with_file_name(format!(".chart-engine-{}", fs::unique_id()));
    fs::create_dir_all(&path)?;
    Ok(Stage(path))
}
#[cfg(not(target_arch = "wasm32"))]
fn convert_audio(audio: &Path, output: &Path) -> Result<()> {
    let encoded = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(audio)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-c:a",
            "pcm_s16le",
            "-ar",
            "44100",
            "-ac",
            "2",
            "-n",
        ])
        .arg(output)
        .output()?;
    ensure!(
        encoded.status.success(),
        "Cannot convert output audio to WAV: {}",
        String::from_utf8_lossy(&encoded.stderr)
    );
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn convert_audio(audio: &Path, output: &Path) -> Result<()> {
    let stereo = models::decode(audio, 44100, 2)?;
    fs::write_new(
        output,
        &crate::audio_decode::Pcm {
            stereo,
            rate: 44100,
            channels: 2,
        }
        .float_wave()?,
    )?;
    Ok(())
}
pub fn lyric_events(lrc: &str, map: &TempoMap, duration: f64) -> String {
    let mut events = Vec::new();
    for line in lrc.lines() {
        let mut rest = line.trim();
        let mut times = Vec::new();
        while let Some(after) = rest.strip_prefix('[') {
            let Some((timestamp, tail)) = after.split_once(']') else {
                break;
            };
            if let Some((minute, second)) = timestamp.split_once(':')
                && let (Ok(minute), Ok(second)) = (minute.parse::<u32>(), second.parse::<f64>())
            {
                let time = minute as f64 * 60.0 + second;
                if (0.0..60.0).contains(&second) && time <= duration {
                    times.push(time);
                }
            }
            rest = tail;
        }
        let body = rest.trim();
        if body.is_empty() {
            continue;
        }
        for time in times {
            if let Some(tick) = map.tick(time) {
                events.push((
                    tick,
                    format!("  {tick} = E \"lyric {}\"\n", chart::quoted(body)),
                ));
            }
        }
    }
    events.sort_by_key(|e| e.0);
    events.into_iter().map(|(_, line)| line).collect()
}
