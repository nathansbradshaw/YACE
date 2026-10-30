//! Reproducible, disposable bass-onset preview drafts. Never musical acceptance.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    chart::{Chart, Note},
    editor::{Action, Actor, ActorKind, Request, Store},
    models, pipeline,
    stem_coverage::{CoverageReport, safe_bass_attack_move},
    stem_review::StemId,
};
use clap::Parser;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(about = "Create a separate, unapproved bass-onset preview project and exported song")]
struct Args {
    /// Immutable exported song folder; never a live editor project.
    #[arg(long)]
    song: PathBuf,
    #[arg(long)]
    coverage: PathBuf,
    /// Required when the coverage report binds a transcription file.
    #[arg(long)]
    transcription: Option<PathBuf>,
    /// New bundle directory containing project/, song/ and receipts.
    #[arg(long)]
    output: PathBuf,
}
#[derive(Clone, Debug, Serialize)]
struct Move {
    before_tick: u64,
    proposed_tick: u64,
    before_seconds: f64,
    proposed_seconds: f64,
    spectral_peak_seconds: f64,
    before_group: Vec<Note>,
    proposed_group: Vec<Note>,
}
#[derive(Clone, Debug, Serialize)]
struct Rejected {
    before_tick: u64,
    proposed_tick: u64,
    reason: String,
}
fn binding(source: &Path, chart: &Path, expected_audio: &str, expected_chart: &str) -> Result<()> {
    ensure!(
        models::hash_file(source)?.eq_ignore_ascii_case(expected_audio),
        "Coverage belongs to another or changed recording"
    );
    ensure!(
        models::hash_file(chart)?.eq_ignore_ascii_case(expected_chart),
        "Coverage belongs to another or changed chart"
    );
    Ok(())
}
fn apply(store: &Store, action: Action) -> Result<()> {
    let current = store.load()?;
    store.apply(Request {
        expected_revision: current.revision,
        actor: Actor {
            kind: ActorKind::Ai,
            name: "Deterministic bass-onset preview draft; no musical approval".into(),
        },
        action,
    })?;
    Ok(())
}
fn validate_saved_review_binding(folder: &Path) -> Result<()> {
    let report: CoverageReport =
        serde_json::from_slice(&fs::read(folder.join("coverage-geometry-validated.json"))?)?;
    let markers: chart_engine::review_regions::ReviewMap =
        serde_json::from_slice(&fs::read(folder.join("baseline-review.json"))?)?;
    let semantic = format!("{:x}", Sha256::digest(serde_json::to_vec(&report)?));
    ensure!(
        markers.input_sha256.get("stem_coverage_semantic") == Some(&semantic),
        "Saved review flags do not bind the saved geometry report"
    );
    ensure!(
        markers.input_sha256.get("stem_coverage_baseline_file")
            == Some(&models::hash_file(&folder.join("coverage-baseline.json"))?),
        "Saved baseline coverage bytes do not match review provenance"
    );
    Ok(())
}
fn run(args: Args) -> Result<PathBuf> {
    ensure!(
        !args.song.join("project.json").exists(),
        "Supply an immutable exported song, not a live project"
    );
    pipeline::destination(&args.output, Some(&args.song))?;
    let song = args.song.canonicalize()?;
    let audio = pipeline::audio_file(&song)?;
    let chart_path = pipeline::chart_file(&song)?;
    ensure!(
        chart_path
            .extension()
            .is_some_and(|extension| extension == "chart"),
        "Bass preview drafts currently require notes.chart"
    );
    let coverage_hash = models::hash_file(&args.coverage)?;
    let mut report: CoverageReport = serde_json::from_slice(&fs::read(&args.coverage)?)?;
    ensure!(report.version == 1, "Unsupported coverage version");
    report.policy.validate()?;
    binding(
        &audio,
        &chart_path,
        &report.source_audio_sha256,
        &report.chart_sha256,
    )?;
    match (&report.transcription_sha256, &args.transcription) {
        (Some(expected), Some(path)) => ensure!(
            models::hash_file(path)?.eq_ignore_ascii_case(expected),
            "Coverage transcription changed"
        ),
        (Some(_), None) => anyhow::bail!("Coverage binds transcription; supply --transcription"),
        (None, Some(_)) => {
            anyhow::bail!("This coverage does not bind a transcription; omit --transcription")
        }
        (None, None) => {}
    }
    drop(report.manifest.load(&audio)?);
    let baseline = Chart::parse(fs::read_to_string(&chart_path)?)?;
    let mut moves = Vec::new();
    let mut rejected = Vec::new();
    let bass = report
        .tracks
        .iter_mut()
        .find(|track| track.stem == StemId::Bass)
        .context("Coverage contains no bass analysis")?;
    ensure!(
        bass.expert_track == "ExpertDoubleBass",
        "Coverage bass track is not Expert"
    );
    let mut seen = BTreeSet::new();
    for observation in &mut bass.alignment {
        let Some(after) = observation.proposed_attack_tick else {
            continue;
        };
        ensure!(
            seen.insert(observation.tick),
            "Coverage contains duplicate attack proposals"
        );
        let validity = (|| -> Result<Vec<Note>> {
            ensure!(
                observation.review_required
                    && observation.supported_attack
                    && observation.periodic_pitch.is_some(),
                "Proposal lacks required onset/pitch review evidence"
            );
            ensure!(
                observation
                    .audio_minus_chart_seconds
                    .is_some_and(|value| value.is_finite() && value.abs() > 0.05),
                "Proposal has no supported timing mismatch"
            );
            ensure!(
                observation
                    .periodic_minus_raw_semitones
                    .is_some_and(|value| value.is_finite() && value.abs() <= 1.),
                "Pitch mismatch remains unresolved; onset-only draft will not guess"
            );
            let peak = observation
                .nearest_flux_seconds
                .context("Proposal has no measured flux timestamp")?;
            ensure!(
                baseline.timing.tick(peak) == Some(after),
                "Proposed tick is not the reported spectral peak"
            );
            ensure!(
                (baseline.timing.seconds(observation.tick) - observation.seconds).abs() < 0.000001,
                "Proposal source time/tick disagree"
            );
            safe_bass_attack_move(&baseline, observation.tick, after)
        })();
        match validity {
            Ok(group) => moves.push(Move {
                before_tick: observation.tick,
                proposed_tick: after,
                before_seconds: observation.seconds,
                proposed_seconds: baseline.timing.seconds(after),
                spectral_peak_seconds: observation.nearest_flux_seconds.unwrap(),
                before_group: baseline
                    .notes("ExpertDoubleBass")?
                    .into_iter()
                    .filter(|note| note.tick == observation.tick)
                    .collect(),
                proposed_group: group,
            }),
            Err(error) => {
                rejected.push(Rejected {
                    before_tick: observation.tick,
                    proposed_tick: after,
                    reason: error.to_string(),
                });
                observation.proposed_attack_tick = None;
            }
        }
    }
    ensure!(
        !moves.is_empty(),
        "No safe onset preview proposals; refusing to create an empty draft"
    );
    moves.sort_by_key(|proposal| proposal.before_tick);
    let duration = report.manifest.frames as f64 / report.manifest.sample_rate as f64;
    report.limits.push("Attack proposals were revalidated against complete same-tick groups, preceding same-lane sustain ends and nearby attacks. Removed proposals remain uncorrected; timing/pitch measurements are the unchanged baseline, not remeasured draft results.".into());
    let mut markers = chart_engine::stem_coverage_review::review_map_with_inputs(
        &report,
        duration,
        std::collections::BTreeMap::from([(
            "stem_coverage_baseline_file".into(),
            coverage_hash.clone(),
        )]),
    )?;
    let bass_tracks: BTreeSet<String> = [
        "EasyDoubleBass",
        "MediumDoubleBass",
        "HardDoubleBass",
        "ExpertDoubleBass",
    ]
    .into_iter()
    .filter(|track| {
        baseline
            .sections
            .iter()
            .any(|section| section.name == *track)
    })
    .map(str::to_owned)
    .collect();
    markers.coverage_limits.push("This disposable preview explicitly regenerates lower bass difficulties from the copied Expert attack arrangement. That policy does not establish that the source was automatically authored or musically accepted. Baseline notes remain preserved in project/source.".into());
    let parent = args
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stage = tempfile::tempdir_in(parent)?;
    let markers_path = stage.path().join("baseline-review.json");
    fs::write(&markers_path, serde_json::to_vec_pretty(&markers)?)?;
    fs::copy(&args.coverage, stage.path().join("coverage-baseline.json"))?;
    fs::write(
        stage.path().join("coverage-geometry-validated.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    let store = Store::create(&song, Some(&markers_path), &stage.path().join("project"))?;
    let bound_stems = stage.path().join("bound-stems.json");
    fs::write(&bound_stems, serde_json::to_vec_pretty(&report.manifest)?)?;
    if store.load()?.content.stems.as_ref() != Some(&report.manifest) {
        apply(
            &store,
            Action::AttachStems {
                manifest: bound_stems,
            },
        )?;
    }
    for track in ["EasyDoubleBass", "MediumDoubleBass", "HardDoubleBass"] {
        if bass_tracks.contains(track) {
            apply(
                &store,
                Action::EnableDifficultyLink {
                    track: track.into(),
                    enabled: true,
                },
            )?;
        }
    }
    for proposal in &moves {
        let current = store.load()?;
        let current_chart = Chart::parse(current.chart_text(&store)?)?;
        let expected =
            safe_bass_attack_move(&current_chart, proposal.before_tick, proposal.proposed_tick)?;
        ensure!(
            expected == proposal.proposed_group,
            "Attack group changed while staging draft"
        );
        let notes = current
            .content
            .tracks
            .get("ExpertDoubleBass")
            .context("Expert bass disappeared")?;
        let playable: Vec<_> = notes
            .iter()
            .filter(|note| {
                note.note.tick == proposal.before_tick
                    && (note.note.lane <= 4 || note.note.lane == 7)
            })
            .cloned()
            .collect();
        for note in playable {
            let replacement = proposal
                .proposed_group
                .iter()
                .find(|moved| moved.lane == note.note.lane)
                .context("Moved attack lost a lane")?;
            apply(
                &store,
                Action::UpdateNote {
                    note_id: note.id,
                    tick: replacement.tick,
                    lane: replacement.lane,
                    sustain: replacement.sustain,
                    cymbal: false,
                },
            )?;
        }
        let after = store.load()?;
        let mut actual: Vec<_> = after.content.tracks["ExpertDoubleBass"]
            .iter()
            .filter(|note| note.note.tick == proposal.proposed_tick)
            .map(|note| note.note.clone())
            .collect();
        actual.sort();
        let mut expected = proposal.proposed_group.clone();
        expected.sort();
        ensure!(
            actual == expected,
            "Entire attack/control group was not preserved"
        );
    }
    let final_doc = store.load()?;
    ensure!(
        final_doc.content.decisions.is_empty(),
        "Preview draft cannot contain musical approvals"
    );
    let revised_text = final_doc.chart_text(&store)?;
    let revised = Chart::parse(revised_text.clone())?;
    for section in baseline
        .sections
        .iter()
        .filter(|section| !section.name.ends_with("DoubleBass"))
    {
        let other = revised
            .sections
            .iter()
            .find(|other| other.name == section.name)
            .context("A protected section disappeared")?;
        ensure!(
            baseline.raw[section.body_start..section.body_end]
                == revised.raw[other.body_start..other.body_end],
            "Protected guitar/drum/metadata section changed: {}",
            section.name
        );
    }
    apply(
        &store,
        Action::Export {
            output: stage.path().join("song"),
        },
    )?;
    binding(
        &audio,
        &chart_path,
        &report.source_audio_sha256,
        &report.chart_sha256,
    )?;
    ensure!(
        models::hash_file(&args.coverage)? == coverage_hash,
        "Coverage report changed while staging draft"
    );
    if let (Some(expected), Some(path)) = (&report.transcription_sha256, &args.transcription) {
        ensure!(
            models::hash_file(path)?.eq_ignore_ascii_case(expected),
            "Transcription changed while staging"
        );
    }
    for asset in &report.manifest.stems {
        ensure!(
            models::hash_file(&asset.path)?.eq_ignore_ascii_case(&asset.sha256),
            "Stem changed while staging"
        );
    }
    let receipt = serde_json::json!({"version":1,"status":"unapproved_onset_preview_draft","source_song":song,"source_audio_sha256":report.source_audio_sha256,"baseline_chart_sha256":report.chart_sha256,"baseline_coverage_sha256":coverage_hash,"transcription_sha256":report.transcription_sha256,"revised_chart_sha256":models::hash_file(&stage.path().join("song/notes.chart"))?,"project_revision":final_doc.revision,"moves":moves,"rejected_moves":rejected,"lower_bass_regeneration":"Explicitly enabled only in this disposable project; ancestry and original notes retained. Other instruments protected byte-for-byte.","musical_approvals":0,"pitch_changes":0,"global_offset_changes":0,"baseline_timing":report.tracks.iter().find(|track|track.stem==StemId::Bass).map(|track|&track.timing),"after_metrics":"Not independently remeasured; no before/after improvement claim. Old source-bound review remains pending and becomes stale after edits. Run a fresh coverage audit of the exported draft.","limits":["These are review proposals, not safe generation defaults. Centered spectral windows can lead physical attacks; stable fundamental support does not prove the chosen onset belongs to the mapped note.","Review isolated bass and waveform before accepting. No preceding hold is trimmed; each moved sustain retains its original END, and whole attack/control groups move together.","Original recordings, the immutable input snapshot and live human project are unchanged. The bundle is atomically published without replacing any existing output."]});
    fs::write(
        stage.path().join("draft-receipt.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    validate_saved_review_binding(stage.path())?;
    pipeline::publish(stage.path(), &args.output)?;
    args.output
        .canonicalize()
        .context("Published draft unavailable")
}
fn main() -> Result<()> {
    println!("{}", run(Args::parse())?.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_source_or_chart_binding_fails_before_output_creation() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("audio");
        let chart = dir.path().join("chart");
        fs::write(&source, b"recording").unwrap();
        fs::write(&chart, b"notes").unwrap();
        let audio_hash = models::hash_file(&source).unwrap();
        let chart_hash = models::hash_file(&chart).unwrap();
        binding(&source, &chart, &audio_hash, &chart_hash).unwrap();
        fs::write(&source, b"different recording").unwrap();
        assert!(binding(&source, &chart, &audio_hash, &chart_hash).is_err());
        fs::write(&source, b"recording").unwrap();
        fs::write(&chart, b"different notes").unwrap();
        assert!(binding(&source, &chart, &audio_hash, &chart_hash).is_err());
        assert!(!dir.path().join("output").exists());
    }
    #[test]
    fn saved_review_binding_detects_post_marker_report_mutation() {
        use chart_engine::{
            stem_coverage::Policy,
            stem_review::{AlignmentEvidence, StemAsset, StemManifest},
        };
        use std::collections::BTreeMap;
        let dir = tempfile::tempdir().unwrap();
        let mut report = CoverageReport {
            version: 1,
            source_audio_sha256: "11".repeat(32),
            chart_sha256: "22".repeat(32),
            transcription_sha256: None,
            manifest: StemManifest {
                version: 1,
                source_audio_sha256: "11".repeat(32),
                sample_rate: 44100,
                channels: 2,
                frames: 4 * 44100,
                alignment: AlignmentEvidence {
                    method: "synthetic_fixture".into(),
                    details: "Source identity fixture; no audio or musical acceptance claimed."
                        .into(),
                },
                stems: vec![StemAsset {
                    id: StemId::Bass,
                    path: PathBuf::from("/tmp/original-bass-binding-fixture.wav"),
                    sha256: "33".repeat(32),
                }],
                generation: None,
            },
            policy: Policy::default(),
            tracks: vec![],
            limits: vec!["Geometry checked; timing remains baseline evidence.".into()],
        };
        let baseline = dir.path().join("coverage-baseline.json");
        fs::write(&baseline, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        let markers = chart_engine::stem_coverage_review::review_map_with_inputs(
            &report,
            4.,
            BTreeMap::from([(
                "stem_coverage_baseline_file".into(),
                models::hash_file(&baseline).unwrap(),
            )]),
        )
        .unwrap();
        fs::write(
            dir.path().join("baseline-review.json"),
            serde_json::to_vec_pretty(&markers).unwrap(),
        )
        .unwrap();
        let geometry = dir.path().join("coverage-geometry-validated.json");
        fs::write(&geometry, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        validate_saved_review_binding(dir.path()).unwrap();
        report
            .limits
            .push("Added after marker identity was calculated".into());
        fs::write(&geometry, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        assert!(validate_saved_review_binding(dir.path()).is_err());
        if let Some(path) = std::env::var_os("CHART_ENGINE_DRAFT_BUNDLE") {
            let path = PathBuf::from(path);
            validate_saved_review_binding(&path).unwrap();
            println!(
                "Verified actual draft report/review binding: {}",
                path.display()
            );
        }
    }
    fn fixture(extra: Option<Note>) -> Chart {
        let timing = chart_engine::timing::TempoMap::constant(120.).unwrap();
        let mut notes = vec![
            Note {
                tick: 960,
                lane: 0,
                sustain: 240,
            },
            Note {
                tick: 960,
                lane: 2,
                sustain: 120,
            },
            Note {
                tick: 960,
                lane: 5,
                sustain: 0,
            },
        ];
        if let Some(extra) = extra {
            notes.push(extra);
        }
        Chart::parse(chart_engine::chart::new_chart(
            "Original fixture",
            "Test",
            &timing,
            &std::collections::BTreeMap::from([("ExpertDoubleBass".into(), notes)]),
        ))
        .unwrap()
    }
    #[test]
    fn entire_chord_and_control_move_preserves_each_sustain_end() {
        let chart = fixture(None);
        let moved = safe_bass_attack_move(&chart, 960, 900).unwrap();
        assert_eq!(moved.len(), 3);
        assert!(moved.iter().all(|note| note.tick == 900));
        assert_eq!(
            moved.iter().find(|note| note.lane == 0).unwrap().sustain,
            300
        );
        assert_eq!(
            moved.iter().find(|note| note.lane == 2).unwrap().sustain,
            180
        );
        assert_eq!(moved.iter().find(|note| note.lane == 5).unwrap().sustain, 0);
        assert_eq!(
            chart
                .notes("ExpertDoubleBass")
                .unwrap()
                .iter()
                .find(|note| note.lane == 0)
                .unwrap()
                .sustain,
            240
        );
    }
    #[test]
    fn preceding_hold_and_next_attack_geometry_are_not_auto_trimmed() {
        assert!(
            safe_bass_attack_move(
                &fixture(Some(Note {
                    tick: 700,
                    lane: 0,
                    sustain: 230
                })),
                960,
                900
            )
            .is_err()
        );
        assert!(
            safe_bass_attack_move(
                &fixture(Some(Note {
                    tick: 1100,
                    lane: 0,
                    sustain: 0
                })),
                960,
                900
            )
            .is_err()
        );
        assert!(safe_bass_attack_move(&fixture(None), 960, 1300).is_err());
    }
}
