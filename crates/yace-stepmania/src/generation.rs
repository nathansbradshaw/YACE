use crate::{Chart, Difficulty, Note, NoteKind, Timing, timing::Boundary};
use anyhow::{Result, ensure};
use rustfft::{FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    Pad,
    Keyboard,
}
impl Profile {
    pub fn name(self) -> &'static str {
        match self {
            Self::Pad => "pad",
            Self::Keyboard => "keyboard",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationRequest {
    pub profile: Profile,
    pub difficulties: Vec<Difficulty>,
    pub seed: u64,
    #[serde(default = "one")]
    pub density: f64,
}
fn one() -> f64 {
    1.
}
impl Default for GenerationRequest {
    fn default() -> Self {
        Self {
            profile: Profile::Pad,
            difficulties: Difficulty::GENERATED.to_vec(),
            seed: 1,
            density: 1.,
        }
    }
}
fn random(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut n = *state;
    n = (n ^ (n >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    n = (n ^ (n >> 27)).wrapping_mul(0x94d049bb133111eb);
    n ^ (n >> 31)
}
/// Step placement selects measured spectral attacks on a reviewed beat map;
/// arrangement uses separate constraints for feet and fingers.
pub fn generate(
    samples: &[f32],
    rate: u32,
    timing: &Timing,
    request: &GenerationRequest,
    cancel: impl Fn() -> bool,
) -> Result<Vec<Chart>> {
    timing.validate()?;
    ensure!(
        (8000..=192000).contains(&rate) && !samples.is_empty(),
        "Expected nonempty mono PCM"
    );
    ensure!(
        samples.iter().all(|v| v.is_finite()),
        "Nonfinite audio sample"
    );
    ensure!(
        !request.difficulties.is_empty() && request.difficulties.len() <= 5,
        "Select one to five difficulties"
    );
    let mut selected = BTreeSet::new();
    for d in &request.difficulties {
        ensure!(
            *d != Difficulty::Edit && selected.insert(d.name()),
            "Select distinct generated difficulties"
        );
    }
    ensure!(
        (0.25..=2.).contains(&request.density),
        "Density must be 0.25–2"
    );
    let hop = (rate as usize / 100).max(1);
    let size = 1024;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(size);
    let mut previous = vec![0.; size / 2];
    let mut flux = Vec::new();
    let mut energy = Vec::new();
    let mut frame = vec![Complex::default(); size];
    for start in (0..samples.len().saturating_sub(size)).step_by(hop) {
        if cancel() {
            anyhow::bail!("Generation cancelled");
        }
        let mut power = 0.;
        for (i, v) in frame.iter_mut().enumerate() {
            let sample = samples[start + i];
            power += sample * sample;
            *v = Complex::new(
                sample * (0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / size as f32).cos()),
                0.,
            );
        }
        fft.process(&mut frame);
        let mut value = 0.;
        for (i, v) in frame[..size / 2].iter().enumerate() {
            let v = v.norm();
            value += (v - previous[i]).max(0.);
            previous[i] = v;
        }
        flux.push(value);
        energy.push(power / size as f32);
    }
    let max = flux.iter().copied().fold(0., f32::max);
    if max < 1e-6 {
        return Ok(Vec::new());
    }
    let mut peaks = Vec::new();
    for i in 1..flux.len().saturating_sub(1) {
        if flux[i] > flux[i - 1] && flux[i] >= flux[i + 1] && flux[i] > max * 0.04 {
            let seconds = (i * hop + size / 2) as f64 / rate as f64;
            peaks.push((seconds, (flux[i] / max) as f64));
        }
    }
    let mut charts = Vec::new();
    for difficulty in &request.difficulties {
        ensure!(!cancel(), "Generation cancelled");
        let level = Difficulty::GENERATED
            .iter()
            .position(|d| d == difficulty)
            .unwrap();
        let snap = [48, 24, 24, 12, 12][level];
        let mut candidates = BTreeMap::<i64, f64>::new();
        for (seconds, strength) in &peaks {
            let crate::timing::AudioPosition::Beat(beat) = timing.position(*seconds) else {
                continue;
            };
            let row = ((beat * 48. / snap as f64).round() as i64) * snap;
            if !(0..=crate::MAX_ROW).contains(&row) || !timing.judgable(row as f64 / 48.) {
                continue;
            }
            let quantized = timing.seconds(row as f64 / 48., Boundary::Note);
            if (quantized - seconds).abs() > 0.055 {
                continue;
            }
            candidates
                .entry(row)
                .and_modify(|s| *s = s.max(*strength))
                .or_insert(*strength);
        }
        let max_nps = if request.profile == Profile::Pad {
            [1.5, 2.5, 4., 5.5, 7.][level]
        } else {
            [2., 3.5, 6., 9., 13.][level]
        } * request.density;
        let mut notes = Vec::new();
        let mut state = request.seed ^ (level as u64).wrapping_mul(1234567);
        let mut last = -100.;
        let mut foot = 0usize;
        let mut recent = Vec::new();
        let mut last_lane = 4;
        for (row, strength) in candidates {
            if cancel() {
                anyhow::bail!("Generation cancelled");
            }
            let seconds = timing.seconds(row as f64 / 48., Boundary::Note);
            if seconds - last < 1. / max_nps || strength < [0.3, 0.2, 0.13, 0.08, 0.04][level] {
                continue;
            }
            recent.retain(|t| seconds - *t < 8.);
            if recent.len() as f64 >= max_nps * 6. {
                continue;
            }
            let lane = if request.profile == Profile::Pad {
                let pair = if foot == 0 { [0, 1] } else { [3, 2] };
                foot = 1 - foot;
                pair[(random(&mut state) & 1) as usize]
            } else {
                let mut lane = (random(&mut state) % 4) as u8;
                if lane == last_lane && seconds - last < 0.2 {
                    lane = (lane + 1) % 4;
                }
                lane
            };
            let id = format!("draft-{}-{}", difficulty.name(), notes.len());
            notes.push(Note {
                id,
                row,
                lane,
                kind: NoteKind::Tap,
                end_row: None,
            });
            if level >= 3 && strength > 0.65 && random(&mut state).is_multiple_of(8) {
                let other = if lane < 2 { 3 } else { 0 };
                notes.push(Note {
                    id: format!("draft-{}-{}", difficulty.name(), notes.len()),
                    row,
                    lane: other,
                    kind: NoteKind::Tap,
                    end_row: None,
                });
            }
            recent.push(seconds);
            last = seconds;
            last_lane = lane;
        }
        // Sustains mark isolated ringing attacks only; never occupy a foot
        // needed by another note before the release.
        for i in 0..notes.len() {
            let row = notes[i].row;
            let next = notes.get(i + 1).map_or(row + 192, |n| n.row);
            if level > 0 && next - row >= 96 && random(&mut state).is_multiple_of(5) {
                let s = timing.seconds(row as f64 / 48., Boundary::Note);
                let frame_index = ((s + 0.3) * rate as f64 / hop as f64) as usize;
                if energy.get(frame_index).copied().unwrap_or(0.) > 0.001 {
                    notes[i].kind = NoteKind::Hold;
                    notes[i].end_row = Some(row + 48);
                }
            }
        }
        if notes.is_empty() {
            continue;
        }
        let chart = Chart {
            profile: Some(request.profile),
            id: format!("draft-{}", difficulty.name()),
            stepstype: "dance-single".into(),
            difficulty: difficulty.name().into(),
            meter: ([1, 3, 5, 7, 9][level] as f64 * request.density)
                .round()
                .max(1.) as u32,
            description: format!("YACE {} draft · estimated rating", request.profile.name()),
            notes,
            timing: None,
            metadata: BTreeMap::from([(
                "CREDIT".into(),
                "YACE automatic draft; human review required".into(),
            )]),
            read_only_reason: None,
            original_notes: String::new(),
        };
        chart.validate()?;
        charts.push(chart);
    }
    Ok(charts)
}
