//! Minimal ONNX IR 9 / opset 17 writer. Field numbers follow ONNX v1.16.2.
//! No executable deserialization or framework runtime is involved.
use crate::checkpoint::Checkpoint;
use anyhow::{Result, ensure};
use std::collections::BTreeSet;
fn var(mut n: u64, o: &mut Vec<u8>) {
    while n > 127 {
        o.push((n as u8 & 127) | 128);
        n >>= 7
    }
    o.push(n as u8)
}
fn vi(f: u32, n: i64, o: &mut Vec<u8>) {
    var((f as u64) << 3, o);
    var(n as u64, o)
}
fn bytes(f: u32, b: &[u8], o: &mut Vec<u8>) {
    var(((f as u64) << 3) | 2, o);
    var(b.len() as u64, o);
    o.extend(b)
}
fn txt(f: u32, s: &str, o: &mut Vec<u8>) {
    bytes(f, s.as_bytes(), o)
}
pub enum Attr<'a> {
    I(&'a str, i64),
    F(&'a str, f32),
    Is(&'a str, &'a [i64]),
}
fn attr(a: Attr<'_>) -> Vec<u8> {
    let mut b = vec![];
    match a {
        Attr::I(n, v) => {
            txt(1, n, &mut b);
            vi(3, v, &mut b);
            vi(20, 2, &mut b)
        }
        Attr::F(n, v) => {
            txt(1, n, &mut b);
            var((2 << 3) | 5, &mut b);
            b.extend(v.to_le_bytes());
            vi(20, 1, &mut b)
        }
        Attr::Is(n, v) => {
            txt(1, n, &mut b);
            for x in v {
                vi(8, *x, &mut b)
            }
            vi(20, 7, &mut b)
        }
    }
    b
}
fn tensor(n: &str, shape: &[i64], dtype: i64, raw: &[u8]) -> Vec<u8> {
    let mut b = vec![];
    for d in shape {
        vi(1, *d, &mut b)
    }
    vi(2, dtype, &mut b);
    txt(8, n, &mut b);
    bytes(9, raw, &mut b);
    b
}
pub fn info(n: &str, dims: &[&str]) -> Vec<u8> {
    let mut sh = vec![];
    for d in dims {
        let mut dm = vec![];
        if let Ok(d) = d.parse() {
            vi(1, d, &mut dm)
        } else {
            txt(2, d, &mut dm)
        }
        bytes(1, &dm, &mut sh)
    }
    let mut tt = vec![];
    vi(1, 1, &mut tt);
    bytes(2, &sh, &mut tt);
    let mut tp = vec![];
    bytes(1, &tt, &mut tp);
    let mut b = vec![];
    txt(1, n, &mut b);
    bytes(2, &tp, &mut b);
    b
}
pub struct Graph<'a> {
    pub ck: &'a Checkpoint,
    nodes: Vec<Vec<u8>>,
    init: Vec<Vec<u8>>,
    loaded: BTreeSet<String>,
    counter: usize,
    pub inputs: Vec<Vec<u8>>,
    pub outputs: Vec<Vec<u8>>,
}
impl<'a> Graph<'a> {
    pub fn new(ck: &'a Checkpoint) -> Self {
        Self {
            ck,
            nodes: vec![],
            init: vec![],
            loaded: BTreeSet::new(),
            counter: 0,
            inputs: vec![],
            outputs: vec![],
        }
    }
    pub fn node(&mut self, op: &str, ins: &[&str], attrs: Vec<Attr<'_>>) -> String {
        self.counter += 1;
        let out = format!("v{}", self.counter);
        let mut b = vec![];
        for i in ins {
            txt(1, i, &mut b)
        }
        txt(2, &out, &mut b);
        txt(3, &out, &mut b);
        txt(4, op, &mut b);
        for a in attrs {
            bytes(5, &attr(a), &mut b)
        }
        self.nodes.push(b);
        out
    }
    pub fn un(&mut self, op: &str, x: &str) -> String {
        self.node(op, &[x], vec![])
    }
    pub fn bin(&mut self, op: &str, x: &str, y: &str) -> String {
        self.node(op, &[x, y], vec![])
    }
    pub fn tr(&mut self, x: &str, perm: &[i64]) -> String {
        self.node("Transpose", &[x], vec![Attr::Is("perm", perm)])
    }
    pub fn ints(&mut self, n: &str, shape: &[i64], v: &[i64]) -> String {
        if self.loaded.insert(n.into()) {
            let raw: Vec<_> = v.iter().flat_map(|n| n.to_le_bytes()).collect();
            self.init.push(tensor(n, shape, 7, &raw))
        }
        n.into()
    }
    pub fn floats(&mut self, n: &str, shape: &[i64], v: &[f32]) -> String {
        if self.loaded.insert(n.into()) {
            let raw: Vec<_> = v.iter().flat_map(|n| n.to_le_bytes()).collect();
            self.init.push(tensor(n, shape, 1, &raw))
        }
        n.into()
    }
    pub fn scalar(&mut self, v: f32) -> String {
        self.floats(&format!("const_{:08x}", v.to_bits()), &[], &[v])
    }
    pub fn scale(&mut self, x: &str, v: f32) -> String {
        let c = self.scalar(v);
        self.bin("Mul", x, &c)
    }
    pub fn weight(&mut self, n: &str) -> Result<String> {
        let name = format!("model.{n}");
        if self.loaded.insert(name.clone()) {
            let t = self.ck.get(&name)?;
            ensure!(
                t.dtype == candle_core::DType::F32,
                "Expected float weight {name}"
            );
            self.init.push(tensor(&name, &t.dims, 1, &t.raw))
        }
        Ok(name)
    }
    pub fn conv(
        &mut self,
        x: &str,
        p: &str,
        k: i64,
        transpose: bool,
        group: i64,
    ) -> Result<String> {
        let w = self.weight(&format!("{p}.weight"))?;
        let b = self.weight(&format!("{p}.bias"))?;
        let out = if transpose {
            self.node(
                "ConvTranspose",
                &[x, &w, &b],
                vec![
                    Attr::Is("kernel_shape", &[k]),
                    Attr::Is("pads", &[k / 2, k / 2]),
                    Attr::Is("strides", &[2]),
                    Attr::Is("output_padding", &[1]),
                ],
            )
        } else {
            self.node(
                "Conv",
                &[x, &w, &b],
                vec![
                    Attr::Is("kernel_shape", &[k]),
                    Attr::Is("pads", &[k / 2, k / 2]),
                    Attr::I("group", group),
                ],
            )
        };
        Ok(out)
    }
    pub fn linear(&mut self, x: &str, p: &str, bias: bool) -> Result<String> {
        let w = self.weight(&format!("{p}.weight"))?;
        let wt = self.tr(&w, &[1, 0]);
        let mut y = self.bin("MatMul", x, &wt);
        if bias {
            let b = self.weight(&format!("{p}.bias"))?;
            y = self.bin("Add", &y, &b)
        }
        Ok(y)
    }
    pub fn ln(&mut self, x: &str, p: &str, eps: f32, bct: bool) -> Result<String> {
        let w = self.weight(&format!("{p}.weight"))?;
        let b = self.weight(&format!("{p}.bias"))?;
        let x = if bct {
            self.tr(x, &[0, 2, 1])
        } else {
            x.into()
        };
        let y = self.node(
            "LayerNormalization",
            &[&x, &w, &b],
            vec![Attr::I("axis", -1), Attr::F("epsilon", eps)],
        );
        Ok(if bct { self.tr(&y, &[0, 2, 1]) } else { y })
    }
    pub fn reshape(&mut self, x: &str, shape: &[i64]) -> String {
        let n = format!(
            "shape_{}",
            shape
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join("_")
        );
        let s = self.ints(&n, &[shape.len() as i64], shape);
        self.bin("Reshape", x, &s)
    }
    pub fn slice(&mut self, x: &str, start: i64, end: i64, axis: i64) -> String {
        let s = self.ints(&format!("slice_s{start}"), &[1], &[start]);
        let e = self.ints(&format!("slice_e{end}"), &[1], &[end]);
        let a = self.ints(&format!("slice_a{axis}"), &[1], &[axis]);
        self.node("Slice", &[x, &s, &e, &a], vec![])
    }
    pub fn finish(self) -> Vec<u8> {
        let mut g = vec![];
        for n in self.nodes {
            bytes(1, &n, &mut g)
        }
        txt(2, "yace-rosvot-research", &mut g);
        for t in self.init {
            bytes(5, &t, &mut g)
        }
        for i in self.inputs {
            bytes(11, &i, &mut g)
        }
        for o in self.outputs {
            bytes(12, &o, &mut g)
        }
        let mut m = vec![];
        vi(1, 9, &mut m);
        txt(2, "YACE Rust ROSVOT research converter", &mut m);
        vi(5, 1, &mut m);
        bytes(7, &g, &mut m);
        let mut ops = vec![];
        vi(2, 17, &mut ops);
        bytes(8, &ops, &mut m);
        m
    }
}
