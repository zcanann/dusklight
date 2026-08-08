use dusklight_automation_contracts::artifact::Digest;

/// Assign an authenticated state group to a deterministic cross-validation
/// fold without consulting the other groups in the corpus.
///
/// A rank in a sorted collection is not a stable identity: inserting one new
/// state can otherwise move every later state to another fold and rewrite the
/// meaning of an earlier calibration result. The state digest is already a
/// uniformly distributed authenticated identity; FNV-1a mixes all of its
/// bytes so synthetic and production digests follow the same stable rule.
pub(crate) fn stable_group_fold(group: Digest, folds: usize) -> usize {
    assert!(folds > 0, "cross-validation requires at least one fold");
    let hash = group
        .0
        .into_iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    (hash % folds as u64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignment_depends_only_on_group_identity() {
        let existing = [Digest([2; 32]), Digest([7; 32]), Digest([11; 32])];
        let before = existing.map(|group| stable_group_fold(group, 4));

        // This identity would sort before every existing identity. Its
        // insertion must not reassign any previously calibrated state.
        let _inserted = stable_group_fold(Digest([1; 32]), 4);
        let after = existing.map(|group| stable_group_fold(group, 4));

        assert_eq!(before, after);
        assert!(before.iter().all(|fold| *fold < 4));
    }
}
