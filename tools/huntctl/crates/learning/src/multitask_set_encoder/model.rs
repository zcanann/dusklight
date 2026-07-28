use super::*;

#[derive(Clone, Debug, Serialize)]
pub struct CompleteSetMultiTaskEncoder {
    actor_feature_schema_sha256: Digest,
    layout: FeatureLayout,
    config: TrainableSetConfig,
    pooling: MultiTaskSetPooling,
    temporal: MultiTaskTemporalConfig,
    target_names: Vec<String>,
    target_conditioning: Vec<AuxiliaryHeadConditioning>,
    target_objectives: Vec<AuxiliaryHeadObjective>,
    target_mean: Vec<f64>,
    target_inverse_stddev: Vec<f64>,
    target_positive_weight: Vec<f64>,
    target_negative_weight: Vec<f64>,
    pub(super) target_decision_thresholds: Vec<Option<f64>>,
    pub(super) node_weights: Vec<f64>,
    node_bias: Vec<f64>,
    pub(super) attention_queries: Vec<f64>,
    history_gru: Option<GatedRecurrent>,
    state_weights: Vec<f64>,
    state_bias: Vec<f64>,
    pub(super) output_weights: Vec<f64>,
    output_bias: Vec<f64>,
    optimizer_steps: u64,
}

struct StateForward {
    node_inputs: Vec<Vec<f64>>,
    node_hidden: Vec<Vec<f64>>,
    max_indices: Vec<Option<usize>>,
    attention_weights: Vec<Vec<f64>>,
    attention_pools: Vec<Vec<f64>>,
    state_input: Vec<f64>,
    state_hidden: Vec<f64>,
}

struct ConditionedForward {
    pre: StateForward,
    post: StateForward,
    history: HistoryForward,
    head_inputs: Vec<Vec<f64>>,
    predictions: Vec<f64>,
}

struct HistoryForward {
    states: Vec<StateForward>,
    recurrent_steps: Vec<GatedRecurrentStep>,
    hidden: Vec<f64>,
}

struct EncoderGradients {
    node_weights: Vec<f64>,
    node_bias: Vec<f64>,
    attention_queries: Vec<f64>,
    state_weights: Vec<f64>,
    state_bias: Vec<f64>,
}

