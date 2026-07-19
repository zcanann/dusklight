use super::*;

pub(super) fn empty_harness_score() -> TrialScore {
    TrialScore {
        depth: 0,
        deepest: "none".into(),
        score_tick: None,
        goal_reached: false,
        milestone_observations: BTreeMap::new(),
        boundary_fingerprints: BTreeMap::new(),
        value_projections: BTreeMap::new(),
    }
}

pub(super) fn score_harness_result(
    result: &HarnessRunResult,
    request: &HarnessRunRequest,
    objective_result: &Path,
    expected_boot: &TapeBoot,
    segment: SegmentProfile,
    anchored: Option<&PreparedAnchoredObjective>,
) -> Result<TrialScore, EvaluateError> {
    let native: NativeMilestoneResult = serde_json::from_slice(&fs::read(objective_result)?)?;
    if native.schema.name != "dusklight.automation.milestones"
        || native.schema.version != 5
        || native.goal.as_deref() != Some(request.objective.goal.as_str())
        || native.program_digest.as_deref() != Some(&request.objective.program_sha256.to_string())
        || native.milestones.len() != 1
    {
        return Err(EvaluateError::NativeResult(
            "core-harness objective artifact does not match its request".into(),
        ));
    }
    validate_native_boot(&native, expected_boot)?;
    let milestone = &native.milestones[0];
    if milestone.id != request.objective.goal
        || milestone.hit != result.objective.reached
        || native.goal_reached != result.objective.reached
    {
        return Err(EvaluateError::NativeResult(
            "core-harness result contradicts its native objective artifact".into(),
        ));
    }
    if !milestone.hit {
        return Ok(if let Some(objective) = anchored {
            anchored_source_score(objective)
        } else {
            empty_harness_score()
        });
    }
    let (sim_tick, tape_frame, evidence) = match (
        milestone.sim_tick,
        milestone.tape_frame,
        milestone.evidence.as_ref(),
    ) {
        (Some(sim_tick), Some(tape_frame), Some(evidence)) => (sim_tick, tape_frame, evidence),
        _ => {
            return Err(EvaluateError::NativeResult(
                "reached harness objective omitted tick or boundary evidence".into(),
            ));
        }
    };
    validate_fingerprint(&evidence.boundary_fingerprint)?;
    let mut observations = BTreeMap::new();
    observations.insert(
        milestone.id.clone(),
        MilestoneObservation {
            sim_tick,
            tape_frame,
            boundary_index: milestone.boundary_index,
            phase: milestone.phase.clone(),
            stable_ticks: milestone.stable_ticks,
            definition_digest: milestone.definition_digest.clone(),
            program_digest: milestone.program_digest.clone(),
        },
    );
    let mut fingerprints = BTreeMap::new();
    fingerprints.insert(milestone.id.clone(), evidence.boundary_fingerprint.clone());
    let projections = validate_value_projections(milestone.projections.as_ref())?;
    let value_projections = if projections.is_empty() {
        BTreeMap::new()
    } else {
        BTreeMap::from([(milestone.id.clone(), projections)])
    };
    if let Some(objective) = anchored {
        let mut score = anchored_source_score(objective);
        score.depth = 2;
        score.deepest = milestone.id.clone();
        score.score_tick = Some(
            tape_frame
                .checked_sub(objective.identity.source_boundary_index)
                .ok_or_else(|| {
                    EvaluateError::NativeResult(
                        "anchored harness goal fired inside the immutable prefix".into(),
                    )
                })?,
        );
        score.goal_reached = true;
        score.milestone_observations.extend(observations);
        score.boundary_fingerprints.extend(fingerprints);
        score.value_projections = value_projections;
        return Ok(score);
    }
    let depth = match segment {
        SegmentProfile::BootToFsp103 => 2,
        SegmentProfile::Fsp103ToFsp104 => 4,
        SegmentProfile::LinkControlToTunnelCrawlStart => unreachable!("anchored profile"),
    };
    Ok(TrialScore {
        depth,
        deepest: milestone.id.clone(),
        score_tick: Some(sim_tick),
        goal_reached: true,
        milestone_observations: observations,
        boundary_fingerprints: fingerprints,
        value_projections,
    })
}

