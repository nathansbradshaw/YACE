use anyhow::{Context, Result, ensure};
use chart_engine::{
    mapping::PitchNote,
    timing::{Tempo, TempoMap},
};
use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};
use serde_json::json;
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::Path,
};
#[allow(dead_code)]
#[path = "../baseline_keys.rs"]
mod baseline_keys;
#[allow(dead_code)]
#[path = "../baseline_pro_keys.rs"]
mod baseline_pro_keys;
#[path = "../../../keyboard-benchmark/src/scoring.rs"]
mod scoring;

fn read_midi(path: &Path) -> Result<(TempoMap, BTreeMap<String, Vec<PitchNote>>)> {
    let bytes = fs::read(path)?;
    let smf = Smf::parse(&bytes)?;
    let Timing::Metrical(res) = smf.header.timing else {
        anyhow::bail!("SMPTE MIDI unsupported")
    };
    let mut tempos = BTreeMap::new();
    for tr in &smf.tracks {
        let mut tick = 0u64;
        for ev in tr {
            tick += ev.delta.as_int() as u64;
            if let TrackEventKind::Meta(MetaMessage::Tempo(t)) = ev.kind {
                let bpm = 60_000_000. / t.as_int() as f64;
                if let Some(old) = tempos.insert(tick, bpm) {
                    ensure!(old == bpm, "conflicting tempo");
                }
            }
        }
    }
    let map = TempoMap::new(
        res.as_int() as u32,
        0.,
        tempos
            .into_iter()
            .map(|(tick, bpm)| Tempo { tick, bpm })
            .collect(),
    )?;
    let mut tracks = BTreeMap::new();
    for tr in &smf.tracks {
        let name = tr
            .iter()
            .find_map(|e| {
                if let TrackEventKind::Meta(MetaMessage::TrackName(n)) = e.kind {
                    Some(String::from_utf8_lossy(n).into_owned())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        if name != "PART KEYS" && name != "PART REAL_KEYS_X" {
            continue;
        }
        let mut tick = 0u64;
        let mut active = BTreeMap::<(u8, u8), VecDeque<u64>>::new();
        let mut notes = Vec::new();
        for ev in tr {
            tick += ev.delta.as_int() as u64;
            if let TrackEventKind::Midi { channel, message } = ev.kind {
                let (key, on) = match message {
                    MidiMessage::NoteOn { key, vel } => (key.as_int(), vel.as_int() > 0),
                    MidiMessage::NoteOff { key, .. } => (key.as_int(), false),
                    _ => continue,
                };
                let playable = if name == "PART KEYS" {
                    (96..=100).contains(&key)
                } else {
                    (48..=72).contains(&key)
                };
                if !playable {
                    continue;
                }
                let queue = active.entry((channel.as_int(), key)).or_default();
                if on {
                    queue.push_back(tick);
                } else {
                    let start = queue.pop_front().context("unmatched reference note-off")?;
                    ensure!(tick > start, "zero length reference note");
                    notes.push(PitchNote {
                        start: map.seconds(start),
                        end: map.seconds(tick),
                        pitch: if name == "PART KEYS" { key - 96 } else { key },
                        confidence: 1.,
                    });
                }
            }
        }
        ensure!(
            active.values().all(|v| v.is_empty()),
            "unclosed reference notes"
        );
        notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
        tracks.insert(name, notes);
    }
    Ok((map, tracks))
}
fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    match args.get(1).map(String::as_str) {
        Some("infer") => {
            let kind = args.get(2).context("kind")?;
            let audio = Path::new(args.get(3).context("audio")?);
            let models = Path::new(args.get(4).context("models")?);
            let out = Path::new(args.get(5).context("output")?);
            ensure!(!out.exists(), "output exists");
            let (notes, raw) = if kind == "piano" {
                let result =
                    chart_engine::piano_model::Transcriber::load(&models.join("piano_kong.onnx"))?
                        .transcribe(audio, Default::default())?;
                (result.notes.clone(), serde_json::to_value(result)?)
            } else {
                ensure!(kind == "synth", "unknown kind");
                let result = chart_engine::muscriptor::Transcriber::load(models)?.transcribe(
                    audio,
                    chart_engine::muscriptor::Options {
                        instrument_groups: vec![32],
                        ..Default::default()
                    },
                )?;
                let indices =
                    chart_engine::keyboard_detection::synth_selected_indices(&result.notes);
                let notes = indices
                    .into_iter()
                    .map(|i| {
                        let n = &result.notes[i];
                        PitchNote {
                            start: n.start,
                            end: n.end,
                            pitch: n.pitch,
                            confidence: 1.,
                        }
                    })
                    .collect::<Vec<_>>();
                (notes, serde_json::to_value(result)?)
            };
            fs::write(
                out,
                serde_json::to_vec_pretty(
                    &json!({"kind":kind,"audio_sha256":chart_engine::models::hash_file(audio)?,"notes":notes,"raw":raw}),
                )?,
            )?;
        }
        Some("compare") | Some("compare-baseline") => {
            let midi = Path::new(args.get(2).context("midi")?);
            let inference: serde_json::Value =
                serde_json::from_slice(&fs::read(args.get(3).context("inference")?)?)?;
            let clip_start = args.get(4).context("clip start")?.parse::<f64>()?;
            let eval_start = args.get(5).context("eval start")?.parse::<f64>()?;
            let eval_end = args.get(6).context("eval end")?.parse::<f64>()?;
            let out = Path::new(args.get(7).context("output")?);
            ensure!(!out.exists(), "output exists");
            let (timing, tracks) = read_midi(midi)?;
            let mut notes: Vec<PitchNote> = serde_json::from_value(
                inference
                    .get("notes")
                    .or_else(|| inference.get("piano"))
                    .context("no detected notes")?
                    .clone(),
            )?;
            for n in &mut notes {
                n.start += clip_start;
                n.end += clip_start;
            }
            let baseline = args[1] == "compare-baseline";
            let (pro, plan) = if baseline {
                let p = baseline_pro_keys::arrange(&notes, &timing)?;
                (p.notes.clone(), serde_json::to_value(p)?)
            } else {
                let p = chart_engine::pro_keys::arrange_generated(&notes, &timing)?;
                (p.notes.clone(), serde_json::to_value(p)?)
            };
            let keys = if baseline {
                baseline_keys::arrange(&notes, &timing, 3, false)?
            } else {
                chart_engine::keys::arrange(&notes, &timing, 3, false)?
            };
            let pro_notes = pro
                .iter()
                .map(|n| PitchNote {
                    start: timing.seconds(n.tick),
                    end: timing.seconds(n.tick + n.sustain),
                    pitch: n.lane,
                    confidence: 1.,
                })
                .collect::<Vec<_>>();
            let keys_notes = keys
                .iter()
                .map(|n| PitchNote {
                    start: timing.seconds(n.tick),
                    end: timing.seconds(n.tick + n.sustain),
                    pitch: n.lane,
                    confidence: 1.,
                })
                .collect::<Vec<_>>();
            let filter = |ns: &[PitchNote]| {
                ns.iter()
                    .filter(|n| n.start >= eval_start && n.start < eval_end)
                    .cloned()
                    .collect::<Vec<_>>()
            };
            let pro_ref = filter(
                tracks
                    .get("PART REAL_KEYS_X")
                    .context("no expert Pro Keys reference")?,
            );
            let keys_ref = filter(
                tracks
                    .get("PART KEYS")
                    .context("no expert Keys reference")?,
            );
            let raw = filter(&notes);
            let pro_notes = filter(&pro_notes);
            let keys_notes = filter(&keys_notes);
            let pc = |ns: &[PitchNote]| {
                ns.iter()
                    .cloned()
                    .map(|mut n| {
                        n.pitch %= 12;
                        n
                    })
                    .collect::<Vec<_>>()
            };
            let report = json!({"controller":"generated-keyboard", "baseline":baseline,"clip_start":clip_start,"evaluation_start":eval_start,"evaluation_end":eval_end,"reference_midi_sha256":chart_engine::models::hash_file(midi)?,"tempo":timing,"reference_pro_keys":pro_ref,"reference_keys":keys_ref,"raw_notes":raw,"arranged_pro_keys":pro_notes,"arranged_keys":keys_notes,"raw_exact_pitch":scoring::metrics(&raw,Some(&pro_ref)),"raw_pitch_class":scoring::metrics(&pc(&raw),Some(&pc(&pro_ref))),"pro_keys_exact_pitch":scoring::metrics(&pro_notes,Some(&pro_ref)),"pro_keys_pitch_class":scoring::metrics(&pc(&pro_notes),Some(&pc(&pro_ref))),"five_lane_keys":scoring::metrics(&keys_notes,Some(&keys_ref)),"arrangement":plan});
            fs::write(out, serde_json::to_vec_pretty(&report)?)?;
        }
        _ => anyhow::bail!(
            "usage: reference infer KIND AUDIO MODELS OUTPUT | compare MIDI INFERENCE CLIP_START EVAL_START EVAL_END OUTPUT"
        ),
    }
    Ok(())
}

#[cfg(test)]
mod reference_tests {
    use super::*;
    #[test]
    fn golden_midi_tempo_change_and_playable_pitch_mapping() -> Result<()> {
        // 480 PPQ: first beat is 0.5s; subsequent beats are 1s.
        let tempo = vec![
            0, 255, 81, 3, 7, 161, 32, 131, 96, 255, 81, 3, 15, 66, 64, 0, 255, 47, 0,
        ];
        let mut pro = vec![0, 255, 3, 16];
        pro.extend(b"PART REAL_KEYS_X");
        pro.extend([0, 144, 48, 100, 135, 64, 128, 48, 0, 0, 255, 47, 0]);
        let mut keys = vec![0, 255, 3, 9];
        keys.extend(b"PART KEYS");
        // Note-on velocity zero is a release, not a second attack.
        keys.extend([131, 96, 144, 97, 100, 131, 96, 144, 97, 0, 0, 255, 47, 0]);
        let mut bytes = b"MThd\0\0\0\x06\0\x01\0\x03\x01\xe0".to_vec();
        for tr in [tempo, pro, keys] {
            bytes.extend(b"MTrk");
            bytes.extend((tr.len() as u32).to_be_bytes());
            bytes.extend(tr);
        }
        let file =
            std::env::temp_dir().join(format!("yace-reference-golden-{}.mid", std::process::id()));
        fs::write(&file, bytes)?;
        let result = read_midi(&file);
        fs::remove_file(file)?;
        let (map, tracks) = result?;
        assert_eq!(map.seconds(480), 0.5);
        assert_eq!(map.seconds(960), 1.5);
        let pro = &tracks["PART REAL_KEYS_X"];
        assert_eq!(pro.len(), 1);
        assert_eq!(pro[0].pitch, 48);
        assert_eq!(pro[0].start, 0.);
        assert_eq!(pro[0].end, 1.5);
        let keys = &tracks["PART KEYS"];
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].pitch, 1);
        assert_eq!(keys[0].start, 0.5);
        assert_eq!(keys[0].end, 1.5);
        Ok(())
    }
}
