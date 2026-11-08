//! Experimental full-evidence note-centre decoder. No voicing, key, scale,
//! octave folding or lyric prior. Callers establish source/model bindings.
use crate::vocal_generation::{PitchFrame, PitchSupport};
use anyhow::{Result, ensure};
pub const STATES: usize = 49;
pub fn spectral_probe_implementation_hash() -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(include_bytes!("../tools/singing-pitch-probe/src/main.rs"));
    h.update(include_bytes!("../tools/singing-pitch-probe/src/rmvpe.rs"));
    h.update(include_bytes!("rmvpe_frontend.rs"));
    h.update(include_bytes!("../tools/singing-pitch-probe/Cargo.toml"));
    h.update(include_bytes!("../tools/singing-pitch-probe/Cargo.lock"));
    format!("{:x}", h.finalize())
}
pub fn fcpe_implementation_hash() -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest(include_bytes!("../examples/vocal_fcpe_probe.rs"))
    )
}
pub const ALGORITHM: &str = "posterior-note-centres-v1;49states36to84;bounded8hop-context;uniform-change-cost;retain-voicing;flat-evidence-fallback;experimental";
/// Frozen on the four development recordings before the fresh eight targets.
pub const FROZEN_PROFILE: &str = "rust-vocal-ensemble-v11-experimental;v10-voicing;RMVPE-CREPEfull-FCPE;max50cent-band;log-strength-ratio-cap2;v10-residual-anchor0.25-cap4;ROSVOT-Swift-agreement0.75st-salience0.5-prior4-cap4;change8;8hop-context;no-key-scale-or-octave-fold";
/// Frozen after development-only gap recovery experiments; no held-out scores.
pub const FROZEN_RECOVERY_PROFILE: &str = "rust-vocal-ensemble-v12-experimental;v11-centres;gap-recovery-ROSVOT-Swift-plus2of3-RMVPE-CREPEfull-FCPE;band-cost-contrast0.25;local-RMS-floor0.0015;stable180ms-including-existing-voicing-context;retain-all-original-voicing;no-lyrics-key-scale-or-octave-fold";
pub const FROZEN_ENERGY_PROFILE: &str = "rust-vocal-ensemble-v13-experimental;v12-recovery;addition-energy-floor-max0.0015-or-half-median-existing-voiced-local-RMS;quiet-existing-voice-retained;no-lyric-or-reference-input";
pub struct ModelFrames {
    pub id: String,
    pub model_sha256: String,
    pub implementation_sha256: String,
    /// Samples on the generation's unchanged acoustic frame clock.
    pub midi: Vec<Option<f64>>,
}
pub fn annotate(
    report: &mut crate::vocal_generation::Generation,
    models: &[ModelFrames],
) -> Result<()> {
    let valid_hash = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit());
    ensure!(
        report.pitch_frames.len() <= 400_000
            && report.note_evidence.len() <= 200_000
            && report.note_evidence.iter().all(|n| n.start < n.end)
            && report
                .pitch_frames
                .iter()
                .all(|f| f.time_seconds.is_finite())
            && report
                .pitch_frames
                .windows(2)
                .all(|w| w[0].time_seconds < w[1].time_seconds)
            && models.len() <= 8
            && models.iter().all(|m| !m.id.is_empty()
                && m.id.len() <= 128
                && valid_hash(&m.model_sha256)
                && valid_hash(&m.implementation_sha256)
                && m.midi.len() == report.pitch_frames.len()
                && m.midi
                    .iter()
                    .flatten()
                    .all(|p| p.is_finite() && (35.5..84.5).contains(p))),
        "Invalid note model observations"
    );
    let timing = report
        .timing
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Missing source timing"))?;
    for note in &mut report.note_evidence {
        let start = timing.seconds(note.start);
        let end = timing.seconds(note.end);
        let a = report
            .pitch_frames
            .partition_point(|f| f.time_seconds < start);
        let b = report
            .pitch_frames
            .partition_point(|f| f.time_seconds < end);
        note.models.clear();
        for model in models {
            let mut counts = [0usize; STATES];
            let mut support = 0;
            for p in model.midi[a..b].iter().flatten() {
                let pitch = p.round() as usize;
                if (36..=84).contains(&pitch) {
                    counts[pitch - 36] += 1;
                }
                support += usize::from((*p - note.selected_midi as f64).abs() <= 0.5);
            }
            let voiced = model.midi[a..b].iter().flatten().count();
            let selected = (0..STATES)
                .filter(|&s| counts[s] > 0)
                .max_by_key(|&s| (counts[s], std::cmp::Reverse(s)))
                .map(|s| (s + 36) as u8);
            let alternatives: Vec<_> = (0..STATES)
                .filter(|&s| counts[s] > 0 && (s + 36) as u8 != note.selected_midi)
                .map(|s| (s + 36) as u8)
                .collect();
            for &p in &alternatives {
                if !note.alternatives.contains(&p) {
                    note.alternatives.push(p);
                }
            }
            note.models
                .push(crate::vocal_generation::NoteModelEvidence {
                    model_id: model.id.clone(),
                    model_sha256: model.model_sha256.clone(),
                    implementation_sha256: model.implementation_sha256.clone(),
                    selected_midi: selected,
                    alternatives,
                    selected_note_support_frames: support,
                    voiced_frames: voiced,
                    inspected_frames: b - a,
                });
        }
        note.alternatives.sort_unstable();
        note.alternatives.dedup();
        note.uncertain |= !note.alternatives.is_empty();
        let explanation = "Independent model counts describe acoustic support, not a probability of correctness or proof of lead-singer identity.";
        if !note.models.is_empty() && !note.reasons.iter().any(|r| r == explanation) {
            note.reasons.push(explanation.into());
        }
    }
    Ok(())
}
/// Research-only recovery from independent pitch bands plus a separate
/// note-model/Swift agreement. No lyrics, authored notes or target timestamps.
/// Inputs are costs, not calibrated probabilities. Disagreement remains a gap.
pub fn recover(
    frames: &mut [PitchFrame],
    bands: [&[[f64; STATES]]; 3],
    note_swift: &[Option<f64>],
    minimum_votes: usize,
    minimum_duration: f64,
    cancelled: &dyn Fn() -> bool,
) -> Result<usize> {
    recover_energy(
        frames,
        bands,
        note_swift,
        minimum_votes,
        minimum_duration,
        0.,
        cancelled,
    )
}
/// Source-relative floor applies to additions; existing quiet voice survives.
pub fn recover_energy(
    frames: &mut [PitchFrame],
    bands: [&[[f64; STATES]]; 3],
    note_swift: &[Option<f64>],
    minimum_votes: usize,
    minimum_duration: f64,
    energy_ratio: f64,
    cancelled: &dyn Fn() -> bool,
) -> Result<usize> {
    ensure!(
        frames.len() <= 400_000
            && bands.iter().all(|b| b.len() == frames.len())
            && note_swift.len() == frames.len()
            && (2..=3).contains(&minimum_votes)
            && minimum_duration.is_finite()
            && (0.04..=0.3).contains(&minimum_duration)
            && energy_ratio.is_finite()
            && (0. ..=1.).contains(&energy_ratio)
            && bands
                .iter()
                .flat_map(|b| b.iter().flatten())
                .all(|x| x.is_finite() && *x >= 0.)
            && frames
                .iter()
                .all(|f| f.time_seconds.is_finite() && f.time_seconds >= 0.)
            && frames
                .windows(2)
                .all(|w| w[0].time_seconds < w[1].time_seconds)
            && note_swift
                .iter()
                .flatten()
                .all(|p| p.is_finite() && (36. ..=84.).contains(p)),
        "Invalid recovery evidence, clock or budget"
    );
    let mut energies: Vec<_> = frames
        .iter()
        .filter(|f| f.midi.is_some())
        .map(|f| f.local_rms.unwrap_or(f.rms))
        .filter(|e| e.is_finite() && *e > 0.)
        .collect();
    energies.sort_by(f64::total_cmp);
    let floor = energies
        .get(energies.len() / 2)
        .map_or(0.0015, |e| (e * energy_ratio).max(0.0015));
    let mut seed = vec![None; frames.len()];
    for (i, f) in frames.iter().enumerate() {
        ensure!(!cancelled(), "Vocal generation cancelled");
        if !f.local_rms.unwrap_or(f.rms).is_finite()
            || (f.local_rms.unwrap_or(f.rms) < floor && f.midi.is_none())
        {
            continue;
        }
        let Some(p) = note_swift[i] else { continue };
        let target = p.round();
        let votes = bands
            .iter()
            .filter(|b| {
                let row = &b[i];
                let best = (0..STATES)
                    .min_by(|&a, &b| row[a].total_cmp(&row[b]))
                    .unwrap();
                let peak = row.iter().copied().fold(0., f64::max);
                peak - row[best] > 0.25 && ((best + 36) as f64 - target).abs() < 0.5
            })
            .count();
        if votes >= minimum_votes {
            seed[i] = Some(target)
        }
    }
    let mut draft = frames.to_vec();
    let mut i = 0;
    let mut recovered = 0;
    while i < frames.len() {
        let Some(p) = seed[i] else {
            i += 1;
            continue;
        };
        let start = i;
        i += 1;
        while i < frames.len()
            && seed[i] == Some(p)
            && frames[i].time_seconds - frames[i - 1].time_seconds <= 0.0201
        {
            i += 1;
        }
        let hop = if i - start > 1 {
            (frames[i - 1].time_seconds - frames[start].time_seconds) / (i - start - 1) as f64
        } else {
            0.
        };
        if frames[i - 1].time_seconds - frames[start].time_seconds + hop + 1e-9 >= minimum_duration
        {
            for f in &mut draft[start..i] {
                if f.midi.is_some() {
                    continue;
                }
                f.midi = Some(p);
                f.support = PitchSupport::AcousticAlternative;
                f.clarity = f.clarity.min(0.5);
                recovered += 1;
            }
        }
    }
    ensure!(!cancelled(), "Vocal generation cancelled");
    frames.clone_from_slice(&draft);
    Ok(recovered)
}
pub fn decode(
    frames: &mut [PitchFrame],
    costs: &[[f64; STATES]],
    change_cost: f64,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    ensure!(!cancelled(), "Vocal generation cancelled");
    ensure!(
        frames.len() == costs.len()
            && frames.len() <= 400_000
            && change_cost.is_finite()
            && (0. ..=32.).contains(&change_cost)
            && frames.iter().all(|f| f.time_seconds.is_finite()
                && f.time_seconds >= 0.
                && f.midi
                    .is_none_or(|p| p.is_finite() && (35.5..84.5).contains(&p)))
            && frames
                .windows(2)
                .all(|w| w[0].time_seconds < w[1].time_seconds)
            && costs
                .iter()
                .flatten()
                .all(|v| v.is_finite() && (0. ..=1_000_000.).contains(v)),
        "Invalid posterior frame grid, costs or budget"
    );
    let mut draft = frames.to_vec();
    let voiced: Vec<_> = frames.iter().map(|f| f.midi.is_some()).collect();
    let mut context = voiced.clone();
    let mut at = 0;
    while at < frames.len() {
        if voiced[at] {
            at += 1;
            continue;
        }
        let a = at;
        while at < frames.len() && !voiced[at] {
            at += 1
        }
        if a > 0 && at < frames.len() && at - a <= 8 {
            context[a..at].fill(true)
        }
    }
    let flat: Vec<_> = costs
        .iter()
        .map(|row| {
            let lo = row.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = row.iter().copied().fold(0., f64::max);
            hi - lo < 1e-12
        })
        .collect();
    let emission = |i: usize, state: usize| {
        if flat[i] {
            frames[i]
                .midi
                .map_or(0., |p| (p - (state + 36) as f64).powi(2).min(4.))
        } else {
            costs[i][state]
        }
    };
    let mut start = 0;
    while start < frames.len() {
        ensure!(!cancelled(), "Vocal generation cancelled");
        if !context[start] {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < frames.len()
            && context[end]
            && frames[end].time_seconds - frames[end - 1].time_seconds <= 0.0201
        {
            end += 1
        }
        let n = end - start;
        let mut back = vec![[0u8; STATES]; n];
        let mut prev = std::array::from_fn::<_, STATES, _>(|s| emission(start, s));
        for i in start + 1..end {
            if i % 256 == 0 {
                ensure!(!cancelled(), "Vocal generation cancelled")
            };
            let best = (0..STATES)
                .min_by(|&a, &b| prev[a].total_cmp(&prev[b]))
                .unwrap();
            let mut next = [0.; STATES];
            for s in 0..STATES {
                let jump = prev[best] + change_cost;
                let predecessor = if prev[s] <= jump { s } else { best };
                back[i - start][s] = predecessor as u8;
                next[s] = emission(i, s) + if predecessor == s { prev[s] } else { jump }
            }
            prev = next
        }
        let mut state = (0..STATES)
            .min_by(|&a, &b| prev[a].total_cmp(&prev[b]))
            .unwrap();
        for i in (start..end).rev() {
            if voiced[i] {
                draft[i].midi = Some((state + 36) as f64);
                if draft[i].midi != frames[i].midi {
                    draft[i].support = PitchSupport::AcousticAlternative;
                    draft[i].clarity = draft[i].clarity.min(0.5)
                }
            }
            state = back[i - start][state] as usize
        }
        start = end
    }
    ensure!(!cancelled(), "Vocal generation cancelled");
    frames.clone_from_slice(&draft);
    Ok(())
}
