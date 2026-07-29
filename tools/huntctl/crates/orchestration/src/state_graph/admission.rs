use super::{
    ActionExpansion, ActionExpansionStatus, CompletedExpansionEvidence, ExactStateId,
    ExpansionAdmission, ExpansionEvidenceAuthority, NativeBoundaryLocator, ObservedSegment,
    RestorationLocator, RouteRecord, StateGraph, StateGraphError, StateGraphNode,
    action_expansion_identity, route_checkpoint_sha256, same_intermediate_boundary_realization,
    tape_prefix, tape_sha256,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_learning::fact_snapshot::FactSnapshot;
use dusklight_learning::option_transition::OptionTransitionSample;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

impl StateGraph {
    pub fn admit_completed_expansion(
        &mut self,
        transition: OptionTransitionSample,
        route: InputTape,
        episode_group: u64,
        authority: ExpansionEvidenceAuthority,
    ) -> Result<ExpansionAdmission, StateGraphError> {
        self.admit_completed_expansion_with_lease(transition, route, episode_group, authority, None)
    }

    pub fn admit_leased_completed_expansion(
        &mut self,
        transition: OptionTransitionSample,
        route: InputTape,
        episode_group: u64,
        authority: ExpansionEvidenceAuthority,
        lease_sha256: Digest,
    ) -> Result<ExpansionAdmission, StateGraphError> {
        if lease_sha256 == Digest::ZERO {
            return Err(StateGraphError::Invalid(
                "completed expansion lease is missing",
            ));
        }
        self.admit_completed_expansion_with_lease(
            transition,
            route,
            episode_group,
            authority,
            Some(lease_sha256),
        )
    }

    fn admit_completed_expansion_with_lease(
        &mut self,
        transition: OptionTransitionSample,
        route: InputTape,
        episode_group: u64,
        authority: ExpansionEvidenceAuthority,
        lease_sha256: Option<Digest>,
    ) -> Result<ExpansionAdmission, StateGraphError> {
        self.validate_transition(&transition, &route)?;
        let option_start = usize::try_from(transition.execution.realized_tape_range.start_frame)
            .map_err(|_| StateGraphError::Invalid("option start frame overflows"))?;
        let option_end =
            usize::try_from(transition.execution.realized_tape_range.end_frame_exclusive)
                .map_err(|_| StateGraphError::Invalid("option end frame overflows"))?;
        let source_route = tape_prefix(&route, option_start)?;
        let source = self.admit_node(
            transition.before.clone(),
            source_route,
            transition.before.terminal.reached == Some(true),
            None,
            authority == ExpansionEvidenceAuthority::Executable,
        )?;
        let expansion_sha256 =
            action_expansion_identity(source.id, &transition.value_sample.action)?;
        let evidence_sha256 = transition.replay_identity_sha256()?;
        if let Some(existing) = self.expansions.get_mut(&expansion_sha256)
            && let ActionExpansionStatus::Completed {
                authority: existing_authority,
                route_checkpoint_sha256: _,
                evidence,
            } = &mut existing.status
        {
            let (canonical_evidence_sha256, canonical) =
                evidence.iter().next().ok_or(StateGraphError::Invariant(
                    "completed expansion has no evidence",
                ))?;
            let differing_fields =
                native_realization_differences(&canonical.transition, &transition);
            if !differing_fields.is_empty() {
                return Err(StateGraphError::ConflictingNativeEvidence {
                    expansion_sha256,
                    canonical_evidence_sha256: *canonical_evidence_sha256,
                    conflicting_evidence_sha256: evidence_sha256,
                    differing_fields: differing_fields.join(","),
                });
            }
            let duplicate = evidence.contains_key(&evidence_sha256);
            if duplicate && authority == ExpansionEvidenceAuthority::Executable {
                evidence
                    .get_mut(&evidence_sha256)
                    .ok_or(StateGraphError::Invariant(
                        "duplicate expansion evidence disappeared",
                    ))?
                    .authority = ExpansionEvidenceAuthority::Executable;
            } else if !duplicate {
                evidence.insert(
                    evidence_sha256,
                    CompletedExpansionEvidence {
                        episode_group,
                        authority,
                        transition: Box::new(transition),
                    },
                );
            }
            let authority_promoted = authority == ExpansionEvidenceAuthority::Executable
                && *existing_authority != ExpansionEvidenceAuthority::Executable;
            let target = existing.target.ok_or(StateGraphError::Invariant(
                "completed expansion has no target",
            ))?;
            let source = existing.source;
            if authority_promoted {
                self.promote_expansion_authority(expansion_sha256)?;
            }
            return Ok(ExpansionAdmission {
                expansion_sha256,
                source,
                target,
                inserted_nodes: 0,
                inserted_segments: 0,
                duplicate,
                authority_promoted,
            });
        }
        match self.expansions.get(&expansion_sha256) {
            Some(ActionExpansion {
                status:
                    ActionExpansionStatus::Leased {
                        lease_sha256: active,
                        ..
                    },
                ..
            }) if Some(*active) == lease_sha256 => {}
            Some(ActionExpansion {
                status: ActionExpansionStatus::Untried | ActionExpansionStatus::Retryable { .. },
                ..
            }) if lease_sha256.is_none() => {}
            Some(ActionExpansion {
                status: ActionExpansionStatus::FailedValidation { .. },
                ..
            }) => {
                return Err(StateGraphError::Invalid(
                    "validation-failed expansion cannot complete without retry",
                ));
            }
            Some(_) => {
                return Err(StateGraphError::Invalid(
                    "completed evidence does not own the scheduled expansion",
                ));
            }
            None if lease_sha256.is_some() => {
                return Err(StateGraphError::Invalid(
                    "leased completion names an unregistered expansion",
                ));
            }
            None => {}
        }

        let mut boundaries = Vec::with_capacity(transition.intermediate_boundaries.len() + 2);
        boundaries.push((0_u32, source));
        let mut inserted_nodes = usize::from(source.inserted);
        for boundary in &transition.intermediate_boundaries {
            let end = option_start
                .checked_add(boundary.offset_ticks as usize)
                .ok_or(StateGraphError::Invalid(
                    "intermediate route prefix overflows",
                ))?;
            let admitted = self.admit_node(
                boundary.state.clone(),
                tape_prefix(&route, end)?,
                false,
                Some(NativeBoundaryLocator {
                    episode_shard_sha256: boundary.episode_shard_sha256,
                    option_offset_ticks: boundary.offset_ticks,
                }),
                authority == ExpansionEvidenceAuthority::Executable,
            )?;
            inserted_nodes += usize::from(admitted.inserted);
            boundaries.push((boundary.offset_ticks, admitted));
        }
        let target = self.admit_node(
            transition.after.clone(),
            tape_prefix(&route, option_end)?,
            transition.value_sample.terminal,
            None,
            authority == ExpansionEvidenceAuthority::Executable,
        )?;
        inserted_nodes += usize::from(target.inserted);
        boundaries.push((transition.execution.duration.realized_ticks, target));

        let mut segment_ids = Vec::with_capacity(boundaries.len().saturating_sub(1));
        for pair in boundaries.windows(2) {
            let (start_offset, source) = pair[0];
            let (end_offset, target) = pair[1];
            let identity_sha256 = segment_identity(
                expansion_sha256,
                source.id,
                target.id,
                start_offset,
                end_offset,
            );
            let segment = ObservedSegment {
                identity_sha256,
                parent_expansion_sha256: expansion_sha256,
                source: source.id,
                target: target.id,
                option_start_offset_ticks: start_offset,
                option_end_offset_ticks: end_offset,
            };
            if self.segments.insert(identity_sha256, segment).is_some() {
                return Err(StateGraphError::Invariant(
                    "new expansion produced a duplicate observed segment",
                ));
            }
            self.nodes
                .get_mut(&target.id)
                .ok_or(StateGraphError::Invariant("segment target is absent"))?
                .incoming_segments
                .insert(identity_sha256);
            self.nodes
                .get_mut(&source.id)
                .ok_or(StateGraphError::Invariant("segment source is absent"))?
                .outgoing_segments
                .insert(identity_sha256);
            segment_ids.push(identity_sha256);
        }

        self.nodes
            .get_mut(&source.id)
            .ok_or(StateGraphError::Invariant("expansion source is absent"))?
            .outgoing_expansions
            .insert(expansion_sha256);
        self.expansions.insert(
            expansion_sha256,
            ActionExpansion {
                identity_sha256: expansion_sha256,
                source: source.id,
                target: Some(target.id),
                action: transition.value_sample.action.clone(),
                execution: Some(transition.execution.clone()),
                observed_segments: segment_ids,
                status: ActionExpansionStatus::Completed {
                    authority,
                    route_checkpoint_sha256: target.id.route_checkpoint_sha256,
                    evidence: BTreeMap::from([(
                        evidence_sha256,
                        CompletedExpansionEvidence {
                            episode_group,
                            authority,
                            transition: Box::new(transition),
                        },
                    )]),
                },
            },
        );
        self.refresh_best_terminal();
        Ok(ExpansionAdmission {
            expansion_sha256,
            source: source.id,
            target: target.id,
            inserted_nodes,
            inserted_segments: boundaries.len().saturating_sub(1),
            duplicate: false,
            authority_promoted: false,
        })
    }

    fn validate_transition(
        &self,
        transition: &OptionTransitionSample,
        route: &InputTape,
    ) -> Result<(), StateGraphError> {
        transition.validate()?;
        route.validate()?;
        transition.execution.validate_against_tape(route)?;
        let start = usize::try_from(transition.execution.realized_tape_range.start_frame)
            .map_err(|_| StateGraphError::Invalid("option start frame overflows"))?;
        let end = usize::try_from(transition.execution.realized_tape_range.end_frame_exclusive)
            .map_err(|_| StateGraphError::Invalid("option end frame overflows"))?;
        if transition.execution_authority_sha256 != self.identity.execution_authority_sha256
            || transition.feature_schema_sha256 != self.identity.feature_schema_sha256
            || end > route.frames.len()
            || transition.source_checkpoint_sha256
                != route_checkpoint_sha256(
                    self.identity.root_checkpoint_sha256,
                    &tape_prefix(route, start)?,
                )?
            || transition.next_checkpoint_sha256
                != route_checkpoint_sha256(
                    self.identity.root_checkpoint_sha256,
                    &tape_prefix(route, end)?,
                )?
        {
            return Err(StateGraphError::Invalid(
                "completed expansion is detached from this graph",
            ));
        }
        Ok(())
    }

    fn admit_node(
        &mut self,
        state: FactSnapshot,
        route: InputTape,
        terminal: bool,
        native_boundary: Option<NativeBoundaryLocator>,
        executable: bool,
    ) -> Result<NodeAdmission, StateGraphError> {
        let state_sha256 = state.content_sha256()?;
        let route_checkpoint_sha256 =
            route_checkpoint_sha256(self.identity.root_checkpoint_sha256, &route)?;
        let id = ExactStateId {
            route_checkpoint_sha256,
            state_sha256,
        };
        let record = RouteRecord {
            route_checkpoint_sha256,
            tape_sha256: tape_sha256(&route)?,
            tape_frames: route.frames.len() as u64,
        };
        let root_ticks = record
            .tape_frames
            .checked_sub(self.root_route_frames)
            .ok_or(StateGraphError::Invalid(
                "node route precedes the graph root",
            ))?;
        match self.routes.get(&route_checkpoint_sha256) {
            Some(existing) if existing != &route => {
                return Err(StateGraphError::DigestCollision(
                    "route checkpoint names different tapes",
                ));
            }
            Some(_) => {}
            None => {
                self.routes.insert(route_checkpoint_sha256, route);
            }
        }
        let restoration = RestorationLocator {
            route: record,
            native_boundary,
            executable,
        };
        if let Some(existing) = self.nodes.get_mut(&id) {
            if existing.state != state
                || existing.terminal != terminal
                || existing.root_ticks != root_ticks
                || existing.restoration.route != restoration.route
            {
                return Err(StateGraphError::DigestCollision(
                    "exact state identity names conflicting node evidence",
                ));
            }
            if existing.restoration.native_boundary.is_none() {
                existing.restoration.native_boundary = restoration.native_boundary;
            }
            existing.restoration.executable |= restoration.executable;
            return Ok(NodeAdmission {
                id,
                inserted: false,
            });
        }
        self.nodes.insert(
            id,
            StateGraphNode {
                id,
                state,
                terminal,
                root_ticks,
                restoration,
                incoming_segments: BTreeSet::new(),
                outgoing_segments: BTreeSet::new(),
                outgoing_expansions: BTreeSet::new(),
            },
        );
        Ok(NodeAdmission { id, inserted: true })
    }

    fn promote_expansion_authority(
        &mut self,
        expansion_sha256: Digest,
    ) -> Result<bool, StateGraphError> {
        let expansion = self
            .expansions
            .get_mut(&expansion_sha256)
            .ok_or(StateGraphError::Invariant("promoted expansion is absent"))?;
        let ActionExpansionStatus::Completed {
            authority,
            evidence: _,
            route_checkpoint_sha256: _,
        } = &mut expansion.status
        else {
            return Err(StateGraphError::Invariant(
                "only completed evidence can be promoted",
            ));
        };
        if *authority == ExpansionEvidenceAuthority::Executable {
            return Ok(false);
        }
        *authority = ExpansionEvidenceAuthority::Executable;
        let mut nodes = vec![expansion.source];
        if let Some(target) = expansion.target {
            nodes.push(target);
        }
        for segment_sha256 in &expansion.observed_segments {
            let segment = self
                .segments
                .get(segment_sha256)
                .ok_or(StateGraphError::Invariant(
                    "promoted expansion segment is absent",
                ))?;
            nodes.push(segment.source);
            nodes.push(segment.target);
        }
        for node in nodes {
            self.nodes
                .get_mut(&node)
                .ok_or(StateGraphError::Invariant(
                    "promoted expansion node is absent",
                ))?
                .restoration
                .executable = true;
        }
        self.refresh_best_terminal();
        Ok(true)
    }
}

fn native_realization_differences(
    left: &OptionTransitionSample,
    right: &OptionTransitionSample,
) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if left.execution_authority_sha256 != right.execution_authority_sha256 {
        fields.push("execution_authority");
    }
    if left.before_state_sha256 != right.before_state_sha256 {
        fields.push("before_state_digest");
    }
    if left.after_state_sha256 != right.after_state_sha256 {
        fields.push("after_state_digest");
    }
    if left.source_checkpoint_sha256 != right.source_checkpoint_sha256 {
        fields.push("source_checkpoint");
    }
    if left.next_checkpoint_sha256 != right.next_checkpoint_sha256 {
        fields.push("next_checkpoint");
    }
    if left.before != right.before {
        fields.push("before_state");
    }
    if left.after != right.after {
        fields.push("after_state");
    }
    if left.execution != right.execution {
        fields.push("execution");
    }
    if left.value_sample.terminal != right.value_sample.terminal {
        fields.push("terminal");
    }
    if !same_intermediate_boundary_realization(
        &left.intermediate_boundaries,
        &right.intermediate_boundaries,
    ) {
        fields.push("intermediate_boundaries");
    }
    fields
}

#[derive(Clone, Copy)]
struct NodeAdmission {
    id: ExactStateId,
    inserted: bool,
}

fn segment_identity(
    expansion_sha256: Digest,
    source: ExactStateId,
    target: ExactStateId,
    start_offset: u32,
    end_offset: u32,
) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-observed-segment/v1");
    hasher.update(expansion_sha256.0);
    hasher.update(source.route_checkpoint_sha256.0);
    hasher.update(source.state_sha256.0);
    hasher.update(target.route_checkpoint_sha256.0);
    hasher.update(target.state_sha256.0);
    hasher.update(start_offset.to_le_bytes());
    hasher.update(end_offset.to_le_bytes());
    Digest(hasher.finalize().into())
}
