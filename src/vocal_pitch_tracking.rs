//! Note-centre decoding of an observed contour. Raw detector evidence is retained
//! by callers. No key, scale, consonance, lyric or octave-folding prior is used.
use crate::vocal_generation::PitchFrame;
use anyhow::{Result, ensure};
const LOW: u8 = 36;
const STATES: usize = 49;

/// Replace observations only where the independent model supplies evidence;
/// retain the complete acoustic voicing mask, then decode stable note centres.
pub fn refine(
    frames: &mut [PitchFrame],
    evidence: &crate::vocal_pitch::Evidence,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    ensure!(!evidence.frames.is_empty(), "Missing comparison contour");
    ensure!(frames.len() <= 400_000, "Contour exceeds source budget");
    let voiced: Vec<_> = frames.iter().map(|f| f.midi.is_some()).collect();
    let mut context = voiced.clone();
    let mut scan = 0;
    while scan < voiced.len() {
        if voiced[scan] {
            scan += 1;
            continue;
        }
        let start = scan;
        while scan < voiced.len() && !voiced[scan] {
            scan += 1;
        }
        if start > 0 && scan < voiced.len() && scan - start <= 8 {
            context[start..scan].fill(true);
        }
    }
    let mut weights = vec![1.; frames.len()];
    for (i, f) in frames.iter_mut().enumerate().filter(|(i, _)| context[*i]) {
        let at = evidence
            .frames
            .partition_point(|m| m.time_seconds < f.time_seconds);
        let at = [at.saturating_sub(1), at.min(evidence.frames.len() - 1)]
            .into_iter()
            .min_by(|&a, &b| {
                (evidence.frames[a].time_seconds - f.time_seconds)
                    .abs()
                    .total_cmp(&(evidence.frames[b].time_seconds - f.time_seconds).abs())
            })
            .unwrap();
        let model = &evidence.frames[at];
        let pitch =
            if (model.time_seconds - f.time_seconds).abs() <= 0.020 && model.confidence > 0.03 {
                model
                    .frequency_hz
                    .map(|hz| 69. + 12. * (hz / 440.).log2())
                    .filter(|p| (35.5..84.5).contains(p))
            } else {
                None
            };
        if let Some(pitch) = pitch {
            f.midi = Some(pitch);
        }
        weights[i] = if pitch.is_some() {
            (model.confidence as f64).max(0.1)
        } else {
            0.1
        };
        if voiced[i] {
            f.support = crate::vocal_generation::PitchSupport::ContourPath;
            f.clarity = f.clarity.min(0.59);
        }
    }
    weighted_note_path(frames, 3.2, &weights, cancelled)?;
    // Context observations help pitch choice; they never create voiced notes.
    for (f, accepted) in frames.iter_mut().zip(voiced) {
        if !accepted {
            f.midi = None;
        }
    }
    Ok(())
}

