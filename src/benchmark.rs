//! Read-only comparisons against human charts on the same recording.
//! Five-fret colors are an adaptation, not acoustic MIDI pitch ground truth.
use crate::{midi, models};
use anyhow::{Context, Result, ensure};
use midly::{MetaMessage, MidiMessage, Smf, TrackEventKind};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Instrument {
    Bass,
    Vocals,
    Guitar,
    Drums,
    ProDrums,
}

#[derive(Clone, Debug, Serialize)]
pub struct Note {
    pub start: f64,
    pub end: f64,
    pub pitch: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lane: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cymbal: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Parsed {
    pub notes: Vec<Note>,
    pub excluded_note_onsets: usize,
    pub phrase_markers: usize,
    pub pitch_bend_events: usize,
    pub excluded_unphrased_vocal_notes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded_expert_plus_kicks: Option<usize>,
}

fn is_drum(instrument: Instrument) -> bool {
    matches!(instrument, Instrument::Drums | Instrument::ProDrums)
}
// YARG.Core TextEvents.TryParseDrumsMixEvent: Expert is3; 'd' alone
// activates disco flip, 'dnoflip' and all other settings disable it.
fn disco_event(text: &str) -> Option<bool> {
    let text = text.trim().trim_matches(['[', ']']);
    let trim: fn(&str) -> &str = |s| s.trim_start_matches([' ', '_']);
    let text = trim(text.strip_prefix("mix")?);
    let text = trim(text.strip_prefix('3')?);
    let text = trim(text.strip_prefix("drums")?);
    let config = text.as_bytes().first()?;
    if !(b'0'..=b'5').contains(config) {
        return None;
    }
    Some(trim(&text[1..]) == "d")
}
fn pro_pad(lane: u8, cymbal: bool, flip: bool) -> (u8, bool) {
    if flip {
        match lane {
            1 => (2, true),
            2 => (1, false),
            _ => (lane, cymbal),
        }
    } else {
        (lane, cymbal)
    }
}

/// Extract text only. MIDI lyric timestamps and phrase markers never become
/// inference inputs in the supplied-text benchmark condition.
pub fn lyric_source(bytes: &[u8]) -> Result<crate::vocal_generation::LyricSource> {
    let parsed = Smf::parse(bytes)?;
    let tracks: Vec<_> = parsed
        .tracks
        .iter()
        .filter(|track| {
            track.iter().any(|e| {
                matches!(
                    e.kind,
                    TrackEventKind::Meta(MetaMessage::TrackName(b"PART VOCALS"))
                )
            })
        })
        .collect();
    ensure!(
        tracks.len() == 1,
        "Expected exactly one PART VOCALS lyric source"
    );
    let mut words: Vec<String> = Vec::new();
    let mut joined = false;
    for event in tracks[0] {
        let text = match event.kind {
            TrackEventKind::Meta(MetaMessage::Lyric(text)) => text,
            TrackEventKind::Meta(MetaMessage::Text(text)) if !text.starts_with(b"[") => text,
            _ => continue,
        };
        let text = std::str::from_utf8(text)?.trim();
        if text.is_empty() || text == "+" || text.starts_with('[') {
            continue;
        }
        let base = text.trim_end_matches(|character| "-=+#^*%/$".contains(character));
        let flags = &text[base.len()..];
        let next_joined = flags.contains(['-', '=']);
        let text = base;
        if text.is_empty() {
            continue;
        }
        if joined && !words.is_empty() {
            words.last_mut().unwrap().push_str(text);
        } else {
            words.push(text.to_owned());
        }
        joined = next_joined;
    }
    ensure!(!words.is_empty(), "No lead-vocal lyric text");
    Ok(crate::vocal_generation::LyricSource {
        text: words.join(" "),
        synced_lyrics: None,
        source: format!(
            "Human-authored reference MIDI {:x}; supplied-text benchmark condition; lyric text only, no word/phrase timing supplied",
            sha2::Sha256::digest(bytes)
        ),
    })
}

pub fn attack_times(parsed: &Parsed) -> Vec<f64> {
    attacks(parsed)
}

fn unpitched_lyric(lyric: &[u8]) -> bool {
    lyric
        .iter()
        .rev()
        .take_while(|byte| b"-=+#^*%/$".contains(byte))
        .any(|byte| b"#^*".contains(byte))
}

pub fn parse(bytes: &[u8], instrument: Instrument, delay_ms: f64) -> Result<Parsed> {
    parse_internal(bytes, instrument, delay_ms, false)
}

// A missing generated PART can be an intentional abstention. Require complete
// SMF chunk framing here because midly's non-strict mode tolerates truncation
// and mismatched declared track counts. Do not turn damaged files into silence.
fn validate_absent_part_midi(bytes: &[u8], parsed: &Smf<'_>) -> Result<()> {
    ensure!(bytes.len() >= 14, "Incomplete generated MIDI header");
    let header_len = u32::from_be_bytes(bytes[4..8].try_into()?) as usize;
    ensure!(header_len >= 6, "Incomplete generated MIDI header");
    let mut position = 8usize
        .checked_add(header_len)
        .context("Generated MIDI header overflow")?;
    ensure!(position <= bytes.len(), "Truncated generated MIDI header");
    let declared = u16::from_be_bytes(bytes[10..12].try_into()?) as usize;
    let mut tracks = 0usize;
    while position < bytes.len() {
        let header = bytes
            .get(position..position.saturating_add(8))
            .context("Truncated generated MIDI chunk header")?;
        let length = u32::from_be_bytes(header[4..8].try_into()?) as usize;
        let end = position
            .checked_add(8)
            .and_then(|start| start.checked_add(length))
            .context("Generated MIDI chunk overflow")?;
        ensure!(end <= bytes.len(), "Truncated generated MIDI chunk");
        if &header[..4] == b"MTrk" {
            tracks += 1;
            let mut events = midly::EventIter::new(&bytes[position + 8..end]);
            let mut ended = false;
            while !events.unread().is_empty() {
                let event = events.next().context("Malformed generated MIDI event")??;
                ended = matches!(event.kind, TrackEventKind::Meta(MetaMessage::EndOfTrack));
                ensure!(
                    !ended || events.unread().is_empty(),
                    "Generated MIDI data follows end-of-track"
                );
            }
            ensure!(ended, "Generated MIDI track has no end-of-track event");
        } else {
            ensure!(&header[..4] != b"MThd", "Duplicate generated MIDI header");
        }
        position = end;
    }
    ensure!(
        tracks > 0 && tracks == declared && tracks == parsed.tracks.len(),
        "Generated MIDI track count mismatch"
    );
    ensure!(
        parsed.header.format != midly::Format::SingleTrack || tracks == 1,
        "Invalid generated single-track MIDI"
    );
    ensure!(
        parsed.tracks.iter().all(|track| matches!(
            track.last().map(|event| event.kind),
            Some(TrackEventKind::Meta(MetaMessage::EndOfTrack))
        )),
        "Generated MIDI track has no final end-of-track event"
    );
    Ok(())
}

fn parse_internal(
    bytes: &[u8],
    instrument: Instrument,
    delay_ms: f64,
    allow_absent_generated_part: bool,
) -> Result<Parsed> {
    ensure!(
        bytes.len() <= 64 * 1024 * 1024 && delay_ms.is_finite() && delay_ms.abs() <= 60000.,
        "Invalid benchmark MIDI size or delay"
    );
    if !bytes.starts_with(b"MThd") {
        ensure!(
            !matches!(instrument, Instrument::Vocals),
            "Lead-vocal reference comparisons require MIDI"
        );
        let chart = crate::chart::Chart::parse(std::str::from_utf8(bytes)?.to_owned())?;
        let mut parsed = Parsed {
            notes: vec![],
            excluded_note_onsets: 0,
            phrase_markers: 0,
            pitch_bend_events: 0,
            excluded_unphrased_vocal_notes: 0,
            excluded_expert_plus_kicks: is_drum(instrument).then_some(0),
        };
        let label = match instrument {
            Instrument::Bass => "ExpertDoubleBass",
            Instrument::Guitar => "ExpertSingle",
            Instrument::Drums | Instrument::ProDrums => "ExpertDrums",
            Instrument::Vocals => unreachable!(),
        };
        let notes = chart.notes(label)?;
        let pro = matches!(instrument, Instrument::ProDrums);
        ensure!(
            !pro || !notes.iter().any(|n| n.lane == 5),
            "ProDrums benchmark supports native four-lane charts; five-lane conversion is not inferred"
        );
        let cymbals: BTreeSet<_> = notes
            .iter()
            .filter(|n| (66..=68).contains(&n.lane))
            .map(|n| (n.tick, n.lane - 64))
            .collect();
        let mut disco = BTreeMap::new();
        if pro && let Some(section) = chart.sections.iter().find(|s| s.name == label) {
            for line in chart.raw[section.body_start..section.body_end].lines() {
                if let Some((tick, event)) = line.split_once('=')
                    && let Some(text) = event.trim().strip_prefix("E ")
                    && let Some(flip) = disco_event(text.trim().trim_matches('"'))
                {
                    disco.insert(tick.trim().parse::<u64>()?, flip);
                }
            }
        }
        for note in notes {
            let playable = if is_drum(instrument) {
                note.lane <= 5
            } else {
                note.lane <= 4 || note.lane == 7
            };
            if playable {
                let flip = disco
                    .range(..=note.tick)
                    .next_back()
                    .is_some_and(|(_, f)| *f);
                let (lane, cymbal) = pro_pad(
                    note.lane,
                    cymbals.contains(&(note.tick, note.lane)),
                    pro && flip,
                );
                parsed.notes.push(Note {
                    start: chart.timing.seconds(note.tick) + delay_ms / 1000.,
                    end: chart.timing.seconds(
                        note.tick
                            .checked_add(note.sustain)
                            .context("Benchmark sustain overflow")?,
                    ) + delay_ms / 1000.,
                    pitch: None,
                    lane: (!matches!(instrument, Instrument::Bass)).then_some(lane),
                    cymbal: pro.then_some(cymbal),
                });
            } else {
                parsed.excluded_note_onsets += 1;
                if note.lane == 32
                    && let Some(count) = &mut parsed.excluded_expert_plus_kicks
                {
                    *count += 1;
                }
            }
        }
        ensure!(
            parsed.notes.len() <= 20000,
            "Benchmark exceeds playable-note budget"
        );
        return Ok(parsed);
    }
    let parsed = Smf::parse(bytes)?;
    let mut timing = midi::timing(bytes)?;
    timing.offset = delay_ms / 1000.;
    let label = match instrument {
        Instrument::Bass => b"PART BASS".as_slice(),
        Instrument::Vocals => b"PART VOCALS".as_slice(),
        Instrument::Guitar => b"PART GUITAR".as_slice(),
        Instrument::Drums | Instrument::ProDrums => b"PART DRUMS".as_slice(),
    };
    let tracks: Vec<_> = parsed.tracks.iter().filter(|track| track.iter().any(|e| matches!(e.kind, TrackEventKind::Meta(MetaMessage::TrackName(name)) if name == label))).collect();
    if tracks.is_empty() && allow_absent_generated_part {
        validate_absent_part_midi(bytes, &parsed)?;
        return Ok(Parsed {
            notes: vec![],
            excluded_note_onsets: 0,
            phrase_markers: 0,
            pitch_bend_events: 0,
            excluded_unphrased_vocal_notes: 0,
            excluded_expert_plus_kicks: is_drum(instrument).then_some(0),
        });
    }
    ensure!(
        tracks.len() == 1,
        "Benchmark requires exactly one {} track",
        String::from_utf8_lossy(label)
    );
    let mut tick = 0u64;
    let mut active = BTreeMap::new();
    let mut active_phrases = BTreeMap::new();
    let mut phrases = Vec::new();
    let mut lyrics = BTreeMap::new();
    let mut notes = Vec::new();
    let mut enhanced_opens = false;
    let mut active_toms = BTreeMap::new();
    let mut toms = Vec::new();
    let mut disco = BTreeMap::new();
    let mut excluded_expert_plus_kicks = is_drum(instrument).then_some(0);
    let (mut excluded_note_onsets, mut phrase_markers, mut pitch_bend_events) = (0, 0, 0);
    for event in tracks[0] {
        tick = tick
            .checked_add(event.delta.as_int() as u64)
            .context("Benchmark tick overflow")?;
        match event.kind {
            TrackEventKind::Meta(MetaMessage::Text(b"[ENHANCED_OPENS]" | b"ENHANCED_OPENS")) => {
                enhanced_opens = true;
            }
            TrackEventKind::Meta(MetaMessage::Text(text))
                if matches!(instrument, Instrument::ProDrums) =>
            {
                if let Some(flip) = disco_event(std::str::from_utf8(text)?) {
                    disco.insert(tick, flip);
                }
            }
            TrackEventKind::Meta(MetaMessage::Lyric(text)) => {
                lyrics.insert(tick, text);
            }
            TrackEventKind::Meta(MetaMessage::Text(text))
                if matches!(instrument, Instrument::Vocals) && !text.starts_with(b"[") =>
            {
                lyrics.insert(tick, text);
            }
            TrackEventKind::Midi { channel, message } => match message {
                MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                    let key = key.as_int();
                    let playable = match instrument {
                        Instrument::Bass | Instrument::Guitar => {
                            (96..=100).contains(&key) || (enhanced_opens && key == 95)
                        }
                        Instrument::Vocals => (36..=84).contains(&key),
                        Instrument::Drums => (96..=101).contains(&key),
                        Instrument::ProDrums => {
                            ensure!(
                                key != 101,
                                "ProDrums benchmark supports native four-lane MIDI; five-lane conversion is not inferred"
                            );
                            (96..=100).contains(&key)
                        }
                    };
                    if playable {
                        ensure!(
                            active.insert((channel.as_int(), key), tick).is_none(),
                            "Overlapping same-key benchmark notes"
                        );
                    } else if matches!(instrument, Instrument::Vocals) && matches!(key, 105 | 106) {
                        phrase_markers += 1;
                        ensure!(
                            active_phrases
                                .insert((channel.as_int(), key), tick)
                                .is_none(),
                            "Overlapping same-key vocal phrases"
                        );
                    } else {
                        excluded_note_onsets += 1;
                        if key == 95
                            && let Some(count) = &mut excluded_expert_plus_kicks
                        {
                            *count += 1;
                        }
                        if matches!(instrument, Instrument::ProDrums) && (110..=112).contains(&key)
                        {
                            ensure!(
                                active_toms.insert((channel.as_int(), key), tick).is_none(),
                                "Overlapping same-key tom marker"
                            );
                        }
                    }
                }
                MidiMessage::NoteOff { key, .. } | MidiMessage::NoteOn { key, .. } => {
                    if let Some(start) = active_toms.remove(&(channel.as_int(), key.as_int())) {
                        toms.push((start, tick, key.as_int() - 108));
                    }
                    if let Some(start) = active_phrases.remove(&(channel.as_int(), key.as_int())) {
                        ensure!(start < tick, "Zero-length vocal phrase");
                        phrases.push((start, tick));
                    }
                    if let Some(start) = active.remove(&(channel.as_int(), key.as_int())) {
                        ensure!(start < tick, "Zero-length benchmark note");
                        notes.push((start, tick, key.as_int()));
                    }
                }
                MidiMessage::PitchBend { .. } => pitch_bend_events += 1,
                _ => {}
            },
            _ => {}
        }
    }
    ensure!(active.is_empty(), "Unterminated playable benchmark notes");
    ensure!(active_toms.is_empty(), "Unterminated ProDrums tom markers");
    ensure!(
        active_phrases.is_empty(),
        "Unterminated vocal benchmark phrases"
    );
    ensure!(
        notes.len() <= 20000,
        "Benchmark exceeds playable-note budget"
    );
    notes.sort_unstable();
    let mut excluded_unphrased_vocal_notes = 0;
    let mut lyric_events = lyrics.into_iter().peekable();
    let notes = notes
        .into_iter()
        .filter_map(|(start, end, pitch)| {
            if matches!(instrument, Instrument::Vocals)
                && !phrases.iter().any(|(lo, hi)| *lo <= start && start < *hi)
            {
                excluded_unphrased_vocal_notes += 1;
                return None;
            }
            let mut lyric = None;
            while lyric_events.peek().is_some_and(|(tick, _)| *tick <= start) {
                lyric = lyric_events.next().map(|(_, text)| text);
            }
            Some(Note {
                start: timing.seconds(start),
                end: timing.seconds(end),
                pitch: (matches!(instrument, Instrument::Vocals)
                    && !lyric.is_some_and(unpitched_lyric))
                .then_some(pitch),
                lane: match instrument {
                    Instrument::Guitar => Some(if pitch == 95 { 7 } else { pitch - 96 }),
                    Instrument::Drums => Some(pitch - 96),
                    Instrument::ProDrums => {
                        let lane = pitch - 96;
                        let cymbal = lane >= 2
                            && toms
                                .iter()
                                .filter(|(lo, hi, pad)| {
                                    *pad == lane
                                        && *lo <= start
                                        && (start < *hi || (*lo == *hi && start == *lo))
                                })
                                .count()
                                % 2
                                == 0;
                        let flip = disco
                            .range(..=start)
                            .next_back()
                            .is_some_and(|(_, flip)| *flip);
                        Some(pro_pad(lane, cymbal, flip).0)
                    }
                    _ => None,
                },
                cymbal: matches!(instrument, Instrument::ProDrums).then(|| {
                    let lane = pitch - 96;
                    let cymbal = lane >= 2
                        && toms
                            .iter()
                            .filter(|(lo, hi, pad)| {
                                *pad == lane
                                    && *lo <= start
                                    && (start < *hi || (*lo == *hi && start == *lo))
                            })
                            .count()
                            % 2
                            == 0;
                    let flip = disco
                        .range(..=start)
                        .next_back()
                        .is_some_and(|(_, flip)| *flip);
                    pro_pad(lane, cymbal, flip).1
                }),
            })
        })
        .collect();
    Ok(Parsed {
        notes,
        excluded_note_onsets,
        phrase_markers,
        pitch_bend_events,
        excluded_unphrased_vocal_notes,
        excluded_expert_plus_kicks,
    })
}

