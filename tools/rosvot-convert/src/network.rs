//! Inference graph translation of ROSVOT 3c8332bf (MIT, Ruiqi Li 2024).
//! Relative attention follows ESPnet's legacy implementation (Apache-2.0,
//! Shigeki Karita 2019). See THIRD_PARTY.md before redistribution.
use crate::onnx::{Attr, Graph, info};
use anyhow::Result;
fn mask(g: &mut Graph<'_>, x: &str, axis: i64) -> String {
    let a = g.un("Abs", x);
    let ax = g.ints(&format!("axis_{axis}"), &[1], &[axis]);
    let a = g.node("ReduceSum", &[&a, &ax], vec![Attr::I("keepdims", 1)]);
    let z = g.scalar(0.);
    let a = g.bin("Greater", &a, &z);
    g.node("Cast", &[&a], vec![Attr::I("to", 1)])
}
fn residual(g: &mut Graph<'_>, x: &str, p: &str, n: usize, eps: f32) -> Result<String> {
    let m = mask(g, x, 1);
    let mut x = x.to_owned();
    for i in 0..n {
        let p = format!("{p}.blocks.{i}");
        let y = g.ln(&x, &format!("{p}.0"), eps, true)?;
        let y = g.conv(&y, &format!("{p}.1"), 3, false, 1)?;
        let y = g.scale(&y, 1. / 3f32.sqrt());
        let y = g.node("LeakyRelu", &[&y], vec![Attr::F("alpha", 0.01)]);
        let y = g.conv(&y, &format!("{p}.4"), 1, false, 1)?;
        let y = g.bin("Add", &x, &y);
        x = g.bin("Mul", &y, &m)
    }
    Ok(x)
}
fn blocks(g: &mut Graph<'_>, x: &str, p: &str, n: usize) -> Result<String> {
    let m = mask(g, x, 1);
    let y = residual(g, x, &format!("{p}.res_blocks.0"), n, 1e-5)?;
    let y = g.bin("Mul", &y, &m);
    let y = g.ln(&y, &format!("{p}.last_norm"), 1e-5, true)?;
    let y = g.bin("Mul", &y, &m);
    let y = g.conv(&y, &format!("{p}.post_net1"), 3, false, 1)?;
    Ok(g.bin("Mul", &y, &m))
}
fn ff(g: &mut Graph<'_>, x: &str, p: &str) -> Result<String> {
    let y = g.tr(x, &[0, 2, 1]);
    let y = g.conv(&y, &format!("{p}.w_1"), 1, false, 1)?;
    let y = g.un("Relu", &y);
    let y = g.conv(&y, &format!("{p}.w_2"), 1, false, 1)?;
    Ok(g.tr(&y, &[0, 2, 1]))
}
fn conformer_conv(g: &mut Graph<'_>, x: &str, p: &str) -> Result<String> {
    let y = g.tr(x, &[0, 2, 1]);
    let y = g.conv(&y, &format!("{p}.pointwise_conv1"), 1, false, 1)?;
    let a = g.slice(&y, 0, 256, 1);
    let b = g.slice(&y, 256, 512, 1);
    let b = g.un("Sigmoid", &b);
    let y = g.bin("Mul", &a, &b);
    let y = g.conv(&y, &format!("{p}.depthwise_conv"), 9, false, 256)?;
    let w = g.weight(&format!("{p}.norm.weight"))?;
    let b = g.weight(&format!("{p}.norm.bias"))?;
    let mean = g.weight(&format!("{p}.norm.running_mean"))?;
    let var = g.weight(&format!("{p}.norm.running_var"))?;
    let y = g.node(
        "BatchNormalization",
        &[&y, &w, &b, &mean, &var],
        vec![Attr::F("epsilon", 1e-5)],
    );
    let s = g.un("Sigmoid", &y);
    let y = g.bin("Mul", &y, &s);
    let y = g.conv(&y, &format!("{p}.pointwise_conv2"), 1, false, 1)?;
    Ok(g.tr(&y, &[0, 2, 1]))
}
fn attention(g: &mut Graph<'_>, x: &str, pos: &str, nonpadding: &str, p: &str) -> Result<String> {
    let q = g.linear(x, &format!("{p}.linear_q"), true)?;
    let q = g.reshape(&q, &[1, -1, 4, 64]);
    let q = g.tr(&q, &[0, 2, 1, 3]);
    let k = g.linear(x, &format!("{p}.linear_k"), true)?;
    let k = g.reshape(&k, &[1, -1, 4, 64]);
    let k = g.tr(&k, &[0, 2, 3, 1]);
    let v = g.linear(x, &format!("{p}.linear_v"), true)?;
    let v = g.reshape(&v, &[1, -1, 4, 64]);
    let v = g.tr(&v, &[0, 2, 1, 3]);
    let pos = g.linear(pos, &format!("{p}.linear_pos"), false)?;
    let pos = g.reshape(&pos, &[1, -1, 4, 64]);
    let pos = g.tr(&pos, &[0, 2, 3, 1]);
    let u = g.weight(&format!("{p}.pos_bias_u"))?;
    let u = g.reshape(&u, &[1, 4, 1, 64]);
    let u = g.bin("Add", &q, &u);
    let ac = g.bin("MatMul", &u, &k);
    let b = g.weight(&format!("{p}.pos_bias_v"))?;
    let b = g.reshape(&b, &[1, 4, 1, 64]);
    let b = g.bin("Add", &q, &b);
    let bd = g.bin("MatMul", &b, &pos);
    // Legacy ESPnet rel_shift: zero-pad last axis, reshape S+1,S, drop first row.
    let shape = g.un("Shape", &bd);
    let ix = g.ints("index2", &[1], &[2]);
    let s = g.node("Gather", &[&shape, &ix], vec![Attr::I("axis", 0)]);
    let one = g.ints("onevec", &[1], &[1]);
    let sp = g.bin("Add", &s, &one);
    let one4 = g.ints("one4", &[2], &[1, 4]);
    let paddedshape = g.node("Concat", &[&one4, &sp, &s], vec![Attr::I("axis", 0)]);
    let pads = g.ints("pad_last_one", &[8], &[0, 0, 0, 1, 0, 0, 0, 0]); // prepend a column
    let padded = g.node("Pad", &[&bd, &pads], vec![]);
    let padded = g.bin("Reshape", &padded, &paddedshape);
    let shifted = g.slice(&padded, 1, i64::MAX, 2);
    let shifted = g.bin("Reshape", &shifted, &shape);
    let scores = g.bin("Add", &ac, &shifted);
    let scores = g.scale(&scores, 0.125);
    let m = g.tr(nonpadding, &[0, 2, 1]);
    let axes = g.ints("axis1", &[1], &[1]);
    let m = g.node("Unsqueeze", &[&m, &axes], vec![]);
    let zero = g.scalar(0.);
    let m = g.bin("Greater", &m, &zero);
    let low = g.scalar(f32::MIN);
    let scores = g.node("Where", &[&m, &scores, &low], vec![]);
    let probs = g.node("Softmax", &[&scores], vec![Attr::I("axis", -1)]);
    let probs = g.node("Where", &[&m, &probs, &zero], vec![]);
    let y = g.bin("MatMul", &probs, &v);
    let y = g.tr(&y, &[0, 2, 1, 3]);
    let y = g.reshape(&y, &[1, -1, 256]);
    g.linear(&y, &format!("{p}.linear_out"), true)
}
fn conformer(g: &mut Graph<'_>, x: &str, p: &str) -> Result<String> {
    let nonpadding = mask(g, x, 2);
    let mut x = g.scale(x, 16.);
    let mut pe = Vec::with_capacity(5000 * 256);
    for i in 0..5000 {
        for j in 0..128 {
            let div = (-(10000f32).ln() * (2 * j) as f32 / 256.).exp();
            let angle = (4999 - i) as f32 * div;
            pe.push(angle.sin());
            pe.push(angle.cos())
        }
    }
    let pos = g.floats("legacy_reverse_pe_5000", &[1, 5000, 256], &pe);
    let shape = g.un("Shape", &x);
    let ix = g.ints("index1", &[1], &[1]);
    let len = g.node("Gather", &[&shape, &ix], vec![Attr::I("axis", 0)]);
    let start = g.ints("zero_vec", &[1], &[0]);
    let axis = g.ints("axis1", &[1], &[1]);
    let pos = g.node("Slice", &[&pos, &start, &len, &axis], vec![]);
    for i in 0..2 {
        let lp = format!("{p}.encoder_layers.{i}");
        let y = g.ln(&x, &format!("{lp}.norm_ff_macaron"), 1e-5, false)?;
        let y = ff(g, &y, &format!("{lp}.feed_forward_macaron"))?;
        let y = g.scale(&y, 0.5);
        x = g.bin("Add", &x, &y);
        let y = g.ln(&x, &format!("{lp}.norm_mha"), 1e-5, false)?;
        let y = attention(g, &y, &pos, &nonpadding, &format!("{lp}.self_attn"))?;
        x = g.bin("Add", &x, &y);
        let y = g.ln(&x, &format!("{lp}.norm_conv"), 1e-5, false)?;
        let y = conformer_conv(g, &y, &format!("{lp}.conv_module"))?;
        x = g.bin("Add", &x, &y);
        let y = g.ln(&x, &format!("{lp}.norm_ff"), 1e-5, false)?;
        let y = ff(g, &y, &format!("{lp}.feed_forward"))?;
        let y = g.scale(&y, 0.5);
        x = g.bin("Add", &x, &y);
        x = g.ln(&x, &format!("{lp}.norm_final"), 1e-5, false)?;
    }
    let x = g.ln(&x, &format!("{p}.layer_norm"), 1e-5, false)?;
    Ok(g.bin("Mul", &x, &nonpadding))
}
fn unet(g: &mut Graph<'_>, x: &str, stages: usize) -> Result<String> {
    let p = "net.net";
    let mut x = x.to_owned();
    let mut skips = vec![];
    for i in 0..stages {
        let lp = format!("{p}.down.layers.{i}");
        x = residual(g, &x, &format!("{lp}.0"), 1, 1e-5)?;
        x = g.conv(&x, &format!("{lp}.1"), 3, false, 1)?;
        x = residual(g, &x, &format!("{lp}.2"), 1, 1e-5)?;
        skips.push(x.clone());
        x = g.node(
            "AveragePool",
            &[&x],
            vec![Attr::Is("kernel_shape", &[2]), Attr::Is("strides", &[2])],
        );
    }
    x = g.ln(&x, &format!("{p}.down.last_norm"), 1e-6, true)?;
    x = g.conv(&x, &format!("{p}.down.post_net"), 3, false, 1)?;
    x = g.conv(&x, &format!("{p}.mid.pre"), 3, false, 1)?;
    x = g.tr(&x, &[0, 2, 1]);
    x = conformer(g, &x, &format!("{p}.mid.net"))?;
    x = g.tr(&x, &[0, 2, 1]);
    x = g.conv(&x, &format!("{p}.mid.post"), 3, false, 1)?;
    for (i, skip) in skips.iter().rev().enumerate() {
        x = g.conv(&x, &format!("{p}.up.ups.{i}.0"), 3, true, 1)?;
        x = g.ln(&x, &format!("{p}.up.ups.{i}.1"), 1e-6, true)?;
        x = g.node("LeakyRelu", &[&x], vec![Attr::F("alpha", 0.01)]);
        x = g.node("Concat", &[&x, skip], vec![Attr::I("axis", 1)]);
        x = g.conv(&x, &format!("{p}.up.layers.{i}.0"), 3, false, 1)?;
        x = residual(g, &x, &format!("{p}.up.layers.{i}.1"), 1, 1e-5)?;
    }
    x = g.ln(&x, &format!("{p}.up.last_norm"), 1e-6, true)?;
    x = g.conv(&x, &format!("{p}.up.post_net"), 3, false, 1)?;
    Ok(g.tr(&x, &[0, 2, 1]))
}
fn embed(g: &mut Graph<'_>, x: &str, p: &str) -> Result<String> {
    let x = g.node("Cast", &[x], vec![Attr::I("to", 7)]);
    let w = g.weight(&format!("{p}.weight"))?;
    let x = g.node("Gather", &[&w, &x], vec![Attr::I("axis", 0)]);
    Ok(g.tr(&x, &[0, 2, 1]))
}
pub fn encoder(g: &mut Graph<'_>, word: bool) -> Result<()> {
    g.inputs.push(info("mel", &["1", "T", "40"]));
    g.inputs.push(info("pitch", &["1", "T"]));
    g.inputs.push(info("uv", &["1", "T"]));
    if word {
        g.inputs.push(info("word_bd", &["1", "T"]))
    }
    let x = g.tr("mel", &[0, 2, 1]);
    let x = g.conv(&x, "mel_proj", 3, false, 1)?;
    let x = blocks(g, &x, "mel_encoder", 2)?;
    let ep = embed(g, "pitch", "pitch_embed")?;
    let eu = embed(g, "uv", "uv_embed")?;
    let y = g.bin("Add", &ep, &eu);
    let mut x = g.bin("Add", &x, &y);
    if word {
        let w = embed(g, "word_bd", "word_bd_embed")?;
        x = g.bin("Add", &x, &w)
    }
    let x = blocks(g, &x, "cond_encoder", 1)?;
    let x = unet(g, &x, if word { 4 } else { 3 })?;
    let bd = g.linear(&x, if word { "note_bd_out" } else { "word_bd_out" }, true)?;
    let bd = g.scale(&bd, 5.);
    let lo = g.scalar(-16.);
    let hi = g.scalar(16.);
    let bd = g.node("Clip", &[&bd, &lo, &hi], vec![]);
    g.outputs.push(info(&bd, &["1", "T", "1"]));
    if word {
        let attn = g.linear(&x, "pitch_decoder.multihead_dot_attn", true)?;
        g.outputs.push(info(&x, &["1", "T", "256"]));
        g.outputs.push(info(&attn, &["1", "T", "4"]));
    }
    Ok(())
}
pub fn decoder(g: &mut Graph<'_>) -> Result<()> {
    g.inputs.push(info("notes", &["1", "N", "256"]));
    let x = g.tr("notes", &[0, 2, 1]);
    let x = blocks(g, &x, "pitch_decoder.post", 1)?;
    let x = g.tr(&x, &[0, 2, 1]);
    let x = g.linear(&x, "pitch_decoder.pitch_out", true)?;
    g.outputs.push(info(&x, &["1", "N", "89"]));
    Ok(())
}
