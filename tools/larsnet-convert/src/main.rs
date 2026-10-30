mod onnx;
use anyhow::{Context, Result, ensure};
use candle_core::pickle::{Object, Stack, TensorInfo};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{BufReader, Cursor, Read, Write},
    path::Path,
    time::Instant,
};

const REVISION: &str = "872991160e7c26b1d9be76ddc625d01373727200";
const MODELS: [(&str, &str); 5] = [
    (
        "kick",
        "15d40aa9760e67305a31be4053a87e2b208ee4c8718357691e3b49f80aff0e46",
    ),
    (
        "snare",
        "166eaa52d05ccdb3842f3bf79d3afefa354757d366db0438f9128df75575a6f8",
    ),
    (
        "toms",
        "1eddcd147b1d771e45c3a443a94f332e2a1ecc5854cb11aa93459cecb1c63ca2",
    ),
    (
        "hihat",
        "1b5d46d05841fb663a77aa8b037847f4f158e916806177ff1a5443724b3f73f3",
    ),
    (
        "cymbals",
        "40b2dc81213729148188806a89abde53e3a90aee909f1a0cfbe1efbf4cf98b89",
    ),
];
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn collect(
    value: Object,
    prefix: &str,
    directory: &Path,
    out: &mut BTreeMap<String, TensorInfo>,
) -> Result<()> {
    match value {
        Object::Build { args, .. } => collect(*args, prefix, directory, out)?,
        Object::Dict(values) => {
            for (key, value) in values {
                let Object::Unicode(key) = key else { continue };
                let name = if prefix.is_empty() {
                    key
                } else {
                    format!("{prefix}.{key}")
                };
                if let Some(info) = value
                    .clone()
                    .into_tensor_info(Object::Unicode(name.clone()), directory)?
                {
                    ensure!(
                        out.insert(name.clone(), info).is_none(),
                        "Duplicate tensor {name}"
                    );
                } else {
                    collect(value, &name, directory, out)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 3,
        "Usage: larsnet-convert kick|snare|toms|hihat|cymbals INPUT.pt NEW_OUTPUT.onnx (local CC-BY-NC4.0 weights; no download)"
    );
    let part = args[0].to_str().context("Invalid part")?;
    let expected = MODELS
        .iter()
        .find(|m| m.0 == part)
        .context("Unknown LarsNet part")?
        .1;
    let input = Path::new(&args[1]);
    let output = Path::new(&args[2]);
    let receipt_path = output.with_extension("onnx.receipt.json");
    ensure!(
        !output.exists() && !receipt_path.exists(),
        "Output/receipt already exists"
    );
    let mut bytes = Vec::new();
    File::open(input)?
        .take(40_000_001)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() < 40_000_000 && hash(&bytes) == expected,
        "Pinned official LARS checkpoint mismatch"
    );
    // Parse exactly the checkpoint bytes that were hashed, even if the path
    // changes concurrently. Pickle is decoded as data, never executed.
    ensure!(
        bytes.starts_with(b"PK\x03\x04"),
        "Pinned checkpoint is not a byte-zero ZIP archive"
    );
    // These checksum-pinned archives start at byte zero. ZIP 9's offset
    // autodetection misidentifies the central directory in two checkpoints.
    let mut zip = zip::ZipArchive::with_config(
        zip::read::Config {
            archive_offset: zip::read::ArchiveOffset::Known(0),
        },
        Cursor::new(&bytes),
    )?;
    let names = zip
        .file_names()
        .map(|name| name.map(|name| name.into_owned()))
        .collect::<zip::result::ZipResult<Vec<_>>>()?;
    let paths: Vec<_> = names
        .into_iter()
        .filter(|s| s.ends_with("/data.pkl"))
        .collect();
    ensure!(paths.len() == 1, "Expected one data.pkl");
    let mut stack = Stack::empty();
    stack.read_loop(&mut BufReader::new(zip.by_name(&paths[0])?))?;
    let mut tensors = BTreeMap::new();
    collect(
        stack.finalize()?,
        "",
        Path::new(paths[0].strip_suffix(".pkl").unwrap()),
        &mut tensors,
    )?;
    ensure!(tensors.len() == 91, "Unexpected model architecture");
    let mut constants = Vec::new();
    for (name, info) in tensors {
        if name.ends_with("num_batches_tracked") || name.starts_with("enc6.bn.") {
            continue;
        }
        ensure!(
            info.dtype == candle_core::DType::F32
                && info.layout.is_contiguous()
                && info.layout.start_offset() == 0,
            "Unsupported tensor layout/type: {name}"
        );
        let mut raw = Vec::new();
        zip.by_name(&info.path)?.read_to_end(&mut raw)?;
        ensure!(
            raw.len() == info.layout.dims().iter().product::<usize>() * 4,
            "Tensor storage differs: {name}"
        );
        ensure!(
            raw.chunks_exact(4)
                .all(|v| f32::from_le_bytes(v.try_into().unwrap()).is_finite()),
            "Nonfinite tensor {name}"
        );
        constants.push((name, info.layout.dims().to_vec(), raw));
    }
    let graph = onnx::build(&constants);
    let start = Instant::now();
    let mut session = ort::session::Session::builder()?
        .with_intra_threads(2)?
        .commit_from_memory(&graph)?;
    let values: Vec<f32> = (0..2 * 2048 * 512)
        .map(|i| if i % 71 == 0 { 1. } else { 0.001 })
        .collect();
    let input = ort::value::Tensor::from_array(([1, 2, 2048, 512], values.into_boxed_slice()))?;
    let prediction = session.run(ort::inputs!["magnitude"=>input])?;
    let (shape, mask) = prediction["mask"].try_extract_tensor::<f32>()?;
    ensure!(shape.as_ref() == [1, 2, 2048, 512], "Unexpected mask shape");
    ensure!(
        mask.iter().all(|v| v.is_finite() && *v >= 0. && *v <= 1.),
        "Invalid sigmoid masks"
    );
    let stats = serde_json::json!({"min":mask.iter().copied().fold(f32::INFINITY,f32::min),"max":mask.iter().copied().fold(f32::NEG_INFINITY,f32::max),"mean":mask.iter().map(|v|*v as f64).sum::<f64>()/mask.len()as f64});
    let seconds = start.elapsed().as_secs_f64();
    let receipt = serde_json::json!({"version":1,"part":part,"source_revision":REVISION,"source_url":format!("https://raw.githubusercontent.com/EdoardoMor/LARS/{REVISION}/drums_demix/DrumsDemixUtils/DrumsDemixModels/my_scripted_module_{part}.pt"),"source_sha256":expected,"source_bytes":bytes.len(),"weight_license":"CC-BY-NC-4.0 (official LARS/LarsNet README); conversion does not relicense weights", "weight_license_source":"https://github.com/EdoardoMor/LARS/blob/872991160e7c26b1d9be76ddc625d01373727200/README.md#license","converter_license":"MIT, independent Rust implementation of the published architecture","architecture_source":"https://github.com/EdoardoMor/LARS/blob/872991160e7c26b1d9be76ddc625d01373727200/drums_demix/net/unet.py","converter_source_sha256":hash(concat!(include_str!("main.rs"),include_str!("onnx.rs"),include_str!("../Cargo.toml"),include_str!("../Cargo.lock")).as_bytes()),"onnx_sha256":hash(&graph),"onnx_bytes":graph.len(),"onnx_opset":17,"input":"magnitude float32 [1,2,2048,512]","output":"mask float32 [1,2,2048,512]","external_stft":"44.1kHz stereo;4096 periodic Hann;hop1024;center reflection;zero-pad final frame blocks;Nyquist mask0;no rescaling","smoke_seconds":seconds,"smoke_mask":stats,"limits":"Native graph shape/finiteness/range smoke, not upstream LibTorch numerical parity or separation quality acceptance. No weights bundled or downloaded by this tool."});
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?
        .write_all(&graph)?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&receipt_path)?
        .write_all(&serde_json::to_vec_pretty(&receipt)?)?;
    println!("{}", serde_json::to_string_pretty(&receipt)?);
    Ok(())
}
