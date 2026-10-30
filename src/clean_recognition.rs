//! Local, cancellable recognition produces reviewable inputs to the clean core.
//! Whisper's word timestamps are model estimates, never a cleanliness verdict.
use crate::{
    clean::{self, AlignedWord, CleanMode, CleanPlan, CleanPolicy, StemAlignment, WordTimings},
    models, pipeline,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    ffi::c_void,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const MIN_WORD_PROBABILITY: f32 = 0.30;
pub const RECOGNITION_WARNING: &str = "Automatic English lyric recognition and word timing are estimates. Singing, backing vocals and instruments can cause missed or incorrect words and boundaries. Listen to each edit; zero matches does not establish that a recording is clean.";

#[derive(Default)]
pub struct Control {
    cancelled: AtomicBool,
    message: Mutex<String>,
}
impl Control {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
    pub fn message(&self) -> String {
        self.message.lock().map(|m| m.clone()).unwrap_or_default()
    }
    pub(crate) fn update(&self, message: impl Into<String>) {
        if let Ok(mut current) = self.message.lock() {
            *current = message.into();
        }
    }
    fn check(&self) -> Result<()> {
        ensure!(!self.is_cancelled(), "Automatic lyric analysis cancelled");
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecognizedSegment {
    pub text: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub mean_token_probability: f32,
    pub no_speech_probability: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recognition {
    pub version: u32,
    pub backend: String,
    pub model_sha256: String,
    pub recognition_audio_sha256: String,
    pub language: String,
    pub words: WordTimings,
    pub segments: Vec<RecognizedSegment>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Detection {
    pub recognition: Recognition,
    pub plan: CleanPlan,
}

pub struct Request<'a> {
    pub audio: &'a Path,
    pub model_dir: &'a Path,
    pub download_model: bool,
    /// Optional full-length aligned vocal stem; original source remains authoritative.
    pub vocal_stem: Option<(&'a Path, &'a StemAlignment)>,
}

pub fn prepare_model(
    dir: &Path,
    download: bool,
    language: crate::lyric_language::Language,
    control: &Control,
) -> Result<PathBuf> {
    let asset = language.asset();
    let resolved =
        models::optional_model_directory(dir, Path::new(""), &[asset.filename.as_str()])?;
    let dir = resolved.as_path();
    let verify = || -> Result<PathBuf> {
        let path = dir.join(&asset.filename);
        ensure!(
            models::hash_file(&path)? == asset.sha256,
            "Lyric model checksum mismatch: {}",
            path.display()
        );
        Ok(path)
    };
    let target = dir.join(&asset.filename);
    control.check()?;
    control.update("Verifying the selected local lyric model…");
    if target.exists() {
        return verify();
    }
    ensure!(
        download,
        "Selected lyric model is missing; allow its one-time download (about 190 MB)"
    );
    fs::create_dir_all(dir)?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(900))
        .user_agent("chart-engine/0.1")
        .build()?;
    control.update("Downloading the selected lyric model (190 MB, once)…");
    let mut response = client.get(&asset.url).send()?.error_for_status()?;
    let mut stage = tempfile::NamedTempFile::new_in(dir)?;
    let mut buffer = [0u8; 65536];
    let mut received = 0u64;
    loop {
        control.check()?;
        let count = response.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        stage.write_all(&buffer[..count])?;
        received += count as u64;
        control.update(format!(
            "Downloading the selected lyric model: {:.1} / 190 MB…",
            received as f64 / 1_000_000.
        ));
    }
    stage.flush()?;
    control.check()?;
    ensure!(
        models::hash_file(stage.path())? == asset.sha256,
        "Downloaded lyric model checksum mismatch; the model was not installed"
    );
    // Another window may have completed the same checksum-verified download.
    if let Err(error) = stage.persist_noclobber(&target)
        && !target.is_file()
    {
        return Err(error.error.into());
    }
    verify()
}

#[cfg(test)]
mod bundle_asset_tests {
    use super::*;
    #[test]
    fn invalid_bundled_lyric_model_is_never_repaired_in_place() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("YACE.app/Contents/Resources/models");
        fs::create_dir_all(&root).unwrap();
        let asset = crate::lyric_language::Language::Auto.asset();
        let path = root.join(asset.filename);
        fs::write(&path, b"retained invalid fixture").unwrap();
        let result = prepare_model(
            &root,
            true,
            crate::lyric_language::Language::Auto,
            &Control::default(),
        );
        assert!(result.is_err());
        assert_eq!(fs::read(path).unwrap(), b"retained invalid fixture");
        assert_eq!(fs::read_dir(root).unwrap().count(), 1);
    }
}
// Both callbacks borrow a Control kept alive by detect until synchronous full()
// returns. They never access Whisper state or unwind across the FFI boundary.
unsafe extern "C" fn progress(
    _: *mut whisper_rs::WhisperSysContext,
    _: *mut whisper_rs::WhisperSysState,
    percent: i32,
    data: *mut c_void,
) {
    let control = unsafe { &*(data as *const Control) };
    control.update(format!(
        "Recognizing lyrics and estimating word times: {}%…",
        percent.clamp(0, 100)
    ));
}
unsafe extern "C" fn abort(data: *mut c_void) -> bool {
    unsafe { &*(data as *const Control) }.is_cancelled()
}

/// Only a single lexical word with valid model-derived bounds receives timing.
/// Never divide a phrase duration evenly to manufacture individual word times.
fn aligned_words(segments: &[RecognizedSegment], duration: f64) -> Vec<AlignedWord> {
    let mut words = Vec::new();
    let mut previous_start = 0.;
    for segment in segments {
        let text = segment.text.trim();
        if text.starts_with('[') || text.starts_with('(') || text.starts_with("<|") {
            continue;
        }
        let lexical = clean::tokens(text);
        let start = segment.start_seconds;
        let end = segment.end_seconds.min(duration);
        let confident = segment.mean_token_probability.is_finite()
            && segment.mean_token_probability >= MIN_WORD_PROBABILITY
            && segment.no_speech_probability.is_finite()
            && segment.no_speech_probability < 0.6;
        let timed = confident
            && lexical.len() == 1
            && start.is_finite()
            && end.is_finite()
            && start >= previous_start
            && start >= 0.
            && start < end
            && start < duration;
        if timed {
            previous_start = start;
        }
        for word in lexical {
            words.push(AlignedWord {
                text: Some(word),
                start: timed.then_some(start),
                end: timed.then_some(end),
            });
        }
    }
    words
}

pub fn detect(
    request: Request<'_>,
    policy: &CleanPolicy,
    mode: CleanMode,
    control: Arc<Control>,
) -> Result<Detection> {
    detect_language(
        request,
        policy,
        mode,
        control,
        crate::lyric_language::Language::English,
    )
}

pub fn detect_language(
    request: Request<'_>,
    policy: &CleanPolicy,
    mode: CleanMode,
    control: Arc<Control>,
    language: crate::lyric_language::Language,
) -> Result<Detection> {
    policy.validate()?;
    control.check()?;
    control.update("Reading the recording…");
    let source_hash = models::hash_file(request.audio)?;
    // The renderer also uses 44.1 kHz. Count its exact frames instead of using
    // container duration or a differently rounded 16 kHz resampling duration.
    let original = models::decode(request.audio, 44100, 2)?;
    let frames = original.len() / 2;
    let duration = frames as f64 / 44100.;
    drop(original);
    control.check()?;
    let (input, input_hash) = if let Some((stem, binding)) = request.vocal_stem {
        ensure!(
            binding.audio_sha256.eq_ignore_ascii_case(&source_hash)
                && !binding.evidence.trim().is_empty(),
            "Vocal stem is not bound to this recording with alignment evidence"
        );
        let hash = models::hash_file(stem)?;
        ensure!(
            binding.stem_sha256.eq_ignore_ascii_case(&hash),
            "Vocal stem changed after binding"
        );
        let decoded = models::decode(stem, 44100, 2)?;
        ensure!(
            decoded.len() / 2 == frames,
            "Recognition vocal stem must match the full recording frame count"
        );
        (stem, hash)
    } else {
        (request.audio, source_hash.clone())
    };
    let model_path = prepare_model(
        request.model_dir,
        request.download_model,
        language,
        &control,
    )?;
    control.check()?;
    control.update("Preparing local lyric recognition…");
    let samples = models::decode(input, 16000, 1)?;
    ensure!(
        samples.len() >= 1600,
        "Recording is too short for lyric recognition"
    );
    whisper_rs::install_logging_hooks();
    let mut context_parameters = WhisperContextParameters::default();
    context_parameters.use_gpu(cfg!(target_os = "macos"));
    let context = WhisperContext::new_with_params(&model_path, context_parameters)
        .context("Cannot load the local lyric model")?;
    ensure!(
        language == crate::lyric_language::Language::English || context.is_multilingual(),
        "Selected language requires the multilingual model"
    );
    control.check()?;
    let mut state = context.create_state()?;
    let mut params = FullParams::new(SamplingStrategy::BeamSearch {
        beam_size: 5,
        patience: -1.,
    });
    params.set_n_threads(
        std::thread::available_parallelism()
            .map(|n| n.get().min(8))
            .unwrap_or(4) as i32,
    );
    params.set_language(language.code());
    params.set_translate(false);
    params.set_no_context(true);
    params.set_token_timestamps(true);
    params.set_max_len(1);
    params.set_split_on_word(true);
    params.set_suppress_blank(true);
    params.set_suppress_nst(true);
    params.set_temperature(0.);
    params.set_temperature_inc(0.);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    let user_data = Arc::as_ptr(&control) as *mut c_void;
    // SAFETY: callbacks only read the live Control through atomics/a mutex;
    // control outlives full() and is not moved or freed while callbacks run.
    unsafe {
        params.set_progress_callback(Some(progress));
        params.set_progress_callback_user_data(user_data);
        params.set_abort_callback(Some(abort));
        params.set_abort_callback_user_data(user_data);
    }
    control.check()?;
    control.update("Recognizing lyrics and estimating word times…");
    let result = state.full(params, &samples);
    control.check()?;
    result.context("Local lyric recognition failed")?;
    let mut segments = Vec::new();
    for segment in state.as_iter() {
        let mut probabilities = Vec::new();
        let mut token_start = i64::MAX;
        let mut token_end = -1;
        for index in 0..segment.n_tokens() {
            if let Some(token) = segment.get_token(index)
                && token.token_id() < context.token_eot()
            {
                probabilities.push(token.token_probability());
                let timing = token.token_data();
                if token.to_bytes()?.iter().any(u8::is_ascii_alphanumeric)
                    && timing.t0 >= 0
                    && timing.t1 > timing.t0
                {
                    token_start = token_start.min(timing.t0);
                    token_end = token_end.max(timing.t1);
                }
            }
        }
        let confidence = if probabilities.is_empty() {
            0.
        } else {
            probabilities.iter().sum::<f32>() / probabilities.len() as f32
        };
        segments.push(RecognizedSegment {
            text: segment.to_str()?.to_owned(),
            // Segment timestamps can include leading silence. Use the actual
            // timestamped lexical tokens, never the larger sentence interval.
            start_seconds: if token_end > token_start {
                token_start as f64 / 100.
            } else {
                -1.
            },
            end_seconds: if token_end > token_start {
                token_end as f64 / 100.
            } else {
                -1.
            },
            mean_token_probability: confidence,
            no_speech_probability: segment.no_speech_probability(),
        });
    }
    let asset = language.asset();
    let model_hash = asset.sha256;
    let detected_language = whisper_rs::get_lang_str(state.full_lang_id_from_state())
        .context("Recognition returned no language")?
        .to_owned();
    let backend = format!(
        "whisper-rs-0.16.0/whisper.cpp-{}; {}; requested_language={language:?}; token_timestamps; split_on_word; max_len=1; beam=5; temperature=0; min_word_probability={MIN_WORD_PROBABILITY}",
        whisper_rs::WHISPER_CPP_VERSION,
        asset.filename
    );
    let words = WordTimings {
        version: 1,
        audio_sha256: source_hash.clone(),
        duration_seconds: duration,
        source: format!(
            "Automatic local recognition; language={detected_language}; {backend}; model_sha256={model_hash}; recognition_audio_sha256={input_hash}; model-estimated word boundaries; listening review required"
        ),
        words: aligned_words(&segments, duration),
    };
    words.validate()?;
    control.update("Matching explicit words and preparing editable ranges…");
    let warning = if language == crate::lyric_language::Language::English {
        RECOGNITION_WARNING
    } else {
        "Automatic multilingual lyric recognition and word timing are estimates. Singing, slang, language changes and backing vocals need listening review. The English explicit-word preset does not certify content in other languages."
    };
    let mut plan = clean::plan(&words, policy, mode)?;
    plan.warnings.push(warning.into());
    if plan.ranges.is_empty() {
        plan.warnings.push("No timed explicit matches were found. Check the recognized lyrics and listen for missed content; no clean audio was certified.".into());
    }
    ensure!(
        models::hash_file(request.audio)? == source_hash && models::hash_file(input)? == input_hash,
        "Recording or vocal stem changed during lyric recognition; results were discarded"
    );
    ensure!(
        models::hash_file(&model_path)? == model_hash,
        "Lyric model changed during recognition; results were discarded"
    );
    control.check()?;
    let warnings = vec![
        warning.into(),
        format!(
            "{} recognized words have uncertain or unavailable timing; no mute range was invented for them.",
            plan.untimed_words
        ),
    ];
    Ok(Detection {
        recognition: Recognition {
            version: 1,
            backend,
            model_sha256: model_hash,
            recognition_audio_sha256: input_hash,
            language: detected_language,
            words,
            segments,
            warnings,
        },
        plan,
    })
}

fn write_detection(dir: &Path, detection: &Detection) -> Result<()> {
    detection.recognition.words.validate()?;
    detection.plan.validate()?;
    ensure!(
        detection.plan.audio_sha256 == detection.recognition.words.audio_sha256,
        "Recognition and plan belong to different recordings"
    );
    for (name, value) in [
        (
            "words.json",
            serde_json::to_vec_pretty(&detection.recognition.words)?,
        ),
        ("plan.json", serde_json::to_vec_pretty(&detection.plan)?),
        (
            "recognition.json",
            serde_json::to_vec_pretty(&detection.recognition)?,
        ),
    ] {
        fs::write(dir.join(name), value)?;
    }
    Ok(())
}

/// Export the generated inputs for recovery/automation; users need not supply JSON.
pub fn save(output: &Path, detection: &Detection) -> Result<()> {
    pipeline::destination(output, None)?;
    let stage = tempfile::tempdir_in(
        output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    write_detection(stage.path(), detection)?;
    pipeline::publish(stage.path(), output)
}

pub fn save_run(parent: &Path, detection: &Detection) -> Result<PathBuf> {
    fs::create_dir_all(parent)?;
    let stage = tempfile::Builder::new()
        .prefix("automatic-")
        .tempdir_in(parent)?;
    write_detection(stage.path(), detection)?;
    Ok(stage.keep())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn segment(text: &str, start: f64, end: f64, probability: f32) -> RecognizedSegment {
        RecognizedSegment {
            text: text.into(),
            start_seconds: start,
            end_seconds: end,
            mean_token_probability: probability,
            no_speech_probability: 0.1,
        }
    }
    #[test]
    fn model_words_receive_only_valid_confident_individual_timing() {
        let segments = vec![
            segment(" FUCK!", 0.1, 0.4, 0.9),
            segment("blow job", 0.5, 1., 0.9),
            segment("shit", 1., 1.2, 0.1),
            segment("[Music]", 1.3, 1.4, 0.9),
            segment("bitch", 1.5, 1.5, 0.9),
            segment("ass", 2., 3.01, 0.9),
        ];
        let words = aligned_words(&segments, 3.);
        assert_eq!(words.len(), 6);
        assert_eq!(
            (words[0].text.as_deref(), words[0].start, words[0].end),
            (Some("fuck"), Some(0.1), Some(0.4))
        );
        assert!(
            words[1..5]
                .iter()
                .all(|w| w.start.is_none() && w.end.is_none())
        );
        assert_eq!(words[5].end, Some(3.));
        let input = WordTimings {
            version: 1,
            audio_sha256: "a".repeat(64),
            duration_seconds: 3.,
            source: "Model timing regression".into(),
            words,
        };
        let plan = clean::plan(&input, &CleanPolicy::default(), CleanMode::FullMix).unwrap();
        assert_eq!(plan.matches.len(), 5);
        assert_eq!(plan.ranges.len(), 2);
        assert_eq!(plan.untimed_words, 4);
    }
    #[test]
    fn spanish_words_keep_accents_and_do_not_receive_fabricated_phrase_timing() {
        let words = aligned_words(
            &[
                segment("sonríele", 0.1, 0.5, 0.9),
                segment("a la vida", 0.6, 1.2, 0.9),
            ],
            2.,
        );
        assert_eq!(words[0].text.as_deref(), Some("sonríele"));
        assert_eq!(words[0].start, Some(0.1));
        assert!(
            words[1..]
                .iter()
                .all(|word| word.start.is_none() && word.end.is_none())
        );
    }
    #[test]
    fn reversed_nonfinite_and_silent_segments_never_get_mute_times() {
        let mut silent = segment("fuck", 1., 2., 0.9);
        silent.no_speech_probability = 0.9;
        let words = aligned_words(
            &[
                segment("shit", 1., 2., 0.9),
                segment("fuck", 0.5, 0.8, 0.9),
                segment("ass", f64::NAN, 2., 0.9),
                silent,
            ],
            3.,
        );
        assert!(words[0].start.is_some());
        assert!(words[1..].iter().all(|w| w.start.is_none()));
    }
    #[test]
    fn cancellation_stops_before_touching_audio_or_downloading_models() {
        let control = Arc::new(Control::default());
        control.cancel();
        let error = detect(
            Request {
                audio: Path::new("missing-recording"),
                model_dir: Path::new("missing-models"),
                download_model: true,
                vocal_stem: None,
            },
            &CleanPolicy::default(),
            CleanMode::FullMix,
            control,
        )
        .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
    }
}
