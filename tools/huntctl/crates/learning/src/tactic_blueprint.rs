//! Bounded blueprint-style composition over executable tactic catalog entries.
//!
//! A blueprint references existing tactic option IDs. It does not serialize or
//! reinterpret their implementation. Compilation and execution remain owned by
//! the existing static, native-observation, and reactive-controller executors.

use crate::artifact::Digest;
use crate::tactic_asset::TacticAssetCatalog;
use dusklight_control::option_execution::{OptionCondition, validate_condition};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const TACTIC_BLUEPRINT_SCHEMA_V1: &str = "dusklight-tactic-blueprint/v1";
pub const MAX_BLUEPRINT_NODES: usize = 256;
pub const MAX_BLUEPRINT_DEPTH: usize = 32;
pub const MAX_BLUEPRINT_CHILDREN: usize = 64;
pub const MAX_UNTIL_ITERATIONS: u32 = 256;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TacticBlueprint {
    pub schema: String,
    pub asset_id: String,
    pub root: TacticBlueprintNode,
}

impl TacticBlueprint {
    pub fn new(
        asset_id: impl Into<String>,
        root: TacticBlueprintNode,
    ) -> Result<Self, TacticBlueprintError> {
        let blueprint = Self {
            schema: TACTIC_BLUEPRINT_SCHEMA_V1.into(),
            asset_id: asset_id.into(),
            root,
        };
        blueprint.validate()?;
        Ok(blueprint)
    }

    pub fn validate(&self) -> Result<(), TacticBlueprintError> {
        if self.schema != TACTIC_BLUEPRINT_SCHEMA_V1 {
            return Err(TacticBlueprintError::UnsupportedSchema);
        }
        validate_portable_id(&self.asset_id).map_err(|_| TacticBlueprintError::InvalidAssetId)?;
        let mut node_count = 0;
        validate_node(&self.root, 1, &mut node_count)
    }

    pub fn validate_against_catalog(
        &self,
        catalog: &TacticAssetCatalog,
    ) -> Result<(), TacticBlueprintError> {
        self.validate()?;
        for option_id in self.referenced_option_ids() {
            if catalog.entry(option_id).is_none() {
                return Err(TacticBlueprintError::UnknownOption(option_id.into()));
            }
        }
        Ok(())
    }