pub(super) fn anchored_source_score(objective: &PreparedAnchoredObjective) -> TrialScore {
    TrialScore {
        depth: 1,
        deepest: objective.identity.source_milestone.clone(),
        score_tick: Some(0),
        goal_reached: false,
        milestone_observations: BTreeMap::from([(
            objective.identity.source_milestone.clone(),
            MilestoneObservation {
                sim_tick: objective.identity.source_tape_frame,
                tape_frame: objective.identity.source_tape_frame,
                boundary_index: Some(objective.identity.source_boundary_index),
                phase: Some(objective.source.phase.clone()),
                stable_ticks: Some(objective.source.stable_ticks),
                definition_digest: Some(objective.source.digest.clone()),
                program_digest: Some(objective.identity.milestone_program_sha256.clone()),
            },
        )]),
        boundary_fingerprints: BTreeMap::from([(
            objective.identity.source_milestone.clone(),
            BoundaryFingerprint {
                schema: "dusklight.milestone-boundary/v1".into(),
                algorithm: "xxh3-128".into(),
                canonical_encoding: "little-endian-fixed-v1".into(),
                digest: objective.identity.source_boundary_fingerprint.clone(),
            },
        )]),
        value_projections: BTreeMap::new(),
    }
}

pub(super) fn validate_native_exit(
    status: ExitStatus,
    goal_reached: bool,
) -> Result<(), EvaluateError> {
    match (status.code(), goal_reached) {
        (Some(0), true) | (Some(NATIVE_GOAL_MISS_EXIT_CODE), false) => Ok(()),
        (code, _) => Err(EvaluateError::NativeResult(format!(
            "worker exit {code:?} disagrees with goal_reached={goal_reached} (expected 0 for a hit or {NATIVE_GOAL_MISS_EXIT_CODE} for a valid miss)"
        ))),
    }
}

#[derive(Debug)]
pub(super) struct TrialScore {
    pub(super) depth: u16,
    pub(super) deepest: String,
    pub(super) score_tick: Option<u64>,
    pub(super) goal_reached: bool,
    pub(super) milestone_observations: BTreeMap<String, MilestoneObservation>,
    pub(super) boundary_fingerprints: BTreeMap<String, BoundaryFingerprint>,
    pub(super) value_projections: BTreeMap<String, BTreeMap<String, ValueProjectionEvidence>>,
}

#[derive(Deserialize)]
struct NativeMilestoneResult {
    schema: NativeSchema,
    boot: Option<TapeBoot>,
    boot_origin_established: Option<bool>,
    goal: Option<String>,
    goal_reached: bool,
    program_digest: Option<String>,
    milestones: Vec<NativeMilestone>,
}

#[derive(Deserialize)]
struct NativeSchema {
    name: String,
    version: u32,
}

#[derive(Deserialize)]
struct NativeMilestone {
    id: String,
    hit: bool,
    sim_tick: Option<u64>,
    tape_frame: Option<u64>,
    phase: Option<String>,
    stable_ticks: Option<u16>,
    definition_digest: Option<String>,
    program_digest: Option<String>,
    boundary_index: Option<u64>,
    evidence: Option<NativeEvidence>,
    projections: Option<Vec<ValueProjectionEvidence>>,
}

#[derive(Deserialize)]
struct NativeEvidence {
    boundary_fingerprint: BoundaryFingerprint,
    boot: Option<TapeBoot>,
    stage: Option<NativeStageEvidence>,
    next_stage: Option<NativeNextStageEvidence>,
    player: Option<NativePlayerEvidence>,
}

#[derive(Deserialize)]
struct NativeStageEvidence {
    name: String,
    room: i32,
    point: i32,
}

#[derive(Deserialize)]
struct NativeNextStageEvidence {
    enabled: bool,
    name: String,
    room: i32,
    point: i32,
}

#[derive(Deserialize)]
struct NativePlayerEvidence {
    present: bool,
    is_link: bool,
    procedure_id: u16,
}

