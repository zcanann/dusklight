use super::{ExactStateId, FutureEquivalenceProof, StateGraph, StateGraphError};
use dusklight_automation_contracts::artifact::Digest;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};
use std::sync::Arc;

impl StateGraph {
    /// Admit externally authenticated native future-equivalence evidence.
    ///
    /// Exact nodes and all incoming segments remain intact. Equivalence only
    /// adds a zero-cost future edge used for restoration choice and cost
    /// relaxation.
    pub fn admit_future_equivalence_proof(
        &mut self,
        proof: FutureEquivalenceProof,
    ) -> Result<bool, StateGraphError> {
        proof.validate().map_err(StateGraphError::Invalid)?;
        if self.identity.future_equivalence_validator_sha256 == Digest::ZERO
            || proof.validator_sha256 != self.identity.future_equivalence_validator_sha256
        {
            return Err(StateGraphError::Invalid(
                "future-equivalence proof validator is not graph-authoritative",
            ));
        }
        let left = self.nodes.get(&proof.left).ok_or(StateGraphError::Invalid(
            "future-equivalence proof names an absent node",
        ))?;
        let right = self
            .nodes
            .get(&proof.right)
            .ok_or(StateGraphError::Invalid(
                "future-equivalence proof names an absent node",
            ))?;
        if !left.restoration.executable
            || !right.restoration.executable
            || left.terminal != right.terminal
        {
            return Err(StateGraphError::Invalid(
                "future-equivalence proof requires executable nodes with equal terminal truth",
            ));
        }
        match self.future_equivalence_proofs.get(&proof.proof_sha256) {
            Some(existing) if existing.as_ref() == &proof => return Ok(false),
            Some(_) => {
                return Err(StateGraphError::DigestCollision(
                    "future-equivalence proof identity names different evidence",
                ));
            }
            None => {}
        }
        let identity = proof.proof_sha256;
        self.future_equivalence_proofs
            .insert(identity, Arc::new(proof));
        if let Err(error) = self.validate() {
            self.future_equivalence_proofs.remove(&identity);
            return Err(error);
        }
        self.mark_proof_persistence_dirty(identity);
        Ok(true)
    }

    pub fn future_equivalence_proofs(&self) -> impl Iterator<Item = &FutureEquivalenceProof> {
        self.future_equivalence_proofs.values().map(Arc::as_ref)
    }

    pub fn equivalent_nodes(
        &self,
        source: ExactStateId,
    ) -> Result<BTreeSet<ExactStateId>, StateGraphError> {
        if !self.nodes.contains_key(&source) {
            return Err(StateGraphError::Invalid(
                "transposition source node is absent",
            ));
        }
        let mut equivalent = BTreeSet::from([source]);
        let mut frontier = VecDeque::from([source]);
        while let Some(node) = frontier.pop_front() {
            for proof in self.future_equivalence_proofs.values() {
                let peer = if proof.left == node {
                    Some(proof.right)
                } else if proof.right == node {
                    Some(proof.left)
                } else {
                    None
                };
                if let Some(peer) = peer
                    && equivalent.insert(peer)
                {
                    frontier.push_back(peer);
                }
            }
        }
        Ok(equivalent)
    }

    /// Fastest authenticated exact node in a proven equivalence class.
    pub fn canonical_restoration_node(
        &self,
        source: ExactStateId,
    ) -> Result<ExactStateId, StateGraphError> {
        self.equivalent_nodes(source)?
            .into_iter()
            .filter_map(|id| self.nodes.get(&id).map(|node| (node.root_ticks, id)))
            .min()
            .map(|(_, id)| id)
            .ok_or(StateGraphError::Invariant(
                "transposition class has no admitted node",
            ))
    }

    /// Shortest graph cost after adding zero-cost edges for validated future
    /// equivalence. Original route costs remain inspectable on every node.
    pub fn relaxed_root_ticks(&self) -> Result<BTreeMap<ExactStateId, u64>, StateGraphError> {
        let mut costs = BTreeMap::from([(self.root, 0_u64)]);
        let mut frontier = BinaryHeap::from([Reverse((0_u64, self.root))]);
        let mut equivalent = BTreeMap::<ExactStateId, Vec<ExactStateId>>::new();
        for proof in self.future_equivalence_proofs.values() {
            equivalent.entry(proof.left).or_default().push(proof.right);
            equivalent.entry(proof.right).or_default().push(proof.left);
        }

        while let Some(Reverse((source_ticks, source))) = frontier.pop() {
            if costs.get(&source).copied() != Some(source_ticks) {
                continue;
            }
            if let Some(peers) = equivalent.get(&source) {
                for peer in peers {
                    relax_cost(&mut costs, &mut frontier, *peer, source_ticks);
                }
            }
            let node = self.nodes.get(&source).ok_or(StateGraphError::Invariant(
                "relaxed graph frontier names an absent node",
            ))?;
            for segment_sha256 in &node.outgoing_segments {
                let segment =
                    self.segments
                        .get(segment_sha256)
                        .ok_or(StateGraphError::Invariant(
                            "relaxed graph node names an absent segment",
                        ))?;
                let segment_ticks = u64::from(
                    segment
                        .option_end_offset_ticks
                        .checked_sub(segment.option_start_offset_ticks)
                        .ok_or(StateGraphError::Invariant(
                            "observed segment offsets are reversed",
                        ))?,
                );
                let candidate = source_ticks
                    .checked_add(segment_ticks)
                    .ok_or(StateGraphError::Invariant("relaxed graph cost overflows"))?;
                relax_cost(&mut costs, &mut frontier, segment.target, candidate);
            }
        }
        if costs.len() != self.nodes.len() {
            return Err(StateGraphError::Invariant(
                "relaxed graph costs do not cover every node",
            ));
        }
        for (id, cost) in &costs {
            if *cost
                > self
                    .nodes
                    .get(id)
                    .ok_or(StateGraphError::Invariant(
                        "relaxed graph cost names an absent node",
                    ))?
                    .root_ticks
            {
                return Err(StateGraphError::Invariant(
                    "relaxed graph cost exceeds authenticated route cost",
                ));
            }
        }
        Ok(costs)
    }

    pub fn relaxed_root_ticks_to(&self, node: ExactStateId) -> Result<u64, StateGraphError> {
        self.relaxed_root_ticks()?
            .get(&node)
            .copied()
            .ok_or(StateGraphError::Invalid(
                "relaxed graph cost node is absent",
            ))
    }
}

fn relax_cost(
    costs: &mut BTreeMap<ExactStateId, u64>,
    frontier: &mut BinaryHeap<Reverse<(u64, ExactStateId)>>,
    target: ExactStateId,
    candidate: u64,
) {
    let improves = costs
        .get(&target)
        .is_none_or(|current| candidate < *current);
    if improves {
        costs.insert(target, candidate);
        frontier.push(Reverse((candidate, target)));
    }
}
