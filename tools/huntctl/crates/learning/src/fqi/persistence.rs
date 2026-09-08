//! Validate a restored forest before any tree can index a query vector.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredFittedQ {
    feature_width: usize,
    actions: Vec<u32>,
    forests: Vec<RegressionForest>,
    bootstrap_unit: FqiBootstrapUnit,
    #[serde(default)]
    parameterized_training: Option<ParameterizedTraining>,
}

impl TryFrom<StoredFittedQ> for FittedQ {
    type Error = &'static str;

    fn try_from(stored: StoredFittedQ) -> Result<Self, Self::Error> {
        if stored.feature_width == 0
            || stored.actions.is_empty()
            || stored.actions.len() > MAX_FQI_ACTIONS
            || stored.actions.len() != stored.forests.len()
            || stored.actions.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err("invalid restored forest shape");
        }
        if let Some(training) = &stored.parameterized_training {
            if stored.actions != [0]
                || training.completed_backups == 0
                || !training.discount.is_finite()
                || !(0.0..=1.0).contains(&training.discount)
                || !training.last_max_target_residual.is_finite()
                || training.last_max_target_residual < 0.0
            {
                return Err("invalid restored Bellman training state");
            }
        }
        for forest in &stored.forests {
            if forest.trees.is_empty() || forest.trees.len() > MAX_FQI_TREES_PER_ACTION {
                return Err("invalid restored forest tree count");
            }
            for tree in &forest.trees {
                validate_node(&tree.root, stored.feature_width, 0)?;
            }
        }
        Ok(Self {
            feature_width: stored.feature_width,
            actions: stored.actions,
            forests: stored.forests,
            bootstrap_unit: stored.bootstrap_unit,
            parameterized_training: stored.parameterized_training,
        })
    }
}

fn validate_node(node: &TreeNode, width: usize, depth: usize) -> Result<(), &'static str> {
    if depth > MAX_FQI_TREE_DEPTH {
        return Err("restored tree exceeds depth bound");
    }
    match node {
        TreeNode::Leaf(value) if value.is_finite() => Ok(()),
        TreeNode::Leaf(_) => Err("nonfinite restored tree value"),
        TreeNode::Split {
            feature,
            rule,
            left,
            right,
        } => {
            let (SplitRule::NumericLessOrEqual(value) | SplitRule::CategoricalEqual(value)) = rule;
            if *feature >= width || !value.is_finite() {
                return Err("invalid restored tree split");
            }
            validate_node(left, width, depth + 1)?;
            validate_node(right, width, depth + 1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_forests_are_rejected_before_prediction() {
        let model = FittedQ {
            feature_width: 1,
            actions: vec![0],
            bootstrap_unit: FqiBootstrapUnit::TransitionRow,
            parameterized_training: None,
            forests: vec![RegressionForest {
                trees: vec![RegressionTree {
                    root: TreeNode::Leaf(1.0),
                }],
            }],
        };
        let mut invalid = model.clone();
        invalid.forests[0].trees[0].root = TreeNode::Split {
            feature: 1,
            rule: SplitRule::NumericLessOrEqual(0.0),
            left: Box::new(TreeNode::Leaf(0.0)),
            right: Box::new(TreeNode::Leaf(1.0)),
        };
        assert!(serde_cbor::from_slice::<FittedQ>(&serde_cbor::to_vec(&invalid).unwrap()).is_err());
        invalid.forests[0].trees[0].root = TreeNode::Leaf(f64::NAN);
        assert!(serde_cbor::from_slice::<FittedQ>(&serde_cbor::to_vec(&invalid).unwrap()).is_err());
        invalid = model;
        invalid.forests[0].trees.clear();
        assert!(serde_cbor::from_slice::<FittedQ>(&serde_cbor::to_vec(&invalid).unwrap()).is_err());
    }
}
