//! Deterministic random and categorical-CEM proposal engines over raw-PAD residuals.

use crate::residual_action::{
    AUTHENTIC_BUTTON_MASK, AnalogChannel, AnalogResidual, ButtonResidual, ButtonResidualMode,
    CompiledResidualCandidate, ResidualActionError, ResidualCandidate, ResidualWindow,
    TemporalBasis, compile_residual_candidate,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::{InputTape, PORT_COUNT};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const RESIDUAL_SEARCH_SPACE_SCHEMA_V1: &str = "dusklight-residual-search-space/v1";
pub const RESIDUAL_RANDOM_SNAPSHOT_SCHEMA_V1: &str = "dusklight-residual-random-snapshot/v1";
pub const RESIDUAL_CEM_SNAPSHOT_SCHEMA_V1: &str = "dusklight-residual-cem-snapshot/v1";
const CEM_MASS: u32 = 1_000_000;
const BASIS_COUNT: usize = 8;
const MAX_CANDIDATE_SLOTS: u8 = 16;
const MAX_FRAME_DOMAIN: u64 = 65_536;
const MAX_CATEGORY_VALUES: usize = 256;
const MAX_PROPOSALS_PER_BATCH: usize = 16_384;
const MAX_ATTEMPTS_PER_PROPOSAL: usize = 256;

/// One finite, authenticated parameterization shared by random search and CEM.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualSearchSpace {
    pub schema: String,
    pub start_frame: u64,
    pub end_frame_exclusive: u64,
    pub candidate_slots: u8,
    pub ports: Vec<u8>,
    pub analog_channels: Vec<AnalogChannel>,
    pub analog_delta_values: Vec<i16>,
    pub button_masks: Vec<u16>,
    pub duration_values: Vec<u16>,
}

impl ResidualSearchSpace {
    pub fn validate(&self) -> Result<(), ResidualOptimizerError> {
        let width = self.end_frame_exclusive.checked_sub(self.start_frame);
        if self.schema != RESIDUAL_SEARCH_SPACE_SCHEMA_V1
            || width.is_none_or(|width| !(32..=MAX_FRAME_DOMAIN).contains(&width))
            || self.candidate_slots == 0
            || self.candidate_slots > MAX_CANDIDATE_SLOTS
            || self.ports.is_empty()
            || self.ports.len() > PORT_COUNT
            || !strictly_increasing(&self.ports)
            || self
                .ports
                .iter()
                .any(|port| usize::from(*port) >= PORT_COUNT)
            || self.analog_channels.is_empty()
            || self.analog_channels.len() > 4
            || !strictly_increasing(&self.analog_channels)
            || self.analog_delta_values.is_empty()
            || self.analog_delta_values.len() > MAX_CATEGORY_VALUES
            || !strictly_increasing(&self.analog_delta_values)
            || self
                .analog_delta_values
                .iter()
                .any(|delta| *delta == 0 || !(-255..=255).contains(delta))
            || self.button_masks.is_empty()
            || self.button_masks.len() > MAX_CATEGORY_VALUES
            || !strictly_increasing(&self.button_masks)
            || self
                .button_masks
                .iter()
                .any(|buttons| *buttons == 0 || buttons & !AUTHENTIC_BUTTON_MASK != 0)
            || self.duration_values.is_empty()
            || self.duration_values.len() > MAX_CATEGORY_VALUES
            || !strictly_increasing(&self.duration_values)
            || self
                .duration_values
                .iter()
                .any(|duration| *duration == 0 || u64::from(*duration) > width.unwrap_or_default())
            || !self.duration_values.iter().any(|duration| *duration >= 2)
        {
            return Err(optimizer_error(
                "residual search space must be canonical, bounded, and cover every temporal basis",
            ));
        }
        Ok(())
    }

    pub fn validate_parent(&self, parent: &InputTape) -> Result<(), ResidualOptimizerError> {
        self.validate()?;
        if self.end_frame_exclusive > parent.frames.len() as u64 {
            return Err(optimizer_error(
                "residual search space exceeds the incumbent tape",
            ));
        }
        Ok(())
    }

    pub fn sha256(&self) -> Result<Digest, ResidualOptimizerError> {
        self.validate()?;
        canonical_digest(b"dusklight.residual-search-space/v1\0", self)
    }

