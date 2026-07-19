use crate::fqi::Transition;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

/// Collapse observed episode suffixes into bounded semi-Markov n-step rows.
/// A truncated episode remains non-terminal and bootstraps from its final
/// observed next state; a terminal row never crosses into another episode.
pub fn aggregate_n_step(
    transitions: &[Transition],
    episode_groups: &[u64],
    steps: usize,
    discount: f64,
) -> Result<Vec<Transition>, NStepError> {
    if transitions.is_empty() {
        return Err(NStepError::Invalid("transitions must be non-empty"));
    }
    if episode_groups.len() != transitions.len() {
        return Err(NStepError::Invalid("episode groups must match transitions"));
    }
    if steps == 0 || steps > super::MAX_RAINBOW_N_STEP {
        return Err(NStepError::Invalid("steps must be within 1..=64"));
    }
    if !discount.is_finite() || !(0.0..=1.0).contains(&discount) {
        return Err(NStepError::Invalid(
            "discount must be finite and within [0, 1]",
        ));
    }
    let mut successors = vec![None; transitions.len()];
    let mut last_by_group = BTreeMap::<u64, usize>::new();
    for (row, group) in episode_groups.iter().copied().enumerate() {
        if let Some(previous) = last_by_group.insert(group, row) {
            successors[previous] = Some(row);
        }
    }

    let mut output = Vec::with_capacity(transitions.len());
    for start in 0..transitions.len() {
        let first = &transitions[start];
        let mut row = start;
        let mut reward = 0.0_f64;
        let mut reward_discount = 1.0_f64;
        let mut total_duration = 0_u32;
        let mut last = first;
        for step in 0..steps {
            let current = &transitions[row];
            reward += reward_discount * f64::from(current.reward);
            total_duration = total_duration
                .checked_add(current.duration)
                .ok_or(NStepError::DurationOverflow)?;
            reward_discount *= discount.powf(f64::from(current.duration));
            last = current;
            if current.terminal || step + 1 == steps {
                break;
            }
            match successors[row] {
                Some(next) => row = next,
                None => break,
            }
        }
        if !reward.is_finite() || reward < f64::from(f32::MIN) || reward > f64::from(f32::MAX) {
            return Err(NStepError::NonFiniteReward);
        }
        output.push(Transition {
            state: first.state.clone(),
            action: first.action,
            duration: total_duration,
            reward: reward as f32,
            next_state: last.next_state.clone(),
            terminal: last.terminal,
        });
    }
    Ok(output)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NStepError {
    Invalid(&'static str),
    DurationOverflow,
    NonFiniteReward,
}

impl fmt::Display for NStepError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid n-step input: {message}"),
            Self::DurationOverflow => formatter.write_str("n-step duration overflowed u32"),
            Self::NonFiniteReward => formatter.write_str("n-step reward is not finite f32"),
        }
    }
}

impl Error for NStepError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(state: f32, reward: f32, next: f32, terminal: bool, duration: u32) -> Transition {
        Transition {
            state: vec![state],
            action: 1,
            duration,
            reward,
            next_state: vec![next],
            terminal,
        }
    }

    #[test]
    fn terminal_and_truncated_suffixes_remain_distinct() {
        let rows = vec![
            row(0.0, 1.0, 1.0, false, 1),
            row(1.0, 2.0, 2.0, true, 2),
            row(10.0, 3.0, 11.0, false, 1),
        ];
        let aggregated = aggregate_n_step(&rows, &[7, 7, 9], 3, 0.5).unwrap();
        assert_eq!(aggregated[0].reward, 2.0);
        assert_eq!(aggregated[0].duration, 3);
        assert_eq!(aggregated[0].next_state, [2.0]);
        assert!(aggregated[0].terminal);
        assert_eq!(aggregated[2].reward, 3.0);
        assert_eq!(aggregated[2].duration, 1);
        assert!(!aggregated[2].terminal);
    }

    #[test]
    fn interleaved_episode_rows_follow_their_own_group() {
        let rows = vec![
            row(0.0, 1.0, 1.0, false, 1),
            row(10.0, 10.0, 11.0, false, 1),
            row(1.0, 2.0, 2.0, true, 1),
            row(11.0, 20.0, 12.0, true, 1),
        ];
        let aggregated = aggregate_n_step(&rows, &[1, 2, 1, 2], 2, 1.0).unwrap();
        assert_eq!(aggregated[0].reward, 3.0);
        assert_eq!(aggregated[0].next_state, [2.0]);
        assert_eq!(aggregated[1].reward, 30.0);
        assert_eq!(aggregated[1].next_state, [12.0]);
    }
}
