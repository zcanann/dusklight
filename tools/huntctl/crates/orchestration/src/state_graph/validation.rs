use super::{
    ActionExpansionStatus, ExactStateId, StateGraph, StateGraphError, TerminalPath,
    action_expansion_identity, route_checkpoint_sha256, tape_sha256,
};
use dusklight_automation_contracts::artifact::Digest;
use std::collections::BTreeSet;

impl StateGraph {
    pub fn validate(&self) -> Result<(), StateGraphError> {
        if self.schema != super::STATE_GRAPH_SCHEMA_V1 {
            return Err(StateGraphError::Invalid(
                "state graph schema is unsupported",
            ));
        }
        self.identity.validate().map_err(StateGraphError::Invalid)?;
        let root = self
            .nodes
            .get(&self.root)
            .ok_or(StateGraphError::Invariant("root node is absent"))?;
        if root.root_ticks != 0
            || root.restoration.route.tape_frames != self.root_route_frames
            || root.id != self.root
        {
            return Err(StateGraphError::Invariant(
                "root node is detached from graph identity",
            ));
        }

        for (route_identity, route) in &self.routes {
            if *route_identity
                != route_checkpoint_sha256(self.identity.root_checkpoint_sha256, route)?
            {
                return Err(StateGraphError::Invariant(
                    "route store key is detached from canonical tape",
                ));
            }
        }
        for (identity, proof) in &self.future_equivalence_proofs {
            proof.validate().map_err(StateGraphError::Invalid)?;
            if *identity != proof.proof_sha256
                || !self
                    .nodes
                    .get(&proof.left)
                    .is_some_and(|node| node.restoration.executable)
                || !self
                    .nodes
                    .get(&proof.right)
                    .is_some_and(|node| node.restoration.executable)
                || self.nodes[&proof.left].terminal != self.nodes[&proof.right].terminal
            {
                return Err(StateGraphError::Invariant(
                    "future-equivalence proof is detached from executable exact nodes",
                ));
            }
        }
        for (id, node) in &self.nodes {
            node.state.validate()?;
            if node.id != *id
                || node.id.state_sha256 != node.state.content_sha256()?
                || node.terminal != (node.state.terminal.reached == Some(true))
                || node.restoration.route.route_checkpoint_sha256 != node.id.route_checkpoint_sha256
            {
                return Err(StateGraphError::Invariant(
                    "node identity is detached from its typed state",
                ));
            }
            let route = self.routes.get(&node.id.route_checkpoint_sha256).ok_or(
                StateGraphError::Invariant("node restoration route is absent"),
            )?;
            if node.restoration.route.tape_frames != route.frames.len() as u64
                || node.restoration.route.tape_sha256 != tape_sha256(route)?
                || node.state.tape_frame != route.frames.len() as u64
                || node.root_ticks
                    != (route.frames.len() as u64)
                        .checked_sub(self.root_route_frames)
                        .ok_or(StateGraphError::Invariant("node route precedes graph root"))?
            {
                return Err(StateGraphError::Invariant(
                    "node restoration evidence is detached",
                ));
            }
            if let Some(native) = &node.restoration.native_boundary
                && (native.episode_shard_sha256 == Digest::ZERO || native.option_offset_ticks == 0)
            {
                return Err(StateGraphError::Invariant(
                    "native restoration locator is invalid",
                ));
            }
            for segment in &node.incoming_segments {
                if self
                    .segments
                    .get(segment)
                    .is_none_or(|value| value.target != *id)
                {
                    return Err(StateGraphError::Invariant(
                        "node incoming segment is detached",
                    ));
                }
            }
            for segment in &node.outgoing_segments {
                if self
                    .segments
                    .get(segment)
                    .is_none_or(|value| value.source != *id)
                {
                    return Err(StateGraphError::Invariant(
                        "node outgoing segment is detached",
                    ));
                }
            }
            for expansion in &node.outgoing_expansions {
                if self
                    .expansions
                    .get(expansion)
                    .is_none_or(|value| value.source != *id)
                {
                    return Err(StateGraphError::Invariant(
                        "node outgoing expansion is detached",
                    ));
                }
            }
        }

        for (identity, segment) in &self.segments {
            if segment.identity_sha256 != *identity
                || segment.option_start_offset_ticks >= segment.option_end_offset_ticks
                || !self.nodes.contains_key(&segment.source)
                || !self.nodes.contains_key(&segment.target)
                || !self
                    .expansions
                    .contains_key(&segment.parent_expansion_sha256)
            {
                return Err(StateGraphError::Invariant("observed segment is detached"));
            }
        }
        for (identity, expansion) in &self.expansions {
            if expansion.identity_sha256 != *identity
                || action_expansion_identity(expansion.source, &expansion.action)? != *identity
                || !self.nodes.contains_key(&expansion.source)
            {
                return Err(StateGraphError::Invariant(
                    "action expansion identity is detached",
                ));
            }
            self.validate_expansion(expansion)?;
        }

        let expected_terminal = self
            .nodes
            .values()
            .filter(|node| node.terminal && node.restoration.executable)
            .min_by_key(|node| (node.root_ticks, node.id))
            .map(|node| TerminalPath {
                terminal: node.id,
                route_checkpoint_sha256: node.id.route_checkpoint_sha256,
                root_to_terminal_ticks: node.root_ticks,
            });
        if self.best_terminal != expected_terminal {
            return Err(StateGraphError::Invariant(
                "best terminal path is not derived from graph nodes",
            ));
        }
        self.relaxed_root_ticks()?;
        Ok(())
    }

