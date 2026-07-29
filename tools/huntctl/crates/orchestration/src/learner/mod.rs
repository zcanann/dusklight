//! Action-conditioned learning contract over authoritative graph evidence.

mod contract;

pub use contract::{
    ActionConditionedGraphLearner, GraphActionInput, GraphBootstrapRule,
    GraphExpansionLearningTarget, GraphLearnerContract, GraphLearnerError, GraphLearningBatch,
    GraphNodeInput, GraphRankingTuple, GraphTargetSupport, GraphUncertaintyRule,
    LearnedGraphActionEstimate,
};
