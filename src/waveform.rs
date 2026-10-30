//! Source-time, signed peak envelopes with zoom-dependent detail.
use std::fmt::Write;

pub const BASE_FRAMES_PER_BIN: usize = 16;
const FANOUT: usize = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Extrema {
    pub min: f32,
    pub max: f32,
}
impl Extrema {
    fn include(&mut self, other: Self) {
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }
}

#[derive(Debug)]
struct Level {
    frames_per_bin: usize,
    bins: Vec<Extrema>,
}

#[derive(Debug)]
pub struct Waveform {
    sample_rate: u32,
    frames: usize,
    levels: Vec<Level>,
}
impl Waveform {
    /// Keep stereo extrema rather than averaging channels, which can cancel
    /// opposite-phase audio. No gain or per-recording normalization is applied.
    pub fn from_stereo(pcm: &[f32], sample_rate: u32) -> Self {
        let mut levels = vec![Level {
            frames_per_bin: BASE_FRAMES_PER_BIN,
            bins: pcm
                .chunks(BASE_FRAMES_PER_BIN * 2)
                .map(|chunk| {
                    let mut range = Extrema::default();
                    for &sample in chunk {
                        if sample.is_finite() {
                            range.min = range.min.min(sample);
                            range.max = range.max.max(sample);
                        }
                    }
                    range
                })
                .collect(),
        }];
        while levels.last().unwrap().bins.len() > 1 {
            let previous = levels.last().unwrap();
            let next = Level {
                frames_per_bin: previous.frames_per_bin * FANOUT,
                bins: previous
                    .bins
                    .chunks(FANOUT)
                    .map(|chunk| {
                        let mut range = Extrema::default();
                        for &bin in chunk {
                            range.include(bin);
                        }
                        range
                    })
                    .collect(),
            };
            levels.push(next);
        }
        Self {
            sample_rate,
            frames: pcm.len() / 2,
            levels,
        }
    }

    /// One signed envelope per display column. Select small enough bins to
    /// retain attacks without scanning full-song PCM on every scroll/zoom.
    pub fn columns(&self, start: f64, span: f64, columns: usize) -> Vec<Extrema> {
        if !start.is_finite()
            || !span.is_finite()
            || span <= 0.0
            || columns == 0
            || self.sample_rate == 0
        {
            return Vec::new();
        }
        let frames_per_column = span * self.sample_rate as f64 / columns as f64;
        let level = self
            .levels
            .iter()
            .rev()
            .find(|level| level.frames_per_bin as f64 <= frames_per_column / 4.0)
            .unwrap_or(&self.levels[0]);
        (0..columns)
            .map(|column| {
                let lo = start * self.sample_rate as f64 + column as f64 * frames_per_column;
                let hi = lo + frames_per_column;
                if hi <= 0.0 || lo >= self.frames as f64 {
                    return Extrema::default();
                }
                let first = lo.max(0.0) as usize / level.frames_per_bin;
                let end =
                    (hi.min(self.frames as f64) / level.frames_per_bin as f64).ceil() as usize;
                let mut range = Extrema::default();
                for &bin in &level.bins[first..end.min(level.bins.len())] {
                    range.include(bin);
                }
                range
            })
            .collect()
    }

    /// A connected, antialiased outline avoids the old wide rectangular bars.
    /// Display clipping is shared full scale; the cached extrema stay intact.
    pub fn geometry(&self, start: f64, span: f64, width: f64) -> String {
        if !width.is_finite() || width <= 0.0 {
            return String::new();
        }
        let columns = self.columns(start, span, width.ceil() as usize);
        if columns.is_empty() {
            return String::new();
        }
        let mut path = String::with_capacity(columns.len() * 36);
        let step = width / columns.len() as f64;
        let top = |range: Extrema| 18.0 - range.max.clamp(0.0, 1.0) * 16.0;
        let bottom = |range: Extrema| 18.0 - range.min.clamp(-1.0, 0.0) * 16.0;
        let _ = write!(path, "M0 {:.2}", top(columns[0]));
        for (i, &range) in columns.iter().enumerate() {
            let _ = write!(path, "L{:.2} {:.2}", (i as f64 + 0.5) * step, top(range));
        }
        let last = *columns.last().unwrap();
        let _ = write!(
            path,
            "L{width:.2} {:.2}L{width:.2} {:.2}",
            top(last),
            bottom(last)
        );
        for (i, &range) in columns.iter().enumerate().rev() {
            let _ = write!(path, "L{:.2} {:.2}", (i as f64 + 0.5) * step, bottom(range));
        }
        let _ = write!(path, "L0 {:.2}Z", bottom(columns[0]));
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_zoom_resolves_attacks_inside_an_old_1024_frame_block() {
        let mut pcm = vec![0.0; 2048];
        pcm[256 * 2] = 0.9;
        pcm[256 * 2 + 1] = -0.4;
        let waveform = Waveform::from_stereo(&pcm, 1024);
        let columns = waveform.columns(0.0, 1.0, 64);
        assert_eq!(
            columns[16],
            Extrema {
                min: -0.4,
                max: 0.9
            }
        );
        assert_eq!(columns.iter().filter(|range| range.max > 0.0).count(), 1);
        assert_eq!(waveform.columns(0.25, 0.5, 32)[0], columns[16]);
    }

    #[test]
    fn stereo_phase_scale_and_partial_tail_survive_all_zoom_levels() {
        let mut pcm = vec![0.0; 8193 * 2];
        pcm[8192 * 2] = 1.2;
        pcm[8192 * 2 + 1] = -0.25;
        let waveform = Waveform::from_stereo(&pcm, 8193);
        let expected = Extrema {
            min: -0.25,
            max: 1.2,
        };
        assert_eq!(waveform.columns(0.0, 1.0, 1), vec![expected]);
        assert_eq!(*waveform.columns(0.0, 1.0, 512).last().unwrap(), expected);
        let path = waveform.geometry(8192.0 / 8193.0, 1.0 / 8193.0, 1.0);
        assert!(path.contains("M0 2.00"));
        assert!(path.contains("22.00"));
    }

    #[test]
    fn out_of_recording_is_silent_and_invalid_views_are_empty() {
        let waveform = Waveform::from_stereo(&[0.5; 64], 32);
        assert_eq!(
            waveform.columns(-1.0, 3.0, 3),
            vec![
                Extrema::default(),
                Extrema { min: 0.0, max: 0.5 },
                Extrema::default()
            ]
        );
        for (start, span, width) in [
            (f64::NAN, 1.0, 10.0),
            (0.0, 0.0, 10.0),
            (0.0, 1.0, f64::INFINITY),
        ] {
            assert!(waveform.geometry(start, span, width).is_empty());
        }
        assert_eq!(
            Waveform::from_stereo(&[], 44100).columns(0.0, 1.0, 1),
            vec![Extrema::default()]
        );
    }
}
