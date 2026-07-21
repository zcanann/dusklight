//! Source-backed return-place writers and the ordinary savewarp reader.

use crate::artifact::Digest;
use crate::identity::{ContentIdentity, ContextSelector, ExactContext, RuntimeConfiguration};
use crate::logic::{
    ComparisonOperator, ContextScope, EvidenceKind, EvidenceRecord, PredicateExpression,
    RuleEvidence, TruthStatus, ValueReference,
};
use crate::state::{ComponentBindingReference, ComponentKind, StateValue};
use crate::transition::{
    ActivationContract, CandidateTransition, GateRule, MechanicsCatalog, ReaderRule,
    StateOperation, TransitionKind, WriterRule, MECHANICS_CATALOG_SCHEMA,
};
use crate::PlannerContractError;
use std::collections::BTreeMap;

pub const GZ2E01_CONTENT_SHA256: Digest = Digest([
    0x6f, 0xc8, 0xc6, 0xf4, 0xc4, 0xdc, 0xd1, 0x67, 0x1c, 0x03, 0x76, 0x46, 0xb2, 0x66,
    0x0a, 0xa4, 0xa0, 0xe5, 0x60, 0x2d, 0x4b, 0xf6, 0x6a, 0xa6, 0xe1, 0x09, 0xab, 0xa5,
    0xf2, 0x0a, 0x4a, 0xaa,
]);
pub const GZ2E01_EN_RUNTIME_SHA256: Digest = Digest([
    0x86, 0x2f, 0x97, 0x75, 0x4d, 0x99, 0xc8, 0xfe, 0x8a, 0xbe, 0x9e, 0xbe, 0x5d, 0x94,
    0xf5, 0x5a, 0xad, 0xaa, 0x9d, 0x95, 0x93, 0xf8, 0xec, 0x64, 0x08, 0xde, 0x60, 0x88,
    0x0b, 0x75, 0x2a, 0x32,
]);

const RETURN_PLACE_COMPONENT: &str = "return-place";
const TOWER_SAVMEM_EXECUTION_COMPONENT: &str = "actor.r_sp107.r03.savmem.1.execution";
const PERSISTENT_EVENT_STORE: &str = "persistent-event-registers";