impl CompleteSetMultiTaskEncoder {
    #[allow(clippy::too_many_arguments)]
    pub fn fit(
        actor_feature_schema_sha256: Digest,
        training_dataset_sha256: Digest,
        held_out_dataset_sha256: Digest,
        target_names: Vec<String>,
        training: &[MultiTaskSetSample],
        held_out: &[MultiTaskSetSample],
        config: TrainableSetConfig,
    ) -> Result<(MultiTaskSetEncoderReport, Self), TrainableSetError> {
        Self::fit_with_pooling(
            actor_feature_schema_sha256,
            training_dataset_sha256,
            held_out_dataset_sha256,
            target_names,
            training,
            held_out,
            config,
            MultiTaskSetPooling::MeanMax,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fit_with_pooling(
        actor_feature_schema_sha256: Digest,
        training_dataset_sha256: Digest,
        held_out_dataset_sha256: Digest,
        target_names: Vec<String>,
        training: &[MultiTaskSetSample],
        held_out: &[MultiTaskSetSample],
        config: TrainableSetConfig,
        pooling: MultiTaskSetPooling,
    ) -> Result<(MultiTaskSetEncoderReport, Self), TrainableSetError> {
        Self::fit_with_pooling_and_temporal(
            actor_feature_schema_sha256,
            training_dataset_sha256,
            held_out_dataset_sha256,
            target_names,
            training,
            held_out,
            config,
            pooling,
            MultiTaskTemporalConfig::none(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fit_with_pooling_and_temporal(
        actor_feature_schema_sha256: Digest,
        training_dataset_sha256: Digest,
        held_out_dataset_sha256: Digest,
        target_names: Vec<String>,
        training: &[MultiTaskSetSample],
        held_out: &[MultiTaskSetSample],
        config: TrainableSetConfig,
        pooling: MultiTaskSetPooling,
        temporal: MultiTaskTemporalConfig,
    ) -> Result<(MultiTaskSetEncoderReport, Self), TrainableSetError> {
        let dimensions = validate_samples(
            actor_feature_schema_sha256,
            training_dataset_sha256,
            held_out_dataset_sha256,
            &target_names,
            training,
            held_out,
            config,
            temporal,
        )?;
        let layout = FeatureLayout::fit(training.iter().flat_map(sample_model_states), dimensions)?;
        let target_conditioning = target_conditioning_for_names(&target_names);
        let target_objectives = target_objectives_for_names(&target_names);
        let normalization = target_normalization(training, &target_objectives)?;
        let target_support_training = normalization.support.clone();
        let target_support_held_out = target_support(held_out, target_names.len());
        if target_support_held_out.contains(&0) {
            return Err(TrainableSetError::new(
                "each auxiliary target requires held-out support",
            ));
        }
        let mut model = Self::initialized(
            actor_feature_schema_sha256,
            layout,
            config,
            target_names.clone(),
            target_conditioning.clone(),
            target_objectives.clone(),
            normalization.mean,
            normalization.inverse_stddev,
            normalization.positive_weight.clone(),
            normalization.negative_weight.clone(),
            pooling,
            temporal,
        )?;
        let mut order = (0..training.len()).collect::<Vec<_>>();
        let mut rng = DeterministicRng::new(config.seed ^ 0x4d55_4c54_4954_4153);
        for _ in 0..config.epochs {
            rng.shuffle(&mut order);
            for &index in &order {
                model.train_one(&training[index])?;
            }
        }
        model.calibrate_binary_thresholds(held_out)?;
        let model_sha256 = model.model_sha256()?;
        let training_objective_loss = model.objective_loss(training)?;
        let held_out_objective_loss = model.objective_loss(held_out)?;
        let held_out_constant_baseline_objective_loss =
            model.constant_baseline_objective_loss(held_out)?;
        let relative_held_out_improvement = relative_improvement(
            held_out_constant_baseline_objective_loss,
            held_out_objective_loss,
        );
        let heads = model.head_metrics(training, held_out)?;
        let decision = if relative_held_out_improvement >= config.minimum_relative_improvement {
            MultiTaskEncoderDecision::SharedEncoderCandidate
        } else {
            MultiTaskEncoderDecision::RetainTrainingMeanBaseline
        };
        let held_out_rare_events = model.rare_event_metrics(held_out)?;
        let held_out_attention = model.attention_diagnostics(held_out)?;
        let mut report = MultiTaskSetEncoderReport {
            schema: MULTITASK_SET_ENCODER_REPORT_SCHEMA_V12,
            actor_feature_schema_sha256,
            training_dataset_sha256,
            held_out_dataset_sha256,
            config,
            pooling,
            temporal,
            target_names,
            target_conditioning,
            target_objectives,
            target_positive_weights: normalization.positive_weight,
            target_negative_weights: normalization.negative_weight,
            target_decision_thresholds: model.target_decision_thresholds.clone(),
            target_support_training,
            target_support_held_out,
            maximum_training_nodes: training
                .iter()
                .flat_map(sample_model_states)
                .map(|state| state.nodes.len())
                .max()
                .unwrap_or(0),
            maximum_held_out_nodes: held_out
                .iter()
                .flat_map(sample_model_states)
                .map(|state| state.nodes.len())
                .max()
                .unwrap_or(0),
            parameter_count: model.parameter_count(),
            optimizer_steps: model.optimizer_steps,
            training_objective_loss,
            held_out_objective_loss,
            held_out_constant_baseline_objective_loss,
            relative_held_out_improvement,
            heads,
            held_out_rare_events,
            held_out_attention,
            decision,
            model_sha256,
            promotion_authority: false,
            report_sha256: Digest::ZERO,
        };
        report.report_sha256 = report_digest(&report)?;
        Ok((report, model))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn initialized(
        actor_feature_schema_sha256: Digest,
        layout: FeatureLayout,
        config: TrainableSetConfig,
        target_names: Vec<String>,
        target_conditioning: Vec<AuxiliaryHeadConditioning>,
        target_objectives: Vec<AuxiliaryHeadObjective>,
        target_mean: Vec<f64>,
        target_inverse_stddev: Vec<f64>,
        target_positive_weight: Vec<f64>,
        target_negative_weight: Vec<f64>,
        pooling: MultiTaskSetPooling,
        temporal: MultiTaskTemporalConfig,
    ) -> Result<Self, TrainableSetError> {
        temporal.validate()?;
        let target_width = target_names.len();
        let global_attention_heads = pooling.global_attention_heads();
        let attention_heads = pooling.attention_heads(target_width);
        let state_input_width =
            layout.base_input_width + 2 + config.node_hidden_width * (2 + global_attention_heads);
        if target_conditioning.len() != target_width
            || target_objectives.len() != target_width
            || target_mean.len() != target_width
            || target_inverse_stddev.len() != target_width
            || target_positive_weight.len() != target_width
            || target_negative_weight.len() != target_width
        {
            return Err(TrainableSetError::new(
                "multitask target conditioning width is invalid",
            ));
        }
        if target_mean
            .iter()
            .chain(&target_inverse_stddev)
            .chain(&target_positive_weight)
            .chain(&target_negative_weight)
            .any(|value| !value.is_finite())
            || target_inverse_stddev.iter().any(|value| *value <= 0.0)
            || target_positive_weight.iter().any(|value| *value <= 0.0)
            || target_negative_weight.iter().any(|value| *value <= 0.0)
            || target_objectives
                .iter()
                .enumerate()
                .any(|(target, objective)| {
                    *objective == AuxiliaryHeadObjective::ClassBalancedBernoulli
                        && !(0.0..=1.0).contains(&target_mean[target])
                })
        {
            return Err(TrainableSetError::new(
                "multitask target statistics are invalid",
            ));
        }
        let task_attention_width =
            usize::from(pooling.uses_task_attention()) * config.node_hidden_width * 2;
        let head_input_width = config.head_hidden_width * 2
            + ACTION_CONTEXT_WIDTH
            + temporal.hidden_width
            + task_attention_width;
        let recurrent_parameter_count = match temporal.encoding {
            MultiTaskTemporalEncoding::None => 0,
            MultiTaskTemporalEncoding::GatedRecurrent => temporal
                .hidden_width
                .checked_mul(3)
                .and_then(|value| {
                    value.checked_mul(
                        config.head_hidden_width + ACTION_CONTEXT_WIDTH + temporal.hidden_width + 1,
                    )
                })
                .ok_or_else(|| TrainableSetError::new("multitask parameter count overflowed"))?,
        };
        let parameter_count = config
            .node_hidden_width
            .checked_mul(layout.node_input_width + 1)
            .and_then(|value| value.checked_add(attention_heads * config.node_hidden_width))
            .and_then(|value| value.checked_add(config.head_hidden_width * (state_input_width + 1)))
            .and_then(|value| value.checked_add(recurrent_parameter_count))
            .and_then(|value| value.checked_add(target_width * (head_input_width + 1)))
            .ok_or_else(|| TrainableSetError::new("multitask parameter count overflowed"))?;
        if parameter_count > MAX_PARAMETERS {
            return Err(TrainableSetError::new(
                "multitask set encoder exceeds its parameter budget",
            ));
        }
        let mut rng = DeterministicRng::new(config.seed ^ 0x5348_4152_4544_0001);
        let node_weights =
            initialized_weights(config.node_hidden_width, layout.node_input_width, &mut rng);
        let attention_queries =
            initialized_weights(attention_heads, config.node_hidden_width, &mut rng);
        let state_weights =
            initialized_weights(config.head_hidden_width, state_input_width, &mut rng);
        let output_weights = initialized_weights(target_width, head_input_width, &mut rng);
        let history_gru = match temporal.encoding {
            MultiTaskTemporalEncoding::None => None,
            MultiTaskTemporalEncoding::GatedRecurrent => Some(GatedRecurrent::initialized(
                config.head_hidden_width + ACTION_CONTEXT_WIDTH,
                temporal.hidden_width,
                &mut rng,
            )?),
        };
        let target_decision_thresholds = target_objectives
            .iter()
            .map(|objective| {
                (*objective == AuxiliaryHeadObjective::ClassBalancedBernoulli).then_some(0.5)
            })
            .collect();
        let model = Self {
            actor_feature_schema_sha256,
            pooling,
            temporal,
            node_weights,
            node_bias: vec![0.0; config.node_hidden_width],
            attention_queries,
            history_gru,
            state_weights,
            state_bias: vec![0.0; config.head_hidden_width],
            output_weights,
            output_bias: vec![0.0; target_width],
            layout,
            config,
            target_names,
            target_conditioning,
            target_objectives,
            target_mean,
            target_inverse_stddev,
            target_positive_weight,
            target_negative_weight,
            target_decision_thresholds,
            optimizer_steps: 0,
        };
        debug_assert_eq!(model.parameter_count(), parameter_count);
        Ok(model)
    }

    pub fn encode(&self, sample: &TypedSetSample) -> Result<Vec<f32>, TrainableSetError> {
        self.validate_input(sample)?;
        Ok(self
            .state_forward(sample)
            .state_hidden
            .into_iter()
            .map(|value| value as f32)
            .collect())
    }

    pub fn predict(&self, sample: &MultiTaskSetSample) -> Result<Vec<f32>, TrainableSetError> {
        self.validate_transition(sample)?;
        Ok(self
            .conditioned_forward(sample)?
            .predictions
            .iter()
            .enumerate()
            .map(|(target, prediction)| self.prediction_value(target, *prediction) as f32)
            .collect())
    }

    fn prediction_value(&self, target: usize, raw_prediction: f64) -> f64 {
        match self.target_objectives[target] {
            AuxiliaryHeadObjective::NormalizedRegression => {
                raw_prediction / self.target_inverse_stddev[target] + self.target_mean[target]
            }
            AuxiliaryHeadObjective::ClassBalancedBernoulli => calibrated_binary_probability(
                raw_prediction,
                self.target_positive_weight[target],
                self.target_negative_weight[target],
            ),
        }
    }

    pub(super) fn target_loss(&self, target: usize, raw_prediction: f64, expected: f64) -> f64 {
        match self.target_objectives[target] {
            AuxiliaryHeadObjective::NormalizedRegression => {
                let normalized =
                    (expected - self.target_mean[target]) * self.target_inverse_stddev[target];
                (raw_prediction - normalized).powi(2)
            }
            AuxiliaryHeadObjective::ClassBalancedBernoulli => {
                self.binary_weight(target, expected)
                    * binary_cross_entropy_from_logit(raw_prediction, expected)
            }
        }
    }

    pub(super) fn constant_baseline_loss(&self, target: usize, expected: f64) -> f64 {
        match self.target_objectives[target] {
            AuxiliaryHeadObjective::NormalizedRegression => {
                ((expected - self.target_mean[target]) * self.target_inverse_stddev[target]).powi(2)
            }
            AuxiliaryHeadObjective::ClassBalancedBernoulli => {
                self.binary_weight(target, expected)
                    * binary_cross_entropy_from_probability(
                        self.constant_baseline_prediction(target),
                        expected,
                    )
            }
        }
    }

    pub(super) fn constant_baseline_prediction(&self, target: usize) -> f64 {
        match self.target_objectives[target] {
            AuxiliaryHeadObjective::NormalizedRegression => self.target_mean[target],
            AuxiliaryHeadObjective::ClassBalancedBernoulli => {
                let positive_mass = self.target_positive_weight[target] * self.target_mean[target];
                let negative_mass =
                    self.target_negative_weight[target] * (1.0 - self.target_mean[target]);
                positive_mass / (positive_mass + negative_mass)
            }
        }
    }

    fn binary_weight(&self, target: usize, expected: f64) -> f64 {
        if expected > 0.5 {
            self.target_positive_weight[target]
        } else {
            self.target_negative_weight[target]
        }
    }

    fn calibrate_binary_thresholds(
        &mut self,
        samples: &[MultiTaskSetSample],
    ) -> Result<(), TrainableSetError> {
        let mut scored = vec![Vec::new(); self.target_names.len()];
        for sample in samples {
            let predictions = self.predict(sample)?;
            for target in 0..self.target_names.len() {
                if sample.target_present[target]
                    && self.target_objectives[target]
                        == AuxiliaryHeadObjective::ClassBalancedBernoulli
                {
                    scored[target]
                        .push((sample.targets[target] > 0.5, f64::from(predictions[target])));
                }
            }
        }
        for (target, rows) in scored.iter().enumerate() {
            if self.target_objectives[target] == AuxiliaryHeadObjective::ClassBalancedBernoulli {
                self.target_decision_thresholds[target] =
                    Some(select_binary_decision_threshold(rows)?);
            }
        }
        Ok(())
    }

    pub fn model_sha256(&self) -> Result<Digest, TrainableSetError> {
        canonical_digest(b"dusklight.complete-set-multitask-encoder/v9\0", self)
    }

    fn attention_head_count(&self) -> usize {
        self.pooling.attention_heads(self.target_names.len())
    }

    fn head_input_width(&self) -> usize {
        self.config.head_hidden_width * 2
            + ACTION_CONTEXT_WIDTH
            + self.temporal.hidden_width
            + usize::from(self.pooling.uses_task_attention()) * self.config.node_hidden_width * 2
    }

    pub fn parameter_count(&self) -> usize {
        self.node_weights.len()
            + self.node_bias.len()
            + self.attention_queries.len()
            + self
                .history_gru
                .as_ref()
                .map_or(0, GatedRecurrent::parameter_count)
            + self.state_weights.len()
            + self.state_bias.len()
            + self.output_weights.len()
            + self.output_bias.len()
    }

    pub fn evaluate(
        &self,
        samples: &[MultiTaskSetSample],
    ) -> Result<MultiTaskSetEvaluation, TrainableSetError> {
        if samples.is_empty() {
            return Err(TrainableSetError::new(
                "multitask evaluation requires samples",
            ));
        }
        let objective_loss = self.objective_loss(samples)?;
        let constant_baseline_objective_loss = self.constant_baseline_objective_loss(samples)?;
        let mut target_loss = vec![0.0; self.target_names.len()];
        let mut baseline_loss = vec![0.0; self.target_names.len()];
        let mut support = vec![0_usize; self.target_names.len()];
        for sample in samples {
            self.validate_transition(sample)?;
            let raw_predictions = self.conditioned_forward(sample)?.predictions;
            for target in 0..self.target_names.len() {
                if sample.target_present[target] {
                    let expected = f64::from(sample.targets[target]);
                    target_loss[target] +=
                        self.target_loss(target, raw_predictions[target], expected);
                    baseline_loss[target] += self.constant_baseline_loss(target, expected);
                    support[target] += 1;
                }
            }
        }
        let heads = (0..self.target_names.len())
            .map(|target| {
                if support[target] == 0 {
                    return Err(TrainableSetError::new(
                        "multitask evaluation target has no support",
                    ));
                }
                let loss = target_loss[target] / support[target] as f64;
                let constant_baseline_loss = baseline_loss[target] / support[target] as f64;
                Ok(AuxiliaryHeadEvaluation {
                    name: self.target_names[target].clone(),
                    objective: self.target_objectives[target],
                    support: support[target],
                    loss,
                    constant_baseline_loss,
                    relative_improvement: relative_improvement(constant_baseline_loss, loss),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let rare_events = self.rare_event_metrics(samples)?;
        Ok(MultiTaskSetEvaluation {
            samples: samples.len(),
            objective_loss,
            constant_baseline_objective_loss,
            relative_improvement: relative_improvement(
                constant_baseline_objective_loss,
                objective_loss,
            ),
            heads,
            rare_events,
        })
    }

    fn rare_event_metrics(
        &self,
        samples: &[MultiTaskSetSample],
    ) -> Result<Vec<RareEventMetrics>, TrainableSetError> {
        let targets = self
            .target_names
            .iter()
            .enumerate()
            .filter(|(_, name)| rare_event_target(name))
            .map(|(index, name)| {
                self.target_decision_thresholds[index]
                    .map(|threshold| (index, name.clone(), threshold))
                    .ok_or_else(|| {
                        TrainableSetError::new(
                            "rare-event target has no calibrated decision threshold",
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut model = vec![BinaryEventAccumulator::default(); targets.len()];
        let mut baseline = vec![BinaryEventAccumulator::default(); targets.len()];
        for sample in samples {
            let predictions = self.predict(sample)?;
            for (metric, (target, _, threshold)) in targets.iter().enumerate() {
                if sample.target_present[*target] {
                    let expected = sample.targets[*target] > 0.5;
                    model[metric].observe(expected, f64::from(predictions[*target]), *threshold);
                    baseline[metric].observe(expected, self.target_mean[*target], 0.5);
                }
            }
        }
        targets
            .into_iter()
            .enumerate()
            .map(|(metric, (_, name, threshold))| {
                Ok(RareEventMetrics {
                    name,
                    threshold,
                    model: model[metric].finish()?,
                    training_mean_baseline: baseline[metric].finish()?,
                })
            })
            .collect()
    }

    fn attention_diagnostics(
        &self,
        samples: &[MultiTaskSetSample],
    ) -> Result<Vec<AttentionHeadDiagnostics>, TrainableSetError> {
        let heads = self.attention_head_count();
        if heads == 0 {
            return Ok(Vec::new());
        }
        let mut entropy_sum = vec![0.0; heads];
        let mut maximum_sum = vec![0.0; heads];
        let mut support = vec![0_usize; heads];
        for sample in samples {
            self.validate_transition(sample)?;
            let pre = self.state_forward(&sample.input);
            let post = self
                .pooling
                .uses_task_attention()
                .then(|| self.state_forward(&sample.post_input));
            for head in 0..heads {
                if self.pooling.uses_task_attention() && !sample.target_present[head] {
                    continue;
                }
                accumulate_attention_distribution(
                    &pre.attention_weights[head],
                    &mut entropy_sum[head],
                    &mut maximum_sum[head],
                    &mut support[head],
                );
                if self.pooling.uses_task_attention()
                    && self.target_conditioning[head] == AuxiliaryHeadConditioning::PreAndPostState
                {
                    accumulate_attention_distribution(
                        &post
                            .as_ref()
                            .expect("task attention computes post state")
                            .attention_weights[head],
                        &mut entropy_sum[head],
                        &mut maximum_sum[head],
                        &mut support[head],
                    );
                }
            }
        }
        (0..heads)
            .map(|head| {
                if support[head] == 0 {
                    return Err(TrainableSetError::new(
                        "learned attention diagnostics have no actor support",
                    ));
                }
                let query = &self.attention_queries[head * self.config.node_hidden_width
                    ..(head + 1) * self.config.node_hidden_width];
                Ok(AttentionHeadDiagnostics {
                    head,
                    target: self
                        .pooling
                        .uses_task_attention()
                        .then(|| self.target_names[head].clone()),
                    conditioning: self
                        .pooling
                        .uses_task_attention()
                        .then(|| self.target_conditioning[head]),
                    observation_support: support[head],
                    query_l2_norm: query.iter().map(|value| value * value).sum::<f64>().sqrt(),
                    mean_normalized_entropy: entropy_sum[head] / support[head] as f64,
                    mean_maximum_weight: maximum_sum[head] / support[head] as f64,
                })
            })
            .collect()
    }

    fn validate_input(&self, sample: &TypedSetSample) -> Result<(), TrainableSetError> {
        if sample.actor_feature_schema_sha256 != self.actor_feature_schema_sha256 {
            return Err(TrainableSetError::new(
                "multitask sample actor schema does not match model",
            ));
        }
        validate_sample_dimensions(sample, self.layout.dimensions())
    }

    fn validate_transition(&self, sample: &MultiTaskSetSample) -> Result<(), TrainableSetError> {
        self.validate_input(&sample.input)?;
        self.validate_input(&sample.post_input)?;
        if sample.action_context.len() != ACTION_CONTEXT_WIDTH
            || sample.action_context.iter().any(|value| !value.is_finite())
        {
            return Err(TrainableSetError::new(
                "multitask action context is invalid",
            ));
        }
        let history_valid = match self.temporal.encoding {
            MultiTaskTemporalEncoding::None => sample.history.is_empty(),
            MultiTaskTemporalEncoding::GatedRecurrent => {
                sample.history.len() <= self.temporal.history_depth
            }
        };
        let mut history_identities = BTreeSet::new();
        if !history_valid {
            return Err(TrainableSetError::new(
                "multitask history does not match the temporal model",
            ));
        }
        for step in &sample.history {
            if step.transition_sha256 == Digest::ZERO
                || !history_identities.insert(step.transition_sha256)
                || step.action_context.len() != ACTION_CONTEXT_WIDTH
                || step.action_context.iter().any(|value| !value.is_finite())
            {
                return Err(TrainableSetError::new(
                    "multitask history identity or action is invalid",
                ));
            }
            self.validate_input(&step.state)?;
        }
        Ok(())
    }

    fn state_forward(&self, sample: &TypedSetSample) -> StateForward {
        let nodes = ordered_nodes(&sample.nodes);
        let node_inputs = nodes
            .iter()
            .map(|node| self.layout.node_input(node))
            .collect::<Vec<_>>();
        let node_hidden = node_inputs
            .iter()
            .map(|input| {
                dense_tanh(
                    input,
                    &self.node_weights,
                    &self.node_bias,
                    self.config.node_hidden_width,
                )
            })
            .collect::<Vec<_>>();
        let mut mean_pool = vec![0.0; self.config.node_hidden_width];
        let mut max_pool = vec![0.0; self.config.node_hidden_width];
        let mut max_indices = vec![None; self.config.node_hidden_width];
        if !node_hidden.is_empty() {
            max_pool.fill(f64::NEG_INFINITY);
            for (node_index, hidden) in node_hidden.iter().enumerate() {
                for feature in 0..hidden.len() {
                    mean_pool[feature] += hidden[feature];
                    if hidden[feature] > max_pool[feature] {
                        max_pool[feature] = hidden[feature];
                        max_indices[feature] = Some(node_index);
                    }
                }
            }
            for value in &mut mean_pool {
                *value /= node_hidden.len() as f64;
            }
        }
        let attention_heads = self.attention_head_count();
        let mut attention_weights = Vec::with_capacity(attention_heads);
        let mut attention_pools = Vec::with_capacity(attention_heads);
        for head in 0..attention_heads {
            if node_hidden.is_empty() {
                attention_weights.push(Vec::new());
                attention_pools.push(vec![0.0; self.config.node_hidden_width]);
                continue;
            }
            let query = &self.attention_queries
                [head * self.config.node_hidden_width..(head + 1) * self.config.node_hidden_width];
            let logits = node_hidden
                .iter()
                .map(|hidden| dot(hidden, query))
                .collect::<Vec<_>>();
            let maximum = logits.iter().copied().max_by(f64::total_cmp).unwrap_or(0.0);
            let mut weights = logits
                .iter()
                .map(|logit| (logit - maximum).exp())
                .collect::<Vec<_>>();
            let denominator = weights.iter().sum::<f64>();
            for weight in &mut weights {
                *weight /= denominator;
            }
            let mut pool = vec![0.0; self.config.node_hidden_width];
            for (hidden, weight) in node_hidden.iter().zip(&weights) {
                for (pooled, value) in pool.iter_mut().zip(hidden) {
                    *pooled += weight * value;
                }
            }
            attention_weights.push(weights);
            attention_pools.push(pool);
        }
        let mut state_input = self.layout.base_input(sample);
        state_input.push(f64::from(!sample.nodes.is_empty()));
        state_input.push((sample.nodes.len() as f64).ln_1p() / (u16::MAX as f64).ln_1p());
        state_input.extend(mean_pool);
        state_input.extend(max_pool);
        state_input.extend(
            attention_pools[..self.pooling.global_attention_heads()]
                .iter()
                .flatten()
                .copied(),
        );
        let state_hidden = dense_tanh(
            &state_input,
            &self.state_weights,
            &self.state_bias,
            self.config.head_hidden_width,
        );
        StateForward {
            node_inputs,
            node_hidden,
            max_indices,
            attention_weights,
            attention_pools,
            state_input,
            state_hidden,
        }
    }

    fn history_forward(
        &self,
        sample: &MultiTaskSetSample,
    ) -> Result<HistoryForward, TrainableSetError> {
        let Some(recurrent) = &self.history_gru else {
            return Ok(HistoryForward {
                states: Vec::new(),
                recurrent_steps: Vec::new(),
                hidden: Vec::new(),
            });
        };
        let states = sample
            .history
            .iter()
            .map(|step| self.state_forward(&step.state))
            .collect::<Vec<_>>();
        let inputs = states
            .iter()
            .zip(&sample.history)
            .map(|(state, step)| {
                let mut input = Vec::with_capacity(recurrent.input_width());
                input.extend(&state.state_hidden);
                input.extend(step.action_context.iter().map(|value| f64::from(*value)));
                input
            })
            .collect::<Vec<_>>();
        let recurrent_steps = recurrent.forward_sequence(&inputs)?;
        let hidden = recurrent_steps.last().map_or_else(
            || vec![0.0; recurrent.hidden_width()],
            |step| step.hidden.clone(),
        );
        Ok(HistoryForward {
            states,
            recurrent_steps,
            hidden,
        })
    }

    fn conditioned_forward(
        &self,
        sample: &MultiTaskSetSample,
    ) -> Result<ConditionedForward, TrainableSetError> {
        let pre = self.state_forward(&sample.input);
        let post = self.state_forward(&sample.post_input);
        let history = self.history_forward(sample)?;
        let head_input_width = self.head_input_width();
        let head_inputs = self
            .target_conditioning
            .iter()
            .enumerate()
            .map(|(target, conditioning)| {
                let mut input = Vec::with_capacity(head_input_width);
                input.extend(&pre.state_hidden);
                match conditioning {
                    AuxiliaryHeadConditioning::PreStateAndAction => {
                        input.extend(std::iter::repeat_n(0.0, self.config.head_hidden_width));
                        input.extend(sample.action_context.iter().map(|value| f64::from(*value)));
                    }
                    AuxiliaryHeadConditioning::PreAndPostState => {
                        input.extend(&post.state_hidden);
                        input.extend(std::iter::repeat_n(0.0, ACTION_CONTEXT_WIDTH));
                    }
                }
                input.extend(&history.hidden);
                if self.pooling.uses_task_attention() {
                    input.extend(&pre.attention_pools[target]);
                    match conditioning {
                        AuxiliaryHeadConditioning::PreStateAndAction => {
                            input.extend(std::iter::repeat_n(0.0, self.config.node_hidden_width))
                        }
                        AuxiliaryHeadConditioning::PreAndPostState => {
                            input.extend(&post.attention_pools[target]);
                        }
                    }
                }
                input
            })
            .collect::<Vec<_>>();
        let predictions = head_inputs
            .iter()
            .enumerate()
            .map(|(target, input)| {
                dot(
                    input,
                    &self.output_weights
                        [target * head_input_width..(target + 1) * head_input_width],
                ) + self.output_bias[target]
            })
            .collect();
        Ok(ConditionedForward {
            pre,
            post,
            history,
            head_inputs,
            predictions,
        })
    }

    pub(super) fn train_one(
        &mut self,
        sample: &MultiTaskSetSample,
    ) -> Result<(), TrainableSetError> {
        self.validate_transition(sample)?;
        let forward = self.conditioned_forward(sample)?;
        let output_before = self.output_weights.clone();
        let state_before = self.state_weights.clone();
        let attention_before = self.attention_queries.clone();
        let head_input_width = self.head_input_width();
        let present_count = sample
            .target_present
            .iter()
            .filter(|present| **present)
            .count();
        let mut d_outputs = vec![0.0; self.target_names.len()];
        for (target, d_output) in d_outputs.iter_mut().enumerate() {
            if !sample.target_present[target] {
                continue;
            }
            let expected = f64::from(sample.targets[target]);
            let gradient = match self.target_objectives[target] {
                AuxiliaryHeadObjective::NormalizedRegression => {
                    let normalized =
                        (expected - self.target_mean[target]) * self.target_inverse_stddev[target];
                    2.0 * (forward.predictions[target] - normalized)
                }
                AuxiliaryHeadObjective::ClassBalancedBernoulli => {
                    self.binary_weight(target, expected)
                        * (logistic(forward.predictions[target]) - expected)
                }
            };
            *d_output = clip(gradient / present_count as f64, self.config.gradient_clip);
            for input in 0..head_input_width {
                let parameter = target * head_input_width + input;
                let gradient = *d_output * forward.head_inputs[target][input]
                    + self.config.l2_penalty * self.output_weights[parameter];
                self.output_weights[parameter] -=
                    self.config.learning_rate * clip(gradient, self.config.gradient_clip);
            }
            self.output_bias[target] -= self.config.learning_rate * *d_output;
        }
        let mut d_pre_hidden = vec![0.0; self.config.head_hidden_width];
        let mut d_post_hidden = vec![0.0; self.config.head_hidden_width];
        let mut d_history_hidden = vec![0.0; self.temporal.hidden_width];
        let mut d_pre_attention =
            vec![vec![0.0; self.config.node_hidden_width]; self.attention_head_count()];
        let mut d_post_attention =
            vec![vec![0.0; self.config.node_hidden_width]; self.attention_head_count()];
        for hidden in 0..self.config.head_hidden_width {
            for target in 0..self.target_names.len() {
                d_pre_hidden[hidden] +=
                    d_outputs[target] * output_before[target * head_input_width + hidden];
                if self.target_conditioning[target] == AuxiliaryHeadConditioning::PreAndPostState {
                    d_post_hidden[hidden] += d_outputs[target]
                        * output_before
                            [target * head_input_width + self.config.head_hidden_width + hidden];
                }
            }
        }
        let history_offset = self.config.head_hidden_width * 2 + ACTION_CONTEXT_WIDTH;
        for (hidden, gradient) in d_history_hidden.iter_mut().enumerate() {
            for target in 0..self.target_names.len() {
                *gradient += d_outputs[target]
                    * output_before[target * head_input_width + history_offset + hidden];
            }
        }
        if self.pooling.uses_task_attention() {
            let pre_offset = history_offset + self.temporal.hidden_width;
            let post_offset = pre_offset + self.config.node_hidden_width;
            for target in 0..self.target_names.len() {
                for hidden in 0..self.config.node_hidden_width {
                    d_pre_attention[target][hidden] += d_outputs[target]
                        * output_before[target * head_input_width + pre_offset + hidden];
                    if self.target_conditioning[target]
                        == AuxiliaryHeadConditioning::PreAndPostState
                    {
                        d_post_attention[target][hidden] += d_outputs[target]
                            * output_before[target * head_input_width + post_offset + hidden];
                    }
                }
            }
        }
        let mut gradients = EncoderGradients {
            node_weights: vec![0.0; self.node_weights.len()],
            node_bias: vec![0.0; self.node_bias.len()],
            attention_queries: vec![0.0; self.attention_queries.len()],
            state_weights: vec![0.0; self.state_weights.len()],
            state_bias: vec![0.0; self.state_bias.len()],
        };
        self.accumulate_encoder_gradients(
            &forward.pre,
            &d_pre_hidden,
            &d_pre_attention,
            &state_before,
            &attention_before,
            &mut gradients,
        );
        let recurrent_gradients = if let Some(recurrent) = &self.history_gru {
            let (recurrent_gradients, history_input_gradients) =
                recurrent.backward_sequence(&forward.history.recurrent_steps, &d_history_hidden)?;
            if history_input_gradients.len() != forward.history.states.len() {
                return Err(TrainableSetError::new(
                    "multitask recurrent history gradient count is invalid",
                ));
            }
            let no_direct_attention =
                vec![vec![0.0; self.config.node_hidden_width]; self.attention_head_count()];
            for (state, input_gradient) in
                forward.history.states.iter().zip(&history_input_gradients)
            {
                self.accumulate_encoder_gradients(
                    state,
                    &input_gradient[..self.config.head_hidden_width],
                    &no_direct_attention,
                    &state_before,
                    &attention_before,
                    &mut gradients,
                );
            }
            Some(recurrent_gradients)
        } else {
            None
        };
        self.accumulate_encoder_gradients(
            &forward.post,
            &d_post_hidden,
            &d_post_attention,
            &state_before,
            &attention_before,
            &mut gradients,
        );
        for (weight, gradient) in self.state_weights.iter_mut().zip(gradients.state_weights) {
            let gradient = gradient + self.config.l2_penalty * *weight;
            *weight -= self.config.learning_rate * clip(gradient, self.config.gradient_clip);
        }
        for (bias, gradient) in self.state_bias.iter_mut().zip(gradients.state_bias) {
            *bias -= self.config.learning_rate * clip(gradient, self.config.gradient_clip);
        }
        for (weight, gradient) in self.node_weights.iter_mut().zip(gradients.node_weights) {
            let gradient = gradient + self.config.l2_penalty * *weight;
            *weight -= self.config.learning_rate * clip(gradient, self.config.gradient_clip);
        }
        for (bias, gradient) in self.node_bias.iter_mut().zip(gradients.node_bias) {
            *bias -= self.config.learning_rate * clip(gradient, self.config.gradient_clip);
        }
        for (weight, gradient) in self
            .attention_queries
            .iter_mut()
            .zip(gradients.attention_queries)
        {
            let gradient = gradient + self.config.l2_penalty * *weight;
            *weight -= self.config.learning_rate * clip(gradient, self.config.gradient_clip);
        }
        if let (Some(recurrent), Some(recurrent_gradients)) =
            (&mut self.history_gru, recurrent_gradients)
        {
            recurrent.apply_gradients(
                recurrent_gradients,
                self.config.learning_rate,
                self.config.l2_penalty,
                self.config.gradient_clip,
            );
        }
        self.optimizer_steps += 1;
        if self
            .node_weights
            .iter()
            .chain(&self.node_bias)
            .chain(&self.attention_queries)
            .chain(&self.state_weights)
            .chain(&self.state_bias)
            .chain(&self.output_weights)
            .chain(&self.output_bias)
            .any(|value| !value.is_finite())
            || self
                .history_gru
                .as_ref()
                .is_some_and(|recurrent| !recurrent.all_finite())
        {
            return Err(TrainableSetError::new(
                "multitask set encoder parameters became non-finite",
            ));
        }
        Ok(())
    }

    fn accumulate_encoder_gradients(
        &self,
        forward: &StateForward,
        d_hidden: &[f64],
        direct_attention: &[Vec<f64>],
        state_before: &[f64],
        attention_before: &[f64],
        gradients: &mut EncoderGradients,
    ) {
        let d_state_pre = d_hidden
            .iter()
            .zip(&forward.state_hidden)
            .map(|(gradient, hidden)| gradient * (1.0 - hidden.powi(2)))
            .collect::<Vec<_>>();
        let mut d_state_input = vec![0.0; forward.state_input.len()];
        for (output, delta) in d_state_pre.iter().copied().enumerate() {
            for (input, d_input) in d_state_input.iter_mut().enumerate() {
                let parameter = output * forward.state_input.len() + input;
                *d_input += state_before[parameter] * delta;
                gradients.state_weights[parameter] += delta * forward.state_input[input];
            }
            gradients.state_bias[output] += delta;
        }
        let pool_offset = self.layout.base_input_width + 2;
        let d_mean = &d_state_input[pool_offset..pool_offset + self.config.node_hidden_width];
        let d_max = &d_state_input[pool_offset + self.config.node_hidden_width
            ..pool_offset + self.config.node_hidden_width * 2];
        let attention_offset = pool_offset + self.config.node_hidden_width * 2;
        let attention_heads = self.attention_head_count();
        let mut d_attention = vec![vec![0.0; self.config.node_hidden_width]; attention_heads];
        for (head, d_pool) in d_attention
            .iter_mut()
            .take(self.pooling.global_attention_heads())
            .enumerate()
        {
            d_pool.copy_from_slice(
                &d_state_input[attention_offset + head * self.config.node_hidden_width
                    ..attention_offset + (head + 1) * self.config.node_hidden_width],
            );
        }
        for (d_pool, direct) in d_attention.iter_mut().zip(direct_attention) {
            for (gradient, additional) in d_pool.iter_mut().zip(direct) {
                *gradient += additional;
            }
        }
        let node_count = forward.node_hidden.len();
        for node_index in 0..node_count {
            for hidden in 0..self.config.node_hidden_width {
                let mut gradient = d_mean[hidden] / node_count as f64;
                if forward.max_indices[hidden] == Some(node_index) {
                    gradient += d_max[hidden];
                }
                for (head, d_pool) in d_attention.iter().enumerate() {
                    let weight = forward.attention_weights[head][node_index];
                    let score_gradient = weight
                        * d_pool
                            .iter()
                            .enumerate()
                            .map(|(feature, d_value)| {
                                d_value
                                    * (forward.node_hidden[node_index][feature]
                                        - forward.attention_pools[head][feature])
                            })
                            .sum::<f64>();
                    gradient += weight * d_pool[hidden]
                        + score_gradient
                            * attention_before[head * self.config.node_hidden_width + hidden];
                    gradients.attention_queries[head * self.config.node_hidden_width + hidden] +=
                        score_gradient * forward.node_hidden[node_index][hidden];
                }
                let delta = gradient * (1.0 - forward.node_hidden[node_index][hidden].powi(2));
                for input in 0..self.layout.node_input_width {
                    gradients.node_weights[hidden * self.layout.node_input_width + input] +=
                        delta * forward.node_inputs[node_index][input];
                }
                gradients.node_bias[hidden] += delta;
            }
        }
    }

    fn objective_loss(&self, samples: &[MultiTaskSetSample]) -> Result<f64, TrainableSetError> {
        let mut loss = 0.0;
        let mut count = 0_usize;
        for sample in samples {
            self.validate_transition(sample)?;
            let prediction = self.conditioned_forward(sample)?.predictions;
            for (target, predicted) in prediction.iter().enumerate() {
                if sample.target_present[target] {
                    loss += self.target_loss(target, *predicted, f64::from(sample.targets[target]));
                    count += 1;
                }
            }
        }
        Ok(loss / count as f64)
    }

    fn constant_baseline_objective_loss(
        &self,
        samples: &[MultiTaskSetSample],
    ) -> Result<f64, TrainableSetError> {
        let mut loss = 0.0;
        let mut count = 0_usize;
        for sample in samples {
            for target in 0..self.target_names.len() {
                if sample.target_present[target] {
                    loss += self.constant_baseline_loss(target, f64::from(sample.targets[target]));
                    count += 1;
                }
            }
        }
        if count == 0 {
            return Err(TrainableSetError::new(
                "multitask baseline has no supported targets",
            ));
        }
        Ok(loss / count as f64)
    }

    fn head_metrics(
        &self,
        training: &[MultiTaskSetSample],
        held_out: &[MultiTaskSetSample],
    ) -> Result<Vec<AuxiliaryHeadMetrics>, TrainableSetError> {
        let collect = |samples: &[MultiTaskSetSample]| {
            let mut target_loss = vec![0.0; self.target_names.len()];
            let mut baseline_loss = vec![0.0; self.target_names.len()];
            let mut support = vec![0_usize; self.target_names.len()];
            for sample in samples {
                self.validate_transition(sample)?;
                let raw_predictions = self.conditioned_forward(sample)?.predictions;
                for target in 0..self.target_names.len() {
                    if sample.target_present[target] {
                        let expected = f64::from(sample.targets[target]);
                        target_loss[target] +=
                            self.target_loss(target, raw_predictions[target], expected);
                        baseline_loss[target] += self.constant_baseline_loss(target, expected);
                        support[target] += 1;
                    }
                }
            }
            Ok::<_, TrainableSetError>((support, target_loss, baseline_loss))
        };
        let (training_support, training_error, _) = collect(training)?;
        let (held_out_support, held_out_error, held_out_baseline_error) = collect(held_out)?;
        Ok((0..self.target_names.len())
            .map(|target| {
                let training_loss = training_error[target] / training_support[target] as f64;
                let held_out_loss = held_out_error[target] / held_out_support[target] as f64;
                let held_out_constant_baseline_loss =
                    held_out_baseline_error[target] / held_out_support[target] as f64;
                AuxiliaryHeadMetrics {
                    name: self.target_names[target].clone(),
                    objective: self.target_objectives[target],
                    training_support: training_support[target],
                    held_out_support: held_out_support[target],
                    training_loss,
                    held_out_loss,
                    held_out_constant_baseline_loss,
                    relative_held_out_improvement: relative_improvement(
                        held_out_constant_baseline_loss,
                        held_out_loss,
                    ),
                }
            })
            .collect())
    }
}
