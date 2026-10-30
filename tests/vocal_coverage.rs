#![cfg(feature = "vocal-alignment")]
//! Explicit recording probes; no copyrighted audio/lyrics are part of the suite.
use anyhow::{Context, Result, ensure};
use chart_engine::{
    chart::Chart,
    game_export, models, pipeline, vocal_alignment,
    vocal_generation::{self, Generation, LyricSource, PitchSupport, Policy},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
struct Emissions {
    audio_sha256: String,
    stem_sha256: String,
    model_sha256: String,
    implementation_sha256: String,
    duration_seconds: f64,
    values: Vec<Vec<f32>>,
}
fn implementation_hash() -> String {
    let mut hash = Sha256::new();
    for bytes in [
        include_bytes!("../vendor/practice-acoustic/src/lib.rs").as_slice(),
        include_bytes!("../vendor/practice-acoustic/src/config.rs"),
        include_bytes!("../vendor/practice-acoustic/src/model/ctc_model.rs"),
        include_bytes!("../vendor/practice-acoustic/src/model/encoder.rs"),
        include_bytes!("../vendor/practice-acoustic/src/model/feature_extractor.rs"),
        include_bytes!("../vendor/practice-acoustic/src/model/layers.rs"),
        include_bytes!("../vendor/practice-acoustic/src/model/mod.rs"),
        include_bytes!("../vendor/practice-acoustic/src/model/feature_projection.rs"),
        include_bytes!("../Cargo.lock"),
    ] {
        hash.update(bytes);
    }
    format!("{:x}", hash.finalize())
}
fn write(path: &Path, value: &impl Serialize) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

#[test]
#[ignore = "frozen paired policy evaluation using source-bound identical words/neural proposals; no target input"]
fn compare_cached_vocal_pitch_policies() -> Result<()> {
    let path = |key: &str| -> Result<PathBuf> {
        Ok(std::env::var(key)
            .with_context(|| format!("Set {key}"))?
            .into())
    };
    let prior: Generation =
        serde_json::from_slice(&fs::read(path("CHART_ENGINE_VOCAL_BASELINE")?)?)?;
    let stem = path("CHART_ENGINE_VOCAL_STEM")?;
    let source = models::hash_file(&stem)?;
    ensure!(
        prior.audio_sha256 == source && prior.vocal_stem_sha256 == source,
        "This runner requires the explicitly labeled oracle-vocal source condition"
    );
    let timing = prior.timing.as_ref().context("Exact prior source clock")?;
    ensure!(
        timing.offset == 0.,
        "Oracle analysis MIDI uses source-zero clock"
    );
    let output = path("CHART_ENGINE_VOCAL_OUTPUT")?;
    ensure!(!output.exists(), "Choose new output");
    fs::create_dir(&output)?;
    let model_dir = path("CHART_ENGINE_VOCAL_MODELS")?;
    let mut neural = prior.neural_pitch.clone();
    let mut refreshed = false;
    if let Some(e) = &mut neural {
        let model = models::verified(&model_dir, "pitch")?;
        ensure!(
            models::hash_file(&model)? == e.model_sha256,
            "Cannot compare different pitch weights"
        );
        if e.implementation_sha256 != vocal_generation::neural_pitch_implementation_hash() {
            let notes = models::pitch(&stem, &model)?;
            ensure!(
                serde_json::to_value(&notes)? == serde_json::to_value(&e.notes)?,
                "Decoder output differs from frozen proposals; paired comparison must stop"
            );
            e.notes = notes;
            e.implementation_sha256 = vocal_generation::neural_pitch_implementation_hash();
            refreshed = true;
        }
        write(&output.join("neural-evidence.json"), e)?;
    }
    let binding = chart_engine::clean::StemAlignment {
        audio_sha256: source.clone(),
        stem_sha256: source.clone(),
        evidence: "Explicit oracle-vocal source condition; no authored timing/notes supplied"
            .into(),
    };
    let evidence = if let Ok(cache) = std::env::var("CHART_ENGINE_VOCAL_FRAME_EVIDENCE") {
        serde_json::from_slice::<chart_engine::vocal_pitch::Evidence>(&fs::read(cache)?)?
    } else {
        chart_engine::vocal_pitch::analyze(
            &stem,
            &stem,
            &binding,
            &models::verified(&model_dir, "vocal-pitch")?,
            &|| false,
        )?
    };
    write(&output.join("vocal-frame-evidence.json"), &evidence)?;
    let mut expected_words = None;
    for (name, energy, swift) in [
        ("baseline", false, false),
        ("local-energy", true, false),
        ("swift-local-energy", true, true),
    ] {
        let policy = Policy {
            local_energy_gate: energy,
            ..prior.policy.clone()
        };
        let result = vocal_generation::generate(vocal_generation::Request {
            audio: &stem,
            vocal_stem: &stem,
            alignment: &binding,
            timing,
            words: prior.words.as_ref(),
            lyrics: prior.lyrics.as_ref(),
            policy: &policy,
            neural_pitch: neural.as_ref(),
            vocal_pitch: swift.then_some(&evidence),
        })?;
        let words = serde_json::to_value(&result.words)?;
        if let Some(expected) = &expected_words {
            ensure!(&words == expected, "Word alignment changed across policies");
        } else {
            expected_words = Some(words);
        }
        if name == "baseline" {
            ensure!(
                serde_json::to_value(prior.chart.as_ref().map(|c| (&c.notes, &c.phrases)))?
                    == serde_json::to_value(result.chart.as_ref().map(|c| (&c.notes, &c.phrases)))?,
                "Fresh baseline note/phrase output changed; paired comparison must stop"
            );
        }
        write(&output.join(format!("{name}.json")), &result)?;
        if let Some(chart) = &result.chart {
            let midi = chart_engine::midi::new_midi(timing, &chart_engine::chart::Tracks::new())?;
            fs::write(
                output.join(format!("{name}.mid")),
                chart_engine::vocals::append_midi(&midi, chart)?,
            )?;
        }
    }
    write(
        &output.join("condition.json"),
        &serde_json::json!({"condition":std::env::var("CHART_ENGINE_VOCAL_EVAL_CONDITION").unwrap_or_else(|_|"source-bound fixed-word oracle comparison".into()),"source_sha256":source,"original_words_sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(&prior.words)?)),"neural_refreshed_with_exact_raw_proposal_match":refreshed,"authored_timing_or_note_input":false,"swift_confidence":0.8,"veto":"both raw DSP pairwise and each vs Swift >0.55 semitone","stable":"3of5 model frames within0.7semi","period_correlation":0.75,"fundamental_contrast":0.15,"all_new_frames_flagged":true,"algorithm":vocal_generation::ALGORITHM}),
    )?;
    Ok(())
}

#[test]
#[ignore = "post-inference development audit against independent human MIDI; never supplies reference to generator"]
fn inspect_development_reference_pitch_stages() -> Result<()> {
    let report: Generation =
        serde_json::from_slice(&fs::read(std::env::var("CHART_ENGINE_VOCAL_BASELINE")?)?)?;
    let bytes = fs::read(std::env::var("CHART_ENGINE_VOCAL_REFERENCE_MIDI")?)?;
    let delay = std::env::var("CHART_ENGINE_VOCAL_REFERENCE_DELAY_MS")
        .unwrap_or_else(|_| "0".into())
        .parse::<f64>()?;
    let reference =
        chart_engine::benchmark::parse(&bytes, chart_engine::benchmark::Instrument::Vocals, delay)?;
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    let mut missed = vec![];
    for frame in &report.pitch_frames {
        let Some(note) = reference.notes.iter().find(|n| {
            n.pitch.is_some()
                && n.start + 0.04 <= frame.time_seconds
                && frame.time_seconds < n.end - 0.04
        }) else {
            continue;
        };
        let expected = note.pitch.unwrap() as f64;
        let near = |pitch: Option<f64>| pitch.is_some_and(|p| (p - expected).abs() <= 0.6);
        *counts.entry("reference_interior_frames").or_default() += 1;
        let accepted = near(frame.midi);
        let yin = near(frame.yin_midi);
        let mcleod = near(frame.mcleod_midi);
        let neural = report.neural_pitch.as_ref().is_some_and(|e| {
            e.notes.iter().any(|n| {
                n.start <= frame.time_seconds
                    && frame.time_seconds < n.end
                    && (n.pitch as f64 - expected).abs() <= 0.5
                    && n.confidence >= 0.6
            })
        });
        let known = report.words.as_ref().is_some_and(|w| {
            w.words.iter().any(|w| {
                w.start
                    .zip(w.end)
                    .is_some_and(|(s, e)| s <= frame.time_seconds && frame.time_seconds < e)
            })
        });
        for (label, ok) in [
            ("accepted_near_reference", accepted),
            ("yin_near_reference", yin),
            ("mcleod_near_reference", mcleod),
            ("neural_near_reference", neural),
            ("has_timed_word", known),
            (
                "yin_only_correct_mcleod_wrong",
                yin && !mcleod && frame.mcleod_midi.is_some(),
            ),
            (
                "mcleod_only_correct_yin_wrong",
                mcleod && !yin && frame.yin_midi.is_some(),
            ),
            (
                "both_estimates_missing",
                frame.yin_midi.is_none() && frame.mcleod_midi.is_none(),
            ),
            ("accepted_wrong", frame.midi.is_some() && !accepted),
        ] {
            if ok {
                *counts.entry(label).or_default() += 1;
            }
        }
        if !accepted {
            missed.push(serde_json::json!({"time_seconds":frame.time_seconds,"reference_midi":expected,"yin":frame.yin_midi,"mcleod":frame.mcleod_midi,"accepted":frame.midi,"neural_matches":neural,"known_word":known,"rms":frame.rms}));
        }
    }
    let output = std::env::var("CHART_ENGINE_VOCAL_OUTPUT")?;
    ensure!(!Path::new(&output).exists(), "Choose new audit output");
    write(
        Path::new(&output),
        &serde_json::json!({"condition":"post-inference human-reference diagnostic; 40ms boundary margin; no reference input to generator","reference_midi_sha256":models::hash_file(Path::new(&std::env::var("CHART_ENGINE_VOCAL_REFERENCE_MIDI")?))?,"source_sha256":report.audio_sha256,"duration_seconds":report.duration_seconds,"counts":counts,"missing_frames":missed}),
    )?;
    eprintln!("{}", serde_json::to_string_pretty(&counts)?);
    Ok(())
}
fn summary(report: &Generation, chart: &Chart) -> serde_json::Value {
    let notes = report
        .chart
        .as_ref()
        .map(|c| c.notes.as_slice())
        .unwrap_or(&[]);
    serde_json::json!({"status":report.status,"words":report.words.as_ref().map_or(0,|words|words.words.len()),
        "timed_words":report.words.as_ref().map_or(0,|words|words.words.iter().filter(|word|word.start.is_some()).count()),
        "pitch_candidates":report.pitch_candidates.len(),"candidate_seconds":report.pitch_candidates.iter().map(|n|n.end_seconds-n.start_seconds).sum::<f64>(),
        "recovered_frames":report.pitch_frames.iter().filter(|frame|frame.support==PitchSupport::AnchoredYin).count(),
        "notes":notes.len(),"phrases":report.chart.as_ref().map_or(0,|c|c.phrases.len()),
        "charted_seconds":notes.iter().map(|note|chart.timing.seconds(note.end)-chart.timing.seconds(note.start)).sum::<f64>(),
        "issues":report.issues.len(),"word_source":report.words.as_ref().map(|words|words.source.as_str()),
        "musical_acceptance":"unverified; broader proposal coverage is not demonstrated transcription accuracy"})
}

#[test]
#[ignore = "requires explicit source-bound cached acoustic emissions and provider lyrics"]
fn select_cached_source_bound_alignment() -> Result<()> {
    let path = |key: &str| -> Result<PathBuf> {
        Ok(std::env::var(key)
            .with_context(|| format!("Set {key}"))?
            .into())
    };
    let song = path("CHART_ENGINE_VOCAL_SONG")?;
    let stem = path("CHART_ENGINE_VOCAL_STEM")?;
    let lyrics: LyricSource =
        serde_json::from_slice(&fs::read(path("CHART_ENGINE_VOCAL_LYRIC_SOURCE")?)?)?;
    let cache: Emissions =
        serde_json::from_slice(&fs::read(path("CHART_ENGINE_VOCAL_EMISSIONS_CACHE")?)?)?;
    let output = path("CHART_ENGINE_VOCAL_OUTPUT")?;
    ensure!(!output.exists(), "Choose a new output directory");
    let directory = vocal_alignment::verified(&path("CHART_ENGINE_VOCAL_MODELS")?)?;
    ensure!(
        models::hash_file(&song.join("song.ogg"))? == cache.audio_sha256
            && models::hash_file(&stem)? == cache.stem_sha256
            && cache.model_sha256 == vocal_alignment::assets()[2].sha256
            && cache.implementation_sha256 == implementation_hash(),
        "Stale or wrong acoustic emission cache"
    );
    let vocabulary = serde_json::from_slice(&fs::read(directory.join("vocab.json"))?)?;
    let (words, diagnostics, selection) = vocal_alignment::align_emissions_with_selection(
        &cache.values,
        &vocabulary,
        &lyrics,
        &cache.audio_sha256,
        cache.duration_seconds,
        &|| false,
    )?;
    fs::create_dir(&output)?;
    write(&output.join("selected-words.json"), &words)?;
    write(&output.join("selected-diagnostics.json"), &diagnostics)?;
    write(&output.join("selection.json"), &selection)?;
    eprintln!("{}", serde_json::to_string_pretty(&selection)?);
    Ok(())
}

#[test]
#[ignore = "requires explicit original recording, aligned stem, provider lyrics, pinned models; run optimized"]
fn compare_source_bound_provider_vocal_coverage() -> Result<()> {
    let env = |key: &str| -> Result<PathBuf> {
        Ok(PathBuf::from(
            std::env::var(key).with_context(|| format!("Set {key}"))?,
        ))
    };
    let song = env("CHART_ENGINE_VOCAL_SONG")?;
    let stem = env("CHART_ENGINE_VOCAL_STEM")?;
    let lyrics_path = env("CHART_ENGINE_VOCAL_LYRIC_SOURCE")?;
    let model_dir = env("CHART_ENGINE_VOCAL_MODELS")?;
    let output = env("CHART_ENGINE_VOCAL_OUTPUT")?;
    ensure!(!output.exists(), "Choose a new output directory");
    let audio = song.join("song.ogg");
    let chart = Chart::parse(fs::read_to_string(song.join("notes.chart"))?)?;
    let lyrics: LyricSource = serde_json::from_slice(&fs::read(&lyrics_path)?)?;
    let directory = vocal_alignment::verified(&model_dir)?;
    let model_hash = vocal_alignment::assets()[2].sha256;
    let audio_hash = models::hash_file(&audio)?;
    let stem_hash = models::hash_file(&stem)?;
    let duration = models::decode(&audio, 44100, 2)?.len() as f64 / 88200.;
    ensure!(
        (models::decode(&stem, 44100, 2)?.len() as f64 / 88200. - duration).abs() < 1. / 44100.,
        "Stem sample clock mismatch"
    );
    let implementation_hash = implementation_hash();
    let emissions = if let Ok(cache) = std::env::var("CHART_ENGINE_VOCAL_EMISSIONS_CACHE") {
        let cache: Emissions = serde_json::from_slice(&fs::read(cache)?)?;
        ensure!(
            cache.audio_sha256 == audio_hash
                && cache.stem_sha256 == stem_hash
                && cache.model_sha256 == model_hash
                && cache.implementation_sha256 == implementation_hash
                && (cache.duration_seconds - duration).abs() < 1. / 44100.,
            "Cached acoustic emissions belong to another recording/model/implementation"
        );
        cache
    } else {
        let samples = models::decode(&stem, 16000, 1)?;
        let model = practice_acoustic::AcousticModel::load(&directory)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let values = model
            .emissions(&samples)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Emissions {
            audio_sha256: audio_hash.clone(),
            stem_sha256: stem_hash.clone(),
            model_sha256: model_hash.into(),
            implementation_sha256: implementation_hash,
            duration_seconds: duration,
            values,
        }
    };
    fs::create_dir(&output)?;
    write(&output.join("emissions.json"), &emissions)?;
    let vocabulary: HashMap<String, usize> =
        serde_json::from_slice(&fs::read(directory.join("vocab.json"))?)?;
    let mut plain = lyrics.clone();
    plain.synced_lyrics = None;
    let (global_words, global_diagnostics) = vocal_alignment::align_emissions(
        &emissions.values,
        &vocabulary,
        &plain,
        &audio_hash,
        duration,
        &|| false,
    )?;
    let (guided_words, guided_diagnostics) = vocal_alignment::align_emissions(
        &emissions.values,
        &vocabulary,
        &lyrics,
        &audio_hash,
        duration,
        &|| false,
    )?;
    write(&output.join("global-words.json"), &global_words)?;
    write(&output.join("global-diagnostics.json"), &global_diagnostics)?;
    write(&output.join("guided-words.json"), &guided_words)?;
    write(&output.join("guided-diagnostics.json"), &guided_diagnostics)?;
    let signal = models::decode(&stem, 22050, 1)?;
    let strict = Policy {
        recover_bounded_yin: false,
        ..Policy::default()
    };
    let before = vocal_generation::generate_pcm(
        &signal,
        &audio_hash,
        &stem_hash,
        duration,
        &chart.timing,
        Some(&global_words),
        Some(&plain),
        &strict,
    )?;
    let recovered = vocal_generation::generate_pcm(
        &signal,
        &audio_hash,
        &stem_hash,
        duration,
        &chart.timing,
        Some(&global_words),
        Some(&plain),
        &Policy::default(),
    )?;
    let guided = vocal_generation::generate_pcm(
        &signal,
        &audio_hash,
        &stem_hash,
        duration,
        &chart.timing,
        Some(&guided_words),
        Some(&lyrics),
        &Policy::default(),
    )?;
    for (name, report) in [
        ("strict-global-generation", &before),
        ("recovered-global-generation", &recovered),
        ("recovered-guided-generation", &guided),
    ] {
        write(&output.join(format!("{name}.json")), report)?;
    }
    let metrics = serde_json::json!({"audio_sha256":audio_hash,"vocal_stem_sha256":stem_hash,"lyric_file_sha256":models::hash_file(&lyrics_path)?,
        "strict_global":summary(&before,&chart),"recovered_global":summary(&recovered,&chart),"recovered_guided":summary(&guided,&chart)});
    write(&output.join("comparison.json"), &metrics)?;
    let vocals = guided
        .chart
        .as_ref()
        .context("No playable lyric-bearing notes in this explicit fixture")?;
    vocals.validate()?;
    write(&output.join("vocals.json"), vocals)?;
    let game = output.join("game");
    fs::create_dir(&game)?;
    let (midi, delay) = game_export::song_with_vocals(&chart, vocals)?;
    fs::write(game.join("notes.mid"), midi)?;
    fs::write(
        game.join("song.ini"),
        game_export::ini_with_delay(&pipeline::song_ini(&song)?, delay),
    )?;
    fs::copy(&audio, game.join("song.ogg"))?;
    ensure!(
        models::hash_file(&audio)? == audio_hash && models::hash_file(&stem)? == stem_hash,
        "Sources changed during acoustic/DSP probe"
    );
    eprintln!("{}", serde_json::to_string_pretty(&metrics)?);
    Ok(())
}
