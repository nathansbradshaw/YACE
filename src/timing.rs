use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tempo {
    pub tick: u64,
    pub bpm: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TempoMap {
    pub resolution: u32,
    pub offset: f64,
    pub tempos: Vec<Tempo>,
}

impl TempoMap {
    pub fn new(resolution: u32, offset: f64, mut tempos: Vec<Tempo>) -> Result<Self> {
        ensure!(resolution > 0, "Resolution must be positive");
        ensure!(offset.is_finite(), "Offset must be finite");
        ensure!(
            tempos
                .iter()
                .all(|t| t.bpm.is_finite() && (1.0..=1000.0).contains(&t.bpm)),
            "Invalid tempo"
        );
        tempos.sort_by_key(|t| t.tick);
        ensure!(
            tempos.windows(2).all(|w| w[0].tick != w[1].tick),
            "Conflicting tempo events at the same tick"
        );
        if tempos.first().is_none_or(|t| t.tick > 0) {
            tempos.insert(
                0,
                Tempo {
                    tick: 0,
                    bpm: 120.0,
                },
            );
        }
        Ok(Self {
            resolution,
            offset,
            tempos,
        })
    }

    pub fn constant(bpm: f64) -> Result<Self> {
        Self::new(480, 0.0, vec![Tempo { tick: 0, bpm }])
    }

    pub fn seconds(&self, tick: u64) -> f64 {
        let mut seconds = self.offset;
        for (i, tempo) in self.tempos.iter().enumerate() {
            let end = self.tempos.get(i + 1).map_or(tick, |t| t.tick.min(tick));
            if end > tempo.tick {
                seconds += (end - tempo.tick) as f64 * 60.0 / (tempo.bpm * self.resolution as f64);
            }
            if end == tick {
                break;
            }
        }
        seconds
    }

    /// Audio seconds -> chart ticks. Events before chart tick zero are rejected,
    /// rather than collapsed into a false chord at tick zero.
    pub fn tick(&self, seconds: f64) -> Option<u64> {
        if !seconds.is_finite() || seconds < self.offset {
            return None;
        }
        let mut start = self.offset;
        for (i, tempo) in self.tempos.iter().enumerate() {
            let step = 60.0 / (tempo.bpm * self.resolution as f64);
            if let Some(next) = self.tempos.get(i + 1) {
                let end = start + (next.tick - tempo.tick) as f64 * step;
                if seconds >= end {
                    start = end;
                    continue;
                }
            }
            let tick = tempo.tick as f64 + (seconds - start) / step;
            return (tick <= u64::MAX as f64).then_some(tick.round() as u64);
        }
        None
    }
}
