//! Match supplied lyrics against a word policy and build editable clean-audio plans.
//! Speech recognition and model loading happen outside this module.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::{fs, process::Command};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WordTimings {
    pub version: u32,
    pub audio_sha256: String,
    /// Decoded PCM duration, not an MP3 container's padded duration.
    pub duration_seconds: f64,
    pub source: String,
    pub words: Vec<AlignedWord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlignedWord {
    /// null means unknown/unrecognized, never a guessed word.
    pub text: Option<String>,
    /// Both null means timing is unknown. A line timestamp is not word timing.
    pub start: Option<f64>,
    pub end: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanPolicy {
    pub version: u32,
    pub name: String,
    /// Editable exact words/phrases; defaults are a curated English preset.
    pub terms: Vec<String>,
    pub padding_before_seconds: f64,
    pub padding_after_seconds: f64,
    /// Fades extend outside the fully attenuated selected ranges.
    pub fade_seconds: f64,
}

/// Practical starting points for explicit-lyric edits, not an official industry
/// dictionary. List inflections explicitly because matching never uses stems.
/// Trailing apostrophes are punctuation, so `fuckin` also matches `fuckin'`.
const COMMON_EXPLICIT_TERMS: &[&str] = &[
    "ass",
    "asses",
    "asshole",
    "assholes",
    "arse",
    "arsehole",
    "arseholes",
    "bastard",
    "bastards",
    "bitch",
    "bitches",
    "bitching",
    "bitchin",
    "bullshit",
    "bullshitting",
    "bullshittin",
    "cock",
    "cocks",
    "cocksucker",
    "cocksuckers",
    "cocksucking",
    "cunt",
    "cunts",
    "dick",
    "dicks",
    "dickhead",
    "dickheads",
    "fuck",
    "fucks",
    "fucked",
    "fucker",
    "fuckers",
    "fucking",
    "fuckin",
    "motherfuck",
    "motherfucks",
    "motherfucked",
    "motherfucker",
    "motherfuckers",
    "motherfucking",
    "motherfuckin",
    "mothafucka",
    "mothafuckas",
    "mothafucker",
    "mothafuckers",
    "mothafucking",
    "mothafuckin",
    "goddamn",
    "goddamned",
    "goddammit",
    "god damn",
    "god damned",
    "piss",
    "pissed",
    "pissing",
    "pissin",
    "shit",
    "shits",
    "shitty",
    "shitting",
    "shittin",
    "shithead",
    "shitheads",
    "horseshit",
    "dipshit",
    "dipshits",
    "dumbass",
    "dumbasses",
    "jackass",
    "jackasses",
    "prick",
    "pricks",
    "twat",
    "twats",
    "wank",
    "wanker",
    "wankers",
    "wanking",
    "slut",
    "sluts",
    "whore",
    "whores",
    "nigga",
    "niggas",
    "nigger",
    "niggers",
    "fag",
    "fags",
    "faggot",
    "faggots",
    "pussy",
    "pussies",
    "tits",
    "titties",
    "blowjob",
    "blowjobs",
    "blow job",
    "blow jobs",
    "handjob",
    "handjobs",
    "hand job",
    "hand jobs",
    "cum",
    "cumming",
    "cummin",
    "jizz",
    "jack off",
    "jacking off",
    "jackin off",
    "jerk off",
    "jerking off",
];

pub const DEFAULT_POLICY_NOTE: &str = "Common English explicit markers: strong profanity, slurs and graphic sexual language. Edit for the recording; sexual, violent and drug-related context still needs review. This is a starting preset, not an official industry word list.";

impl Default for CleanPolicy {
    fn default() -> Self {
        Self {
            version: 1,
            name: "Common explicit lyrics (English)".into(),
            terms: COMMON_EXPLICIT_TERMS
                .iter()
                .map(|term| (*term).into())
                .collect(),
            padding_before_seconds: 0.04,
            padding_after_seconds: 0.06,
            fade_seconds: 0.015,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanMode {
    FullMix,
    VocalOnly,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LyricMatch {
    pub term: String,
    pub first_word: usize,
    pub end_word_exclusive: usize,
    pub start: Option<f64>,
    pub end: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MuteRange {
    pub start: f64,
    pub end: f64,
    pub enabled: bool,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanPlan {
    pub version: u32,
    pub audio_sha256: String,
    pub duration_seconds: f64,
    pub word_source: String,
    pub policy: CleanPolicy,
    pub mode: CleanMode,
    /// Evidence only: review/edit ranges explicitly to choose rendering.
    pub matches: Vec<LyricMatch>,
    pub ranges: Vec<MuteRange>,
    pub unknown_words: usize,
    pub untimed_words: usize,
    pub warnings: Vec<String>,
}

fn digest_valid(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|c| c.is_ascii_hexdigit())
}
fn duration_valid(duration: f64) -> Result<()> {
    ensure!(
        duration.is_finite() && duration > 0.,
        "Decoded audio duration must be positive and finite"
    );
    Ok(())
}
fn span_valid(start: f64, end: f64, duration: f64) -> Result<()> {
    ensure!(
        start.is_finite() && end.is_finite() && start >= 0. && start < end && end <= duration,
        "Invalid clean range {start}..{end} for duration {duration}"
    );
    Ok(())
}

/// Unicode lowercase and lexical boundaries; straight/curly internal apostrophes
/// are equivalent. Punctuation separates tokens instead of disappearing and
/// accidentally joining two words. No substring, stemming or fuzzy matching.
pub fn tokens(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut word = String::new();
    for (index, &c) in chars.iter().enumerate() {
        if c.is_alphanumeric()
            || (("'’".contains(c))
                && !word.is_empty()
                && chars.get(index + 1).is_some_and(|c| c.is_alphanumeric()))
        {
            if c == '’' {
                word.push('\'');
            } else {
                word.extend(c.to_lowercase());
            }
        } else if !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

impl WordTimings {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "Unsupported word-timing version");
        ensure!(
            digest_valid(&self.audio_sha256),
            "Word timings need the recording's SHA-256"
        );
        duration_valid(self.duration_seconds)?;
        ensure!(
            !self.source.trim().is_empty(),
            "Word-timing provenance is required"
        );
        let mut previous = 0.;
        for word in &self.words {
            if let Some(text) = &word.text {
                ensure!(
                    tokens(text).len() == 1,
                    "Each aligned word must contain exactly one lexical token; unknown text must be null"
                );
            }
            match (word.start, word.end) {
                (Some(start), Some(end)) => {
                    span_valid(start, end, self.duration_seconds)?;
                    ensure!(start >= previous, "Word timings must be in temporal order");
                    previous = start;
                }
                (None, None) => {}
                _ => anyhow::bail!("Both word start/end must be supplied or both null"),
            }
        }
        Ok(())
    }
}

impl CleanPolicy {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "Unsupported clean-policy version");
        ensure!(
            !self.name.trim().is_empty() && !self.terms.is_empty(),
            "Named clean policy needs at least one exact term"
        );
        ensure!(
            self.terms.iter().all(|t| !tokens(t).is_empty()),
            "Empty/nonlexical clean policy term"
        );
        ensure!(
            [
                self.padding_before_seconds,
                self.padding_after_seconds,
                self.fade_seconds
            ]
            .iter()
            .all(|v| v.is_finite() && *v >= 0.),
            "Padding/fades must be finite and nonnegative"
        );
        Ok(())
    }
}

impl CleanPlan {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && digest_valid(&self.audio_sha256),
            "Unsupported clean plan or invalid audio identity"
        );
        duration_valid(self.duration_seconds)?;
        self.policy.validate()?;
        ensure!(
            !self.word_source.trim().is_empty(),
            "Word-timing provenance is required"
        );
        for range in &self.ranges {
            span_valid(range.start, range.end, self.duration_seconds)?;
            ensure!(!range.reason.trim().is_empty(), "Mute range needs a reason");
        }
        for found in &self.matches {
            ensure!(
                found.first_word < found.end_word_exclusive && !tokens(&found.term).is_empty(),
                "Invalid lyric match"
            );
            match (found.start, found.end) {
                (Some(start), Some(end)) => span_valid(start, end, self.duration_seconds)?,
                (None, None) => {}
                _ => anyhow::bail!("Incomplete lyric match timing"),
            }
        }
        Ok(())
    }
}

/// Every lexical policy match is retained. Overlap is merged only for audio
/// rendering, so a reviewer can disable/edit ranges without losing provenance.
pub fn plan(words: &WordTimings, policy: &CleanPolicy, mode: CleanMode) -> Result<CleanPlan> {
    words.validate()?;
    policy.validate()?;
    let normalized: Vec<_> = words
        .words
        .iter()
        .map(|w| w.text.as_ref().map(|s| tokens(s).remove(0)))
        .collect();
    let mut matches = Vec::new();
    let mut ranges = Vec::new();
    for term in &policy.terms {
        let term_words = tokens(term);
        for first in 0..words
            .words
            .len()
            .saturating_add(1)
            .saturating_sub(term_words.len())
        {
            if normalized[first..first + term_words.len()]
                .iter()
                .zip(&term_words)
                .all(|(word, expected)| word.as_ref() == Some(expected))
            {
                let last = first + term_words.len();
                let selected = &words.words[first..last];
                let timed = selected.iter().all(|w| w.start.is_some());
                let start = timed.then(|| {
                    selected
                        .iter()
                        .filter_map(|w| w.start)
                        .fold(f64::INFINITY, f64::min)
                });
                let end = timed.then(|| selected.iter().filter_map(|w| w.end).fold(0., f64::max));
                if let (Some(start), Some(end)) = (start, end) {
                    ranges.push(MuteRange{start:(start-policy.padding_before_seconds).max(0.),end:(end+policy.padding_after_seconds).min(words.duration_seconds),enabled:true,reason:format!("Exact policy term {term:?}, words {first}..{last}; candidate needs review")});
                }
                matches.push(LyricMatch {
                    term: term.clone(),
                    first_word: first,
                    end_word_exclusive: last,
                    start,
                    end,
                });
            }
        }
    }
    matches.sort_by_key(|m| (m.first_word, m.end_word_exclusive));
    ranges.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.end.total_cmp(&b.end)));
    let unknown_words = words.words.iter().filter(|w| w.text.is_none()).count();
    let untimed_words = words.words.iter().filter(|w| w.start.is_none()).count();
    let mut warnings=vec!["Exact policy matching does not prove that every explicit lyric was found. Review recognized or supplied text, timing and selected ranges against the recording.".into()];
    if unknown_words > 0 || untimed_words > 0 {
        warnings.push(format!("{unknown_words} unknown words and {untimed_words} untimed words remain; no words/times were invented."));
    }
    let out = CleanPlan {
        version: 1,
        audio_sha256: words.audio_sha256.to_ascii_lowercase(),
        duration_seconds: words.duration_seconds,
        word_source: words.source.clone(),
        policy: policy.clone(),
        mode,
        matches,
        ranges,
        unknown_words,
        untimed_words,
        warnings,
    };
    out.validate()?;
    Ok(out)
}

