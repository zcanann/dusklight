use super::*;
use std::error::Error;

#[derive(Debug)]
pub enum TacticQCampaignError {
    InvalidState(&'static str),
    CheckpointIdentityMismatch {
        stored: Digest,
        reconstructed: Digest,
    },
    Features(String),
    Tape(String),
    Io(String),
    Serialization(String),
    Frontier(String),
    LearnerState(LearnerStateError),
    Catalog(LiveTacticCatalogError),
    Exploration(TacticExplorationError),
    Transition(OptionTransitionError),
    Values(OptionValueError),
    Shaping(ShapingError),
    Hindsight(HindsightError),
    FrozenPolicy(TacticFrozenPolicyError),
    Native(NativeTacticWorkerError),
    GeneralizedValue(GeneralizedTacticValueError),
    StateGraph(StateGraphError),
    Scheduler(crate::scheduler::SchedulerError),
    GraphLearner(crate::learner::GraphLearnerError),
}

impl fmt::Display for TacticQCampaignError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidState(message) => {
                write!(formatter, "tactic-Q campaign invalid: {message}")
            }
            Self::CheckpointIdentityMismatch {
                stored,
                reconstructed,
            } => write!(
                formatter,
                "tactic-Q checkpoint identity mismatch: stored {stored}, reconstructed {reconstructed}"
            ),
            Self::Features(message) => write!(formatter, "tactic-Q features failed: {message}"),
            Self::Tape(message) => write!(formatter, "tactic-Q tape failed: {message}"),
            Self::Io(message) => write!(formatter, "tactic-Q checkpoint I/O failed: {message}"),
            Self::Serialization(message) => {
                write!(formatter, "tactic-Q serialization failed: {message}")
            }
            Self::Frontier(message) => write!(formatter, "tactic-Q frontier failed: {message}"),
            Self::LearnerState(error) => write!(formatter, "tactic-Q state failed: {error}"),
            Self::Catalog(error) => write!(formatter, "tactic-Q catalog failed: {error}"),
            Self::Exploration(error) => write!(formatter, "tactic-Q selection failed: {error}"),
            Self::Transition(error) => write!(formatter, "tactic-Q transition failed: {error}"),
            Self::Values(error) => write!(formatter, "tactic-Q refit failed: {error}"),
            Self::Shaping(error) => write!(formatter, "tactic-Q reward failed: {error}"),
            Self::Hindsight(error) => write!(formatter, "tactic-Q hindsight failed: {error}"),
            Self::FrozenPolicy(error) => write!(formatter, "tactic-Q freeze failed: {error}"),
            Self::Native(error) => write!(formatter, "tactic-Q native execution failed: {error}"),
            Self::GeneralizedValue(error) => {
                write!(formatter, "tactic-Q generalized value failed: {error}")
            }
            Self::StateGraph(error) => write!(formatter, "tactic-Q state graph failed: {error}"),
            Self::Scheduler(error) => write!(formatter, "tactic-Q scheduling failed: {error}"),
            Self::GraphLearner(error) => {
                write!(formatter, "tactic-Q learner contract failed: {error}")
            }
        }
    }
}

impl Error for TacticQCampaignError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::LearnerState(error) => Some(error),
            Self::Catalog(error) => Some(error),
            Self::Exploration(error) => Some(error),
            Self::Transition(error) => Some(error),
            Self::Values(error) => Some(error),
            Self::Shaping(error) => Some(error),
            Self::Hindsight(error) => Some(error),
            Self::FrozenPolicy(error) => Some(error),
            Self::Native(error) => Some(error),
            Self::GeneralizedValue(error) => Some(error),
            Self::StateGraph(error) => Some(error),
            Self::Scheduler(error) => Some(error),
            Self::GraphLearner(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StateGraphError> for TacticQCampaignError {
    fn from(value: StateGraphError) -> Self {
        Self::StateGraph(value)
    }
}

impl From<crate::scheduler::SchedulerError> for TacticQCampaignError {
    fn from(value: crate::scheduler::SchedulerError) -> Self {
        Self::Scheduler(value)
    }
}

impl From<crate::learner::GraphLearnerError> for TacticQCampaignError {
    fn from(value: crate::learner::GraphLearnerError) -> Self {
        Self::GraphLearner(value)
    }
}

impl From<LearnerStateError> for TacticQCampaignError {
    fn from(value: LearnerStateError) -> Self {
        Self::LearnerState(value)
    }
}

impl From<LiveTacticCatalogError> for TacticQCampaignError {
    fn from(value: LiveTacticCatalogError) -> Self {
        Self::Catalog(value)
    }
}

impl From<TacticExplorationError> for TacticQCampaignError {
    fn from(value: TacticExplorationError) -> Self {
        Self::Exploration(value)
    }
}

impl From<OptionTransitionError> for TacticQCampaignError {
    fn from(value: OptionTransitionError) -> Self {
        Self::Transition(value)
    }
}

impl From<OptionValueError> for TacticQCampaignError {
    fn from(value: OptionValueError) -> Self {
        Self::Values(value)
    }
}

impl From<ShapingError> for TacticQCampaignError {
    fn from(value: ShapingError) -> Self {
        Self::Shaping(value)
    }
}

impl From<NativeTacticWorkerError> for TacticQCampaignError {
    fn from(value: NativeTacticWorkerError) -> Self {
        Self::Native(value)
    }
}

impl From<GeneralizedTacticValueError> for TacticQCampaignError {
    fn from(value: GeneralizedTacticValueError) -> Self {
        Self::GeneralizedValue(value)
    }
}
