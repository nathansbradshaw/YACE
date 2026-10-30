//! Review vocal evidence against its recording.
//! Store decisions in the editor's normal review history.
use crate::{
    editor::Project,
    timing::TempoMap,
    vocal_generation::{Generation, Reason},
    vocals::VocalChart,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const TRACK: &str = "Vocals";
fn digest(value: &impl Serialize) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VocalReviewRegion {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub reason: Reason,
    pub detail: String,
    pub issues: Vec<crate::vocal_generation::Issue>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VocalReviewMap {
    pub version: u32,
    pub audio_sha256: String,
    pub vocal_stem_sha256: String,
    pub duration_seconds: f64,
    #[serde(default)]
    pub timing: Option<TempoMap>,
    /// SHA-256 of compact typed serde serialization, including defaulted
    /// legacy fields; this is not the raw pretty-printed sidecar file hash.
    pub report_sha256: String,
    pub generated_chart_sha256: String,
    pub regions: Vec<VocalReviewRegion>,
}
impl VocalReviewMap {
    fn region_id(report: &str, index: usize, region: &VocalReviewRegion) -> Result<String> {
        Ok(format!(
            "vocal-{report}-{index}-{}",
            digest(&(
                region.start,
                region.end,
                region.reason,
                &region.detail,
                &region.issues
            ))?
        ))
    }
    pub fn from_generation(report: &Generation) -> Result<Self> {
        ensure!(report.version == 1, "Unsupported vocal generation report");
        ensure!(
            report.duration_seconds.is_finite()
                && report.duration_seconds > 0.
                && (report.duration_seconds / 12.).ceil() <= 100_000.,
            "Invalid vocal report duration"
        );
        ensure!(report.issues.len() <= 100_000, "Too many vocal issues");
        let chart = report
            .chart
            .as_ref()
            .context("Vocal checks need a generated chart witness")?;
        chart.validate()?;
        let report_sha256 = digest(report)?;
        let mut windows =
            std::collections::BTreeMap::<u32, Vec<crate::vocal_generation::Issue>>::new();
        let mut expanded = 0usize;
        for issue in &report.issues {
            ensure!(
                issue.start_seconds.is_finite()
                    && issue.end_seconds.is_finite()
                    && issue.start_seconds >= 0.
                    && issue.start_seconds < report.duration_seconds
                    && issue.end_seconds >= issue.start_seconds
                    && issue.end_seconds <= report.duration_seconds,
                "Invalid vocal issue range"
            );
            let first = (issue.start_seconds / 12.).floor() as u32;
            let last = if issue.end_seconds > issue.start_seconds {
                ((issue.end_seconds / 12.).ceil() as u32).saturating_sub(1)
            } else {
                first
            };
            for window in first..=last.max(first) {
                expanded += 1;
                ensure!(
                    expanded <= 1_000_000,
                    "Too much expanded vocal review evidence"
                );
                windows.entry(window).or_default().push(issue.clone());
            }
        }
        let regions = windows
            .into_iter()
            .enumerate()
            .map(|(index, (window, issues))| {
                let primary = &issues[0];
                let mut region = VocalReviewRegion {
                    id: String::new(),
                    start: f64::from(window) * 12.,
                    end: (f64::from(window + 1) * 12.).min(report.duration_seconds),
                    reason: primary.reason,
                    detail: primary.detail.clone(),
                    issues,
                };
                region.id = Self::region_id(&report_sha256, index, &region)?;
                Ok(region)
            })
            .collect::<Result<Vec<_>>>()?;
        let map = Self {
            version: 1,
            audio_sha256: report.audio_sha256.clone(),
            vocal_stem_sha256: report.vocal_stem_sha256.clone(),
            duration_seconds: report.duration_seconds,
            timing: report.timing.clone(),
            report_sha256,
            generated_chart_sha256: digest(chart)?,
            regions,
        };
        map.validate()?;
        Ok(map)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && self.duration_seconds.is_finite() && self.duration_seconds > 0.,
            "Invalid vocal review version or duration"
        );
        ensure!(
            [
                &self.audio_sha256,
                &self.vocal_stem_sha256,
                &self.report_sha256,
                &self.generated_chart_sha256
            ]
            .iter()
            .all(|hash| valid_hash(hash)),
            "Invalid vocal review fingerprint"
        );
        ensure!(self.regions.len() <= 100_000, "Too many vocal checks");
        let mut ids = std::collections::BTreeSet::new();
        for (index, region) in self.regions.iter().enumerate() {
            ensure!(
                region.id == Self::region_id(&self.report_sha256, index, region)?
                    && ids.insert(&region.id),
                "Invalid vocal review region identity"
            );
            ensure!(
                region.start.is_finite()
                    && region.end.is_finite()
                    && region.start >= 0.
                    && region.end > region.start
                    && region.end <= self.duration_seconds
                    && region.detail.len() <= 100_000,
                "Invalid vocal review region"
            );
            ensure!(
                !region.issues.is_empty()
                    && region.issues.len() <= 100_000
                    && region.reason == region.issues[0].reason
                    && region.detail == region.issues[0].detail,
                "Invalid vocal review evidence"
            );
            for issue in &region.issues {
                ensure!(
                    issue.start_seconds.is_finite()
                        && issue.end_seconds.is_finite()
                        && issue.start_seconds >= 0.
                        && issue.end_seconds >= issue.start_seconds
                        && issue.end_seconds <= self.duration_seconds
                        && issue.detail.len() <= 100_000,
                    "Invalid vocal issue evidence"
                );
                let overlaps = if issue.start_seconds == issue.end_seconds {
                    (region.start..region.end).contains(&issue.start_seconds)
                } else {
                    issue.end_seconds > region.start && issue.start_seconds < region.end
                };
                ensure!(overlaps, "Vocal issue is outside its review section");
            }
        }
        Ok(())
    }
    pub fn validate_binding(&self, project: &Project) -> Result<()> {
        ensure!(
            self.audio_sha256
                .eq_ignore_ascii_case(&project.audio_sha256)
                && (self.duration_seconds - project.duration).abs() <= 0.1,
            "Vocal checks belong to another recording"
        );
        if let Some(manifest) = &project.content.stems {
            let vocal = manifest
                .stems
                .iter()
                .find(|stem| stem.id == crate::stem_review::StemId::Vocals)
                .context("Attached stems have no matching vocal stem")?;
            ensure!(
                vocal.sha256.eq_ignore_ascii_case(&self.vocal_stem_sha256),
                "Vocal checks describe a different vocal stem"
            );
        }
        Ok(())
    }
    pub fn validate_timing(&self, timing: &TempoMap, automated: bool) -> Result<()> {
        if let Some(recorded) = &self.timing {
            ensure!(
                recorded == timing,
                "Vocal review tempo map differs from this project"
            );
        } else {
            ensure!(
                !automated,
                "Automated vocal generation requires an exact tempo-map witness; explicitly adopt legacy checks instead"
            );
        }
        Ok(())
    }
    pub fn validate_generated_chart(&self, document: &VocalChart) -> Result<()> {
        ensure!(
            digest(document)? == self.generated_chart_sha256,
            "Vocal chart does not match its generation checks"
        );
        Ok(())
    }
    pub fn region(&self, id: &str) -> Option<&VocalReviewRegion> {
        self.regions.iter().find(|r| r.id == id)
    }
    pub fn fingerprint(&self, project: &Project, id: &str, timing: &TempoMap) -> Result<String> {
        self.validate_binding(project)?;
        self.validate_timing(timing, false)?;
        let region = self.region(id).context("Unknown vocal review region")?;
        let chart = project
            .content
            .vocals
            .as_ref()
            .context("No vocal chart to review")?;
        let mut indices: std::collections::BTreeSet<_> = chart
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                timing.seconds(n.end) >= region.start - 0.1
                    && timing.seconds(n.start) <= region.end + 0.1
            })
            .map(|(index, _)| index)
            .collect();
        include_lyric_runs(chart, &mut indices);
        let notes: Vec<_> = indices
            .into_iter()
            .map(|index| &chart.notes[index])
            .collect();
        let phrases: Vec<_> = chart
            .phrases
            .iter()
            .filter(|p| {
                timing.seconds(p.end) >= region.start - 0.1
                    && timing.seconds(p.start) <= region.end + 0.1
            })
            .collect();
        digest(&(
            1,
            TRACK,
            &self.audio_sha256,
            &self.vocal_stem_sha256,
            &self.report_sha256,
            region,
            timing,
            notes,
            phrases,
        ))
    }
}

/// A visible syllable depends on its whole continued/hyphenated word, even
/// when its owner or ending lies outside this review window. Separate words
/// remain independent; a lyric run never crosses a scoring phrase boundary.
fn include_lyric_runs(chart: &VocalChart, indices: &mut std::collections::BTreeSet<usize>) {
    let selected = indices.clone();
    let include = |run: &[usize], indices: &mut std::collections::BTreeSet<usize>| {
        if run.iter().any(|index| selected.contains(index)) {
            indices.extend(run.iter().copied());
        }
    };
    for phrase in &chart.phrases {
        let mut run = Vec::new();
        let mut join_next = false;
        for (index, note) in chart
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.start >= phrase.start && n.end <= phrase.end)
        {
            if note.lyric == crate::vocals::Syllable::Missing {
                include(&run, indices);
                run.clear();
                join_next = false;
                include(&[index], indices);
                continue;
            }
            // Intentional slides and independent ContinueWord contours both
            // retain this word owner, even outside the review section.
            if let crate::vocals::Syllable::Text {
                join_next: join, ..
            } = &note.lyric
            {
                if !join_next {
                    include(&run, indices);
                    run.clear();
                }
                join_next = *join;
            }
            run.push(index);
        }
        include(&run, indices);
    }
}
