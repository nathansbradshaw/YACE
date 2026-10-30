//! Fit keyboard notes to controller limits and keep the raw detector output.
use crate::mapping::PitchNote;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const RULE_ID: &str = "keyboard-voicing-v1";

#[derive(Clone, Debug, Serialize)]
pub struct Voice {
    pub note: PitchNote,
    pub source_indices: Vec<usize>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Selection {
    pub rule_id: String,
    pub voices: Vec<Voice>,
    pub weak_short_candidates: Vec<usize>,
    pub below_confidence_candidates: Vec<usize>,
}

/// Collapse different source octaves of one co-onset into one controller voice.
/// A repeated attack of the same source key starts another group, even below
/// 35ms. Short, weak fragments need local same-key support; strong fast notes
/// and long quiet notes survive. No chart/reference data enters this rule.
pub fn select(source: &[PitchNote]) -> Selection {
    let mut strong = BTreeMap::<u8, Vec<f64>>::new();
    for n in source.iter().filter(|n| n.confidence >= 0.65) {
        strong.entry(n.pitch).or_default().push(n.start);
    }
    for times in strong.values_mut() {
        times.sort_by(f64::total_cmp);
    }
    let mut eligible = Vec::new();
    let mut weak = Vec::new();
    let mut below = Vec::new();
    for (i, n) in source.iter().enumerate() {
        if n.confidence < 0.20 {
            below.push(i);
            continue;
        }
        let short_weak = n.confidence >= 0.20 && n.confidence < 0.65 && n.end - n.start < 0.080;
        let support = strong.get(&n.pitch).is_some_and(|times| {
            times
                .get(times.partition_point(|&t| t < n.start - 0.75))
                .is_some_and(|&t| t <= n.start + 0.75)
        });
        if short_weak && !support {
            weak.push(i)
        } else {
            eligible.push(i)
        }
    }
    eligible.sort_by(|&a, &b| {
        source[a]
            .start
            .total_cmp(&source[b].start)
            .then(source[a].pitch.cmp(&source[b].pitch))
            .then(a.cmp(&b))
    });
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut seen = BTreeSet::new();
    for i in eligible {
        let same_attack = groups.last().is_some_and(|g| {
            source[i].start - source[g[0]].start <= 0.035
                && (!seen.contains(&source[i].pitch)
                    || (source[i].start - source[g[0]].start).abs() < 0.001)
        });
        if !same_attack {
            groups.push(Vec::new());
            seen.clear();
        }
        groups.last_mut().unwrap().push(i);
        seen.insert(source[i].pitch);
    }
    let mut voices = Vec::new();
    for group in groups {
        let mut classes = BTreeMap::<u8, Vec<usize>>::new();
        for i in group {
            classes.entry(source[i].pitch % 12).or_default().push(i);
        }
        for indices in classes.into_values() {
            let best = *indices
                .iter()
                .max_by(|&&a, &&b| {
                    source[a]
                        .confidence
                        .total_cmp(&source[b].confidence)
                        .then(source[a].pitch.cmp(&source[b].pitch))
                })
                .unwrap();
            let mut note = source[best].clone();
            note.start = indices
                .iter()
                .map(|&i| source[i].start)
                .min_by(f64::total_cmp)
                .unwrap();
            note.end = indices
                .iter()
                .map(|&i| source[i].end)
                .max_by(f64::total_cmp)
                .unwrap();
            voices.push(Voice {
                note,
                source_indices: indices,
            });
        }
    }
    voices.sort_by(|a, b| {
        a.note
            .start
            .total_cmp(&b.note.start)
            .then(a.note.pitch.cmp(&b.note.pitch))
    });
    Selection {
        rule_id: RULE_ID.into(),
        voices,
        weak_short_candidates: weak,
        below_confidence_candidates: below,
    }
}
