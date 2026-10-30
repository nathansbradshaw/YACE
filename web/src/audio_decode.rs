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
}
impl Pcm {
    pub fn duration(&self) -> f64 {
        self.stereo.len() as f64 / 2. / self.rate as f64
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
    Ok(Pcm { stereo, rate })
}
