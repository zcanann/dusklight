use super::{
    ActionExpansion, ActionExpansionStatus, ExactStateId, StateGraph, StateGraphError,
    action_expansion_identity,
};
use dusklight_automation_contracts::artifact::Digest;
use dusklight_learning::option_values::OptionActionDescriptor;

impl StateGraph {
    pub fn register_action_expansion(
        &mut self,
        source: ExactStateId,
        action: OptionActionDescriptor,
    ) -> Result<Digest, StateGraphError> {
        let node = self
            .nodes
            .get(&source)
            .ok_or(StateGraphError::Invalid("expansion source is absent"))?;
        if node.terminal || !node.restoration.executable {
            return Err(StateGraphError::Invalid(
                "expansion source is terminal or learner-only",
            ));
        }
        let identity_sha256 = action_expansion_identity(source, &action)?;
        if let Some(existing) = self.expansions.get(&identity_sha256) {
            if existing.source != source || existing.action != action {
                return Err(StateGraphError::DigestCollision(
                    "action expansion identity names different work",
                ));
            }
            return Ok(identity_sha256);
        }
        self.expansions.insert(
            identity_sha256,
            ActionExpansion {
                identity_sha256,
                source,
                target: None,
                action,
                execution: None,
                observed_segments: Vec::new(),
                status: ActionExpansionStatus::Untried,
            },
        );
        self.nodes
            .get_mut(&source)
            .ok_or(StateGraphError::Invariant(
                "registered expansion source disappeared",
            ))?
            .outgoing_expansions
            .insert(identity_sha256);
        Ok(identity_sha256)
    }

    pub fn lease_action_expansion(
        &mut self,
        expansion_sha256: Digest,
        lease_sha256: Digest,
        current_generation: u64,
        expires_at_generation: u64,
    ) -> Result<(), StateGraphError> {
        if lease_sha256 == Digest::ZERO || expires_at_generation <= current_generation {
            return Err(StateGraphError::Invalid(
                "expansion lease identity or expiry is invalid",
            ));
        }
        let expansion = self
            .expansions
            .get_mut(&expansion_sha256)
            .ok_or(StateGraphError::Invalid("leased expansion is absent"))?;
        let available = match expansion.status {
            ActionExpansionStatus::Untried | ActionExpansionStatus::Retryable { .. } => true,
            ActionExpansionStatus::Leased {
                expires_at_generation,
                ..
            } => expires_at_generation <= current_generation,
            ActionExpansionStatus::Completed { .. }
            | ActionExpansionStatus::FailedValidation { .. } => false,
        };
        if !available {
            return Err(StateGraphError::Invalid(
                "expansion is not available for lease",
            ));
        }
        expansion.status = ActionExpansionStatus::Leased {
            lease_sha256,
            expires_at_generation,
        };
        Ok(())
    }

    pub fn mark_expansion_retryable(
        &mut self,
        expansion_sha256: Digest,
        lease_sha256: Digest,
        attempts: u32,
    ) -> Result<(), StateGraphError> {
        if attempts == 0 {
            return Err(StateGraphError::Invalid(
                "retryable expansion requires a positive attempt count",
            ));
        }
        let expansion = self.match_leased_expansion(expansion_sha256, lease_sha256)?;
        expansion.status = ActionExpansionStatus::Retryable { attempts };
        Ok(())
    }

    pub fn mark_expansion_failed_validation(
        &mut self,
        expansion_sha256: Digest,
        lease_sha256: Digest,
        evidence_sha256: Digest,
    ) -> Result<(), StateGraphError> {
        if evidence_sha256 == Digest::ZERO {
            return Err(StateGraphError::Invalid(
                "validation failure requires an evidence identity",
            ));
        }
        let expansion = self.match_leased_expansion(expansion_sha256, lease_sha256)?;
        expansion.status = ActionExpansionStatus::FailedValidation { evidence_sha256 };
        Ok(())
    }

    pub fn expansion_is_schedulable(
        &self,
        expansion_sha256: Digest,
        current_generation: u64,
    ) -> bool {
        self.expansions
            .get(&expansion_sha256)
            .is_some_and(|expansion| match expansion.status {
                ActionExpansionStatus::Untried | ActionExpansionStatus::Retryable { .. } => true,
                ActionExpansionStatus::Leased {
                    expires_at_generation,
                    ..
                } => expires_at_generation <= current_generation,
                ActionExpansionStatus::Completed { .. }
                | ActionExpansionStatus::FailedValidation { .. } => false,
            })
    }

    fn match_leased_expansion(
        &mut self,
        expansion_sha256: Digest,
        lease_sha256: Digest,
    ) -> Result<&mut ActionExpansion, StateGraphError> {
        let expansion = self
            .expansions
            .get_mut(&expansion_sha256)
            .ok_or(StateGraphError::Invalid("leased expansion is absent"))?;
        if !matches!(
            expansion.status,
            ActionExpansionStatus::Leased {
                lease_sha256: active,
                ..
            } if active == lease_sha256
        ) {
            return Err(StateGraphError::Invalid(
                "worker result does not own the expansion lease",
            ));
        }
        Ok(expansion)
    }
}
