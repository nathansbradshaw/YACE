//! Optional native English acoustic alignment of supplied lyrics. Reuses the
//! pinned Rust wav2vec2 model from EntreNota. No Python, ASR text invention or
//! evenly divided word/line timestamps.
use crate::{
    clean::{AlignedWord, StemAlignment, WordTimings},
    clean_recognition::Control,
    models,
    vocal_generation::LyricSource,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub const REVISION: &str = "22aad52d435eb6dbaf354bdad9b0da84ce7d6156";
pub const METHOD: &str = "rust-native-wav2vec2-ctc-v2; English; 16kHz; 20ms emissions; packed Viterbi; optional transcript-checked provider line windows; free leading/trailing audio; low evidence remains untimed";
pub struct Asset {
    pub filename: &'static str,
    pub sha256: &'static str,
}
pub fn assets() -> [Asset; 3] {
    [
        Asset {
            filename: "config.json",
            sha256: "2dbf0f1ff1bf68a3f0a6e61a53eeab3a68189af0a86ba4644d58342b52e1ebaa",
        },
        Asset {
            filename: "vocab.json",
            sha256: "19727f8944fe6459fc3f240ae2c198395b740f6a029bd23e06656266b83bcf64",
        },
        Asset {
            filename: "model.safetensors",
            sha256: "8aa76ab2243c81747a1f832954586bc566090c83a0ac167df6f31f0fa917d74a",
        },
    ]
}
pub fn verified(model_dir: &Path) -> Result<PathBuf> {
    let directory = alignment_directory(model_dir)?;
    for asset in assets() {
        ensure!(
            models::hash_file(&directory.join(asset.filename))? == asset.sha256,
            "English acoustic alignment asset {} failed verification",
            asset.filename
        );
    }
    Ok(directory)
}
fn alignment_directory(root: &Path) -> Result<PathBuf> {
    models::optional_model_directory(
        root,
        Path::new("vocal-alignment/en"),
        &assets()
            .iter()
            .map(|asset| asset.filename)
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
mod bundle_asset_tests {
    use super::*;
    #[test]
    fn invalid_bundled_alignment_assets_fail_before_any_write_or_download() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("YACE.app/Contents/Resources/models");
        let directory = root.join("vocal-alignment/en");
        fs::create_dir_all(&directory).unwrap();
        for asset in assets() {
            fs::write(directory.join(asset.filename), b"retained invalid fixture").unwrap();
        }
        let error = prepare(&root, true, &Control::default()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Bundled English alignment assets")
        );
        for asset in assets() {
            assert_eq!(
                fs::read(directory.join(asset.filename)).unwrap(),
                b"retained invalid fixture"
            );
        }
        assert_eq!(fs::read_dir(directory).unwrap().count(), 3);
    }
}
fn prepare(root: &Path, download: bool, control: &Control) -> Result<PathBuf> {
    if let Ok(path) = verified(root) {
        return Ok(path);
    }
    ensure!(
        download,
        "The optional English acoustic alignment model is missing or invalid; allow its verified one-time download"
    );
    let directory = alignment_directory(root)?;
    ensure!(
        !models::bundle_model_directory(&directory),
        "Bundled English alignment assets failed verification; rebuild the bundle or choose a custom model directory"
    );
    fs::create_dir_all(&directory)?;
    for (name, data) in [
        (
            "config.json",
            include_bytes!("../assets/vocal-alignment/config.json").as_slice(),
        ),
        (
            "vocab.json",
            include_bytes!("../assets/vocal-alignment/vocab.json").as_slice(),
        ),
    ] {
        let mut stage = tempfile::NamedTempFile::new_in(&directory)?;
        stage.write_all(data)?;
        stage.flush()?;
        stage.persist(directory.join(name))?;
    }
    ensure!(
        !control.is_cancelled(),
        "Acoustic lyric alignment cancelled"
    );
    let asset = assets()
        .into_iter()
        .find(|asset| asset.filename == "model.safetensors")
        .unwrap();
    if models::hash_file(&directory.join(asset.filename))
        .ok()
        .as_deref()
        != Some(asset.sha256)
    {
        let url = format!(
            "https://huggingface.co/facebook/wav2vec2-base-960h/resolve/{REVISION}/model.safetensors"
        );
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(900))
            .user_agent("chart-engine-native-lyrics/0.1")
            .build()?;
        let mut response = client.get(url).send()?.error_for_status()?;
        let mut stage = tempfile::NamedTempFile::new_in(&directory)?;
        let mut buffer = [0; 65536];
        loop {
            ensure!(
                !control.is_cancelled(),
                "Acoustic lyric alignment cancelled"
            );
            let count = response.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            stage.write_all(&buffer[..count])?;
        }
        stage.flush()?;
        ensure!(
            models::hash_file(stage.path())? == asset.sha256,
            "Downloaded acoustic alignment model checksum mismatch"
        );
        stage.persist(directory.join(asset.filename))?;
    }
    verified(root)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WordDiagnostic {
    pub text: String,
    pub proposed_start_seconds: Option<f64>,
    pub proposed_end_seconds: Option<f64>,
    pub mean_log_probability: Option<f32>,
    pub uncertain: bool,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Alignment {
    pub version: u32,
    pub language: String,
    pub model_revision: String,
    pub model_sha256: String,
    pub vocal_stem_sha256: String,
    pub words: WordTimings,
    pub diagnostics: Vec<WordDiagnostic>,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub selection: Option<PathSelection>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PathSelection {
    pub global_supported_words: usize,
    pub guided_supported_words: Option<usize>,
    pub used_guide: bool,
    pub reason: String,
}
pub struct Request<'a> {
    pub audio: &'a Path,
    pub vocal_stem: &'a Path,
    pub alignment: &'a StemAlignment,
    pub lyrics: &'a LyricSource,
    pub model_dir: &'a Path,
    pub download_model: bool,
}
pub fn align(request: Request<'_>, control: Arc<Control>) -> Result<Alignment> {
    ensure!(
        !control.is_cancelled(),
        "Acoustic lyric alignment cancelled"
    );
    ensure!(
        !request.lyrics.source.trim().is_empty(),
        "Supplied lyrics need provenance"
    );
    ensure!(
        whatlang::detect(&request.lyrics.text)
            .is_some_and(|info| info.lang() == whatlang::Lang::Eng),
        "The optional acoustic aligner supports English lyrics only; this text was not identified as English"
    );
    control.update("Preparing the local English word aligner…");
    let source_hash = models::hash_file(request.audio)?;
    let stem_hash = models::hash_file(request.vocal_stem)?;
    ensure!(
        request
            .alignment
            .audio_sha256
            .eq_ignore_ascii_case(&source_hash)
            && request
                .alignment
                .stem_sha256
                .eq_ignore_ascii_case(&stem_hash)
            && !request.alignment.evidence.trim().is_empty(),
        "Vocal stem alignment does not match this recording"
    );
    let original = models::decode(request.audio, 44100, 2)?;
    let stem = models::decode(request.vocal_stem, 44100, 2)?;
    ensure!(
        original.len() == stem.len(),
        "Vocal stem must span the source recording's full sample clock"
    );
    let duration = original.len() as f64 / 88200.;
    drop(original);
    drop(stem);
    let directory = prepare(request.model_dir, request.download_model, &control)?;
    ensure!(
        !control.is_cancelled(),
        "Acoustic lyric alignment cancelled"
    );
    let vocabulary: HashMap<String, usize> =
        serde_json::from_slice(&fs::read(directory.join("vocab.json"))?)?;
    let samples = models::decode(request.vocal_stem, 16000, 1)?;
    ensure!(
        samples.iter().all(|sample| sample.is_finite()),
        "Nonfinite alignment PCM"
    );
    let model = practice_acoustic::AcousticModel::load(&directory)
        .map_err(|error| anyhow::anyhow!("Loading the native English acoustic model: {error}"))?;
    control.update("Aligning the supplied lyrics to the vocal stem…");
    let emissions = model
        .emissions_cancellable(&samples, &|| control.is_cancelled(), &|position, length| {
            control.update(format!(
                "Analyzing sung-word timing: {}%…",
                position.saturating_mul(100) / length.max(1)
            ));
        })
        .map_err(|error| anyhow::anyhow!("Native acoustic lyric alignment: {error}"))?;
    control.update("Checking acoustic word boundaries…");
    let (words, diagnostics, selection) = align_emissions_with_selection(
        &emissions,
        &vocabulary,
        request.lyrics,
        &source_hash,
        duration,
        &|| control.is_cancelled(),
    )?;
    ensure!(
        models::hash_file(request.audio)? == source_hash
            && models::hash_file(request.vocal_stem)? == stem_hash,
        "Recording or vocal stem changed during acoustic alignment"
    );
    verified(request.model_dir)?;
    let mut warnings=vec!["Acoustic word timing is model-estimated. The English speech model is not singing-specific; repeated choruses, long vowels, missing lines and backing vocals require review. Uncertain words retain proposed bounds only in diagnostics and are excluded from playable timing.".into()];
    if request.lyrics.synced_lyrics.is_some() && !selection.used_guide {
        warnings.push(format!(
            "Provider line timing was not selected: {}",
            selection.reason
        ));
    }
    Ok(Alignment {
        version: 1,
        language: "en".into(),
        model_revision: REVISION.into(),
        model_sha256: assets()[2].sha256.into(),
        vocal_stem_sha256: stem_hash,
        words,
        diagnostics,
        warnings,
        selection: Some(selection),
    })
}

struct Target {
    text: String,
    states: Vec<usize>,
    unsupported: bool,
    window: Option<(usize, usize)>,
}

/// Use LRC lines to narrow the word search only when all words match the
/// supplied plain lyrics in order. Line timestamps cannot supply word timing.
fn lyric_line_windows(lyrics: &LyricSource, duration: f64) -> Result<Option<Vec<(f64, f64)>>> {
    let Some(synced) = &lyrics.synced_lyrics else {
        return Ok(None);
    };
    let mut lines = Vec::new();
    for line in synced.lines() {
        let mut rest = line.trim();
        let mut times = Vec::new();
        while let Some(after) = rest.strip_prefix('[') {
            let Some((timestamp, tail)) = after.split_once(']') else {
                break;
            };
            if let Some((minute, second)) = timestamp.split_once(':')
                && let (Ok(minute), Ok(second)) = (minute.parse::<u32>(), second.parse::<f64>())
                && second.is_finite()
                && (0.0..60.0).contains(&second)
            {
                let time = minute as f64 * 60. + second;
                ensure!(
                    time <= duration + 0.75,
                    "Provider lyric line extends beyond this recording"
                );
                times.push(time);
            }
            rest = tail;
        }
        for time in times {
            lines.push((time, crate::clean::tokens(rest)));
        }
    }
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    ensure!(
        !lines.is_empty() && lines.windows(2).all(|pair| pair[0].0 < pair[1].0),
        "Conflicting or missing provider line timestamps"
    );
    ensure!(
        lines
            .iter()
            .flat_map(|(_, words)| words.iter())
            .eq(crate::clean::tokens(&lyrics.text).iter()),
        "Provider line transcript differs from supplied plain lyrics"
    );
    let mut windows = Vec::new();
    for (index, (start, words)) in lines.iter().enumerate() {
        let end = lines
            .get(index + 1)
            .map_or(duration, |(time, _)| (*time + 0.75).min(duration));
        let start = (*start - 0.75).max(0.);
        ensure!(start < end, "Invalid provider lyric line window");
        windows.extend(std::iter::repeat_n((start, end), words.len()));
    }
    Ok(Some(windows))
}

/// Shared deterministic CTC core, also usable with externally provided native
/// acoustic emissions. Only acoustically supported words receive timing.
pub fn align_emissions(
    emissions: &[Vec<f32>],
    vocabulary: &HashMap<String, usize>,
    lyrics: &LyricSource,
    audio_sha256: &str,
    duration: f64,
    cancelled: &dyn Fn() -> bool,
) -> Result<(WordTimings, Vec<WordDiagnostic>)> {
    let (words, diagnostics, _) = align_emissions_with_selection(
        emissions,
        vocabulary,
        lyrics,
        audio_sha256,
        duration,
        cancelled,
    )?;
    Ok((words, diagnostics))
}

/// Compare complete paths against identical acoustic evidence. Line metadata
/// cannot override a globally better-supported transcript, and no word-level
/// mixing can silently reorder or duplicate a lyric occurrence.
pub fn align_emissions_with_selection(
    emissions: &[Vec<f32>],
    vocabulary: &HashMap<String, usize>,
    lyrics: &LyricSource,
    audio_sha256: &str,
    duration: f64,
    cancelled: &dyn Fn() -> bool,
) -> Result<(WordTimings, Vec<WordDiagnostic>, PathSelection)> {
    let mut plain = lyrics.clone();
    plain.synced_lyrics = None;
    let (mut global, global_diagnostics, _, _) = align_candidate(
        emissions,
        vocabulary,
        &plain,
        audio_sha256,
        duration,
        cancelled,
    )?;
    let count = |words: &WordTimings| {
        words
            .words
            .iter()
            .filter(|word| word.start.is_some() && word.end.is_some())
            .count()
    };
    let global_count = count(&global);
    let mut selection = PathSelection {
        global_supported_words: global_count,
        guided_supported_words: None,
        used_guide: false,
        reason: "No provider line guide supplied".into(),
    };
    if lyrics.synced_lyrics.is_none() {
        return Ok((global, global_diagnostics, selection));
    }
    let (mut guided, guided_diagnostics, guide_valid, guide_status) = align_candidate(
        emissions,
        vocabulary,
        lyrics,
        audio_sha256,
        duration,
        cancelled,
    )?;
    let guided_count = count(&guided);
    selection.guided_supported_words = Some(guided_count);
    if guide_valid && guided_count >= global_count {
        selection.used_guide = true;
        selection.reason = format!(
            "Selected whole guided path: {guided_count} supported words versus {global_count} globally"
        );
        guided.source.push_str(&format!("; {}", selection.reason));
        return Ok((guided, guided_diagnostics, selection));
    }
    selection.reason = if guide_valid {
        format!(
            "Rejected whole guided path: supported words regressed from {global_count} globally to {guided_count}; unchanged acoustic/word-duration gates"
        )
    } else {
        guide_status
    };
    global
        .source
        .push_str(&format!("; line_guide=not-selected; {}", selection.reason));
    Ok((global, global_diagnostics, selection))
}

fn align_candidate(
    emissions: &[Vec<f32>],
    vocabulary: &HashMap<String, usize>,
    lyrics: &LyricSource,
    audio_sha256: &str,
    duration: f64,
    cancelled: &dyn Fn() -> bool,
) -> Result<(WordTimings, Vec<WordDiagnostic>, bool, String)> {
    ensure!(
        !lyrics.source.trim().is_empty(),
        "Supplied lyrics need provenance"
    );
    ensure!(
        duration.is_finite() && duration > 0. && duration <= 14400.,
        "Invalid alignment duration"
    );
    ensure!(
        (emissions.len() as f64 * 0.020 - duration).abs() <= 0.050,
        "Acoustic emissions do not match the original source duration"
    );
    let blank = *vocabulary.get("<pad>").context("Missing CTC blank token")?;
    let separator = *vocabulary.get("|").context("Missing CTC word separator")?;
    let text = crate::clean::tokens(&lyrics.text);
    ensure!(
        !text.is_empty() && text.len() <= 3000,
        "Lyrics exceed the supported word budget or contain no words"
    );
    let mut states = vec![blank];
    let (guide, mut guide_status) = match lyric_line_windows(lyrics, duration) {
        Ok(guide) => (guide, "line_guide=none".to_owned()),
        Err(error) => (
            None,
            format!("line_guide=rejected ({error}); global acoustic alignment retained"),
        ),
    };
    let mut state_windows = vec![None];
    let mut targets = Vec::new();
    for (word_index, text) in text.into_iter().enumerate() {
        if !targets.is_empty() {
            states.extend([separator, blank]);
            state_windows.extend([None, None]);
        }
        let window = guide.as_ref().map(|guide| {
            let (start, end) = guide[word_index];
            (
                (start / 0.020).floor() as usize,
                (end / 0.020).ceil() as usize,
            )
        });
        let mut target = Target {
            text,
            states: Vec::new(),
            unsupported: false,
            window,
        };
        for letter in target.text.chars() {
            let token = vocabulary
                .get(&letter.to_uppercase().collect::<String>())
                .or_else(|| vocabulary.get(&letter.to_string()));
            if let Some(&token) = token {
                target.states.push(states.len());
                states.extend([token, blank]);
                state_windows.extend([window, window]);
            } else {
                target.unsupported = true;
            }
        }
        if target.states.is_empty() {
            target.unsupported = true;
        }
        targets.push(target);
    }
    ensure!(
        targets.iter().any(|target| !target.states.is_empty()),
        "Lyrics do not match the supported English acoustic vocabulary"
    );
    if let Some(last) = state_windows.last_mut() {
        *last = None;
    }
    let (path, guide_used) = if guide.is_some() {
        match trellis(emissions, &states, Some(&state_windows), cancelled) {
            Ok(path) => {
                guide_status="line_guide=provider-line-bounds-with-0.75s-tolerance; words remain acoustically aligned".into();
                (path, true)
            }
            Err(error) => {
                ensure!(!cancelled(), "Acoustic lyric alignment cancelled");
                guide_status =
                    format!("line_guide=infeasible ({error}); global acoustic alignment retained");
                (trellis(emissions, &states, None, cancelled)?, false)
            }
        }
    } else {
        (trellis(emissions, &states, None, cancelled)?, false)
    };
    let mut occupied: Vec<Vec<(usize, f32)>> = vec![Vec::new(); states.len()];
    for (frame, state) in path.into_iter().enumerate() {
        occupied[state].push((frame, emissions[frame][states[state]]));
    }
    let mut aligned = Vec::new();
    let mut diagnostics = Vec::new();
    let mut previous_end = 0.;
    for target in targets {
        let observations: Vec<_> = target
            .states
            .iter()
            .flat_map(|&state| occupied[state].iter().copied())
            .collect();
        let all_letters = target
            .states
            .iter()
            .all(|&state| !occupied[state].is_empty());
        let mut first = observations.iter().map(|v| v.0).min();
        let mut last = observations.iter().map(|v| v.0).max();
        // Intro/outro skips can shrink a sustained edge vowel to one frame.
        // Extend it only through adjacent frames supporting the same letter.
        let dominant = |frame: usize, token: usize| {
            let row = &emissions[frame];
            row[token] >= 0.03f32.ln()
                && row[token] >= row.iter().copied().fold(f32::NEG_INFINITY, f32::max) - 0.05
        };
        if let (Some(frame), Some(&state)) = (&mut first, target.states.first()) {
            while *frame > 0
                && (!guide_used || target.window.is_none_or(|(start, _)| *frame > start))
                && dominant(*frame - 1, states[state])
            {
                *frame -= 1;
            }
        }
        if let (Some(frame), Some(&state)) = (&mut last, target.states.last()) {
            while *frame + 1 < emissions.len()
                && (!guide_used || target.window.is_none_or(|(_, end)| *frame + 1 < end))
                && dominant(*frame + 1, states[state])
            {
                *frame += 1;
            }
        }
        // The feature extractor's receptive field is 400 samples at 16 kHz.
        // Center timestamps are source-time estimates, not millisecond truth.
        let start = first.map(|frame| frame as f64 * 0.020 + 0.0125);
        let end = last.map(|frame| ((frame + 1) as f64 * 0.020 + 0.0125).min(duration));
        let mean = (!observations.is_empty())
            .then(|| observations.iter().map(|v| v.1).sum::<f32>() / observations.len() as f32);
        let letter_peaks: Vec<_> = target
            .states
            .iter()
            .map(|&state| {
                occupied[state]
                    .iter()
                    .map(|v| v.1)
                    .fold(f32::NEG_INFINITY, f32::max)
            })
            .collect();
        let peaks_mean = (!letter_peaks.is_empty())
            .then(|| letter_peaks.iter().sum::<f32>() / letter_peaks.len() as f32);
        let reason = if target.unsupported {
            Some("Word contains characters outside the English acoustic vocabulary")
        } else if !all_letters || observations.is_empty() {
            Some("Not every lyric letter has an acoustic alignment")
        } else if !mean.is_some_and(|mean| mean.is_finite() && mean.exp() >= 0.01)
            || !peaks_mean.is_some_and(|mean| mean.is_finite() && mean.exp() >= 0.03)
        {
            Some("Insufficient acoustic evidence for this supplied word")
        } else if !start
            .zip(end)
            .is_some_and(|(start, end)| start >= previous_end && start < end && end - start <= 4.0)
        {
            Some("Unreliable, overlapping or unusually long word boundary")
        } else {
            None
        };
        if reason.is_none() {
            previous_end = end.unwrap();
        }
        aligned.push(AlignedWord {
            text: Some(target.text.clone()),
            start: reason.is_none().then_some(start).flatten(),
            end: reason.is_none().then_some(end).flatten(),
        });
        diagnostics.push(WordDiagnostic {
            text: target.text,
            proposed_start_seconds: start,
            proposed_end_seconds: end,
            mean_log_probability: mean.filter(|p| p.is_finite()),
            uncertain: reason.is_some(),
            reason: reason.map(str::to_owned),
        });
    }
    let words = WordTimings {
        version: 1,
        audio_sha256: audio_sha256.into(),
        duration_seconds: duration,
        source: format!(
            "{METHOD}; {guide_status}; model facebook/wav2vec2-base-960h@{REVISION}; supplied lyrics: {}; uncertain words remain untimed",
            lyrics.source
        ),
        words: aligned,
    };
    words.validate()?;
    Ok((words, diagnostics, guide_used, guide_status))
}

fn trellis(
    probabilities: &[Vec<f32>],
    tokens: &[usize],
    windows: Option<&[Option<(usize, usize)>]>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<usize>> {
    let frames = probabilities.len();
    let states = tokens.len();
    ensure!(
        frames > 0
            && states > 1
            && frames >= states / 2
            && frames.saturating_mul(states) <= 250_000_000,
        "Lyrics and recording exceed the bounded acoustic alignment budget"
    );
    ensure!(
        probabilities.iter().all(
            |row| tokens.iter().max().is_some_and(|token| *token < row.len())
                && row.iter().all(|p| p.is_finite() && *p <= 0.001)
        ),
        "Invalid acoustic log probabilities or vocabulary mismatch"
    );
    let mut back = vec![0u8; (frames * states).div_ceil(4)];
    let mut previous = vec![f32::NEG_INFINITY; states];
    let mut current = previous.clone();
    previous[0] = 0.;
    if windows.is_none_or(|windows| windows[1].is_none_or(|(start, end)| start == 0 && end > 0)) {
        previous[1] = probabilities[0][tokens[1]];
    }
    for (frame, row) in probabilities.iter().enumerate().skip(1) {
        ensure!(!cancelled(), "Acoustic lyric alignment cancelled");
        current.fill(f32::NEG_INFINITY);
        let lower = states
            .saturating_sub(2)
            .saturating_sub(2 * (frames - 1 - frame));
        let upper = (2 * frame + 1).min(states - 1);
        for state in lower..=upper {
            if windows.is_some_and(|windows| {
                windows[state].is_some_and(|(start, end)| frame < start || frame >= end)
            }) {
                continue;
            }
            let mut best = previous[state];
            let mut step = 0;
            if state > 0 && previous[state - 1] > best {
                best = previous[state - 1];
                step = 1;
            }
            if state > 1 && tokens[state] != tokens[state - 2] && previous[state - 2] > best {
                best = previous[state - 2];
                step = 2;
            }
            current[state] = best
                + if state == 0 || state == states - 1 {
                    0.
                } else {
                    row[tokens[state]]
                };
            let index = frame * states + state;
            back[index / 4] |= step << ((index % 4) * 2);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let mut state = states - 1;
    if previous[states - 2] > previous[state] {
        state -= 1;
    }
    ensure!(
        previous[state].is_finite(),
        "Supplied lyrics could not be acoustically aligned"
    );
    let mut path = vec![0; frames];
    for frame in (0..frames).rev() {
        path[frame] = state;
        if frame > 0 {
            let index = frame * states + state;
            state = state.saturating_sub(((back[index / 4] >> ((index % 4) * 2)) & 3) as usize);
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feasible_provider_guide_cannot_reduce_supported_word_quality() {
        let mut tokens = vec![0; 200];
        tokens[70..75].fill(2);
        tokens[160..165].fill(3);
        let rows = emissions(&tokens);
        let lyrics = LyricSource {
            text: "a b".into(),
            source: "Synthetic early provider timestamp".into(),
            synced_lyrics: Some("[00:00.00] a\n[00:00.50] b".into()),
        };
        let (words, diagnostics, selection) = align_emissions_with_selection(
            &rows,
            &vocabulary(),
            &lyrics,
            &"a".repeat(64),
            4.,
            &|| false,
        )
        .unwrap();
        assert_eq!(selection.global_supported_words, 2);
        assert!(
            selection
                .guided_supported_words
                .is_some_and(|count| count < 2)
        );
        assert!(!selection.used_guide);
        assert!(selection.reason.contains("regressed"));
        assert!(words.source.contains("line_guide=not-selected"));
        assert!(diagnostics.iter().all(|word| !word.uncertain));
        assert!((words.words[0].start.unwrap() - 1.4125).abs() < 0.00001);
    }
    #[test]
    #[ignore = "requires explicit provider lyric source and global/guided recording analysis"]
    fn inspect_provider_line_guide_disagreement() -> Result<()> {
        let source = std::env::var("CHART_ENGINE_VOCAL_LYRIC_SOURCE")?;
        let root = PathBuf::from(std::env::var("CHART_ENGINE_VOCAL_ANALYSIS")?);
        let lyrics: LyricSource = serde_json::from_slice(&fs::read(source)?)?;
        let global: WordTimings =
            serde_json::from_slice(&fs::read(root.join("global-words.json"))?)?;
        let guided: WordTimings =
            serde_json::from_slice(&fs::read(root.join("guided-words.json"))?)?;
        let diagnostics: Vec<WordDiagnostic> =
            serde_json::from_slice(&fs::read(root.join("global-diagnostics.json"))?)?;
        let windows =
            lyric_line_windows(&lyrics, global.duration_seconds)?.context("No line guide")?;
        let mut lost = Vec::new();
        let mut gained = Vec::new();
        let mut moved = Vec::new();
        for (index, ((a, b), (lo, hi))) in global
            .words
            .iter()
            .zip(&guided.words)
            .zip(windows)
            .enumerate()
        {
            match(a.start.zip(a.end),b.start.zip(b.end)) {
                (Some((s,e)),None)=>lost.push(serde_json::json!({"index":index,"global_start":s,"global_end":e,
                    "line_window_start":lo,"line_window_end":hi,"global_outside_line_window":s<lo||e>hi,
                    "mean_global_log_probability":diagnostics[index].mean_log_probability})),
                (None,Some((s,e)))=>gained.push(serde_json::json!({"index":index,"guided_start":s,"guided_end":e})),
                (Some((a,b)),Some((c,d))) if (a-c).abs()>0.04||(b-d).abs()>0.04=>moved.push(serde_json::json!({
                    "index":index,"global_start":a,"guided_start":c,"start_delta":c-a,"end_delta":d-b})),
                _=>()
            }
        }
        let report = serde_json::json!({"lost_global_words":lost,"gained_guided_words":gained,"moved_words":moved});
        fs::write(
            root.join("line-guide-comparison.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        eprintln!("{}", serde_json::to_string_pretty(&report)?);
        Ok(())
    }
    fn vocabulary() -> HashMap<String, usize> {
        [("<pad>", 0), ("|", 1), ("A", 2), ("B", 3), ("L", 4)]
            .into_iter()
            .map(|(s, i)| (s.into(), i))
            .collect()
    }
    fn emissions(tokens: &[usize]) -> Vec<Vec<f32>> {
        tokens
            .iter()
            .map(|&token| {
                let mut row = vec![-9.; 5];
                row[token] = -0.01;
                row
            })
            .collect()
    }
    #[test]
    fn repeated_letters_require_real_separation_and_keep_sung_vowel_duration() {
        let rows = emissions(&[0, 0, 2, 2, 2, 0, 4, 0, 4, 4, 1, 3, 3, 0, 0]);
        let lyrics = LyricSource {
            synced_lyrics: None,
            text: "all b".into(),
            source: "Explicit synthetic supplied text".into(),
        };
        let (words, report) = align_emissions(
            &rows,
            &vocabulary(),
            &lyrics,
            &"a".repeat(64),
            0.30,
            &|| false,
        )
        .unwrap();
        assert_eq!(words.words.len(), 2);
        assert!(report.iter().all(|word| !word.uncertain));
        assert!((words.words[0].start.unwrap() - 0.0525).abs() < 0.00001);
        assert!((words.words[0].end.unwrap() - 0.2125).abs() < 0.00001);
        assert!(words.words[0].end.unwrap() <= words.words[1].start.unwrap());
    }
    #[test]
    fn unsupported_lyrics_and_weak_acoustics_never_get_fabricated_times() {
        let rows = vec![vec![-8.; 5]; 20];
        for text in ["a b", "a corazón"] {
            let lyrics = LyricSource {
                synced_lyrics: None,
                text: text.into(),
                source: "Supplied original text".into(),
            };
            let (words, diagnostics) = align_emissions(
                &rows,
                &vocabulary(),
                &lyrics,
                &"a".repeat(64),
                0.40,
                &|| false,
            )
            .unwrap();
            assert!(
                words
                    .words
                    .iter()
                    .all(|word| word.start.is_none() && word.end.is_none())
            );
            assert!(diagnostics.iter().all(|word| word.uncertain));
        }
    }
    #[test]
    fn malformed_emissions_source_clock_and_cancellation_fail_closed() {
        let lyrics = LyricSource {
            synced_lyrics: None,
            text: "ab".into(),
            source: "Test".into(),
        };
        let rows = emissions(&[0, 2, 0, 3, 0]);
        assert!(
            align_emissions(&rows, &vocabulary(), &lyrics, &"a".repeat(64), 1.0, &|| {
                false
            })
            .is_err()
        );
        assert!(
            align_emissions(
                &rows,
                &vocabulary(),
                &lyrics,
                &"a".repeat(64),
                0.10,
                &|| true
            )
            .is_err()
        );
        let mut bad = rows;
        bad[2][0] = f32::NAN;
        assert!(
            align_emissions(&bad, &vocabulary(), &lyrics, &"a".repeat(64), 0.10, &|| {
                false
            })
            .is_err()
        );
    }
    #[test]
    fn line_guides_constrain_real_acoustic_peaks_without_spreading_words() {
        let mut rows = vec![vec![-10.; 5]; 250];
        for row in &mut rows {
            row[0] = -0.01;
        }
        for (frame, token, score) in [
            (10, 2, -0.0001),
            (70, 2, -0.2),
            (159, 1, -0.01),
            (160, 3, -0.01),
        ] {
            rows[frame][token] = score;
            rows[frame][0] = -2.;
        }
        let lyrics = LyricSource {
            text: "a b".into(),
            source: "Synthetic source-bound line guide".into(),
            synced_lyrics: Some("[00:01.00]a\n[00:03.00]b".into()),
        };
        let (words, _) =
            align_emissions(&rows, &vocabulary(), &lyrics, &"a".repeat(64), 5., &|| {
                false
            })
            .unwrap();
        assert!((words.words[0].start.unwrap() - 1.4125).abs() < 0.001);
        assert!((words.words[1].start.unwrap() - 3.2125).abs() < 0.001);
        assert!(words.source.contains("line_guide=provider-line-bounds"));
    }
    #[test]
    fn mismatched_line_text_falls_back_to_acoustics_and_preserves_plain_words() {
        let rows = emissions(&[0, 2, 0, 1, 3, 0]);
        let lyrics = LyricSource {
            text: "a b".into(),
            source: "Explicit supplied text".into(),
            synced_lyrics: Some("[00:00.00]wrong words".into()),
        };
        let (words, _) = align_emissions(
            &rows,
            &vocabulary(),
            &lyrics,
            &"a".repeat(64),
            0.12,
            &|| false,
        )
        .unwrap();
        assert_eq!(
            words
                .words
                .iter()
                .map(|word| word.text.as_deref().unwrap())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(words.source.contains("line_guide=rejected"));
    }

    #[test]
    #[ignore = "requires explicitly supplied original recording, aligned vocal stem, words and verified English weights"]
    fn local_english_acoustic_alignment_retains_real_word_diagnostics() {
        let song = PathBuf::from(std::env::var("CHART_ENGINE_VOCAL_SONG").unwrap());
        let stem = PathBuf::from(std::env::var("CHART_ENGINE_VOCAL_STEM").unwrap());
        let word_file = PathBuf::from(std::env::var("CHART_ENGINE_VOCAL_WORDS").unwrap());
        let model_dir = PathBuf::from(std::env::var("CHART_ENGINE_VOCAL_MODELS").unwrap());
        let output = PathBuf::from(std::env::var("CHART_ENGINE_VOCAL_OUTPUT").unwrap());
        assert!(!output.exists());
        let original = song.join("song.ogg");
        let supplied: WordTimings = serde_json::from_slice(&fs::read(word_file).unwrap()).unwrap();
        let lyrics = if let Ok(file) = std::env::var("CHART_ENGINE_VOCAL_LYRICS") {
            serde_json::from_slice::<LyricSource>(&fs::read(file).unwrap()).unwrap()
        } else {
            LyricSource {
                synced_lyrics: None,
                text: supplied
                    .words
                    .iter()
                    .filter_map(|word| word.text.as_deref())
                    .collect::<Vec<_>>()
                    .join(" "),
                source: format!(
                    "Existing local ASR text; not trusted/fetched original lyrics; {}",
                    supplied.source
                ),
            }
        };
        let binding = StemAlignment {
            audio_sha256: models::hash_file(&original).unwrap(), stem_sha256: models::hash_file(&stem).unwrap(),
            evidence: "Previously audited full-length six-stem cache; historical alignment attestation; exact lyric and vocal correctness remain unverified".into(),
        };
        let result = align(
            Request {
                audio: &original,
                vocal_stem: &stem,
                alignment: &binding,
                lyrics: &lyrics,
                model_dir: &model_dir,
                download_model: false,
            },
            Arc::new(Control::default()),
        )
        .unwrap();
        let timed = result
            .words
            .words
            .iter()
            .filter(|word| word.start.is_some())
            .count();
        assert!(
            timed > 0,
            "Actual vocal stem must supply some acoustic word evidence in this explicit integration"
        );
        assert_eq!(result.words.words.len(), result.diagnostics.len());
        assert!(
            result
                .words
                .words
                .iter()
                .zip(&result.diagnostics)
                .all(|(word, diagnostics)| word.start.is_none() == diagnostics.uncertain)
        );
        fs::create_dir(&output).unwrap();
        fs::write(
            output.join("alignment.json"),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
        fs::write(
            output.join("words.json"),
            serde_json::to_vec_pretty(&result.words).unwrap(),
        )
        .unwrap();
        eprintln!(
            "native acoustic alignment: {timed}/{} timed words; source {}; output {}",
            result.words.words.len(),
            lyrics.source,
            output.display()
        );
    }
}
