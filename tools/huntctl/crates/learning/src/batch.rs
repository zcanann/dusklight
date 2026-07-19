//! Shared bounded loading of compatible transition corpora for learner CLIs.

use crate::artifact::Digest;
use crate::fqi::{MAX_FQI_TRANSITIONS, Transition as FqiTransition};
use crate::transition_corpus::TransitionCorpus;
use std::error::Error;

#[derive(Clone, Debug)]
pub struct LoadedFqiBatch {
    pub feature_schema: Digest,
    pub action_schema: Digest,
    pub feature_count: usize,
    pub transitions: Vec<FqiTransition>,
    pub episode_groups: Vec<u64>,
    pub corpus_digests: Vec<Digest>,
}

pub fn load_fqi_batch(
    paths: &[String],
    label: &str,
    max_corpora: usize,
) -> Result<LoadedFqiBatch, Box<dyn Error>> {
    if paths.is_empty() || paths.len() > max_corpora {
        return Err(format!("{label} requires 1..={max_corpora} transition corpora").into());
    }
    let mut feature_schema = None;
    let mut action_schema = None;
    let mut feature_count = None;
    let mut transitions = Vec::new();
    let mut episode_groups = Vec::new();
    let mut corpus_digests = Vec::new();
    let mut next_episode_group = 0_u64;
    for path in paths {
        let corpus = TransitionCorpus::read_zstd_file(path)?;
        corpus_digests.push(corpus.content_digest()?);
        if feature_schema.is_some_and(|value| value != corpus.feature_schema)
            || action_schema.is_some_and(|value| value != corpus.action_schema)
            || feature_count.is_some_and(|value| value != corpus.feature_count)
        {
            return Err(format!("{label} corpora use incompatible schemas").into());
        }
        feature_schema = Some(corpus.feature_schema);
        action_schema = Some(corpus.action_schema);
        feature_count = Some(corpus.feature_count);
        if transitions
            .len()
            .checked_add(corpus.transitions.len())
            .is_none_or(|count| count > MAX_FQI_TRANSITIONS)
        {
            return Err(format!("{label} exceeds {MAX_FQI_TRANSITIONS} merged transitions").into());
        }
        let mut ended_terminal = false;
        for transition in corpus.transitions {
            let terminal = transition.terminal;
            transitions.push(FqiTransition {
                state: transition.state,
                action: transition.action.action_id,
                duration: transition.duration_ticks,
                reward: transition.reward,
                next_state: transition.next_state,
                terminal,
            });
            episode_groups.push(next_episode_group);
            ended_terminal = terminal;
            if terminal {
                next_episode_group = next_episode_group
                    .checked_add(1)
                    .ok_or_else(|| format!("{label} episode-group count overflowed"))?;
            }
        }
        if !ended_terminal {
            next_episode_group = next_episode_group
                .checked_add(1)
                .ok_or_else(|| format!("{label} episode-group count overflowed"))?;
        }
    }
    Ok(LoadedFqiBatch {
        feature_schema: feature_schema.ok_or_else(|| format!("{label} has no feature schema"))?,
        action_schema: action_schema.ok_or_else(|| format!("{label} has no action schema"))?,
        feature_count: feature_count.ok_or_else(|| format!("{label} has no feature width"))?
            as usize,
        transitions,
        episode_groups,
        corpus_digests,
    })
}