/// Compiles the exact conditional R_SP107 room-3 `Savmem` writer plus the
/// generic retail savewarp reader. Actor execution remains an input fact: a
/// missing/corrupted actor makes writer activation unknown instead of silently
/// preserving or overwriting the incoming value.
pub fn gz2e01_tower_return_place_mechanics(
    content: &ContentIdentity,
    runtime: &RuntimeConfiguration,
) -> Result<MechanicsCatalog, PlannerContractError> {
    content.validate()?;
    runtime.validate()?;
    let content_sha256 = content.digest()?;
    let runtime_sha256 = runtime.digest()?;
    if content_sha256 != GZ2E01_CONTENT_SHA256
        || runtime_sha256 != GZ2E01_EN_RUNTIME_SHA256
        || runtime.content_sha256 != content_sha256
    {
        return Err(PlannerContractError::new(
            "return_place.identity",
            "requires the exact GZ2E01/English context",
        ));
    }

    let scope = ContextScope {
        selectors: vec![ContextSelector::Exact {
            context: ExactContext {
                content_sha256,
                runtime_configuration_sha256: runtime_sha256,
            },
        }],
    };
    let placement = EvidenceRecord {
        id: "resource.r_sp107.r03.savmem.1".into(),
        kind: EvidenceKind::Extracted,
        source_sha256: Some(parse_digest(
            "ff9ac474c6c282be807b78c289a31a9358f752033b586ecc6888f497c0647370",
        )),
        note: "Exact room.dzr placement: unlayered Savmem record 1, parameters 0x0000ff01, angles [45,47,-1], home room 3.".into(),
    };
    let actor_source = EvidenceRecord {
        id: "source.kytag14.return-place-writer".into(),
        kind: EvidenceKind::SourceAudited,
        source_sha256: Some(parse_digest(
            "57744385e319f4f6df99298ce4ebeeb48b67558e557dd8dc0d56af35b22d9283",
        )),
        note: "Savmem decodes placement guards and atomically writes stage, room, and player-status/spawn when eligible.".into(),
    };
    let savewarp_source = EvidenceRecord {
        id: "source.game-start.return-place-reader".into(),
        kind: EvidenceKind::SourceAudited,
        source_sha256: Some(parse_digest(
            "b9b37aed0b76eef2d27b35a2ece6ee077086a970f98d18936a83649303f15761",
        )),
        note: "dComIfGs_gameStart passes the stored return-place stage, player status, and room to setNextStage.".into(),
    };
    let writer_evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![placement, actor_source.clone()],
    };
    let source_evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![actor_source],
    };
    let reader_evidence = RuleEvidence {
        truth: TruthStatus::Established,
        records: vec![savewarp_source],
    };

    let active_runtime = ComponentBindingReference::ActiveRuntimeFile;
    let compare = |left: ValueReference, value: StateValue| PredicateExpression::Compare {
        left,
        operator: ComparisonOperator::Equal,
        right: ValueReference::Literal { value },
    };
    let writer = WriterRule {
        id: "writer.r_sp107.r03.savmem.1.return-place".into(),
        scope: scope.clone(),
        activation: PredicateExpression::All {
            terms: vec![
                compare(
                    ValueReference::LocationStage,
                    StateValue::Text("R_SP107".into()),
                ),
                compare(ValueReference::LocationRoom, StateValue::Signed(3)),
                compare(
                    ValueReference::ComponentField {
                        component_id: TOWER_SAVMEM_EXECUTION_COMPONENT.into(),
                        field: "executing".into(),
                    },
                    StateValue::Boolean(true),
                ),
                compare(
                    ValueReference::BoundRawBits {
                        component_kind: ComponentKind::Custom {
                            id: PERSISTENT_EVENT_STORE.into(),
                        },
                        binding: active_runtime.clone(),
                        byte_offset: 5,
                        byte_width: 1,
                        mask: 0x08,
                    },
                    StateValue::Unsigned(0x08),
                ),
                compare(
                    ValueReference::BoundRawBits {
                        component_kind: ComponentKind::Custom {
                            id: PERSISTENT_EVENT_STORE.into(),
                        },
                        binding: active_runtime.clone(),
                        byte_offset: 5,
                        byte_width: 1,
                        mask: 0x02,
                    },
                    StateValue::Unsigned(0),
                ),
            ],
        },
        operation: StateOperation::WriteFields {
            component_id: RETURN_PLACE_COMPONENT.into(),
            fields: BTreeMap::from([
                ("player_status".into(), StateValue::Unsigned(1)),
                ("room".into(), StateValue::Signed(3)),
                ("stage".into(), StateValue::Text("R_SP107".into())),
            ]),
        },
        evidence: writer_evidence,
    };
    let gate = GateRule {
        id: "gate.temporary.no-telop".into(),
        scope: scope.clone(),
        active_when: compare(
            ValueReference::BoundRawBits {
                component_kind: ComponentKind::TemporaryFlags,
                binding: active_runtime,
                byte_offset: 0x13,
                byte_width: 1,
                mask: 0x01,
            },
            StateValue::Unsigned(1),
        ),
        blocked_writer_ids: vec![writer.id.clone()],
        lifetime: crate::state::SemanticLifetime::RuntimeFile,
        evidence: source_evidence,
    };
    let savewarp = CandidateTransition {
        id: "transition.savewarp.from-player-return-place".into(),
        label: "Savewarp to the stored player return place".into(),
        scope: scope.clone(),
        transition_kind: TransitionKind::SaveWarp,
        approach_id: "savewarp.player-return-place".into(),
        activation: ActivationContract {
            hard_guards: PredicateExpression::True,
            physical_obligation_ids: Vec::new(),
            effects: vec![StateOperation::SetLocationFromFields {
                component_id: RETURN_PLACE_COMPONENT.into(),
                stage_field: "stage".into(),
                room_field: "room".into(),
                spawn_field: "player_status".into(),
                layer: -1,
            }],
            unknown_requirements: Vec::new(),
        },
        evidence: reader_evidence.clone(),
    };
    let readers = ["player_status", "room", "stage"]
        .into_iter()
        .map(|field| ReaderRule {
            id: format!("reader.savewarp.return-place.{field}"),
            scope: scope.clone(),
            source: ValueReference::ComponentField {
                component_id: RETURN_PLACE_COMPONENT.into(),
                field: field.into(),
            },
            consuming_transition_id: savewarp.id.clone(),
            interpretation_fact_id: None,
            evidence: reader_evidence.clone(),
        })
        .collect();
    let catalog = MechanicsCatalog {
        schema: MECHANICS_CATALOG_SCHEMA.into(),
        transitions: vec![savewarp],
        obligations: Vec::new(),
        writers: vec![writer],
        gates: vec![gate],
        readers,
        reconstruction_rules: Vec::new(),
        obstructions: Vec::new(),
        resolvers: Vec::new(),
        techniques: Vec::new(),
        microtraces: Vec::new(),
        goals: Vec::new(),
    };
    catalog.validate()?;
    Ok(catalog)
}

