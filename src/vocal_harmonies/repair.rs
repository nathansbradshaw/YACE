//! Repair one independent singer without moving another singer's shared ranges.
use super::{HarmonyChart, Part};
use crate::{
    snap_grid::{RepairPolicy, RepairPreview},
    vocals::{VocalNote, VocalPhrase},
};
use anyhow::{Context, Result, ensure};

fn retain_range(before: &VocalNote, after: &VocalNote, ranges: &[VocalPhrase]) -> Result<()> {
    let owner = ranges
        .partition_point(|p| p.start <= before.start)
        .checked_sub(1)
        .and_then(|i| ranges.get(i))
        .context("Missing harmony phrase owner")?;
    ensure!(
        owner.start <= after.start && after.end <= owner.end,
        "Repair would leave the original shared harmony phrase; choose a finer grid or smaller tolerance"
    );
    Ok(())
}

impl HarmonyChart {
    /// Immutable whole-voice preview. Original order retains lyric/word owners;
    /// overlaps and moves across shared scoring/lyric ranges refuse atomically.
    pub fn preview_timing_repair(
        &self,
        part: Part,
        policy: RepairPolicy,
    ) -> Result<(Self, RepairPreview)> {
        self.validate()?;
        let voice = self
            .voices
            .get(&part)
            .context("No editable notes for this harmony voice")?;
        ensure!(
            voice.notes.len() <= crate::note_groups::MAX_NOTES,
            "Repair at most 100,000 notes at once"
        );
        let grid = policy.grid(self.resolution)?;
        let mut candidate = self.clone();
        let mut report = RepairPreview::default();
        for (before, after) in voice
            .notes
            .iter()
            .zip(&mut candidate.voices.get_mut(&part).unwrap().notes)
        {
            after.start = policy.destination(grid, before.start)?;
            after.end = after
                .start
                .checked_add(before.end - before.start)
                .context("Harmony timing overflow")?;
            retain_range(before, after, &self.phrases)?;
            if part != Part::Harm1 {
                retain_range(before, after, &self.lyric_phrases)?;
            }
            report.observe(
                before.start,
                grid.snap_tick(before.start)
                    .context("Grid timing overflow")?,
                after.start,
            );
        }
        candidate.validate()?;
        Ok((candidate, report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{song_files::game_midi::NoteAttributes, vocals::Syllable};
    fn near() -> RepairPolicy {
        RepairPolicy {
            divisions: 4,
            maximum_shift: Some(5),
        }
    }
    #[test]
    fn each_voice_has_an_immutable_preview_without_changing_shared_owners_or_other_singers() {
        for part in Part::ALL {
            let mut before = crate::vocal_harmonies::tests::fixture();
            let note = &mut before.voices.get_mut(&part).unwrap().notes[0];
            note.start += 3;
            note.end += 3;
            note.midi = Some(NoteAttributes {
                channel: 4,
                velocity: 89,
                off_velocity: 17,
                off_as_note_on: false,
                zero_length: false,
                talkie_pitch: None,
            });
            let frozen = before.clone();
            let (after, report) = before.preview_timing_repair(part, near()).unwrap();
            assert_eq!(before, frozen);
            assert_eq!(
                (report.examined, report.changed, report.largest_shift),
                (1, 1, 3)
            );
            let mut expected = before.clone();
            let note = &mut expected.voices.get_mut(&part).unwrap().notes[0];
            note.start -= 3;
            note.end -= 3;
            assert_eq!(after, expected);
            assert_eq!(
                after.preview_timing_repair(part, near()).unwrap().1.changed,
                0
            );
            let kept = before
                .preview_timing_repair(
                    part,
                    RepairPolicy {
                        maximum_shift: Some(2),
                        ..near()
                    },
                )
                .unwrap();
            assert_eq!(kept.0, before);
            assert_eq!(kept.1.kept_off_grid, 1);
        }
    }
    #[test]
    fn coarse_repair_refuses_collisions_and_shared_score_or_lyric_owner_changes() {
        let mut before = crate::vocal_harmonies::tests::fixture();
        let first = &mut before.voices.get_mut(&Part::Harm2).unwrap().notes[0];
        first.start = 243;
        first.end = 300;
        let mut second = first.clone();
        second.start = 360;
        second.end = 400;
        second.lyric = Syllable::Text {
            text: "next".into(),
            join_next: false,
        };
        before
            .voices
            .get_mut(&Part::Harm2)
            .unwrap()
            .notes
            .push(second);
        before.validate().unwrap();
        let frozen = before.clone();
        assert!(
            before
                .preview_timing_repair(
                    Part::Harm2,
                    RepairPolicy {
                        divisions: 1,
                        maximum_shift: None
                    }
                )
                .is_err()
        );
        assert_eq!(before, frozen);
        for lyrics in [false, true] {
            let mut before = crate::vocal_harmonies::tests::fixture();
            let note = &mut before.voices.get_mut(&Part::Harm3).unwrap().notes[0];
            note.start = 3;
            note.end = 63;
            if lyrics {
                before.lyric_phrases[0].start = 2;
            } else {
                before.phrases[0].start = 2;
            }
            before.validate().unwrap();
            assert!(
                before
                    .preview_timing_repair(Part::Harm3, near())
                    .unwrap_err()
                    .to_string()
                    .contains("original shared harmony phrase")
            );
        }
    }
    #[test]
    fn joined_words_and_pitch_continuations_keep_their_original_order_and_owner() {
        let mut before = crate::vocal_harmonies::tests::fixture();
        let voice = before.voices.get_mut(&Part::Harm2).unwrap();
        let mut first = voice.notes[0].clone();
        first.start = 123;
        first.end = 180;
        first.lyric = Syllable::Text {
            text: "sing".into(),
            join_next: true,
        };
        let mut continuation = first.clone();
        continuation.start = 243;
        continuation.end = 280;
        continuation.lyric = Syllable::Continue;
        let mut last = first.clone();
        last.start = 363;
        last.end = 430;
        last.lyric = Syllable::Text {
            text: "ing".into(),
            join_next: false,
        };
        voice.notes = vec![first, continuation, last];
        let (after, report) = before.preview_timing_repair(Part::Harm2, near()).unwrap();
        assert_eq!(report.changed, 3);
        for (original, repaired) in before.voices[&Part::Harm2]
            .notes
            .iter()
            .zip(&after.voices[&Part::Harm2].notes)
        {
            assert_eq!(original.lyric, repaired.lyric);
            assert_eq!(original.tone, repaired.tone);
            assert_eq!(repaired.start, original.start - 3);
            assert_eq!(repaired.end, original.end - 3);
        }
    }
}