    pub fn referenced_option_ids(&self) -> BTreeSet<&str> {
        let mut option_ids = BTreeSet::new();
        collect_option_ids(&self.root, &mut option_ids);
        option_ids
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, TacticBlueprintError> {
        self.validate()?;
        serde_json::to_vec(self)
            .map_err(|error| TacticBlueprintError::Serialization(error.to_string()))
    }

    pub fn content_sha256(&self) -> Result<Digest, TacticBlueprintError> {
        Ok(Digest(Sha256::digest(self.canonical_bytes()?).into()))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TacticBlueprintNode {
    /// Invoke exactly one existing, concretely parameterized catalog entry.
    Invoke { option_id: String },
    /// Execute every child in order. A child failure ends the sequence.
    Sequence { steps: Vec<TacticBlueprintNode> },
    /// Execute children concurrently under controller channel ownership rules.
    Layer { layers: Vec<TacticBlueprintNode> },
    /// Evaluate the condition at the node boundary and select one branch.
    Conditional {
        condition: OptionCondition,
        when_true: Box<TacticBlueprintNode>,
        when_false: Box<TacticBlueprintNode>,
    },
    /// Repeat the body until the condition is true or the explicit bound wins.
    Until {
        condition: OptionCondition,
        max_iterations: u32,
        body: Box<TacticBlueprintNode>,
    },
    /// Try children in order until one can start and completes successfully.
    Fallback { attempts: Vec<TacticBlueprintNode> },
}

fn validate_node(
    node: &TacticBlueprintNode,
    depth: usize,
    node_count: &mut usize,
) -> Result<(), TacticBlueprintError> {
    if depth > MAX_BLUEPRINT_DEPTH {
        return Err(TacticBlueprintError::TooDeep);
    }
    *node_count = node_count
        .checked_add(1)
        .ok_or(TacticBlueprintError::TooManyNodes)?;
    if *node_count > MAX_BLUEPRINT_NODES {
        return Err(TacticBlueprintError::TooManyNodes);
    }

    match node {
        TacticBlueprintNode::Invoke { option_id } => {
            validate_portable_id(option_id).map_err(|_| TacticBlueprintError::InvalidOptionId)
        }
        TacticBlueprintNode::Sequence { steps } => {
            validate_children(steps, 1, "sequence")?;
            for child in steps {
                validate_node(child, depth + 1, node_count)?;
            }
            Ok(())
        }
        TacticBlueprintNode::Layer { layers } => {
            validate_children(layers, 2, "layer")?;
            for child in layers {
                validate_node(child, depth + 1, node_count)?;
            }
            Ok(())
        }
        TacticBlueprintNode::Conditional {
            condition,
            when_true,
            when_false,
        } => {
            validate_guard(condition)?;
            validate_node(when_true, depth + 1, node_count)?;
            validate_node(when_false, depth + 1, node_count)
        }
        TacticBlueprintNode::Until {
            condition,
            max_iterations,
            body,
        } => {
            validate_guard(condition)?;
            if *max_iterations == 0 || *max_iterations > MAX_UNTIL_ITERATIONS {
                return Err(TacticBlueprintError::InvalidIterationBound);
            }
            validate_node(body, depth + 1, node_count)
        }
        TacticBlueprintNode::Fallback { attempts } => {
            validate_children(attempts, 2, "fallback")?;
            for child in attempts {
                validate_node(child, depth + 1, node_count)?;
            }
            Ok(())
        }
    }
}

fn validate_children(
    children: &[TacticBlueprintNode],
    minimum: usize,
    node_kind: &'static str,
) -> Result<(), TacticBlueprintError> {
    if children.len() < minimum || children.len() > MAX_BLUEPRINT_CHILDREN {
        return Err(TacticBlueprintError::InvalidChildCount(node_kind));
    }
    Ok(())
}

fn validate_guard(condition: &OptionCondition) -> Result<(), TacticBlueprintError> {
    validate_condition(condition)
        .map_err(|error| TacticBlueprintError::InvalidCondition(error.to_string()))?;
    if matches!(condition, OptionCondition::DurationElapsed) {
        return Err(TacticBlueprintError::InvalidCondition(
            "duration_elapsed has no node-boundary fact value".into(),
        ));
    }
    Ok(())
}

fn validate_portable_id(value: &str) -> Result<(), ()> {
    if value.is_empty()
        || value.len() > 96
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/'))
    {
        return Err(());
    }
    Ok(())
}

fn collect_option_ids<'a>(node: &'a TacticBlueprintNode, output: &mut BTreeSet<&'a str>) {
    match node {
        TacticBlueprintNode::Invoke { option_id } => {
            output.insert(option_id);
        }
        TacticBlueprintNode::Sequence { steps } => {
            for child in steps {
                collect_option_ids(child, output);
            }
        }
        TacticBlueprintNode::Layer { layers } => {
            for child in layers {
                collect_option_ids(child, output);
            }
        }
        TacticBlueprintNode::Conditional {
            when_true,
            when_false,
            ..
        } => {
            collect_option_ids(when_true, output);
            collect_option_ids(when_false, output);
        }
        TacticBlueprintNode::Until { body, .. } => collect_option_ids(body, output),
        TacticBlueprintNode::Fallback { attempts } => {
            for child in attempts {
                collect_option_ids(child, output);
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TacticBlueprintError {
    UnsupportedSchema,
    InvalidAssetId,
    InvalidOptionId,
    UnknownOption(String),
    TooManyNodes,
    TooDeep,
    InvalidChildCount(&'static str),
    InvalidIterationBound,
    InvalidCondition(String),
    Serialization(String),
}

impl fmt::Display for TacticBlueprintError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema => formatter.write_str("unsupported tactic blueprint schema"),
            Self::InvalidAssetId => formatter.write_str("tactic blueprint asset ID is invalid"),
            Self::InvalidOptionId => formatter.write_str("tactic blueprint option ID is invalid"),
            Self::UnknownOption(option_id) => {
                write!(
                    formatter,
                    "tactic blueprint references unknown option {option_id}"
                )
            }
            Self::TooManyNodes => formatter.write_str("tactic blueprint has too many nodes"),
            Self::TooDeep => formatter.write_str("tactic blueprint is nested too deeply"),
            Self::InvalidChildCount(node_kind) => {
                write!(
                    formatter,
                    "tactic blueprint {node_kind} has an invalid child count"
                )
            }
            Self::InvalidIterationBound => {
                formatter.write_str("tactic blueprint until bound is invalid")
            }
            Self::InvalidCondition(message) => {
                write!(
                    formatter,
                    "tactic blueprint condition is invalid: {message}"
                )
            }
            Self::Serialization(message) => {
                write!(
                    formatter,
                    "tactic blueprint serialization failed: {message}"
                )
            }
        }
    }
}

impl Error for TacticBlueprintError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tactic_asset::{TacticAssetSource, TacticCatalogEntry};
    use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};

    fn condition() -> OptionCondition {
        OptionCondition::TargetReached {
            target: "door".into(),
        }
    }

    fn invoke(option_id: &str) -> TacticBlueprintNode {
        TacticBlueprintNode::Invoke {
            option_id: option_id.into(),
        }
    }

    #[test]
    fn all_blueprint_node_kinds_round_trip_with_stable_identity() {
        let blueprint = TacticBlueprint::new(
            "route.opening",
            TacticBlueprintNode::Sequence {
                steps: vec![
                    TacticBlueprintNode::Layer {
                        layers: vec![invoke("move"), invoke("camera")],
                    },
                    TacticBlueprintNode::Conditional {
                        condition: condition(),
                        when_true: Box::new(invoke("interact")),
                        when_false: Box::new(TacticBlueprintNode::Fallback {
                            attempts: vec![invoke("roll"), invoke("wait")],
                        }),
                    },
                    TacticBlueprintNode::Until {
                        condition: condition(),
                        max_iterations: 3,
                        body: Box::new(invoke("move")),
                    },
                ],
            },
        )
        .unwrap();

        let bytes = blueprint.canonical_bytes().unwrap();
        let decoded: TacticBlueprint = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, blueprint);
        assert_eq!(
            decoded.content_sha256().unwrap(),
            blueprint.content_sha256().unwrap()
        );
        assert_eq!(
            blueprint.referenced_option_ids(),
            ["camera", "interact", "move", "roll", "wait"]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn blueprint_rejects_unknown_catalog_references() {
        let catalog = TacticAssetCatalog::new(vec![
            TacticCatalogEntry::new(
                "wait",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield {
                    frames: 1,
                })),
            )
            .unwrap(),
        ])
        .unwrap();
        let blueprint = TacticBlueprint::new(
            "unknown.reference",
            TacticBlueprintNode::Sequence {
                steps: vec![invoke("wait"), invoke("missing")],
            },
        )
        .unwrap();

        assert_eq!(
            blueprint.validate_against_catalog(&catalog).unwrap_err(),
            TacticBlueprintError::UnknownOption("missing".into())
        );
    }

    #[test]
    fn blueprint_rejects_unbounded_or_meaningless_control_flow() {
        assert_eq!(
            TacticBlueprint::new(
                "unbounded",
                TacticBlueprintNode::Until {
                    condition: condition(),
                    max_iterations: 0,
                    body: Box::new(invoke("wait")),
                },
            )
            .unwrap_err(),
            TacticBlueprintError::InvalidIterationBound
        );
        assert!(matches!(
            TacticBlueprint::new(
                "bad.guard",
                TacticBlueprintNode::Conditional {
                    condition: OptionCondition::DurationElapsed,
                    when_true: Box::new(invoke("wait")),
                    when_false: Box::new(invoke("wait")),
                },
            )
            .unwrap_err(),
            TacticBlueprintError::InvalidCondition(_)
        ));
    }
}
