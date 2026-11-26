//! Voting rules from config file. Might drop this later

use serde::{Deserialize, Serialize};

// IMPORTANT: Please do not set these quorums blindly.
// 0.25, 0.50, 0.75 and such values are safe but some values will cause floating-point rounding errors
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Policy {
    /// Fraction of an organisation's members who must vote for it to count (0 to 1)
    pub internal_quorum: f64,
    /// Fraction of the entities that must count for the vote to be valid (0 to 1)
    pub coalition_quorum: f64,
    /// How long a vote stays open if its first mail doesn't say
    pub window_hours: u32,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            internal_quorum: 0.5,
            coalition_quorum: 0.5,
            window_hours: 24,
        }
    }
}

impl Policy {
    pub fn check(&self) -> Result<(), String> {
        for (name, quorum) in [
            ("internal_quorum", self.internal_quorum),
            ("coalition_quorum", self.coalition_quorum),
        ] {
            if !(0.0..=1.0).contains(&quorum) {
                return Err(format!("{name} must be between 0 and 1, got {quorum}"));
            }
        }
        if self.window_hours == 0 {
            return Err("window_hours must more than 0".to_string());
        }
        Ok(())
    }
}