fn parse_digest(value: &str) -> Digest {
    value.parse().expect("compile-time SHA-256 literal")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{
        CONTENT_IDENTITY_SCHEMA, ContentFingerprint, GamePlatform, GameRegion,
        RUNTIME_CONFIGURATION_SCHEMA,
    };

    fn context() -> (ContentIdentity, RuntimeConfiguration) {
        let content = ContentIdentity {
            schema: CONTENT_IDENTITY_SCHEMA.into(),
            id: "gcn-us-1.0-gz2e01".into(),
            fingerprint: ContentFingerprint {
                platform: GamePlatform::GameCube,
                region: GameRegion::Usa,
                revision: "1.0".into(),
                product_id: "GZ2E01".into(),
                executable_sha256: parse_digest(
                    "e7f197436815e66c4a11df3d7bd557d66083b641ff8a8e76439f3caba7ae60e8",
                ),
                game_data_sha256: parse_digest(
                    "0bc3bb229279d4b8a8c7cbe962b0bffdfecd35ff21e2d6761ad42e90a070f772",
                ),
                resource_manifest_sha256: parse_digest(
                    "2ab36f6c1d9d551c1397e1cf59e13288d2684c973cb7bd0ad6878f5a3b3a2ab1",
                ),
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
    fn tower_writer_uses_raw_guards_atomic_record_write_and_dynamic_savewarp() {
        let (content, runtime) = context();
        let catalog = gz2e01_tower_return_place_mechanics(&content, &runtime).unwrap();
        let writer = &catalog.writers[0];
        let StateOperation::WriteFields {
            component_id,
            fields,
        } = &writer.operation
        else {
            panic!("Savmem must update one atomic record")
        };
        assert_eq!(component_id, RETURN_PLACE_COMPONENT);
        assert_eq!(fields.len(), 3);
        assert_eq!(
            catalog.gates[0].blocked_writer_ids,
            std::slice::from_ref(&writer.id)
        );
        assert!(matches!(
            catalog.transitions[0].activation.effects.as_slice(),
            [StateOperation::SetLocationFromFields { .. }]
        ));
        assert_eq!(catalog.readers.len(), 3);
    }

    #[test]
    fn wrong_exact_context_is_rejected() {
        let (content, mut runtime) = context();
        runtime.language = "fr".into();
        assert_eq!(
            gz2e01_tower_return_place_mechanics(&content, &runtime)
                .unwrap_err()
                .field(),
            "return_place.identity"
        );
    }
}
