//! Rust codecs keep source-time audio usable across browser codec differences.
use anyhow::{Context, Result, ensure};
use std::io::Cursor;
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error, formats::FormatOptions,
    io::MediaSourceStream, probe::Hint,
};
pub struct Pcm {
    pub stereo: Vec<f32>,
    pub rate: u32,
    pub channels: usize,
}
impl Pcm {
    pub fn duration(&self) -> f64 {
        self.stereo.len() as f64 / 2. / self.rate as f64
    }
    /// Windowed-sinc conversion preserves the source clock and filters content
    /// above the new Nyquist frequency before downsampling analysis audio.
    pub fn samples(&self, rate: u32, channels: u32) -> Result<Vec<f32>> {
        ensure!(
            rate > 0 && self.rate > 0 && [1, 2].contains(&channels),
            "Invalid PCM format"
        );
        let frames = self.stereo.len() / 2;
        ensure!(frames > 0, "Empty audio");
        // Match the native float decoder's speaker matrix. Stereo-to-mono
        // uses -3 dB per speaker; mono-to-stereo uses -3 dB per output.
        // An arithmetic average quietly lowered every stereo model input.
        let mono_gain = if self.channels == 1 {
            0.5
        } else {
            std::f32::consts::FRAC_1_SQRT_2
        };
        let stereo_gain = if self.channels == 1 {
            std::f32::consts::FRAC_1_SQRT_2
        } else {
            1.
        };
        if rate == self.rate {
            return Ok(if channels == 2 {
                self.stereo
                    .iter()
                    .map(|sample| sample * stereo_gain)
                    .collect()
            } else {
                self.stereo
                    .chunks_exact(2)
                    .map(|s| s[0] * mono_gain + s[1] * mono_gain)
                    .collect()
            });
        }
        let count =
            ((frames as u64 * rate as u64 + self.rate as u64 / 2) / self.rate as u64) as usize;
        let cutoff = (rate as f64 / self.rate as f64).min(1.) * 0.94;
        let radius = (32. / cutoff).ceil() as i64;
        let mut a = rate;
        let mut b = self.rate;
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let gcd = a;
        let phases = (rate / gcd) as usize;
        // The fractional positions repeat exactly for integer sample rates.
        // Build each filter phase once, rather than evaluating three trig
        // functions for every tap of every sample of a complete song.
        let mut kernels = std::collections::BTreeMap::new();
        let mut out = Vec::with_capacity(count * channels as usize);
        for frame in 0..count {
            let numerator = frame as u64 * self.rate as u64;
            let center = (numerator / rate as u64) as i64;
            let phase = ((numerator % rate as u64) / gcd as u64) as usize;
            let (kernel, weight) = kernels.entry(phase).or_insert_with(|| {
                let fraction = phase as f64 / phases as f64;
                let mut taps = Vec::with_capacity((radius * 2 + 1) as usize);
                let mut weight = 0.;
                for offset in -radius..=radius {
                    let distance = offset as f64 - fraction;
                    let window = distance / radius as f64;
                    if window.abs() > 1. {
                        continue;
                    }
                    let x = std::f64::consts::PI * distance * cutoff;
                    let sinc = if x.abs() < 1e-12 { 1. } else { x.sin() / x };
                    let w = sinc
                        * (0.42
                            + 0.5 * (std::f64::consts::PI * window).cos()
                            + 0.08 * (2. * std::f64::consts::PI * window).cos());
                    taps.push((offset, w));
                    weight += w;
                }
                (taps, weight)
            });
            let mut sum = [0.; 2];
            for &(offset, w) in kernel.iter() {
                let i = (center + offset).clamp(0, frames as i64 - 1) as usize * 2;
                sum[0] += self.stereo[i] as f64 * w;
                sum[1] += self.stereo[i + 1] as f64 * w;
            }
            if channels == 1 {
                out.push(((sum[0] + sum[1]) * mono_gain as f64 / *weight) as f32);
            } else {
                out.extend(sum.map(|s| (s * stereo_gain as f64 / *weight) as f32));
            }
        }
        Ok(out)
    }
    pub fn wave(&self) -> Result<Vec<u8>> {
        let size = u32::try_from(
            self.stereo
                .len()
                .checked_mul(2)
                .context("Recording is too long")?,
        )?;
        ensure!(size <= u32::MAX - 36, "Recording exceeds WAV size limit");
        let mut out = Vec::with_capacity(size as usize + 44);
        out.extend(b"RIFF");
        out.extend((size + 36).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(2u16.to_le_bytes());
        out.extend(self.rate.to_le_bytes());
        out.extend((self.rate * 4).to_le_bytes());
        out.extend(4u16.to_le_bytes());
        out.extend(16u16.to_le_bytes());
        out.extend(b"data");
        out.extend(size.to_le_bytes());
        for &value in &self.stereo {
            let sample = (value.clamp(-1., 1.) * i16::MAX as f32).round() as i16;
            out.extend(sample.to_le_bytes());
        }
        Ok(out)
    }
    /// Export processing results without clipping or quantizing float samples.
    pub fn float_wave(&self) -> Result<Vec<u8>> {
        ensure!(
            self.rate > 0
                && !self.stereo.is_empty()
                && self.stereo.len().is_multiple_of(2)
                && self.stereo.iter().all(|v| v.is_finite()),
            "Invalid stereo PCM"
        );
        let size = u32::try_from(
            self.stereo
                .len()
                .checked_mul(4)
                .context("Audio is too long")?,
        )?;
        ensure!(size <= u32::MAX - 36, "Audio exceeds WAV size limit");
        let mut out = Vec::with_capacity(size as usize + 44);
        out.extend(b"RIFF");
        out.extend((size + 36).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend(3u16.to_le_bytes());
        out.extend(2u16.to_le_bytes());
        out.extend(self.rate.to_le_bytes());
        out.extend((self.rate * 8).to_le_bytes());
        out.extend(8u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        out.extend(b"data");
        out.extend(size.to_le_bytes());
        for value in &self.stereo {
            out.extend(value.to_le_bytes());
        }
        Ok(out)
    }
}
pub async fn decode(bytes: &[u8]) -> Result<Pcm> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let mut format = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            source,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &Default::default(),
        )?
        .format;
    let track = format
        .default_track()
        .context("Recording has no audio track")?;
    let id = track.id;
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let mut stereo = Vec::new();
    let mut rate = 0;
    let mut source_channels = 0;
    #[cfg(target_arch = "wasm32")]
    let mut packets = 0;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(error) => return Err(error.into()),
        };
        if packet.track_id() != id {
            continue;
        }
        let decoded = decoder.decode(&packet)?;
        let spec = *decoded.spec();
        ensure!(
            rate == 0 || rate == spec.rate,
            "Recording changes sample rate"
        );
        rate = spec.rate;
        let channels = spec.channels.count();
        ensure!(
            source_channels == 0 || source_channels == channels,
            "Recording changes channel count"
        );
        source_channels = channels;
        ensure!(channels > 0, "Recording has no channels");
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        for frame in buffer.samples().chunks(channels) {
            stereo.push(frame[0]);
            stereo.push(if channels > 1 { frame[1] } else { frame[0] });
        }
        #[cfg(target_arch = "wasm32")]
        {
            packets += 1;
            if packets % 256 == 0 {
                gloo_timers::future::TimeoutFuture::new(0).await;
            }
        }
    }
    ensure!(
        rate > 0 && !stereo.is_empty() && stereo.iter().all(|v| v.is_finite()),
        "The recording did not decode to finite audio"
    );
    Ok(Pcm {
        stereo,
        rate,
        channels: source_channels,
    })
}
