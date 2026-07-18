//! Stable, portable actor identities shared by observations, proofs, and interventions.

use serde::{Deserialize, Serialize};

pub const STAGE_NAME_CAPACITY: usize = 8;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct PlacedActorSelector {
    pub stage: String,
    pub home_room: i8,
    pub set_id: u16,
    pub actor_name: i16,
}

impl PlacedActorSelector {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.stage.is_empty()
            || self.stage.len() > STAGE_NAME_CAPACITY
            || !self
                .stage
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(
                "placed actor stage names must be 1..8 ASCII uppercase, digit, or underscore bytes",
            );
        }
        if !(-1..=63).contains(&self.home_room) {
            return Err("placed actor home room must be -1..63");
        }
        if self.set_id == u16::MAX {
            return Err("placed actor set ID 65535 is reserved as unavailable");
        }
        if self.actor_name < 0 {
            return Err("placed actor name must be nonnegative");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_placed_identity_has_one_shared_bound() {
        let valid = PlacedActorSelector {
            stage: "F_SP104".into(),
            home_room: 1,
            set_id: 12,
            actor_name: 7,
        };
        assert!(valid.validate().is_ok());
        assert!(
            PlacedActorSelector {
                stage: "lower".into(),
                ..valid.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            PlacedActorSelector {
                home_room: 64,
                ..valid.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            PlacedActorSelector {
                set_id: u16::MAX,
                ..valid.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            PlacedActorSelector {
                actor_name: -1,
                ..valid
            }
            .validate()
            .is_err()
        );
    }
}
