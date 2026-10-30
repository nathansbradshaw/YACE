use chart_engine::{
    clean::{
        self, AlignedWord, CleanMode, CleanPlan, CleanPolicy, MuteRange, RenderRequest,
        StemAlignment, WordTimings,
    },
    models,
};

fn policy(terms: &[&str]) -> CleanPolicy {
    CleanPolicy {
        version: 1,
        name: "User-selected test policy".into(),
        terms: terms.iter().map(|s| (*s).into()).collect(),
        padding_before_seconds: 0.,
        padding_after_seconds: 0.,
        fade_seconds: 0.,
    }
}
fn words(text: &[&str], duration: f64) -> WordTimings {
    WordTimings {
        version: 1,
        audio_sha256: "a".repeat(64),
        duration_seconds: duration,
        source: "Manually supplied test words; not ASR".into(),
        words: text
            .iter()
            .enumerate()
            .map(|(i, s)| AlignedWord {
                text: Some((*s).into()),
                start: Some(i as f64),
                end: Some(i as f64 + 0.5),
            })
            .collect(),
    }
}

#[test]
fn default_explicit_markers_match_common_variants_without_censoring_substrings_or_mild_words() {
    let input = words(
        &[
            "FUCKING!",
            "motherfuckin’",
            "bullshit",
            "bitches",
            "niggas",
            "blow",
            "job",
            "class",
            "bass",
            "Scunthorpe",
            "cocktail",
            "Dickinson",
            "hell",
            "damn",
            "sex",
            "drugs",
            "kill",
            "heck",
        ],
        20.,
    );
    let policy = CleanPolicy::default();
    policy.validate().unwrap();
    let normalized: std::collections::HashSet<_> = policy
        .terms
        .iter()
        .map(|term| clean::tokens(term))
        .collect();
    assert_eq!(
        normalized.len(),
        policy.terms.len(),
        "Preset terms must not produce duplicate normalized matches"
    );
    let plan = clean::plan(&input, &policy, CleanMode::VocalOnly).unwrap();
    assert_eq!(
        plan.matches
            .iter()
            .map(|m| (m.term.as_str(), m.first_word, m.end_word_exclusive))
            .collect::<Vec<_>>(),
        [
            ("fucking", 0, 1),
            ("motherfuckin", 1, 2),
            ("bullshit", 2, 3),
            ("bitches", 3, 4),
            ("niggas", 4, 5),
            ("blow job", 5, 7),
        ]
    );
    assert_eq!(plan.policy, policy);
    assert_eq!(plan.ranges.len(), 6);
    assert!((plan.ranges[0].end - 0.56).abs() < 1e-9);
    assert!((plan.ranges[1].start - 0.96).abs() < 1e-9);
    let restored: CleanPlan = serde_json::from_slice(&serde_json::to_vec(&plan).unwrap()).unwrap();
    assert_eq!(restored, plan);
}

#[test]
fn exact_unicode_words_contractions_and_phrases_avoid_substring_false_positives() {
    let input = words(
        &[
            "CLASS",
            "bass",
            "ass!",
            "Scunthorpe",
            "shitake",
            "Holy,",
            "SHIT!",
            "CAN’T",
            "CAFÉ",
        ],
        10.,
    );
    let plan = clean::plan(
        &input,
        &policy(&["ass", "shit", "holy shit", "can't", "café"]),
        CleanMode::FullMix,
    )
    .unwrap();
    assert_eq!(
        plan.matches
            .iter()
            .map(|m| (m.term.as_str(), m.first_word, m.end_word_exclusive))
            .collect::<Vec<_>>(),
        [
            ("ass", 2, 3),
            ("holy shit", 5, 7),
            ("shit", 6, 7),
            ("can't", 7, 8),
            ("café", 8, 9)
        ]
    );
    assert_eq!(clean::tokens("word/other-word"), ["word", "other", "word"]);
}

#[test]
fn unknown_text_blocks_phrases_and_unknown_timing_never_gets_guessed() {
    let mut input = words(&["holy", "gap", "shit", "ass"], 5.);
    input.words[1].text = None;
    input.words[2].start = None;
    input.words[2].end = None;
    let plan = clean::plan(
        &input,
        &policy(&["holy shit", "shit", "ass"]),
        CleanMode::FullMix,
    )
    .unwrap();
    assert_eq!(plan.unknown_words, 1);
    assert_eq!(plan.untimed_words, 1);
    assert_eq!(plan.matches.len(), 2);
    assert_eq!(plan.matches[0].start, None);
    assert_eq!(plan.ranges.len(), 1);
    assert_eq!(plan.ranges[0].start, 3.);
}

