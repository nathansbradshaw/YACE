//! Check project structure on creation and load.
//! These checks catch invalid edits and mismatched evidence; listening decides
//! whether the chart follows the music.
use crate::{
    chart::Chart,
    editor::{Project, playable},
};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Component};

fn supported_track(name: &str) -> bool {
    ["Easy", "Medium", "Hard", "Expert"].iter().any(|tier| {
        ["Single", "DoubleBass", "Drums", "Keyboard", "ProKeys"]
            .iter()
            .any(|part| name == format!("{tier}{part}"))
    })
}

fn sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// New sections may only be one-way lower-difficulty derivatives of an
/// immutable source Expert section. Pausing a valid link keeps its notes.
pub(crate) fn is_link_created_track(project: &Project, chart: &Chart, track: &str) -> bool {
    let Some(suffix) = ["Easy", "Medium", "Hard"]
        .into_iter()
        .find_map(|tier| track.strip_prefix(tier))
    else {
        return false;
    };
    if !matches!(suffix, "Single" | "DoubleBass" | "Drums") {
        return false;
    }
    let expert = format!("Expert{suffix}");
    (chart.sections.iter().any(|section| section.name == expert)
        || project.content.track_origins.contains_key(&expert))
        && project.content.tracks.contains_key(&expert)
        && project
            .content
            .difficulty_links
            .links
            .get(track)
            .is_some_and(|link| {
                link.expert_track == expert && link.rule_id == crate::difficulty_links::RULE_ID
            })
}

