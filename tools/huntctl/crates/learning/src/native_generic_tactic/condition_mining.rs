//! Infer portable tactic entry and exit conditions from comparative experience.

use super::*;

pub fn mine_tactic_conditions(
    experiences: &[TacticExperience],
) -> Result<MinedTacticConditions, NativeTacticError> {
    if experiences.len() < 2 || experiences.iter().all(|experience| !experience.successful) {
        return Err(NativeTacticError::InvalidPlan(
            "condition mining requires successful and comparative experience",
        ));
    }
    for experience in experiences {
        experience.start.validate()?;
        experience.end.validate()?;
    }
    let positives = experiences
        .iter()
        .filter(|experience| experience.successful)
        .collect::<Vec<_>>();
    let negatives = experiences
        .iter()
        .filter(|experience| !experience.successful)
        .collect::<Vec<_>>();
    let positive_starts = positives
        .iter()
        .map(|experience| predicates(&experience.start))
        .collect::<Vec<_>>();
    let negative_starts = negatives
        .iter()
        .map(|experience| predicates(&experience.start))
        .collect::<Vec<_>>();
    let positive_ends = positives
        .iter()
        .map(|experience| predicates(&experience.end))
        .collect::<Vec<_>>();
    let initiation = discriminating_intersection(&positive_starts, &negative_starts);
    let termination = discriminating_intersection(&positive_ends, &positive_starts);
    Ok(MinedTacticConditions {
        schema: MINED_TACTIC_CONDITIONS_SCHEMA_V1.into(),
        experience_count: experiences.len() as u32,
        successful_count: positives.len() as u32,
        initiation,
        termination,
        coordinate_literals_embedded: false,
        published_procedures_embedded: false,
    })
}

fn predicates(observation: &NativeTacticObservation) -> BTreeSet<MinedObservationPredicate> {
    BTreeSet::from([
        MinedObservationPredicate::Stage(observation.stage.clone()),
        MinedObservationPredicate::Room(observation.room),
        MinedObservationPredicate::PlayerProcedure(observation.player_procedure),
        MinedObservationPredicate::PlayerModeFlags(observation.player_mode_flags),
        MinedObservationPredicate::PlayerContacts(observation.player_contacts),
    ])
}

fn discriminating_intersection(
    positive: &[BTreeSet<MinedObservationPredicate>],
    comparison: &[BTreeSet<MinedObservationPredicate>],
) -> Vec<MinedObservationPredicate> {
    let Some(first) = positive.first() else {
        return Vec::new();
    };
    first
        .iter()
        .filter(|predicate| {
            positive.iter().all(|row| row.contains(*predicate))
                && comparison.iter().all(|row| !row.contains(*predicate))
        })
        .cloned()
        .collect()
}
