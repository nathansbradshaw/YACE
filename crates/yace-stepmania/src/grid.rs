//! One musical grid for pointer edits, visible lines and keyboard navigation.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_DIVISIONS: u32 = 1024;

#[derive(Clone, Copy, Debug)]
pub struct Grid {
    resolution: u32,
    divisions: u32,
}

impl Grid {
    pub fn new(resolution: u32, divisions: u32) -> Result<Self> {
        ensure!(resolution > 0, "Chart resolution must be positive");
        ensure!(
            (1..=MAX_DIVISIONS).contains(&divisions),
            "Use 1–1024 steps per beat"
        );
        Ok(Self {
            resolution,
            divisions,
        })
    }

    fn at(self, index: u128) -> Option<u64> {
        let tick = (index.checked_mul(self.resolution as u128)? + self.divisions as u128 / 2)
            / self.divisions as u128;
        u64::try_from(tick).ok()
    }

    /// Integer chart positions do not pass through floating point during repair.
    pub fn snap_tick(self, tick: u64) -> Option<u64> {
        self.at(
            (tick as u128 * self.divisions as u128 + self.resolution as u128 / 2)
                / self.resolution as u128,
        )
    }

    /// Select the nearest musical position before rounding to the file's tick.
    pub fn snap(self, tick: f64) -> Option<u64> {
        if !tick.is_finite() || tick < 0.0 || tick >= u64::MAX as f64 {
            return None;
        }
        let index = (tick * self.divisions as f64 / self.resolution as f64).round();
        self.at(index as u128)
    }

    /// Distinct representable grid ticks, bounded even at extreme zoom levels.
    /// Dense views show an aligned subset; zooming in reveals finer lines.
    pub fn visible(self, first: u64, last: u64, limit: usize) -> Vec<u64> {
        if last < first || limit == 0 {
            return Vec::new();
        }
        let a = first as u128 * self.divisions as u128 / self.resolution as u128;
        let b = (last as u128 * self.divisions as u128).div_ceil(self.resolution as u128);
        let stride = (b - a + 1).div_ceil(limit as u128).max(1);
        (0..limit)
            .map(|i| a + i as u128 * stride)
            .take_while(|i| *i <= b)
            .filter_map(|i| self.at(i))
            .filter(|t| (first..=last).contains(t))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Advance by one distinct grid tick, including grids finer than one tick.
    pub fn adjacent(self, tick: u64, forward: bool) -> Option<u64> {
        let mut index = tick as u128 * self.divisions as u128 / self.resolution as u128;
        for _ in 0..=MAX_DIVISIONS + 2 {
            if let Some(value) = self.at(index)
                && ((forward && value > tick) || (!forward && value < tick))
            {
                return Some(value);
            }
            index = if forward {
                index.checked_add(1)?
            } else {
                index.checked_sub(1)?
            };
        }
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairPolicy {
    pub divisions: u32,
    /// None explicitly snaps all notes; Some keeps notes farther from the grid.
    pub maximum_shift: Option<u64>,
}
impl RepairPolicy {
    pub fn grid(self, resolution: u32) -> Result<Grid> {
        Grid::new(resolution, self.divisions)
    }
    pub fn destination(self, grid: Grid, tick: u64) -> Result<u64> {
        let nearest = grid
            .snap_tick(tick)
            .ok_or_else(|| anyhow::anyhow!("Grid timing overflow"))?;
        Ok(
            if self
                .maximum_shift
                .is_some_and(|limit| tick.abs_diff(nearest) > limit)
            {
                tick
            } else {
                nearest
            },
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepairPreview {
    pub examined: usize,
    pub changed: usize,
    pub already_on_grid: usize,
    pub kept_off_grid: usize,
    pub largest_shift: u64,
    /// A bounded sample, independent of project size.
    pub examples: Vec<(u64, u64)>,
}
impl RepairPreview {
    pub fn observe(&mut self, before: u64, nearest: u64, after: u64) {
        self.examined += 1;
        if before == after {
            if before == nearest {
                self.already_on_grid += 1;
            } else {
                self.kept_off_grid += 1;
            }
        } else {
            self.changed += 1;
            self.largest_shift = self.largest_shift.max(before.abs_diff(after));
            if self.examples.len() < 12 {
                self.examples.push((before, after));
            }
        }
    }
}
