//! Independent random and CEM proposal policy over one shared residual surface.

use crate::residual_action::{
    AnalogResidual, ButtonResidual, ResidualActionError, ResidualCandidate,
};
use dusklight_automation_contracts::artifact::Digest;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

pub const RESIDUAL_SEARCH_SPACE_SCHEMA_V1: &str = "dusklight-residual-search-space/v1";
pub const RESIDUAL_CEM_SNAPSHOT_SCHEMA_V1: &str = "dusklight-residual-cem-snapshot/v1";
const MAX_ATOMS: usize = 65_536;
const MAX_ACTIVE_ATOMS: usize = 16;
const MAX_PROPOSALS: usize = 16_384;
const PROBABILITY_SCALE: u32 = 1_000_000;
const MIN_CEM_PROBABILITY: u32 = 1_000;
const MAX_CEM_PROBABILITY: u32 = PROBABILITY_SCALE - MIN_CEM_PROBABILITY;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualAtom {
    pub id: String,
    pub analog: Vec<AnalogResidual>,
    pub buttons: Vec<ButtonResidual>,
}

impl ResidualAtom {
    pub fn new(
        id: impl Into<String>,
        mut analog: Vec<AnalogResidual>,
        mut buttons: Vec<ButtonResidual>,
    ) -> Self {
        analog.sort();
        buttons.sort();
        Self {
            id: id.into(),
            analog,
            buttons,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualSearchSpace {
    pub schema: String,
    pub content_sha256: Digest,
    pub parent_tape_sha256: Digest,
    pub maximum_active_atoms: u8,
    pub atoms: Vec<ResidualAtom>,
}

impl ResidualSearchSpace {
    pub fn seal(
        parent_tape_bytes: &[u8],
        maximum_active_atoms: u8,
        mut atoms: Vec<ResidualAtom>,
    ) -> Result<Self, ResidualSearchError> {
        atoms.sort_by(|left, right| left.id.cmp(&right.id));
        let mut space = Self {
            schema: RESIDUAL_SEARCH_SPACE_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            parent_tape_sha256: sha256(parent_tape_bytes),
            maximum_active_atoms,
            atoms,
        };
        space.content_sha256 = space.compute_identity()?;
        space.validate(parent_tape_bytes)?;
        Ok(space)
    }

    pub fn validate(&self, parent_tape_bytes: &[u8]) -> Result<(), ResidualSearchError> {
        if self.schema != RESIDUAL_SEARCH_SPACE_SCHEMA_V1
            || self.parent_tape_sha256 == Digest::ZERO
            || self.parent_tape_sha256 != sha256(parent_tape_bytes)
            || self.atoms.is_empty()
            || self.atoms.len() > MAX_ATOMS
            || self.maximum_active_atoms == 0
            || usize::from(self.maximum_active_atoms) > MAX_ACTIVE_ATOMS
            || usize::from(self.maximum_active_atoms) > self.atoms.len()
            || !self.atoms.windows(2).all(|pair| pair[0].id < pair[1].id)
        {
            return Err(search_error(
                "residual search-space envelope, parent, or bounds are invalid",
            ));
        }
        for atom in &self.atoms {
            validate_id(&atom.id)?;
            if atom.analog.is_empty() && atom.buttons.is_empty() {
                return Err(search_error("residual search atom is empty"));
            }
            ResidualCandidate::seal(parent_tape_bytes, atom.analog.clone(), atom.buttons.clone())
                .map_err(ResidualSearchError::from)?;
        }
        if self.content_sha256 == Digest::ZERO || self.content_sha256 != self.compute_identity()? {
            return Err(search_error(
                "residual search-space content seal is invalid",
            ));
        }
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, ResidualSearchError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.residual-search-space/v1\0", &canonical)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualProposal {
    pub generation: u64,
    pub sample_index: u32,
    pub atom_indices: Vec<u32>,
    pub candidate: ResidualCandidate,
}

/// Independent baseline sampler. Its RNG domain and selection policy are not
/// shared with CEM, so matching results are evidence from the shared action
/// surface rather than the same proposal stream under two names.
pub fn sample_random_residuals(
    space: &ResidualSearchSpace,
    parent_tape_bytes: &[u8],
    seed: u64,
    proposal_count: usize,
) -> Result<Vec<ResidualProposal>, ResidualSearchError> {
    space.validate(parent_tape_bytes)?;
    if proposal_count == 0 || proposal_count > MAX_PROPOSALS {
        return Err(search_error("random residual proposal count is invalid"));
    }
    let mut rng = DeterministicRng::new(seed ^ 0x7261_6e64_6f6d_5f72);
    let mut proposals = Vec::with_capacity(proposal_count);
    let mut seen = BTreeSet::new();
    let attempt_limit = proposal_count.saturating_mul(256);
    for _ in 0..attempt_limit {
        if proposals.len() == proposal_count {
            break;
        }
        let count = 1 + rng.index(usize::from(space.maximum_active_atoms));
        let mut selected = BTreeSet::new();
        while selected.len() < count {
            selected.insert(rng.index(space.atoms.len()) as u32);
        }
        let atom_indices = selected.into_iter().collect::<Vec<_>>();
        let candidate = materialize(space, parent_tape_bytes, &atom_indices)?;
        if seen.insert(candidate.content_sha256) {
            proposals.push(ResidualProposal {
                generation: 0,
                sample_index: proposals.len() as u32,
                atom_indices,
                candidate,
            });
        }
    }
    if proposals.len() != proposal_count {
        return Err(search_error(
            "random residual sampler exhausted unique structural candidates",
        ));
    }
    Ok(proposals)
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualCemConfig {
    pub population: u32,
    pub elites: u32,
    pub smoothing_millionths: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualCemSnapshot {
    pub schema: String,
    pub content_sha256: Digest,
    pub search_space_sha256: Digest,
    pub config: ResidualCemConfig,
    pub generation: u64,
    pub rng_state: u64,
    pub atom_probabilities_millionths: Vec<u32>,
    pub pending: Vec<ResidualProposal>,
}

impl ResidualCemSnapshot {
    pub fn validate(&self, space: &ResidualSearchSpace) -> Result<(), ResidualSearchError> {
        validate_cem_config(self.config)?;
        if self.schema != RESIDUAL_CEM_SNAPSHOT_SCHEMA_V1
            || self.search_space_sha256 != space.content_sha256
            || self.atom_probabilities_millionths.len() != space.atoms.len()
            || self
                .atom_probabilities_millionths
                .iter()
                .any(|probability| {
                    !(MIN_CEM_PROBABILITY..=MAX_CEM_PROBABILITY).contains(probability)
                })
            || self.pending.len() > self.config.population as usize
            || self.pending.iter().any(|proposal| {
                proposal.generation != self.generation
                    || proposal.atom_indices.is_empty()
                    || proposal.atom_indices.len() > usize::from(space.maximum_active_atoms)
            })
            || self.content_sha256 == Digest::ZERO
            || self.content_sha256 != self.compute_identity()?
        {
            return Err(search_error("residual CEM snapshot is invalid"));
        }
        Ok(())
    }

    fn reseal(&mut self) -> Result<(), ResidualSearchError> {
        self.content_sha256 = Digest::ZERO;
        self.content_sha256 = self.compute_identity()?;
        Ok(())
    }

    fn compute_identity(&self) -> Result<Digest, ResidualSearchError> {
        let mut canonical = self.clone();
        canonical.content_sha256 = Digest::ZERO;
        canonical_digest(b"dusklight.residual-cem-snapshot/v1\0", &canonical)
    }
}

#[derive(Clone, Debug)]
pub struct ResidualCem {
    space: ResidualSearchSpace,
    snapshot: ResidualCemSnapshot,
}

impl ResidualCem {
    pub fn new(
        space: ResidualSearchSpace,
        parent_tape_bytes: &[u8],
        config: ResidualCemConfig,
        seed: u64,
    ) -> Result<Self, ResidualSearchError> {
        space.validate(parent_tape_bytes)?;
        validate_cem_config(config)?;
        let initial = ((u64::from(space.maximum_active_atoms) * u64::from(PROBABILITY_SCALE))
            / space.atoms.len() as u64)
            .clamp(
                u64::from(MIN_CEM_PROBABILITY),
                u64::from(MAX_CEM_PROBABILITY),
            ) as u32;
        let mut snapshot = ResidualCemSnapshot {
            schema: RESIDUAL_CEM_SNAPSHOT_SCHEMA_V1.into(),
            content_sha256: Digest::ZERO,
            search_space_sha256: space.content_sha256,
            config,
            generation: 0,
            rng_state: seed ^ 0x6365_6d5f_7265_7369,
            atom_probabilities_millionths: vec![initial; space.atoms.len()],
            pending: Vec::new(),
        };
        snapshot.reseal()?;
        snapshot.validate(&space)?;
        Ok(Self { space, snapshot })
    }

    pub fn restore(
        space: ResidualSearchSpace,
        parent_tape_bytes: &[u8],
        snapshot: ResidualCemSnapshot,
    ) -> Result<Self, ResidualSearchError> {
        space.validate(parent_tape_bytes)?;
        snapshot.validate(&space)?;
        for proposal in &snapshot.pending {
            validate_proposal(&space, parent_tape_bytes, proposal)?;
        }
        Ok(Self { space, snapshot })
    }

    pub fn snapshot(&self) -> &ResidualCemSnapshot {
        &self.snapshot
    }

    pub fn ask(
        &mut self,
        parent_tape_bytes: &[u8],
    ) -> Result<Vec<ResidualProposal>, ResidualSearchError> {
        self.space.validate(parent_tape_bytes)?;
        if !self.snapshot.pending.is_empty() {
            return Err(search_error(
                "residual CEM generation is already awaiting ranked results",
            ));
        }
        let mut rng = DeterministicRng(self.snapshot.rng_state);
        let mut proposals = Vec::with_capacity(self.snapshot.config.population as usize);
        let mut seen = BTreeSet::new();
        let attempt_limit = self.snapshot.config.population as usize * 512;
        for _ in 0..attempt_limit {
            if proposals.len() == self.snapshot.config.population as usize {
                break;
            }
            let atom_indices = draw_cem_selection(
                &self.snapshot.atom_probabilities_millionths,
                usize::from(self.space.maximum_active_atoms),
                &mut rng,
            );
            let candidate = materialize(&self.space, parent_tape_bytes, &atom_indices)?;
            if seen.insert(candidate.content_sha256) {
                proposals.push(ResidualProposal {
                    generation: self.snapshot.generation,
                    sample_index: proposals.len() as u32,
                    atom_indices,
                    candidate,
                });
            }
        }
        if proposals.len() != self.snapshot.config.population as usize {
            return Err(search_error(
                "residual CEM exhausted unique structural candidates",
            ));
        }
        self.snapshot.rng_state = rng.0;
        self.snapshot.pending = proposals.clone();
        self.snapshot.reseal()?;
        Ok(proposals)
    }

    /// Update from the complete generation sorted best-to-worst by external
    /// deterministic rollout rank. Fitness magnitudes are deliberately absent.
    pub fn tell(
        &mut self,
        parent_tape_bytes: &[u8],
        ranked: &[ResidualProposal],
    ) -> Result<(), ResidualSearchError> {
        if ranked.len() != self.snapshot.pending.len()
            || ranked.len() != self.snapshot.config.population as usize
        {
            return Err(search_error(
                "residual CEM requires the complete pending generation",
            ));
        }
        let pending = self
            .snapshot
            .pending
            .iter()
            .map(|proposal| (proposal.sample_index, proposal.candidate.content_sha256))
            .collect::<BTreeMap<_, _>>();
        let mut ranked_ids = BTreeSet::new();
        for proposal in ranked {
            validate_proposal(&self.space, parent_tape_bytes, proposal)?;
            if proposal.generation != self.snapshot.generation
                || pending.get(&proposal.sample_index) != Some(&proposal.candidate.content_sha256)
                || !ranked_ids.insert(proposal.sample_index)
            {
                return Err(search_error("residual CEM ranking is stale or detached"));
            }
        }
        let elites = &ranked[..self.snapshot.config.elites as usize];
        let smoothing = u64::from(self.snapshot.config.smoothing_millionths);
        for atom in 0..self.space.atoms.len() {
            let selected = elites
                .iter()
                .filter(|proposal| proposal.atom_indices.binary_search(&(atom as u32)).is_ok())
                .count() as u64;
            let target = selected * u64::from(PROBABILITY_SCALE) / elites.len() as u64;
            let old = u64::from(self.snapshot.atom_probabilities_millionths[atom]);
            let updated = ((u64::from(PROBABILITY_SCALE) - smoothing) * old
                + smoothing * target
                + u64::from(PROBABILITY_SCALE) / 2)
                / u64::from(PROBABILITY_SCALE);
            self.snapshot.atom_probabilities_millionths[atom] = updated.clamp(
                u64::from(MIN_CEM_PROBABILITY),
                u64::from(MAX_CEM_PROBABILITY),
            ) as u32;
        }
        self.snapshot.generation = self
            .snapshot
            .generation
            .checked_add(1)
            .ok_or_else(|| search_error("residual CEM generation overflowed"))?;
        self.snapshot.pending.clear();
        self.snapshot.reseal()?;
        self.snapshot.validate(&self.space)
    }
}

fn materialize(
    space: &ResidualSearchSpace,
    parent_tape_bytes: &[u8],
    atom_indices: &[u32],
) -> Result<ResidualCandidate, ResidualSearchError> {
    if atom_indices.is_empty()
        || atom_indices.len() > usize::from(space.maximum_active_atoms)
        || !atom_indices.windows(2).all(|pair| pair[0] < pair[1])
    {
        return Err(search_error("residual atom selection is noncanonical"));
    }
    let mut analog = BTreeSet::new();
    let mut buttons = BTreeSet::new();
    for index in atom_indices {
        let atom = space
            .atoms
            .get(*index as usize)
            .ok_or_else(|| search_error("residual atom selection is out of bounds"))?;
        analog.extend(atom.analog.iter().cloned());
        buttons.extend(atom.buttons.iter().cloned());
    }
    ResidualCandidate::seal(
        parent_tape_bytes,
        analog.into_iter().collect(),
        buttons.into_iter().collect(),
    )
    .map_err(ResidualSearchError::from)
}

fn validate_proposal(
    space: &ResidualSearchSpace,
    parent_tape_bytes: &[u8],
    proposal: &ResidualProposal,
) -> Result<(), ResidualSearchError> {
    if proposal.generation == u64::MAX
        || proposal.candidate != materialize(space, parent_tape_bytes, &proposal.atom_indices)?
    {
        return Err(search_error("residual proposal is detached from its atoms"));
    }
    Ok(())
}

fn draw_cem_selection(
    probabilities: &[u32],
    maximum_active: usize,
    rng: &mut DeterministicRng,
) -> Vec<u32> {
    let mut selected = probabilities
        .iter()
        .enumerate()
        .filter_map(|(index, probability)| {
            let draw = rng.next() as u32 % PROBABILITY_SCALE;
            (draw < *probability).then_some((draw, index as u32))
        })
        .collect::<Vec<_>>();
    if selected.is_empty() {
        let maximum = probabilities.iter().copied().max().unwrap_or(0);
        let tied = probabilities
            .iter()
            .enumerate()
            .filter_map(|(index, probability)| (*probability == maximum).then_some(index as u32))
            .collect::<Vec<_>>();
        selected.push((0, tied[rng.index(tied.len())]));
    }
    selected.sort_by_key(|(draw, index)| (*draw, *index));
    selected.truncate(maximum_active);
    let mut indices = selected
        .into_iter()
        .map(|(_, index)| index)
        .collect::<Vec<_>>();
    indices.sort_unstable();
    indices
}

fn validate_cem_config(config: ResidualCemConfig) -> Result<(), ResidualSearchError> {
    if config.population < 2
        || config.population as usize > MAX_PROPOSALS
        || config.elites == 0
        || config.elites >= config.population
        || config.smoothing_millionths == 0
        || config.smoothing_millionths > PROBABILITY_SCALE
    {
        return Err(search_error("residual CEM configuration is invalid"));
    }
    Ok(())
}

fn validate_id(value: &str) -> Result<(), ResidualSearchError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        return Err(search_error("residual atom ID is invalid"));
    }
    Ok(())
}

fn canonical_digest(domain: &[u8], value: &impl Serialize) -> Result<Digest, ResidualSearchError> {
    let bytes = serde_json::to_vec(value).map_err(|source| search_error(source.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Ok(Digest(hasher.finalize().into()))
}

fn sha256(bytes: &[u8]) -> Digest {
    Digest(Sha256::digest(bytes).into())
}

#[derive(Clone, Copy, Debug)]
struct DeterministicRng(u64);

impl DeterministicRng {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9e37_79b9_7f4a_7c15)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn index(&mut self, length: usize) -> usize {
        (self.next() % length as u64) as usize
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResidualSearchError(String);

fn search_error(message: impl Into<String>) -> ResidualSearchError {
    ResidualSearchError(message.into())
}

impl fmt::Display for ResidualSearchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ResidualSearchError {}

impl From<ResidualActionError> for ResidualSearchError {
    fn from(value: ResidualActionError) -> Self {
        Self(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::residual_action::{AnalogChannel, TemporalBasis};
    use dusklight_automation_contracts::tape::{InputFrame, InputTape};

    fn parent() -> (InputTape, Vec<u8>) {
        let tape = InputTape {
            frames: (0..16)
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

    fn space(bytes: &[u8]) -> ResidualSearchSpace {
        let atoms = (0..8)
            .map(|frame| {
                ResidualAtom::new(
                    format!("main-x-frame-{frame}"),
                    vec![AnalogResidual {
                        port: 0,
                        channel: AnalogChannel::MainX,
                        basis: TemporalBasis::ExactFrame {
                            frame,
                            delta: frame as i16 + 1,
                        },
                    }],
                    vec![],
                )
            })
            .collect();
        ResidualSearchSpace::seal(bytes, 3, atoms).unwrap()
    }

    #[test]
    fn independent_random_sampler_is_seeded_bounded_and_structurally_unique() {
        let (_, bytes) = parent();
        let space = space(&bytes);
        let first = sample_random_residuals(&space, &bytes, 104_729, 12).unwrap();
        let repeated = sample_random_residuals(&space, &bytes, 104_729, 12).unwrap();
        let different = sample_random_residuals(&space, &bytes, 130_363, 12).unwrap();
        assert_eq!(first, repeated);
        assert_ne!(first, different);
        assert!(first.iter().all(|proposal| {
            !proposal.atom_indices.is_empty()
                && proposal.atom_indices.len() <= 3
                && proposal.candidate.validate().is_ok()
        }));
        assert_eq!(
            first
                .iter()
                .map(|proposal| proposal.candidate.content_sha256)
                .collect::<BTreeSet<_>>()
                .len(),
            first.len()
        );
    }

    #[test]
    fn cem_learns_ranked_atoms_and_restores_pending_rng_exactly() {
        let (_, bytes) = parent();
        let space = space(&bytes);
        let config = ResidualCemConfig {
            population: 12,
            elites: 3,
            smoothing_millionths: 500_000,
        };
        let mut cem = ResidualCem::new(space.clone(), &bytes, config, 155_921).unwrap();
        let initial_target = cem.snapshot().atom_probabilities_millionths[0];
        for _ in 0..4 {
            let mut proposals = cem.ask(&bytes).unwrap();
            proposals.sort_by_key(|proposal| {
                (
                    proposal.atom_indices.binary_search(&0).is_err(),
                    proposal.atom_indices.len(),
                    proposal.sample_index,
                )
            });
            cem.tell(&bytes, &proposals).unwrap();
        }
        assert!(cem.snapshot().atom_probabilities_millionths[0] > initial_target);

        let pending = cem.ask(&bytes).unwrap();
        let snapshot = cem.snapshot().clone();
        let mut restored = ResidualCem::restore(space.clone(), &bytes, snapshot).unwrap();
        assert!(restored.ask(&bytes).is_err());
        let mut ranked = pending;
        ranked.sort_by_key(|proposal| proposal.sample_index);
        cem.tell(&bytes, &ranked).unwrap();
        restored.tell(&bytes, &ranked).unwrap();
        assert_eq!(cem.snapshot(), restored.snapshot());
        assert_eq!(cem.ask(&bytes).unwrap(), restored.ask(&bytes).unwrap());
    }

    #[test]
    fn random_and_cem_materialize_the_same_authoritative_candidate_type() {
        let (parent, bytes) = parent();
        let space = space(&bytes);
        let random = sample_random_residuals(&space, &bytes, 181_081, 2).unwrap();
        let mut cem = ResidualCem::new(
            space,
            &bytes,
            ResidualCemConfig {
                population: 2,
                elites: 1,
                smoothing_millionths: 250_000,
            },
            181_081,
        )
        .unwrap();
        let cem = cem.ask(&bytes).unwrap();
        for proposal in random.iter().chain(&cem) {
            let compiled = crate::residual_action::compile_residual_candidate(
                &parent,
                &bytes,
                &proposal.candidate,
            )
            .unwrap();
            assert!(compiled.report.realized_tape_authoritative);
        }
    }
}
