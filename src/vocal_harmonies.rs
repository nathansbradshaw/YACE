//! Independent harmony voices with the MIDI format's shared phrase ownership.
//! Lead vocals remain a separate document; overlapping singers never share a
//! monophonic note list. HARM1 owns scoring, HARM2 owns the second lyric range.
//! Format reference: https://github.com/TheNathannator/GuitarGame_ChartFormats/blob/main/docs/Chart-File-Formats/mid-format/Tracks/Vocals.md
use crate::{
    timing::TempoMap,
    vocals::{VocalChart, VocalNote, VocalPhrase},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[path = "vocal_harmonies/midi.rs"]
mod midi;
pub use midi::{append_midi, clear_midi, import_midi, rewrite_midi};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Part {
    Harm1,
    Harm2,
    Harm3,
}
impl Part {
    pub const ALL: [Self; 3] = [Self::Harm1, Self::Harm2, Self::Harm3];
    pub fn track(self) -> &'static str {
        match self {
            Self::Harm1 => "HARM1",
            Self::Harm2 => "HARM2",
            Self::Harm3 => "HARM3",
        }
    }
    pub fn from_midi(name: &str) -> Option<Self> {
        match name.trim().to_ascii_uppercase().as_str() {
            "HARM1" | "PART HARM1" => Some(Self::Harm1),
            "HARM2" | "PART HARM2" => Some(Self::Harm2),
            "HARM3" | "PART HARM3" => Some(Self::Harm3),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Voice {
    pub source: String,
    pub notes: Vec<VocalNote>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HarmonyChart {
    pub version: u32,
    pub resolution: u32,
    pub source: String,
    /// HARM1 scoring phrases apply to all voices, including HARM2/HARM3-only spans.
    pub phrases: Vec<VocalPhrase>,
    /// HARM2 lyric ranges must cover both HARM2 and HARM3; HARM3 owns no markers.
    pub lyric_phrases: Vec<VocalPhrase>,
    pub voices: BTreeMap<Part, Voice>,
}

fn contains(phrase: &VocalPhrase, note: &VocalNote) -> bool {
    phrase.start <= note.start && note.end <= phrase.end
}
fn phrase_shape(phrases: &[VocalPhrase]) -> Result<()> {
    ensure!(phrases.len() <= 16384, "Too many harmony phrases");
    ensure!(
        phrases.iter().all(|phrase| phrase.start < phrase.end
            && phrase.end <= u64::from(u32::MAX)
            && matches!(phrase.player, 1 | 2)),
        "Invalid harmony phrase span or player"
    );
    ensure!(
        phrases.windows(2).all(|pair| pair[0].end <= pair[1].start),
        "Harmony phrases must be ordered and non-overlapping"
    );
    Ok(())
}
impl HarmonyChart {
    /// A display/validation view of one voice. It does not transfer ownership
    /// of scoring phrases or mutate another singer's notes.
    pub fn voice_chart(&self, part: Part) -> Option<VocalChart> {
        let voice = self.voices.get(&part)?;
        Some(VocalChart {
            version: 1,
            resolution: self.resolution,
            source: voice.source.clone(),
            phrases: self
                .phrases
                .iter()
                .filter(|phrase| voice.notes.iter().any(|note| contains(phrase, note)))
                .cloned()
                .collect(),
            notes: voice.notes.clone(),
        })
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && (1..=32767).contains(&self.resolution),
            "Unsupported harmony document version or resolution"
        );
        ensure!(
            !self.source.trim().is_empty(),
            "Harmony source/provenance is required"
        );
        ensure!(
            !self.voices.is_empty() && self.voices.values().any(|voice| !voice.notes.is_empty()),
            "Harmony chart needs at least one authored voice"
        );
        phrase_shape(&self.phrases)?;
        phrase_shape(&self.lyric_phrases)?;
        for (&part, voice) in &self.voices {
            ensure!(
                !voice.source.trim().is_empty(),
                "Harmony voice source/provenance is required"
            );
            if !voice.notes.is_empty() {
                // Each singer is monophonic, while simultaneous notes in other
                // voices are independent and must not trigger collisions.
                self.voice_chart(part).unwrap().validate()?;
            }
        }
        ensure!(
            self.phrases.iter().all(|phrase| self
                .voices
                .values()
                .any(|voice| voice.notes.iter().any(|note| contains(phrase, note)))),
            "Empty shared harmony scoring phrase"
        );
        for (&part, voice) in &self.voices {
            if part == Part::Harm1 {
                continue;
            }
            ensure!(
                voice.notes.iter().all(|note| self
                    .lyric_phrases
                    .iter()
                    .filter(|phrase| contains(phrase, note))
                    .count()
                    == 1),
                "HARM2 lyric ranges must cover every HARM2 and HARM3 note"
            );
        }
        ensure!(
            self.lyric_phrases
                .iter()
                .all(|phrase| [Part::Harm2, Part::Harm3]
                    .into_iter()
                    .filter_map(|part| self.voices.get(&part))
                    .any(|voice| voice.notes.iter().any(|note| contains(phrase, note)))),
            "Empty HARM2 lyric range"
        );
        Ok(())
    }
    pub fn validate_timing(&self, timing: &TempoMap, duration: f64) -> Result<()> {
        self.validate()?;
        ensure!(
            timing.resolution == self.resolution && duration.is_finite() && duration > 0.,
            "Harmony/project timing mismatch"
        );
        ensure!(self.voices.values().flat_map(|voice| &voice.notes).map(|note| note.end)
            .chain(self.phrases.iter().chain(&self.lyric_phrases).map(|phrase| phrase.end))
            .all(|end| timing.seconds(end).is_finite() && timing.seconds(end) <= duration + 0.1),
            "Harmony notes or phrases extend beyond the timeline");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vocals::{Syllable, VocalTone};
    fn apply(store: &crate::editor::Store, action: crate::editor::Action) -> Result<()> {
        use crate::editor::{Actor, ActorKind, Request};
        store.apply(Request {
            expected_revision: store.load()?.revision,
            actor: Actor {
                kind: ActorKind::Human,
                name: "Independent harmony project fixture".into(),
            },
            action,
        })?;
        Ok(())
    }
    fn project(midi: bool) -> (tempfile::TempDir, crate::editor::Store) {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("song");
        std::fs::create_dir(&source).unwrap();
        let text = "[Song]\n{\nResolution = 480\n}\n[SyncTrack]\n{\n0 = B 120000\n}\n[ExpertSingle]\n{\n240 = N 0 120\n}\n";
        if midi {
            let clock = crate::chart::Chart::parse(text.into()).unwrap();
            let original = crate::game_export::song(&clock, None).unwrap().0;
            let bytes = append_midi(&original, &fixture()).unwrap();
            let mut midi = midly::Smf::parse(&bytes).unwrap();
            let duplicate = midi
                .tracks
                .iter()
                .find(|track| {
                    track.iter().any(|event| {
                        matches!(
                            event.kind,
                            midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(b"HARM3"))
                        )
                    })
                })
                .unwrap()
                .clone();
            midi.tracks.push(duplicate);
            let mut opaque = Vec::new();
            midi.write_std(&mut opaque).unwrap();
            std::fs::write(source.join("notes.mid"), opaque).unwrap();
        } else {
            std::fs::write(source.join("notes.chart"), text).unwrap();
        }
        std::fs::write(
            source.join("song.ini"),
            "[song]\nname = Harmony project fixture\nsong_length = 4000\n",
        )
        .unwrap();
        let store =
            crate::editor::Store::create(&source, None, &root.path().join("project")).unwrap();
        (root, store)
    }
    #[test]
    fn independent_harmony_project_reopens_with_undo_redo_and_unchanged_lead() {
        use crate::editor::{Action, Store};
        let (_root, store) = project(false);
        let document = fixture();
        let lead = document.voice_chart(Part::Harm1).unwrap();
        apply(
            &store,
            Action::SetVocals {
                document: lead.clone(),
            },
        )
        .unwrap();
        apply(
            &store,
            Action::SetHarmonies {
                document: document.clone(),
            },
        )
        .unwrap();
        let mut edited = document.clone();
        let note = &mut edited.voices.get_mut(&Part::Harm2).unwrap().notes[0];
        note.start += 1;
        note.tone = VocalTone::Pitched { midi: 65 };
        apply(
            &store,
            Action::SetHarmonies {
                document: edited.clone(),
            },
        )
        .unwrap();
        let reopened = Store::new(&store.root);
        let doc = reopened.load().unwrap();
        assert_eq!(doc.content.harmonies, Some(edited.clone()));
        assert_eq!(doc.content.vocals, Some(lead.clone()));
        apply(&reopened, Action::Undo).unwrap();
        assert_eq!(reopened.load().unwrap().content.harmonies, Some(document));
        apply(&reopened, Action::Redo).unwrap();
        assert_eq!(reopened.load().unwrap().content.harmonies, Some(edited));
        assert_eq!(reopened.load().unwrap().content.vocals, Some(lead));
        let bytes = serde_json::to_vec(&crate::editor::Content::default()).unwrap();
        let legacy: crate::editor::Content = serde_json::from_slice(&bytes).unwrap();
        assert!(legacy.harmonies.is_none());
        assert!(!String::from_utf8(bytes).unwrap().contains("harmonies"));
    }
    #[test]
    fn invalid_timing_and_chart_conversion_leave_harmony_project_intact() {
        use crate::editor::Action;
        let (root, store) = project(false);
        apply(
            &store,
            Action::SetHarmonies {
                document: fixture(),
            },
        )
        .unwrap();
        let path = store.root.join("project.json");
        let before = std::fs::read(&path).unwrap();
        let mut invalid = fixture();
        invalid.phrases[0].end = 12000;
        invalid.lyric_phrases[0].end = 12000;
        invalid.voices.get_mut(&Part::Harm2).unwrap().notes[0].end = 10000;
        assert!(
            apply(&store, Action::SetHarmonies { document: invalid })
                .unwrap_err()
                .to_string()
                .contains("beyond the timeline")
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let output = root.path().join("export");
        let error = store
            .export_with_options(
                &store.load().unwrap(),
                &output,
                &crate::editor::Actor {
                    kind: crate::editor::ActorKind::Human,
                    name: "Explicit chart conversion fixture".into(),
                },
                &crate::export_options::Options {
                    format: crate::export_options::Format::Chart,
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert!(error.to_string().contains("require MIDI"));
        assert!(!output.exists());
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
    #[test]
    fn original_opaque_harmonies_cannot_be_replaced_without_an_editable_baseline() {
        let (_root, store) = project(true);
        let path = store.root.join("project.json");
        let before = std::fs::read(&path).unwrap();
        assert!(
            apply(
                &store,
                crate::editor::Action::SetHarmonies {
                    document: fixture()
                }
            )
            .unwrap_err()
            .to_string()
            .contains("imported editable baseline")
        );
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
    #[test]
    fn portable_harmony_copy_keeps_independent_voices_and_content_addressed_history() {
        use crate::{
            editor::{Action, Store},
            project_portability::{self, Files},
        };
        fn collect(root: &std::path::Path, path: &std::path::Path, files: &mut Files) {
            for entry in std::fs::read_dir(path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    collect(root, &path, files);
                } else {
                    files.insert(
                        path.strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .into_owned(),
                        std::fs::read(path).unwrap(),
                    );
                }
            }
        }
        let (root, store) = project(false);
        let before = fixture();
        apply(
            &store,
            Action::SetHarmonies {
                document: before.clone(),
            },
        )
        .unwrap();
        let mut after = before.clone();
        after.voices.get_mut(&Part::Harm3).unwrap().notes[0].tone = VocalTone::Pitched { midi: 70 };
        apply(
            &store,
            Action::SetHarmonies {
                document: after.clone(),
            },
        )
        .unwrap();
        let mut files = Files::new();
        collect(&store.root, &store.root, &mut files);
        let destination = tempfile::tempdir().unwrap();
        project_portability::relocate(&mut files, destination.path()).unwrap();
        for (name, bytes) in files {
            let path = destination.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        drop(root);
        let moved = Store::new(destination.path());
        assert_eq!(moved.load().unwrap().content.harmonies, Some(after.clone()));
        apply(&moved, Action::Undo).unwrap();
        assert_eq!(moved.load().unwrap().content.harmonies, Some(before));
        apply(&moved, Action::Redo).unwrap();
        assert_eq!(moved.load().unwrap().content.harmonies, Some(after));
    }
    #[test]
    fn manually_authored_harmonies_export_as_midi_and_reopen_without_an_original_midi() {
        use crate::editor::{Action, Store};
        let (root, store) = project(false);
        let expected = fixture();
        apply(
            &store,
            Action::SetHarmonies {
                document: expected.clone(),
            },
        )
        .unwrap();
        let output = root.path().join("new-harmony-song");
        apply(
            &store,
            Action::Export {
                output: output.clone(),
            },
        )
        .unwrap();
        assert!(output.join("notes.mid").is_file());
        assert!(!output.join("notes.chart").exists());
        assert!(
            std::fs::read_to_string(output.join("song.ini"))
                .unwrap()
                .contains("diff_vocals_harm = 0")
        );
        let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
        assert_eq!(reopened.load().unwrap().content.harmonies, Some(expected));
    }
    pub(super) fn fixture() -> HarmonyChart {
        HarmonyChart {
            version: 1,
            resolution: 480,
            source: "Manual harmony fixture".into(),
            phrases: vec![VocalPhrase {
                start: 0,
                end: 960,
                player: 1,
            }],
            lyric_phrases: vec![VocalPhrase {
                start: 0,
                end: 960,
                player: 1,
            }],
            voices: Part::ALL
                .into_iter()
                .enumerate()
                .map(|(index, part)| {
                    (
                        part,
                        Voice {
                            source: format!("Independent {}", part.track()),
                            notes: vec![VocalNote {
                                start: 240,
                                end: 720,
                                tone: VocalTone::Pitched {
                                    midi: 60 + index as u8 * 4,
                                },
                                lyric: Syllable::Text {
                                    text: format!("voice{index}"),
                                    join_next: false,
                                },
                                midi: None,
                            }],
                        },
                    )
                })
                .collect(),
        }
    }
    #[test]
    fn simultaneous_singers_remain_independent_and_round_trip_the_document() {
        let mut chart = fixture();
        chart.validate().unwrap();
        let others = (
            chart.voices[&Part::Harm1].clone(),
            chart.voices[&Part::Harm3].clone(),
        );
        chart.voices.get_mut(&Part::Harm2).unwrap().notes[0].tone = VocalTone::Pitched { midi: 65 };
        chart.validate().unwrap();
        assert_eq!(
            others,
            (
                chart.voices[&Part::Harm1].clone(),
                chart.voices[&Part::Harm3].clone()
            )
        );
        let encoded = serde_json::to_vec(&chart).unwrap();
        assert_eq!(
            serde_json::from_slice::<HarmonyChart>(&encoded).unwrap(),
            chart
        );
        let voice = chart.voice_chart(Part::Harm3).unwrap();
        assert_eq!(voice.notes[0].start, 240);
        assert_eq!(voice.phrases, chart.phrases);
    }
    #[test]
    fn collisions_within_a_voice_and_uncovered_shared_ranges_refuse() {
        let mut chart = fixture();
        let duplicate = chart.voices[&Part::Harm2].notes[0].clone();
        chart
            .voices
            .get_mut(&Part::Harm2)
            .unwrap()
            .notes
            .push(duplicate);
        assert!(chart.validate().is_err());
        let mut chart = fixture();
        chart.phrases[0].end = 600;
        assert!(chart.validate().is_err());
        let mut chart = fixture();
        chart.lyric_phrases[0].end = 600;
        assert!(chart.validate().is_err());
    }
    #[test]
    fn shared_scoring_can_cover_a_span_with_no_harm1_notes() {
        let mut chart = fixture();
        chart.voices.remove(&Part::Harm1);
        chart.validate().unwrap();
        assert!(chart.voice_chart(Part::Harm1).is_none());
        assert_eq!(Part::from_midi("PART HARM3"), Some(Part::Harm3));
        assert_eq!(Part::from_midi("PART VOCALS"), None);
    }
}
