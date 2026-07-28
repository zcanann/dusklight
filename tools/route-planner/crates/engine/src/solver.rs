//! Bounded forward state search with explicit feasibility choices and proofs.

use crate::authorization::{AuthorizationGraph, AuthorizationRecorder};
use crate::evaluation::{
    EvaluatedTruth, EvidencePolicy, FeasibilityMode, FeasibilityResolution, FeasibilitySelection,
    PredicateEvaluator, RuleClassification, TransitionAssessment, TransitionClassification,
    WriterAssessment, WriterClassification,
};
use crate::execution::PlannerExecutionState;
use crate::identity::EquivalenceSet;
use crate::logic::{FactCatalog, PredicateExpression, RuleEvidence, TruthStatus};
use crate::relevance::{
    BackwardRelevance, StateDependency, dependencies_overlap, operation_outputs,
    predicate_leaf_dependencies,
};
use crate::route_book::{RouteActionRef, RouteBook, RouteDirectiveKind};
use crate::transition::{
    CandidateTransition, FeasibilityObligation, GateRule, MechanicsCatalog, PathConstraint,
    StateOperation, UnknownRequirement,
};
use crate::{PlannerContractError, artifact::Digest, validate_stable_id};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

mod contracts;
mod expansion;
mod policy;
mod search;
mod search_state;
mod witness;

pub use contracts::*;
pub use search::ForwardSolver;

use policy::*;
use search_state::*;
use witness::*;

#[cfg(test)]
mod tests;