#[derive(Clone, Copy, Default)]
struct Score {
    count: u32,
    error: f64,
}
impl Score {
    fn better(self, other: Self) -> bool {
        self.count > other.count || (self.count == other.count && self.error < other.error)
    }
}

/// Maximum-cardinality chronological one-to-one matching, with minimum summed
/// absolute error as a tie-break. A dense output cannot claim several hits for
/// a single reference attack, and a nearest-neighbor tie cannot lose a match.
pub fn match_attacks(
    reference: &[f64],
    generated: &[f64],
    tolerance: f64,
) -> Result<Vec<(usize, usize)>> {
    ensure!(
        tolerance.is_finite() && tolerance >= 0.,
        "Invalid onset tolerance"
    );
    for notes in [reference, generated] {
        ensure!(
            notes.iter().all(|t| t.is_finite()) && notes.windows(2).all(|w| w[0] < w[1]),
            "Attack positions must be finite, sorted and unique"
        );
    }
    let width = generated
        .len()
        .checked_add(1)
        .context("Benchmark size overflow")?;
    let cells = reference
        .len()
        .checked_add(1)
        .and_then(|n| n.checked_mul(width))
        .context("Benchmark size overflow")?;
    ensure!(
        cells <= 25_000_000,
        "Benchmark exceeds alignment budget; compare shorter source-bound sections"
    );
    let mut decisions = vec![0u8; cells];
    let mut previous = vec![Score::default(); width];
    for (i, a) in reference.iter().enumerate() {
        let mut row = vec![Score::default(); width];
        for (j, b) in generated.iter().enumerate() {
            let mut best = previous[j + 1];
            let mut decision = 1;
            if row[j].better(best) {
                best = row[j];
                decision = 2;
            }
            let residual = (a - b).abs();
            if residual <= tolerance {
                let candidate = Score {
                    count: previous[j].count + 1,
                    error: previous[j].error + residual,
                };
                if candidate.better(best) {
                    best = candidate;
                    decision = 3;
                }
            }
            row[j + 1] = best;
            decisions[(i + 1) * width + j + 1] = decision;
        }
        previous = row;
    }
    let (mut i, mut j) = (reference.len(), generated.len());
    let mut result = Vec::new();
    while i > 0 && j > 0 {
        match decisions[i * width + j] {
            1 => i -= 1,
            2 => j -= 1,
            3 => {
                result.push((i - 1, j - 1));
                i -= 1;
                j -= 1;
            }
            _ => anyhow::bail!("Invalid benchmark alignment"),
        }
    }
    result.reverse();
    Ok(result)
}

