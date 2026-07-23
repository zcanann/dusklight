//! Exact GZ2E01 unsaved-file-0 goal and optional hypothetical escape overlay.

use crate::PlannerContractError;
use crate::identity::{ContentIdentity, ContextSelector, ExactContext, RuntimeConfiguration};
use crate::logic::{
    ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, PredicateExpression,
    RuleEvidence, TruthStatus, ValueReference,
};
use crate::refinement::{
    REFINEMENT_PACK_SCHEMA, RefinementOperation, RefinementPack, RefinementPackManifest,
    RefinementRule,
};
use crate::return_place::{GZ2E01_CONTENT_SHA256, GZ2E01_EN_RUNTIME_SHA256};
use crate::state::{ExecutionContext, StateValue};
pub use crate::title_boundary::GZ2E01_UNSAVED_FILE_ZERO_GOAL_ID;
use crate::transition::{RouteCost, StateOperation, Technique};
use std::collections::BTreeMap;

pub const GZ2E01_HYPOTHETICAL_FILE_ZERO_ESCAPE_PACK_ID: &str =
    "theorycraft.gz2e01.file-zero-world-escape";

/// Returns an opt-in theorycraft pack that resumes the last retained world
/// context without changing, saving, or reattaching the active title-origin
/// runtime file. It is intentionally not part of the exact mechanics catalog.
pub fn gz2e01_hypothetical_file_zero_escape_pack(
    content: &ContentIdentity,
    runtime: &RuntimeConfiguration,
) -> Result<RefinementPack, PlannerContractError> {
    require_exact_context(content, runtime)?;
    let scope = exact_scope();
    let evidence = RuleEvidence {
        truth: TruthStatus::Hypothetical,
        records: vec![EvidenceRecord {
            id: "theorycraft.gz2e01.file-zero-world-escape".into(),
            kind: EvidenceKind::Theorycraft,
            source_sha256: None,
            note: "Opt-in research assumption: leave the title/name process for its retained world without ending, saving, loading, or reattaching the title-origin runtime file.".into(),
        }],
    };
    let pack = RefinementPack {
        schema: REFINEMENT_PACK_SCHEMA.into(),
        manifest: RefinementPackManifest {
            id: GZ2E01_HYPOTHETICAL_FILE_ZERO_ESCAPE_PACK_ID.into(),
            version: "1.0.0".into(),
            author: "Dusklight route research".into(),
            source: "Explicit hypothetical file-0 escape overlay".into(),
            scope: scope.clone(),
            precedence: 0,
            dependencies: Vec::new(),
            conflicts: Vec::new(),
        },
        rules: vec![RefinementRule {
            id: "theorycraft.gz2e01.file-zero-world-escape.add-technique".into(),
            label: "Add hypothetical file-0 world escape".into(),
            operation: RefinementOperation::AddTechnique {
                technique: Technique {
                    id: "technique.hypothetical.gz2e01.file-zero-world-escape".into(),
                    label: "Hypothetically resume the retained world on file 0".into(),
                    scope,
                    prerequisites: PredicateExpression::All {
                        terms: vec![
                            compare(
                                ValueReference::ActiveRuntimeFileOrigin,
                                StateValue::Text("title_file_0".into()),
                            ),
                            compare(
                                ValueReference::WorldExecutionActive,
                                StateValue::Boolean(false),
                            ),
                        ],
                    },
                    operations: vec![StateOperation::SetExecutionContext {
                        context: ExecutionContext::World,
                    }],
                    discharged_obligation_ids: Vec::new(),
                    introduced_obligation_ids: Vec::new(),
                    cost: RouteCost {
                        axes: BTreeMap::from([("theorycraft".into(), 1)]),
                    },
                    evidence: evidence.clone(),
                },
            },
            evidence,
        }],
    };
    pack.validate()?;
    Ok(pack)
}

fn require_exact_context(
    content: &ContentIdentity,
    runtime: &RuntimeConfiguration,
) -> Result<(), PlannerContractError> {
    content.validate()?;
    runtime.validate()?;
    let content_sha256 = content.digest()?;
    let runtime_sha256 = runtime.digest()?;
    if content_sha256 != GZ2E01_CONTENT_SHA256
        || runtime_sha256 != GZ2E01_EN_RUNTIME_SHA256
        || runtime.content_sha256 != content_sha256
    {
        return Err(PlannerContractError::new(
            "file_zero_research.identity",
            "requires the exact GZ2E01/English context",
        ));
    }
    Ok(())
}

