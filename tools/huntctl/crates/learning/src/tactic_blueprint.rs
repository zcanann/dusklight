//! Bounded blueprint-style composition over executable tactic catalog entries.
//!
//! A blueprint references existing tactic option IDs. It does not serialize or
//! reinterpret their implementation. Compilation and execution remain owned by
//! the existing static, native-observation, and reactive-controller executors.

use crate::artifact::Digest;
use crate::tactic_asset::{
    TacticAssetCatalog, TacticAssetDescription, TacticAssetSource, TacticCatalogEntry,
    TacticDurationBounds, TacticExecutor,
};
use dusklight_control::controller_compilation::compile_static_controller;
use dusklight_control::controller_program::ControllerProgram;
use dusklight_control::option_execution::{
    MAX_OPTION_TICKS, OptionCondition, OptionEndReason, OptionExecution, OptionParameter,
    OptionType, TapeRange, validate_condition,
};
use dusklight_control::tape::{InputFrame, InputTape};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const TACTIC_BLUEPRINT_SCHEMA_V1: &str = "dusklight-tactic-blueprint/v1";
pub const MAX_BLUEPRINT_NODES: usize = 256;
pub const MAX_BLUEPRINT_DEPTH: usize = 32;
pub const MAX_BLUEPRINT_CHILDREN: usize = 64;
pub const MAX_UNTIL_ITERATIONS: u32 = 256;
pub const COMPILED_STATIC_BLUEPRINT_SCHEMA_V1: &str =
    "dusklight-compiled-static-tactic-blueprint/v1";
