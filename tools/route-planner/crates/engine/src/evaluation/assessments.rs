use super::*;

impl<'a> PredicateEvaluator<'a> {
    pub fn assess_transition(
        &self,
        transition: &CandidateTransition,
        discharged_obligation_ids: &BTreeSet<String>,
        unknown_obligation_ids: &BTreeSet<String>,
        mode: FeasibilityMode,
    ) -> TransitionAssessment {
        let scope_applies = self.scope_applies(&transition.scope);
        let evidence_permitted = self.policy.permits(transition.evidence.truth);
        let hard_guard = if scope_applies && evidence_permitted {
            self.evaluate(&transition.activation.hard_guards)
        } else {
            EvaluatedTruth::Unknown
        };
        let outstanding_obligation_ids = transition
            .activation
            .physical_obligation_ids
            .iter()
            .filter(|id| !discharged_obligation_ids.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        let unknown_requirement_ids = transition
            .activation
            .unknown_requirements
            .iter()
            .map(|requirement| requirement.id.clone())
            .collect::<Vec<_>>();
        let unknown_obligation_ids = transition
            .activation
            .physical_obligation_ids
            .iter()
            .filter(|id| unknown_obligation_ids.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        let classification = if !scope_applies {
            TransitionClassification::Inapplicable
        } else if hard_guard == EvaluatedTruth::False {
            TransitionClassification::GuardBlocked
        } else if !evidence_permitted
            || hard_guard == EvaluatedTruth::Unknown
            || (mode == FeasibilityMode::Modeled && !unknown_obligation_ids.is_empty())
            || !unknown_requirement_ids.is_empty()
        {
            TransitionClassification::FeasibilityUnknown
        } else if mode == FeasibilityMode::Modeled && !outstanding_obligation_ids.is_empty() {
            TransitionClassification::Obstructed
        } else {
            TransitionClassification::Executable
        };
        TransitionAssessment {
            transition_id: transition.id.clone(),
            classification,
            scope_applies,
            evidence_permitted,
            hard_guard,
            outstanding_obligation_ids,
            unknown_obligation_ids,
            unknown_requirement_ids,
        }
    }

    pub fn assess_obligation(
        &self,
        obligation: &FeasibilityObligation,
        microtraces: &[WitnessedMicrotrace],
    ) -> ObligationAssessment {
        let mut supporting_microtrace_ids = Vec::new();
        let (classification, predicate) = if !self.scope_applies(&obligation.scope) {
            (ObligationClassification::Inapplicable, None)
        } else if !self.policy.permits(obligation.evidence.truth) {
            (ObligationClassification::EvidenceUnknown, None)
        } else {
            match &obligation.detail {
                ObligationDetail::Predicate { predicate } => {
                    let result = self.evaluate(predicate);
                    (
                        match result {
                            EvaluatedTruth::True => ObligationClassification::Satisfied,
                            EvaluatedTruth::False => ObligationClassification::Unsatisfied,
                            EvaluatedTruth::Unknown => ObligationClassification::EvaluationUnknown,
                        },
                        Some(result),
                    )
                }
                ObligationDetail::Interaction {
                    actor_instance_id,
                    required_volumes,
                    excluded_volumes,
                    pose_predicate,
                    temporal_requirement,
                    ..
                } => {
                    let pose = self.evaluate(pose_predicate);
                    let actor = self.interaction_actor_loaded(actor_instance_id);
                    let spatial = required_volumes
                        .iter()
                        .map(|volume| self.player_inside_volume(volume))
                        .chain(
                            excluded_volumes
                                .iter()
                                .map(|volume| self.player_inside_volume(volume).not()),
                        )
                        .fold(EvaluatedTruth::True, and_evaluated_truth);
                    let temporal = temporal_requirement
                        .as_ref()
                        .map_or((EvaluatedTruth::True, Vec::new()), |requirement| {
                            self.assess_temporal(requirement, microtraces)
                        });
                    supporting_microtrace_ids = temporal.1;
                    let combined = and_evaluated_truth(
                        and_evaluated_truth(and_evaluated_truth(pose, actor), spatial),
                        temporal.0,
                    );
                    (classify_obligation_truth(combined), Some(combined))
                }
                ObligationDetail::CompoundInteraction {
                    actor_instance_id,
                    branches,
                    temporal_requirement,
                    ..
                } => {
                    let actor = self.interaction_actor_loaded(actor_instance_id);
                    let branch_result = branches
                        .iter()
                        .map(|branch| match self.evaluate(&branch.when) {
                            EvaluatedTruth::False => EvaluatedTruth::False,
                            EvaluatedTruth::Unknown => EvaluatedTruth::Unknown,
                            EvaluatedTruth::True => branch
                                .volume_tests
                                .iter()
                                .map(|test| {
                                    let result = self.interaction_position_inside_volume(
                                        test.position,
                                        &test.volume,
                                    );
                                    if test.must_be_inside {
                                        result
                                    } else {
                                        result.not()
                                    }
                                })
                                .fold(self.evaluate(&branch.pose_predicate), and_evaluated_truth),
                        })
                        .fold(EvaluatedTruth::False, or_evaluated_truth);
                    let temporal = temporal_requirement
                        .as_ref()
                        .map_or((EvaluatedTruth::True, Vec::new()), |requirement| {
                            self.assess_temporal(requirement, microtraces)
                        });
                    supporting_microtrace_ids = temporal.1;
                    let combined =
                        and_evaluated_truth(and_evaluated_truth(actor, branch_result), temporal.0);
                    (classify_obligation_truth(combined), Some(combined))
                }
                ObligationDetail::Temporal {
                    requirement,
                    precondition,
                } => {
                    let precondition = self.evaluate(precondition);
                    let temporal = self.assess_temporal(requirement, microtraces);
                    supporting_microtrace_ids = temporal.1;
                    let combined = and_evaluated_truth(precondition, temporal.0);
                    (classify_obligation_truth(combined), Some(combined))
                }
                ObligationDetail::Geometry {
                    approach_id,
                    source_region_id,
                    destination_region_id,
                } => (
                    match self.spatial_connection(
                        approach_id,
                        source_region_id,
                        destination_region_id,
                    ) {
                        Some(SpatialConnectionStatus::Traversable) => {
                            ObligationClassification::Satisfied
                        }
                        Some(SpatialConnectionStatus::Blocked) => {
                            ObligationClassification::Unsatisfied
                        }
                        None => ObligationClassification::EvaluationUnknown,
                    },
                    None,
                ),
                ObligationDetail::PlaneSide { plane_id, relation } => {
                    let result = self.player_on_plane_side(plane_id, *relation);
                    (
                        match result {
                            EvaluatedTruth::True => ObligationClassification::Satisfied,
                            EvaluatedTruth::False => ObligationClassification::Unsatisfied,
                            EvaluatedTruth::Unknown => ObligationClassification::EvaluationUnknown,
                        },
                        Some(result),
                    )
                }
                ObligationDetail::Facing {
                    yaw,
                    target_yaw,
                    maximum_delta,
                } => {
                    let result = match self.resolve_value(yaw) {
                        Some(StateValue::Signed(value)) => {
                            i16::try_from(value).ok().map(|observed| {
                                observed.wrapping_sub(*target_yaw).unsigned_abs() <= *maximum_delta
                            })
                        }
                        _ => None,
                    };
                    (
                        match result {
                            Some(true) => ObligationClassification::Satisfied,
                            Some(false) => ObligationClassification::Unsatisfied,
                            None => ObligationClassification::EvaluationUnknown,
                        },
                        result.map(|value| {
                            if value {
                                EvaluatedTruth::True
                            } else {
                                EvaluatedTruth::False
                            }
                        }),
                    )
                }
                ObligationDetail::Unresolved { .. } => (ObligationClassification::Unmodeled, None),
            }
        };
        if classification != ObligationClassification::Satisfied {
            supporting_microtrace_ids.clear();
        }
        ObligationAssessment {
            obligation_id: obligation.id.clone(),
            classification,
            predicate,
            supporting_microtrace_ids,
        }
    }

    fn assess_temporal(
        &self,
        requirement: &TemporalRequirement,
        microtraces: &[WitnessedMicrotrace],
    ) -> (EvaluatedTruth, Vec<String>) {
        let mut matched = false;
        let mut uncertain = false;
        let mut supporting = Vec::new();
        for trace in microtraces
            .iter()
            .filter(|trace| self.scope_applies(&trace.scope) && trace.witnesses(requirement))
        {
            matched = true;
            if !self.policy.permits(trace.evidence.truth) {
                uncertain = true;
                continue;
            }
            match self.evaluate(&trace.precondition) {
                EvaluatedTruth::True => supporting.push(trace.id.clone()),
                EvaluatedTruth::Unknown => uncertain = true,
                EvaluatedTruth::False => {}
            }
        }
        if !supporting.is_empty() {
            (EvaluatedTruth::True, supporting)
        } else if uncertain || !matched {
            (EvaluatedTruth::Unknown, Vec::new())
        } else {
            (EvaluatedTruth::False, Vec::new())
        }
    }

    fn player_inside_volume(&self, reference: &VolumeReference) -> EvaluatedTruth {
        if !self.world_execution_active() {
            return EvaluatedTruth::Unknown;
        }
        self.position_inside_volume(self.snapshot.environment.player.position, reference)
    }

    fn interaction_position_inside_volume(
        &self,
        position: InteractionPosition,
        reference: &VolumeReference,
    ) -> EvaluatedTruth {
        if !self.world_execution_active() {
            return EvaluatedTruth::Unknown;
        }
        let position = match position {
            InteractionPosition::Player => Some(self.snapshot.environment.player.position),
            InteractionPosition::PlayerAttention => {
                self.snapshot.environment.player.attention_position
            }
        };
        position.map_or(EvaluatedTruth::Unknown, |position| {
            self.position_inside_volume(position, reference)
        })
    }

    fn position_inside_volume(
        &self,
        position: [f32; 3],
        reference: &VolumeReference,
    ) -> EvaluatedTruth {
        let Some(volume) = self
            .snapshot
            .environment
            .spatial_volumes
            .iter()
            .find(|volume| {
                volume.object_id == reference.object_id && volume.volume_id == reference.volume_id
            })
        else {
            return EvaluatedTruth::Unknown;
        };
        match &volume.shape {
            SpatialVolumeShape::AxisAlignedBox { minimum, maximum } => {
                if position
                    .iter()
                    .zip(minimum.iter().zip(maximum))
                    .all(|(value, (minimum, maximum))| value >= minimum && value <= maximum)
                {
                    EvaluatedTruth::True
                } else {
                    EvaluatedTruth::False
                }
            }
            SpatialVolumeShape::Sphere { center, radius } => {
                let squared_distance = position
                    .iter()
                    .zip(center)
                    .map(|(value, center)| {
                        let delta = f64::from(*value) - f64::from(*center);
                        delta * delta
                    })
                    .sum::<f64>();
                if squared_distance <= f64::from(*radius).powi(2) {
                    EvaluatedTruth::True
                } else {
                    EvaluatedTruth::False
                }
            }
            SpatialVolumeShape::VerticalCylinder {
                center_xz,
                minimum_y,
                maximum_y,
                radius,
            } => {
                let delta_x = f64::from(position[0]) - f64::from(center_xz[0]);
                let delta_z = f64::from(position[2]) - f64::from(center_xz[1]);
                if position[1] >= *minimum_y
                    && position[1] <= *maximum_y
                    && delta_x * delta_x + delta_z * delta_z <= f64::from(*radius).powi(2)
                {
                    EvaluatedTruth::True
                } else {
                    EvaluatedTruth::False
                }
            }
            SpatialVolumeShape::YawOrientedRectangle {
                origin_xz,
                yaw,
                minimum_local_xz,
                maximum_local_xz,
            } => {
                let delta_x = f64::from(position[0]) - f64::from(origin_xz[0]);
                let delta_z = f64::from(position[2]) - f64::from(origin_xz[1]);
                let radians = f64::from(*yaw) * std::f64::consts::TAU / 65536.0;
                let (sin, cos) = radians.sin_cos();
                // This is the inverse of the game's actor-local +Y yaw:
                // world +Z is (sin(yaw), cos(yaw)) in the X/Z plane.
                let local_x = cos * delta_x - sin * delta_z;
                let local_z = sin * delta_x + cos * delta_z;
                if local_x >= f64::from(minimum_local_xz[0])
                    && local_x <= f64::from(maximum_local_xz[0])
                    && local_z >= f64::from(minimum_local_xz[1])
                    && local_z <= f64::from(maximum_local_xz[1])
                {
                    EvaluatedTruth::True
                } else {
                    EvaluatedTruth::False
                }
            }
            SpatialVolumeShape::YawOrientedStrip {
                origin_xz,
                yaw,
                axis,
                minimum,
                maximum,
            } => {
                let delta_x = f64::from(position[0]) - f64::from(origin_xz[0]);
                let delta_z = f64::from(position[2]) - f64::from(origin_xz[1]);
                let radians = f64::from(*yaw) * std::f64::consts::TAU / 65536.0;
                let (sin, cos) = radians.sin_cos();
                let local = match axis {
                    crate::state::SpatialLocalAxis::X => cos * delta_x - sin * delta_z,
                    crate::state::SpatialLocalAxis::Z => sin * delta_x + cos * delta_z,
                };
                if local >= f64::from(*minimum) && local <= f64::from(*maximum) {
                    EvaluatedTruth::True
                } else {
                    EvaluatedTruth::False
                }
            }
        }
    }

    fn spatial_connection(
        &self,
        approach_id: &str,
        source_region_id: &str,
        destination_region_id: &str,
    ) -> Option<SpatialConnectionStatus> {
        if !self.world_execution_active() {
            return None;
        }
        self.snapshot
            .environment
            .spatial_connections
            .iter()
            .find(|connection| {
                connection.approach_id == approach_id
                    && connection.source_region_id == source_region_id
                    && connection.destination_region_id == destination_region_id
            })
            .map(|connection| connection.status)
    }

    fn player_on_plane_side(&self, plane_id: &str, relation: PlaneRelation) -> EvaluatedTruth {
        if !self.world_execution_active() {
            return EvaluatedTruth::Unknown;
        }
        let Some(plane) = self
            .snapshot
            .environment
            .spatial_planes
            .iter()
            .find(|plane| plane.plane_id == plane_id)
        else {
            return EvaluatedTruth::Unknown;
        };
        let signed_distance = plane
            .normal
            .iter()
            .zip(self.snapshot.environment.player.position)
            .map(|(normal, coordinate)| f64::from(*normal) * f64::from(coordinate))
            .sum::<f64>()
            + f64::from(plane.offset);
        let satisfied = match relation {
            PlaneRelation::Positive => signed_distance > 0.0,
            PlaneRelation::NonNegative => signed_distance >= 0.0,
            PlaneRelation::Negative => signed_distance < 0.0,
            PlaneRelation::NonPositive => signed_distance <= 0.0,
        };
        if satisfied {
            EvaluatedTruth::True
        } else {
            EvaluatedTruth::False
        }
    }

    fn interaction_actor_loaded(&self, instance_id: &str) -> EvaluatedTruth {
        if !self.world_execution_active() {
            return EvaluatedTruth::Unknown;
        }
        match self
            .snapshot
            .environment
            .live_world_objects
            .iter()
            .find(|object| object.instance_id == instance_id)
            .map(|object| object.lifecycle)
        {
            Some(ActorLifecycle::Loaded) => EvaluatedTruth::True,
            Some(
                ActorLifecycle::Unloading | ActorLifecycle::Unloaded | ActorLifecycle::Destroyed,
            ) => EvaluatedTruth::False,
            None => EvaluatedTruth::Unknown,
        }
    }

    pub fn assess_gate(&self, gate: &GateRule) -> GateAssessment {
        let scope_applies = self.scope_applies(&gate.scope);
        let evidence_permitted = self.policy.permits(gate.evidence.truth);
        let active = if scope_applies && evidence_permitted {
            self.evaluate(&gate.active_when)
        } else {
            EvaluatedTruth::Unknown
        };
        GateAssessment {
            gate_id: gate.id.clone(),
            scope_applies,
            evidence_permitted,
            active,
        }
    }

    pub fn assess_writer(&self, writer: &WriterRule, gates: &[GateRule]) -> WriterAssessment {
        let scope_applies = self.scope_applies(&writer.scope);
        let evidence_permitted = self.policy.permits(writer.evidence.truth);
        let activation = if scope_applies && evidence_permitted {
            self.evaluate(&writer.activation)
        } else {
            EvaluatedTruth::Unknown
        };
        let mut active_gate_ids = Vec::new();
        let mut unknown_gate_ids = Vec::new();
        for gate in gates.iter().filter(|gate| {
            gate.blocked_writer_ids
                .iter()
                .any(|writer_id| writer_id == &writer.id)
        }) {
            let assessment = self.assess_gate(gate);
            match assessment.active {
                EvaluatedTruth::True => active_gate_ids.push(gate.id.clone()),
                EvaluatedTruth::Unknown => unknown_gate_ids.push(gate.id.clone()),
                EvaluatedTruth::False => {}
            }
        }
        let classification = if !scope_applies {
            WriterClassification::Inapplicable
        } else if activation == EvaluatedTruth::False {
            WriterClassification::Inactive
        } else if !evidence_permitted || activation == EvaluatedTruth::Unknown {
            WriterClassification::ActivationUnknown
        } else if !active_gate_ids.is_empty() {
            WriterClassification::GateBlocked
        } else if !unknown_gate_ids.is_empty() {
            WriterClassification::GateUnknown
        } else {
            WriterClassification::Executable
        };
        WriterAssessment {
            writer_id: writer.id.clone(),
            classification,
            scope_applies,
            evidence_permitted,
            activation,
            active_gate_ids,
            unknown_gate_ids,
        }
    }

    pub fn assess_reader(&self, reader: &ReaderRule) -> ReaderAssessment {
        let scope_applies = self.scope_applies(&reader.scope);
        let evidence_permitted = self.policy.permits(reader.evidence.truth);
        let source_value = if scope_applies && evidence_permitted {
            self.resolve_value(&reader.source)
        } else {
            None
        };
        let interpretation = if scope_applies && evidence_permitted {
            reader.interpretation_fact_id.as_ref().map(|fact_id| {
                self.evaluate(&PredicateExpression::Fact {
                    fact_id: fact_id.clone(),
                })
            })
        } else {
            None
        };
        ReaderAssessment {
            reader_id: reader.id.clone(),
            scope_applies,
            evidence_permitted,
            source_value,
            interpretation,
        }
    }

    pub fn assess_obstruction(&self, obstruction: &Obstruction) -> ObstructionAssessment {
        let (classification, activation) = self.assess_rule(
            &obstruction.scope,
            obstruction.evidence.truth,
            &obstruction.active_when,
        );
        ObstructionAssessment {
            obstruction_id: obstruction.id.clone(),
            classification,
            activation,
            obligation_ids: obstruction.obligation_ids.clone(),
        }
    }

    pub fn assess_resolver(&self, resolver: &ObstructionResolver) -> ResolverAssessment {
        let (classification, applicability) = self.assess_rule(
            &resolver.scope,
            resolver.evidence.truth,
            &resolver.applicable_when,
        );
        ResolverAssessment {
            resolver_id: resolver.id.clone(),
            obstruction_id: resolver.obstruction_id.clone(),
            classification,
            applicability,
        }
    }

    pub fn assess_technique(&self, technique: &Technique) -> TechniqueAssessment {
        let (classification, prerequisites) = self.assess_rule(
            &technique.scope,
            technique.evidence.truth,
            &technique.prerequisites,
        );
        TechniqueAssessment {
            technique_id: technique.id.clone(),
            classification,
            prerequisites,
            discharged_obligation_ids: technique.discharged_obligation_ids.clone(),
            introduced_obligation_ids: technique.introduced_obligation_ids.clone(),
        }
    }

    pub fn assess_reconstruction(
        &self,
        rule: &ActorReconstructionRule,
    ) -> ReconstructionAssessment {
        let (classification, activation) =
            self.assess_rule(&rule.scope, rule.evidence.truth, &rule.instantiate_when);
        ReconstructionAssessment {
            reconstruction_rule_id: rule.id.clone(),
            classification,
            activation,
        }
    }

    /// Resolves only records relevant to one transition and approach. A
    /// resolver discharges the obligations named by its obstruction; a
    /// technique discharges only its explicit list. Neither deletes the
    /// obstruction or changes its underlying activation fact.
    pub fn resolve_feasibility(
        &self,
        transition: &CandidateTransition,
        obligations: &[FeasibilityObligation],
        obstructions: &[Obstruction],
        resolvers: &[ObstructionResolver],
        techniques: &[Technique],
        selection: FeasibilitySelection<'_>,
    ) -> FeasibilityResolution {
        let mut resolution = FeasibilityResolution {
            claimed_obligation_ids: selection.already_discharged.clone(),
            discharged_obligation_ids: BTreeSet::new(),
            unknown_obligation_ids: BTreeSet::new(),
            supporting_microtrace_ids: BTreeSet::new(),
            active_obstruction_ids: Vec::new(),
            unknown_obstruction_ids: Vec::new(),
            applied_resolver_ids: Vec::new(),
            applicable_technique_ids: Vec::new(),
        };

        for technique in techniques
            .iter()
            .filter(|technique| selection.technique_ids.contains(&technique.id))
        {
            let assessment = self.assess_technique(technique);
            if assessment.classification == RuleClassification::Active {
                resolution
                    .claimed_obligation_ids
                    .extend(assessment.discharged_obligation_ids);
                for introduced in assessment.introduced_obligation_ids {
                    resolution.claimed_obligation_ids.remove(&introduced);
                }
                resolution
                    .applicable_technique_ids
                    .push(technique.id.clone());
            }
        }

        for obstruction in obstructions.iter().filter(|obstruction| {
            obstruction.blocked_action_id == transition.id
                && obstruction.approach_id == transition.approach_id
        }) {
            let assessment = self.assess_obstruction(obstruction);
            match assessment.classification {
                RuleClassification::Active => {
                    resolution
                        .active_obstruction_ids
                        .push(obstruction.id.clone());
                    let applicable = resolvers
                        .iter()
                        .filter(|resolver| resolver.obstruction_id == obstruction.id)
                        .filter(|resolver| selection.resolver_ids.contains(&resolver.id))
                        .filter(|resolver| {
                            self.assess_resolver(resolver).classification
                                == RuleClassification::Active
                        })
                        .collect::<Vec<_>>();
                    if !applicable.is_empty() {
                        resolution
                            .claimed_obligation_ids
                            .extend(obstruction.obligation_ids.iter().cloned());
                        resolution
                            .applied_resolver_ids
                            .extend(applicable.into_iter().map(|resolver| resolver.id.clone()));
                    }
                }
                RuleClassification::EvidenceUnknown | RuleClassification::ActivationUnknown => {
                    resolution
                        .unknown_obstruction_ids
                        .push(obstruction.id.clone())
                }
                RuleClassification::Inapplicable | RuleClassification::Inactive => {}
            }
        }
        self.refresh_obligation_assessments(
            transition,
            obligations,
            selection.microtraces,
            &mut resolution,
        );
        resolution
    }

    pub fn refresh_obligation_assessments(
        &self,
        transition: &CandidateTransition,
        obligations: &[FeasibilityObligation],
        microtraces: &[WitnessedMicrotrace],
        resolution: &mut FeasibilityResolution,
    ) {
        resolution.discharged_obligation_ids = resolution.claimed_obligation_ids.clone();
        resolution.unknown_obligation_ids.clear();
        resolution.supporting_microtrace_ids.clear();
        for obligation_id in &transition.activation.physical_obligation_ids {
            if resolution.claimed_obligation_ids.contains(obligation_id) {
                continue;
            }
            let Some(obligation) = obligations
                .iter()
                .find(|record| record.id == *obligation_id)
            else {
                resolution
                    .unknown_obligation_ids
                    .insert(obligation_id.clone());
                continue;
            };
            let assessment = self.assess_obligation(obligation, microtraces);
            resolution
                .supporting_microtrace_ids
                .extend(assessment.supporting_microtrace_ids);
            match assessment.classification {
                ObligationClassification::Satisfied => {
                    resolution
                        .discharged_obligation_ids
                        .insert(obligation.id.clone());
                }
                ObligationClassification::Inapplicable
                | ObligationClassification::EvidenceUnknown
                | ObligationClassification::EvaluationUnknown
                | ObligationClassification::Unmodeled => {
                    resolution
                        .unknown_obligation_ids
                        .insert(obligation.id.clone());
                }
                ObligationClassification::Unsatisfied => {}
            }
        }
    }

    fn assess_rule(
        &self,
        scope: &ContextScope,
        truth: TruthStatus,
        expression: &PredicateExpression,
    ) -> (RuleClassification, EvaluatedTruth) {
        if !self.scope_applies(scope) {
            return (RuleClassification::Inapplicable, EvaluatedTruth::Unknown);
        }
        if !self.policy.permits(truth) {
            return (RuleClassification::EvidenceUnknown, EvaluatedTruth::Unknown);
        }
        let activation = self.evaluate(expression);
        let classification = match activation {
            EvaluatedTruth::True => RuleClassification::Active,
            EvaluatedTruth::False => RuleClassification::Inactive,
            EvaluatedTruth::Unknown => RuleClassification::ActivationUnknown,
        };
        (classification, activation)
    }
}