fn exact_scope() -> ContextScope {
    ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: ExactContext {
                content_sha256: GZ2E01_CONTENT_SHA256,
                runtime_configuration_sha256: GZ2E01_EN_RUNTIME_SHA256,
            },
        }],
    }
}

fn compare(left: ValueReference, value: StateValue) -> PredicateExpression {
    PredicateExpression::Compare {
        left,
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal { value },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{
        CONTENT_IDENTITY_SCHEMA, ContentFingerprint, GamePlatform, GameRegion,
        RUNTIME_CONFIGURATION_SCHEMA,
    };
    use crate::refinement::RefinementOperation;
    use crate::title_boundary::gz2e01_reset_to_opening_mechanics;

    fn context() -> (ContentIdentity, RuntimeConfiguration) {
        let content = ContentIdentity {
            schema: CONTENT_IDENTITY_SCHEMA.into(),
            id: "gcn-us-1.0-gz2e01".into(),
            fingerprint: ContentFingerprint {
                platform: GamePlatform::GameCube,
                region: GameRegion::Usa,
                revision: "1.0".into(),
                product_id: "GZ2E01".into(),
                executable_sha256:
                    "e7f197436815e66c4a11df3d7bd557d66083b641ff8a8e76439f3caba7ae60e8"
                        .parse()
                        .unwrap(),
                game_data_sha256:
                    "0bc3bb229279d4b8a8c7cbe962b0bffdfecd35ff21e2d6761ad42e90a070f772"
                        .parse()
                        .unwrap(),
                resource_manifest_sha256:
                    "2ab36f6c1d9d551c1397e1cf59e13288d2684c973cb7bd0ad6878f5a3b3a2ab1"
                        .parse()
                        .unwrap(),
            },
        };
        let runtime = RuntimeConfiguration {
            schema: RUNTIME_CONFIGURATION_SCHEMA.into(),
            content_sha256: GZ2E01_CONTENT_SHA256,
            language: "en".into(),
            settings: BTreeMap::new(),
        };
        (content, runtime)
    }

    #[test]
    fn exact_catalog_exposes_unsaved_file_zero_world_goal_without_an_escape() {
        let (content, runtime) = context();
        let mechanics = gz2e01_reset_to_opening_mechanics(&content, &runtime).unwrap();
        let goal = mechanics
            .goals
            .iter()
            .find(|goal| goal.id == GZ2E01_UNSAVED_FILE_ZERO_GOAL_ID)
            .unwrap();
        assert!(matches!(goal.predicate, PredicateExpression::All { .. }));
        assert!(mechanics.techniques.is_empty());
    }

    #[test]
    fn overlay_changes_only_execution_context_and_remains_hypothetical() {
        let (content, runtime) = context();
        let pack = gz2e01_hypothetical_file_zero_escape_pack(&content, &runtime).unwrap();
        assert_eq!(pack.rules.len(), 1);
        let RefinementOperation::AddTechnique { technique } = &pack.rules[0].operation else {
            panic!("expected technique overlay");
        };
        assert_eq!(technique.evidence.truth, TruthStatus::Hypothetical);
        assert_eq!(
            technique.operations,
            vec![StateOperation::SetExecutionContext {
                context: ExecutionContext::World,
            }]
        );
        assert!(technique.operations.iter().all(|operation| !matches!(
            operation,
            StateOperation::SetActiveRuntimeFile { .. }
                | StateOperation::SaveRuntimeToSlot { .. }
                | StateOperation::SaveActiveRuntimeToSlot { .. }
                | StateOperation::LoadRuntimeFromSlot { .. }
                | StateOperation::LoadActiveRuntimeFromSlot { .. }
                | StateOperation::BeginRuntimeFileLifetime { .. }
        )));
    }

    #[test]
    fn overlay_rejects_other_content() {
        let (mut content, runtime) = context();
        content.id = "gcn-us-1.0-gz2e01-mutant".into();
        assert!(gz2e01_hypothetical_file_zero_escape_pack(&content, &runtime).is_err());
    }
}
