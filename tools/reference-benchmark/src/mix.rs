//! Canonical synchronized stem sum. No separation, retiming or target notes.
use crate::archive::{Archive, sha};
use anyhow::{Context, Result, ensure};
use std::{io::Write, path::Path, process::Command};
const RATE: u32 = 44100;
const MAX_SECONDS: usize = 1800;
pub fn create(directory: &Path, destination: &Path) -> Result<()> {
    ensure!(!destination.exists(), "refusing to replace existing mix");
    let manifest_path = destination.with_extension("mix.json");
    ensure!(!manifest_path.exists(), "mix receipt already exists");
    let source_path = directory.join("reference-source.json");
    let source: serde_json::Value = serde_json::from_slice(&std::fs::read(&source_path)?)?;
    let archive_path = source["source_archive"]
        .as_str()
        .context("missing archive identity")?;
    let current = Archive::open(Path::new(archive_path))?;
    ensure!(
        source["source_archive_sha256"] == current.sha256,
        "source archive changed"
    );
    let mut stems = Vec::new();
    for entry in &current.entries {
        let path = Path::new(&entry.name);
        let Some(stem) = path.file_stem().and_then(|x| x.to_str()) else {
            continue;
        };
        let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("");
        let included = matches!(
            stem,
            "song"
                | "guitar"
                | "bass"
                | "rhythm"
                | "keys"
                | "vocals"
                | "vocals_1"
                | "vocals_2"
                | "drums"
                | "drums_1"
                | "drums_2"
                | "drums_3"
                | "drums_4"
        );
        if !included || !matches!(ext, "opus" | "ogg" | "wav" | "mp3" | "flac") {
            continue;
        }
        ensure!(
            !stems
                .iter()
                .any(|(s, _, _): &(String, Vec<f32>, String)| s == stem),
            "duplicate alternate encodings forstem{stem}; choose explicitly"
        );
        let extracted = std::fs::read(directory.join(&entry.name))?;
        let stem_hash = sha(&extracted);
        ensure!(
            stem_hash == sha(&current.data(entry)),
            "extracted stem changed:{}",
            entry.name
        );
        ensure!(
            !stems.iter().any(|(_, _, hash)| hash == &stem_hash),
            "identical audio assets in multiple roles; explicit disambiguation required"
        );
        let decoded = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(directory.join(&entry.name))
            .args([
                "-t",
                "1800",
                "-f",
                "f32le",
                "-acodec",
                "pcm_f32le",
                "-ac",
                "2",
                "-ar",
                "44100",
                "pipe:1",
            ])
            .output()?;
        ensure!(
            decoded.status.success(),
            "audio decoding failed:{}",
            String::from_utf8_lossy(&decoded.stderr)
        );
        ensure!(decoded.stdout.len() % 8 == 0, "invalid stereoPCM length");
        let pcm = decoded
            .stdout
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect::<Vec<_>>();
        ensure!(pcm.iter().all(|s| s.is_finite()), "nonfinite PCM");
        ensure!(
            pcm.len() < MAX_SECONDS * RATE as usize * 2,
            "stem at1800s limit; refusing possible truncation"
        );
        stems.push((stem.to_owned(), pcm, stem_hash));
    }
    ensure!(!stems.is_empty(), "no recognized audio stems");
    ensure!(
        !(stems.iter().any(|(s, _, _)| s == "drums")
            && stems.iter().any(|(s, _, _)| s.starts_with("drums_"))),
        "full drum mix and split drum stems coexist; explicit disambiguation required"
    );
    ensure!(
        !(stems.iter().any(|(s, _, _)| s == "vocals")
            && stems.iter().any(|(s, _, _)| s.starts_with("vocals_"))),
        "full vocal mix and split vocal stems coexist; explicit disambiguation required"
    );
    let length = stems.iter().map(|(_, s, _)| s.len()).max().unwrap();
    let mut mixed = vec![0.0f64; length];
    for (_, pcm, _) in &stems {
        for (i, v) in pcm.iter().enumerate() {
            mixed[i] += f64::from(*v);
        }
    }
    let peak = mixed.iter().map(|v| v.abs()).fold(0.0, f64::max);
    let gain = if peak > 0.95 { 0.95 / peak } else { 1.0 };
    let pcm = mixed.iter().map(|v| (v * gain) as f32).collect::<Vec<_>>();
    let data_length = u32::try_from(pcm.len() * 4)?;
    let mut wav = Vec::with_capacity(data_length as usize + 44);
    wav.extend(b"RIFF");
    wav.extend((data_length + 36).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(3u16.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(RATE.to_le_bytes());
    wav.extend((RATE * 8).to_le_bytes());
    wav.extend(8u16.to_le_bytes());
    wav.extend(32u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(data_length.to_le_bytes());
    for sample in pcm {
        wav.extend(sample.to_le_bytes());
    }
    let manifest = serde_json::json!({"version":1,"source_archive":archive_path,"source_archive_sha256":current.sha256,"output":destination,"output_sha256":sha(&wav),"sample_rate":RATE,"channels":2,"frames":length/2,"duration_seconds":length as f64/2.0/RATE as f64,"format":"IEEE float32 little-endian WAV","algorithm":"decode each bundled unique game stem at44100Hz stereo; sample-index aligned sum with shorter tails zero-padded; one global peak gain if needed; no normalization of individual stems, no limiter, no time stretch or offset","global_gain":gain,"pre_gain_peak":peak,"ini_delay_milliseconds":current.metadata.get("delay").map(|v|v.parse::<f64>()).transpose()?,"midi_notes_do_not_enter_mix":true,"included_stems":stems.iter().map(|(s,pcm,hash)|serde_json::json!({"stem":s,"sha256":hash,"frames":pcm.len()/2,"starts_at_source_frame":0})).collect::<Vec<_>>()});
    let receipt = serde_json::to_vec_pretty(&manifest)?;
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    out.write_all(&wav)?;
    out.sync_all()?;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&manifest_path)?
        .write_all(&receipt)?;
    println!(
        "{}",
        serde_json::json!({"mix":destination,"sha256":sha(&wav),"duration_seconds":manifest["duration_seconds"],"stems":stems.len(),"receipt":manifest_path})
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_mix_receipt_prevents_output_before_source_reads() {
        let destination =
            std::env::temp_dir().join(format!("yace-mix-guard-{}.wav", std::process::id()));
        let receipt = destination.with_extension("mix.json");
        let bytes = b"preserved prior receipt";
        std::fs::write(&receipt, bytes).unwrap();
        assert!(create(Path::new("missing-source-directory"), &destination).is_err());
        assert!(!destination.exists());
        assert_eq!(std::fs::read(&receipt).unwrap(), bytes);
        std::fs::remove_file(receipt).unwrap();
    }
}