pub(super) fn parse_anchored_milestones(
    path: &Path,
    objective: &PreparedAnchoredObjective,
    expected_boot: &TapeBoot,
) -> Result<TrialScore, EvaluateError> {
    let native: NativeMilestoneResult =
        serde_json::from_slice(&fs::read(path).map_err(|error| {
            EvaluateError::NativeResult(format!(
                "worker produced no readable milestone result at {}: {error}",
                path.display()
            ))
        })?)?;
    if native.schema.name != "dusklight.automation.milestones"
        || !matches!(native.schema.version, 1..=5)
    {
        return Err(EvaluateError::NativeResult(
            "unsupported native milestone schema".into(),
        ));
    }
    validate_native_boot(&native, expected_boot)?;
    if native.program_digest.as_deref()
        != Some(objective.identity.milestone_program_sha256.as_str())
    {
        return Err(EvaluateError::NativeResult(
            "native result milestone program digest does not match the anchored objective".into(),
        ));
    }
    if native.goal.as_deref() != Some(objective.identity.goal_milestone.as_str()) {
        return Err(EvaluateError::NativeResult(format!(
            "native result goal {:?} does not match anchored goal {}",
            native.goal, objective.identity.goal_milestone
        )));
    }
    let mut milestones = BTreeMap::new();
    for milestone in native.milestones {
        let id = milestone.id.clone();
        if milestones.insert(id.clone(), milestone).is_some() {
            return Err(EvaluateError::NativeResult(format!(
                "duplicate native milestone {id}"
            )));
        }
    }
    let requested = [
        objective.identity.source_milestone.as_str(),
        objective.identity.goal_milestone.as_str(),
    ];
    if milestones.len() != requested.len()
        || requested.iter().any(|id| !milestones.contains_key(*id))
    {
        return Err(EvaluateError::NativeResult(
            "native result does not contain the exact anchored milestone set".into(),
        ));
    }
    let expected = |id: &str| {
        if id == objective.identity.source_milestone {
            &objective.source
        } else {
            &objective.goal
        }
    };
    let mut observations = BTreeMap::new();
    let mut fingerprints = BTreeMap::new();
    let mut value_projections = BTreeMap::new();
    for (id, milestone) in &milestones {
        let definition = expected(id);
        if milestone.phase.as_deref() != Some(definition.phase.as_str())
            || milestone.stable_ticks != Some(definition.stable_ticks)
            || milestone.definition_digest.as_deref() != Some(definition.digest.as_str())
            || milestone.program_digest.as_deref()
                != Some(objective.identity.milestone_program_sha256.as_str())
        {
            return Err(EvaluateError::NativeResult(format!(
                "milestone {id} authored proof metadata does not match the anchored objective"
            )));
        }
        if milestone.hit {
            if native.schema.version >= 5 && milestone.projections.is_none() {
                return Err(EvaluateError::NativeResult(format!(
                    "milestone {id} omitted its value projection evidence"
                )));
            }
            let projections = validate_value_projections(milestone.projections.as_ref())?;
            if projections.len() != definition.projections.len()
                || definition.projections.iter().any(|(name, identity)| {
                    projections.get(name).map(|projection| &projection.identity) != Some(identity)
                })
            {
                return Err(EvaluateError::NativeResult(format!(
                    "milestone {id} value projection identities do not match the authored program"
                )));
            }
            if !projections.is_empty() {
                value_projections.insert(id.clone(), projections);
            }
        } else if milestone.projections.is_some() {
            return Err(EvaluateError::NativeResult(format!(
                "unhit milestone {id} contains value projection evidence"
            )));
        }
        match (
            milestone.hit,
            milestone.boundary_index,
            milestone.sim_tick,
            milestone.tape_frame,
            &milestone.evidence,
        ) {
            (true, Some(boundary_index), Some(sim_tick), Some(tape_frame), Some(evidence)) => {
                if boundary_index != tape_frame.saturating_add(1) || sim_tick != tape_frame {
                    return Err(EvaluateError::NativeResult(format!(
                        "milestone {id} tick, tape frame, and boundary index are not one absolute fixed-step boundary"
                    )));
                }
                validate_fingerprint(&evidence.boundary_fingerprint)?;
                validate_evidence_boot(evidence, native.schema.version, expected_boot)?;
                observations.insert(
                    id.clone(),
                    MilestoneObservation {
                        sim_tick,
                        tape_frame,
                        boundary_index: Some(boundary_index),
                        phase: milestone.phase.clone(),
                        stable_ticks: milestone.stable_ticks,
                        definition_digest: milestone.definition_digest.clone(),
                        program_digest: milestone.program_digest.clone(),
                    },
                );
                fingerprints.insert(id.clone(), evidence.boundary_fingerprint.clone());
            }
            (false, None, None, None, None) => {}
            _ => {
                return Err(EvaluateError::NativeResult(format!(
                    "milestone {id} has inconsistent authored hit evidence"
                )));
            }
        }
    }
    let source = &milestones[&objective.identity.source_milestone];
    if !source.hit {
        return Err(EvaluateError::NativeResult(
            "immutable prefix did not reproduce the anchored source milestone".into(),
        ));
    }
    if source.tape_frame != Some(objective.identity.source_tape_frame)
        || source.boundary_index != Some(objective.identity.source_boundary_index)
        || fingerprints[&objective.identity.source_milestone].digest
            != objective.identity.source_boundary_fingerprint
    {
        return Err(EvaluateError::NativeResult(
            "immutable prefix source frame, boundary index, or fingerprint changed".into(),
        ));
    }
    let goal = &milestones[&objective.identity.goal_milestone];
    if native.goal_reached != goal.hit {
        return Err(EvaluateError::NativeResult(
            "goal_reached disagrees with the authored anchored goal".into(),
        ));
    }
    let score_tick = if goal.hit {
        let goal_frame = goal.tape_frame.expect("hit tuple checked above");
        if goal_frame < objective.identity.prefix_frames {
            return Err(EvaluateError::NativeResult(
                "anchored goal fired inside the immutable prefix".into(),
            ));
        }
        let evidence = goal.evidence.as_ref().expect("hit tuple checked above");
        let stage = evidence.stage.as_ref().ok_or_else(|| {
            EvaluateError::NativeResult("anchored goal evidence has no stage object".into())
        })?;
        match objective.identity.segment {
            SegmentProfile::Fsp103ToFsp104 => {
                let next_stage = evidence.next_stage.as_ref().ok_or_else(|| {
                    EvaluateError::NativeResult(
                        "Ordon transition goal evidence has no next_stage object".into(),
                    )
                })?;
                if stage.name != "F_SP103"
                    || stage.room != 1
                    || !next_stage.enabled
                    || next_stage.name != "F_SP104"
                    || next_stage.room != 1
                    || next_stage.point != 0
                {
                    return Err(EvaluateError::NativeResult(
                        "anchored goal evidence is not the committed F_SP103 to F_SP104 room 1 spawn 0 transition"
                            .into(),
                    ));
                }
            }
            SegmentProfile::LinkControlToTunnelCrawlStart => {
                let player = evidence.player.as_ref().ok_or_else(|| {
                    EvaluateError::NativeResult("tunnel goal evidence has no player object".into())
                })?;
                if stage.name != "F_SP104"
                    || stage.room != 1
                    || stage.point != 0
                    || !player.present
                    || !player.is_link
                    || player.procedure_id != 53
                {
                    return Err(EvaluateError::NativeResult(
                        "anchored goal evidence is not F_SP104 room 1 spawn 0 crawl_start (53)"
                            .into(),
                    ));
                }
            }
            SegmentProfile::BootToFsp103 => unreachable!("validated anchored profile"),
        }
        Some(goal_frame - objective.identity.source_boundary_index)
    } else {
        Some(0)
    };
    Ok(TrialScore {
        depth: if goal.hit { 2 } else { 1 },
        deepest: if goal.hit {
            objective.identity.goal_milestone.clone()
        } else {
            objective.identity.source_milestone.clone()
        },
        score_tick,
        goal_reached: goal.hit,
        milestone_observations: observations,
        boundary_fingerprints: fingerprints,
        value_projections,
    })
}