    fn frame_domain(&self) -> usize {
        (self.end_frame_exclusive - self.start_frame) as usize
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidualGeneKind {
    Analog,
    Button,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidualGeneButtonMode {
    Press,
    Release,
}

/// Indices into a [`ResidualSearchSpace`]. Irrelevant fields remain explicit so
/// random search and CEM sample one identical rectangular finite surface.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualGene {
    pub enabled: bool,
    pub kind: ResidualGeneKind,
    pub port_index: u16,
    pub channel_index: u16,
    pub basis_index: u8,
    pub start_index: u32,
    pub duration_index: u16,
    pub delta_indices: [u16; 4],
    pub button_index: u16,
    pub button_mode: ResidualGeneButtonMode,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualGenome {
    pub genes: Vec<ResidualGene>,
}

impl ResidualGenome {
    pub fn validate(&self, space: &ResidualSearchSpace) -> Result<(), ResidualOptimizerError> {
        space.validate()?;
        if self.genes.len() != usize::from(space.candidate_slots)
            || self.genes.iter().any(|gene| {
                usize::from(gene.port_index) >= space.ports.len()
                    || usize::from(gene.channel_index) >= space.analog_channels.len()
                    || usize::from(gene.basis_index) >= BASIS_COUNT
                    || gene.start_index as usize >= space.frame_domain()
                    || usize::from(gene.duration_index) >= space.duration_values.len()
                    || gene
                        .delta_indices
                        .iter()
                        .any(|index| usize::from(*index) >= space.analog_delta_values.len())
                    || usize::from(gene.button_index) >= space.button_masks.len()
            })
        {
            return Err(optimizer_error(
                "residual genome width or category index is invalid",
            ));
        }
        Ok(())
    }

    pub fn candidate(
        &self,
        parent_tape_bytes: &[u8],
        space: &ResidualSearchSpace,
    ) -> Result<ResidualCandidate, ResidualOptimizerError> {
        self.validate(space)?;
        let mut analog = Vec::new();
        let mut buttons = Vec::new();
        for gene in self.genes.iter().filter(|gene| gene.enabled) {
            match gene.kind {
                ResidualGeneKind::Analog => analog.push(render_analog(gene, space)),
                ResidualGeneKind::Button => buttons.push(render_button(gene, space)),
            }
        }
        analog.sort();
        analog.dedup();
        buttons.sort();
        buttons.dedup();
        ResidualCandidate::seal(parent_tape_bytes, analog, buttons).map_err(Into::into)
    }
}

#[derive(Clone, Debug)]
pub struct ResidualProposal {
    pub generation: u32,
    pub sample_index: u32,
    pub genome: ResidualGenome,
    pub candidate: ResidualCandidate,
    pub compiled: CompiledResidualCandidate,
}

#[derive(Clone, Debug)]
pub struct ResidualProposalBatch {
    pub proposals: Vec<ResidualProposal>,
    pub rejected_invalid: u64,
    pub rejected_duplicate_tape: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualRandomSnapshot {
    pub schema: String,
    pub content_sha256: Digest,
    pub search_space_sha256: Digest,
    pub parent_tape_sha256: Digest,
    pub rng_state: u64,
    pub produced_candidates: u64,
    pub attempted_genomes: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rejected_invalid_genomes: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rejected_duplicate_tapes: u64,
    pub seen_tape_sha256: Vec<Digest>,
}

#[derive(Clone, Debug)]
pub struct ResidualRandomSampler {
    space: ResidualSearchSpace,
    parent_tape_sha256: Digest,
    rng: DeterministicRng,
    produced_candidates: u64,
    attempted_genomes: u64,
    rejected_invalid_genomes: u64,
    rejected_duplicate_tapes: u64,
    seen_tape_sha256: BTreeSet<Digest>,
}

impl ResidualRandomSampler {
    pub fn new(
        space: ResidualSearchSpace,
        parent_tape_bytes: &[u8],
        seed: u64,
    ) -> Result<Self, ResidualOptimizerError> {
        space.validate()?;
        Ok(Self {
            space,
            parent_tape_sha256: sha256_bytes(parent_tape_bytes),
            rng: DeterministicRng::new(seed ^ 0x7261_6e64_6f6d_5f72),
            produced_candidates: 0,
            attempted_genomes: 0,
            rejected_invalid_genomes: 0,
            rejected_duplicate_tapes: 0,
            seen_tape_sha256: BTreeSet::new(),
        })
    }

    pub fn restore(
        space: ResidualSearchSpace,
        parent_tape_bytes: &[u8],
        snapshot: ResidualRandomSnapshot,
    ) -> Result<Self, ResidualOptimizerError> {
        space.validate()?;
        if snapshot.schema != RESIDUAL_RANDOM_SNAPSHOT_SCHEMA_V1
            || snapshot.content_sha256 == Digest::ZERO
            || snapshot.content_sha256 != snapshot.compute_identity()?
            || snapshot.search_space_sha256 != space.sha256()?
            || snapshot.parent_tape_sha256 != sha256_bytes(parent_tape_bytes)
            || snapshot
                .seen_tape_sha256
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || snapshot.produced_candidates != snapshot.seen_tape_sha256.len() as u64
            || snapshot.attempted_genomes < snapshot.produced_candidates
            || snapshot
                .rejected_invalid_genomes
                .checked_add(snapshot.rejected_duplicate_tapes)
                .is_none_or(|classified| {
                    classified > snapshot.attempted_genomes - snapshot.produced_candidates
                })
        {
            return Err(optimizer_error("random residual snapshot is invalid"));
        }
        Ok(Self {
            space,
            parent_tape_sha256: snapshot.parent_tape_sha256,
            rng: DeterministicRng {
                state: snapshot.rng_state,
            },
            produced_candidates: snapshot.produced_candidates,
            attempted_genomes: snapshot.attempted_genomes,
            rejected_invalid_genomes: snapshot.rejected_invalid_genomes,
            rejected_duplicate_tapes: snapshot.rejected_duplicate_tapes,
            seen_tape_sha256: snapshot.seen_tape_sha256.into_iter().collect(),
        })
    }

    pub fn sample(
        &mut self,
        parent: &InputTape,
        parent_tape_bytes: &[u8],
        count: usize,
    ) -> Result<ResidualProposalBatch, ResidualOptimizerError> {
        self.space.validate_parent(parent)?;
        self.validate_parent_bytes(parent_tape_bytes)?;
        validate_batch_count(count)?;
        let generation = u32::try_from(self.produced_candidates)
            .map_err(|_| optimizer_error("random proposal index overflowed"))?;
        let batch = sample_unique(
            &self.space,
            &mut self.rng,
            parent,
            parent_tape_bytes,
            count,
            generation,
            &mut self.attempted_genomes,
            &mut self.produced_candidates,
            &mut self.seen_tape_sha256,
            None,
        )?;
        self.rejected_invalid_genomes = self
            .rejected_invalid_genomes
            .checked_add(batch.rejected_invalid)
            .ok_or_else(|| optimizer_error("random invalid-rejection count overflowed"))?;
        self.rejected_duplicate_tapes = self
            .rejected_duplicate_tapes
            .checked_add(batch.rejected_duplicate_tape)
            .ok_or_else(|| optimizer_error("random duplicate-rejection count overflowed"))?;
        Ok(batch)
    }

    pub fn snapshot(&self) -> Result<ResidualRandomSnapshot, ResidualOptimizerError> {
        let mut snapshot = ResidualRandomSnapshot {
            schema: RESIDUAL_RANDOM_SNAPSHOT_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            search_space_sha256: self.space.sha256()?,
            parent_tape_sha256: self.parent_tape_sha256,
            rng_state: self.rng.state,
            produced_candidates: self.produced_candidates,
            attempted_genomes: self.attempted_genomes,
            rejected_invalid_genomes: self.rejected_invalid_genomes,
            rejected_duplicate_tapes: self.rejected_duplicate_tapes,
            seen_tape_sha256: self.seen_tape_sha256.iter().copied().collect(),
        };
        snapshot.content_sha256 = snapshot.compute_identity()?;
        Ok(snapshot)
    }

    fn validate_parent_bytes(
        &self,
        parent_tape_bytes: &[u8],
    ) -> Result<(), ResidualOptimizerError> {
        if sha256_bytes(parent_tape_bytes) != self.parent_tape_sha256 {
            return Err(optimizer_error(
                "random residual sampler is detached from its parent tape",
            ));
        }
        Ok(())
    }
}

impl ResidualRandomSnapshot {
    fn compute_identity(&self) -> Result<Digest, ResidualOptimizerError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.residual-random-snapshot/v1\0", &canonical)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualCemConfig {
    pub population: usize,
    pub elites: usize,
    pub smoothing_millionths: u32,
    pub seed: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CategoricalDistribution {
    pub weights_millionths: Vec<u32>,
}

impl CategoricalDistribution {
    fn uniform(categories: usize) -> Self {
        let base = CEM_MASS / categories as u32;
        let remainder = CEM_MASS % categories as u32;
        Self {
            weights_millionths: (0..categories)
                .map(|index| base + u32::from(index < remainder as usize))
                .collect(),
        }
    }

    fn validate(&self, categories: usize) -> bool {
        self.weights_millionths.len() == categories
            && self.weights_millionths.iter().all(|weight| *weight > 0)
            && self
                .weights_millionths
                .iter()
                .map(|weight| u64::from(*weight))
                .sum::<u64>()
                == u64::from(CEM_MASS)
    }

    fn sample(&self, rng: &mut DeterministicRng) -> usize {
        let draw = rng.bounded(u64::from(CEM_MASS)) as u32;
        let mut cumulative = 0_u32;
        for (index, weight) in self.weights_millionths.iter().enumerate() {
            cumulative += *weight;
            if draw < cumulative {
                return index;
            }
        }
        self.weights_millionths.len() - 1
    }

    fn update(&mut self, observed: &[usize], smoothing: u32) {
        let categories = self.weights_millionths.len();
        let mut counts = vec![0_u32; categories];
        for index in observed {
            counts[*index] += 1;
        }
        let target = probability_from_counts(&counts, observed.len() as u32);
        let keep = CEM_MASS - smoothing;
        let raw = self
            .weights_millionths
            .iter()
            .zip(target)
            .map(|(old, target)| {
                (u64::from(*old) * u64::from(keep)
                    + u64::from(target) * u64::from(smoothing)
                    + u64::from(CEM_MASS / 2))
                    / u64::from(CEM_MASS)
            })
            .collect::<Vec<_>>();
        self.weights_millionths = normalize_probability(raw);
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualGeneDistribution {
    pub enabled: CategoricalDistribution,
    pub kind: CategoricalDistribution,
    pub port: CategoricalDistribution,
    pub channel: CategoricalDistribution,
    pub basis: CategoricalDistribution,
    pub start: CategoricalDistribution,
    pub duration: CategoricalDistribution,
    pub deltas: [CategoricalDistribution; 4],
    pub button: CategoricalDistribution,
    pub button_mode: CategoricalDistribution,
}

impl ResidualGeneDistribution {
    fn uniform(space: &ResidualSearchSpace) -> Self {
        Self {
            enabled: CategoricalDistribution::uniform(2),
            kind: CategoricalDistribution::uniform(2),
            port: CategoricalDistribution::uniform(space.ports.len()),
            channel: CategoricalDistribution::uniform(space.analog_channels.len()),
            basis: CategoricalDistribution::uniform(BASIS_COUNT),
            start: CategoricalDistribution::uniform(space.frame_domain()),
            duration: CategoricalDistribution::uniform(space.duration_values.len()),
            deltas: std::array::from_fn(|_| {
                CategoricalDistribution::uniform(space.analog_delta_values.len())
            }),
            button: CategoricalDistribution::uniform(space.button_masks.len()),
            button_mode: CategoricalDistribution::uniform(2),
        }
    }

    fn validate(&self, space: &ResidualSearchSpace) -> bool {
        self.enabled.validate(2)
            && self.kind.validate(2)
            && self.port.validate(space.ports.len())
            && self.channel.validate(space.analog_channels.len())
            && self.basis.validate(BASIS_COUNT)
            && self.start.validate(space.frame_domain())
            && self.duration.validate(space.duration_values.len())
            && self
                .deltas
                .iter()
                .all(|distribution| distribution.validate(space.analog_delta_values.len()))
            && self.button.validate(space.button_masks.len())
            && self.button_mode.validate(2)
    }

    fn sample(&self, rng: &mut DeterministicRng) -> ResidualGene {
        ResidualGene {
            enabled: self.enabled.sample(rng) == 1,
            kind: if self.kind.sample(rng) == 0 {
                ResidualGeneKind::Analog
            } else {
                ResidualGeneKind::Button
            },
            port_index: self.port.sample(rng) as u16,
            channel_index: self.channel.sample(rng) as u16,
            basis_index: self.basis.sample(rng) as u8,
            start_index: self.start.sample(rng) as u32,
            duration_index: self.duration.sample(rng) as u16,
            delta_indices: std::array::from_fn(|index| self.deltas[index].sample(rng) as u16),
            button_index: self.button.sample(rng) as u16,
            button_mode: if self.button_mode.sample(rng) == 0 {
                ResidualGeneButtonMode::Press
            } else {
                ResidualGeneButtonMode::Release
            },
        }
    }

    fn update(&mut self, genes: &[&ResidualGene], smoothing: u32) {
        self.enabled.update(
            &genes
                .iter()
                .map(|gene| usize::from(gene.enabled))
                .collect::<Vec<_>>(),
            smoothing,
        );
        self.kind.update(
            &genes
                .iter()
                .map(|gene| usize::from(gene.kind == ResidualGeneKind::Button))
                .collect::<Vec<_>>(),
            smoothing,
        );
        self.port.update(
            &genes
                .iter()
                .map(|gene| usize::from(gene.port_index))
                .collect::<Vec<_>>(),
            smoothing,
        );
        self.channel.update(
            &genes
                .iter()
                .map(|gene| usize::from(gene.channel_index))
                .collect::<Vec<_>>(),
            smoothing,
        );
        self.basis.update(
            &genes
                .iter()
                .map(|gene| usize::from(gene.basis_index))
                .collect::<Vec<_>>(),
            smoothing,
        );
        self.start.update(
            &genes
                .iter()
                .map(|gene| gene.start_index as usize)
                .collect::<Vec<_>>(),
            smoothing,
        );
        self.duration.update(
            &genes
                .iter()
                .map(|gene| usize::from(gene.duration_index))
                .collect::<Vec<_>>(),
            smoothing,
        );
        for index in 0..4 {
            self.deltas[index].update(
                &genes
                    .iter()
                    .map(|gene| usize::from(gene.delta_indices[index]))
                    .collect::<Vec<_>>(),
                smoothing,
            );
        }
        self.button.update(
            &genes
                .iter()
                .map(|gene| usize::from(gene.button_index))
                .collect::<Vec<_>>(),
            smoothing,
        );
        self.button_mode.update(
            &genes
                .iter()
                .map(|gene| usize::from(gene.button_mode == ResidualGeneButtonMode::Release))
                .collect::<Vec<_>>(),
            smoothing,
        );
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PendingResidualCemSample {
    pub sample_index: u32,
    pub candidate_sha256: Digest,
    pub genome: ResidualGenome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualCemSnapshot {
    pub schema: String,
    pub content_sha256: Digest,
    pub search_space_sha256: Digest,
    pub parent_tape_sha256: Digest,
    pub config: ResidualCemConfig,
    pub generation: u32,
    pub rng_state: u64,
    pub attempted_genomes: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rejected_invalid_genomes: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rejected_duplicate_tapes: u64,
    pub distributions: Vec<ResidualGeneDistribution>,
    pub seen_tape_sha256: Vec<Digest>,
    pub pending: Vec<PendingResidualCemSample>,
}

#[derive(Clone, Debug)]
pub struct ResidualCemOptimizer {
    space: ResidualSearchSpace,
    parent_tape_sha256: Digest,
    config: ResidualCemConfig,
    generation: u32,
    rng: DeterministicRng,
    attempted_genomes: u64,
    rejected_invalid_genomes: u64,
    rejected_duplicate_tapes: u64,
    distributions: Vec<ResidualGeneDistribution>,
    seen_tape_sha256: BTreeSet<Digest>,
    pending: BTreeMap<Digest, PendingResidualCemSample>,
}

impl ResidualCemOptimizer {
    pub fn new(
        space: ResidualSearchSpace,
        parent_tape_bytes: &[u8],
        config: ResidualCemConfig,
    ) -> Result<Self, ResidualOptimizerError> {
        space.validate()?;
        validate_cem_config(config)?;
        Ok(Self {
            distributions: (0..space.candidate_slots)
                .map(|_| ResidualGeneDistribution::uniform(&space))
                .collect(),
            space,
            parent_tape_sha256: sha256_bytes(parent_tape_bytes),
            config,
            generation: 0,
            rng: DeterministicRng::new(config.seed ^ 0x6365_6d5f_7265_7369),
            attempted_genomes: 0,
            rejected_invalid_genomes: 0,
            rejected_duplicate_tapes: 0,
            seen_tape_sha256: BTreeSet::new(),
            pending: BTreeMap::new(),
        })
    }

    pub fn restore(
        space: ResidualSearchSpace,
        config: ResidualCemConfig,
        parent_tape_bytes: &[u8],
        snapshot: ResidualCemSnapshot,
    ) -> Result<Self, ResidualOptimizerError> {
        space.validate()?;
        validate_cem_config(config)?;
        let pending = snapshot
            .pending
            .iter()
            .map(|sample| (sample.candidate_sha256, sample.clone()))
            .collect::<BTreeMap<_, _>>();
        if snapshot.schema != RESIDUAL_CEM_SNAPSHOT_SCHEMA_V1
            || snapshot.content_sha256 == Digest::ZERO
            || snapshot.content_sha256 != snapshot.compute_identity()?
            || snapshot.search_space_sha256 != space.sha256()?
            || snapshot.parent_tape_sha256 != sha256_bytes(parent_tape_bytes)
            || snapshot.config != config
            || snapshot.distributions.len() != usize::from(space.candidate_slots)
            || snapshot
                .distributions
                .iter()
                .any(|distribution| !distribution.validate(&space))
            || snapshot
                .seen_tape_sha256
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || snapshot
                .pending
                .windows(2)
                .any(|pair| pair[0].candidate_sha256 >= pair[1].candidate_sha256)
            || pending.len() != snapshot.pending.len()
            || pending.len() > config.population
            || snapshot.attempted_genomes < snapshot.seen_tape_sha256.len() as u64
            || snapshot
                .rejected_invalid_genomes
                .checked_add(snapshot.rejected_duplicate_tapes)
                .is_none_or(|classified| {
                    classified > snapshot.attempted_genomes - snapshot.seen_tape_sha256.len() as u64
                })
            || snapshot
                .pending
                .iter()
                .any(|sample| !pending_sample_matches(sample, &space, parent_tape_bytes))
        {
            return Err(optimizer_error("residual CEM snapshot is invalid"));
        }
        Ok(Self {
            space,
            parent_tape_sha256: snapshot.parent_tape_sha256,
            config,
            generation: snapshot.generation,
            rng: DeterministicRng {
                state: snapshot.rng_state,
            },
            attempted_genomes: snapshot.attempted_genomes,
            rejected_invalid_genomes: snapshot.rejected_invalid_genomes,
            rejected_duplicate_tapes: snapshot.rejected_duplicate_tapes,
            distributions: snapshot.distributions,
            seen_tape_sha256: snapshot.seen_tape_sha256.into_iter().collect(),
            pending,
        })
    }

    pub fn ask(
        &mut self,
        parent: &InputTape,
        parent_tape_bytes: &[u8],
    ) -> Result<ResidualProposalBatch, ResidualOptimizerError> {
        if !self.pending.is_empty() {
            return Err(optimizer_error(
                "residual CEM must rank the pending generation before asking again",
            ));
        }
        self.space.validate_parent(parent)?;
        self.validate_parent_bytes(parent_tape_bytes)?;
        let mut batch = ResidualProposalBatch {
            proposals: Vec::with_capacity(self.config.population),
            rejected_invalid: 0,
            rejected_duplicate_tape: 0,
        };
        if self.generation == 0 {
            for genome in inferred_button_cadence_repairs(parent, &self.space)
                .into_iter()
                .take(self.config.population)
            {
                self.attempted_genomes = self
                    .attempted_genomes
                    .checked_add(1)
                    .ok_or_else(|| optimizer_error("residual attempt counter overflowed"))?;
                let compiled = match compile_genome(parent, parent_tape_bytes, &self.space, &genome)
                {
                    Ok(compiled) => compiled,
                    Err(_) => {
                        batch.rejected_invalid += 1;
                        continue;
                    }
                };
                if !self
                    .seen_tape_sha256
                    .insert(compiled.report.realized_tape_sha256)
                {
                    batch.rejected_duplicate_tape += 1;
                    continue;
                }
                let candidate = genome.candidate(parent_tape_bytes, &self.space)?;
                batch.proposals.push(ResidualProposal {
                    generation: self.generation,
                    sample_index: 0,
                    genome,
                    candidate,
                    compiled,
                });
            }
        }
        let mut produced = batch.proposals.len() as u64;
        let remaining = self.config.population - batch.proposals.len();
        if remaining > 0 {
            let sampled = sample_unique(
                &self.space,
                &mut self.rng,
                parent,
                parent_tape_bytes,
                remaining,
                self.generation,
                &mut self.attempted_genomes,
                &mut produced,
                &mut self.seen_tape_sha256,
                Some(&self.distributions),
            )?;
            batch.rejected_invalid += sampled.rejected_invalid;
            batch.rejected_duplicate_tape += sampled.rejected_duplicate_tape;
            batch.proposals.extend(sampled.proposals);
        }
        for (sample_index, proposal) in batch.proposals.iter_mut().enumerate() {
            proposal.sample_index = u32::try_from(sample_index)
                .map_err(|_| optimizer_error("residual sample index overflowed"))?;
        }
        self.rejected_invalid_genomes = self
            .rejected_invalid_genomes
            .checked_add(batch.rejected_invalid)
            .ok_or_else(|| optimizer_error("CEM invalid-rejection count overflowed"))?;
        self.rejected_duplicate_tapes = self
            .rejected_duplicate_tapes
            .checked_add(batch.rejected_duplicate_tape)
            .ok_or_else(|| optimizer_error("CEM duplicate-rejection count overflowed"))?;
        for proposal in &batch.proposals {
            self.pending.insert(
                proposal.candidate.content_sha256,
                PendingResidualCemSample {
                    sample_index: proposal.sample_index,
                    candidate_sha256: proposal.candidate.content_sha256,
                    genome: proposal.genome.clone(),
                },
            );
        }
        batch
            .proposals
            .sort_by_key(|proposal| proposal.sample_index);
        Ok(batch)
    }

    /// Updates from candidate identities ordered best-to-worst by native
    /// evidence. Fitness magnitudes and shaped rewards are deliberately absent.
    pub fn tell(
        &mut self,
        ranked_candidate_sha256: &[Digest],
    ) -> Result<(), ResidualOptimizerError> {
        if ranked_candidate_sha256.len() != self.pending.len()
            || ranked_candidate_sha256.len() < self.config.elites
            || ranked_candidate_sha256
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != ranked_candidate_sha256.len()
            || ranked_candidate_sha256
                .iter()
                .any(|candidate| !self.pending.contains_key(candidate))
        {
            return Err(optimizer_error(
                "residual CEM requires one exact native-evidence rank for every pending candidate",
            ));
        }
        let elites = ranked_candidate_sha256
            .iter()
            .take(self.config.elites)
            .map(|digest| &self.pending[digest].genome)
            .collect::<Vec<_>>();
        for slot in 0..self.distributions.len() {
            let genes = elites
                .iter()
                .map(|genome| &genome.genes[slot])
                .collect::<Vec<_>>();
            self.distributions[slot].update(&genes, self.config.smoothing_millionths);
        }
        self.pending.clear();
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| optimizer_error("residual CEM generation overflowed"))?;
        Ok(())
    }

    pub fn snapshot(&self) -> Result<ResidualCemSnapshot, ResidualOptimizerError> {
        let mut snapshot = ResidualCemSnapshot {
            schema: RESIDUAL_CEM_SNAPSHOT_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            search_space_sha256: self.space.sha256()?,
            parent_tape_sha256: self.parent_tape_sha256,
            config: self.config,
            generation: self.generation,
            rng_state: self.rng.state,
            attempted_genomes: self.attempted_genomes,
            rejected_invalid_genomes: self.rejected_invalid_genomes,
            rejected_duplicate_tapes: self.rejected_duplicate_tapes,
            distributions: self.distributions.clone(),
            seen_tape_sha256: self.seen_tape_sha256.iter().copied().collect(),
            pending: self.pending.values().cloned().collect(),
        };
        snapshot.content_sha256 = snapshot.compute_identity()?;
        Ok(snapshot)
    }

    fn validate_parent_bytes(
        &self,
        parent_tape_bytes: &[u8],
    ) -> Result<(), ResidualOptimizerError> {
        if sha256_bytes(parent_tape_bytes) != self.parent_tape_sha256 {
            return Err(optimizer_error(
                "residual CEM is detached from its parent tape",
            ));
        }
        Ok(())
    }
}

impl ResidualCemSnapshot {
    fn compute_identity(&self) -> Result<Digest, ResidualOptimizerError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.residual-cem-snapshot/v1\0", &canonical)
    }
}

fn pending_sample_matches(
    sample: &PendingResidualCemSample,
    space: &ResidualSearchSpace,
    parent_tape_bytes: &[u8],
) -> bool {
    sample.genome.validate(space).is_ok()
        && sample
            .genome
            .candidate(parent_tape_bytes, space)
            .is_ok_and(|candidate| candidate.content_sha256 == sample.candidate_sha256)
}

/// Derive ordinary generation-zero candidates from doubled gaps in repeated
/// incumbent button edges. This names no game action, route, frame, button, or
/// terminal; native predicate evidence still provides the only ranking signal.
fn inferred_button_cadence_repairs(
    parent: &InputTape,
    space: &ResidualSearchSpace,
) -> Vec<ResidualGenome> {
    let mut repairs = Vec::new();
    for (port_index, &port) in space.ports.iter().enumerate() {
        for (button_index, &mask) in space.button_masks.iter().enumerate() {
            let edges = (space.start_frame..space.end_frame_exclusive)
                .filter(|&frame| {
                    let frame = frame as usize;
                    parent.frames[frame].pads[usize::from(port)].buttons & mask == mask
                        && (frame == 0
                            || parent.frames[frame - 1].pads[usize::from(port)].buttons & mask
                                != mask)
                })
                .collect::<Vec<_>>();
            let intervals = edges
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .collect::<Vec<_>>();
            for (gap_index, &gap) in intervals.iter().enumerate() {
                if gap < 2 || gap % 2 != 0 {
                    continue;
                }
                let cadence = gap / 2;
                if !intervals
                    .iter()
                    .enumerate()
                    .any(|(index, interval)| index != gap_index && *interval == cadence)
                {
                    continue;
                }
                let frame = edges[gap_index] + cadence;
                if frame >= space.end_frame_exclusive
                    || parent.frames[frame as usize].pads[usize::from(port)].buttons & mask != 0
                {
                    continue;
                }
                let Some(duration_index) =
                    space.duration_values.iter().position(|value| *value == 1)
                else {
                    continue;
                };
                let mut genes = vec![disabled_gene(); usize::from(space.candidate_slots)];
                genes[0] = ResidualGene {
                    enabled: true,
                    kind: ResidualGeneKind::Button,
                    port_index: port_index as u16,
                    channel_index: 0,
                    basis_index: 0,
                    start_index: (frame - space.start_frame) as u32,
                    duration_index: duration_index as u16,
                    delta_indices: [0; 4],
                    button_index: button_index as u16,
                    button_mode: ResidualGeneButtonMode::Press,
                };
                repairs.push(ResidualGenome { genes });
            }
        }
    }
    repairs
}

fn disabled_gene() -> ResidualGene {
    ResidualGene {
        enabled: false,
        kind: ResidualGeneKind::Analog,
        port_index: 0,
        channel_index: 0,
        basis_index: 0,
        start_index: 0,
        duration_index: 0,
        delta_indices: [0; 4],
        button_index: 0,
        button_mode: ResidualGeneButtonMode::Release,
    }
}

#[allow(clippy::too_many_arguments)]
fn sample_unique(
    space: &ResidualSearchSpace,
    rng: &mut DeterministicRng,
    parent: &InputTape,
    parent_tape_bytes: &[u8],
    count: usize,
    generation: u32,
    attempted_genomes: &mut u64,
    produced_candidates: &mut u64,
    seen_tape_sha256: &mut BTreeSet<Digest>,
    distributions: Option<&[ResidualGeneDistribution]>,
) -> Result<ResidualProposalBatch, ResidualOptimizerError> {
    validate_batch_count(count)?;
    let maximum_attempts = count
        .checked_mul(MAX_ATTEMPTS_PER_PROPOSAL)
        .ok_or_else(|| optimizer_error("residual proposal attempt budget overflowed"))?;
    let mut proposals = Vec::with_capacity(count);
    let mut rejected_invalid = 0_u64;
    let mut rejected_duplicate_tape = 0_u64;
    for _ in 0..maximum_attempts {
        if proposals.len() == count {
            break;
        }
        *attempted_genomes = attempted_genomes
            .checked_add(1)
            .ok_or_else(|| optimizer_error("residual attempt counter overflowed"))?;
        let genome = sample_genome(space, rng, distributions);
        let compiled = match compile_genome(parent, parent_tape_bytes, space, &genome) {
            Ok(compiled) => compiled,
            Err(_) => {
                rejected_invalid += 1;
                continue;
            }
        };
        if !seen_tape_sha256.insert(compiled.report.realized_tape_sha256) {
            rejected_duplicate_tape += 1;
            continue;
        }
        let candidate = genome.candidate(parent_tape_bytes, space)?;
        let sample_index = u32::try_from(proposals.len())
            .map_err(|_| optimizer_error("residual sample index overflowed"))?;
        proposals.push(ResidualProposal {
            generation,
            sample_index,
            genome,
            candidate,
            compiled,
        });
        *produced_candidates = produced_candidates
            .checked_add(1)
            .ok_or_else(|| optimizer_error("residual proposal counter overflowed"))?;
    }
    if proposals.len() != count {
        return Err(optimizer_error(format!(
            "residual surface yielded only {} unique valid tapes after {maximum_attempts} attempts",
            proposals.len()
        )));
    }
    Ok(ResidualProposalBatch {
        proposals,
        rejected_invalid,
        rejected_duplicate_tape,
    })
}

fn sample_genome(
    space: &ResidualSearchSpace,
    rng: &mut DeterministicRng,
    distributions: Option<&[ResidualGeneDistribution]>,
) -> ResidualGenome {
    let genes = if let Some(distributions) = distributions {
        distributions
            .iter()
            .map(|distribution| distribution.sample(rng))
            .collect()
    } else {
        (0..space.candidate_slots)
            .map(|_| ResidualGene {
                enabled: rng.bounded(2) == 1,
                kind: if rng.bounded(2) == 0 {
                    ResidualGeneKind::Analog
                } else {
                    ResidualGeneKind::Button
                },
                port_index: rng.bounded(space.ports.len() as u64) as u16,
                channel_index: rng.bounded(space.analog_channels.len() as u64) as u16,
                basis_index: rng.bounded(BASIS_COUNT as u64) as u8,
                start_index: rng.bounded(space.frame_domain() as u64) as u32,
                duration_index: rng.bounded(space.duration_values.len() as u64) as u16,
                delta_indices: std::array::from_fn(|_| {
                    rng.bounded(space.analog_delta_values.len() as u64) as u16
                }),
                button_index: rng.bounded(space.button_masks.len() as u64) as u16,
                button_mode: if rng.bounded(2) == 0 {
                    ResidualGeneButtonMode::Press
                } else {
                    ResidualGeneButtonMode::Release
                },
            })
            .collect()
    };
    ResidualGenome { genes }
}

fn compile_genome(
    parent: &InputTape,
    parent_tape_bytes: &[u8],
    space: &ResidualSearchSpace,
    genome: &ResidualGenome,
) -> Result<CompiledResidualCandidate, ResidualOptimizerError> {
    let candidate = genome.candidate(parent_tape_bytes, space)?;
    compile_residual_candidate(parent, parent_tape_bytes, &candidate).map_err(Into::into)
}

fn render_analog(gene: &ResidualGene, space: &ResidualSearchSpace) -> AnalogResidual {
    let duration = u64::from(space.duration_values[usize::from(gene.duration_index)]);
    let span = match gene.basis_index {
        0 => 1,
        1 => 2,
        2 => 4,
        3 => 8,
        4 => 16,
        5 => 32,
        6 | 7 => duration.max(2),
        _ => unreachable!("validated basis index"),
    };
    let start_frame = bounded_start(space, gene.start_index, span);
    let deltas = gene
        .delta_indices
        .map(|index| space.analog_delta_values[usize::from(index)]);
    let basis = match gene.basis_index {
        0 => TemporalBasis::ExactFrame {
            frame: start_frame,
            delta: deltas[0],
        },
        1..=5 => TemporalBasis::Window {
            start_frame,
            window: match gene.basis_index {
                1 => ResidualWindow::Frames2,
                2 => ResidualWindow::Frames4,
                3 => ResidualWindow::Frames8,
                4 => ResidualWindow::Frames16,
                5 => ResidualWindow::Frames32,
                _ => unreachable!(),
            },
            delta: deltas[0],
        },
        6 => TemporalBasis::PiecewiseLinearRamp {
            start_frame,
            end_frame: start_frame + span - 1,
            start_delta: deltas[0],
            end_delta: deltas[1],
        },
        7 => TemporalBasis::CubicControlCurve {
            start_frame,
            end_frame: start_frame + span - 1,
            control_deltas: deltas,
        },
        _ => unreachable!(),
    };
    AnalogResidual {
        port: space.ports[usize::from(gene.port_index)],
        channel: space.analog_channels[usize::from(gene.channel_index)],
        basis,
    }
}

fn render_button(gene: &ResidualGene, space: &ResidualSearchSpace) -> ButtonResidual {
    let duration_frames = u64::from(space.duration_values[usize::from(gene.duration_index)]);
    ButtonResidual {
        port: space.ports[usize::from(gene.port_index)],
        buttons: space.button_masks[usize::from(gene.button_index)],
        start_frame: bounded_start(space, gene.start_index, duration_frames),
        duration_frames,
        mode: match gene.button_mode {
            ResidualGeneButtonMode::Press => ButtonResidualMode::Press,
            ResidualGeneButtonMode::Release => ButtonResidualMode::Release,
        },
    }
}

fn bounded_start(space: &ResidualSearchSpace, sampled_index: u32, span: u64) -> u64 {
    let valid_starts = (space.end_frame_exclusive - space.start_frame) - span + 1;
    space.start_frame + u64::from(sampled_index) % valid_starts
}

fn probability_from_counts(counts: &[u32], total: u32) -> Vec<u32> {
    let raw = counts
        .iter()
        .map(|count| u64::from(*count) * u64::from(CEM_MASS) / u64::from(total))
        .collect::<Vec<_>>();
    normalize_probability(raw)
}

fn normalize_probability(raw: Vec<u64>) -> Vec<u32> {
    let categories = raw.len();
    let floor = (CEM_MASS / (categories as u32).saturating_mul(100)).max(1);
    let mut weights = raw
        .into_iter()
        .map(|weight| u32::try_from(weight).unwrap_or(CEM_MASS).max(floor))
        .collect::<Vec<_>>();
    let mut total = weights.iter().map(|weight| u64::from(*weight)).sum::<u64>();
    if total < u64::from(CEM_MASS) {
        let index = weights
            .iter()
            .enumerate()
            .max_by_key(|(index, weight)| (**weight, std::cmp::Reverse(*index)))
            .map(|(index, _)| index)
            .unwrap_or(0);
        weights[index] += (u64::from(CEM_MASS) - total) as u32;
        total = u64::from(CEM_MASS);
    }
    if total > u64::from(CEM_MASS) {
        let mut excess = total - u64::from(CEM_MASS);
        let mut indices = (0..weights.len()).collect::<Vec<_>>();
        indices.sort_by_key(|index| (std::cmp::Reverse(weights[*index]), *index));
        for index in indices {
            let removable = u64::from(weights[index] - floor).min(excess);
            weights[index] -= removable as u32;
            excess -= removable;
            if excess == 0 {
                break;
            }
        }
        debug_assert_eq!(excess, 0);
    }
    debug_assert_eq!(
        weights.iter().map(|weight| u64::from(*weight)).sum::<u64>(),
        u64::from(CEM_MASS)
    );
    weights
}

fn validate_batch_count(count: usize) -> Result<(), ResidualOptimizerError> {
    if count == 0 || count > MAX_PROPOSALS_PER_BATCH {
        return Err(optimizer_error(
            "residual proposal batch must contain 1..=16384 candidates",
        ));
    }
    Ok(())
}

fn validate_cem_config(config: ResidualCemConfig) -> Result<(), ResidualOptimizerError> {
    if config.population < 2
        || config.population > MAX_PROPOSALS_PER_BATCH
        || config.elites == 0
        || config.elites >= config.population
        || config.smoothing_millionths == 0
        || config.smoothing_millionths > CEM_MASS
    {
        return Err(optimizer_error(
            "residual CEM population, elites, or smoothing is invalid",
        ));
    }
    Ok(())
}

fn strictly_increasing<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn canonical_digest(
    domain: &[u8],
    value: &impl Serialize,
) -> Result<Digest, ResidualOptimizerError> {
    let bytes = serde_json::to_vec(value).map_err(|source| optimizer_error(source.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

fn sha256_bytes(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

#[derive(Clone, Copy, Debug)]
struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn bounded(&mut self, bound: u64) -> u64 {
        debug_assert!(bound > 0);
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let value = self.next();
            if value >= threshold {
                return value % bound;
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResidualOptimizerError(String);

fn optimizer_error(message: impl Into<String>) -> ResidualOptimizerError {
    ResidualOptimizerError(message.into())
}

impl From<ResidualActionError> for ResidualOptimizerError {
    fn from(source: ResidualActionError) -> Self {
        optimizer_error(source.to_string())
    }
}

impl fmt::Display for ResidualOptimizerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ResidualOptimizerError {}

#[cfg(test)]
mod tests {
    use super::*;
    use dusklight_automation_contracts::tape::InputFrame;

    fn parent(frame_count: usize) -> (InputTape, Vec<u8>) {
        let tape = InputTape {
            frames: (0..frame_count)
                .map(|_| InputFrame {
                    owned_ports: 1,
                    ..InputFrame::default()
                })
                .collect(),
            ..InputTape::default()
        };
        let bytes = tape.encode().unwrap();
        (tape, bytes)
    }

    fn space() -> ResidualSearchSpace {
        ResidualSearchSpace {
            schema: RESIDUAL_SEARCH_SPACE_SCHEMA_V1.into(),
            start_frame: 0,
            end_frame_exclusive: 96,
            candidate_slots: 4,
            ports: vec![0],
            analog_channels: vec![
                AnalogChannel::MainX,
                AnalogChannel::MainY,
                AnalogChannel::CameraX,
                AnalogChannel::CameraY,
            ],
            analog_delta_values: vec![-64, -16, -4, 4, 16, 64],
            button_masks: vec![0x0010, 0x0020, 0x0040, 0x0100, 0x0200, 0x0400],
            duration_values: vec![1, 2, 4, 8, 16, 32],
        }
    }

    #[test]
    fn random_sampler_is_seeded_independent_and_compiles_unique_raw_tapes() {
        let (parent, bytes) = parent(96);
        let mut first = ResidualRandomSampler::new(space(), &bytes, 104_729).unwrap();
        let mut second = ResidualRandomSampler::new(space(), &bytes, 104_729).unwrap();
        let left = first.sample(&parent, &bytes, 64).unwrap();
        let right = second.sample(&parent, &bytes, 64).unwrap();
        assert_eq!(
            left.proposals
                .iter()
                .map(|proposal| proposal.compiled.report.realized_tape_sha256)
                .collect::<Vec<_>>(),
            right
                .proposals
                .iter()
                .map(|proposal| proposal.compiled.report.realized_tape_sha256)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            left.proposals
                .iter()
                .map(|proposal| proposal.compiled.report.realized_tape_sha256)
                .collect::<BTreeSet<_>>()
                .len(),
            64
        );
        assert!(left.proposals.iter().any(|proposal| {
            proposal.candidate.analog.len() + proposal.candidate.buttons.len() > 1
        }));
        assert!(left.proposals.iter().all(|proposal| {
            proposal.compiled.report.realized_tape_authoritative
                && proposal.compiled.report.parent_tape_sha256 == sha256(&bytes)
        }));
        let snapshot = first.snapshot().unwrap();
        assert_eq!(snapshot.rejected_invalid_genomes, left.rejected_invalid);
        assert_eq!(
            snapshot.rejected_duplicate_tapes,
            left.rejected_duplicate_tape
        );
        assert_eq!(
            snapshot.attempted_genomes,
            snapshot.produced_candidates
                + snapshot.rejected_invalid_genomes
                + snapshot.rejected_duplicate_tapes
        );
    }

    #[test]
    fn random_snapshot_resumes_without_repeating_or_skipping() {
        let (parent, bytes) = parent(96);
        let mut uninterrupted = ResidualRandomSampler::new(space(), &bytes, 17).unwrap();
        let all = uninterrupted.sample(&parent, &bytes, 24).unwrap();

        let mut interrupted = ResidualRandomSampler::new(space(), &bytes, 17).unwrap();
        let prefix = interrupted.sample(&parent, &bytes, 9).unwrap();
        let snapshot = interrupted.snapshot().unwrap();
        let mut resumed = ResidualRandomSampler::restore(space(), &bytes, snapshot).unwrap();
        let suffix = resumed.sample(&parent, &bytes, 15).unwrap();
        let joined = prefix
            .proposals
            .iter()
            .chain(&suffix.proposals)
            .map(|proposal| proposal.compiled.report.realized_tape_sha256)
            .collect::<Vec<_>>();
        assert_eq!(
            joined,
            all.proposals
                .iter()
                .map(|proposal| proposal.compiled.report.realized_tape_sha256)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn categorical_cem_updates_only_from_exact_rank_and_resumes_byte_exactly() {
        let (parent, bytes) = parent(96);
        let config = ResidualCemConfig {
            population: 12,
            elites: 3,
            smoothing_millionths: 250_000,
            seed: 31,
        };
        let mut optimizer = ResidualCemOptimizer::new(space(), &bytes, config).unwrap();
        let first = optimizer.ask(&parent, &bytes).unwrap();
        assert_eq!(first.proposals.len(), 12);
        assert!(optimizer.ask(&parent, &bytes).is_err());
        let mut ranking = first
            .proposals
            .iter()
            .map(|proposal| proposal.candidate.content_sha256)
            .collect::<Vec<_>>();
        ranking.sort();
        let before = optimizer.snapshot().unwrap();
        assert_eq!(before.rejected_invalid_genomes, first.rejected_invalid);
        assert_eq!(
            before.rejected_duplicate_tapes,
            first.rejected_duplicate_tape
        );
        assert!(optimizer.tell(&ranking[..11]).is_err());
        optimizer.tell(&ranking).unwrap();
        let updated = optimizer.snapshot().unwrap();
        assert_eq!(updated.generation, 1);
        assert_ne!(updated.distributions, before.distributions);

        let restored =
            ResidualCemOptimizer::restore(space(), config, &bytes, updated.clone()).unwrap();
        assert_eq!(restored.snapshot().unwrap(), updated);
    }

    #[test]
    fn random_and_cem_share_the_same_genome_renderer_and_complete_basis_catalog() {
        let (parent, bytes) = parent(96);
        let search_space = space();
        let genome = ResidualGenome {
            genes: vec![
                ResidualGene {
                    enabled: true,
                    kind: ResidualGeneKind::Analog,
                    port_index: 0,
                    channel_index: 0,
                    basis_index: 7,
                    start_index: 90,
                    duration_index: 3,
                    delta_indices: [0, 1, 4, 5],
                    button_index: 0,
                    button_mode: ResidualGeneButtonMode::Press,
                },
                ResidualGene {
                    enabled: true,
                    kind: ResidualGeneKind::Button,
                    port_index: 0,
                    channel_index: 3,
                    basis_index: 0,
                    start_index: 95,
                    duration_index: 2,
                    delta_indices: [0; 4],
                    button_index: 3,
                    button_mode: ResidualGeneButtonMode::Press,
                },
                ResidualGene {
                    enabled: false,
                    kind: ResidualGeneKind::Analog,
                    port_index: 0,
                    channel_index: 0,
                    basis_index: 0,
                    start_index: 0,
                    duration_index: 0,
                    delta_indices: [0; 4],
                    button_index: 0,
                    button_mode: ResidualGeneButtonMode::Release,
                },
                ResidualGene {
                    enabled: false,
                    kind: ResidualGeneKind::Button,
                    port_index: 0,
                    channel_index: 0,
                    basis_index: 0,
                    start_index: 0,
                    duration_index: 0,
                    delta_indices: [0; 4],
                    button_index: 0,
                    button_mode: ResidualGeneButtonMode::Release,
                },
            ],
        };
        let candidate = genome.candidate(&bytes, &search_space).unwrap();
        assert!(matches!(
            candidate.analog[0].basis,
            TemporalBasis::CubicControlCurve { .. }
        ));
        assert_eq!(candidate.buttons[0].duration_frames, 4);
        assert!(compile_genome(&parent, &bytes, &search_space, &genome).is_ok());
        assert_eq!(BASIS_COUNT, 8);
    }

    #[test]
    fn checked_ordon_canary_surface_can_express_the_exact_q125_repair() {
        let repository_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        let segment_root = repository_root.join("routes/Glitch Exhibition/intro/segments");
        let degraded_bytes =
            std::fs::read(segment_root.join("to_ordon_spring_degraded_q131.tape")).unwrap();
        let q125_bytes = std::fs::read(segment_root.join("to_ordon_spring_q125.tape")).unwrap();
        let degraded = InputTape::decode(&degraded_bytes).unwrap().tape;
        let search_space = ResidualSearchSpace {
            schema: RESIDUAL_SEARCH_SPACE_SCHEMA_V1.into(),
            start_frame: 0,
            end_frame_exclusive: 126,
            candidate_slots: 4,
            ports: vec![0],
            analog_channels: vec![
                AnalogChannel::MainX,
                AnalogChannel::MainY,
                AnalogChannel::CameraX,
                AnalogChannel::CameraY,
            ],
            analog_delta_values: vec![-64, -32, -16, -8, -4, 4, 8, 16, 32, 64],
            button_masks: vec![1, 2, 4, 8, 16, 32, 64, 256, 512, 1024, 2048, 4096],
            duration_values: vec![1, 2, 4, 8, 16, 32],
        };
        search_space.validate().unwrap();
        assert!(search_space.ports.contains(&0));
        assert!(search_space.button_masks.contains(&0x0100));
        assert!(search_space.duration_values.contains(&1));
        assert!((search_space.start_frame..search_space.end_frame_exclusive).contains(&100));

        // This witness proves only that the sealed residual language can express
        // the known repair. It is intentionally never inserted into CEM's
        // population, replay, rank, or proposal distribution.
        let witness = ResidualCandidate::seal(
            &degraded_bytes,
            Vec::new(),
            vec![ButtonResidual {
                port: 0,
                buttons: 0x0100,
                start_frame: 100,
                duration_frames: 1,
                mode: ButtonResidualMode::Press,
            }],
        )
        .unwrap();
        let compiled = compile_residual_candidate(&degraded, &degraded_bytes, &witness).unwrap();

        assert_eq!(compiled.bytes, q125_bytes);
        assert_eq!(witness.buttons.len(), 1);
        assert_eq!(witness.buttons[0].start_frame, 100);
    }

    #[test]
    fn cem_generation_zero_proposes_interior_button_cadence_repairs() {
        let (mut tape, _) = parent(96);
        for frame in [10, 30, 50, 90] {
            tape.frames[frame].pads[0].buttons = 0x0100;
        }
        let bytes = tape.encode().unwrap();
        let config = ResidualCemConfig {
            population: 12,
            elites: 3,
            smoothing_millionths: 250_000,
            seed: 31,
        };
        let mut optimizer = ResidualCemOptimizer::new(space(), &bytes, config).unwrap();
        let batch = optimizer.ask(&tape, &bytes).unwrap();
        assert!(batch.proposals.iter().any(|proposal| {
            proposal.candidate.buttons
                == vec![ButtonResidual {
                    port: 0,
                    buttons: 0x0100,
                    start_frame: 70,
                    duration_frames: 1,
                    mode: ButtonResidualMode::Press,
                }]
        }));
    }

    #[test]
    fn checked_ordon_canary_generation_zero_contains_the_q125_repair() {
        let repository_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        let segment_root = repository_root.join("routes/Glitch Exhibition/intro/segments");
        let degraded_bytes =
            std::fs::read(segment_root.join("to_ordon_spring_degraded_q131.tape")).unwrap();
        let q125_bytes = std::fs::read(segment_root.join("to_ordon_spring_q125.tape")).unwrap();
        let degraded = InputTape::decode(&degraded_bytes).unwrap().tape;
        let search_space = ResidualSearchSpace {
            schema: RESIDUAL_SEARCH_SPACE_SCHEMA_V1.into(),
            start_frame: 0,
            end_frame_exclusive: 126,
            candidate_slots: 4,
            ports: vec![0],
            analog_channels: vec![
                AnalogChannel::MainX,
                AnalogChannel::MainY,
                AnalogChannel::CameraX,
                AnalogChannel::CameraY,
            ],
            analog_delta_values: vec![-64, -32, -16, -8, -4, 4, 8, 16, 32, 64],
            button_masks: vec![1, 2, 4, 8, 16, 32, 64, 256, 512, 1024, 2048, 4096],
            duration_values: vec![1, 2, 4, 8, 16, 32],
        };
        let config = ResidualCemConfig {
            population: 64,
            elites: 8,
            smoothing_millionths: 250_000,
            seed: 104_729,
        };
        let mut optimizer =
            ResidualCemOptimizer::new(search_space, &degraded_bytes, config).unwrap();
        let batch = optimizer.ask(&degraded, &degraded_bytes).unwrap();
        assert!(
            batch
                .proposals
                .iter()
                .any(|proposal| proposal.compiled.bytes == q125_bytes)
        );
    }

    #[test]
    fn detached_spaces_genomes_and_snapshots_fail_closed() {
        let (parent, bytes) = parent(96);
        let mut invalid = space();
        invalid.analog_delta_values.push(0);
        assert!(invalid.validate().is_err());

        let mut sampler = ResidualRandomSampler::new(space(), &bytes, 1).unwrap();
        sampler.sample(&parent, &bytes, 2).unwrap();
        let mut snapshot = sampler.snapshot().unwrap();
        snapshot.search_space_sha256 = Digest([9; 32]);
        snapshot.content_sha256 = snapshot.compute_identity().unwrap();
        assert!(ResidualRandomSampler::restore(space(), &bytes, snapshot).is_err());

        let mut short_parent = parent.clone();
        short_parent.frames.truncate(40);
        assert!(space().validate_parent(&short_parent).is_err());

        let config = ResidualCemConfig {
            population: 4,
            elites: 1,
            smoothing_millionths: 250_000,
            seed: 9,
        };
        let mut cem = ResidualCemOptimizer::new(space(), &bytes, config).unwrap();
        cem.ask(&parent, &bytes).unwrap();
        let cem_snapshot = cem.snapshot().unwrap();
        let different_config = ResidualCemConfig {
            smoothing_millionths: 500_000,
            ..config
        };
        assert!(
            ResidualCemOptimizer::restore(space(), different_config, &bytes, cem_snapshot.clone())
                .is_err()
        );
        let mut detached_pending = cem_snapshot;
        detached_pending.pending[0].candidate_sha256 = Digest([8; 32]);
        detached_pending.content_sha256 = detached_pending.compute_identity().unwrap();
        assert!(ResidualCemOptimizer::restore(space(), config, &bytes, detached_pending).is_err());
    }

    fn sha256(bytes: &[u8]) -> Digest {
        Digest(Sha256::digest(bytes).into())
    }
}
