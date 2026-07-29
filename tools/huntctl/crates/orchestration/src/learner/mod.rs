//! Action-conditioned learning contract over authoritative graph evidence.

mod contract;
mod exact_table;

pub use contract::{
    ActionConditionedGraphLearner, GraphActionInput, GraphBootstrapRule,
    GraphExpansionLearningTarget, GraphLearnerContract, GraphLearnerError, GraphLearningBatch,
    GraphNodeInput, GraphRankingTuple, GraphTargetSupport, GraphUncertaintyRule,
    LearnedGraphActionEstimate,
};
pub use exact_table::{ExactGraphTableLearner, ExactGraphTableSnapshot, GraphAuxiliaryPrediction};
