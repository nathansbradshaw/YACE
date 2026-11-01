//! Explicit private-file acceptance: consume the browser's real downloaded ZIP
//! through the native decoder and shared DSP. No recording is bundled in Git.
use chart_engine::{
    clean::{self, CleanMode, CleanPlan, CleanReceipt},
    models,
};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::PathBuf};
#[test]
#[ignore = "requires YACE_PARITY_AUDIO and YACE_PARITY_CLEAN_ZIP; optional YACE_PARITY_VOCAL_STEM"]
fn downloaded_browser_clean_song_matches_native_source_clock_and_processing() {
    let audio =
        PathBuf::from(std::env::var_os("YACE_PARITY_AUDIO").expect("explicit source recording"));
    let archive =
        PathBuf::from(std::env::var_os("YACE_PARITY_CLEAN_ZIP").expect("actual browser download"));
    let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
    let mut read = |name: &str| {
        let mut bytes = Vec::new();
        zip.by_name(name).unwrap().read_to_end(&mut bytes).unwrap();
        bytes
    };
    let plan: CleanPlan = serde_json::from_slice(&read("plan.json")).unwrap();
    let receipt: CleanReceipt = serde_json::from_slice(&read("receipt.json")).unwrap();
    let wave = read("clean.wav");
    assert_eq!(&wave[..4], b"RIFF");
    assert_eq!(&wave[8..16], b"WAVEfmt ");
    assert_eq!(u16::from_le_bytes(wave[20..22].try_into().unwrap()), 3);
    assert_eq!(u16::from_le_bytes(wave[22..24].try_into().unwrap()), 2);
    assert_eq!(u32::from_le_bytes(wave[24..28].try_into().unwrap()), 44100);
    assert_eq!(&wave[36..40], b"data");
    let rendered: Vec<f32> = wave[44..]
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let original = models::decode(&audio, 44100, 2).unwrap();
    let stem = if plan.mode == CleanMode::VocalOnly {
        Some(
            models::decode(
                &PathBuf::from(
                    std::env::var_os("YACE_PARITY_VOCAL_STEM").expect("bound vocal stem"),
                ),
                44100,
                2,
            )
            .unwrap(),
        )
    } else {
        None
    };
    let expected = clean::render_pcm_stereo(&original, stem.as_deref(), 44100, &plan).unwrap();
    assert_eq!(rendered.len(), expected.len());
    assert_eq!(receipt.frames * 2, rendered.len());
    assert_eq!(receipt.source_sha256, models::hash_file(&audio).unwrap());
    assert_eq!(
        receipt.output_sha256,
        format!("{:x}", Sha256::digest(&wave))
    );
    let maximum = rendered
        .iter()
        .zip(&expected)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    let at = 44100 * 12;
    let mut best = (f64::INFINITY, 0);
    for lag in -4096i32..=4096 {
        let error = (0..4096usize)
            .step_by(4)
            .map(|n| {
                let a = rendered[(at + n) * 2];
                let b = original[((at + n) as i32 + lag) as usize * 2];
                ((a - b) as f64).powi(2)
            })
            .sum::<f64>();
        if error < best.0 {
            best = (error, lag);
        }
    }
    let ab: f64 = rendered[at * 2..(at + 44100) * 2]
        .iter()
        .zip(&original[at * 2..(at + 44100) * 2])
        .map(|(a, b)| *a as f64 * *b as f64)
        .sum();
    let aa: f64 = original[at * 2..(at + 44100) * 2]
        .iter()
        .map(|v| (*v as f64).powi(2))
        .sum();
    eprintln!(
        "decoder comparison max={maximum}, best source lag={} frames, residual={}, gain={}",
        best.1,
        best.0,
        ab / aa
    );
    // The clients use different Vorbis decoders; neither rescales source time.
    assert!(
        maximum < 0.0002,
        "browser/native decoded sample difference {maximum}"
    );
    assert_eq!(
        receipt.rendered_ranges,
        clean::merged_ranges(&plan).unwrap()
    );
    if plan.mode == CleanMode::FullMix {
        for (start, end) in &receipt.rendered_ranges {
            assert!(
                rendered
                    [((start * 44100.).ceil() as usize * 2)..((end * 44100.).floor() as usize * 2)]
                    .iter()
                    .all(|v| *v == 0.)
            );
        }
    }
    eprintln!(
        "Verified {} stereo frames; max browser/native sample delta {maximum}",
        receipt.frames
    );
}
