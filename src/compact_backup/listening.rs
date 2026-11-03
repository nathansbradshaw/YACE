//! A reversible container for peak-scaled 24-bit listening copies of stems.
//! Quantization is explicit; source recordings never use this representation.
use super::FloatLayout;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
const BLOCK: usize = 4096;
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Listening {
    pub layout: FloatLayout,
    pub step: f64,
}
fn quantize(sample: &[u8], step: f64) -> i32 {
    (f32::from_le_bytes(sample.try_into().unwrap()) as f64 / step).round() as i32
}
pub fn prepare(bytes: &[u8], layout: FloatLayout) -> Option<(Listening, String, u64)> {
    let audio = &bytes[layout.offset..layout.offset + layout.bytes];
    let mut peak = 0f64;
    for sample in audio.chunks_exact(4) {
        let value = f32::from_le_bytes(sample.try_into().ok()?);
        if !value.is_finite() {
            return None;
        }
        peak = peak.max(f64::from(value).abs());
    }
    let copy = Listening {
        layout,
        step: if peak == 0. {
            1. / 8_388_607.
        } else {
            peak / 8_388_607.
        },
    };
    let mut hash = Sha256::new();
    hash.update(&bytes[..layout.offset]);
    for sample in audio.chunks_exact(4) {
        let value = (f64::from(quantize(sample, copy.step)) * copy.step) as f32;
        hash.update(value.to_le_bytes());
    }
    hash.update(&bytes[layout.offset + layout.bytes..]);
    let size = (bytes.len() - layout.bytes
        + layout.bytes / 4 * 3
        + (layout.bytes / 4).div_ceil(BLOCK)) as u64;
    Some((copy, format!("{:x}", hash.finalize()), size))
}
pub fn write<W: Write>(writer: &mut W, bytes: &[u8], copy: Listening) -> Result<()> {
    let layout = copy.layout;
    writer.write_all(&bytes[..layout.offset])?;
    let mut packed = vec![0u8; BLOCK * 3];
    let mut values = vec![0u32; BLOCK];
    for block in bytes[layout.offset..layout.offset + layout.bytes].chunks(BLOCK * 4) {
        let count = block.len() / 4;
        let mut left = 0u32;
        for (i, sample) in block.chunks_exact(4).enumerate() {
            let value = quantize(sample, copy.step) as u32 & 0xffffff;
            values[i] = if layout.channels == 2 {
                if i % 2 == 0 {
                    left = value;
                    value
                } else {
                    value.wrapping_sub(left) & 0xffffff
                }
            } else {
                value
            };
        }
        let mut costs = [0u64; 2];
        let mut previous = [0u32; 8];
        let mut older = [0u32; 8];
        for (i, value) in values[..count].iter().copied().enumerate() {
            let channel = i % layout.channels;
            let first = value.wrapping_sub(previous[channel]);
            let second = first.wrapping_sub(previous[channel].wrapping_sub(older[channel]));
            costs[0] += u64::from(((first << 8) as i32 >> 8).unsigned_abs());
            costs[1] += u64::from(((second << 8) as i32 >> 8).unsigned_abs());
            older[channel] = previous[channel];
            previous[channel] = value;
        }
        let order = if costs[1] < costs[0] { 2u8 } else { 1u8 };
        writer.write_all(&[order])?;
        let mut previous = [0u32; 8];
        let mut older = [0u32; 8];
        for (i, value) in values[..count].iter().copied().enumerate() {
            let channel = i % layout.channels;
            let prediction = if order == 2 {
                previous[channel]
                    .wrapping_mul(2)
                    .wrapping_sub(older[channel])
            } else {
                previous[channel]
            };
            let delta = value.wrapping_sub(prediction).to_le_bytes();
            older[channel] = previous[channel];
            previous[channel] = value;
            for plane in 0..3 {
                packed[plane * count + i] = delta[plane];
            }
        }
        writer.write_all(&packed[..count * 3])?;
    }
    writer.write_all(&bytes[layout.offset + layout.bytes..])?;
    Ok(())
}
pub fn restore(bytes: Vec<u8>, copy: Listening, length: u64) -> Result<Vec<u8>> {
    let layout = copy.layout;
    ensure!(
        (1..=8).contains(&layout.channels)
            && copy.step.is_finite()
            && copy.step > 0.
            && layout.bytes.is_multiple_of(layout.channels * 4),
        "Invalid listening-stem representation"
    );
    let length = usize::try_from(length)?;
    ensure!(
        layout
            .offset
            .checked_add(layout.bytes)
            .is_some_and(|end| end <= length),
        "Invalid listening-stem length"
    );
    let samples = layout.bytes / 4;
    let payload = samples * 3 + samples.div_ceil(BLOCK);
    ensure!(
        length - layout.bytes + payload == bytes.len(),
        "Listening-stem payload length differs"
    );
    let mut restored = Vec::with_capacity(length);
    restored.extend_from_slice(&bytes[..layout.offset]);
    let mut at = layout.offset;
    for start in (0..samples).step_by(BLOCK) {
        let count = (samples - start).min(BLOCK);
        let order = bytes[at];
        at += 1;
        ensure!(order == 1 || order == 2, "Invalid listening-stem predictor");
        let block = &bytes[at..at + count * 3];
        at += count * 3;
        let mut previous = [0u32; 8];
        let mut older = [0u32; 8];
        let mut left = 0u32;
        for i in 0..count {
            let delta = u32::from_le_bytes([block[i], block[count + i], block[2 * count + i], 0]);
            let channel = i % layout.channels;
            let prediction = if order == 2 {
                previous[channel]
                    .wrapping_mul(2)
                    .wrapping_sub(older[channel])
            } else {
                previous[channel]
            };
            let value = delta.wrapping_add(prediction) & 0xffffff;
            older[channel] = previous[channel];
            previous[channel] = value;
            let raw = if layout.channels == 2 {
                if i % 2 == 0 {
                    left = value;
                    value
                } else {
                    value.wrapping_add(left) & 0xffffff
                }
            } else {
                value
            };
            let signed = ((raw << 8) as i32) >> 8;
            let sample = (f64::from(signed) * copy.step) as f32;
            ensure!(sample.is_finite(), "Listening-stem sample is nonfinite");
            restored.extend_from_slice(&sample.to_le_bytes());
        }
    }
    restored.extend_from_slice(&bytes[layout.offset + payload..]);
    ensure!(
        super::float_layout(&restored).is_some_and(|actual| actual.offset == layout.offset
            && actual.bytes == layout.bytes
            && actual.channels == layout.channels),
        "Listening-stem WAV header differs"
    );
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::super::digest;
    use super::*;
    #[test]
    fn listening_copies_preserve_clock_headroom_and_bound_rounding_error() {
        let mut bytes = super::super::tests::wave();
        for sample in bytes[44..].chunks_exact_mut(4) {
            let value = f32::from_le_bytes(sample.try_into().unwrap());
            let value = if value.is_finite() { value * 3.7 } else { 0. };
            sample.copy_from_slice(&value.to_le_bytes());
        }
        let layout = super::super::float_layout(&bytes).unwrap();
        let (copy, hash, _) = prepare(&bytes, layout).unwrap();
        let mut packed = Vec::new();
        write(&mut packed, &bytes, copy).unwrap();
        let restored = restore(packed, copy, bytes.len() as u64).unwrap();
        assert_eq!(digest(&restored), hash);
        assert_eq!(&restored[..44], &bytes[..44]);
        let mut error = 0f64;
        for (a, b) in bytes[44..]
            .chunks_exact(4)
            .zip(restored[44..].chunks_exact(4))
        {
            error = error.max(
                (f64::from(f32::from_le_bytes(a.try_into().unwrap()))
                    - f64::from(f32::from_le_bytes(b.try_into().unwrap())))
                .abs(),
            );
        }
        assert!(error < 3.7 * 1.3e-7);
    }
}
