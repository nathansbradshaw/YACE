use chart_engine::muscriptor::{self, Event, NoteDecoder, Options, Release, decode_token};
use std::{fs, path::PathBuf};
const P: i64 = 1004;
const V: i64 = 1132;
const PR: i64 = 1135;
const T: i64 = 1134;
fn feed(d: &mut NoteDecoder, tokens: &[i64]) {
    for &t in tokens {
        d.feed(t);
    }
}
#[test]
fn actual_token_boundaries_and_bos_are_distinct() {
    assert_eq!(decode_token(3), Some(Event::Shift(0)));
    assert_eq!(decode_token(1003), Some(Event::Shift(1000)));
    assert_eq!(decode_token(P + 127), Some(Event::Pitch(127)));
    assert_eq!(decode_token(V + 1), Some(Event::Velocity(1)));
    assert_eq!(decode_token(T), Some(Event::Tie));
    assert_eq!(decode_token(PR + 129), Some(Event::Program(129)));
    assert_eq!(decode_token(1392), Some(Event::Drum(127)));
    assert_eq!(decode_token(muscriptor::BOS), None);
}
#[test]
fn shifts_are_absolute_not_cumulative() {
    let mut d = NoteDecoder::new();
    d.begin_chunk(0., 5.).unwrap();
    feed(&mut d, &[T, PR, V + 1, 3 + 10, P + 60, 3 + 50, V, P + 60]);
    let (n, x) = d.finish(5.);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].start, 0.1);
    assert_eq!(n[0].end, 0.5);
    assert_eq!(n[0].release, Release::Explicit);
    assert_eq!(x.backwards_shifts, 0);
}
#[test]
fn tie_carries_across_boundary_without_duplicate_onset() {
    let mut d = NoteDecoder::new();
    d.begin_chunk(0., 5.).unwrap();
    feed(&mut d, &[T, PR + 80, V + 1, 3 + 400, P + 72]);
    d.begin_chunk(5., 10.).unwrap();
    assert_eq!(d.tie_tokens(), vec![PR + 80, P + 72, T]);
    feed(&mut d, &[PR + 80, P + 72, T, 3 + 100, V, P + 72]);
    let (n, x) = d.finish(10.);
    assert_eq!(n.len(), 1);
    assert_eq!((n[0].start, n[0].end), (4., 6.));
    assert_eq!(n[0].instrument, "synth_lead");
    assert_eq!(x.missing_release_at_eof, 0);
}
#[test]
fn missing_tie_closes_prior_note_at_real_boundary() {
    let mut d = NoteDecoder::new();
    d.begin_chunk(0., 5.).unwrap();
    feed(&mut d, &[T, PR, V + 1, 3 + 400, P + 60]);
    d.begin_chunk(5., 10.).unwrap();
    feed(&mut d, &[T]);
    let (n, _) = d.finish(10.);
    assert_eq!(n[0].end, 5.);
    assert_eq!(n[0].release, Release::MissingTieAtBoundary);
}
#[test]
fn retriggers_keep_both_attacks_and_program_identity() {
    let mut d = NoteDecoder::new();
    d.begin_chunk(0., 5.).unwrap();
    feed(
        &mut d,
        &[
            T,
            PR,
            V + 1,
            3 + 10,
            P + 60,
            PR + 80,
            P + 60,
            PR,
            3 + 30,
            P + 60,
            3 + 50,
            V,
            P + 60,
            PR + 80,
            P + 60,
        ],
    );
    let (n, _) = d.finish(5.);
    assert_eq!(n.len(), 3);
    assert_eq!(n[0].start, 0.1);
    assert_eq!(n[0].end, 0.3);
    assert_eq!(n[0].release, Release::Retrigger);
    assert_eq!(n[1].program, 80);
    assert_eq!(n[1].end, 0.5);
    assert_eq!(n[2].start, 0.3);
}
#[test]
fn malformed_prologue_drops_chunk_and_reports_it() {
    let mut d = NoteDecoder::new();
    d.begin_chunk(0., 5.).unwrap();
    feed(&mut d, &[3 + 10, PR, V + 1, P + 60]);
    let (n, x) = d.finish(5.);
    assert!(n.is_empty());
    assert_eq!(x.malformed_tie_chunks, 1);
}
#[test]
fn source_bounds_and_eof_uncertainty_are_explicit() {
    let mut d = NoteDecoder::new();
    d.begin_chunk(0., 1.2).unwrap();
    feed(&mut d, &[T, PR, V + 1, 3 + 30, P + 60, 3 + 150, P + 64]);
    let (n, x) = d.finish(1.2);
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].end, 1.2);
    assert_eq!(n[0].release, Release::SourceEndUnknown);
    assert_eq!(x.missing_release_at_eof, 1);
    assert_eq!(x.out_of_window_events, 1);
}
#[test]
fn backward_shift_is_diagnostic_not_silently_relabelled() {
    let mut d = NoteDecoder::new();
    d.begin_chunk(0., 5.).unwrap();
    feed(&mut d, &[T, PR, V + 1, 3 + 100, P + 60, 3 + 20, V, P + 60]);
    let (n, x) = d.finish(5.);
    assert!(n.is_empty());
    assert_eq!(x.backwards_shifts, 1);
    assert_eq!(x.collapsed_intervals, 1);
}
#[test]
fn chunk_program_and_velocity_reset() {
    let mut d = NoteDecoder::new();
    d.begin_chunk(0., 5.).unwrap();
    feed(&mut d, &[T, PR, V + 1, 3 + 10, P + 60, 3 + 20, V, P + 60]);
    d.begin_chunk(5., 10.).unwrap();
    feed(&mut d, &[T, 3 + 10, P + 64]);
    let (n, x) = d.finish(10.);
    assert_eq!(n.len(), 1);
    assert_eq!(x.missing_state_pitches, 1);
}
#[test]
fn all_model_assets_pinned() {
    let a = muscriptor::assets();
    assert_eq!(a.len(), 2);
    for a in a {
        assert!(a.url.contains(muscriptor::MODEL_REVISION));
        assert_eq!(a.sha256.len(), 64);
        assert!(a.license.contains("NC"));
    }
}

