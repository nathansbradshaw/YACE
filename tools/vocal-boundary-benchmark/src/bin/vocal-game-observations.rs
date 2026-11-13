//! Offline experimental GAME ONNX probe. No references, lyrics, or note targets are read.
//! Pipeline follows openvpi/GAME (MIT); weights are CC-BY-NC-SA-4.0, private only.
use anyhow::{Context, Result, bail, ensure};
use ort::{session::Session, value::Tensor};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::Path,
    time::Instant,
};
use vocal_boundary_benchmark::read;
#[derive(Clone)]
struct Field<'a> {
    number: u64,
    wire: u64,
    raw: &'a [u8],
    data: &'a [u8],
}
fn varint(b: &[u8], i: &mut usize) -> Result<u64> {
    let mut n = 0;
    for shift in (0..70).step_by(7) {
        let v = *b.get(*i).context("Truncated protobuf")?;
        *i += 1;
        ensure!(shift < 63 || v <= 1, "Varint overflow");
        n |= ((v & 127) as u64) << shift;
        if v & 128 == 0 {
            return Ok(n);
        }
    }
    bail!("Varint overflow")
}
fn fields(b: &[u8]) -> Result<Vec<Field<'_>>> {
    let mut i = 0;
    let mut out = vec![];
    while i < b.len() {
        let start = i;
        let k = varint(b, &mut i)?;
        let wire = k & 7;
        let (a, z) = match wire {
            0 => {
                let a = i;
                varint(b, &mut i)?;
                (a, i)
            }
            1 => {
                let a = i;
                i += 8;
                (a, i)
            }
            2 => {
                let len = usize::try_from(varint(b, &mut i)?)?;
                let a = i;
                i = i.checked_add(len).context("Field overflow")?;
                (a, i)
            }
            5 => {
                let a = i;
                i += 4;
                (a, i)
            }
            _ => bail!("Unsupported protobuf wire"),
        };
        ensure!(i <= b.len() && k >> 3 > 0, "Invalid protobuf field");
        out.push(Field {
            number: k >> 3,
            wire,
            raw: &b[start..i],
            data: &b[a..z],
        });
    }
    Ok(out)
}
fn push_var(b: &mut Vec<u8>, mut v: u64) {
    while v >= 128 {
        b.push((v as u8 & 127) | 128);
        v >>= 7;
    }
    b.push(v as u8)
}
fn push_field(b: &mut Vec<u8>, n: u64, data: &[u8]) {
    push_var(b, n << 3 | 2);
    push_var(b, data.len() as u64);
    b.extend(data)
}
fn strings<'a>(f: &'a [Field<'a>], n: u64) -> Result<Vec<String>> {
    f.iter()
        .filter(|f| f.number == n && f.wire == 2)
        .map(|f| Ok(std::str::from_utf8(f.data)?.into()))
        .collect()
}
fn instrument_estimator(original: &[u8]) -> Result<(Vec<u8>, String)> {
    let outer = fields(original)?;
    let graph = outer.iter().find(|f| f.number == 7).context("ONNX graph")?;
    let gf = fields(graph.data)?;
    let mut nodes = BTreeMap::new();
    let mut output = None;
    for f in &gf {
        if f.number == 1 {
            let nf = fields(f.data)?;
            let ins = strings(&nf, 1)?;
            let outs = strings(&nf, 2)?;
            let op = strings(&nf, 4)?
                .into_iter()
                .next()
                .context("Node operation")?;
            for o in outs {
                nodes.insert(o, (op.clone(), ins.clone()));
            }
        }
        if f.number == 12 {
            let vf = fields(f.data)?;
            if strings(&vf, 1)?.first().is_some_and(|n| n == "scores") {
                output = Some(f.data.to_vec())
            }
        }
    }
    let mut todo = vec!["presence".to_owned()];
    let mut seen = BTreeSet::new();
    let mut confidence = None;
    while let Some(name) = todo.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some((op, ins)) = nodes.get(&name) {
            if (op == "Greater" || op == "GreaterOrEqual")
                && ins.get(1).is_some_and(|n| n == "threshold")
            {
                ensure!(confidence.is_none(), "Ambiguous threshold ancestry");
                confidence = ins.first().cloned();
            } else {
                todo.extend(ins.iter().cloned());
            }
        }
    }
    let confidence = confidence.context("Presence threshold ancestor")?;
    let template = output.context("Scores output type")?;
    let mut vi = vec![];
    push_field(&mut vi, 1, confidence.as_bytes());
    for f in fields(&template)? {
        if f.number != 1 {
            vi.extend(f.raw)
        }
    }
    let mut newgraph = graph.data.to_vec();
    push_field(&mut newgraph, 12, &vi);
    let mut model = vec![];
    for f in outer {
        if f.number == 7 {
            push_field(&mut model, 7, &newgraph)
        } else {
            model.extend(f.raw)
        }
    }
    Ok((model, confidence))
}
fn session(path: &Path) -> Result<Session> {
    Ok(Session::builder()?
        .with_intra_threads(2)?
        .with_inter_threads(1)?
        .commit_from_file(path)?)
}

fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().collect();
    ensure!(
        a.len() == 7,
        "vocal-game-observations MODEL_DIR INPUT EXPECTED_STEM_SHA NEW_OUTPUT STEPS LANGUAGE"
    );
    let root = Path::new(&a[1]);
    let input = Path::new(&a[2]);
    let out = Path::new(&a[4]);
    let steps: usize = a[5].parse()?;
    let language: i64 = a[6].parse()?;
    ensure!(
        !out.exists() && [1, 2, 8].contains(&steps) && (0..=4).contains(&language),
        "Invalid protocol/output"
    );
    let source = chart_engine::models::hash_file(input)?;
    ensure!(source == a[3], "Wrong vocal stem");
    let cfg: Value = read(root.join("config.json"))?;
    ensure!(
        cfg["samplerate"] == 44100
            && cfg["timestep"] == 0.01
            && cfg["embedding_dim"] == 128
            && cfg["loop"] == true,
        "Small GAME configuration required"
    );
    let mut assetpins = vec![];
    for (file, sha) in [
        (
            "encoder.onnx",
            "a6868a702d9c88ccc06b19e54f49d88b7f8cba4a68517f20806bd81ef6f94e06",
        ),
        (
            "segmenter.onnx",
            "7724db4ba59a388fff5aba2f08f0ac025d4ca59caef80e7f474053a1455c88e5",
        ),
        (
            "estimator.onnx",
            "a84738b2504472a1f22773cc6b0eccad1353a8f36229ad490b8fea85388bb5e1",
        ),
    ] {
        let h = chart_engine::models::hash_file(&root.join(file))?;
        ensure!(h == sha, "GAME asset changed");
        assetpins.push(json!({"file":file,"sha256":h}));
    }
    let runtime = std::env::var_os("ORT_DYLIB_PATH").context("Explicit research runtime")?;
    ort::init_from(runtime.to_string_lossy()).commit()?;
    let original = fs::read(root.join("estimator.onnx"))?;
    let (patched, confidence_name) = instrument_estimator(&original)?;
    let patched_hash = format!("{:x}", Sha256::digest(&patched));
    let mut estimator = Session::builder()?
        .with_intra_threads(2)?
        .with_inter_threads(1)?
        .commit_from_memory(&patched)?;
    let mut encoder = session(&root.join("encoder.onnx"))?;
    let mut segmenter = session(&root.join("segmenter.onnx"))?;
    let pcm = chart_engine::models::decode(input, 44100, 1)?;
    let duration = pcm.len() as f64 / 44100.;
    ensure!((0.1..=360.).contains(&duration), "Clip budget");
    let n = pcm.len() / 441;
    let mut rows = vec![json!(null); n];
    let mut chunkpins = vec![];
    let starttime = Instant::now();
    for core in (0..n).step_by(2000) {
        let end = (core + 2000).min(n);
        let lo = core.saturating_sub(100);
        let hi = (end + 100).min(n);
        let sample_a = lo * 441;
        let sample_b = if hi == n { pcm.len() } else { hi * 441 };
        let audio = pcm[sample_a..sample_b].to_vec();
        let seconds = audio.len() as f32 / 44100.;
        let latent=encoder.run(ort::inputs!["waveform"=>Tensor::from_array(([1,audio.len()],audio))?,"duration"=>Tensor::from_array(([1],vec![seconds]))?])?;
        let (shape, seg) = latent["x_seg"].try_extract_tensor::<f32>()?;
        let dims: Vec<_> = shape.iter().map(|v| *v as usize).collect();
        ensure!(
            dims.len() == 3 && dims[0] == 1 && dims[2] == 128,
            "Embedding shape"
        );
        let t = dims[1];
        let seg = seg.to_vec();
        let est = latent["x_est"].try_extract_tensor::<f32>()?.1.to_vec();
        let mask = latent["maskT"].try_extract_tensor::<bool>()?.1.to_vec();
        ensure!(
            mask.len() == t && mask.iter().filter(|v| **v).count() == hi - lo,
            "Frame clock mismatch"
        );
        drop(latent);
        let known = vec![false; t];
        let mut boundaries = known.clone();
        for step in 0..steps {
            let result=segmenter.run(ort::inputs!["x_seg"=>Tensor::from_array((dims.clone(),seg.clone()))?,"language"=>Tensor::from_array(([1],vec![language]))?,"known_boundaries"=>Tensor::from_array(([1,t],known.clone()))?,"prev_boundaries"=>Tensor::from_array(([1,t],boundaries))?,"t"=>Tensor::from_array(([1],vec![step as f32/steps as f32]))?,"maskT"=>Tensor::from_array(([1,t],mask.clone()))?,"threshold"=>Tensor::from_array(([0usize;0],vec![0.2f32]))?,"radius"=>Tensor::from_array(([0usize;0],vec![2i64]))?])?;
            boundaries = result["boundaries"]
                .try_extract_tensor::<bool>()?
                .1
                .to_vec();
            ensure!(boundaries.len() == t, "Boundary shape");
        }
        let mut region = 0usize;
        let indices: Vec<_> = boundaries
            .iter()
            .zip(&mask)
            .map(|(b, m)| {
                region += usize::from(*b);
                if *m { Some(region) } else { None }
            })
            .collect();
        let notes = indices.iter().flatten().max().context("No frames")? + 1;
        let maskn = vec![true; notes];
        let results=estimator.run(ort::inputs!["x_est"=>Tensor::from_array((dims.clone(),est.clone()))?,"boundaries"=>Tensor::from_array(([1,t],boundaries.clone()))?,"maskT"=>Tensor::from_array(([1,t],mask.clone()))?,"maskN"=>Tensor::from_array(([1,notes],maskn.clone()))?,"threshold"=>Tensor::from_array(([0usize;0],vec![0.5f32]))?])?;
        let presence = results["presence"].try_extract_tensor::<bool>()?.1.to_vec();
        let pitch = results["scores"].try_extract_tensor::<f32>()?.1.to_vec();
        let confidence = results[confidence_name.as_str()]
            .try_extract_tensor::<f32>()?
            .1
            .to_vec();
        ensure!(
            presence.len() == notes
                && pitch.len() == notes
                && confidence.len() == notes
                && confidence
                    .iter()
                    .all(|v| v.is_finite() && (0.0..=1.).contains(v)),
            "Estimator output"
        );
        ensure!(
            presence
                .iter()
                .zip(&confidence)
                .all(|(p, c)| *p == (*c > 0.5)),
            "Instrumented confidence differs from presence"
        );
        drop(results);
        if core == 0 {
            let mut unmodified = session(&root.join("estimator.onnx"))?;
            let result=unmodified.run(ort::inputs!["x_est"=>Tensor::from_array((dims,est))?,"boundaries"=>Tensor::from_array(([1,t],boundaries.clone()))?,"maskT"=>Tensor::from_array(([1,t],mask))?,"maskN"=>Tensor::from_array(([1,notes],maskn))?,"threshold"=>Tensor::from_array(([0usize;0],vec![0.5f32]))?])?;
            ensure!(
                result["presence"].try_extract_tensor::<bool>()?.1 == presence
                    && result["scores"].try_extract_tensor::<f32>()?.1 == pitch,
                "Graph instrumentation changed original outputs"
            );
        }
        for (i, row) in rows.iter_mut().enumerate().take(end).skip(core) {
            let k = i - lo;
            let j = indices[k].context("Unmasked owned frame")?;
            *row = json!({"boundary":boundaries[k],"presence_confidence":confidence[j],"game_pitch":pitch[j]});
        }
        chunkpins.push(json!({"owned_frames":[core,end],"context_frames":[lo,hi],"notes":notes,"boundaries":boundaries}));
        eprintln!(
            "{} core {core}..{end}, {:.2}s",
            input.display(),
            starttime.elapsed().as_secs_f64()
        );
    }
    ensure!(
        rows.iter().all(|r| !r.is_null()) && chart_engine::models::hash_file(input)? == source,
        "Incomplete/changed source"
    );
    let value = json!({"status":"EXPERIMENTAL_CHUNKED_GAME_SMALL_NOT_UPSTREAM_PARITY_NOT_PROMOTED","source_sha256":source,"duration_seconds":duration,"timestep":0.01,"frame_kind":"interval_start","coverage_seconds":n as f64*0.01,"unmodeled_tail_seconds":duration-n as f64*0.01,"core_frames":2000,"context_frames":100,"steps":steps,"language":language,"boundary_threshold":0.2,"radius":2,"rows":rows,"chunks":chunkpins,"assets":assetpins,"patched_estimator_sha256":patched_hash,"instrumentation_original_outputs_equal_first_chunk":true,"implementation_sha256":format!("{:x}",Sha256::digest(include_bytes!("vocal-game-observations.rs"))),"executable_sha256":chart_engine::models::hash_file(&std::env::current_exe()?)?,"runtime_sha256":chart_engine::models::hash_file(Path::new(&runtime))?,"weight_terms":"CC-BY-NC-SA-4.0 private evaluation only","elapsed_seconds":starttime.elapsed().as_secs_f64()});
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(out)?
        .write_all(&serde_json::to_vec(&value)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protobuf_rejects_truncation_and_overflow() {
        assert!(fields(&[10, 5, 1]).is_err());
        assert!(fields(&[0x80]).is_err());
        assert!(fields(&[0x0e]).is_err());
        assert!(fields(&[8, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255]).is_err());
    }
    #[test]
    fn instrumentation_adds_only_a_typed_output() {
        let mut graph = vec![];
        for (ins, out, op) in [
            (vec!["voice_probability", "threshold"], "greater", "Greater"),
            (vec!["greater", "maskN"], "presence", "And"),
        ] {
            let mut node = vec![];
            for input in ins {
                push_field(&mut node, 1, input.as_bytes());
            }
            push_field(&mut node, 2, out.as_bytes());
            push_field(&mut node, 4, op.as_bytes());
            push_field(&mut graph, 1, &node);
        }
        let mut output = vec![];
        push_field(&mut output, 1, b"scores");
        push_field(&mut output, 2, &[10, 2, 8, 1]);
        push_field(&mut graph, 12, &output);
        let mut original = vec![];
        push_field(&mut original, 7, &graph);
        let frozen = original.clone();
        let (patched, name) = instrument_estimator(&original).unwrap();
        assert_eq!(name, "voice_probability");
        assert_eq!(original, frozen);
        let outer = fields(&patched).unwrap();
        let newgraph = outer.iter().find(|f| f.number == 7).unwrap().data;
        assert!(newgraph.starts_with(&graph));
        let outputs = fields(newgraph).unwrap();
        assert_eq!(outputs.iter().filter(|f| f.number == 12).count(), 2);
        let last = fields(outputs.last().unwrap().data).unwrap();
        assert_eq!(strings(&last, 1).unwrap(), ["voice_probability"]);
        assert_eq!(
            last.iter().find(|f| f.number == 2).unwrap().data,
            [10, 2, 8, 1]
        );
    }
}
