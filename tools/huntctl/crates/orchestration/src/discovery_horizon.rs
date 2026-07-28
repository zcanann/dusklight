//! Minimum route capacity for terminal discovery.
//!
//! Promotion and final route quality are optimization thresholds. They are not
//! evidence that an unassisted learner can discover a terminal within roughly
//! the same amount of time. Discovery therefore reserves enough native time
//! for a materially inefficient exploratory trajectory before shortening.

pub(crate) const NATIVE_LOGICAL_TICKS_PER_SECOND: u64 = 30;
pub(crate) const MINIMUM_UNASSISTED_DISCOVERY_SECONDS: u64 = 30;
pub(crate) const MINIMUM_UNASSISTED_DISCOVERY_TICKS: u64 =
    NATIVE_LOGICAL_TICKS_PER_SECOND * MINIMUM_UNASSISTED_DISCOVERY_SECONDS;

pub(crate) fn minimum_discovery_horizon_ticks(promotion_before_tick: u64) -> Option<u64> {
    promotion_before_tick
        .checked_mul(2)
        .map(|scaled| scaled.max(MINIMUM_UNASSISTED_DISCOVERY_TICKS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_reserves_real_exploration_time_and_scales_with_the_target() {
        assert_eq!(minimum_discovery_horizon_ticks(1), Some(900));
        assert_eq!(minimum_discovery_horizon_ticks(125), Some(900));
        assert_eq!(minimum_discovery_horizon_ticks(131), Some(900));
        assert_eq!(minimum_discovery_horizon_ticks(1_000), Some(2_000));
        assert_eq!(minimum_discovery_horizon_ticks(u64::MAX), None);
    }
}