pub(super) fn parse_native_milestones(
    path: &Path,
    segment: SegmentProfile,
    expected_boot: &TapeBoot,
) -> Result<TrialScore, EvaluateError> {
    let native: NativeMilestoneResult =
        serde_json::from_slice(&fs::read(path).map_err(|error| {
            EvaluateError::NativeResult(format!(
                "worker produced no readable milestone result at {}: {error}",
                path.display()
            ))
        })?)?;
    if native.schema.name != "dusklight.automation.milestones"
        || !matches!(native.schema.version, 1..=5)
    {
        return Err(EvaluateError::NativeResult(
            "unsupported native milestone schema".into(),
        ));
    }
    validate_native_boot(&native, expected_boot)?;
    let expected_goal = match segment {
        SegmentProfile::BootToFsp103 => "gameplay-ready-f-sp103",
        SegmentProfile::Fsp103ToFsp104 => "entered-f-sp104",
        SegmentProfile::LinkControlToTunnelCrawlStart => {
            unreachable!("anchored profiles are evaluated through evaluate_anchored_population")
        }
    };
    if native.goal.as_deref() != Some(expected_goal) {
        return Err(EvaluateError::NativeResult(format!(
            "native result goal {:?} does not match {expected_goal}",
            native.goal
        )));
    }
    let mut milestones = BTreeMap::new();
    for milestone in native.milestones {
        let id = milestone.id.clone();
        if milestones.insert(id.clone(), milestone).is_some() {
            return Err(EvaluateError::NativeResult(format!(
                "duplicate native milestone {id}"
            )));
        }
    }
    let requested: &[&str] = match segment {
        SegmentProfile::BootToFsp103 => &["gameplay-ready-f-sp103"],
        SegmentProfile::Fsp103ToFsp104 => &[
            "gameplay-ready-f-sp103",
            "exit-f-sp103-to-f-sp104",
            "entered-f-sp104",
        ],
        SegmentProfile::LinkControlToTunnelCrawlStart => {
            unreachable!("anchored profiles are evaluated through evaluate_anchored_population")
        }
    };
    if milestones.len() != requested.len()
        || requested.iter().any(|id| !milestones.contains_key(*id))
    {
        return Err(EvaluateError::NativeResult(
            "native result does not contain the exact requested milestone set".into(),
        ));
    }
    let mut fingerprints = BTreeMap::new();
    let mut observations = BTreeMap::new();
    let value_projections = BTreeMap::new();
    for (id, milestone) in &milestones {
        match (
            milestone.hit,
            milestone.sim_tick,
            milestone.tape_frame,
            &milestone.evidence,
        ) {
            (true, Some(sim_tick), Some(tape_frame), Some(evidence)) => {
                validate_fingerprint(&evidence.boundary_fingerprint)?;
                validate_evidence_boot(evidence, native.schema.version, expected_boot)?;
                observations.insert(
                    id.clone(),
                    MilestoneObservation {
                        sim_tick,
                        tape_frame,
                        boundary_index: None,
                        phase: None,
                        stable_ticks: None,
                        definition_digest: None,
                        program_digest: None,
                    },
                );
                fingerprints.insert(id.clone(), evidence.boundary_fingerprint.clone());
            }
            (false, None, None, None) => {}
            _ => {
                return Err(EvaluateError::NativeResult(format!(
                    "milestone {id} has inconsistent hit evidence"
                )));
            }
        }
    }
    let hit = |id: &str| milestones[id].hit;
    let tick = |id: &str| milestones[id].sim_tick;
    if native.goal_reached != hit(expected_goal) {
        return Err(EvaluateError::NativeResult(
            "goal_reached disagrees with the goal milestone".into(),
        ));
    }
    let (depth, deepest, score_tick) = match segment {
        SegmentProfile::BootToFsp103 if hit("gameplay-ready-f-sp103") => {
            (2, "gameplay-ready-f-sp103", tick("gameplay-ready-f-sp103"))
        }
        SegmentProfile::BootToFsp103 => (0, "none", None),
        SegmentProfile::Fsp103ToFsp104 if hit("entered-f-sp104") => {
            if !hit("exit-f-sp103-to-f-sp104") {
                return Err(EvaluateError::NativeResult(
                    "entered F_SP104 without the required source-exit milestone".into(),
                ));
            }
            (4, "entered-f-sp104", tick("exit-f-sp103-to-f-sp104"))
        }
        SegmentProfile::Fsp103ToFsp104 if hit("exit-f-sp103-to-f-sp104") => (
            3,
            "exit-f-sp103-to-f-sp104",
            tick("exit-f-sp103-to-f-sp104"),
        ),
        SegmentProfile::Fsp103ToFsp104 if hit("gameplay-ready-f-sp103") => {
            (2, "gameplay-ready-f-sp103", tick("gameplay-ready-f-sp103"))
        }
        SegmentProfile::Fsp103ToFsp104 => (0, "none", None),
        SegmentProfile::LinkControlToTunnelCrawlStart => {
            unreachable!("anchored profiles are evaluated through evaluate_anchored_population")
        }
    };
    if segment == SegmentProfile::Fsp103ToFsp104
        && hit("exit-f-sp103-to-f-sp104")
        && !hit("gameplay-ready-f-sp103")
    {
        return Err(EvaluateError::NativeResult(
            "source exit was hit without the gameplay-ready prerequisite".into(),
        ));
    }
    Ok(TrialScore {
        depth,
        deepest: deepest.into(),
        score_tick,
        goal_reached: native.goal_reached,
        milestone_observations: observations,
        boundary_fingerprints: fingerprints,
        value_projections,
    })
}