#[test]
fn padding_clips_and_enabled_overlapping_ranges_merge_without_double_subtraction() {
    let mut selected = policy(&["one", "one two", "two"]);
    selected.padding_before_seconds = 1.;
    selected.padding_after_seconds = 1.;
    let mut plan =
        clean::plan(&words(&["one", "two"], 2.), &selected, CleanMode::VocalOnly).unwrap();
    assert_eq!(clean::merged_ranges(&plan).unwrap(), [(0., 2.)]);
    let original = vec![0.7; 20];
    let stem = vec![0.2; 20];
    let rendered = clean::render_pcm_stereo(&original, Some(&stem), 5, &plan).unwrap();
    assert!(rendered.iter().all(|x| (*x - 0.5).abs() < 1e-6));
    for range in &mut plan.ranges {
        range.enabled = false;
    }
    assert!(clean::render_pcm_stereo(&original, Some(&stem), 5, &plan).is_err());
}

#[test]
fn fades_extend_outside_full_mute_and_channels_remain_distinct() {
    let mut plan =
        clean::plan(&words(&["one"], 1.), &policy(&["one"]), CleanMode::FullMix).unwrap();
    plan.ranges = vec![MuteRange {
        start: 0.4,
        end: 0.6,
        enabled: true,
        reason: "Manual precise edit".into(),
    }];
    plan.policy.fade_seconds = 0.2;
    let original: Vec<_> = (0..10).flat_map(|_| [0.8, -0.4]).collect();
    let rendered = clean::render_pcm_stereo(&original, None, 10, &plan).unwrap();
    assert_eq!(rendered.len(), original.len());
    assert_eq!(&rendered[..4], &original[..4]);
    assert_eq!(&rendered[16..], &original[16..]);
    assert!((rendered[6] - 0.4).abs() < 1e-6);
    assert!((rendered[7] + 0.2).abs() < 1e-6);
    assert_eq!(&rendered[8..12], &[0., -0., 0., -0.]);
    assert!((rendered[14] - 0.4).abs() < 1e-6);
}

#[test]
fn vocal_only_retains_synthetic_accompaniment_and_never_falls_back_to_full_mix() {
    let mut plan = clean::plan(
        &words(&["one"], 1.),
        &policy(&["one"]),
        CleanMode::VocalOnly,
    )
    .unwrap();
    plan.ranges = vec![MuteRange {
        start: 0.2,
        end: 0.5,
        enabled: true,
        reason: "Test word".into(),
    }];
    let accompaniment: Vec<f32> = (0..10).flat_map(|i| [i as f32 * 0.01, -0.3]).collect();
    let stem: Vec<f32> = (0..10).flat_map(|_| [0.2, 0.4]).collect();
    let original: Vec<_> = accompaniment
        .iter()
        .zip(&stem)
        .map(|(a, v)| a + v)
        .collect();
    let output = clean::render_pcm_stereo(&original, Some(&stem), 10, &plan).unwrap();
    for (i, sample) in output.iter().enumerate() {
        let expected = if (4..10).contains(&i) {
            accompaniment[i]
        } else {
            original[i]
        };
        assert!((*sample - expected).abs() < 1e-6);
    }
    assert!(clean::render_pcm_stereo(&original, None, 10, &plan).is_err());
    assert!(clean::render_pcm_stereo(&original, Some(&stem[..18]), 10, &plan).is_err());
    plan.mode = CleanMode::FullMix;
    assert!(clean::render_pcm_stereo(&original, Some(&stem), 10, &plan).is_err());
}

