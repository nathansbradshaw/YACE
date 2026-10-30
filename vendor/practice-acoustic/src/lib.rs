mod config;
mod model;
use candle_core::{D, DType, Device, Tensor};
use candle_nn::VarBuilder;
use std::path::Path;
pub type Error = Box<dyn std::error::Error + Send + Sync>;

pub struct AcousticModel {
    model: model::ctc_model::Wav2Vec2ForCTC,
}
impl AcousticModel {
    pub fn load(directory: &Path) -> Result<Self, Error> {
        let config = serde_json::from_slice(&std::fs::read(directory.join("config.json"))?)?;
        let weights = std::fs::read(directory.join("model.safetensors"))?;
        let builder = VarBuilder::from_buffered_safetensors(weights, DType::F32, &Device::Cpu)?;
        Ok(Self {
            model: model::ctc_model::Wav2Vec2ForCTC::load(&config, builder)?,
        })
    }
    /// Emit frame log probabilities at 20 ms intervals with bounded context.
    pub fn emissions(&self, audio: &[f32]) -> Result<Vec<Vec<f32>>, Error> {
        self.emissions_cancellable(audio, &|| false, &|_, _| {})
    }
    /// Chart Engine checks cancellation between bounded native inference
    /// windows; callbacks do not enter the underlying tensor operation.
    pub fn emissions_cancellable(
        &self,
        audio: &[f32],
        cancelled: &dyn Fn() -> bool,
        progress: &dyn Fn(usize, usize),
    ) -> Result<Vec<Vec<f32>>, Error> {
        let mut emissions = Vec::new();
        let step = 80_000;
        for start in (0..audio.len()).step_by(step) {
            if cancelled() {
                return Err("Acoustic lyric alignment cancelled".into());
            }
            progress(start, audio.len());
            let lo = start.saturating_sub(16_000);
            let hi = (start + step + 16_000).min(audio.len());
            let window = &audio[lo..hi];
            if window.len() < 400 {
                continue;
            }
            let mean = window.iter().map(|&v| f64::from(v)).sum::<f64>() / window.len() as f64;
            let variance = window
                .iter()
                .map(|&v| (f64::from(v) - mean).powi(2))
                .sum::<f64>()
                / window.len() as f64;
            let scale = (variance + 1e-7).sqrt();
            let samples = window
                .iter()
                .map(|&v| ((f64::from(v) - mean) / scale) as f32)
                .collect::<Vec<_>>();
            let input = Tensor::from_vec(samples, (1, window.len()), &Device::Cpu)?;
            let logits = self.model.forward(&input)?;
            let rows = candle_nn::ops::log_softmax(&logits, D::Minus1)?
                .squeeze(0)?
                .to_vec2::<f32>()?;
            let first = (start - lo) / 320;
            let count = (audio.len() - start).min(step) / 320;
            emissions.extend(rows.into_iter().skip(first).take(count));
        }
        if cancelled() {
            return Err("Acoustic lyric alignment cancelled".into());
        }
        Ok(emissions)
    }
}