pub const APPLICABLE_TACTIC_CHOICES_SCHEMA_V1: &str = "dusklight-applicable-tactic-choices/v1";
pub const MAX_APPLICABLE_TACTIC_CHOICES: usize = 1_024;

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

    pub fn validate_for_execution<F>(
        &self,
        catalog: &TacticAssetCatalog,
        mut condition_available: F,
    ) -> Result<(), TacticBlueprintError>
    where
        F: FnMut(&OptionCondition) -> bool,
    {
        self.validate_against_catalog(catalog)?;
        validate_available_conditions(&self.root, &mut condition_available)
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

    pub fn compile_static(
        &self,
        catalog: &TacticAssetCatalog,
    ) -> Result<CompiledStaticBlueprint, TacticBlueprintError> {
        self.validate_against_catalog(catalog)?;
        let mut frames = Vec::new();
        let mut pending = Vec::new();
        compile_static_node(&self.root, "root", catalog, &mut frames, &mut pending)?;
        let tape = InputTape {
            frames,
            ..InputTape::default()
        };
        tape.validate()
            .map_err(|error| TacticBlueprintError::StaticExecution(error.to_string()))?;
        let segments = pending
            .into_iter()
            .map(|segment| segment.finish(&tape, catalog))
            .collect::<Result<Vec<_>, _>>()?;
        let compiled = CompiledStaticBlueprint {
            schema: COMPILED_STATIC_BLUEPRINT_SCHEMA_V1.into(),
            blueprint_sha256: self.content_sha256()?,
            catalog_action_schema_sha256: catalog.action_schema_sha256(),
            tape,
            segments,
        };
        compiled.validate()?;
        Ok(compiled)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompiledStaticBlueprint {
    pub schema: String,
    pub blueprint_sha256: Digest,
    pub catalog_action_schema_sha256: Digest,
    pub tape: InputTape,
    pub segments: Vec<CompiledStaticSegment>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompiledStaticSegment {
    Invoke {
        node_path: String,
        option_id: String,
        execution: OptionExecution,
    },
    Layer {
        node_path: String,
        option_ids: Vec<String>,
        controller_sha256: Digest,
        tape_range: TapeRange,
        emitted_raw_actions: Vec<InputFrame>,
    },
}

impl CompiledStaticBlueprint {
    pub fn validate(&self) -> Result<(), TacticBlueprintError> {
        if self.schema != COMPILED_STATIC_BLUEPRINT_SCHEMA_V1
            || self.blueprint_sha256 == Digest::ZERO
            || self.catalog_action_schema_sha256 == Digest::ZERO
            || self.segments.is_empty()
        {
            return Err(TacticBlueprintError::InvalidCompiled(
                "compiled blueprint identity or segment set is invalid".into(),
            ));
        }
        self.tape
            .validate()
            .map_err(|error| TacticBlueprintError::InvalidCompiled(error.to_string()))?;
        let mut expected_start = 0_u64;
        for segment in &self.segments {
            let (node_path, range) = match segment {
                CompiledStaticSegment::Invoke {
                    node_path,
                    option_id,
                    execution,
                } => {
                    if !valid_node_path(node_path) || !valid_portable_id_value(option_id) {
                        return Err(TacticBlueprintError::InvalidCompiled(
                            "compiled invoke identity is invalid".into(),
                        ));
                    }
                    execution
                        .validate_against_tape(&self.tape)
                        .map_err(|error| {
                            TacticBlueprintError::InvalidCompiled(error.to_string())
                        })?;
                    (node_path, execution.realized_tape_range)
                }
                CompiledStaticSegment::Layer {
                    node_path,
                    option_ids,
                    controller_sha256,
                    tape_range,
                    emitted_raw_actions,
                } => {
                    if !valid_node_path(node_path)
                        || option_ids.len() < 2
                        || option_ids.iter().any(|id| !valid_portable_id_value(id))
                        || *controller_sha256 == Digest::ZERO
                        || tape_range.end_frame_exclusive <= tape_range.start_frame
                        || tape_range.end_frame_exclusive - tape_range.start_frame
                            != emitted_raw_actions.len() as u64
                    {
                        return Err(TacticBlueprintError::InvalidCompiled(
                            "compiled layer identity or range is invalid".into(),
                        ));
                    }
                    let start = usize::try_from(tape_range.start_frame).map_err(|_| {
                        TacticBlueprintError::InvalidCompiled(
                            "compiled layer start is out of range".into(),
                        )
                    })?;
                    let end = usize::try_from(tape_range.end_frame_exclusive).map_err(|_| {
                        TacticBlueprintError::InvalidCompiled(
                            "compiled layer end is out of range".into(),
                        )
                    })?;
                    if self.tape.frames.get(start..end) != Some(emitted_raw_actions.as_slice()) {
                        return Err(TacticBlueprintError::InvalidCompiled(
                            "compiled layer frames differ from the exact tape".into(),
                        ));
                    }
                    (node_path, *tape_range)
                }
            };
            if node_path.is_empty() || range.start_frame != expected_start {
                return Err(TacticBlueprintError::InvalidCompiled(
                    "compiled blueprint segments are not contiguous and ordered".into(),
                ));
            }
            expected_start = range.end_frame_exclusive;
        }
        if expected_start != self.tape.frames.len() as u64 {
            return Err(TacticBlueprintError::InvalidCompiled(
                "compiled blueprint segments do not cover the exact tape".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConcreteTacticChoiceKind {
    CatalogEntry,
    Blueprint,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConcreteTacticChoice {
    pub choice_id: String,
    pub kind: ConcreteTacticChoiceKind,
    pub descriptor: crate::option_values::OptionActionDescriptor,
    pub duration: TacticDurationBounds,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicableTacticChoices {
    pub schema: String,
    pub catalog_action_schema_sha256: Digest,
    pub choice_schema_sha256: Digest,
    pub applicability_sha256: Digest,
    pub candidates: Vec<ConcreteTacticChoice>,
    pub applicable_mask: Vec<bool>,
}

impl ApplicableTacticChoices {
    pub fn enumerate<FA, FC>(
        catalog: &TacticAssetCatalog,
        blueprints: &[TacticBlueprint],
        mut entry_applicable: FA,
        mut condition_value: FC,
    ) -> Result<Self, TacticBlueprintError>
    where
        FA: FnMut(&TacticAssetDescription) -> bool,
        FC: FnMut(&OptionCondition) -> Option<bool>,
    {
        if catalog.entries().len().saturating_add(blueprints.len()) > MAX_APPLICABLE_TACTIC_CHOICES
        {
            return Err(TacticBlueprintError::TooManyApplicableChoices);
        }
        let applicable_entries = catalog
            .entries()
            .iter()
            .filter(|entry| entry_applicable(entry.description()))
            .map(|entry| entry.option_id())
            .collect::<BTreeSet<_>>();
        let mut choices = catalog
            .entries()
            .iter()
            .map(|entry| {
                (
                    ConcreteTacticChoice {
                        choice_id: entry.option_id().into(),
                        kind: ConcreteTacticChoiceKind::CatalogEntry,
                        descriptor: entry.description().option.clone(),
                        duration: entry.description().duration,
                    },
                    applicable_entries.contains(entry.option_id()),
                )
            })
            .collect::<Vec<_>>();

        let mut blueprint_ids = BTreeSet::new();
        for blueprint in blueprints {
            blueprint.validate_against_catalog(catalog)?;
            if !blueprint_ids.insert(blueprint.asset_id.as_str()) {
                return Err(TacticBlueprintError::DuplicateBlueprintId(
                    blueprint.asset_id.clone(),
                ));
            }
            let applicable = blueprint_start_applicable(
                &blueprint.root,
                &applicable_entries,
                &mut condition_value,
            );
            let blueprint_sha256 = blueprint.content_sha256()?;
            let mut parameters = BTreeMap::new();
            parameters.insert(
                "blueprint_sha256".into(),
                OptionParameter::Digest(blueprint_sha256),
            );
            parameters.insert(
                "catalog_action_schema_sha256".into(),
                OptionParameter::Digest(catalog.action_schema_sha256()),
            );
            let choice_id = format!("blueprint/{}", blueprint.asset_id);
            choices.push((
                ConcreteTacticChoice {
                    descriptor: crate::option_values::OptionActionDescriptor {
                        option_id: choice_id.clone(),
                        option_type: OptionType::Custom("tactic_blueprint".into()),
                        parameters,
                    },
                    choice_id,
                    kind: ConcreteTacticChoiceKind::Blueprint,
                    duration: blueprint_duration(&blueprint.root, catalog)?,
                },
                applicable,
            ));
        }
        choices.sort_by(|left, right| left.0.choice_id.cmp(&right.0.choice_id));
        if choices
            .windows(2)
            .any(|pair| pair[0].0.choice_id == pair[1].0.choice_id)
        {
            return Err(TacticBlueprintError::DuplicateChoiceId);
        }
        let (candidates, applicable_mask): (Vec<_>, Vec<_>) = choices.into_iter().unzip();
        let choice_schema_sha256 = Digest(
            Sha256::digest(
                serde_json::to_vec(&(
                    APPLICABLE_TACTIC_CHOICES_SCHEMA_V1,
                    catalog.action_schema_sha256(),
                    &candidates,
                ))
                .map_err(|error| TacticBlueprintError::Serialization(error.to_string()))?,
            )
            .into(),
        );
        let applicability_sha256 = Digest(
            Sha256::digest(
                serde_json::to_vec(&(
                    APPLICABLE_TACTIC_CHOICES_SCHEMA_V1,
                    choice_schema_sha256,
                    &applicable_mask,
                ))
                .map_err(|error| TacticBlueprintError::Serialization(error.to_string()))?,
            )
            .into(),
        );
        Ok(Self {
            schema: APPLICABLE_TACTIC_CHOICES_SCHEMA_V1.into(),
            catalog_action_schema_sha256: catalog.action_schema_sha256(),
            choice_schema_sha256,
            applicability_sha256,
            candidates,
            applicable_mask,
        })
    }
}

fn blueprint_start_applicable<FC>(
    node: &TacticBlueprintNode,
    applicable_entries: &BTreeSet<&str>,
    condition_value: &mut FC,
) -> bool
where
    FC: FnMut(&OptionCondition) -> Option<bool>,
{
    match node {
        TacticBlueprintNode::Invoke { option_id } => {
            applicable_entries.contains(option_id.as_str())
        }
        TacticBlueprintNode::Sequence { steps } => steps.first().is_some_and(|first| {
            blueprint_start_applicable(first, applicable_entries, condition_value)
        }),
        TacticBlueprintNode::Layer { layers } => layers
            .iter()
            .all(|layer| blueprint_start_applicable(layer, applicable_entries, condition_value)),
        TacticBlueprintNode::Conditional {
            condition,
            when_true,
            when_false,
        } => match condition_value(condition) {
            Some(true) => {
                blueprint_start_applicable(when_true, applicable_entries, condition_value)
            }
            Some(false) => {
                blueprint_start_applicable(when_false, applicable_entries, condition_value)
            }
            None => false,
        },
        TacticBlueprintNode::Until {
            condition, body, ..
        } => match condition_value(condition) {
            // An already-satisfied loop emits no option and is not a concrete
            // learner action at this decision boundary.
            Some(true) | None => false,
            Some(false) => blueprint_start_applicable(body, applicable_entries, condition_value),
        },
        TacticBlueprintNode::Fallback { attempts } => attempts.iter().any(|attempt| {
            blueprint_start_applicable(attempt, applicable_entries, condition_value)
        }),
    }
}

fn blueprint_duration(
    node: &TacticBlueprintNode,
    catalog: &TacticAssetCatalog,
) -> Result<TacticDurationBounds, TacticBlueprintError> {
    let (minimum_ticks, maximum_ticks) = match node {
        TacticBlueprintNode::Invoke { option_id } => {
            let duration = catalog
                .entry(option_id)
                .ok_or_else(|| TacticBlueprintError::UnknownOption(option_id.clone()))?
                .description()
                .duration;
            (duration.minimum_ticks, duration.maximum_ticks)
        }
        TacticBlueprintNode::Sequence { steps } => {
            let mut minimum = 0_u32;
            let mut maximum = 0_u32;
            for child in steps {
                let duration = blueprint_duration(child, catalog)?;
                minimum = checked_duration_add(minimum, duration.minimum_ticks)?;
                maximum = checked_duration_add(maximum, duration.maximum_ticks)?;
            }
            (minimum, maximum)
        }
        TacticBlueprintNode::Layer { layers } => {
            let mut minimum = 0_u32;
            let mut maximum = 0_u32;
            for child in layers {
                let duration = blueprint_duration(child, catalog)?;
                minimum = minimum.max(duration.minimum_ticks);
                maximum = maximum.max(duration.maximum_ticks);
            }
            (minimum, maximum)
        }
        TacticBlueprintNode::Conditional {
            when_true,
            when_false,
            ..
        } => {
            let when_true = blueprint_duration(when_true, catalog)?;
            let when_false = blueprint_duration(when_false, catalog)?;
            (
                when_true.minimum_ticks.min(when_false.minimum_ticks),
                when_true.maximum_ticks.max(when_false.maximum_ticks),
            )
        }
        TacticBlueprintNode::Until {
            max_iterations,
            body,
            ..
        } => {
            let body = blueprint_duration(body, catalog)?;
            (
                body.minimum_ticks,
                checked_duration_mul(body.maximum_ticks, *max_iterations)?,
            )
        }
        TacticBlueprintNode::Fallback { attempts } => {
            let mut minimum = u32::MAX;
            let mut maximum = 0_u32;
            for child in attempts {
                let duration = blueprint_duration(child, catalog)?;
                minimum = minimum.min(duration.minimum_ticks);
                maximum = checked_duration_add(maximum, duration.maximum_ticks)?;
            }
            (minimum, maximum)
        }
    };
    if minimum_ticks == 0 || minimum_ticks > maximum_ticks || maximum_ticks > MAX_OPTION_TICKS {
        return Err(TacticBlueprintError::InvalidCompositeDuration);
    }
    Ok(TacticDurationBounds {
        minimum_ticks,
        maximum_ticks,
    })
}

fn checked_duration_add(left: u32, right: u32) -> Result<u32, TacticBlueprintError> {
    left.checked_add(right)
        .filter(|value| *value <= MAX_OPTION_TICKS)
        .ok_or(TacticBlueprintError::InvalidCompositeDuration)
}

fn checked_duration_mul(left: u32, right: u32) -> Result<u32, TacticBlueprintError> {
    left.checked_mul(right)
        .filter(|value| *value <= MAX_OPTION_TICKS)
        .ok_or(TacticBlueprintError::InvalidCompositeDuration)
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

enum PendingStaticSegment {
    Invoke {
        node_path: String,
        option_id: String,
        tape_range: TapeRange,
    },
    Layer {
        node_path: String,
        option_ids: Vec<String>,
        controller_sha256: Digest,
        tape_range: TapeRange,
        emitted_raw_actions: Vec<InputFrame>,
    },
}

impl PendingStaticSegment {
    fn finish(
        self,
        tape: &InputTape,
        catalog: &TacticAssetCatalog,
    ) -> Result<CompiledStaticSegment, TacticBlueprintError> {
        match self {
            Self::Invoke {
                node_path,
                option_id,
                tape_range,
            } => {
                let entry = catalog
                    .entry(&option_id)
                    .ok_or_else(|| TacticBlueprintError::UnknownOption(option_id.clone()))?;
                let execution = capture_entry_execution(entry, tape, tape_range)?;
                Ok(CompiledStaticSegment::Invoke {
                    node_path,
                    option_id,
                    execution,
                })
            }
            Self::Layer {
                node_path,
                option_ids,
                controller_sha256,
                tape_range,
                emitted_raw_actions,
            } => Ok(CompiledStaticSegment::Layer {
                node_path,
                option_ids,
                controller_sha256,
                tape_range,
                emitted_raw_actions,
            }),
        }
    }
}

fn compile_static_node(
    node: &TacticBlueprintNode,
    node_path: &str,
    catalog: &TacticAssetCatalog,
    frames: &mut Vec<InputFrame>,
    segments: &mut Vec<PendingStaticSegment>,
) -> Result<(), TacticBlueprintError> {
    match node {
        TacticBlueprintNode::Invoke { option_id } => {
            let entry = catalog
                .entry(option_id)
                .ok_or_else(|| TacticBlueprintError::UnknownOption(option_id.clone()))?;
            let start_frame = frames.len() as u64;
            frames.extend(realize_entry_frames(entry, start_frame)?);
            let end_frame_exclusive = frames.len() as u64;
            segments.push(PendingStaticSegment::Invoke {
                node_path: node_path.into(),
                option_id: option_id.clone(),
                tape_range: TapeRange {
                    start_frame,
                    end_frame_exclusive,
                },
            });
            Ok(())
        }
        TacticBlueprintNode::Sequence { steps } => {
            for (index, child) in steps.iter().enumerate() {
                compile_static_node(
                    child,
                    &format!("{node_path}/sequence[{index}]"),
                    catalog,
                    frames,
                    segments,
                )?;
            }
            Ok(())
        }
        TacticBlueprintNode::Layer { .. } => {
            let (controller, option_ids) = compile_controller_layer_with_ids(node, catalog)?;
            let controller_sha256 =
                Digest(
                    Sha256::digest(controller.encode().map_err(|error| {
                        TacticBlueprintError::StaticExecution(error.to_string())
                    })?)
                    .into(),
                );
            let layer_tape = compile_static_controller(&controller)
                .map_err(|error| TacticBlueprintError::StaticExecution(error.to_string()))?;
            let start_frame = frames.len() as u64;
            let emitted_raw_actions = layer_tape.frames;
            frames.extend(emitted_raw_actions.iter().cloned());
            let end_frame_exclusive = frames.len() as u64;
            segments.push(PendingStaticSegment::Layer {
                node_path: node_path.into(),
                option_ids,
                controller_sha256,
                tape_range: TapeRange {
                    start_frame,
                    end_frame_exclusive,
                },
                emitted_raw_actions,
            });
            Ok(())
        }
        TacticBlueprintNode::Conditional { .. } => {
            Err(TacticBlueprintError::UnsupportedStaticNode("conditional"))
        }
        TacticBlueprintNode::Until { .. } => {
            Err(TacticBlueprintError::UnsupportedStaticNode("until"))
        }
        TacticBlueprintNode::Fallback { .. } => {
            Err(TacticBlueprintError::UnsupportedStaticNode("fallback"))
        }
    }
}

pub fn compile_controller_layer(
    node: &TacticBlueprintNode,
    catalog: &TacticAssetCatalog,
) -> Result<ControllerProgram, TacticBlueprintError> {
    compile_controller_layer_with_ids(node, catalog).map(|(program, _)| program)
}

fn compile_controller_layer_with_ids(
    node: &TacticBlueprintNode,
    catalog: &TacticAssetCatalog,
) -> Result<(ControllerProgram, Vec<String>), TacticBlueprintError> {
    let mut duration_frames = 0;
    let mut layers = Vec::new();
    let mut option_ids = Vec::new();
    collect_controller_layers(
        node,
        catalog,
        &mut duration_frames,
        &mut layers,
        &mut option_ids,
    )?;
    let program = ControllerProgram {
        duration_frames,
        layers,
    };
    program
        .validate()
        .map_err(|error| TacticBlueprintError::StaticExecution(error.to_string()))?;
    Ok((program, option_ids))
}

fn collect_controller_layers(
    node: &TacticBlueprintNode,
    catalog: &TacticAssetCatalog,
    duration_frames: &mut u32,
    layers: &mut Vec<dusklight_control::controller_program::Layer>,
    option_ids: &mut Vec<String>,
) -> Result<(), TacticBlueprintError> {
    match node {
        TacticBlueprintNode::Invoke { option_id } => {
            let entry = catalog
                .entry(option_id)
                .ok_or_else(|| TacticBlueprintError::UnknownOption(option_id.clone()))?;
            let TacticAssetSource::ReactiveController(program) = entry.source() else {
                return Err(TacticBlueprintError::StaticExecution(format!(
                    "layer child {option_id:?} is not an existing controller program"
                )));
            };
            *duration_frames = (*duration_frames).max(program.duration_frames);
            layers.extend(program.layers.iter().cloned());
            option_ids.push(option_id.clone());
            Ok(())
        }
        TacticBlueprintNode::Layer {
            layers: child_layers,
        } => {
            for child in child_layers {
                collect_controller_layers(child, catalog, duration_frames, layers, option_ids)?;
            }
            Ok(())
        }
        _ => Err(TacticBlueprintError::StaticExecution(
            "layer children must be controller invokes or nested layers".into(),
        )),
    }
}

fn realize_entry_frames(
    entry: &TacticCatalogEntry,
    start_frame: u64,
) -> Result<Vec<InputFrame>, TacticBlueprintError> {
    if entry.description().executor != TacticExecutor::StaticPlan {
        return Err(TacticBlueprintError::StaticExecution(format!(
            "option {:?} requires an observation-driven executor",
            entry.option_id()
        )));
    }
    match entry.source() {
        TacticAssetSource::GameTactic(plan) => plan
            .realize(None)
            .map(|realization| realization.frames)
            .map_err(|error| TacticBlueprintError::StaticExecution(error.to_string())),
        TacticAssetSource::MotionPath(plan) => plan
            .realize(None)
            .map(|realization| realization.frames)
            .map_err(|error| TacticBlueprintError::StaticExecution(error.to_string())),
        TacticAssetSource::Roll(plan) => plan
            .realize(start_frame, None)
            .map(|realization| realization.frames)
            .map_err(|error| TacticBlueprintError::StaticExecution(error.to_string())),
        TacticAssetSource::ReactiveController(program) => compile_static_controller(program)
            .map(|tape| tape.frames)
            .map_err(|error| TacticBlueprintError::StaticExecution(error.to_string())),
        TacticAssetSource::NativeGenericTactic(_) => Err(TacticBlueprintError::StaticExecution(
            format!("option {:?} has no static realization", entry.option_id()),
        )),
    }
}

fn capture_entry_execution(
    entry: &TacticCatalogEntry,
    tape: &InputTape,
    range: TapeRange,
) -> Result<OptionExecution, TacticBlueprintError> {
    let execution = match entry.source() {
        TacticAssetSource::GameTactic(plan) => plan
            .capture_execution(entry.option_id().into(), tape, range, None)
            .map_err(|error| TacticBlueprintError::StaticExecution(error.to_string()))?,
        TacticAssetSource::MotionPath(plan) => plan
            .capture_execution(entry.option_id().into(), tape, range, None)
            .map_err(|error| TacticBlueprintError::StaticExecution(error.to_string()))?,
        TacticAssetSource::Roll(plan) => plan
            .capture_execution(entry.option_id().into(), tape, range, None)
            .map_err(|error| TacticBlueprintError::StaticExecution(error.to_string()))?,
        TacticAssetSource::ReactiveController(_) => {
            let description = entry.description();
            OptionExecution::capture(
                entry.option_id().into(),
                description.option.option_type.clone(),
                description.option.parameters.clone(),
                description.duration.minimum_ticks,
                description.duration.maximum_ticks,
                description.stopping.termination.clone(),
                description.stopping.cancellation.clone(),
                OptionEndReason::Completed,
                tape,
                range,
            )
            .map_err(|error| TacticBlueprintError::StaticExecution(error.to_string()))?
        }
        TacticAssetSource::NativeGenericTactic(_) => {
            return Err(TacticBlueprintError::StaticExecution(format!(
                "option {:?} has no static execution capture",
                entry.option_id()
            )));
        }
    };
    execution
        .validate_against_tape(tape)
        .map_err(|error| TacticBlueprintError::StaticExecution(error.to_string()))?;
    Ok(execution)
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

fn validate_available_conditions<F>(
    node: &TacticBlueprintNode,
    condition_available: &mut F,
) -> Result<(), TacticBlueprintError>
where
    F: FnMut(&OptionCondition) -> bool,
{
    match node {
        TacticBlueprintNode::Invoke { .. } => Ok(()),
        TacticBlueprintNode::Sequence { steps } => {
            for child in steps {
                validate_available_conditions(child, condition_available)?;
            }
            Ok(())
        }
        TacticBlueprintNode::Layer { layers } => {
            for child in layers {
                validate_available_conditions(child, condition_available)?;
            }
            Ok(())
        }
        TacticBlueprintNode::Conditional {
            condition,
            when_true,
            when_false,
        } => {
            if !condition_available(condition) {
                return Err(TacticBlueprintError::UnavailableCondition);
            }
            validate_available_conditions(when_true, condition_available)?;
            validate_available_conditions(when_false, condition_available)
        }
        TacticBlueprintNode::Until {
            condition, body, ..
        } => {
            if !condition_available(condition) {
                return Err(TacticBlueprintError::UnavailableCondition);
            }
            validate_available_conditions(body, condition_available)
        }
        TacticBlueprintNode::Fallback { attempts } => {
            for child in attempts {
                validate_available_conditions(child, condition_available)?;
            }
            Ok(())
        }
    }
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

fn valid_portable_id_value(value: &str) -> bool {
    validate_portable_id(value).is_ok()
}

fn valid_node_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b'[' | b']')
        })
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
    UnavailableCondition,
    UnsupportedStaticNode(&'static str),
    StaticExecution(String),
    InvalidCompiled(String),
    TooManyApplicableChoices,
    DuplicateBlueprintId(String),
    DuplicateChoiceId,
    InvalidCompositeDuration,
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
            Self::UnavailableCondition => formatter.write_str(
                "tactic blueprint condition is unavailable in the current fact registry",
            ),
            Self::UnsupportedStaticNode(node_kind) => {
                write!(
                    formatter,
                    "tactic blueprint {node_kind} requires runtime condition evaluation"
                )
            }
            Self::StaticExecution(message) => {
                write!(
                    formatter,
                    "static tactic blueprint execution failed: {message}"
                )
            }
            Self::InvalidCompiled(message) => {
                write!(formatter, "compiled tactic blueprint is invalid: {message}")
            }
            Self::TooManyApplicableChoices => {
                formatter.write_str("applicable tactic choice set exceeds its finite bound")
            }
            Self::DuplicateBlueprintId(asset_id) => {
                write!(formatter, "duplicate tactic blueprint asset ID {asset_id}")
            }
            Self::DuplicateChoiceId => {
                formatter.write_str("applicable tactic choice IDs are not unique")
            }
            Self::InvalidCompositeDuration => {
                formatter.write_str("tactic blueprint duration is invalid or unbounded")
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
    use dusklight_control::controller_program::ControllerProgram;
    use dusklight_control::game_tactic::{GameTactic, GameTacticPlan};
    use dusklight_control::roll_option::{RollOptionPlan, RollSpacing};

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

    #[test]
    fn sequence_and_layer_compile_to_one_exact_ordered_tape() {
        let entry = |id, source| TacticCatalogEntry::new(id, source).unwrap();
        let catalog = TacticAssetCatalog::new(vec![
            entry(
                "buttons",
                TacticAssetSource::ReactiveController(
                    ControllerProgram::parse(
                        "duskcontrol 1\nframes 2\nbuttons from 0 for 2 B\n",
                    )
                    .unwrap(),
                ),
            ),
            entry(
                "move",
                TacticAssetSource::ReactiveController(
                    ControllerProgram::parse(
                        "duskcontrol 1\nframes 2\nbezier replace from 0 for 2 p0 0 80 p1 0 80 p2 0 80 p3 0 80\n",
                    )
                    .unwrap(),
                ),
            ),
            entry(
                "shield",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield {
                    frames: 1,
                })),
            ),
        ])
        .unwrap();
        let blueprint = TacticBlueprint::new(
            "static.sequence-layer",
            TacticBlueprintNode::Sequence {
                steps: vec![
                    invoke("shield"),
                    TacticBlueprintNode::Layer {
                        layers: vec![invoke("move"), invoke("buttons")],
                    },
                ],
            },
        )
        .unwrap();

        let compiled = blueprint.compile_static(&catalog).unwrap();
        assert_eq!(compiled.tape.frames.len(), 3);
        assert_eq!(compiled.tape.frames[0].pads[0].buttons, 0x0020);
        for frame in &compiled.tape.frames[1..] {
            assert_eq!(frame.pads[0].stick_y, 80);
            assert_eq!(frame.pads[0].buttons, 0x0200);
        }
        assert_eq!(compiled.segments.len(), 2);
        let CompiledStaticSegment::Invoke { execution, .. } = &compiled.segments[0] else {
            panic!("sequence prefix must remain an option execution")
        };
        execution.validate_against_tape(&compiled.tape).unwrap();
        assert_eq!(
            execution.realized_tape_range,
            TapeRange {
                start_frame: 0,
                end_frame_exclusive: 1,
            }
        );
        let CompiledStaticSegment::Layer {
            option_ids,
            tape_range,
            ..
        } = &compiled.segments[1]
        else {
            panic!("concurrent children must remain a layer segment")
        };
        assert_eq!(option_ids, &["move", "buttons"]);
        assert_eq!(
            *tape_range,
            TapeRange {
                start_frame: 1,
                end_frame_exclusive: 3,
            }
        );
        compiled.validate().unwrap();
    }

    #[test]
    fn layer_rejects_ambiguous_writers_and_non_controller_children() {
        let program = |magnitude| {
            ControllerProgram::parse(&format!(
                "duskcontrol 1\nframes 1\nturn replace from 0 for 1 direction right magnitude {magnitude}\n"
            ))
            .unwrap()
        };
        let catalog = TacticAssetCatalog::new(vec![
            TacticCatalogEntry::new("first", TacticAssetSource::ReactiveController(program(20)))
                .unwrap(),
            TacticCatalogEntry::new("second", TacticAssetSource::ReactiveController(program(70)))
                .unwrap(),
            TacticCatalogEntry::new(
                "shield",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield {
                    frames: 1,
                })),
            )
            .unwrap(),
        ])
        .unwrap();
        let layer = TacticBlueprintNode::Layer {
            layers: vec![invoke("first"), invoke("second")],
        };
        assert!(matches!(
            compile_controller_layer(&layer, &catalog),
            Err(TacticBlueprintError::StaticExecution(message))
                if message.contains("replace stick layers")
        ));

        let invalid = TacticBlueprintNode::Layer {
            layers: vec![invoke("first"), invoke("shield")],
        };
        assert!(compile_controller_layer(&invalid, &catalog).is_err());
    }

    #[test]
    fn sequence_does_not_insert_hidden_frames_to_rephase_a_roll() {
        let mut roll = RollOptionPlan::new(0, 100, 0);
        roll.spacing = RollSpacing {
            period_ticks: 2,
            phase_tick: 0,
        };
        let catalog = TacticAssetCatalog::new(vec![
            TacticCatalogEntry::new("roll", TacticAssetSource::Roll(roll)).unwrap(),
            TacticCatalogEntry::new(
                "shield",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield {
                    frames: 1,
                })),
            )
            .unwrap(),
        ])
        .unwrap();
        let blueprint = TacticBlueprint::new(
            "roll.phase",
            TacticBlueprintNode::Sequence {
                steps: vec![invoke("shield"), invoke("roll")],
            },
        )
        .unwrap();
        assert!(matches!(
            blueprint.compile_static(&catalog),
            Err(TacticBlueprintError::StaticExecution(_))
        ));
    }

    #[test]
    fn execution_validation_rejects_conditions_absent_from_the_fact_registry() {
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
            "fact.availability",
            TacticBlueprintNode::Conditional {
                condition: condition(),
                when_true: Box::new(invoke("wait")),
                when_false: Box::new(invoke("wait")),
            },
        )
        .unwrap();
        assert_eq!(
            blueprint
                .validate_for_execution(&catalog, |_| false)
                .unwrap_err(),
            TacticBlueprintError::UnavailableCondition
        );
        blueprint
            .validate_for_execution(&catalog, |available| available == &condition())
            .unwrap();
    }

    #[test]
    fn applicability_enumeration_is_finite_concrete_and_schema_bound() {
        let catalog = TacticAssetCatalog::new(vec![
            TacticCatalogEntry::new(
                "shield.short",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield {
                    frames: 1,
                })),
            )
            .unwrap(),
            TacticCatalogEntry::new(
                "shield.long",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield {
                    frames: 3,
                })),
            )
            .unwrap(),
            TacticCatalogEntry::new(
                "interact",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Interact {
                    press_frames: 1,
                    recovery_frames: 1,
                })),
            )
            .unwrap(),
        ])
        .unwrap();
        let blueprint = TacticBlueprint::new(
            "shield.then.interact",
            TacticBlueprintNode::Sequence {
                steps: vec![invoke("shield.short"), invoke("interact")],
            },
        )
        .unwrap();

        let choices =
            ApplicableTacticChoices::enumerate(&catalog, &[blueprint], |_| true, |_| Some(true))
                .unwrap();
        assert_eq!(
            choices
                .candidates
                .iter()
                .map(|choice| choice.choice_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "blueprint/shield.then.interact",
                "interact",
                "shield.long",
                "shield.short",
            ]
        );
        assert_eq!(choices.applicable_mask, vec![true; 4]);
        assert_ne!(choices.choice_schema_sha256, Digest::ZERO);
        assert_ne!(choices.applicability_sha256, Digest::ZERO);
        assert_eq!(
            choices.candidates[0].duration,
            TacticDurationBounds {
                minimum_ticks: 2,
                maximum_ticks: 3,
            }
        );
        let short = choices
            .candidates
            .iter()
            .find(|choice| choice.choice_id == "shield.short")
            .unwrap();
        let long = choices
            .candidates
            .iter()
            .find(|choice| choice.choice_id == "shield.long")
            .unwrap();
        assert_ne!(short.descriptor.parameters, long.descriptor.parameters);
    }

    #[test]
    fn applicability_excludes_blueprints_with_unavailable_inputs() {
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
        let conditional = TacticBlueprint::new(
            "conditional",
            TacticBlueprintNode::Conditional {
                condition: condition(),
                when_true: Box::new(invoke("wait")),
                when_false: Box::new(invoke("wait")),
            },
        )
        .unwrap();

        let choices =
            ApplicableTacticChoices::enumerate(&catalog, &[conditional], |_| true, |_| None)
                .unwrap();
        assert_eq!(choices.candidates.len(), 2);
        assert_eq!(
            choices
                .candidates
                .iter()
                .map(|choice| choice.choice_id.as_str())
                .collect::<Vec<_>>(),
            vec!["blueprint/conditional", "wait"]
        );
        assert_eq!(choices.applicable_mask, vec![false, true]);
    }

    #[test]
    fn composite_applicability_uses_only_the_current_start_path() {
        let catalog = TacticAssetCatalog::new(vec![
            TacticCatalogEntry::new(
                "available",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield {
                    frames: 1,
                })),
            )
            .unwrap(),
            TacticCatalogEntry::new(
                "blocked",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Interact {
                    press_frames: 1,
                    recovery_frames: 1,
                })),
            )
            .unwrap(),
        ])
        .unwrap();
        let sequence = TacticBlueprint::new(
            "sequence",
            TacticBlueprintNode::Sequence {
                steps: vec![invoke("available"), invoke("blocked")],
            },
        )
        .unwrap();
        let conditional = TacticBlueprint::new(
            "conditional",
            TacticBlueprintNode::Conditional {
                condition: condition(),
                when_true: Box::new(invoke("available")),
                when_false: Box::new(invoke("blocked")),
            },
        )
        .unwrap();
        let until = TacticBlueprint::new(
            "until",
            TacticBlueprintNode::Until {
                condition: condition(),
                max_iterations: 2,
                body: Box::new(invoke("available")),
            },
        )
        .unwrap();

        let choices = ApplicableTacticChoices::enumerate(
            &catalog,
            &[sequence, conditional, until],
            |description| description.option.option_id == "available",
            |_| Some(true),
        )
        .unwrap();
        assert_eq!(
            choices
                .candidates
                .iter()
                .map(|choice| choice.choice_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "available",
                "blocked",
                "blueprint/conditional",
                "blueprint/sequence",
                "blueprint/until",
            ]
        );
        assert_eq!(
            choices.applicable_mask,
            vec![true, false, true, true, false]
        );
    }

    #[test]
    fn applicability_can_report_an_empty_bounded_choice_set() {
        let catalog = TacticAssetCatalog::new(vec![
            TacticCatalogEntry::new(
                "blocked",
                TacticAssetSource::GameTactic(GameTacticPlan::new(GameTactic::Shield {
                    frames: 1,
                })),
            )
            .unwrap(),
        ])
        .unwrap();

        let choices =
            ApplicableTacticChoices::enumerate(&catalog, &[], |_| false, |_| None).unwrap();
        assert_eq!(choices.candidates.len(), 1);
        assert_eq!(choices.applicable_mask, vec![false]);
        assert_ne!(choices.choice_schema_sha256, Digest::ZERO);
        assert_ne!(choices.applicability_sha256, Digest::ZERO);
    }
}
