//! Action-conditioned learning contract over authoritative graph evidence.

mod calibration;
mod contract;
mod exact_table;
mod graph_replay;
mod objective_double_q;
mod objective_knn;
mod treatment_comparison;

pub use calibration::{HELD_OUT_GRAPH_CALIBRATION_SCHEMA_V2, HeldOutGraphCalibrationReport};
pub use contract::{
    ActionConditionedGraphLearner, GraphActionInput, GraphBootstrapRule,
    GraphExpansionLearningTarget, GraphLearnerContract, GraphLearnerError, GraphLearningBatch,
    GraphNodeInput, GraphRankingTuple, GraphTargetSupport, GraphUncertaintyRule,
    LearnedGraphActionEstimate,
};
pub use exact_table::{ExactGraphTableLearner, ExactGraphTableSnapshot, GraphAuxiliaryPrediction};
pub use graph_replay::{GRAPH_REPLAY_PLAN_SCHEMA_V1, GraphReplayPlan, GraphReplayRowPriority};
pub use treatment_comparison::{
    GRAPH_TREATMENT_COMPARISON_SCHEMA_V1, GraphObjectiveTreatment, GraphTreatmentComparisonReport,
    GraphTreatmentMetrics,
};
