//! Episode-bootstrapped ensembles of deterministic twin Double-Q critics.

use crate::artifact::Digest;
use crate::double_q::{DoubleQ, DoubleQConfig};
use crate::fqi::{MAX_FQI_ACTIONS, MAX_FQI_TRANSITIONS, Transition};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const BOOTSTRAPPED_Q_ENSEMBLE_SCHEMA_V1: &str = "dusklight-bootstrapped-q-ensemble/v1";
pub const MAX_ENSEMBLE_MEMBERS: usize = 31;
pub const MAX_ENSEMBLE_GRADIENT_UPDATES: usize = 50_000_000;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BootstrappedQConfig {
    pub members: usize,
    pub seed: u64,
    pub critic: DoubleQConfig,
}

impl Default for BootstrappedQConfig {
    fn default() -> Self {
        Self {
            members: 7,
            seed: 0xe115_ea5e_5eed_0001,
            critic: DoubleQConfig::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct EnsembleQEstimate {
    pub action: u32,
    pub mean_q: f64,
    pub member_variance: f64,
    pub mean_twin_disagreement: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct BootstrappedQEnsemble {
    feature_width: usize,
    actions: Vec<u32>,
    members: Vec<DoubleQ>,
    member_bootstrap_episode_groups: Vec<Vec<u64>>,
    support_repair_episode_draws: u64,
}

impl BootstrappedQEnsemble {
    pub fn fit(
        feature_width: usize,
        actions: &[u32],
        transitions: &[Transition],
        episode_groups: &[u64],
        config: &BootstrappedQConfig,
    ) -> Result<Self, EnsembleQError> {
        validate(actions, transitions, episode_groups, config)?;
        let mut actions = actions.to_vec();
        actions.sort_unstable();
        let mut grouped = BTreeMap::<u64, Vec<usize>>::new();
        for (row, group) in episode_groups.iter().copied().enumerate() {
            grouped.entry(group).or_default().push(row);
        }
        let group_ids = grouped.keys().copied().collect::<Vec<_>>();
        let groups_by_action = actions
            .iter()
            .map(|action| {
                group_ids
                    .iter()
                    .copied()
                    .filter(|group| {
                        grouped[group]
                            .iter()
                            .any(|row| transitions[*row].action == *action)
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let mut rng = Rng::new(config.seed ^ config.critic.seed);
        let mut members = Vec::with_capacity(config.members);
        let mut member_bootstrap_episode_groups = Vec::with_capacity(config.members);
        let mut support_repair_episode_draws = 0_u64;

        for member_index in 0..config.members {
            let mut selected_groups = (0..group_ids.len())
                .map(|_| group_ids[(rng.next_u64() % group_ids.len() as u64) as usize])
                .collect::<Vec<_>>();
            let mut selected_actions = selected_groups
                .iter()
                .flat_map(|group| grouped[group].iter())
                .map(|row| transitions[*row].action)
                .collect::<BTreeSet<_>>();
            for (action_index, action) in actions.iter().enumerate() {
                if !selected_actions.contains(action) {
                    let candidates = &groups_by_action[action_index];
                    let group = candidates[(rng.next_u64() % candidates.len() as u64) as usize];
                    selected_groups.push(group);
                    support_repair_episode_draws += 1;
                    selected_actions
                        .extend(grouped[&group].iter().map(|row| transitions[*row].action));
                }
            }
            let expanded_rows = selected_groups
                .iter()
                .map(|group| grouped[group].len())
                .sum::<usize>();
            if expanded_rows > MAX_FQI_TRANSITIONS {
                return Err(EnsembleQError::new(
                    "episode bootstrap expansion exceeds transition bound",
                ));
            }
            let member_transitions = selected_groups
                .iter()
                .flat_map(|group| grouped[group].iter())
                .map(|row| transitions[*row].clone())
                .collect::<Vec<_>>();
            let mut critic_config = config.critic.clone();
            critic_config.seed = derive_seed(config.seed ^ config.critic.seed, member_index as u64);
            let member = DoubleQ::fit(feature_width, &actions, &member_transitions, &critic_config)
                .map_err(|error| EnsembleQError::new(error.to_string()))?;
            members.push(member);
            member_bootstrap_episode_groups.push(selected_groups);
        }

        Ok(Self {
            feature_width,
            actions,
            members,
            member_bootstrap_episode_groups,
            support_repair_episode_draws,
        })
    }

    pub fn members(&self) -> usize {
        self.members.len()
    }

    pub fn support_repair_episode_draws(&self) -> u64 {
        self.support_repair_episode_draws
    }

    pub fn member_bootstrap_episode_groups(&self) -> &[Vec<u64>] {
        &self.member_bootstrap_episode_groups
    }

    pub fn rank_actions(&self, state: &[f32]) -> Result<Vec<EnsembleQEstimate>, EnsembleQError> {
        if state.len() != self.feature_width || state.iter().any(|value| !value.is_finite()) {
            return Err(EnsembleQError::new("invalid ensemble query state"));
        }
        let mut ranking = Vec::with_capacity(self.actions.len());
        for action in &self.actions {
            let estimates = self
                .members
                .iter()
                .map(|member| member.estimate(state, *action))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| EnsembleQError::new(error.to_string()))?;
            let count = estimates.len() as f64;
            let mean_q = estimates.iter().map(|estimate| estimate.mean).sum::<f64>() / count;
            ranking.push(EnsembleQEstimate {
                action: *action,
                mean_q,
                member_variance: estimates
                    .iter()
                    .map(|estimate| (estimate.mean - mean_q).powi(2))
                    .sum::<f64>()
                    / count,
                mean_twin_disagreement: estimates
                    .iter()
                    .map(|estimate| estimate.critic_disagreement)
                    .sum::<f64>()
                    / count,
            });
        }
        ranking.sort_by(|left, right| {
            right
                .mean_q
                .total_cmp(&left.mean_q)
                .then_with(|| left.member_variance.total_cmp(&right.member_variance))
                .then_with(|| left.action.cmp(&right.action))
        });
        Ok(ranking)
    }

    pub fn artifact_bytes(
        &self,
        feature_schema: Digest,
        action_schema: Digest,
        training_dataset_sha256: Option<Digest>,
        training_corpus_sha256: &[Digest],
        config: &BootstrappedQConfig,
    ) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(&EnsembleArtifact {
            schema: BOOTSTRAPPED_Q_ENSEMBLE_SCHEMA_V1,
            feature_schema,
            action_schema,
            training_dataset_sha256,
            training_corpus_sha256,
            config,
            model: self,
        })
    }
}

#[derive(Serialize)]
struct EnsembleArtifact<'a> {
    schema: &'static str,
    feature_schema: Digest,
    action_schema: Digest,
    training_dataset_sha256: Option<Digest>,
    training_corpus_sha256: &'a [Digest],
    config: &'a BootstrappedQConfig,
    model: &'a BootstrappedQEnsemble,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnsembleQError(String);

impl EnsembleQError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for EnsembleQError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for EnsembleQError {}

fn validate(
    actions: &[u32],
    transitions: &[Transition],
    episode_groups: &[u64],
    config: &BootstrappedQConfig,
) -> Result<(), EnsembleQError> {
    if actions.is_empty()
        || actions.len() > MAX_FQI_ACTIONS
        || transitions.is_empty()
        || transitions.len() > MAX_FQI_TRANSITIONS
        || transitions.len() != episode_groups.len()
        || config.members < 2
        || config.members > MAX_ENSEMBLE_MEMBERS
    {
        return Err(EnsembleQError::new("invalid bounded ensemble batch"));
    }
    let work = transitions
        .len()
        .checked_mul(config.critic.epochs)
        .and_then(|value| value.checked_mul(config.members))
        .ok_or_else(|| EnsembleQError::new("ensemble work bound overflowed"))?;
    if work > MAX_ENSEMBLE_GRADIENT_UPDATES {
        return Err(EnsembleQError::new("ensemble gradient work exceeds bound"));
    }
    let unique_actions = actions.iter().copied().collect::<BTreeSet<_>>();
    if unique_actions.len() != actions.len()
        || actions.iter().any(|action| {
            !transitions
                .iter()
                .any(|transition| transition.action == *action)
        })
    {
        return Err(EnsembleQError::new("invalid ensemble action support"));
    }
    Ok(())
}

fn derive_seed(seed: u64, stream: u64) -> u64 {
    seed ^ stream.wrapping_add(1).wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transition(state: f32, action: u32, reward: f32) -> Transition {
        Transition {
            state: vec![state],
            action,
            duration: 1,
            reward,
            next_state: vec![state + 1.0],
            terminal: true,
        }
    }

    #[test]
    fn seeded_members_resample_whole_episodes_and_repair_rare_action_support() {
        let transitions = vec![
            transition(0.0, 1, 1.0),
            transition(0.5, 1, 1.0),
            transition(1.0, 1, 1.0),
            transition(2.0, 2, 3.0),
        ];
        let groups = [10, 10, 20, 30];
        let config = BootstrappedQConfig {
            members: 5,
            seed: 7,
            critic: DoubleQConfig {
                epochs: 16,
                hidden_width: 4,
                learning_rate: 0.01,
                target_sync_steps: 8,
                ..DoubleQConfig::default()
            },
        };
        let first = BootstrappedQEnsemble::fit(1, &[1, 2], &transitions, &groups, &config).unwrap();
        let second =
            BootstrappedQEnsemble::fit(1, &[1, 2], &transitions, &groups, &config).unwrap();
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&second).unwrap()
        );
        assert_eq!(first.members(), 5);
        assert!(first.support_repair_episode_draws() > 0);
        assert!(first.member_bootstrap_episode_groups().iter().all(
            |draws| draws.len() >= 3 && draws.iter().all(|group| [10, 20, 30].contains(group))
        ));
        let ranking = first.rank_actions(&[2.0]).unwrap();
        assert_eq!(ranking.len(), 2);
        assert!(ranking.iter().all(|entry| entry.member_variance >= 0.0));
    }
}
