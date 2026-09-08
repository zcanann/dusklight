//! Auxiliary spatial tasks derived solely from authenticated achieved states.
//! These labels never modify a physical transition or its native goal evidence.
use super::*;
use crate::fact_snapshot::FactSnapshot;
use crate::tactic_features::GoalConditionedTacticFeatureEncoder;
use std::collections::BTreeMap;
use std::sync::LazyLock;

const MAX_GOALS: usize = 4;
const WORLD_WIDTH: usize = 4; // V6: stage, room, layer-present, layer.
static REFERENCE: LazyLock<GoalConditionedTacticFeatureEncoder> = LazyLock::new(|| {
    GoalConditionedTacticFeatureEncoder::new([0.0; 3]).expect("finite reference goal")
});
static TERMINAL_COLUMNS: LazyLock<[usize; 4]> = LazyLock::new(|| {
    [
        "terminal_available",
        "terminal_reached",
        "terminal_hit_fraction",
        "terminal_stability_fraction",
    ]
    .map(|name| {
        REFERENCE
            .feature_names
            .iter()
            .position(|feature| feature == name)
            .expect("V6 terminal column")
    })
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BellmanGoalKind {
    Authored,
    Coordinate,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct BellmanReplayStats {
    pub native_rows: usize,
    pub native_terminals: usize,
    pub auxiliary_goals: usize,
    pub auxiliary_rows: usize,
    pub auxiliary_terminals: usize,
    pub already_satisfied_omissions: usize,
    pub successor_feature_values: usize,
}

pub(super) struct CoordinateGoal {
    position: [u32; 3],
    stage: String,
    room: i8,
    layer: Option<i8>,
    pub encoder: GoalConditionedTacticFeatureEncoder,
    world_features: Vec<f32>,
}

impl CoordinateGoal {
    pub fn reached(&self, facts: &FactSnapshot) -> bool {
        facts.player.present
            && facts.world.stage == self.stage
            && facts.world.room == self.room
            && facts.world.layer == self.layer
            && facts.player.position_f32_bits.map(f32::from_bits)
                == self.position.map(f32::from_bits)
    }
    pub fn features(
        &self,
        facts: &FactSnapshot,
        original: &[f32],
    ) -> Result<Vec<f32>, GeneralizedTacticValueError> {
        let base_width = self.encoder.goal_distance_feature() - 3;
        let mut features = self
            .encoder
            .encode_from_base(facts, &original[..base_width])
            .map_err(|error| GeneralizedTacticValueError::InvalidFacts(error.to_string()))?;
        relabel_terminal_features(&mut features, self.reached(facts));
        append_kind(
            &mut features,
            BellmanGoalKind::Coordinate,
            &self.world_features,
        );
        Ok(features)
    }
}

pub(super) fn sampled_goals(
    rows: &[OptionTransitionSample],
) -> Result<Vec<CoordinateGoal>, GeneralizedTacticValueError> {
    let mut unique = BTreeMap::new();
    for row in rows.iter().filter(|row| row.after.player.present) {
        let facts = &row.after;
        unique
            .entry((
                facts.world.stage.clone(),
                facts.world.room,
                facts.world.layer,
                facts.player.position_f32_bits,
            ))
            .or_insert(row);
    }
    let candidates = unique.into_values().collect::<Vec<_>>();
    let count = MAX_GOALS.min(candidates.len());
    (0..count)
        .map(|index| {
            let row = candidates[index * candidates.len() / count];
            let facts = &row.after;
            Ok(CoordinateGoal {
                position: facts.player.position_f32_bits,
                stage: facts.world.stage.clone(),
                room: facts.world.room,
                layer: facts.world.layer,
                encoder: GoalConditionedTacticFeatureEncoder::new(
                    facts.player.position_f32_bits.map(f32::from_bits),
                )
                .map_err(|error| GeneralizedTacticValueError::InvalidFacts(error.to_string()))?,
                world_features: row.value_sample.next_state[..WORLD_WIDTH].to_vec(),
            })
        })
        .collect()
}

pub(super) fn query_features(
    features: &[f32],
    kind: BellmanGoalKind,
) -> Result<Vec<f32>, GeneralizedTacticValueError> {
    if features.len() != REFERENCE.feature_width() {
        return Err(GeneralizedTacticValueError::FeatureWidth);
    }
    let mut output = features.to_vec();
    if kind == BellmanGoalKind::Coordinate {
        let dx = REFERENCE.goal_distance_feature() - 3;
        relabel_terminal_features(
            &mut output,
            features[dx..dx + 3].iter().all(|value| *value == 0.0),
        );
    }
    append_kind(&mut output, kind, &features[..WORLD_WIDTH]);
    Ok(output)
}

fn relabel_terminal_features(features: &mut [f32], reached: bool) {
    features[TERMINAL_COLUMNS[0]] = 1.0;
    for index in &TERMINAL_COLUMNS[1..] {
        features[*index] = f32::from(reached);
    }
}

fn append_kind(features: &mut Vec<f32>, kind: BellmanGoalKind, world: &[f32]) {
    match kind {
        BellmanGoalKind::Authored => features.extend([0.0; WORLD_WIDTH + 1]),
        BellmanGoalKind::Coordinate => {
            features.push(1.0);
            features.extend_from_slice(world);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_evidence::native_episode_shard::NativeEpisodeShard;
    #[test]
    fn coordinate_queries_relabel_terminal_columns_without_mutating_native_features() {
        let mut original = vec![0.0; REFERENCE.feature_width()];
        let dx = REFERENCE.goal_distance_feature() - 3;
        original[dx] = 10.0;
        for index in *TERMINAL_COLUMNS {
            original[index] = 1.0;
        }
        let relabeled = query_features(&original, BellmanGoalKind::Coordinate).unwrap();
        assert_eq!(relabeled[TERMINAL_COLUMNS[0]], 1.0);
        for index in &TERMINAL_COLUMNS[1..] {
            assert_eq!(relabeled[*index], 0.0);
            assert_eq!(original[*index], 1.0);
        }
        assert_eq!(
            &query_features(&original, BellmanGoalKind::Authored).unwrap()[..original.len()],
            original.as_slice()
        );
        original[dx] = 0.0;
        for index in *TERMINAL_COLUMNS {
            original[index] = 0.0;
        }
        let arrived = query_features(&original, BellmanGoalKind::Coordinate).unwrap();
        for index in *TERMINAL_COLUMNS {
            assert_eq!(arrived[index], 1.0);
            assert_eq!(original[index], 0.0);
        }
    }

    #[test]
    fn coordinate_success_requires_world_and_height_and_player() {
        let shard = NativeEpisodeShard::decode(include_bytes!(
            "../../../../../../tests/fixtures/automation/native_episode_v28.dseps"
        ))
        .unwrap();
        let facts = FactSnapshot::from_native_learning(
            &shard.episodes[0].steps[0].pre_input,
            &[],
            None,
            Vec::new(),
        )
        .unwrap();
        let goal = CoordinateGoal {
            position: facts.player.position_f32_bits,
            stage: facts.world.stage.clone(),
            room: facts.world.room,
            layer: facts.world.layer,
            encoder: GoalConditionedTacticFeatureEncoder::new([0.0; 3]).unwrap(),
            world_features: vec![0.0; 4],
        };
        assert!(goal.reached(&facts));
        let mut other = facts.clone();
        other.world.room = other.world.room.wrapping_add(1);
        assert!(!goal.reached(&other));
        let mut other = facts.clone();
        other.player.position_f32_bits[1] =
            (f32::from_bits(other.player.position_f32_bits[1]) + 10.0).to_bits();
        assert!(!goal.reached(&other));
        let mut other = facts;
        other.player.present = false;
        assert!(!goal.reached(&other));
    }
}
