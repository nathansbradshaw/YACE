//! Six-fret source lanes are not five-fret colors or technique markers.
pub const PARTS: [(&str, &str); 5] = [
    ("Six-fret guitar", "GHLGuitar"),
    ("Six-fret bass", "GHLBass"),
    ("Six-fret rhythm guitar", "GHLRhythm"),
    ("Six-fret co-op guitar", "GHLCoop"),
    ("Six-fret keys", "GHLKeys"),
];
pub const ROWS: [u8; 7] = [0, 1, 2, 3, 4, 8, 7];

pub fn is_track(track: &str) -> bool {
    PARTS.iter().any(|(_, suffix)| track.ends_with(suffix))
}
pub fn name(lane: u8) -> Option<&'static str> {
    Some(match lane {
        0 => "White 1",
        1 => "White 2",
        2 => "White 3",
        3 => "Black 1",
        4 => "Black 2",
        8 => "Black 3",
        7 => "Open",
        _ => return None,
    })
}
pub fn color(lane: u8) -> &'static str {
    match lane {
        0..=2 => "#e9eff5",
        3 | 4 | 8 => "#697789",
        _ => "#c889ff",
    }
}
/// Three paired columns; technique lanes5/6 never receive a gem position.
pub fn column(lane: u8) -> Option<usize> {
    match lane {
        0 | 3 => Some(0),
        1 | 4 => Some(1),
        2 | 8 => Some(2),
        _ => None,
    }
}
pub fn midi_key(lane: u8, base: u8) -> Option<u8> {
    match lane {
        0..=4 => Some(base + lane - 1),
        8 => Some(base + 4),
        7 => Some(base - 2),
        _ => None,
    }
}
pub fn chart_lane(key: u8, base: u8) -> Option<u8> {
    match key.checked_sub(base - 2)? {
        0 => Some(7),
        1..=5 => Some(key + 1 - base),
        6 => Some(8),
        _ => None,
    }
}
pub fn midi_name(suffix: &str) -> Option<&'static str> {
    Some(match suffix {
        "GHLGuitar" => "MIDI: PART GUITAR GHL",
        "GHLBass" => "MIDI: PART BASS GHL",
        "GHLRhythm" => "MIDI: PART RHYTHM GHL",
        "GHLCoop" => "MIDI: PART GUITAR COOP GHL",
        "GHLKeys" => "MIDI: PART KEYS GHL",
        _ => return None,
    })
}

pub fn rank(track: &str) -> Option<&'static str> {
    PARTS
        .iter()
        .position(|(_, suffix)| track.ends_with(suffix))
        .map(|index| {
            [
                "diff_guitarghl",
                "diff_bassghl",
                "diff_rhythm_ghl",
                "diff_guitar_coop_ghl",
                "diff_keys_ghl",
            ][index]
        })
}
