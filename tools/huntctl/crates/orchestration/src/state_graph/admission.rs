use super::{
    ActionExpansion, ActionExpansionStatus, ExactStateId, ExpansionAdmission,
    ExpansionEvidenceAuthority, NativeBoundaryLocator, ObservedSegment, RestorationLocator,
    RouteRecord, StateGraph, StateGraphError, StateGraphNode, route_checkpoint_sha256, tape_prefix,
    tape_sha256,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_learning::fact_snapshot::FactSnapshot;
use dusklight_learning::option_transition::OptionTransitionSample;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

impl StateGraph {
    pub fn admit_completed_expansion(
        &mut self,
        transition: OptionTransitionSample,
        route: InputTape,
        episode_group: u64,
        authority: ExpansionEvidenceAuthority,
    ) -> Result<ExpansionAdmission, StateGraphError> {
        self.validate_transition(&transition, &route)?;
        let expansion_sha256 = transition.replay_identity_sha256()?;
        if self.expansions.contains_key(&expansion_sha256) {
            let authority_promoted = authority == ExpansionEvidenceAuthority::Executable
                && self.promote_expansion_authority(expansion_sha256)?;
            let existing =
                self.expansions
                    .get(&expansion_sha256)
                    .ok_or(StateGraphError::Invariant(
                        "duplicate expansion disappeared during promotion",
                    ))?;
            let target = existing.target.ok_or(StateGraphError::Invariant(
                "completed expansion has no target",
            ))?;
            return Ok(ExpansionAdmission {
                expansion_sha256,
                source: existing.source,
                target,
                inserted_nodes: 0,
                inserted_segments: 0,
                duplicate: true,
                authority_promoted,
            });
        }

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
                    episode_group,
                    authority,
                    route_checkpoint_sha256: target.id.route_checkpoint_sha256,
                    transition: Box::new(transition),
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
            transition: _,
            episode_group: _,
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