pub fn merged_ranges(plan: &CleanPlan) -> Result<Vec<(f64, f64)>> {
    plan.validate()?;
    let mut ranges: Vec<_> = plan
        .ranges
        .iter()
        .filter(|r| r.enabled)
        .map(|r| (r.start, r.end))
        .collect();
    ranges.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (start, end) in ranges {
        if let Some(previous) = merged.last_mut().filter(|p| start <= p.1) {
            previous.1 = previous.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    Ok(merged)
}

/// Stereo interleaved PCM. Fades are raised-cosine ramps outside full mute
/// intervals. Stem subtraction leaves accompaniment only to the extent that
/// the supplied vocal stem is correctly aligned and isolated.
pub fn render_pcm_stereo(
    original: &[f32],
    vocal_stem: Option<&[f32]>,
    sample_rate: u32,
    plan: &CleanPlan,
) -> Result<Vec<f32>> {
    ensure!(sample_rate > 0, "Sample rate must be positive");
    ensure!(
        !original.is_empty()
            && original.len().is_multiple_of(2)
            && original.iter().all(|x| x.is_finite()),
        "Original audio must be finite interleaved stereo PCM"
    );
    let ranges = merged_ranges(plan)?;
    ensure!(
        !ranges.is_empty(),
        "No enabled timed mute ranges; refusing to label unchanged audio clean"
    );
    let duration = (original.len() / 2) as f64 / sample_rate as f64;
    ensure!(
        (duration - plan.duration_seconds).abs() <= 1. / sample_rate as f64 + 1e-9,
        "Plan duration does not match decoded audio; do not rescale word timing"
    );
    let stem = match plan.mode {
        CleanMode::VocalOnly => {
            let stem = vocal_stem.context(
                "Vocal-only mode requires an aligned full-length vocal stem; no full-mix fallback",
            )?;
            ensure!(
                stem.len() == original.len() && stem.iter().all(|x| x.is_finite()),
                "Vocal stem must have identical decoded frame count and finite stereo PCM"
            );
            Some(stem)
        }
        CleanMode::FullMix => {
            ensure!(
                vocal_stem.is_none(),
                "Full-mix mode does not use a vocal stem; choose mode explicitly"
            );
            None
        }
    };
    let fade = plan.policy.fade_seconds;
    let mut out = original.to_vec();
    let mut next = 0;
    for (frame, samples) in out.chunks_exact_mut(2).enumerate() {
        let time = frame as f64 / sample_rate as f64;
        while next < ranges.len() && time >= ranges[next].1 {
            next += 1;
        }
        // Since merged ranges are ordered/non-overlapping and every ramp has
        // the same duration, only the closest preceding/following fades can
        // dominate. This avoids scanning every range for every song sample.
        let mut amount = if ranges.get(next).is_some_and(|r| time >= r.0) {
            1.0f64
        } else {
            0.0
        };
        if amount == 0. && fade > 0. {
            if let Some(&(start, _)) = ranges.get(next).filter(|r| time >= r.0 - fade) {
                amount = (0.5 - 0.5 * (std::f64::consts::PI * (time - start + fade) / fade).cos())
                    .clamp(0., 1.);
            }
            if let Some(previous) = next
                .checked_sub(1)
                .map(|i| ranges[i])
                .filter(|r| time < r.1 + fade)
            {
                let gain = (0.5 + 0.5 * (std::f64::consts::PI * (time - previous.1) / fade).cos())
                    .clamp(0., 1.);
                amount = amount.max(gain);
            }
        }
        if amount == 0. {
            continue;
        }
        for channel in 0..2 {
            samples[channel] = if let Some(stem) = stem {
                samples[channel] - (amount as f32) * stem[frame * 2 + channel]
            } else {
                samples[channel] * (1. - amount as f32)
            };
            ensure!(
                samples[channel].is_finite(),
                "Audio arithmetic overflow; output was not published"
            );
        }
    }
    Ok(out)
}

/// Alignment is a source-bound provenance assertion, not an interactive prompt.
/// A verified cache may provide it; external callers may attest their own stem.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StemAlignment {
    pub audio_sha256: String,
    pub stem_sha256: String,
    pub evidence: String,
}

pub struct RenderRequest<'a> {
    pub audio: &'a Path,
    pub plan: &'a CleanPlan,
    pub vocal_stem: Option<&'a Path>,
    pub stem_alignment: Option<&'a StemAlignment>,
    /// New directory containing clean.wav, plan.json and receipt.json.
    pub output: &'a Path,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CleanReceipt {
    pub version: u32,
    pub status: String,
    pub source_audio: PathBuf,
    pub source_sha256: String,
    pub vocal_stem: Option<PathBuf>,
    pub vocal_stem_sha256: Option<String>,
    pub alignment: Option<StemAlignment>,
    pub output_sha256: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub frames: usize,
    pub mode: CleanMode,
    pub rendered_ranges: Vec<(f64, f64)>,
    pub unresolved_matches: usize,
    pub unknown_words: usize,
    pub untimed_words: usize,
    pub output_peak: f32,
    pub samples_over_unity: usize,
    pub warnings: Vec<String>,
}

