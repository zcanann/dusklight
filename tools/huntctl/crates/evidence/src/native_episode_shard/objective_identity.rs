//! Bind authored milestone predicates and definitions to one stable objective.

use super::*;

/// Reproduces the native writer's canonical identity for an exact authored
/// milestone definition. Both SHA-256 digests are part of the domain-separated
/// preimage; changing predicate code or only one definition changes the goal.
pub fn authored_milestone_objective_identity(
    program_sha256: &str,
    definition_sha256: &str,
) -> Result<String, NativeEpisodeShardError> {
    for (label, digest) in [
        ("program SHA-256", program_sha256),
        ("definition SHA-256", definition_sha256),
    ] {
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(NativeEpisodeShardError::new(format!(
                "authored milestone {label} is not canonical lowercase hex"
            )));
        }
    }

    let mut material = Vec::with_capacity(19 + 1 + 64 + 1 + 64);
    material.extend_from_slice(b"authored-milestone");
    material.push(0);
    material.extend_from_slice(program_sha256.as_bytes());
    material.push(0);
    material.extend_from_slice(definition_sha256.as_bytes());
    Ok(format!("{:032x}", xxhash_rust::xxh3::xxh3_128(&material)))
}