/// A change-point path balances source pitch residual against a cost for changing
/// note. Every supported MIDI pitch is available at every frame. Gaps reset the
/// path; this operation cannot add/remove voiced frames or stretch boundaries.
pub fn note_path(
    frames: &mut [PitchFrame],
    change_cost: f64,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    decode_path(frames, change_cost, None, None, cancelled)
}
pub fn weighted_note_path(
    frames: &mut [PitchFrame],
    change_cost: f64,
    weights: &[f64],
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    ensure!(
        weights.len() == frames.len()
            && weights
                .iter()
                .all(|w| w.is_finite() && (0.01..=2.).contains(w)),
        "Invalid contour residual weights"
    );
    decode_path(frames, change_cost, Some(weights), None, cancelled)
}
pub fn anchored_note_path(
    frames: &mut [PitchFrame],
    change_cost: f64,
    weights: &[f64],
    anchors: &[(f64, f64)],
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    ensure!(
        weights.len() == frames.len()
            && weights
                .iter()
                .all(|w| w.is_finite() && (0.01..=2.).contains(w))
            && anchors.len() == frames.len()
            && anchors
                .iter()
                .all(|(p, w)| p.is_finite() && w.is_finite() && (0.0..=2.).contains(w)),
        "Invalid anchored contour weights"
    );
    decode_path(frames, change_cost, Some(weights), Some(anchors), cancelled)
}
fn decode_path(
    frames: &mut [PitchFrame],
    change_cost: f64,
    weights: Option<&[f64]>,
    anchors: Option<&[(f64, f64)]>,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    ensure!(
        change_cost.is_finite() && (0.0..=16.).contains(&change_cost),
        "Invalid contour change cost"
    );
    ensure!(
        frames.len() <= 400_000
            && frames
                .iter()
                .all(|f| f.time_seconds.is_finite() && f.time_seconds >= 0.)
            && frames
                .windows(2)
                .all(|w| w[0].time_seconds < w[1].time_seconds)
            && frames.iter().all(|f| f
                .midi
                .is_none_or(|p| p.is_finite() && (35.5..84.5).contains(&p))),
        "Invalid selected pitch contour"
    );
    let mut start = 0;
    while start < frames.len() {
        ensure!(!cancelled(), "Vocal generation cancelled");
        if frames[start].midi.is_none() {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < frames.len()
            && frames[end].midi.is_some()
            && frames[end].time_seconds - frames[end - 1].time_seconds <= 0.0201
        {
            end += 1;
        }
        let mut previous = [0f64; STATES];
        let mut pointers = Vec::with_capacity(end - start);
        for (index, frame) in frames[start..end].iter().enumerate() {
            if index % 1024 == 0 {
                ensure!(!cancelled(), "Vocal generation cancelled");
            }
            let best = (0..STATES)
                .min_by(|&a, &b| previous[a].total_cmp(&previous[b]))
                .unwrap();
            let mut next = [0f64; STATES];
            let mut back = [0u8; STATES];
            let pitch = frame.midi.unwrap();
            for state in 0..STATES {
                let chosen = if previous[state] <= previous[best] + change_cost {
                    state
                } else {
                    best
                };
                back[state] = chosen as u8;
                let centre = (LOW as usize + state) as f64;
                let residual = (pitch - centre).powi(2).min(4.)
                    * weights.map_or(1., |w| w[start + index])
                    + anchors.map_or(0., |a| {
                        let (p, w) = a[start + index];
                        w * (p - centre).powi(2).min(4.)
                    });
                next[state] =
                    previous[chosen] + if chosen == state { 0. } else { change_cost } + residual;
            }
            let minimum = next.iter().copied().fold(f64::INFINITY, f64::min);
            for cost in &mut next {
                *cost -= minimum;
            }
            previous = next;
            pointers.push(back);
        }
        let mut state = (0..STATES)
            .min_by(|&a, &b| previous[a].total_cmp(&previous[b]))
            .unwrap();
        for index in (start..end).rev() {
            frames[index].midi = Some((LOW as usize + state) as f64);
            state = pointers[index - start][state] as usize;
        }
        start = end;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frames(pitches: &[Option<f64>]) -> Vec<PitchFrame> {
        pitches
            .iter()
            .enumerate()
            .map(|(i, &midi)| PitchFrame {
                time_seconds: i as f64 * 0.01,
                midi,
                yin_midi: midi,
                mcleod_midi: midi,
                clarity: 1.,
                rms: 0.1,
                local_rms: Some(0.1),
                local_energy_active: Some(midi.is_some()),
                support: Default::default(),
            })
            .collect()
    }
    #[test]
    fn decoding_preserves_voicing_gaps_short_notes_and_real_leaps() {
        let mut p = vec![Some(60.1); 20];
        p[8] = Some(72.);
        p.extend(vec![Some(62.1); 10]);
        p.push(None);
        p.extend(vec![Some(72.2); 10]);
        let mut f = frames(&p);
        note_path(&mut f, 3.2, &|| false).unwrap();
        assert!(f[..20].iter().all(|v| v.midi == Some(60.)));
        assert!(f[20..30].iter().all(|v| v.midi == Some(62.)));
        assert!(f[30].midi.is_none());
        assert!(f[31..].iter().all(|v| v.midi == Some(72.)));
        assert_eq!(
            f.iter().map(|f| f.midi.is_some()).collect::<Vec<_>>(),
            p.iter().map(Option::is_some).collect::<Vec<_>>()
        );
    }
}
