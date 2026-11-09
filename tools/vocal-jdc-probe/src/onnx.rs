//! Small ONNX serializer for the private JDC representation. IR9/opset17.
use std::collections::BTreeSet;
fn var(mut n: u64, o: &mut Vec<u8>) {
    while n > 127 {
        o.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    o.push(n as u8);
}
fn vi(f: u32, n: i64, o: &mut Vec<u8>) {
    var((f as u64) << 3, o);
    var(n as u64, o);
}
fn bytes(f: u32, b: &[u8], o: &mut Vec<u8>) {
    var(((f as u64) << 3) | 2, o);
    var(b.len() as u64, o);
    o.extend(b);
}
fn txt(f: u32, s: &str, o: &mut Vec<u8>) {
    bytes(f, s.as_bytes(), o);
}
pub enum Attr<'a> {
    I(&'a str, i64),
    F(&'a str, f32),
    Is(&'a str, &'a [i64]),
    S(&'a str, &'a str),
    Ss(&'a str, &'a [&'a str]),
}
fn attr(a: Attr<'_>) -> Vec<u8> {
    let mut b = vec![];
    match a {
        Attr::I(n, v) => {
            txt(1, n, &mut b);
            vi(3, v, &mut b);
            vi(20, 2, &mut b);
        }
        Attr::F(n, v) => {
            txt(1, n, &mut b);
            var((2 << 3) | 5, &mut b);
            b.extend(v.to_le_bytes());
            vi(20, 1, &mut b);
        }
        Attr::Is(n, v) => {
            txt(1, n, &mut b);
            for x in v {
                vi(8, *x, &mut b);
            }
            vi(20, 7, &mut b);
        }
        Attr::S(n, v) => {
            txt(1, n, &mut b);
            txt(4, v, &mut b);
            vi(20, 3, &mut b);
        }
        Attr::Ss(n, v) => {
            txt(1, n, &mut b);
            for x in v {
                txt(9, x, &mut b);
            }
            vi(20, 8, &mut b);
        }
    }
    b
}
fn tensor(n: &str, shape: &[i64], dtype: i64, raw: &[u8]) -> Vec<u8> {
    let mut b = vec![];
    for d in shape {
        vi(1, *d, &mut b);
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
            vi(1, d, &mut dm);
        } else {
            txt(2, d, &mut dm);
        }
        bytes(1, &dm, &mut sh);
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
#[derive(Default)]
pub struct Graph {
    nodes: Vec<Vec<u8>>,
    init: Vec<Vec<u8>>,
    loaded: BTreeSet<String>,
    counter: usize,
    pub inputs: Vec<Vec<u8>>,
    pub outputs: Vec<Vec<u8>>,
}
impl Graph {
    pub fn node(&mut self, op: &str, ins: &[&str], attrs: Vec<Attr<'_>>) -> String {
        self.counter += 1;
        let out = format!("v{}", self.counter);
        let mut b = vec![];
        for i in ins {
            txt(1, i, &mut b);
        }
        txt(2, &out, &mut b);
        txt(3, &out, &mut b);
        txt(4, op, &mut b);
        for a in attrs {
            bytes(5, &attr(a), &mut b);
        }
        self.nodes.push(b);
        out
    }
    pub fn tr(&mut self, x: &str, p: &[i64]) -> String {
        self.node("Transpose", &[x], vec![Attr::Is("perm", p)])
    }
    pub fn floats(&mut self, n: &str, shape: &[i64], v: &[f32]) -> String {
        assert_eq!(shape.iter().product::<i64>() as usize, v.len());
        assert!(self.loaded.insert(n.into()));
        let raw: Vec<_> = v.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.init.push(tensor(n, shape, 1, &raw));
        n.into()
    }
    pub fn ints(&mut self, n: &str, v: &[i64]) -> String {
        assert!(self.loaded.insert(n.into()));
        let raw: Vec<_> = v.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.init.push(tensor(n, &[v.len() as i64], 7, &raw));
        n.into()
    }
    pub fn reshape(&mut self, x: &str, dims: &[i64]) -> String {
        let n = format!("shape_{}", self.counter);
        let s = self.ints(&n, dims);
        self.node("Reshape", &[x, &s], vec![])
    }
    pub fn finish(self) -> Vec<u8> {
        let mut g = vec![];
        for n in self.nodes {
            bytes(1, &n, &mut g);
        }
        txt(2, "yace-private-jdc-original", &mut g);
        for t in self.init {
            bytes(5, &t, &mut g);
        }
        for i in self.inputs {
            bytes(11, &i, &mut g);
        }
        for o in self.outputs {
            bytes(12, &o, &mut g);
        }
        let mut m = vec![];
        vi(1, 9, &mut m);
        txt(2, "YACE Rust private research", &mut m);
        bytes(7, &g, &mut m);
        let mut op = vec![];
        vi(2, 17, &mut op);
        bytes(8, &op, &mut m);
        m
    }
}
