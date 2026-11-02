//! Versioned detection strictness shared by native and browser workers.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const POLICY: &str = "sensitivity-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub sensitivity: u8,
    pub policy_version: u8,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            sensitivity: 50,
            policy_version: 1,
        }
    }
}
impl Settings {
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.sensitivity <= 100,
            "Sensitivity must be between 0 and 100"
        );
        ensure!(
            self.policy_version == 1,
            "Unsupported sensitivity policy; retained drafts were kept"
        );
        Ok(())
    }
    pub fn multiplier(self) -> f32 {
        2_f32.powf((50. - f32::from(self.sensitivity)) / 50.)
    }
    pub fn threshold(self, baseline: f32) -> f32 {
        (baseline * self.multiplier()).clamp(0.001, 0.999)
    }
    pub fn piano(self) -> crate::piano_model::Options {
        let mut config = crate::piano_model::Options::default();
        config.onset_threshold = self.threshold(config.onset_threshold);
        config.frame_threshold = self.threshold(config.frame_threshold);
        config
    }
    pub fn vocals(
        self,
        mode: crate::vocal_generation::AnalysisMode,
    ) -> crate::vocal_generation::Policy {
        let mut policy = crate::vocal_generation::Policy {
            analysis_mode: mode,
            ..Default::default()
        };
        policy.absolute_rms_floor *= f64::from(self.multiplier());
        policy.relative_rms_floor *= f64::from(self.multiplier());
        policy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_endpoints_and_validation() {
        assert_eq!(Settings::default().multiplier(), 1.);
        assert_eq!(
            Settings {
                sensitivity: 0,
                ..Default::default()
            }
            .multiplier(),
            2.
        );
        assert_eq!(
            Settings {
                sensitivity: 100,
                ..Default::default()
            }
            .multiplier(),
            0.5
        );
        assert!(
            Settings {
                sensitivity: 101,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Settings {
                policy_version: 2,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        let default = Settings::default();
        assert_eq!(
            serde_json::to_value(default.piano()).unwrap(),
            serde_json::to_value(crate::piano_model::Options::default()).unwrap()
        );
        assert_eq!(
            serde_json::to_value(default.vocals(Default::default())).unwrap(),
            serde_json::to_value(crate::vocal_generation::Policy::default()).unwrap()
        );
        assert!(
            Settings {
                sensitivity: 100,
                ..default
            }
            .vocals(Default::default())
            .absolute_rms_floor
                < default.vocals(Default::default()).absolute_rms_floor
        );
    }
}
