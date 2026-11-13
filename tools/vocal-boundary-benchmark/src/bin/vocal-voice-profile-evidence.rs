use anyhow::{Context, Result, ensure};
use chart_engine::{models, vocal_generation::Generation};
use ort::{session::Session, value::Tensor};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::{fs, path::Path, time::Instant};
use vocal_boundary_benchmark::{read, voice_profile as voice};
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().collect();
    ensure!(
        a.len() == 7,
        "vocal-voice-profile-evidence SOURCE_INVENTORY MODEL EXPECTED_SHA RUNTIME NEW_OUTPUT STRIDE_SECONDS"
    );
    let stride: f64 = a[6].parse()?;
    ensure!(stride == 0.5, "Frozen half-second center stride");
    let model = Path::new(&a[2]);
    let runtime = Path::new(&a[4]);
    ensure!(
        a[3] == "b50810498b5bcf5773d086f6993d344476bd0c88b566a41e8d801aaf8461efad"
            && models::hash_file(model)? == a[3]
            && runtime.is_file(),
        "Model/runtime changed"
    );
    ort::init_from(runtime.to_string_lossy()).commit()?;
    let mut session = Session::builder()?
        .with_intra_threads(2)?
        .with_inter_threads(1)?
        .commit_from_file(model)?;
    ensure!(
        session.inputs.len() == 1
            && session.inputs[0].name == "feats"
            && session.outputs.iter().any(|o| o.name == "embs"),
        "Unexpected voice graph"
    );
    let out = Path::new(&a[5]);
    ensure!(!out.exists(), "Output exists");
    fs::create_dir(out)?;
    fs::write(
        out.join("interface.txt"),
        format!("{:?}\n{:?}\n", session.inputs, session.outputs),
    )?;
    let m: Value = read(&a[1])?;
    let mut pins = vec![];
    let mut ids = BTreeSet::new();
    for e in m["recordings"].as_array().context("recordings")? {
        let timer = Instant::now();
        let id = e["id"].as_str().context("id")?;
        let gp = Path::new(e["generation"].as_str().context("generation")?);
        let g: Generation = read(gp)?;
        ensure!(
            !id.is_empty()
                && id.len() <= 80
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                && ids.insert(id.to_owned())
                && g.duration_seconds.is_finite()
                && (0.1..=600.).contains(&g.duration_seconds),
            "Invalid/duplicate source ID or duration"
        );
        let audio = Path::new(e["vocal_stem"].as_str().context("stem")?);
        ensure!(
            models::hash_file(audio)? == g.vocal_stem_sha256,
            "Stem changed"
        );
        let gamepath = Path::new(e["game_features"].as_str().context("GAME")?);
        let game: Value = read(gamepath)?;
        ensure!(
            game["source_sha256"] == g.vocal_stem_sha256 && game["timestep"] == 0.01,
            "GAME source"
        );
        let pcm = models::decode(audio, 16000, 1)?;
        ensure!(
            (pcm.len() as f64 / 16000. - g.duration_seconds).abs() <= 1. / 16000. + 1e-8,
            "Voice source clock"
        );
        let raw = g.rmvpe_pitch.as_ref().context("raw")?;
        ensure!(
            raw.audio_sha256 == g.audio_sha256 && raw.vocal_stem_sha256 == g.vocal_stem_sha256,
            "Raw voice source mismatch"
        );
        let mut embeddings = vec![];
        let mut norms = vec![];
        let mut qualities = vec![];
        let mut background = vec![];
        let mut times = vec![];
        for k in 0..(g.duration_seconds / stride).ceil() as usize {
            let t = k as f64 * stride;
            let center = (t * 16000.).round() as isize;
            let clip: Vec<_> = (0..32000)
                .map(|j| {
                    let at = center + j - 16000;
                    if at < 0 {
                        0.
                    } else {
                        pcm.get(at as usize).copied().unwrap_or(0.)
                    }
                })
                .collect();
            let features = voice::fbank(&clip)?;
            let frames = features.len() / 80;
            let outputs = session
                .run(ort::inputs!["feats"=>Tensor::from_array(([1,frames,80],features))?])?;
            let (shape, data) = outputs["embs"].try_extract_tensor::<f32>()?;
            ensure!(
                shape.len() == 2 && shape[0] == 1 && (64..=512).contains(&data.len()),
                "Voice embedding shape"
            );
            let mut embedding = data.to_vec();
            let norm = voice::normalize(&mut embedding)?;
            embeddings.push(embedding);
            norms.push(norm);
            times.push(t);
            let lo = raw
                .frames
                .partition_point(|f| f.time_seconds < (t - 1.).max(0.));
            let hi = raw.frames.partition_point(|f| f.time_seconds < t + 1.);
            let (mut positive, mut negative) = (0., 0.);
            for i in lo..hi {
                let voice = game["rows"][i]["presence_confidence"]
                    .as_f64()
                    .unwrap_or(0.);
                if raw.frames[i].confidence >= 0.6 {
                    positive += voice;
                    negative += 1. - voice;
                }
            }
            let count = (hi - lo).max(1) as f64;
            qualities.push(positive / count);
            background.push(negative / count);
            if k % 100 == 99 {
                eprintln!(
                    "{id}: {} voice windows in {:.1}s",
                    k + 1,
                    timer.elapsed().as_secs_f64()
                );
            }
        }
        let seeds = |q: &[f64]| {
            let mut ids: Vec<_> = (0..times.len())
                .filter(|&i| times[i] >= 1. && times[i] + 1. <= g.duration_seconds && q[i] >= 0.5)
                .collect();
            ids.sort_by(|&a, &b| q[b].total_cmp(&q[a]).then(a.cmp(&b)));
            ids.truncate(32);
            ids
        };
        let pos = seeds(&qualities);
        let neg = seeds(&background);
        let positive = voice::prototypes(&embeddings, &pos)?;
        let negative = voice::prototypes(&embeddings, &neg)?;
        let mut frames = vec![];
        for i in 0..times.len() {
            let v = &embeddings[i];
            let p = positive
                .iter()
                .map(|m| voice::cosine(m, v))
                .fold(-1., f64::max);
            let n = negative
                .iter()
                .map(|m| voice::cosine(m, v))
                .fold(-1., f64::max);
            frames.push([
                p,
                n,
                p - n,
                norms[i],
                f64::from(!positive.is_empty()) + 2. * f64::from(!negative.is_empty()),
                qualities[i],
            ]);
        }
        let file = format!("{id}.json");
        let bytes = serde_json::to_vec(
            &json!({"audio_sha256":g.audio_sha256,"vocal_stem_sha256":g.vocal_stem_sha256,"generation_sha256":models::hash_file(gp)?,"game_sha256":models::hash_file(gamepath)?,"model_sha256":a[3],"duration_seconds":g.duration_seconds,"window_seconds":2.,"stride_seconds":stride,"times":times,"frames":frames,"positive_seed_indices":pos,"negative_seed_indices":neg,"embeddings":embeddings,"seconds":timer.elapsed().as_secs_f64()}),
        )?;
        fs::write(out.join(&file), &bytes)?;
        pins.push(json!({"id":id,"file":file,"sha256":format!("{:x}",Sha256::digest(&bytes))}));
        eprintln!("{id}: voice profile complete");
    }
    fs::write(
        out.join("inference-receipt.json"),
        serde_json::to_vec_pretty(
            &json!({"reads_reference_labels":false,"status":"EXPERIMENTAL_SPEECH_EMBEDDING_NOT_SINGING_OR_IDENTITY_VALIDATION","model_sha256":a[3],"runtime_sha256":models::hash_file(runtime)?,"implementation_sha256":format!("{:x}",Sha256::digest([include_bytes!("vocal-voice-profile-evidence.rs").as_slice(),include_bytes!("../voice_profile.rs").as_slice()].concat())),"inputs":pins}),
        )?,
    )?;
    Ok(())
}
