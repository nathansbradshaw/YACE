//! Read-only source fixtures; editor commands run only against temporary metadata copies.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, hint::black_box, path::Path, process::Command, time::Instant};

// Exercise the production algorithms, rather than copies of their implementations.
#[path = "../../../src/timing.rs"]
#[allow(dead_code)]
mod timing;
#[path = "../../../src/waveform.rs"]
mod waveform;

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn stats(mut values: Vec<f64>) -> Value {
    values.sort_by(f64::total_cmp);
    let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
    json!({"samples":values.len(),"p50_ms":at(0.5),"p95_ms":at(0.95),"max_ms":at(1.0)})
}
fn measure(mut run: impl FnMut() -> Result<()>, samples: usize) -> Result<Value> {
    for _ in 0..2 {
        run()?;
    }
    let mut elapsed = Vec::new();
    for _ in 0..samples {
        let start = Instant::now();
        run()?;
        elapsed.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(stats(elapsed))
}
fn command(executable: &Path, args: &[&std::ffi::OsStr]) -> Result<()> {
    let output = Command::new(executable).args(args).output()?;
    ensure!(
        output.status.success(),
        "Command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
fn fixture(executable: &Path, root: &Path, samples: usize) -> Result<Value> {
    let original = fs::read(root.join("project.json"))?;
    let chart = fs::read(root.join("source/notes.chart"))?;
    let project: Value = serde_json::from_slice(&original)?;
    let tracks = project["content"]["tracks"]
        .as_object()
        .context("Missing tracks")?;
    let notes: usize = tracks
        .values()
        .filter_map(Value::as_array)
        .map(Vec::len)
        .sum();
    let track = project["selected_track"]
        .as_str()
        .context("Missing track")?;
    ensure!(
        track != "Vocals",
        "Use a fixture with an instrumental track selected"
    );
    let copy = tempfile::tempdir()?;
    fs::create_dir(copy.path().join("source"))?;
    fs::create_dir(copy.path().join("checkpoints"))?;
    fs::write(copy.path().join("project.json"), &original)?;
    fs::write(copy.path().join("source/notes.chart"), &chart)?;
    // Store load/apply do not need audio for these actions. Never copy or mutate recordings.
    let show = measure(
        || {
            command(
                executable,
                &["editor".as_ref(), "show".as_ref(), copy.path().as_os_str()],
            )
        },
        samples,
    )?;
    let mut results = serde_json::Map::new();
    for (name, action) in [
        ("select_track", json!({"type":"select_track","track":track})),
        ("seek", json!({"type":"seek","seconds":0.0})),
        ("playback_pause", json!({"type":"playback","playing":false})),
        // An identical update still exercises the production content checkpoint/save path.
        ("update_note", {
            let note = tracks[track]
                .as_array()
                .and_then(|n| n.first())
                .context("Empty selected track")?;
            json!({"type":"update_note","note_id":note["id"],"tick":note["note"]["tick"],"lane":note["note"]["lane"],"sustain":note["note"]["sustain"],"cymbal":false})
        }),
    ] {
        // Reset between action families so the growing audit of one family cannot bias another.
        fs::write(copy.path().join("project.json"), &original)?;
        let request_path = copy.path().join("request.json");
        let initial_revision = project["revision"].as_u64().context("Missing revision")?;
        let mut times = Vec::new();
        for iteration in 0..samples + 2 {
            let revision = initial_revision + iteration as u64;
            fs::write(
                &request_path,
                serde_json::to_vec(
                    &json!({"expected_revision":revision,"actor":{"kind":"human","name":"performance-probe"},"action":action}),
                )?,
            )?;
            let start = Instant::now();
            command(
                executable,
                &[
                    "editor".as_ref(),
                    "apply".as_ref(),
                    copy.path().as_os_str(),
                    "--request".as_ref(),
                    request_path.as_os_str(),
                ],
            )?;
            if iteration >= 2 {
                times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        results.insert(name.into(), stats(times));
    }
    ensure!(
        fs::read(root.join("project.json"))? == original
            && fs::read(root.join("source/notes.chart"))? == chart,
        "Source fixture changed during measurement"
    );
    Ok(
        json!({"project_sha256":digest(&original),"source_chart_sha256":digest(&chart),"project_bytes":original.len(),"track_count":tracks.len(),"note_count":notes,"audit_entries":project["audit"].as_array().map_or(0,Vec::len),"show_process":show,"apply_process":results}),
    )
}
fn algorithms(samples: usize) -> Result<Value> {
    let mut tempos = Vec::new();
    for count in [1, 100, 1000, 10000] {
        let map = timing::TempoMap::new(
            480,
            0.0,
            (0..count)
                .map(|i| timing::Tempo {
                    tick: i * 480,
                    bpm: 120.0 + (i % 40) as f64,
                })
                .collect(),
        )?;
        let result = measure(
            || {
                for i in 0..4096_u64 {
                    black_box(map.seconds(black_box(i * count * 480 / 4096)));
                }
                Ok(())
            },
            samples,
        )?;
        tempos.push(
            json!({"tempo_events":count,"queries_per_sample":4096,"seconds_conversion":result}),
        );
    }
    // A deterministic 60-second stereo signal: build cost and viewport cost are separate.
    let pcm: Vec<_> = (0..44100 * 60 * 2)
        .map(|i| ((i % 997) as f32 / 498.5) - 1.0)
        .collect();
    let build = measure(
        || {
            black_box(waveform::Waveform::from_stereo(black_box(&pcm), 44100));
            Ok(())
        },
        samples,
    )?;
    let wave = waveform::Waveform::from_stereo(&pcm, 44100);
    let viewport = measure(
        || {
            black_box(wave.geometry(10.0, 12.0, 1400.0));
            Ok(())
        },
        samples,
    )?;
    Ok(
        json!({"tempo":tempos,"waveform":{"audio_seconds":60,"pcm_bytes":pcm.len()*4,"build":build,"viewport_1400px":viewport}}),
    )
}
fn audio(path: &Path, samples: usize) -> Result<Value> {
    let source_hash = digest(&fs::read(path)?);
    let mut results = Vec::new();
    for rate in [1.0, 0.75, 0.5] {
        let mut pcm_bytes = 0;
        let times = measure(
            || {
                let mut command = Command::new("ffmpeg");
                command.args(["-v", "error", "-nostdin", "-i"]).arg(path);
                if rate != 1.0 {
                    command.args(["-af", &format!("atempo={rate:.8}")]);
                }
                let output = command
                    .args([
                        "-map", "0:a:0", "-f", "f32le", "-ar", "44100", "-ac", "2", "pipe:1",
                    ])
                    .output()?;
                ensure!(
                    output.status.success(),
                    "FFmpeg: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                ensure!(output.stdout.len().is_multiple_of(4), "Invalid PCM length");
                let pcm: Vec<_> = output
                    .stdout
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                    .collect();
                ensure!(
                    !pcm.is_empty() && pcm.iter().all(|n| n.is_finite()),
                    "Invalid audio samples"
                );
                pcm_bytes = pcm.len() * 4;
                black_box(pcm);
                Ok(())
            },
            samples.min(5),
        )?;
        results.push(
            json!({"rate":rate,"decoded_pcm_bytes":pcm_bytes,"decode_process_and_pcm_copy":times}),
        );
    }
    ensure!(
        digest(&fs::read(path)?) == source_hash,
        "Audio source changed during measurement"
    );
    Ok(
        json!({"source_sha256":source_hash,"source_bytes":fs::metadata(path)?.len(),"rates":results,"scope":"FFmpeg output plus f32 copy/validation mirrors models::decode_at_tempo; excludes file hashing, stems, waveforms, device creation and UI. At most five samples per rate."}),
    )
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() >= 2,
        "Usage: yace-performance-probe EXECUTABLE PROJECT [PROJECT ...]; YACE_PERF_SAMPLES=20 controls samples (2 warmups)"
    );
    let samples: usize = std::env::var("YACE_PERF_SAMPLES")
        .unwrap_or_else(|_| "20".into())
        .parse()?;
    ensure!(
        (3..=1000).contains(&samples),
        "Sample count must be 3..=1000"
    );
    let executable = Path::new(&args[0]).canonicalize()?;
    let before = digest(&fs::read(&executable)?);
    let startup = measure(|| command(&executable, &["--help".as_ref()]), samples)?;
    let mut fixtures = Vec::new();
    for root in &args[1..] {
        fixtures.push(fixture(&executable, Path::new(root), samples)?);
    }
    let algorithms = algorithms(samples)?;
    let audio = std::env::var_os("YACE_PERF_AUDIO")
        .map(|path| audio(Path::new(&path), samples))
        .transpose()?;
    ensure!(
        digest(&fs::read(&executable)?) == before,
        "Executable changed during measurement; rerun against a stable build"
    );
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"schema_version":1,"executable_sha256":before,"samples":samples,"warmups":2,"command_help_process":startup,"fixtures":fixtures,"algorithms":algorithms,"audio":audio,"scope":"CLI process wall times include process startup and captured stdout, not UI paint or dispatch reload. Metadata-only copies do not measure audio, export, undo restore, or model inference. Warm filesystem cache; system contention uncontrolled. Percentiles are descriptive, not confidence intervals."})
        )?
    );
    Ok(())
}
