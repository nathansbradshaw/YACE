//! Bounded protobuf metadata operations; never evaluates a model.
use anyhow::{Result, bail, ensure};
pub struct Field<'a> {
    pub id: u32,
    pub data: &'a [u8],
    pub raw: &'a [u8],
}
fn varint(b: &[u8], at: &mut usize) -> Result<u64> {
    let mut v = 0;
    for shift in (0..70).step_by(7) {
        ensure!(*at < b.len(), "Truncated varint");
        let x = b[*at];
        *at += 1;
        ensure!(shift < 63 || x <= 1, "Varint overflow");
        v |= ((x & 127) as u64) << shift;
        if x < 128 {
            return Ok(v);
        }
    }
    bail!("Invalid varint")
}
pub fn fields(b: &[u8]) -> Result<Vec<Field<'_>>> {
    let mut result = vec![];
    let mut at = 0;
    while at < b.len() {
        let start = at;
        let key = varint(b, &mut at)?;
        let id = (key >> 3) as u32;
        ensure!(id > 0, "Zero protobuf field");
        let data = match key & 7 {
            0 => {
                let begin = at;
                varint(b, &mut at)?;
                &b[begin..at]
            }
            1 | 5 => {
                let count = if key & 7 == 1 { 8 } else { 4 };
                ensure!(at + count <= b.len(), "Truncated scalar");
                let data = &b[at..at + count];
                at += count;
                data
            }
            2 => {
                let n = varint(b, &mut at)? as usize;
                let end = at
                    .checked_add(n)
                    .filter(|end| *end <= b.len())
                    .ok_or_else(|| anyhow::anyhow!("Truncated protobuf bytes"))?;
                let data = &b[at..end];
                at = end;
                data
            }
            _ => bail!("Unsupported protobuf wire"),
        };
        result.push(Field {
            id,
            data,
            raw: &b[start..at],
        });
        ensure!(result.len() < 100_000, "Field budget exceeded")
    }
    Ok(result)
}
fn var(mut n: u64, b: &mut Vec<u8>) {
    while n > 127 {
        b.push(n as u8 | 128);
        n >>= 7
    }
    b.push(n as u8)
}
fn msg(id: u32, v: &[u8], b: &mut Vec<u8>) {
    var((id as u64) << 3 | 2, b);
    var(v.len() as u64, b);
    b.extend(v)
}
/// Select a raw graph tensor as the sole output, preserving every node/weight.
pub fn output(model: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut result = vec![];
    for f in fields(model)? {
        if f.id != 7 {
            result.extend(f.raw);
            continue;
        }
        let mut g = vec![];
        let mut found = false;
        for n in fields(f.data)? {
            if n.id == 1 {
                for o in fields(n.data)? {
                    if o.id == 2 && o.data == name.as_bytes() {
                        found = true
                    }
                }
            }
            if n.id != 12 {
                g.extend(n.raw)
            }
        }
        ensure!(found, "Requested output absent");
        let mut tt = vec![8, 1]; // tensor_type: elem_type FLOAT; omit shape for runtime inference
        let mut shape = vec![];
        for d in ["1", "T", "360"] {
            let mut dim = vec![];
            if let Ok(v) = d.parse::<u64>() {
                var(8, &mut dim);
                var(v, &mut dim)
            } else {
                msg(2, d.as_bytes(), &mut dim)
            }
            msg(1, &dim, &mut shape)
        }
        msg(2, &shape, &mut tt);
        let mut tp = vec![];
        msg(1, &tt, &mut tp);
        let mut info = vec![];
        msg(1, name.as_bytes(), &mut info);
        msg(2, &tp, &mut info);
        msg(12, &info, &mut g);
        msg(7, &g, &mut result)
    }
    Ok(result)
}