#[cfg(not(target_arch = "wasm32"))]
fn require_stereo(path: &Path) -> Result<()> {
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=channels",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .context("ffprobe is required on PATH")?;
    ensure!(
        probe.status.success(),
        "Cannot inspect audio channels: {}",
        String::from_utf8_lossy(&probe.stderr)
    );
    let channels = std::str::from_utf8(&probe.stdout)?
        .trim()
        .parse::<u32>()
        .context("Missing/invalid source audio channel count")?;
    ensure!(
        channels == 2,
        "Clean renderer requires stereo input; refusing to silently upmix/downmix {channels} channels"
    );
    Ok(())
}

/// Decode using existing FFmpeg helpers, render without touching source files,
/// then publish an entire new output directory atomically without replacement.
#[cfg(not(target_arch = "wasm32"))]
pub fn render(request: RenderRequest<'_>) -> Result<CleanReceipt> {
    request.plan.validate()?;
    crate::pipeline::destination(request.output, None)?;
    let source_sha256 = crate::models::hash_file(request.audio)?;
    ensure!(
        source_sha256.eq_ignore_ascii_case(&request.plan.audio_sha256),
        "Stale clean plan: source audio hash differs"
    );
    require_stereo(request.audio)?;
    let original = crate::models::decode(request.audio, 44100, 2)?;
    let (vocal_stem, vocal_stem_sha256, alignment) = match request.plan.mode {
        CleanMode::VocalOnly => {
            let path = request
                .vocal_stem
                .context("Vocal-only mode requires a supplied aligned stem; no fallback")?;
            let hash = crate::models::hash_file(path)?;
            require_stereo(path)?;
            let alignment = request
                .stem_alignment
                .context("Vocal-only mode requires source/stem-bound alignment provenance")?;
            ensure!(
                source_sha256.eq_ignore_ascii_case(&alignment.audio_sha256)
                    && hash.eq_ignore_ascii_case(&alignment.stem_sha256)
                    && !alignment.evidence.trim().is_empty(),
                "Stem alignment provenance does not match the current source/stem hashes"
            );
            (
                Some(crate::models::decode(path, 44100, 2)?),
                Some(hash),
                Some(alignment.clone()),
            )
        }
        CleanMode::FullMix => {
            ensure!(
                request.vocal_stem.is_none() && request.stem_alignment.is_none(),
                "Full-mix mode does not use stem/alignment inputs"
            );
            (None, None, None)
        }
    };
    let clean = render_pcm_stereo(&original, vocal_stem.as_deref(), 44100, request.plan)?;
    let parent = request
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stage = tempfile::Builder::new()
        .prefix(".chart-engine-clean-")
        .tempdir_in(parent)?;
    let left: Vec<_> = clean.chunks_exact(2).map(|s| s[0]).collect();
    let right: Vec<_> = clean.chunks_exact(2).map(|s| s[1]).collect();
    let output_audio = stage.path().join("clean.wav");
    crate::models::write_audio(&output_audio, &left, &right)?;
    let mut warnings = request.plan.warnings.clone();
    warnings.push("This is a rendered clean candidate, not certification that every explicit word is removed. Review all edits and the full recording.".into());
    if request.plan.mode == CleanMode::VocalOnly {
        warnings.push("Vocal-only processing subtracts the supplied stem; duration/hash/provenance checks do not establish sample/phase alignment or isolation. Leakage, residual vocals, clipped/processed stems and phase mismatch can damage accompaniment or leave audible words.".into());
    }
    let samples_over_unity = clean.iter().filter(|sample| sample.abs() > 1.).count();
    if samples_over_unity > 0 {
        warnings.push("Float output contains samples above unity; no limiter, normalization or clipping was silently applied.".into());
    }
    let receipt = CleanReceipt {
        version: 1,
        status: "rendered_candidate_requires_listening_review".into(),
        source_audio: request.audio.canonicalize()?,
        source_sha256: source_sha256.clone(),
        vocal_stem: request.vocal_stem.map(Path::canonicalize).transpose()?,
        vocal_stem_sha256: vocal_stem_sha256.clone(),
        alignment,
        output_sha256: crate::models::hash_file(&output_audio)?,
        sample_rate: 44100,
        channels: 2,
        frames: clean.len() / 2,
        mode: request.plan.mode,
        rendered_ranges: merged_ranges(request.plan)?,
        unresolved_matches: request
            .plan
            .matches
            .iter()
            .filter(|m| m.start.is_none())
            .count(),
        unknown_words: request.plan.unknown_words,
        untimed_words: request.plan.untimed_words,
        output_peak: clean.iter().map(|s| s.abs()).fold(0., f32::max),
        samples_over_unity,
        warnings,
    };
    fs::write(
        stage.path().join("plan.json"),
        serde_json::to_vec_pretty(request.plan)?,
    )?;
    fs::write(
        stage.path().join("receipt.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    ensure!(
        crate::models::hash_file(request.audio)? == source_sha256,
        "Source audio changed during rendering; output was not published"
    );
    if let (Some(path), Some(hash)) = (request.vocal_stem, vocal_stem_sha256) {
        ensure!(
            crate::models::hash_file(path)? == hash,
            "Vocal stem changed during rendering; output was not published"
        );
    }
    crate::pipeline::publish(stage.path(), request.output)?;
    Ok(receipt)
}
