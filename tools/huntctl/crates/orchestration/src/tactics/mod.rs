//! Ownership of executable primitive and promoted graph actions.

use crate::state_graph::{ExactStateId, StateGraph, StateGraphError};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_learning::option_values::{OptionActionDescriptor, OptionValueError};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const GRAPH_TACTIC_CATALOG_SCHEMA_V1: &str = "dusklight-graph-tactic-catalog/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PromotedGraphTactic {
    pub descriptor: OptionActionDescriptor,
    pub primitive_components: Vec<Digest>,
    pub held_out_evidence_sha256: Digest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphTacticCatalog {
    pub schema: String,
    primitives: BTreeMap<Digest, OptionActionDescriptor>,
    promoted: BTreeMap<Digest, PromotedGraphTactic>,
}

impl GraphTacticCatalog {
    pub fn new(
        primitives: impl IntoIterator<Item = OptionActionDescriptor>,
    ) -> Result<Self, TacticsError> {
        let mut catalog = Self {
            schema: GRAPH_TACTIC_CATALOG_SCHEMA_V1.into(),
            primitives: BTreeMap::new(),
            promoted: BTreeMap::new(),
        };
        for descriptor in primitives {
            descriptor.validate()?;
            let identity = descriptor.content_sha256()?;
            if catalog.primitives.insert(identity, descriptor).is_some() {
                return Err(TacticsError::Invalid(
                    "primitive tactic catalog contains a duplicate action",
                ));
            }
        }
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn promote(&mut self, tactic: PromotedGraphTactic) -> Result<Digest, TacticsError> {
        tactic.descriptor.validate()?;
        if tactic.held_out_evidence_sha256 == Digest::ZERO
            || tactic.primitive_components.is_empty()
            || tactic
                .primitive_components
                .iter()
                .any(|component| !self.primitives.contains_key(component))
            || tactic
                .primitive_components
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != tactic.primitive_components.len()
        {
            return Err(TacticsError::Invalid(
                "promoted tactic lacks independent primitive evidence",
            ));
        }
        let identity = tactic.descriptor.content_sha256()?;
        if self.primitives.contains_key(&identity) {
            return Err(TacticsError::Invalid(
                "promoted tactic duplicates an existing primitive",
            ));
        }
        match self.promoted.get(&identity) {
            Some(existing) if existing == &tactic => return Ok(identity),
            Some(_) => return Err(TacticsError::Collision),
            None => {}
        }
        self.promoted.insert(identity, tactic);
        self.validate()?;
        Ok(identity)
    }

    pub fn primitive_count(&self) -> usize {
        self.primitives.len()
    }

    pub fn promoted_count(&self) -> usize {
        self.promoted.len()
    }

    pub fn actions(&self) -> impl Iterator<Item = &OptionActionDescriptor> {
        self.primitives
            .values()
            .chain(self.promoted.values().map(|tactic| &tactic.descriptor))
    }

    /// Register every currently applicable action on one exact graph node.
    /// Applicability is supplied by the typed environment, not the catalog.
    pub fn register_applicable<F>(
        &self,
        graph: &mut StateGraph,
        source: ExactStateId,
        mut applicable: F,
    ) -> Result<Vec<Digest>, TacticsError>
    where
        F: FnMut(&OptionActionDescriptor) -> bool,
    {
        graph.validate()?;
        if graph.node(source).is_none() {
            return Err(TacticsError::Invalid(
                "tactic registration source is absent",
            ));
        }
        let mut registered = Vec::new();
        for action in self.actions().filter(|action| applicable(action)) {
            registered.push(graph.register_action_expansion(source, action.clone())?);
        }
        Ok(registered)
    }

    pub fn validate(&self) -> Result<(), TacticsError> {
        if self.schema != GRAPH_TACTIC_CATALOG_SCHEMA_V1 || self.primitives.is_empty() {
            return Err(TacticsError::Invalid("graph tactic catalog is invalid"));
        }
        for (identity, descriptor) in &self.primitives {
            descriptor.validate()?;
            if descriptor.content_sha256()? != *identity {
                return Err(TacticsError::Invalid(
                    "primitive tactic identity is detached",
                ));
            }
        }
        for (identity, tactic) in &self.promoted {
            tactic.descriptor.validate()?;
            if tactic.descriptor.content_sha256()? != *identity
                || self.primitives.contains_key(identity)
                || tactic.held_out_evidence_sha256 == Digest::ZERO
                || tactic.primitive_components.is_empty()
                || tactic
                    .primitive_components
                    .iter()
                    .any(|component| !self.primitives.contains_key(component))
                || tactic
                    .primitive_components
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .len()
                    != tactic.primitive_components.len()
            {
                return Err(TacticsError::Invalid(
                    "promoted tactic evidence is detached",
                ));
            }
        }
        Ok(())
    }

    pub fn content_sha256(&self) -> Result<Digest, TacticsError> {
        self.validate()?;
        let bytes = serde_cbor::to_vec(self)
            .map_err(|error| TacticsError::Serialization(error.to_string()))?;
        Ok(Digest(Sha256::digest(bytes).into()))
    }
}

#[derive(Debug)]
pub enum TacticsError {
    Invalid(&'static str),
    Collision,
    Serialization(String),
    Graph(StateGraphError),
    Action(OptionValueError),
}

impl fmt::Display for TacticsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(formatter, "invalid graph tactics: {message}"),
            Self::Collision => formatter.write_str("graph tactic content identity collision"),
            Self::Serialization(message) => {
                write!(formatter, "graph tactic serialization failed: {message}")
            }
            Self::Graph(error) => write!(formatter, "graph tactic state failed: {error}"),
            Self::Action(error) => write!(formatter, "graph tactic action failed: {error}"),
        }
    }
}

impl Error for TacticsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Graph(error) => Some(error),
            Self::Action(error) => Some(error),
            Self::Invalid(_) | Self::Collision | Self::Serialization(_) => None,
        }
    }
}

impl From<StateGraphError> for TacticsError {
    fn from(value: StateGraphError) -> Self {
        Self::Graph(value)
    }
}

impl From<OptionValueError> for TacticsError {
    fn from(value: OptionValueError) -> Self {
        Self::Action(value)
    }
}