fn validate_native_boot(
    native: &NativeMilestoneResult,
    expected_boot: &TapeBoot,
) -> Result<(), EvaluateError> {
    if native.schema.version < 3 {
        if *expected_boot != TapeBoot::Process {
            return Err(EvaluateError::NativeResult(
                "legacy native milestone result cannot authenticate a stage boot origin".into(),
            ));
        }
        return Ok(());
    }
    if native.boot.as_ref() != Some(expected_boot) {
        return Err(EvaluateError::NativeResult(format!(
            "native milestone boot origin {:?} does not match tape origin {:?}",
            native.boot, expected_boot
        )));
    }
    if native.boot_origin_established != Some(true) {
        return Err(EvaluateError::NativeResult(
            "native milestone result did not establish its declared boot origin".into(),
        ));
    }
    Ok(())
}

fn validate_evidence_boot(
    evidence: &NativeEvidence,
    schema_version: u32,
    expected_boot: &TapeBoot,
) -> Result<(), EvaluateError> {
    if schema_version >= 3 && evidence.boot.as_ref() != Some(expected_boot) {
        return Err(EvaluateError::NativeResult(
            "native boundary evidence lost or changed its boot origin".into(),
        ));
    }
    Ok(())
}

fn validate_fingerprint(fingerprint: &BoundaryFingerprint) -> Result<(), EvaluateError> {
    let supported_contract = (fingerprint.schema == "dusklight.milestone-boundary/v1"
        && fingerprint.canonical_encoding == "little-endian-fixed-v1")
        || (fingerprint.schema == "dusklight.milestone-boundary/v2"
            && fingerprint.canonical_encoding == "little-endian-fixed-v2")
        || (fingerprint.schema == "dusklight.milestone-boundary/v3"
            && fingerprint.canonical_encoding == "little-endian-fixed-v3")
        || (fingerprint.schema == "dusklight.milestone-boundary/v4"
            && fingerprint.canonical_encoding == "little-endian-fixed-v4");
    if !supported_contract
        || fingerprint.algorithm != "xxh3-128"
        || fingerprint.digest.len() != 32
        || !fingerprint
            .digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(EvaluateError::NativeResult(
            "invalid native boundary fingerprint".into(),
        ));
    }
    Ok(())
}

