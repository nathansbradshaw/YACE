//! Optional experimental evidence; no model weights or recordings are bundled.
use super::Cell;
use anyhow::{Context, Result, ensure};
use chart_engine::vocal_generation::Generation;
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
use vocal_boundary_benchmark::read;
#[derive(Deserialize)]
struct SpectralEvidence {
    audio_sha256: String,
    vocal_stem_sha256: String,
    generation_sha256: String,
    game_sha256: String,
    times: Vec<f64>,
    frames: Vec<[f64; 20]>,
}
fn spectral_context(rows: &[[f64; 20]], i: usize) -> Vec<f64> {
    let mut out = rows[i].to_vec();
    for w in [5, 20] {
        for (a, b) in [
            (i.saturating_sub(w), i),
            ((i + 1).min(rows.len()), (i + w + 1).min(rows.len())),
        ] {
            for j in [0, 3, 16, 18] {
                out.push(if a < b {
                    rows[a..b].iter().map(|r| r[j]).sum::<f64>() / (b - a) as f64
                } else {
                    rows[i][j]
                });
            }
        }
    }
    out
}
pub(crate) fn append_spectral(
    g: &Generation,
    c: &[Cell],
    x: &mut [Vec<f64>],
    path: &Path,
    generation_sha256: &str,
    game_sha256: &str,
) -> Result<String> {
    let e: SpectralEvidence = read(path)?;
    ensure!(
        e.audio_sha256 == g.audio_sha256
            && e.vocal_stem_sha256 == g.vocal_stem_sha256
            && e.generation_sha256 == generation_sha256
            && e.game_sha256 == game_sha256,
        "Spectral source lineage mismatch"
    );
    ensure!(
        !c.is_empty()
            && x.len() == c.len()
            && e.times.len() == c.len()
            && e.frames.len() == c.len()
            && e.times
                .iter()
                .zip(c)
                .all(|(t, c)| t.is_finite() && (t - c.t).abs() < 1e-9)
            && e.frames.iter().flatten().all(|v| v.is_finite()),
        "Spectral native clock/values mismatch"
    );
    let receipt: Value = read(
        path.parent()
            .context("Spectral parent")?
            .join("inference-receipt.json"),
    )?;
    let file = path
        .file_name()
        .context("Spectral file")?
        .to_str()
        .context("UTF8 spectral path")?;
    let pins: Vec<_> = receipt["inputs"]
        .as_array()
        .context("Spectral pins")?
        .iter()
        .filter(|p| p["file"] == file)
        .collect();
    let hash = chart_engine::models::hash_file(path)?;
    ensure!(
        receipt["reads_reference_labels"] == false
            && receipt["schema"] == "spectral-v1-20-16khz-1024-hann-32mel-80-6000-source-template"
            && pins.len() == 1
            && pins[0]["sha256"] == hash,
        "Spectral receipt/protocol mismatch"
    );
    for (i, row) in x.iter_mut().enumerate() {
        ensure!(
            row.len() == 148,
            "Spectral study requires frozen GAME+role features"
        );
        row.extend(spectral_context(&e.frames, i));
        ensure!(row.len() == 184, "Spectral feature width");
    }
    Ok(hash)
}
#[derive(Deserialize)]
struct GameRow {
    boundary: bool,
    presence_confidence: f64,
    game_pitch: f64,
}
#[derive(Deserialize)]
struct GameAsset {
    file: String,
    sha256: String,
}
fn pinned_game_assets(assets: &[GameAsset]) -> bool {
    let expected = [
        (
            "encoder.onnx",
            "a6868a702d9c88ccc06b19e54f49d88b7f8cba4a68517f20806bd81ef6f94e06",
        ),
        (
            "segmenter.onnx",
            "7724db4ba59a388fff5aba2f08f0ac025d4ca59caef80e7f474053a1455c88e5",
        ),
        (
            "estimator.onnx",
            "a84738b2504472a1f22773cc6b0eccad1353a8f36229ad490b8fea85388bb5e1",
        ),
    ];
    assets.len() == expected.len()
        && expected.iter().all(|(file, sha)| {
            assets
                .iter()
                .filter(|a| a.file == *file && a.sha256 == *sha)
                .count()
                == 1
        })
}
#[derive(Deserialize)]
struct GameEvidence {
    source_sha256: String,
    duration_seconds: f64,
    timestep: f64,
    frame_kind: String,
    steps: usize,
    language: usize,
    core_frames: usize,
    context_frames: usize,
    instrumentation_original_outputs_equal_first_chunk: bool,
    rows: Vec<GameRow>,
    assets: Vec<GameAsset>,
}
fn game_features(c: &Cell, i: usize, e: &GameEvidence) -> Vec<f64> {
    let mut x = if let Some(r) = e.rows.get(i) {
        vec![
            1.,
            r.presence_confidence,
            f64::from(r.boundary),
            c.p.map_or(0., |p| (r.game_pitch - p).clamp(-12., 12.)),
            f64::from(c.p.is_some_and(|p| (r.game_pitch - p).abs() < 0.5)),
        ]
    } else {
        vec![0.; 5]
    };
    for w in [5, 20, 50] {
        for (a, b) in [
            (i.saturating_sub(w), i.min(e.rows.len())),
            ((i + 1).min(e.rows.len()), (i + w + 1).min(e.rows.len())),
        ] {
            let rows = &e.rows[a..b];
            let n = rows.len().max(1) as f64;
            x.extend([
                rows.iter().map(|r| r.presence_confidence).sum::<f64>() / n,
                rows.iter()
                    .map(|r| r.presence_confidence)
                    .min_by(f64::total_cmp)
                    .unwrap_or(0.),
                rows.iter()
                    .map(|r| r.presence_confidence)
                    .max_by(f64::total_cmp)
                    .unwrap_or(0.),
                rows.iter().filter(|r| r.boundary).count() as f64 / n,
            ]);
        }
    }
    let left = (i.saturating_sub(50)..i.min(e.rows.len()))
        .rev()
        .find(|&j| e.rows[j].boundary)
        .map_or(0.5, |j| (i - j) as f64 * 0.01);
    let right = ((i + 1).min(e.rows.len())..(i + 51).min(e.rows.len()))
        .find(|&j| e.rows[j].boundary)
        .map_or(0.5, |j| (j - i) as f64 * 0.01);
    x.extend([left, right]);
    x
}
pub(crate) fn append_game(
    g: &Generation,
    c: &[Cell],
    x: &mut [Vec<f64>],
    path: &Path,
) -> Result<String> {
    let e: GameEvidence = read(path)?;
    ensure!(
        c.first().is_some_and(|c| c.t.abs() < 1e-8)
            && c.iter()
                .all(|c| c.t.is_finite() && c.t >= 0. && c.t <= e.duration_seconds + 0.01),
        "GAME context requires native source-zero 100 Hz pitch observations"
    );
    ensure!(
        e.source_sha256 == g.vocal_stem_sha256
            && (e.duration_seconds - g.duration_seconds).abs() <= 1. / 16000.
            && e.timestep == 0.01
            && e.frame_kind == "interval_start"
            && [2, 8].contains(&e.steps)
            && e.language == 0
            && e.core_frames == 2000
            && e.context_frames == 100
            && e.instrumentation_original_outputs_equal_first_chunk
            && pinned_game_assets(&e.assets),
        "GAME source/protocol mismatch"
    );
    ensure!(
        e.rows.len() >= 2
            && e.rows.len() <= 36000
            && e.duration_seconds - e.rows.len() as f64 * 0.01 >= -1e-8
            && e.duration_seconds - e.rows.len() as f64 * 0.01 < 0.010001
            && e.rows.iter().all(|r| r.presence_confidence.is_finite()
                && (0.0..=1.).contains(&r.presence_confidence)
                && r.game_pitch.is_finite()
                && (0.0..=128.).contains(&r.game_pitch)),
        "Invalid GAME values/coverage"
    );
    let receipt: Value = read(
        path.parent()
            .context("GAME parent")?
            .join("inference-receipt.json"),
    )?;
    ensure!(receipt["steps"] == e.steps, "GAME schedule differs");
    let file = path
        .file_name()
        .context("GAME filename")?
        .to_str()
        .context("UTF8 filename")?;
    let pins: Vec<_> = receipt["rows"]
        .as_array()
        .context("GAME source pins")?
        .iter()
        .filter(|r| r["file"] == file)
        .collect();
    let hash = chart_engine::models::hash_file(path)?;
    ensure!(
        pins.len() == 1
            && pins[0]["stem_sha256"] == g.vocal_stem_sha256
            && pins[0]["sha256"] == hash,
        "GAME output changed"
    );
    for (cell, row) in c.iter().zip(x) {
        let i = (cell.t / 0.01).round() as usize;
        row.extend(game_features(cell, i, &e));
    }
    Ok(hash)
}
#[derive(Deserialize)]
struct RoleFrame {
    time_seconds: f64,
    x: Vec<f64>,
}
#[derive(Deserialize)]
struct RoleEvidence {
    audio_sha256: String,
    vocal_stem_sha256: String,
    duration_seconds: f64,
    model_sha256: String,
    sample_rate: u32,
    rms_radius_samples: usize,
    correlation_window_samples: usize,
    correlation_stride: usize,
    lag_multipliers: Vec<f64>,
    frames: Vec<RoleFrame>,
}
pub(crate) fn append_role(
    g: &Generation,
    c: &[Cell],
    x: &mut [Vec<f64>],
    path: &Path,
    generation_sha256: &str,
) -> Result<String> {
    let e: RoleEvidence = read(path)?;
    ensure!(
        e.audio_sha256 == g.audio_sha256
            && e.vocal_stem_sha256 == g.vocal_stem_sha256
            && (e.duration_seconds - g.duration_seconds).abs() < 1e-8
            && e.model_sha256 == "bf32e15105a09c0f7dddd2b67346146334d6f3ecb399ed7638eba2ab07cbf5f4"
            && e.sample_rate == 44100
            && e.rms_radius_samples == 882
            && e.correlation_window_samples == 2048
            && e.correlation_stride == 4
            && e.lag_multipliers == [1., 2., 0.5],
        "Role evidence source/protocol mismatch"
    );
    ensure!(
        e.frames.len() == c.len()
            && e.frames.iter().zip(c).all(|(f, c)| f.time_seconds == c.t
                && f.x.len() == 12
                && f.x.iter().all(|v| v.is_finite())),
        "Role feature clock/values mismatch"
    );
    let receipt: Value = read(
        path.parent()
            .context("Role parent")?
            .join("inference-receipt.json"),
    )?;
    let file = path
        .file_name()
        .context("Role filename")?
        .to_str()
        .context("UTF8 role path")?;
    let pins: Vec<_> = receipt["inputs"]
        .as_array()
        .context("Role input pins")?
        .iter()
        .filter(|r| r["file"] == file)
        .collect();
    let hash = chart_engine::models::hash_file(path)?;
    ensure!(
        pins.len() == 1
            && pins[0]["sha256"] == hash
            && pins[0]["generation_sha256"] == generation_sha256,
        "Role evidence changed since prediction"
    );
    for (i, row) in x.iter_mut().enumerate() {
        row.extend_from_slice(&e.frames[i].x);
        for w in [5, 20] {
            for (a, b) in [(i.saturating_sub(w), i), (i + 1, (i + w + 1).min(c.len()))] {
                for j in [2, 3, 4, 5, 10] {
                    row.push(if a < b {
                        e.frames[a..b].iter().map(|r| r.x[j]).sum::<f64>() / (b - a) as f64
                    } else {
                        e.frames[i].x[j]
                    });
                }
            }
        }
    }
    Ok(hash)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spectral_context_preserves_native_origin_and_bounds_both_edges() {
        let rows: Vec<_> = (0..30).map(|i| [i as f64; 20]).collect();
        let a = spectral_context(&rows, 0);
        let b = spectral_context(&rows, 29);
        assert_eq!(a.len(), 36);
        assert_eq!(&a[..20], &[0.; 20]);
        assert_eq!(&a[20..24], &[0.; 4]);
        assert_eq!(&a[24..28], &[3.; 4]);
        assert_eq!(&b[..20], &[29.; 20]);
        assert_eq!(&b[20..24], &[26.; 4]);
        assert_eq!(&b[24..28], &[29.; 4]);
        assert_eq!(spectral_context(&[[2.; 20]], 0), vec![2.; 36]);
    }
    #[test]
    fn game_pitch_context_is_relative_and_missing_pitch_has_no_fixed_origin() {
        let mut e = GameEvidence {
            source_sha256: "a".repeat(64),
            duration_seconds: 1.,
            timestep: 0.01,
            frame_kind: "interval_start".into(),
            steps: 2,
            language: 0,
            core_frames: 2000,
            context_frames: 100,
            instrumentation_original_outputs_equal_first_chunk: true,
            assets: vec![],
            rows: (0..100)
                .map(|i| GameRow {
                    boundary: i == 40,
                    presence_confidence: 0.7,
                    game_pitch: 60.2,
                })
                .collect(),
        };
        let a = Cell {
            t: 0.5,
            p: Some(60.),
            c: 0.9,
            r: -20.,
            j: 0.8,
        };
        let x = game_features(&a, 50, &e);
        assert_eq!(x.len(), 31);
        for r in &mut e.rows {
            r.game_pitch += 12.;
        }
        let mut b = a.clone();
        b.p = Some(72.);
        assert!(
            x.iter()
                .zip(game_features(&b, 50, &e))
                .all(|(a, b)| (a - b).abs() < 1e-10)
        );
        b.p = None;
        let missing = game_features(&b, 50, &e);
        assert_eq!(missing[3], 0.);
        assert_eq!(missing[4], 0.);
        assert_eq!(game_features(&b, 100, &e)[0], 0.);
    }
}
