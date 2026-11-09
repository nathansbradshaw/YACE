//! Original tensor conversion, without executing framework code.
use crate::onnx::{self, Attr, Graph};
use anyhow::{Context, Result, ensure};
use chart_engine::models;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::Path};
const MODEL_SHA: &str = "72c9c038a093031cfbfd2f61bb878e3351720e4230ab2a35596df1e5dd969005";
fn implementation() -> String {
    let mut h = Sha256::new();
    h.update(include_bytes!("convert.rs"));
    h.update(include_bytes!("onnx.rs"));
    h.update(include_bytes!("../Cargo.lock"));
    format!("{:x}", h.finalize())
}
fn save(p: impl AsRef<Path>, v: &impl serde::Serialize) -> Result<()> {
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(p)?
        .write_all(&serde_json::to_vec_pretty(v)?)?;
    Ok(())
}
fn weight(f: &hdf5_pure::File, layer: &str, name: &str) -> Result<(Vec<i64>, Vec<f32>)> {
    let d = f.dataset(&format!("/model_weights/{layer}/{layer}/{name}:0"))?;
    let shape = d.shape()?.iter().map(|d| *d as i64).collect::<Vec<_>>();
    let v = d.read_f32()?;
    ensure!(
        v.len() == shape.iter().product::<i64>() as usize && v.iter().all(|v| v.is_finite()),
        "Invalid weight"
    );
    Ok((shape, v))
}
fn conv(g: &mut Graph, f: &hdf5_pure::File, x: &str, n: &str) -> Result<String> {
    let (s, v) = weight(f, n, "kernel")?;
    ensure!(s.len() == 4, "Conv weight");
    let (h, w, ci, co) = (s[0] as usize, s[1] as usize, s[2] as usize, s[3] as usize);
    let mut a = vec![0.; v.len()];
    for o in 0..co {
        for i in 0..ci {
            for y in 0..h {
                for xx in 0..w {
                    a[((o * ci + i) * h + y) * w + xx] = v[((y * w + xx) * ci + i) * co + o];
                }
            }
        }
    }
    let w = g.floats(
        &format!("{n}.kernel"),
        &[co as i64, ci as i64, h as i64, w as i64],
        &a,
    );
    Ok(g.node(
        "Conv",
        &[x, &w],
        vec![Attr::Is(
            "pads",
            &[h as i64 / 2, s[1] / 2, h as i64 / 2, s[1] / 2],
        )],
    ))
}
fn bn(g: &mut Graph, f: &hdf5_pure::File, x: &str, i: usize) -> Result<String> {
    let n = format!("batch_normalization_{i}");
    let mut a = vec![];
    for k in ["gamma", "beta", "moving_mean", "moving_variance"] {
        let (s, v) = weight(f, &n, k)?;
        a.push(g.floats(&format!("{n}.{k}"), &s, &v));
    }
    Ok(g.node(
        "BatchNormalization",
        &[x, &a[0], &a[1], &a[2], &a[3]],
        vec![Attr::F("epsilon", 0.001)],
    ))
}
fn leaky(g: &mut Graph, x: &str) -> String {
    g.node("LeakyRelu", &[x], vec![Attr::F("alpha", 0.01)])
}
fn pool(g: &mut Graph, x: &str, n: i64) -> String {
    g.node(
        "MaxPool",
        &[x],
        vec![
            Attr::Is("kernel_shape", &[1, n]),
            Attr::Is("strides", &[1, n]),
        ],
    )
}
fn block(g: &mut Graph, f: &hdf5_pure::File, x: &str, i: usize) -> Result<String> {
    let x = bn(g, f, x, (i - 2) * 2 + 2)?;
    let x = leaky(g, &x);
    let x = pool(g, &x, 4);
    let skip = conv(g, f, &x, &format!("conv{i}_1x1"))?;
    let x = conv(g, f, &x, &format!("conv{i}_1"))?;
    let x = bn(g, f, &x, (i - 2) * 2 + 3)?;
    let x = leaky(g, &x);
    let x = conv(g, f, &x, &format!("conv{i}_2"))?;
    Ok(g.node("Add", &[&skip, &x], vec![]))
}
fn flatten(g: &mut Graph, x: &str) -> String {
    let x = g.tr(x, &[0, 2, 3, 1]);
    g.reshape(&x, &[0, 31, 512])
}
fn lstm(g: &mut Graph, f: &hdf5_pure::File, x: &str, index: usize, h: usize) -> Result<String> {
    let n = format!("bidirectional_{index}");
    let (mut w, mut r, mut b) = (vec![], vec![], vec![]);
    for direction in ["forward", "backward"] {
        let p = format!("{direction}_lstm_{index}");
        let (ws, wv) = weight(f, &n, &format!("{p}/kernel"))?;
        let (rs, rv) = weight(f, &n, &format!("{p}/recurrent_kernel"))?;
        let (bs, bv) = weight(f, &n, &format!("{p}/bias"))?;
        ensure!(
            ws == [512, (4 * h) as i64]
                && rs == [h as i64, (4 * h) as i64]
                && bs == [(4 * h) as i64],
            "LSTM shapes"
        );
        // Keras gate order [i,f,c,o] -> ONNX [i,o,f,c]; zero recurrent bias.
        for gate in [0, 3, 1, 2] {
            for j in 0..h {
                for i in 0..512 {
                    w.push(wv[i * 4 * h + gate * h + j]);
                }
            }
        }
        for gate in [0, 3, 1, 2] {
            for j in 0..h {
                for i in 0..h {
                    r.push(rv[i * 4 * h + gate * h + j]);
                }
            }
        }
        for gate in [0, 3, 1, 2] {
            b.extend_from_slice(&bv[gate * h..(gate + 1) * h]);
        }
        b.resize(b.len() + 4 * h, 0.);
    }
    let w = g.floats(&format!("{n}.W"), &[2, (4 * h) as i64, 512], &w);
    let r = g.floats(&format!("{n}.R"), &[2, (4 * h) as i64, h as i64], &r);
    let b = g.floats(&format!("{n}.B"), &[2, (8 * h) as i64], &b);
    let x = g.tr(x, &[1, 0, 2]);
    let y = g.node(
        "LSTM",
        &[&x, &w, &r, &b],
        vec![
            Attr::I("hidden_size", h as i64),
            Attr::S("direction", "bidirectional"),
            Attr::Ss(
                "activations",
                &["HardSigmoid", "Tanh", "Tanh", "HardSigmoid", "Tanh", "Tanh"],
            ),
        ],
    );
    let y = g.tr(&y, &[2, 0, 1, 3]);
    Ok(g.reshape(&y, &[0, 31, (2 * h) as i64]))
}
fn dense(g: &mut Graph, f: &hdf5_pure::File, x: &str, i: usize) -> Result<String> {
    let n = format!("time_distributed_{i}");
    let (s, v) = weight(f, &n, "kernel")?;
    let w = g.floats(&format!("{n}.W"), &s, &v);
    let (s, v) = weight(f, &n, "bias")?;
    let b = g.floats(&format!("{n}.B"), &s, &v);
    let x = g.node("MatMul", &[x, &w], vec![]);
    let x = g.node("Add", &[&x, &b], vec![]);
    Ok(g.node("Softmax", &[&x], vec![Attr::I("axis", -1)]))
}
pub fn convert(file: &Path, output: &Path) -> Result<()> {
    ensure!(models::hash_file(file)? == MODEL_SHA, "Checkpoint pin");
    let f = hdf5_pure::File::open(file)?;
    let attribute = f
        .root()
        .attrs()?
        .into_iter()
        .find(|(name, _)| name == "model_config")
        .context("Original model config")?
        .1;
    let text = match attribute {
        hdf5_pure::AttrValue::String(v)
        | hdf5_pure::AttrValue::AsciiString(v)
        | hdf5_pure::AttrValue::VarLenAsciiString(v) => v,
        hdf5_pure::AttrValue::StringSized { value, .. }
        | hdf5_pure::AttrValue::AsciiStringSized { value, .. } => value,
        _ => anyhow::bail!("Unsupported original model config"),
    };
    let config: Value = serde_json::from_str(&text)?;
    for l in config["config"]["layers"].as_array().context("Layers")? {
        if l["class_name"] == "Bidirectional" {
            ensure!(
                l["config"]["layer"]["config"]["recurrent_activation"] == "hard_sigmoid"
                    && l["config"]["layer"]["config"]["activation"] == "tanh"
                    && l["config"]["merge_mode"] == "concat",
                "Recurrent config"
            );
        }
    }
    let mut g = Graph::default();
    g.inputs
        .push(onnx::info("features", &["B", "1", "31", "513"]));
    let a = conv(&mut g, &f, "features", "conv1_1")?;
    let a = bn(&mut g, &f, &a, 1)?;
    let a = leaky(&mut g, &a);
    let a = conv(&mut g, &f, &a, "conv1_2")?;
    let b = block(&mut g, &f, &a, 2)?;
    let c = block(&mut g, &f, &b, 3)?;
    let d = block(&mut g, &f, &c, 4)?;
    let d = bn(&mut g, &f, &d, 8)?;
    let d = leaky(&mut g, &d);
    let d = pool(&mut g, &d, 4);
    let p = flatten(&mut g, &d);
    let p = lstm(&mut g, &f, &p, 1, 256)?;
    let p = dense(&mut g, &f, &p, 1)?;
    let a = pool(&mut g, &a, 256);
    let b = pool(&mut g, &b, 64);
    let c = pool(&mut g, &c, 16);
    let v = g.node("Concat", &[&a, &b, &c, &d], vec![Attr::I("axis", 1)]);
    let v = conv(&mut g, &f, &v, "conv2d_1")?;
    let v = bn(&mut g, &f, &v, 9)?;
    let v = leaky(&mut g, &v);
    let v = flatten(&mut g, &v);
    let v = lstm(&mut g, &f, &v, 2, 32)?;
    let v = dense(&mut g, &f, &v, 2)?;
    let zero = g.ints("pitch_zero", &[0]);
    let ns = g.node("Gather", &[&p, &zero], vec![Attr::I("axis", 2)]);
    let one = g.floats("one", &[], &[1.]);
    let s = g.node("Sub", &[&one, &ns], vec![]);
    let vv = g.node("Concat", &[&ns, &s], vec![Attr::I("axis", 2)]);
    let v = g.node("Add", &[&v, &vv], vec![]);
    let v = g.node("Softmax", &[&v], vec![Attr::I("axis", -1)]);
    g.outputs.push(onnx::info(&p, &["B", "31", "722"]));
    g.outputs.push(onnx::info(&v, &["B", "31", "2"]));
    let bytes = g.finish();
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == chart_engine::vocal_singing::MODEL_SHA256,
        "Converted graph differs from frozen graph"
    );
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)?
        .write_all(&bytes)?;
    save(
        output.with_extension("conversion.json"),
        &json!({"publisher_revision":"86e6d2b225c441cf5641ca74c342348646ff1bdc","original_checkpoint_sha256":MODEL_SHA,"onnx_sha256":models::hash_file(output)?,"implementation_sha256":implementation(),"trained_weights_changed":false,"publisher_numeric_parity_established":false,"recipe":"NHWC/NCHW layout; Keras gate order and hard sigmoid; original two heads; inference dropout disabled"}),
    )?;
    Ok(())
}
