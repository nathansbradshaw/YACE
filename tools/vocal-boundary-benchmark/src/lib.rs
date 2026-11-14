pub mod rejection;
pub mod spectral;
pub mod voice_profile;
use anyhow::{Context, Result, ensure};
use chart_engine::{
    vocal_benchmark::Metrics,
    vocal_generation::Generation,
    vocals::{Syllable, VocalChart, VocalNote, VocalPhrase, VocalTone},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fs, path::Path};
#[derive(Clone, Serialize, Deserialize)]
pub struct Note {
    pub start: f64,
    pub end: f64,
    pub midi: u8,
}
pub fn read<T: serde::de::DeserializeOwned>(p: impl AsRef<Path>) -> Result<T> {
    ensure!(
        fs::metadata(p.as_ref())?.len() <= 128 * 1024 * 1024,
        "JSON exceeds study budget"
    );
    Ok(serde_json::from_slice(&fs::read(p)?)?)
}
pub fn emit(g: &Generation, notes: &[Note]) -> Result<Generation> {
    let mut out = g.clone();
    let t = g.timing.as_ref().context("clock")?;
    let notes = notes
        .iter()
        .map(|n| {
            ensure!(
                (36..=84).contains(&n.midi)
                    && n.start.is_finite()
                    && n.end.is_finite()
                    && n.start >= 0.
                    && n.start < n.end
                    && n.end <= g.duration_seconds + 1e-9,
                "Invalid note; no silent clipping or transposition"
            );
            Ok(VocalNote {
                midi: None,
                start: t.tick(n.start).context("start")?,
                end: t.tick(n.end).context("end")?,
                tone: VocalTone::Pitched { midi: n.midi },
                lyric: Syllable::Missing,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    out.chart = notes.first().zip(notes.last()).map(|(a, b)| VocalChart {
        version: 1,
        resolution: t.resolution,
        source: "Offline vocal boundary study, not production".into(),
        phrases: vec![VocalPhrase {
            midi: None,
            start: a.start,
            end: b.end,
            player: 1,
        }],
        notes: notes.clone(),
    });
    if let Some(c) = &out.chart {
        c.validate()?;
    }
    Ok(out)
}
pub fn rates(m: &Metrics) -> Value {
    let p = m.note_matches as f64 / m.predicted_notes.max(1) as f64;
    let r = m.note_matches as f64 / m.reference_notes.max(1) as f64;
    json!({"precision":p,"recall":r,"f1":if p+r>0.{2.*p*r/(p+r)}else{0.},"matches":m.note_matches,"predicted":m.predicted_notes,"expected":m.reference_notes})
}

pub fn roundtrip(g: &Generation) -> Result<()> {
    let Some(chart) = &g.chart else {
        return Ok(());
    };
    // Source resolution is inserted into a minimal valid MIDI before export.
    let mut empty = vec![
        77, 84, 104, 100, 0, 0, 0, 6, 0, 1, 0, 1, 1, 224, 77, 84, 114, 107, 0, 0, 0, 4, 0, 255, 47,
        0,
    ];
    let division = u16::try_from(chart.resolution)?;
    empty[12..14].copy_from_slice(&division.to_be_bytes());
    let bytes = chart_engine::vocals::append_midi(&empty, chart)?;
    let imported = chart_engine::vocals::import_midi(&bytes)?;
    ensure!(
        chart.notes.len() == imported.notes.len()
            && chart
                .notes
                .iter()
                .zip(&imported.notes)
                .all(|(a, b)| a.start == b.start && a.end == b.end && a.tone == b.tone),
        "MIDI roundtrip changed notes"
    );
    Ok(())
}
/// Canonicalize nominal <=1 ns touching-boundary arithmetic only, on a copy.
/// Originals and performed-F0 diagnostics remain unchanged.
pub fn canonical_note_reference(
    r: &chart_engine::vocal_benchmark::Reference,
) -> anyhow::Result<(chart_engine::vocal_benchmark::Reference, usize)> {
    let mut out = r.clone();
    let mut count = 0;
    for i in 0..out.notes.len().saturating_sub(1) {
        let next = out.notes[i + 1].start_seconds;
        if out.notes[i].end_seconds > next {
            anyhow::ensure!(
                out.notes[i].end_seconds <= next + 1e-9,
                "Genuine overlapping annotation needs adjudication"
            );
            out.notes[i].end_seconds = next;
            count += 1;
        }
    }
    Ok((out, count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chart_engine::vocal_benchmark::{Reference, ReferenceNote};
    fn reference(end: f64) -> Reference {
        Reference {
            reference_kind: "performed_contour".into(),
            audio_sha256: "a".repeat(64),
            duration_seconds: 10.,
            provenance: "Synthetic boundary arithmetic test".into(),
            reviewed: true,
            frames: vec![],
            words: vec![],
            notes: vec![
                ReferenceNote {
                    start_seconds: 4.,
                    end_seconds: end,
                    midi: 60,
                },
                ReferenceNote {
                    start_seconds: 5.,
                    end_seconds: 6.,
                    midi: 62,
                },
            ],
        }
    }
    #[test]
    fn nominal_nanosecond_arithmetic_is_counted_without_mutating_original() {
        let r = reference(5. + 1e-9);
        let (c, n) = canonical_note_reference(&r).unwrap();
        assert_eq!(n, 1);
        assert_eq!(c.notes[0].end_seconds, 5.);
        assert_eq!(r.notes[0].end_seconds, 5. + 1e-9);
        assert!(c.frames.is_empty());
    }
    #[test]
    fn genuine_overlap_is_rejected_and_real_gap_is_preserved() {
        assert!(canonical_note_reference(&reference(5.0001)).is_err());
        let r = reference(4.98);
        let (c, n) = canonical_note_reference(&r).unwrap();
        assert_eq!(n, 0);
        assert_eq!(c.notes[0].end_seconds, 4.98);
    }
}