    fn validate_expansion(
        &self,
        expansion: &super::ActionExpansion,
    ) -> Result<(), StateGraphError> {
        match &expansion.status {
            ActionExpansionStatus::Completed {
                authority,
                route_checkpoint_sha256,
                evidence,
            } => {
                let target = expansion.target.ok_or(StateGraphError::Invariant(
                    "completed expansion has no target",
                ))?;
                let first = evidence.values().next().ok_or(StateGraphError::Invariant(
                    "completed expansion has no evidence",
                ))?;
                let any_executable = evidence
                    .values()
                    .any(|row| row.authority == super::ExpansionEvidenceAuthority::Executable);
                for (evidence_sha256, row) in evidence {
                    row.transition.validate()?;
                    if row.transition.replay_identity_sha256()? != *evidence_sha256
                        || !same_native_realization(&first.transition, &row.transition)
                        || row.transition.before_state_sha256 != expansion.source.state_sha256
                        || row.transition.source_checkpoint_sha256
                            != expansion.source.route_checkpoint_sha256
                        || row.transition.after_state_sha256 != target.state_sha256
                        || row.transition.next_checkpoint_sha256 != target.route_checkpoint_sha256
                        || row.transition.value_sample.action != expansion.action
                        || expansion.execution.as_ref() != Some(&row.transition.execution)
                    {
                        return Err(StateGraphError::Invariant(
                            "completed evidence is detached from its action expansion",
                        ));
                    }
                }
                if *route_checkpoint_sha256 != target.route_checkpoint_sha256
                    || (*authority == super::ExpansionEvidenceAuthority::Executable)
                        != any_executable
                    || (*authority == super::ExpansionEvidenceAuthority::Executable
                        && (!self
                            .nodes
                            .get(&expansion.source)
                            .is_some_and(|node| node.restoration.executable)
                            || !self
                                .nodes
                                .get(&target)
                                .is_some_and(|node| node.restoration.executable)))
                {
                    return Err(StateGraphError::Invariant(
                        "completed action expansion is detached from native evidence",
                    ));
                }
                self.validate_segment_chain(
                    expansion.source,
                    target,
                    first.transition.execution.duration.realized_ticks,
                    expansion.identity_sha256,
                    &expansion.observed_segments,
                )
            }
            ActionExpansionStatus::Leased {
                lease_sha256,
                expires_at_generation,
            } if *lease_sha256 == Digest::ZERO || *expires_at_generation == 0 => Err(
                StateGraphError::Invariant("leased expansion has no lease identity"),
            ),
            ActionExpansionStatus::FailedValidation { evidence_sha256 }
                if *evidence_sha256 == Digest::ZERO =>
            {
                Err(StateGraphError::Invariant(
                    "failed expansion has no evidence identity",
                ))
            }
            ActionExpansionStatus::Retryable { attempts } if *attempts == 0 => Err(
                StateGraphError::Invariant("retryable expansion has no attempts"),
            ),
            _ if expansion.target.is_some()
                || expansion.execution.is_some()
                || !expansion.observed_segments.is_empty() =>
            {
                Err(StateGraphError::Invariant(
                    "unfinished expansion contains realized evidence",
                ))
            }
            _ => Ok(()),
        }
    }

    fn validate_segment_chain(
        &self,
        source: ExactStateId,
        target: ExactStateId,
        duration_ticks: u32,
        expansion_sha256: Digest,
        segment_ids: &[Digest],
    ) -> Result<(), StateGraphError> {
        if segment_ids.is_empty()
            || segment_ids.iter().copied().collect::<BTreeSet<_>>().len() != segment_ids.len()
        {
            return Err(StateGraphError::Invariant(
                "completed expansion has an empty or duplicate segment chain",
            ));
        }
        let mut expected_source = source;
        let mut expected_offset = 0_u32;
        for identity in segment_ids {
            let segment = self
                .segments
                .get(identity)
                .ok_or(StateGraphError::Invariant(
                    "completed expansion segment is absent",
                ))?;
            if segment.parent_expansion_sha256 != expansion_sha256
                || segment.source != expected_source
                || segment.option_start_offset_ticks != expected_offset
            {
                return Err(StateGraphError::Invariant(
                    "completed expansion segment chain is discontinuous",
                ));
            }
            expected_source = segment.target;
            expected_offset = segment.option_end_offset_ticks;
        }
        if expected_source != target || expected_offset != duration_ticks {
            return Err(StateGraphError::Invariant(
                "completed expansion segment chain misses its target",
            ));
        }
        Ok(())
    }
}

fn same_native_realization(
    left: &dusklight_learning::option_transition::OptionTransitionSample,
    right: &dusklight_learning::option_transition::OptionTransitionSample,
) -> bool {
    left.execution_authority_sha256 == right.execution_authority_sha256
        && left.before_state_sha256 == right.before_state_sha256
        && left.after_state_sha256 == right.after_state_sha256
        && left.source_checkpoint_sha256 == right.source_checkpoint_sha256
        && left.next_checkpoint_sha256 == right.next_checkpoint_sha256
        && left.before == right.before
        && left.after == right.after
        && left.execution == right.execution
        && left.value_sample.terminal == right.value_sample.terminal
        && left.intermediate_boundaries == right.intermediate_boundaries
}
