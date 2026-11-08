//! The song's drum profile is independent of the current editable note set.
use crate::{chart::Chart, song_metadata::ini_fields};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    #[default]
    FourLane,
    FiveLane,
}

impl Layout {
    /// Match the source format's present-value bool convention. Explicit false
    /// must not be overridden by a fifth-lane event in contradictory input.
    pub fn from_source(ini: &str, chart: &Chart) -> Result<Self> {
        if let Some(value) = ini_fields(ini).get("five_lane_drums") {
            return Ok(
                if value.starts_with('1')
                    || value
                        .get(..4)
                        .is_some_and(|v| v.eq_ignore_ascii_case("true"))
                {
                    Self::FiveLane
                } else {
                    Self::FourLane
                },
            );
        }
        for section in chart.sections.iter().filter(|s| s.name.ends_with("Drums")) {
            if chart
                .notes(&section.name)?
                .iter()
                .any(|note| note.lane == 5)
            {
                return Ok(Self::FiveLane);
            }
        }
        Ok(Self::FourLane)
    }

    pub fn pad(self, lane: u8) -> bool {
        (1..=self.pads()).contains(&lane)
    }

    pub fn pads(self) -> u8 {
        match self {
            Self::FourLane => 4,
            Self::FiveLane => 5,
        }
    }

    pub fn playable(self, lane: u8) -> bool {
        lane == 0 || lane == 32 || self.pad(lane)
    }

    pub fn cymbal_flag(self, lane: u8) -> bool {
        self == Self::FourLane && (2..=4).contains(&lane)
    }

    pub fn name(self, lane: u8) -> &'static str {
        match (self, lane) {
            (_, 0) => "Kick",
            (_, 32) => "Double kick",
            (_, 1) => "Snare",
            (_, 2) => "Yellow",
            (_, 3) => "Blue",
            (Self::FourLane, 4) | (Self::FiveLane, 5) => "Green",
            (Self::FiveLane, 4) => "Orange",
            _ => "Control",
        }
    }

    pub fn color(self, lane: u8) -> &'static str {
        match (self, lane) {
            (_, 0 | 32) | (Self::FiveLane, 4) => "#f2a65a",
            (_, 1) => "#ec5360",
            (_, 2) => "#ead452",
            (_, 3) => "#4b9be9",
            (Self::FourLane, 4) | (Self::FiveLane, 5) => "#45c878",
            _ => "#666",
        }
    }

    pub fn rows(self) -> Vec<u8> {
        [0, 32].into_iter().chain(1..=self.pads()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(part: &str, lane: u8) -> Chart {
        Chart::parse(format!(
            "[Song]\n{{\n Resolution = 192\n}}\n[{part}]\n{{\n 0 = N {lane} 0\n}}\n"
        ))
        .unwrap()
    }

    #[test]
    fn explicit_false_has_priority_over_a_fifth_pad() {
        let chart = source("ExpertDrums", 5);
        for value in ["false", "0", "FALSE"] {
            assert_eq!(
                Layout::from_source(&format!("[song]\nfive_lane_drums = {value}\n"), &chart)
                    .unwrap(),
                Layout::FourLane
            );
        }
        assert_eq!(Layout::from_source("", &chart).unwrap(), Layout::FiveLane);
    }

    #[test]
    fn explicit_five_lane_does_not_require_a_fifth_pad() {
        let chart = source("ExpertDrums", 4);
        for value in ["true", "1", "TRUE"] {
            assert_eq!(
                Layout::from_source(&format!("[song]\nfive_lane_drums = {value}\n"), &chart)
                    .unwrap(),
                Layout::FiveLane
            );
        }
        assert_eq!(
            Layout::from_source("", &source("ExpertSingle", 5)).unwrap(),
            Layout::FourLane
        );
    }
}
