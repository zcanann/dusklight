//! Encode compiled goal semantics without provenance leakage.

use super::*;

/// Fixed semantic embedding. Provenance digests and names are intentionally
/// absent; only typed values, masks, graph roles, roots, and projections enter.
pub fn goal_embedding(goal: &SemanticGoalInput) -> Result<Vec<f64>, NativeGoalReachabilityError> {
    goal.validate()
        .map_err(|error| NativeGoalReachabilityError::new(error.to_string()))?;
    let node_count = goal.node_features.len();
    let mut output = goal
        .metadata
        .iter()
        .map(|value| f64::from(*value))
        .collect::<Vec<_>>();
    output.extend([
        node_count as f64 / f64::from(u16::MAX),
        goal.edges.len() as f64 / f64::from(u16::MAX),
        goal.roots.len() as f64 / f64::from(u16::MAX),
        goal.projection_features.len() as f64 / f64::from(u16::MAX),
    ]);
    append_mean_max(&mut output, &goal.node_features, GOAL_NODE_FEATURE_WIDTH);
    append_mean(
        &mut output,
        &goal.node_feature_masks,
        GOAL_NODE_FEATURE_WIDTH,
    );
    for role in [
        GoalEdgeRole::UnaryChild,
        GoalEdgeRole::LeftChild,
        GoalEdgeRole::RightChild,
    ] {
        let mut messages = vec![vec![0.0_f32; GOAL_NODE_FEATURE_WIDTH]; node_count];
        let mut counts = vec![0_u32; node_count];
        for edge in goal.edges.iter().filter(|edge| edge.role == role) {
            let source = usize::from(edge.source);
            let target = usize::from(edge.target);
            counts[target] += 1;
            for (feature, message) in messages[target].iter_mut().enumerate() {
                *message +=
                    goal.node_features[source][feature] * goal.node_feature_masks[source][feature];
            }
        }
        for (message, count) in messages.iter_mut().zip(counts) {
            if count != 0 {
                for value in message {
                    *value /= count as f32;
                }
            }
        }
        append_mean_max(&mut output, &messages, GOAL_NODE_FEATURE_WIDTH);
    }
    let roots = goal
        .roots
        .iter()
        .map(|root| goal.node_features[usize::from(root.node)].clone())
        .collect::<Vec<_>>();
    append_mean_max(&mut output, &roots, GOAL_NODE_FEATURE_WIDTH);
    append_mean_max(
        &mut output,
        &goal.projection_features,
        GOAL_PROJECTION_FEATURE_WIDTH,
    );
    append_mean(
        &mut output,
        &goal.projection_feature_masks,
        GOAL_PROJECTION_FEATURE_WIDTH,
    );
    if output.is_empty() || output.iter().any(|value| !value.is_finite()) {
        return Err(NativeGoalReachabilityError::new(
            "semantic goal embedding is invalid",
        ));
    }
    Ok(output)
}

pub(super) fn append_mean_max(output: &mut Vec<f64>, rows: &[Vec<f32>], width: usize) {
    append_mean(output, rows, width);
    let mut maximum = vec![f64::NEG_INFINITY; width];
    for row in rows {
        for (maximum, value) in maximum.iter_mut().zip(row) {
            *maximum = maximum.max(f64::from(*value));
        }
    }
    if rows.is_empty() {
        maximum.fill(0.0);
    }
    output.extend(maximum);
}

pub(super) fn append_mean(output: &mut Vec<f64>, rows: &[Vec<f32>], width: usize) {
    let mut mean = vec![0.0; width];
    for row in rows {
        for (mean, value) in mean.iter_mut().zip(row) {
            *mean += f64::from(*value);
        }
    }
    if !rows.is_empty() {
        for value in &mut mean {
            *value /= rows.len() as f64;
        }
    }
    output.extend(mean);
}

pub fn goal_embedding_schema_sha256() -> Digest {
    Digest(Sha256::digest(format!(
        "{NATIVE_GOAL_EMBEDDING_SCHEMA_V1}\nmetadata={GOAL_METADATA_WIDTH}\nnode={GOAL_NODE_FEATURE_WIDTH}\nprojection={GOAL_PROJECTION_FEATURE_WIDTH}\nraw=mean,max,mask_mean\nedges=unary,left,right:masked_source_mean,max\nroots=mean,max\nprojections=mean,max,mask_mean\nprovenance_features=none\n"
    )).into())
}

pub(super) fn reachability_input_schema_sha256(goal_width: usize) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"dusklight.native-goal-reachability-input/v1\0");
    hasher.update(NATIVE_POLICY_FEATURE_SCHEMA_SHA256);
    hasher.update(goal_embedding_schema_sha256().0);
    hasher.update((NATIVE_POLICY_FEATURE_WIDTH as u64).to_le_bytes());
    hasher.update((goal_width as u64).to_le_bytes());
    Digest(hasher.finalize().into())
}

pub(super) fn text_schema_sha256(domain: &[u8], value: &str) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
    Digest(hasher.finalize().into())
}
