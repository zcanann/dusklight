use super::{
    ExactStateId, GRAPH_RESTORATION_PLAN_SCHEMA_V1, GRAPH_RESTORATION_PLAN_SCHEMA_V2,
    GraphRestorationPlan, RestoredStateReceipt, StateGraph, StateGraphError, StateGraphIdentity,
    StateGraphNode,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_automation_contracts::tape::InputTape;
use dusklight_learning::fact_snapshot::FactSnapshot;
use sha2::{Digest as _, Sha256};

impl StateGraph {
    pub fn restoration_plan(
        &self,
        node: ExactStateId,
    ) -> Result<GraphRestorationPlan, StateGraphError> {
        let admitted = self
            .nodes
            .get(&node)
            .ok_or(StateGraphError::Invalid("restoration node is absent"))?;
        if !admitted.restoration.executable {
            return Err(StateGraphError::Invalid(
                "restoration node is not executable",
            ));
        }
        let dispatch_graph_sha256 =
            restoration_node_authority_sha256(&self.identity, admitted.as_ref());
        let plan_sha256 = restoration_plan_sha256(
            GRAPH_RESTORATION_PLAN_SCHEMA_V2,
            dispatch_graph_sha256,
            node,
            admitted.id.state_sha256,
            admitted.restoration.route.route_checkpoint_sha256,
            admitted.restoration.route.tape_sha256,
            admitted.restoration.route.tape_frames,
            admitted
                .restoration
                .native_boundary
                .as_ref()
                .map(|boundary| (boundary.evidence_sha256, boundary.option_offset_ticks)),
        );
        Ok(GraphRestorationPlan {
            schema: GRAPH_RESTORATION_PLAN_SCHEMA_V2.into(),
            dispatch_graph_sha256,
            node,
            expected_state_sha256: admitted.id.state_sha256,
            route: admitted.restoration.route.clone(),
            native_boundary: admitted.restoration.native_boundary.clone(),
            plan_sha256,
        })
    }

    pub fn restoration_route(
        &self,
        plan: &GraphRestorationPlan,
    ) -> Result<&InputTape, StateGraphError> {
        self.validate_restoration_plan(plan)?;
        self.routes
            .get(&plan.route.route_checkpoint_sha256)
            .map(AsRef::as_ref)
            .ok_or(StateGraphError::Invariant(
                "restoration plan route is absent",
            ))
    }

    /// Validate the complete typed boundary before its selected expansion is
    /// admitted as executable native evidence.
    pub fn validate_restored_state(
        &self,
        plan: &GraphRestorationPlan,
        observed: &FactSnapshot,
    ) -> Result<RestoredStateReceipt, StateGraphError> {
        self.validate_restoration_plan(plan)?;
        observed.validate()?;
        let observed_state_sha256 = observed.content_sha256()?;
        let expected = self
            .nodes
            .get(&plan.node)
            .ok_or(StateGraphError::Invariant(
                "restoration plan node disappeared",
            ))?;
        if observed_state_sha256 != plan.expected_state_sha256
            || observed != expected.state.as_ref()
        {
            return Err(StateGraphError::Invalid(
                "restored typed state does not match its exact graph node",
            ));
        }
        Ok(RestoredStateReceipt {
            restoration_plan_sha256: plan.plan_sha256,
            node: plan.node,
            observed_state_sha256,
            route_checkpoint_sha256: plan.route.route_checkpoint_sha256,
        })
    }

    pub(crate) fn validate_prehashed_restored_state(
        &self,
        plan: &GraphRestorationPlan,
        observed: &FactSnapshot,
        observed_state_sha256: Digest,
    ) -> Result<RestoredStateReceipt, StateGraphError> {
        self.validate_restoration_plan(plan)?;
        let expected = self
            .nodes
            .get(&plan.node)
            .ok_or(StateGraphError::Invariant(
                "restoration plan node disappeared",
            ))?;
        if observed_state_sha256 != plan.expected_state_sha256
            || observed_state_sha256 != expected.id.state_sha256
            || observed != expected.state.as_ref()
        {
            return Err(StateGraphError::Invalid(
                "prehashed restored state does not match its exact graph node",
            ));
        }
        Ok(RestoredStateReceipt {
            restoration_plan_sha256: plan.plan_sha256,
            node: plan.node,
            observed_state_sha256,
            route_checkpoint_sha256: plan.route.route_checkpoint_sha256,
        })
    }

    fn validate_restoration_plan(
        &self,
        plan: &GraphRestorationPlan,
    ) -> Result<(), StateGraphError> {
        let node = self
            .nodes
            .get(&plan.node)
            .ok_or(StateGraphError::Invalid("restoration plan node is absent"))?;
        let expected_dispatch_sha256 = match plan.schema.as_str() {
            GRAPH_RESTORATION_PLAN_SCHEMA_V1 => plan.dispatch_graph_sha256,
            GRAPH_RESTORATION_PLAN_SCHEMA_V2 => {
                restoration_node_authority_sha256(&self.identity, node.as_ref())
            }
            _ => {
                return Err(StateGraphError::Invalid(
                    "restoration plan schema is unsupported",
                ));
            }
        };
        let expected_plan_sha256 = restoration_plan_sha256(
            &plan.schema,
            plan.dispatch_graph_sha256,
            plan.node,
            plan.expected_state_sha256,
            plan.route.route_checkpoint_sha256,
            plan.route.tape_sha256,
            plan.route.tape_frames,
            plan.native_boundary
                .as_ref()
                .map(|boundary| (boundary.evidence_sha256, boundary.option_offset_ticks)),
        );
        if plan.dispatch_graph_sha256 == Digest::ZERO
            || plan.dispatch_graph_sha256 != expected_dispatch_sha256
            || plan.plan_sha256 != expected_plan_sha256
            || !node.restoration.executable
            || node.id.state_sha256 != plan.expected_state_sha256
            || node.restoration.route != plan.route
            || node.restoration.native_boundary != plan.native_boundary
        {
            return Err(StateGraphError::Invalid(
                "restoration plan is detached from its exact graph node",
            ));
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn restoration_plan_sha256(
    schema: &str,
    dispatch_graph_sha256: Digest,
    node: ExactStateId,
    expected_state_sha256: Digest,
    route_checkpoint_sha256: Digest,
    tape_sha256: Digest,
    tape_frames: u64,
    native_boundary: Option<(Digest, u32)>,
) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(schema.as_bytes());
    hasher.update(dispatch_graph_sha256.0);
    hasher.update(node.route_checkpoint_sha256.0);
    hasher.update(node.state_sha256.0);
    hasher.update(expected_state_sha256.0);
    hasher.update(route_checkpoint_sha256.0);
    hasher.update(tape_sha256.0);
    hasher.update(tape_frames.to_le_bytes());
    match native_boundary {
        Some((episode, offset)) => {
            hasher.update([1]);
            hasher.update(episode.0);
            hasher.update(offset.to_le_bytes());
        }
        None => hasher.update([0]),
    }
    Digest(hasher.finalize().into())
}

fn restoration_node_authority_sha256(
    identity: &StateGraphIdentity,
    node: &StateGraphNode,
) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight-graph-restoration-node-authority/v1");
    hasher.update(identity.execution_authority_sha256.0);
    hasher.update(identity.future_equivalence_validator_sha256.0);
    hasher.update(identity.feature_schema_sha256.0);
    hasher.update(identity.objective_sha256.0);
    hasher.update(identity.root_checkpoint_sha256.0);
    hasher.update(node.id.route_checkpoint_sha256.0);
    hasher.update(node.id.state_sha256.0);
    hasher.update([u8::from(node.terminal)]);
    hasher.update(node.root_ticks.to_le_bytes());
    hasher.update(node.restoration.route.route_checkpoint_sha256.0);
    hasher.update(node.restoration.route.tape_sha256.0);
    hasher.update(node.restoration.route.tape_frames.to_le_bytes());
    match &node.restoration.native_boundary {
        Some(boundary) => {
            hasher.update([1]);
            hasher.update(boundary.evidence_sha256.0);
            hasher.update(boundary.option_offset_ticks.to_le_bytes());
        }
        None => hasher.update([0]),
    }
    hasher.update([u8::from(node.restoration.executable)]);
    Digest(hasher.finalize().into())
}