#[test]
#[ignore = "runs real MuScriptor conditioner and cached autoregressive ONNX; requires MUSCRIPTOR_AUDIO"]
fn native_onnx_audio_receipt() {
    let source = PathBuf::from(std::env::var_os("MUSCRIPTOR_AUDIO").expect("MUSCRIPTOR_AUDIO"));
    let models = std::env::var_os("MUSCRIPTOR_MODEL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models/muscriptor"));
    let groups = std::env::var("MUSCRIPTOR_GROUPS")
        .unwrap_or_default()
        .split(',')
        .filter(|x| !x.is_empty())
        .map(|s| s.parse::<u16>().unwrap())
        .collect();
    let max_tokens = std::env::var("MUSCRIPTOR_MAX_TOKENS")
        .ok()
        .map(|s| s.parse().unwrap())
        .unwrap_or(2048);
    let result = muscriptor::transcribe(
        &source,
        &models,
        Options {
            instrument_groups: groups,
            max_tokens,
            ..Options::default()
        },
    )
    .unwrap();
    if let Some(path) = std::env::var_os("MUSCRIPTOR_OUTPUT") {
        assert!(!PathBuf::from(&path).exists(), "Preserve old evidence");
        fs::write(path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    }
    eprintln!(
        "MuScriptor {}notes {}chunks diagnostics={}",
        result.notes.len(),
        result.chunks.len(),
        serde_json::to_string(&result.diagnostics).unwrap()
    );
    assert!(!result.chunks.is_empty());
    assert!(result.chunks.iter().all(|c| c.emitted_eos));
    assert!(
        result
            .notes
            .iter()
            .all(|n| n.start >= 0. && n.end <= result.duration_seconds && n.start < n.end)
    );
    assert!(
        result
            .chunks
            .iter()
            .flat_map(|c| &c.generated_tokens)
            .all(|t| (0..1393).contains(t))
    );
}
