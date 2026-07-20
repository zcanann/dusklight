use super::{
    NativeActorIdentity, NativeActorObservation, NativeChannelStatus, NativeEpisodeShardError,
    NativePlayerRelationshipsObservation, Reader, decode_actor_identity, decode_channel_status,
};
use std::collections::BTreeSet;

fn relationship_identity(
    identity: NativeActorIdentity,
) -> Result<Option<NativeActorIdentity>, NativeEpisodeShardError> {
    if identity.present {
        return Ok(Some(identity));
    }
    if identity.runtime_generation != u32::MAX
        || identity.actor_name != -1
        || identity.set_id != u16::MAX
        || identity.home_room != -1
        || identity.current_room != -1
        || identity.home_position.is_some()
    {
        return Err(NativeEpisodeShardError::new(
            "absent player relationship has a noncanonical actor identity",
        ));
    }
    Ok(None)
}

pub(super) fn decode_player_relationships(
    reader: &mut Reader<'_>,
) -> Result<(NativeChannelStatus, NativePlayerRelationshipsObservation), NativeEpisodeShardError> {
    let status = decode_channel_status(reader)?;
    if reader.u8()? != 11 || reader.u16()? != 0 {
        return Err(NativeEpisodeShardError::new(
            "invalid player-relationship role count or reserved bytes",
        ));
    }
    let relationships = NativePlayerRelationshipsObservation {
        targeted_actor: relationship_identity(decode_actor_identity(reader)?)?,
        ride_actor: relationship_identity(decode_actor_identity(reader)?)?,
        held_item_actor: relationship_identity(decode_actor_identity(reader)?)?,
        grabbed_actor: relationship_identity(decode_actor_identity(reader)?)?,
        thrown_boomerang_actor: relationship_identity(decode_actor_identity(reader)?)?,
        copy_rod_actor: relationship_identity(decode_actor_identity(reader)?)?,
        hookshot_roof_wait_actor: relationship_identity(decode_actor_identity(reader)?)?,
        chain_grab_actor: relationship_identity(decode_actor_identity(reader)?)?,
        attention_hint_actor: relationship_identity(decode_actor_identity(reader)?)?,
        attention_catch_actor: relationship_identity(decode_actor_identity(reader)?)?,
        attention_look_actor: relationship_identity(decode_actor_identity(reader)?)?,
    };
    if status != NativeChannelStatus::Present
        && player_relationship_identities(&relationships).any(Option::is_some)
    {
        return Err(NativeEpisodeShardError::new(
            "player-relationship payload is present for an unavailable channel",
        ));
    }
    Ok((status, relationships))
}

fn player_relationship_identities(
    relationships: &NativePlayerRelationshipsObservation,
) -> impl Iterator<Item = &Option<NativeActorIdentity>> {
    [
        &relationships.targeted_actor,
        &relationships.ride_actor,
        &relationships.held_item_actor,
        &relationships.grabbed_actor,
        &relationships.thrown_boomerang_actor,
        &relationships.copy_rod_actor,
        &relationships.hookshot_roof_wait_actor,
        &relationships.chain_grab_actor,
        &relationships.attention_hint_actor,
        &relationships.attention_catch_actor,
        &relationships.attention_look_actor,
    ]
    .into_iter()
}

pub(super) fn validate_player_relationship_joins(
    relationships: &NativePlayerRelationshipsObservation,
    actors: &[NativeActorObservation],
) -> Result<(), NativeEpisodeShardError> {
    let actor_generations = actors
        .iter()
        .map(|actor| actor.runtime_generation)
        .collect::<BTreeSet<_>>();
    if player_relationship_identities(relationships)
        .flatten()
        .any(|identity| !actor_generations.contains(&u64::from(identity.runtime_generation)))
    {
        return Err(NativeEpisodeShardError::new(
            "player relationship does not join the complete actor population",
        ));
    }
    Ok(())
}