#[test]
fn malformed_ranges_word_timings_and_nonfinite_audio_are_rejected() {
    let valid = words(&["one", "two"], 2.);
    for (start, end) in [
        (f64::NAN, 1.),
        (0., f64::INFINITY),
        (-0.1, 0.5),
        (1., 1.),
        (1., 3.),
    ] {
        let mut bad = valid.clone();
        bad.words[0].start = Some(start);
        bad.words[0].end = Some(end);
        assert!(bad.validate().is_err());
    }
    let mut bad = valid.clone();
    bad.words[0].end = None;
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.words[0].text = Some("two words".into());
    assert!(bad.validate().is_err());
    let mut bad = valid.clone();
    bad.words.swap(0, 1);
    assert!(bad.validate().is_err());
    let mut bad = policy(&["one"]);
    bad.fade_seconds = f64::NAN;
    assert!(bad.validate().is_err());
    let mut plan = clean::plan(&valid, &policy(&["one"]), CleanMode::FullMix).unwrap();
    assert!(clean::render_pcm_stereo(&[f32::NAN; 40], None, 10, &plan).is_err());
    assert!(clean::render_pcm_stereo(&[0.; 39], None, 10, &plan).is_err());
    assert!(clean::render_pcm_stereo(&[0.; 40], None, 0, &plan).is_err());
    assert!(clean::render_pcm_stereo(&[0.; 20], None, 10, &plan).is_err());
    plan.ranges[0].end = f64::NAN;
    assert!(plan.validate().is_err());
}

#[test]
fn stereo_wav_render_is_transactional_source_bound_and_does_not_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let audio = temp.path().join("original.wav");
    let stem = temp.path().join("vocals.wav");
    let left = vec![0.7; 4410];
    let right = vec![-0.2; 4410];
    models::write_audio(&audio, &left, &right).unwrap();
    models::write_audio(&stem, &vec![0.2; 4410], &vec![0.1; 4410]).unwrap();
    let original_bytes = std::fs::read(&audio).unwrap();
    let original_hash = models::hash_file(&audio).unwrap();
    let stem_hash = models::hash_file(&stem).unwrap();
    let mut input = words(&["one"], 0.1);
    input.words[0].end = Some(0.1);
    input.audio_sha256 = original_hash.clone();
    let plan = clean::plan(&input, &policy(&["one"]), CleanMode::VocalOnly).unwrap();
    let alignment = StemAlignment {
        audio_sha256: original_hash.clone(),
        stem_sha256: stem_hash,
        evidence: "Synthetic sample-aligned source plus vocal stem".into(),
    };
    let output = temp.path().join("clean-copy");
    let receipt = clean::render(RenderRequest {
        audio: &audio,
        plan: &plan,
        vocal_stem: Some(&stem),
        stem_alignment: Some(&alignment),
        output: &output,
    })
    .unwrap();
    assert_eq!(receipt.channels, 2);
    assert_eq!(receipt.frames, 4410);
    assert_eq!(receipt.sample_rate, 44100);
    let rendered = models::decode(&output.join("clean.wav"), 44100, 2).unwrap();
    for sample in rendered.chunks_exact(2) {
        assert!((sample[0] - 0.5).abs() < 1e-6);
        assert!((sample[1] + 0.3).abs() < 1e-6);
    }
    assert!(output.join("receipt.json").is_file());
    assert!(output.join("plan.json").is_file());
    let produced = std::fs::read(output.join("clean.wav")).unwrap();
    assert!(
        clean::render(RenderRequest {
            audio: &audio,
            plan: &plan,
            vocal_stem: Some(&stem),
            stem_alignment: Some(&alignment),
            output: &output
        })
        .is_err()
    );
    assert_eq!(std::fs::read(output.join("clean.wav")).unwrap(), produced);
    assert_eq!(std::fs::read(&audio).unwrap(), original_bytes);
    let refused = temp.path().join("stale-plan");
    let mut stale = plan.clone();
    stale.audio_sha256 = "b".repeat(64);
    assert!(
        clean::render(RenderRequest {
            audio: &audio,
            plan: &stale,
            vocal_stem: Some(&stem),
            stem_alignment: Some(&alignment),
            output: &refused
        })
        .is_err()
    );
    assert!(!refused.exists());
    let refused = temp.path().join("bad-alignment");
    let mut stale_alignment = alignment.clone();
    stale_alignment.stem_sha256 = "c".repeat(64);
    assert!(
        clean::render(RenderRequest {
            audio: &audio,
            plan: &plan,
            vocal_stem: Some(&stem),
            stem_alignment: Some(&stale_alignment),
            output: &refused
        })
        .is_err()
    );
    assert!(!refused.exists());
    assert_eq!(models::hash_file(&audio).unwrap(), original_hash);
}
