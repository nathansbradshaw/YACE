use super::*;
/// Prepared immutable control data. Rendering runs on a processing worker,
/// before the recording and clicks enter the same tempo-conversion stream.
pub struct Mix {
    plan: Arc<Plan>,
    settings: Settings,
    active: bool,
}
impl Mix {
    pub fn new(plan: Arc<Plan>, settings: Settings) -> Result<Self> {
        settings.validate()?;
        let active = plan.events().iter().any(|e| match e.kind {
            Kind::Beat | Kind::Accent => settings.metronome && settings.beat_gain > 0.,
            Kind::Clap => settings.claps && settings.clap_gain > 0.,
        });
        Ok(Self {
            plan,
            settings,
            active,
        })
    }
    pub fn active(&self) -> bool {
        self.active
    }
    pub fn render(&self, start: f64, sample_rate: u32, output: &mut [f32]) -> Result<()> {
        self.render_output(start, 1., sample_rate, output)
    }
    pub fn render_output(
        &self,
        start: f64,
        rate: f64,
        sample_rate: u32,
        output: &mut [f32],
    ) -> Result<()> {
        self.plan
            .render(start, rate, sample_rate, &self.settings, output)
    }
    pub fn render_output_epoch(
        &self,
        start: f64,
        rate: f64,
        sample_rate: u32,
        output: &mut [f32],
        epoch: Option<f64>,
    ) -> Result<()> {
        self.plan
            .render_epoch(start, rate, sample_rate, &self.settings, output, epoch)
    }
    /// The click bus reserves at most 25% of nominal full scale. Its fade uses
    /// exactly the headroom currently available while the music gain ramps.
    pub fn apply(
        mix: Option<&Self>,
        start: f64,
        sample_rate: u32,
        stereo: &mut [f32],
        scratch: &mut Vec<f32>,
        music_gain: &mut f32,
    ) -> Result<()> {
        ensure!(
            stereo.len().is_multiple_of(2),
            "Audition needs complete stereo frames"
        );
        let active = mix.is_some_and(Self::active);
        if !active && *music_gain == 1. {
            return Ok(());
        }
        scratch.resize(stereo.len() / 2, 0.);
        if let Some(mix) = mix.filter(|m| m.active) {
            mix.render(start, sample_rate, scratch)?;
        } else {
            scratch.fill(0.);
        }
        let target = if active { 1. - PEAK } else { 1. };
        let step = 1. / (0.006 * sample_rate as f32).max(1.);
        for (frame, &click) in stereo.chunks_exact_mut(2).zip(scratch.iter()) {
            *music_gain += (target - *music_gain).clamp(-step, step);
            let click_gain = ((1. - *music_gain) / PEAK).clamp(0., 1.);
            for sample in frame {
                *sample = *sample * *music_gain + click * click_gain;
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_is_bit_identical_and_enabling_keeps_nominal_headroom() {
        let original: Vec<_> = (0..1600)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let mut out = original.clone();
        let mut scratch = Vec::new();
        let mut gain = 1.;
        Mix::apply(None, 0., 8000, &mut out, &mut scratch, &mut gain).unwrap();
        assert_eq!(out, original);
        assert!(scratch.is_empty());
        let plan = Arc::new(
            Plan::new(
                vec![
                    Event {
                        seconds: 0.,
                        kind: Kind::Accent,
                    },
                    Event {
                        seconds: 0.,
                        kind: Kind::Clap,
                    },
                ],
                1.,
            )
            .unwrap(),
        );
        let mix = Mix::new(
            plan,
            Settings {
                metronome: true,
                claps: true,
                beat_gain: 1.,
                clap_gain: 1.,
                ..Settings::default()
            },
        )
        .unwrap();
        Mix::apply(Some(&mix), 0., 8000, &mut out, &mut scratch, &mut gain).unwrap();
        assert!(out.iter().all(|x| x.abs() <= 1.));
        assert_eq!(gain, 1. - PEAK);
        assert_ne!(out, original);
        Mix::apply(None, 0.1, 8000, &mut out, &mut scratch, &mut gain).unwrap();
        assert_eq!(gain, 1.);
        let empty = Mix::new(
            Arc::new(Plan::new(vec![], 1.).unwrap()),
            Settings {
                claps: true,
                ..Settings::default()
            },
        )
        .unwrap();
        assert!(!empty.active());
    }
}
