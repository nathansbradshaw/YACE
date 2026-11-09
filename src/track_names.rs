//! Display names for editable and preserved interchange tracks.
//! Keep unknown source identifiers visible rather than guessing an instrument.
pub fn title(track: &str) -> String {
    for tier in ["Expert", "Hard", "Medium", "Easy"] {
        let Some(part) = track.strip_prefix(tier) else {
            continue;
        };
        let instrument = match part {
            "Single" => "Guitar",
            "DoubleBass" => "Bass",
            "DoubleRhythm" => "Rhythm guitar",
            "DoubleGuitar" => "Co-op guitar",
            "GHLGuitar" => "Six-fret guitar",
            "GHLBass" => "Six-fret bass",
            "GHLRhythm" => "Six-fret rhythm guitar",
            "GHLCoop" => "Six-fret co-op guitar",
            "GHLKeys" => "Six-fret keys",
            "Drums" => "Drums",
            "Keyboard" => "Piano",
            "ProKeys" => "Pro Piano",
            "ProGuitar17" => "Pro Guitar (17 fret)",
            "ProGuitar22" => "Pro Guitar (22 fret)",
            "ProBass17" => "Pro Bass (17 fret)",
            "ProBass22" => "Pro Bass (22 fret)",
            _ => return track.to_owned(),
        };
        return format!("{instrument} · {tier}");
    }
    track.to_owned()
}

#[cfg(test)]
mod tests {
    use super::title;
    use std::collections::BTreeSet;

    #[test]
    fn interchange_parts_have_distinct_choices_at_every_difficulty() {
        let parts = [
            ("Single", "Guitar"),
            ("DoubleBass", "Bass"),
            ("DoubleRhythm", "Rhythm guitar"),
            ("DoubleGuitar", "Co-op guitar"),
            ("GHLGuitar", "Six-fret guitar"),
            ("GHLBass", "Six-fret bass"),
            ("GHLRhythm", "Six-fret rhythm guitar"),
            ("GHLCoop", "Six-fret co-op guitar"),
            ("GHLKeys", "Six-fret keys"),
            ("Drums", "Drums"),
            ("Keyboard", "Piano"),
            ("ProKeys", "Pro Piano"),
        ];
        let mut choices = BTreeSet::new();
        for tier in ["Easy", "Medium", "Hard", "Expert"] {
            for (part, name) in parts {
                let label = title(&format!("{tier}{part}"));
                assert_eq!(label, format!("{name} · {tier}"));
                assert!(choices.insert(label), "Indistinguishable track choice");
            }
        }
        assert_eq!(choices.len(), 48);
    }

    #[test]
    fn opaque_and_unknown_parts_retain_their_source_identity() {
        for track in [
            "Vocals",
            "MIDI: PART KEYS GHL",
            "ExpertCustomInstrument",
            "ExpertSingleExtra",
            "Expert",
            "CustomSingle",
        ] {
            assert_eq!(title(track), track);
        }
    }
}