fn validate_value_projections(
    projections: Option<&Vec<ValueProjectionEvidence>>,
) -> Result<BTreeMap<String, ValueProjectionEvidence>, EvaluateError> {
    let mut output = BTreeMap::new();
    for projection in projections.into_iter().flatten() {
        if projection.name.is_empty()
            || projection.name.len() > 96
            || projection.identity.len() != 64
            || !projection
                .identity
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || projection.values.is_empty()
        {
            return Err(EvaluateError::NativeResult(
                "invalid native value projection identity".into(),
            ));
        }
        let all_items_available = projection
            .values
            .iter()
            .all(|value| value.get("available").and_then(serde_json::Value::as_bool) == Some(true));
        if projection.available != all_items_available {
            return Err(EvaluateError::NativeResult(format!(
                "value projection {:?} availability disagrees with its items",
                projection.name
            )));
        }
        match (&projection.value_fingerprint, projection.available) {
            (Some(fingerprint), true)
                if fingerprint.schema == "dusklight.value-projection/v1"
                    && fingerprint.algorithm == "xxh3-128"
                    && fingerprint.canonical_encoding == "little-endian-exact-v1"
                    && fingerprint.digest.len() == 32
                    && fingerprint
                        .digest
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()) => {}
            (None, false) => {}
            _ => {
                return Err(EvaluateError::NativeResult(format!(
                    "value projection {:?} has an invalid value fingerprint",
                    projection.name
                )));
            }
        }
        if output
            .insert(projection.name.clone(), projection.clone())
            .is_some()
        {
            return Err(EvaluateError::NativeResult(format!(
                "duplicate native value projection {:?}",
                projection.name
            )));
        }
    }
    Ok(output)
}