fn attacks(parsed: &Parsed) -> Vec<f64> {
    let mut times: Vec<_> = parsed.notes.iter().map(|note| note.start).collect();
    times.sort_by(f64::total_cmp);
    times.dedup();
    times
}
fn fraction(a: f64, b: f64) -> Option<f64> {
    (b > 0.).then_some(a / b)
}

#[derive(Clone, Debug, Serialize)]
pub struct AttackMetrics {
    pub tolerance_ms: u32,
    pub matched: usize,
    pub reference_attacks: usize,
    pub generated_attacks: usize,
    pub precision: Option<f64>,
    pub recall: Option<f64>,
    pub median_absolute_error_ms: Option<f64>,
    pub p90_absolute_error_ms: Option<f64>,
    pub median_signed_error_ms: Option<f64>,
    pub mean_signed_error_ms: Option<f64>,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Region {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub unmatched_reference_attacks: usize,
    pub unmatched_generated_attacks: usize,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct PitchMetrics {
    pub reference_pitched_seconds: f64,
    pub generated_pitched_seconds: f64,
    pub overlapping_pitched_seconds: f64,
    pub unambiguous_compared_seconds: f64,
    pub same_pitch_seconds: f64,
    pub same_pitch_class_seconds: f64,
    pub reference_coverage: Option<f64>,
    pub exact_pitch_agreement: Option<f64>,
    pub octave_folded_pitch_agreement: Option<f64>,
    pub generated_time_overlap: Option<f64>,
    pub exact_pitch_precision: Option<f64>,
    pub exact_pitch_recall: Option<f64>,
    pub octave_folded_pitch_precision: Option<f64>,
    pub octave_folded_pitch_recall: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WindowPattern {
    pub attacks: usize,
    pub raw_gems: usize,
    pub attacks_per_second: f64,
    pub gems_per_attack_histogram: BTreeMap<usize, usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sustained_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kick_attacks: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hand_attacks: Option<usize>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SectionCoverage {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub reference: WindowPattern,
    pub generated: WindowPattern,
    // Cross-boundary timing matches belong to each side's own onset window.
    pub matched_reference_attacks: usize,
    pub matched_generated_attacks: usize,
    pub precision_at_50ms: Option<f64>,
    pub recall_at_50ms: Option<f64>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SustainMetrics {
    pub minimum_sustain_seconds: f64,
    pub reference_sustained_seconds: f64,
    pub generated_sustained_seconds: f64,
    pub overlapping_seconds: f64,
    pub generated_time_precision: Option<f64>,
    pub reference_time_recall: Option<f64>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ClassMetrics {
    pub class: String,
    pub attacks: Vec<AttackMetrics>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ProHitMetrics {
    pub attacks: Vec<AttackMetrics>,
    pub lane_articulation_classes: Vec<ClassMetrics>,
}

fn measured(a: &[f64], b: &[f64], matched: &[(usize, usize)], tolerance_ms: u32) -> AttackMetrics {
    let mut errors: Vec<_> = matched
        .iter()
        .map(|(i, j)| (b[*j] - a[*i]).abs() * 1000.)
        .collect();
    errors.sort_by(f64::total_cmp);
    let mut signed: Vec<_> = matched
        .iter()
        .map(|(i, j)| (b[*j] - a[*i]) * 1000.)
        .collect();
    signed.sort_by(f64::total_cmp);
    let quantile = |q: f64| {
        (!errors.is_empty()).then(|| errors[((errors.len() - 1) as f64 * q).round() as usize])
    };
    AttackMetrics {
        tolerance_ms,
        matched: matched.len(),
        reference_attacks: a.len(),
        generated_attacks: b.len(),
        precision: fraction(matched.len() as f64, b.len() as f64),
        recall: fraction(matched.len() as f64, a.len() as f64),
        median_absolute_error_ms: quantile(0.5),
        p90_absolute_error_ms: quantile(0.9),
        median_signed_error_ms: (!signed.is_empty()).then(|| signed[(signed.len() - 1) / 2]),
        mean_signed_error_ms: fraction(signed.iter().sum(), signed.len() as f64),
    }
}
fn selected_times(track: &Parsed, select: impl Fn(&Note) -> bool) -> Vec<f64> {
    let mut times: Vec<_> = track
        .notes
        .iter()
        .filter(|n| select(n))
        .map(|n| n.start)
        .collect();
    times.sort_by(f64::total_cmp);
    times.dedup();
    times
}
fn class_metrics(class: &str, a: &[f64], b: &[f64]) -> Result<ClassMetrics> {
    let mut metrics = vec![];
    for tolerance in [25, 50, 100] {
        let pairs = match_attacks(a, b, tolerance as f64 / 1000.)?;
        metrics.push(measured(a, b, &pairs, tolerance));
    }
    Ok(ClassMetrics {
        class: class.into(),
        attacks: metrics,
    })
}
fn drum_roles(a: &Parsed, b: &Parsed) -> Result<Vec<ClassMetrics>> {
    [("kick", true), ("hands", false)]
        .into_iter()
        .map(|(name, kick)| {
            class_metrics(
                name,
                &selected_times(a, |n| (n.lane == Some(0)) == kick),
                &selected_times(b, |n| (n.lane == Some(0)) == kick),
            )
        })
        .collect()
}
fn pro_hits(a: &Parsed, b: &Parsed) -> Result<ProHitMetrics> {
    let classes = [
        ("kick", 0, false),
        ("red_drum", 1, false),
        ("yellow_tom", 2, false),
        ("yellow_cymbal", 2, true),
        ("blue_tom", 3, false),
        ("blue_cymbal", 3, true),
        ("green_tom", 4, false),
        ("green_cymbal", 4, true),
    ];
    let mut per_class = vec![];
    let mut all_a = vec![];
    let mut all_b = vec![];
    let mut all_pairs: [Vec<(usize, usize)>; 3] = std::array::from_fn(|_| vec![]);
    for (name, lane, cymbal) in classes {
        let aa = selected_times(a, |n| n.lane == Some(lane) && n.cymbal == Some(cymbal));
        let bb = selected_times(b, |n| n.lane == Some(lane) && n.cymbal == Some(cymbal));
        for (index, tolerance) in [25, 50, 100].into_iter().enumerate() {
            all_pairs[index].extend(
                match_attacks(&aa, &bb, tolerance as f64 / 1000.)?
                    .into_iter()
                    .map(|(i, j)| (i + all_a.len(), j + all_b.len())),
            );
        }
        per_class.push(class_metrics(name, &aa, &bb)?);
        all_a.extend(aa);
        all_b.extend(bb);
    }
    let attacks = all_pairs
        .iter()
        .zip([25, 50, 100])
        .map(|(pairs, tolerance)| measured(&all_a, &all_b, pairs, tolerance))
        .collect();
    Ok(ProHitMetrics {
        attacks,
        lane_articulation_classes: per_class,
    })
}
fn sustained_intervals(track: &Parsed, lo: f64, hi: f64) -> Vec<(f64, f64)> {
    let mut raw: Vec<_> = track
        .notes
        .iter()
        .filter(|n| n.end - n.start >= 0.1)
        .filter_map(|n| {
            let (a, b) = (n.start.max(lo), n.end.min(hi));
            (a < b).then_some((a, b))
        })
        .collect();
    raw.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut union: Vec<(f64, f64)> = vec![];
    for (a, b) in raw {
        if let Some(last) = union.last_mut()
            && a <= last.1
        {
            last.1 = last.1.max(b);
        } else {
            union.push((a, b));
        }
    }
    union
}
fn sustain_metrics(a: &Parsed, b: &Parsed) -> SustainMetrics {
    let aa = sustained_intervals(a, f64::NEG_INFINITY, f64::INFINITY);
    let bb = sustained_intervals(b, f64::NEG_INFINITY, f64::INFINITY);
    let total = |v: &[(f64, f64)]| v.iter().map(|(a, b)| b - a).sum::<f64>();
    let (mut i, mut j, mut overlap) = (0, 0, 0.);
    while i < aa.len() && j < bb.len() {
        overlap += (aa[i].1.min(bb[j].1) - aa[i].0.max(bb[j].0)).max(0.);
        if aa[i].1 <= bb[j].1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    let (reference, generated) = (total(&aa), total(&bb));
    SustainMetrics {
        minimum_sustain_seconds: 0.1,
        reference_sustained_seconds: reference,
        generated_sustained_seconds: generated,
        overlapping_seconds: overlap,
        generated_time_precision: fraction(overlap, generated),
        reference_time_recall: fraction(overlap, reference),
    }
}
fn window_pattern(track: &Parsed, lo: f64, hi: f64, instrument: Instrument) -> WindowPattern {
    let mut groups = BTreeMap::<u64, usize>::new();
    let mut raw_gems = 0;
    for note in track.notes.iter().filter(|n| lo <= n.start && n.start < hi) {
        *groups.entry(note.start.to_bits()).or_default() += 1;
        raw_gems += 1;
    }
    let mut histogram = BTreeMap::new();
    for count in groups.values() {
        *histogram.entry(*count).or_default() += 1;
    }
    let in_window = |n: &Note| lo <= n.start && n.start < hi;
    WindowPattern {
        attacks: groups.len(),
        raw_gems,
        attacks_per_second: groups.len() as f64 / (hi - lo),
        gems_per_attack_histogram: histogram,
        sustained_seconds: matches!(instrument, Instrument::Guitar).then(|| {
            sustained_intervals(track, lo, hi)
                .iter()
                .map(|(a, b)| b - a)
                .sum()
        }),
        kick_attacks: is_drum(instrument)
            .then(|| selected_times(track, |n| in_window(n) && n.lane == Some(0)).len()),
        hand_attacks: is_drum(instrument)
            .then(|| selected_times(track, |n| in_window(n) && n.lane != Some(0)).len()),
    }
}
fn sections(
    a: &Parsed,
    b: &Parsed,
    matched: &[(usize, usize)],
    instrument: Instrument,
) -> Result<Vec<SectionCoverage>> {
    let mut bins = BTreeSet::new();
    for n in a.notes.iter().chain(&b.notes) {
        ensure!(
            n.start.is_finite()
                && n.end.is_finite()
                && n.start.abs() <= 86400.
                && n.end.abs() <= 86400.,
            "Full-band benchmark exceeds one-day clock budget"
        );
        bins.insert((n.start / 12.).floor() as i64);
        if matches!(instrument, Instrument::Guitar) && n.end - n.start >= 0.1 {
            let last = (n.end / 12.).ceil() as i64;
            bins.extend((n.start / 12.).floor() as i64..last);
        }
    }
    let aa = attacks(a);
    let bb = attacks(b);
    let mut output = vec![];
    for bin in bins {
        let (lo, hi) = (bin as f64 * 12., (bin + 1) as f64 * 12.);
        let reference = window_pattern(a, lo, hi, instrument);
        let generated = window_pattern(b, lo, hi, instrument);
        let ma = matched
            .iter()
            .filter(|(i, _)| lo <= aa[*i] && aa[*i] < hi)
            .count();
        let mb = matched
            .iter()
            .filter(|(_, j)| lo <= bb[*j] && bb[*j] < hi)
            .count();
        let precision = fraction(mb as f64, generated.attacks as f64);
        let recall = fraction(ma as f64, reference.attacks as f64);
        output.push(SectionCoverage {
            start_seconds: lo,
            end_seconds: hi,
            reference,
            generated,
            matched_reference_attacks: ma,
            matched_generated_attacks: mb,
            precision_at_50ms: precision,
            recall_at_50ms: recall,
        });
    }
    Ok(output)
}

fn pitch_metrics(reference: &Parsed, generated: &Parsed) -> PitchMetrics {
    // Sweep intervals instead of sampling at a fixed frame rate; chords and
    // overlapping notes do not double-count coverage. Ambiguous lead pitches
    // are excluded from pitch agreement while retained in coverage totals.
    let mut events = Vec::new();
    for (side, track) in [(0usize, reference), (1usize, generated)] {
        for note in &track.notes {
            if let Some(pitch) = note.pitch {
                events.push((note.start, side, pitch, 1i32));
                events.push((note.end, side, pitch, -1i32));
            }
        }
    }
    events.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut result = PitchMetrics::default();
    let mut active: [BTreeMap<u8, usize>; 2] = [BTreeMap::new(), BTreeMap::new()];
    let mut previous = events.first().map_or(0., |e| e.0);
    let mut cursor = 0;
    while cursor < events.len() {
        let time = events[cursor].0;
        let duration = time - previous;
        if !active[0].is_empty() {
            result.reference_pitched_seconds += duration;
        }
        if !active[1].is_empty() {
            result.generated_pitched_seconds += duration;
        }
        if !active[0].is_empty() && !active[1].is_empty() {
            result.overlapping_pitched_seconds += duration;
        }
        if active[0].len() == 1 && active[1].len() == 1 {
            result.unambiguous_compared_seconds += duration;
            let a = *active[0].first_key_value().unwrap().0;
            let b = *active[1].first_key_value().unwrap().0;
            if a == b {
                result.same_pitch_seconds += duration;
            }
            if a % 12 == b % 12 {
                result.same_pitch_class_seconds += duration;
            }
        }
        while cursor < events.len() && events[cursor].0 == time {
            let (_, side, pitch, delta) = events[cursor];
            if delta > 0 {
                *active[side].entry(pitch).or_default() += 1;
            } else if let Some(count) = active[side].get_mut(&pitch) {
                *count -= 1;
                if *count == 0 {
                    active[side].remove(&pitch);
                }
            }
            cursor += 1;
        }
        previous = time;
    }
    result.reference_coverage = fraction(
        result.overlapping_pitched_seconds,
        result.reference_pitched_seconds,
    );
    result.generated_time_overlap = fraction(
        result.overlapping_pitched_seconds,
        result.generated_pitched_seconds,
    );
    result.exact_pitch_precision =
        fraction(result.same_pitch_seconds, result.generated_pitched_seconds);
    result.exact_pitch_recall =
        fraction(result.same_pitch_seconds, result.reference_pitched_seconds);
    result.octave_folded_pitch_precision = fraction(
        result.same_pitch_class_seconds,
        result.generated_pitched_seconds,
    );
    result.octave_folded_pitch_recall = fraction(
        result.same_pitch_class_seconds,
        result.reference_pitched_seconds,
    );
    result.exact_pitch_agreement = fraction(
        result.same_pitch_seconds,
        result.unambiguous_compared_seconds,
    );
    result.octave_folded_pitch_agreement = fraction(
        result.same_pitch_class_seconds,
        result.unambiguous_compared_seconds,
    );
    result
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub version: u32,
    pub instrument: Instrument,
    pub recording_sha256: String,
    pub reference_notes_sha256: String,
    pub generated_notes_sha256: String,
    pub reference_delay_ms: f64,
    pub generated_delay_ms: f64,
    pub reference_gems: usize,
    pub generated_gems: usize,
    pub attacks: Vec<AttackMetrics>,
    pub differences_at_50ms: Vec<Region>,
    pub vocal_pitch: Option<PitchMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expert_sections: Option<Vec<SectionCoverage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guitar_sustain: Option<SustainMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drum_roles: Option<Vec<ClassMetrics>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pro_drum_hits: Option<ProHitMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded_expert_plus_kicks: Option<(usize, usize)>,
    pub limitations: Vec<String>,
}

pub struct Request<'a> {
    pub reference_midi: &'a Path,
    pub generated_midi: &'a Path,
    pub reference_audio: &'a Path,
    pub generated_audio: &'a Path,
    pub reference_delay_ms: f64,
    pub generated_delay_ms: f64,
    pub instrument: Instrument,
}
pub fn compare(request: Request<'_>) -> Result<Report> {
    let audio_hash = models::hash_file(request.reference_audio)?;
    ensure!(
        audio_hash == models::hash_file(request.generated_audio)?,
        "Benchmark recordings differ; do not compare different recordings or silently estimate an offset"
    );
    let load = |path: &Path| -> Result<Vec<u8>> {
        ensure!(
            fs::metadata(path)?.len() <= 64 * 1024 * 1024,
            "Benchmark MIDI exceeds 64 MiB"
        );
        Ok(fs::read(path)?)
    };
    let reference_bytes = load(request.reference_midi)?;
    let generated_bytes = load(request.generated_midi)?;
    let reference = parse(
        &reference_bytes,
        request.instrument,
        request.reference_delay_ms,
    )?;
    let generated = parse_internal(
        &generated_bytes,
        request.instrument,
        request.generated_delay_ms,
        true,
    )?;
    let a = attacks(&reference);
    let b = attacks(&generated);
    ensure!(!a.is_empty(), "Reference has no playable Expert notes");
    let mut metrics = Vec::new();
    let mut regions: BTreeMap<i64, Region> = BTreeMap::new();
    let mut matched_at_50ms = vec![];
    for tolerance_ms in [25, 50, 100] {
        let matched = match_attacks(&a, &b, tolerance_ms as f64 / 1000.)?;
        let mut errors: Vec<_> = matched
            .iter()
            .map(|(i, j)| (b[*j] - a[*i]).abs() * 1000.)
            .collect();
        errors.sort_by(f64::total_cmp);
        let mut signed_errors: Vec<_> = matched
            .iter()
            .map(|(i, j)| (b[*j] - a[*i]) * 1000.)
            .collect();
        signed_errors.sort_by(f64::total_cmp);
        let quantile = |q: f64| -> Option<f64> {
            (!errors.is_empty()).then(|| errors[((errors.len() - 1) as f64 * q).round() as usize])
        };
        metrics.push(AttackMetrics {
            tolerance_ms,
            matched: matched.len(),
            reference_attacks: a.len(),
            generated_attacks: b.len(),
            precision: fraction(matched.len() as f64, b.len() as f64),
            recall: fraction(matched.len() as f64, a.len() as f64),
            median_absolute_error_ms: quantile(0.5),
            p90_absolute_error_ms: quantile(0.9),
            median_signed_error_ms: (!signed_errors.is_empty())
                .then(|| signed_errors[(signed_errors.len() - 1) / 2]),
            mean_signed_error_ms: fraction(signed_errors.iter().sum(), signed_errors.len() as f64),
        });
        if tolerance_ms == 50 {
            matched_at_50ms = matched.clone();
            let used_a: BTreeSet<_> = matched.iter().map(|p| p.0).collect();
            let used_b: BTreeSet<_> = matched.iter().map(|p| p.1).collect();
            for (times, used, is_reference) in [(&a, &used_a, true), (&b, &used_b, false)] {
                for (index, time) in times
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| !used.contains(index))
                {
                    let _ = index;
                    ensure!(
                        time.abs() <= 86400.,
                        "Benchmark timing exceeds one-day budget"
                    );
                    let bin = (time / 12.).floor() as i64;
                    let region = regions.entry(bin).or_insert(Region {
                        start_seconds: bin as f64 * 12.,
                        end_seconds: (bin + 1) as f64 * 12.,
                        ..Default::default()
                    });
                    if is_reference {
                        region.unmatched_reference_attacks += 1;
                    } else {
                        region.unmatched_generated_attacks += 1;
                    }
                }
            }
        }
    }
    ensure!(
        models::hash_file(request.reference_audio)? == audio_hash
            && models::hash_file(request.generated_audio)? == audio_hash,
        "Benchmark audio changed while comparing"
    );
    Ok(Report {
        version: 1, instrument: request.instrument, recording_sha256: audio_hash,
        reference_notes_sha256: format!("{:x}", sha2::Sha256::digest(&reference_bytes)),
        generated_notes_sha256: format!("{:x}", sha2::Sha256::digest(&generated_bytes)),
        reference_delay_ms: request.reference_delay_ms, generated_delay_ms: request.generated_delay_ms,
        reference_gems: reference.notes.len(), generated_gems: generated.notes.len(), attacks: metrics,
        differences_at_50ms: regions.into_values().collect(),
        vocal_pitch: matches!(request.instrument, Instrument::Vocals).then(|| pitch_metrics(&reference, &generated)),
        expert_sections: matches!(request.instrument, Instrument::Guitar|Instrument::Drums|Instrument::ProDrums).then(|| sections(&reference,&generated,&matched_at_50ms,request.instrument)).transpose()?,
        guitar_sustain: matches!(request.instrument,Instrument::Guitar).then(||sustain_metrics(&reference,&generated)),
        drum_roles: is_drum(request.instrument).then(||drum_roles(&reference,&generated)).transpose()?,
        pro_drum_hits: matches!(request.instrument,Instrument::ProDrums).then(||pro_hits(&reference,&generated)).transpose()?,
        excluded_expert_plus_kicks: is_drum(request.instrument).then(|| (reference.excluded_expert_plus_kicks.unwrap_or(0),generated.excluded_expert_plus_kicks.unwrap_or(0))),
        limitations: vec![
            "Recording identity compares the supplied audio bytes; retain archive/extraction provenance binding the authored chart to that audio.".into(),
            "Reference differences are diagnostics, not a claim that every human choice is the only valid charting choice. Bass fret colors are not acoustic pitch truth.".into(),
            "Pitch agreement is conditional on overlapping unambiguous pitched time; precision/recall also report full generated/reference denominators so added non-overlapping tones cannot inflate that agreement. Vocal octave folding is reported separately because game scoring allows octave equivalents.".into(),
            format!("Expert-only comparison. Other-difficulty and marker note onsets excluded: reference {}, generated {}. Vocal pitch bends are not modeled: reference {} events, generated {}.", reference.excluded_note_onsets, generated.excluded_note_onsets, reference.pitch_bend_events, generated.pitch_bend_events),
            format!("Vocal notes starting outside game phrase markers are excluded: reference {}, generated {}. Lyric flags consume preceding lyric events at each note, matching the supported YARG lead-vocal dialect; vocal slides are compared as exported pitch intervals.", reference.excluded_unphrased_vocal_notes, generated.excluded_unphrased_vocal_notes),
        ].into_iter().chain(matches!(request.instrument,Instrument::Guitar|Instrument::Drums|Instrument::ProDrums).then(|| "Expert only; no lower difficulties assessed. Guitar lane colors are descriptors, not acoustic pitch truth or exact-color error scores. Fixed12-second section density and gem-count histograms describe arrangements; matched onsets across a boundary are credited in each side's own window. Guitar sustain coverage unions notes at least100ms long, not chord-gem duration sums; this measures written chart holds, not audio sustain accuracy. Standard Drums groups raw four/five-lane onsets and separates kick/hands; no lane conversion or articulation accuracy is inferred. ProDrums is native four-lane only: lane+tom/cymbal hit matching, MIDI cymbal-default/tom-marker XOR spans and Expert disco flip; chart cymbals marked at exact gem ticks. No note snapping, accent/ghost/dynamics, elite/five-lane conversion or physical kit truth. ExpertPlus kicks are separately counted and excluded (MIDI95/chart32). Simultaneous same-pad/articulation duplicates are one hit in the classified ProDrums totals.".into())).collect(),
    })
}

use sha2::Digest;
