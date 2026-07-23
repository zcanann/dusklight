//! Learning command adapters, separated from the binary entry point.

mod algorithms;
mod commands;

pub(super) const MAX_LEARN_INPUT_CORPORA: usize = 256;

pub use algorithms::{
    command_ensemble_q, command_iql, command_option_values, command_prioritized_q,
    command_q_ablation,
};
pub use commands::command_learn;
