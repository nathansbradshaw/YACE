//! Selective .chart local-text rendering; unrelated rows keep their bytes.
use super::{Event, parse, validate};
use crate::chart::Chart;
use anyhow::{Context, Result, ensure};
use std::collections::BTreeMap;
pub type Tracks = BTreeMap<String, Vec<Event>>;
pub fn difficulty(track: &str) -> Result<u8> {
    match track {
        "EasyDrums" => Ok(0),
        "MediumDrums" => Ok(1),
        "HardDrums" => Ok(2),
        "ExpertDrums" => Ok(3),
        _ => anyhow::bail!("Choose an editable drum difficulty"),
    }
}
fn local(line: &str) -> Result<Option<Event>> {
    let Some((tick, body)) = line.split_once('=') else {
        return Ok(None);
    };
    let body = body.trim();
    let Some(text) = body.strip_prefix("E ").or_else(|| body.strip_prefix("E\t")) else {
        return Ok(None);
    };
    let text = text.trim();
    let text = if let Some(inner) = text.strip_prefix('"') {
        match inner.strip_suffix('"') {
            Some(inner) => inner,
            None => {
                ensure!(
                    !inner.starts_with("mix") && !inner.starts_with("[mix"),
                    "Unclosed drum mix text; retain source"
                );
                return Ok(None);
            }
        }
    } else {
        text
    };
    let Some(mix) = parse(text)? else {
        return Ok(None);
    };
    Ok(Some(Event {
        tick: tick.trim().parse().context("Invalid drum mix tick")?,
        mix,
        text: text.into(),
    }))
}
pub fn import_chart(chart: &Chart, track: &str) -> Result<Vec<Event>> {
    let expected = difficulty(track)?;
    let Some(section) = chart.sections.iter().find(|s| s.name == track) else {
        return Ok(vec![]);
    };
    let mut events = vec![];
    for line in chart.raw[section.body_start..section.body_end].lines() {
        if let Some(event) = local(line)? {
            ensure!(
                event.mix.difficulty == expected,
                "Drum mix text targets another difficulty; retain source"
            );
            events.push(event);
        }
    }
    events.sort_by_key(|e| (e.tick, e.mix.difficulty));
    validate(&events)?;
    Ok(events)
}
pub fn render_chart(text: String, tracks: &Tracks) -> Result<String> {
    if tracks.is_empty() {
        return Ok(text);
    }
    let chart = Chart::parse(text)?;
    let newline = if chart.raw.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut changes = BTreeMap::new();
    for (track, events) in tracks {
        let expected = difficulty(track)?;
        validate(events)?;
        ensure!(
            events.iter().all(|e| e.mix.difficulty == expected),
            "Drum mix target difficulty changed"
        );
        let original = import_chart(&chart, track)?;
        ensure!(
            events.is_empty() || chart.sections.iter().any(|s| s.name == *track),
            "Missing drum mix part {track}"
        );
        if original != *events {
            changes.insert(track, events);
        }
    }
    let mut output = chart.raw.clone();
    for section in chart.sections.iter().rev() {
        let Some(events) = changes.get(&section.name) else {
            continue;
        };
        let mut body = String::new();
        for line in chart.raw[section.body_start..section.body_end].split_inclusive('\n') {
            if local(line)?.is_none() {
                body.push_str(line)
            }
        }
        if !body.is_empty() && !body.ends_with('\n') {
            body.push_str(newline)
        }
        for event in *events {
            body.push_str(&format!(
                "  {} = E \"{}\"{newline}",
                event.tick,
                crate::chart::quoted(&event.text)
            ));
        }
        output.replace_range(section.body_start..section.body_end, &body);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    const CHART: &str = "[Song]\r\n{\r\nResolution = 480\r\n}\r\n[SyncTrack]\r\n{\r\n0 = B 120000\r\n}\r\n[Events]\r\n{\r\n1920 = E \"section End\"\r\n}\r\n[ExpertDrums]\r\n{\r\n  240 = N 1 0\r\n  960 = S 64 480\r\n  240 = E mix_3_drums3d\r\n  480 = E \"mix_3_drums3\"\r\n  123 = E \"[author sentinel]\"\r\n}\r\n[HardDrums]\r\n{\r\n  0 = E \"[mix 2 drums3dnoflip]\"\r\n  240 = N 2 0\r\n}\r\n";
    #[test]
    fn drum_mix_chart_noop_move_clear_and_other_rows_are_preserved() {
        let chart = Chart::parse(CHART.into()).unwrap();
        let before = import_chart(&chart, "ExpertDrums").unwrap();
        assert_eq!(
            render_chart(
                CHART.into(),
                &[("ExpertDrums".into(), before.clone())].into()
            )
            .unwrap(),
            CHART
        );
        let range = super::super::ranges(&before, 3).unwrap()[0].clone();
        let after = super::super::edited(
            &before,
            3,
            &super::super::Edit::Put {
                previous: Some(range),
                start: 720,
                end: 960,
                configuration: 3,
            },
        )
        .unwrap();
        let output = render_chart(
            CHART.into(),
            &[("ExpertDrums".into(), after.clone())].into(),
        )
        .unwrap();
        for row in [
            "  240 = N 1 0\r\n",
            "  960 = S 64 480\r\n",
            "  123 = E \"[author sentinel]\"\r\n",
            "[HardDrums]\r\n{\r\n  0 = E \"[mix 2 drums3dnoflip]\"\r\n  240 = N 2 0\r\n}\r\n",
        ] {
            assert!(output.contains(row), "Lost {row:?}");
        }
        assert_eq!(
            import_chart(&Chart::parse(output.clone()).unwrap(), "ExpertDrums").unwrap(),
            after
        );
        let current = super::super::ranges(&after, 3).unwrap()[0].clone();
        let clear = super::super::edited(&after, 3, &super::super::Edit::Remove { range: current })
            .unwrap();
        let output = render_chart(output, &[("ExpertDrums".into(), clear)].into()).unwrap();
        assert!(
            super::super::ranges(
                &import_chart(&Chart::parse(output).unwrap(), "ExpertDrums").unwrap(),
                3
            )
            .unwrap()
            .is_empty()
        );
    }
    #[test]
    fn drum_mix_chart_refuses_wrong_tier_unknown_flags_and_missing_parts() {
        let bad = CHART.replace("mix_3_drums3d", "mix_2_drums3d");
        assert!(import_chart(&Chart::parse(bad).unwrap(), "ExpertDrums").is_err());
        let bad = CHART.replace("mix_3_drums3d", "mix_3_drums3future");
        assert!(import_chart(&Chart::parse(bad).unwrap(), "ExpertDrums").is_err());
        let chart = Chart::parse(CHART.into()).unwrap();
        let events = import_chart(&chart, "ExpertDrums").unwrap();
        assert!(render_chart(CHART.into(), &[("EasyDrums".into(), events)].into()).is_err());
    }

    #[test]
    fn drum_mix_chart_conversion_and_ordinary_project_note_edits_keep_midi_mix_text() {
        use crate::editor::{Action, Actor, ActorKind, Request, Store};
        let text = CHART.replace("  123 = E \"[author sentinel]\"\r\n", "");
        let chart = Chart::parse(text).unwrap();
        let (bytes, _) = crate::game_export::song(&chart, None).unwrap();
        let before = crate::drum_mix::import_midi(&bytes).unwrap();
        let mut expected = import_chart(&chart, "ExpertDrums").unwrap();
        expected.extend(import_chart(&chart, "HardDrums").unwrap());
        expected.sort_by_key(|e| (e.tick, e.mix.difficulty));
        assert_eq!(before.events, expected);
        let root = tempfile::tempdir().unwrap();
        let song = root.path().join("song");
        std::fs::create_dir(&song).unwrap();
        std::fs::write(song.join("notes.mid"), &bytes).unwrap();
        std::fs::write(
            song.join("song.ini"),
            "[song]\nname = Drum mix conversion\nsong_length = 12000\npro_drums = true\n",
        )
        .unwrap();
        let store = Store::create(&song, None, &root.path().join("project")).unwrap();
        let actor = Actor {
            kind: ActorKind::Human,
            name: "Drum mix regression".into(),
        };
        store
            .apply(Request {
                expected_revision: 0,
                actor: actor.clone(),
                action: Action::AddNote {
                    track: "ExpertDrums".into(),
                    tick: 1920,
                    lane: 2,
                    sustain: 0,
                    cymbal: true,
                },
            })
            .unwrap();
        let output = root.path().join("export");
        store
            .export_with_options(
                &store.load().unwrap(),
                &output,
                &actor,
                &crate::export_options::Options {
                    include_audio: false,
                    ..Default::default()
                },
            )
            .unwrap();
        let midi = std::fs::read(output.join("notes.mid")).unwrap();
        assert_eq!(
            crate::drum_mix::import_midi(&midi).unwrap().events,
            before.events
        );
        let reopened = Store::create(&output, None, &root.path().join("reopened")).unwrap();
        let again = root.path().join("again");
        reopened
            .export_with_options(
                &reopened.load().unwrap(),
                &again,
                &actor,
                &crate::export_options::Options {
                    include_audio: false,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(std::fs::read(again.join("notes.mid")).unwrap(), midi);
    }
}
