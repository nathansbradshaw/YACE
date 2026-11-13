use super::*;
use crate::{
    vocal_harmonies::{Part as HarmonyPart, Voice},
    vocals::{Syllable, VocalNote, VocalTone},
};
pub(super) fn fixture() -> PercussionChart {
    let track = Track {
        source: "Manual percussion".into(),
        phrases: vec![Phrase {
            range: VocalPhrase {
                midi: None,
                start: 0,
                end: 960,
                player: 1,
            },
            midi: None,
        }],
        hits: vec![Hit {
            start: 240,
            end: 260,
            displayed: true,
            midi: None,
        }],
    };
    PercussionChart {
        version: 1,
        resolution: 480,
        source: "Independent vocal percussion fixture".into(),
        tracks: BTreeMap::from([(Part::Lead, track.clone()), (Part::Harm1, track)]),
    }
}
#[test]
fn displayed_hidden_and_zero_length_hits_round_trip_without_syllables() {
    let mut chart = fixture();
    chart.tracks.get_mut(&Part::Lead).unwrap().hits.push(Hit {
        start: 480,
        end: 480,
        displayed: false,
        midi: Some(NoteAttributes {
            channel: 6,
            velocity: 87,
            off_velocity: 0,
            off_as_note_on: true,
            zero_length: true,
            talkie_pitch: None,
        }),
    });
    chart.validate().unwrap();
    let bytes = serde_json::to_vec(&chart).unwrap();
    assert_eq!(
        serde_json::from_slice::<PercussionChart>(&bytes).unwrap(),
        chart
    );
    let lead = &chart.tracks[&Part::Lead];
    assert_eq!(lead.hits[0].key(), 96);
    assert_eq!(lead.hits[1].key(), 97);
    assert_eq!(Part::from_midi("PART HARM1"), Some(Part::Harm1));
    assert_eq!(Part::from_midi("PART HARM2"), None);
}
#[test]
fn simultaneous_parts_are_independent_but_duplicate_or_held_hits_in_one_part_refuse() {
    let chart = fixture();
    chart.validate().unwrap();
    let mut duplicate = chart.clone();
    let lead = duplicate.tracks.get_mut(&Part::Lead).unwrap();
    lead.hits.push(lead.hits[0].clone());
    assert!(duplicate.validate().is_err());
    let mut overlap = chart.clone();
    overlap
        .tracks
        .get_mut(&Part::Harm1)
        .unwrap()
        .hits
        .push(Hit {
            start: 250,
            end: 270,
            displayed: false,
            midi: None,
        });
    assert!(overlap.validate().is_err());
    assert_eq!(chart.tracks[&Part::Lead].hits.len(), 1);
}
#[test]
fn hidden_hits_need_a_complete_phrase_and_empty_or_overlapping_phrases_refuse() {
    let chart = fixture();
    let mut uncovered = chart.clone();
    let hit = &mut uncovered.tracks.get_mut(&Part::Lead).unwrap().hits[0];
    hit.displayed = false;
    hit.end = 961;
    assert!(uncovered.validate().is_err());
    let mut empty = chart.clone();
    empty
        .tracks
        .get_mut(&Part::Lead)
        .unwrap()
        .phrases
        .push(Phrase {
            range: VocalPhrase {
                midi: None,
                start: 960,
                end: 1920,
                player: 2,
            },
            midi: None,
        });
    assert!(empty.validate().is_err());
    let mut overlap = chart.clone();
    let lead = overlap.tracks.get_mut(&Part::Lead).unwrap();
    lead.phrases.push(lead.phrases[0].clone());
    assert!(overlap.validate().is_err());
    let mut at_end = chart;
    let hit = &mut at_end.tracks.get_mut(&Part::Lead).unwrap().hits[0];
    hit.start = 960;
    hit.end = 960;
    assert!(at_end.validate().is_err());
}
#[test]
fn imported_hit_attributes_refuse_talkie_pitch_and_inconsistent_duration_or_note_off() {
    let mut chart = fixture();
    let mut attributes = NoteAttributes {
        channel: 3,
        velocity: 85,
        off_velocity: 17,
        off_as_note_on: false,
        zero_length: false,
        talkie_pitch: None,
    };
    chart.tracks.get_mut(&Part::Lead).unwrap().hits[0].midi = Some(attributes.clone());
    chart.validate().unwrap();
    attributes.talkie_pitch = Some(60);
    chart.tracks.get_mut(&Part::Lead).unwrap().hits[0].midi = Some(attributes.clone());
    assert!(chart.validate().is_err());
    attributes.talkie_pitch = None;
    attributes.zero_length = true;
    chart.tracks.get_mut(&Part::Lead).unwrap().hits[0].midi = Some(attributes.clone());
    assert!(chart.validate().is_err());
    attributes.zero_length = false;
    attributes.off_as_note_on = true;
    chart.tracks.get_mut(&Part::Lead).unwrap().hits[0].midi = Some(attributes);
    assert!(chart.validate().is_err());
}
#[test]
fn percussion_cannot_reclassify_lead_or_other_harmony_singers_scoring_phrases() {
    let chart = fixture();
    let note = VocalNote {
        start: 480,
        end: 720,
        tone: VocalTone::Pitched { midi: 60 },
        lyric: Syllable::Missing,
        midi: None,
    };
    let range = VocalPhrase {
        midi: None,
        start: 0,
        end: 960,
        player: 1,
    };
    let mut lead = VocalChart {
        version: 1,
        resolution: 480,
        source: "Existing lead".into(),
        phrases: vec![range.clone()],
        notes: vec![note.clone()],
    };
    assert!(chart.validate_with_singing(Some(&lead), None).is_err());
    let mut harmonies = HarmonyChart {
        version: 1,
        resolution: 480,
        source: "Existing HARM3".into(),
        phrases: vec![range.clone()],
        lyric_phrases: vec![range],
        voices: BTreeMap::from([(
            HarmonyPart::Harm3,
            Voice {
                source: "Existing third singer".into(),
                notes: vec![note],
            },
        )]),
    };
    assert!(chart.validate_with_singing(None, Some(&harmonies)).is_err());
    lead.notes[0].start = 1200;
    lead.notes[0].end = 1440;
    lead.phrases[0].start = 960;
    lead.phrases[0].end = 1920;
    let voice = harmonies.voices.get_mut(&HarmonyPart::Harm3).unwrap();
    voice.notes[0].start = 1200;
    voice.notes[0].end = 1440;
    harmonies.phrases = lead.phrases.clone();
    harmonies.lyric_phrases = lead.phrases.clone();
    chart
        .validate_with_singing(Some(&lead), Some(&harmonies))
        .unwrap();
    assert_eq!(harmonies.voices[&HarmonyPart::Harm3].notes[0].start, 1200);
}
#[test]
fn percussion_uses_the_existing_clock_and_refuses_beyond_the_recording() {
    let chart = fixture();
    let timing = TempoMap::constant(120.).unwrap();
    chart.validate_timing(&timing, 1.).unwrap();
    assert!(chart.validate_timing(&timing, 0.8).is_err());
    let mut other_clock = timing;
    other_clock.resolution = 960;
    assert!(chart.validate_timing(&other_clock, 1.).is_err());
}