pub fn validate(project: &Project, chart: &Chart) -> Result<()> {
    let timing = crate::sync_track::effective(project, &chart.timing)?;
    if let Some(review) = project
        .content
        .sync
        .as_ref()
        .and_then(|sync| sync.archived_vocal_review.as_ref())
    {
        review.validate()?;
        ensure!(
            review
                .audio_sha256
                .eq_ignore_ascii_case(&project.audio_sha256)
                && (review.duration_seconds - project.duration).abs() <= 0.1,
            "Archived vocal checks belong to another recording"
        );
    }
    if let Some(markers) = &project.content.sections {
        crate::sections::validate(markers, &timing, project.duration)?;
    }
    ensure!(project.version == 1, "Unsupported editor project version");
    ensure!(
        project.duration.is_finite() && project.duration > 0.0,
        "Invalid project duration"
    );
    for manifest in project
        .content
        .stems
        .iter()
        .chain(project.content.keyboard_stems.iter())
    {
        manifest.validate_binding(&project.audio_sha256, project.duration)?;
    }
    project
        .content
        .stem_mix
        .prepare(project.content.stem_ids())?;
    ensure!(
        sha256(&project.source_chart_sha256)
            && project.source_chart_sha256 == format!("{:x}", Sha256::digest(chart.raw.as_bytes()))
            && sha256(&project.audio_sha256),
        "Invalid project input fingerprint"
    );
    let mut components = std::path::Path::new(&project.audio_filename).components();
    ensure!(
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none(),
        "Invalid audio filename"
    );

    let tracks: BTreeSet<_> = project.content.tracks.keys().map(String::as_str).collect();
    if let Some(review) = &project.content.vocal_review {
        review.validate()?;
        review.validate_timing(&timing, false)?;
        ensure!(
            review
                .audio_sha256
                .eq_ignore_ascii_case(&project.audio_sha256)
                && (review.duration_seconds - project.duration).abs() <= 0.1,
            "Vocal review recording mismatch"
        );
        ensure!(
            project.content.vocals.is_some(),
            "Vocal review requires a vocal chart"
        );
    }
    if let Some(vocals) = &project.content.vocals {
        vocals.validate()?;
        ensure!(
            vocals.resolution == timing.resolution,
            "Vocal/project resolution mismatch"
        );
        ensure!(
            vocals
                .notes
                .iter()
                .all(|note| timing.seconds(note.end) <= project.duration + 0.1),
            "Vocal note extends beyond the recording"
        );
        ensure!(
            vocals
                .phrases
                .iter()
                .all(|phrase| timing.seconds(phrase.end) <= project.duration + 0.1),
            "Vocal phrase extends beyond the recording"
        );
    }
    ensure!(
        project.content.track_origins.keys().all(|track| {
            if track == "Vocals" {
                project.content.vocals.is_some()
                    || matches!(
                        project.content.track_origins.get(track),
                        Some(crate::parts::Origin::Authored)
                    )
            } else {
                supported_track(track) && project.content.tracks.contains_key(track)
            }
        }),
        "Invalid added-part registry"
    );
    let source_tracks: BTreeSet<_> = chart
        .sections
        .iter()
        .filter(|section| supported_track(&section.name))
        .map(|section| section.name.as_str())
        .collect();
    ensure!(
        !tracks.is_empty()
            && source_tracks.is_subset(&tracks)
            && tracks
                .difference(&source_tracks)
                .all(|track| is_link_created_track(project, chart, track)
                    || (supported_track(track)
                        && project.content.track_origins.contains_key(*track))),
        "Project tracks do not match editable source sections"
    );
    let mut note_ids = BTreeSet::new();
    for (track, notes) in &project.content.tracks {
        let mut positions = BTreeSet::new();
        for item in notes {
            ensure!(
                !item.id.is_empty() && note_ids.insert(item.id.as_str()),
                "Empty or duplicate note ID"
            );
            ensure!(
                positions.insert((item.note.tick, item.note.lane)),
                "Duplicate lane at the same tick in {track}"
            );
            let end_tick = item
                .note
                .tick
                .checked_add(item.note.sustain)
                .context("Note tick overflow")?;
            if playable(track, item.note.lane) {
                let end = timing.seconds(end_tick);
                ensure!(
                    end.is_finite() && end <= project.duration + 0.1,
                    "Note extends beyond recording"
                );
            }
            // All source control lanes remain representable. Unknown lanes
            // must not be silently stripped merely because the UI hides them.
        }
    }
    ensure!(
        project.selected_track == "Vocals" || tracks.contains(project.selected_track.as_str()),
        "Selected track does not exist"
    );
    if let Some(id) = &project.selected_note {
        let (track, item) = project
            .find_note(id)
            .context("Selected note does not exist")?;
        ensure!(
            track == project.selected_track && playable(track, item.note.lane),
            "Selected note is outside the active playable track"
        );
    }

    let transport = &project.transport;
    ensure!(
        transport.playback_rate.is_finite() && (0.5..=1.5).contains(&transport.playback_rate),
        "Invalid practice playback rate"
    );
    ensure!(
        transport.position.is_finite() && (0.0..=project.duration).contains(&transport.position),
        "Invalid transport position"
    );
    if let Some(range) = transport.loop_range {
        ensure!(
            range.start.is_finite()
                && range.end.is_finite()
                && range.start >= 0.0
                && range.end > range.start
                && range.end <= project.duration,
            "Invalid loop range"
        );
    }

    let review = &project.review;
    ensure!(
        review.version == 1
            && review.duration_seconds.is_finite()
            && (review.duration_seconds - project.duration).abs() <= 0.1,
        "Review markers do not match project duration/version"
    );
    ensure!(
        review.input_sha256.get("chart") == Some(&project.source_chart_sha256)
            && review.input_sha256.get("original_audio") == Some(&project.audio_sha256),
        "Review markers do not match immutable source inputs"
    );
    ensure!(
        review.policy.window_seconds.is_finite()
            && review.policy.window_seconds >= 1.0
            && review.policy.low_candidate_score_below.is_finite()
            && (0.0..=1.0).contains(&review.policy.low_candidate_score_below),
        "Invalid review policy"
    );
    if let Some(generated) = &review.generated_tracks {
        ensure!(
            generated
                .iter()
                .all(|track| tracks.contains(track.as_str())),
            "Generated-track provenance names an absent track"
        );
    }
    // Old scopes may refer to superseded region IDs. They remain inert history,
    // never approval, and is_flag_hidden only matches currently present scopes.
    for scope in &project.content.hidden_flags {
        ensure!(
            !scope.region_id.trim().is_empty()
                && (tracks.contains(scope.track.as_str())
                    || scope.track == crate::vocal_review::TRACK),
            "Invalid hidden flag scope"
        );
    }
    let mut region_ids = BTreeSet::new();
    for region in &review.regions {
        ensure!(
            !region.id.is_empty() && region_ids.insert(region.id.as_str()),
            "Empty or duplicate review region ID"
        );
        ensure!(
            region.start.is_finite()
                && region.end.is_finite()
                && region.start >= 0.0
                && region.end > region.start
                && region.end <= project.duration,
            "Invalid review region range"
        );
        ensure!(
            !region.issues.is_empty()
                && !region.tracks.is_empty()
                && region
                    .tracks
                    .iter()
                    .all(|track| tracks.contains(track.as_str())),
            "Review region has no issues or names an absent track"
        );
        let issue_tracks: BTreeSet<_> = region.issues.iter().map(|i| i.track.as_str()).collect();
        ensure!(
            issue_tracks == region.tracks.iter().map(String::as_str).collect(),
            "Review region track scope does not match its issues"
        );
        for issue in &region.issues {
            ensure!(
                issue.start.is_finite()
                    && issue.end.is_finite()
                    && issue.start >= 0.0
                    && issue.end >= issue.start
                    && issue.end <= project.duration
                    && issue.evidence.values().all(|value| value.is_finite()),
                "Invalid timed review issue"
            );
            // A long issue may span several bounded review windows. Point
            // issues at a window boundary belong to the following window.
            let overlaps = if issue.start == issue.end {
                (region.start..region.end).contains(&issue.start)
            } else {
                issue.end > region.start && issue.start < region.end
            };
            ensure!(overlaps, "Review issue does not overlap its region");
        }
    }
    if let Some(id) = &project.selected_region {
        if project
            .content
            .vocal_review
            .as_ref()
            .is_some_and(|map| map.region(id).is_some())
        {
            return Ok(());
        }
        let region = review
            .regions
            .iter()
            .find(|region| &region.id == id)
            .context("Selected review region does not exist")?;
        ensure!(
            region.tracks.contains(&project.selected_track),
            "Selected review region is outside the active track"
        );
    }
    Ok(())
}
