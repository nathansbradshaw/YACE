//! Per-difficulty drum mix transitions, shared by .chart and MIDI local text.
//! Reference: GuitarGame_ChartFormats, mid-format/Tracks/Drums.md#mix-event-details.
//! Disco authoring edits bounded d/off pairs and never invents stem routing.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[path = "drum_mix_midi.rs"]
mod midi;
pub use midi::{State, hash, import_midi, rewrite_midi};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Setting {
    Clear,
    Disco,
    DiscoNoFlip,
    Easy,
    EasyNoKick,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mix {
    pub difficulty: u8,
    pub configuration: u8,
    pub setting: Setting,
}

fn separators(text: &str) -> &str {
    text.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '_')
}
/// Ordinary local text is not managed. Ambiguous mix-prefixed text refuses
/// authoring instead of silently treating an unknown routing flag as a reset.
pub fn parse(text: &str) -> Result<Option<Mix>> {
    let mut text = text.trim();
    if text.starts_with('[') {
        let Some(inner) = text.strip_suffix(']') else {
            ensure!(
                !text.starts_with("[mix"),
                "Unclosed drum mix event; retain original"
            );
            return Ok(None);
        };
        text = inner[1..].trim();
    }
    let Some(rest) = text.strip_prefix("mix") else {
        return Ok(None);
    };
    let rest = separators(rest);
    if rest.starts_with(|c: char| c.is_ascii_alphabetic())
        && !text.starts_with("mix ")
        && !text.starts_with("mix_")
    {
        return Ok(None);
    }
    let difficulty = rest
        .as_bytes()
        .first()
        .copied()
        .filter(|b| (b'0'..=b'3').contains(b))
        .context("Unsupported drum mix difficulty")?
        - b'0';
    let rest = separators(&rest[1..])
        .strip_prefix("drums")
        .context("Malformed drum mix event")?;
    let rest = separators(rest);
    let configuration = rest
        .as_bytes()
        .first()
        .copied()
        .filter(|b| (b'0'..=b'5').contains(b))
        .context("Unsupported drum stem configuration")?
        - b'0';
    let setting = match separators(&rest[1..]).trim_end() {
        "" => Setting::Clear,
        "d" => Setting::Disco,
        "dnoflip" => Setting::DiscoNoFlip,
        "easy" => Setting::Easy,
        "easynokick" => Setting::EasyNoKick,
        _ => anyhow::bail!("Unsupported drum mix flag; retain original"),
    };
    Ok(Some(Mix {
        difficulty,
        configuration,
        setting,
    }))
}
impl Mix {
    pub fn text(self) -> String {
        let flag = match self.setting {
            Setting::Clear => "",
            Setting::Disco => "d",
            Setting::DiscoNoFlip => "dnoflip",
            Setting::Easy => "easy",
            Setting::EasyNoKick => "easynokick",
        };
        format!(
            "[mix {} drums{}{flag}]",
            self.difficulty, self.configuration
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub tick: u64,
    pub mix: Mix,
    pub text: String,
}
impl Event {
    pub fn new(tick: u64, mix: Mix) -> Self {
        Self {
            tick,
            mix,
            text: mix.text(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Range {
    pub on: Event,
    pub off: Event,
}
impl Range {
    pub fn start(&self) -> u64 {
        self.on.tick
    }
    pub fn end(&self) -> u64 {
        self.off.tick
    }
}
pub fn validate(events: &[Event]) -> Result<()> {
    ensure!(events.len() <= 8192, "Too many drum mix transitions");
    for e in events {
        ensure!(
            e.tick <= u64::from(u32::MAX) && parse(&e.text)? == Some(e.mix),
            "Drum mix event witness changed"
        );
    }
    ensure!(
        events
            .windows(2)
            .all(|p| (p[0].tick, p[0].mix.difficulty) < (p[1].tick, p[1].mix.difficulty)),
        "Unordered or duplicate drum mix transitions"
    );
    Ok(())
}
/// Every supported range has an explicit matching clear with the same stem
/// configuration. Other reset variants stay retained and read-only.
pub fn ranges(events: &[Event], difficulty: u8) -> Result<Vec<Range>> {
    validate(events)?;
    ensure!(difficulty <= 3, "Choose a drum difficulty");
    let events: Vec<_> = events
        .iter()
        .filter(|e| e.mix.difficulty == difficulty)
        .collect();
    let mut result = Vec::new();
    for (i, on) in events
        .iter()
        .enumerate()
        .filter(|(_, e)| e.mix.setting == Setting::Disco)
    {
        let off = *events
            .get(i + 1)
            .context("Open-ended disco flip is read-only; retain its source")?;
        ensure!(
            off.mix.setting == Setting::Clear && off.mix.configuration == on.mix.configuration,
            "This disco boundary also changes mix routing; retain its source"
        );
        ensure!(
            i == 0 || events[i - 1].mix.setting == Setting::Clear,
            "This disco range starts within another mix setting; retain its source"
        );
        result.push(Range {
            on: (*on).clone(),
            off: off.clone(),
        });
    }
    Ok(result)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    Put {
        previous: Option<Range>,
        start: u64,
        end: u64,
        configuration: u8,
    },
    Remove {
        range: Range,
    },
}
pub fn edited(events: &[Event], difficulty: u8, edit: &Edit) -> Result<Vec<Event>> {
    let current = ranges(events, difficulty)?;
    let previous = match edit {
        Edit::Put { previous, .. } => previous.as_ref(),
        Edit::Remove { range } => Some(range),
    };
    if let Some(previous) = previous {
        ensure!(
            current.contains(previous),
            "This disco range changed. Select it again."
        );
    }
    // One configuration represents the song's stem layout. Changing it while
    // editing a range could silently reroute another difficulty's recording.
    let source_configuration = events.first().map(|e| e.mix.configuration);
    ensure!(
        events
            .iter()
            .all(|e| Some(e.mix.configuration) == source_configuration),
        "Changing source stem configurations are read-only"
    );
    let mut next = events.to_vec();
    if let Some(previous) = previous {
        // Removing the first flip must retain its declared stem layout.
        let on = next.iter_mut().find(|e| **e == previous.on).unwrap();
        on.mix.setting = Setting::Clear;
        on.text = previous.off.text.clone();
    }
    if let Edit::Put {
        start,
        end,
        configuration,
        ..
    } = edit
    {
        ensure!(
            start < end && *end <= u64::from(u32::MAX),
            "Disco range needs nonempty whole-tick bounds"
        );
        ensure!(
            *configuration <= 5 && source_configuration.is_none_or(|c| c == *configuration),
            "Keep the source drum stem configuration"
        );
        if let Some(previous) = previous {
            next.retain(|e| {
                !(e.mix.difficulty == difficulty
                    && (*start..=*end).contains(&e.tick)
                    && (e.tick == previous.start() || e.tick == previous.end()))
            });
        }
        let part: Vec<_> = next
            .iter()
            .filter(|e| e.mix.difficulty == difficulty)
            .collect();
        let prior = part.iter().rev().find(|e| e.tick < *start);
        ensure!(
            !part.iter().any(|e| (*start < e.tick && e.tick <= *end)
                || e.tick == *start
                    && (e.mix.setting != Setting::Clear
                        || prior.is_some_and(|e| e.mix.setting != Setting::Clear))),
            "This range crosses another mix transition; keep a gap or edit the existing range"
        );
        ensure!(
            prior.is_none_or(|e| e.mix.setting == Setting::Clear),
            "This range would overwrite another mix setting"
        );
        next.retain(|e| !(e.tick == *start && e.mix.difficulty == difficulty));
        let mix = Mix {
            difficulty,
            configuration: *configuration,
            setting: Setting::Disco,
        };
        let mut on = previous.map_or_else(|| Event::new(*start, mix), |p| p.on.clone());
        let mut off = previous.map_or_else(
            || {
                Event::new(
                    *end,
                    Mix {
                        setting: Setting::Clear,
                        ..mix
                    },
                )
            },
            |p| p.off.clone(),
        );
        on.tick = *start;
        off.tick = *end;
        next.extend([on, off]);
    }
    next.sort_by_key(|e| (e.tick, e.mix.difficulty));
    validate(&next)?;
    ranges(&next, difficulty)?;
    Ok(next)
}

#[cfg(test)]
#[path = "drum_mix_midi_tests.rs"]
mod midi_tests;
#[cfg(test)]
#[path = "drum_mix_tests.rs"]
mod tests;
