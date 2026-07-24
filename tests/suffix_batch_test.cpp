#include "dusk/automation/suffix_batch.hpp"

#include <array>
#include <cstdlib>
#include <iostream>
#include <string>
#include <utility>

using namespace dusk::automation;

namespace {

void require(const bool condition, const char* expression, const int line) {
    if (!condition) {
        std::cerr << "suffix_batch_test.cpp:" << line
                  << ": check failed: " << expression << '\n';
        std::abort();
    }
}

#define REQUIRE(expression) require((expression), #expression, __LINE__)

std::string valid_batch() {
    return R"({
        "schema":"dusklight-suffix-batch/v3",
        "source_frame":440,
        "source_boundary_fingerprint":"ac7c32788fc3b5c59046386d95b9b5b4",
        "checkpoint_validation":{"kind":"recorded_replay_window","ticks":2},
        "maximum_ticks":3,
        "verify_state_hashes":true,
        "candidates":[
            {"id":"baseline","actions":[
                {"op":"pad_run","pad":{"buttons":256,"stick_x":127,"stick_y":2,
                 "substick_x":0,"substick_y":0,"trigger_left":0,"trigger_right":0,
                 "analog_a":0,"analog_b":0,"connected":true,"error":0},"frames":1},
                {"op":"pad_run","pad":{"buttons":0,"stick_x":126,"stick_y":12,
                 "substick_x":0,"substick_y":0,"trigger_left":0,"trigger_right":0,
                 "analog_a":0,"analog_b":0,"connected":true,"error":0},"frames":2}
            ]}
        ]
    })";
}

void test_valid_batch_expands_before_the_hot_path() {
    SuffixBatchDefinition batch;
    std::string error;
    REQUIRE(parse_suffix_batch(valid_batch(), batch, error));
    REQUIRE(error.empty());
    REQUIRE(batch.sourceFrame == 440);
    REQUIRE(batch.sourceBoundaryFingerprint == "ac7c32788fc3b5c59046386d95b9b5b4");
    REQUIRE(batch.checkpointValidation == SuffixCheckpointValidation::RecordedReplayWindow);
    REQUIRE(batch.validationTicks == 2);
    REQUIRE(batch.maximumTicks == 3);
    REQUIRE(batch.verifyStateHashes);
    REQUIRE(batch.candidates.size() == 1);
    REQUIRE(batch.candidates[0].id == "baseline");
    REQUIRE(batch.candidates[0].pads.size() == 3);
    REQUIRE(batch.candidates[0].pads[0].buttons == 256);
    REQUIRE(batch.candidates[0].pads[1].stickX == 126);
    REQUIRE(batch.candidates[0].pads[1] == batch.candidates[0].pads[2]);
}

void test_tape_passthrough_candidate() {
    const std::string source = R"({
        "schema":"dusklight-suffix-batch/v3","source_frame":440,
        "source_boundary_fingerprint":"ac7c32788fc3b5c59046386d95b9b5b4","maximum_ticks":3,
        "checkpoint_validation":{"kind":"recorded_replay_window","ticks":2},
        "verify_state_hashes":true,
        "candidates":[{"id":"raw-tape","source":"tape"}]
    })";
    SuffixBatchDefinition batch;
    std::string error;
    REQUIRE(parse_suffix_batch(source, batch, error));
    REQUIRE(batch.candidates.size() == 1);
    REQUIRE(batch.candidates[0].tapePassthrough);
    REQUIRE(batch.candidates[0].pads.empty());
}

void test_legacy_fixed_milestone_batch_remains_distinct() {
    const std::string source = R"({
        "schema":"dusklight-suffix-batch/v2","source_frame":440,
        "source_boundary_fingerprint":"ac7c32788fc3b5c59046386d95b9b5b4",
        "maximum_ticks":3,"verify_state_hashes":true,
        "candidates":[{"id":"raw-tape","source":"tape"}]
    })";

    SuffixBatchDefinition batch;
    std::string error;
    REQUIRE(parse_suffix_batch(source, batch, error));
    REQUIRE(batch.checkpointValidation == SuffixCheckpointValidation::GameplayReadyFSp103);
    REQUIRE(batch.validationTicks == 0);
}

std::string factorized_batch() {
    return R"({
        "schema":"dusklight-suffix-batch/v4","source_frame":500,
        "source_boundary_fingerprint":"1f849e432274771426236d60fbf7d72f",
        "checkpoint_validation":{"kind":"recorded_replay_window","ticks":2},
        "maximum_ticks":3,"verify_state_hashes":false,
        "candidates":[{"id":"factorized-online",
          "policy_head":{"schema":"dusklight-factorized-pad-policy-head/v1",
            "maximum_duration_ticks":2,"button_logit_threshold":0.0},
          "policy_outputs":[
            [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
            [1,-1,0.5,-0.5,1,0.5,0.25,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1]
          ]}]
    })";
}

void test_factorized_policy_rows_expand_to_an_online_native_program() {
    SuffixBatchDefinition batch;
    std::string error;
    REQUIRE(parse_suffix_batch(factorized_batch(), batch, error));
    REQUIRE(error.empty());
    REQUIRE(batch.candidates.size() == 1);
    const auto& candidate = batch.candidates[0];
    REQUIRE(candidate.factorizedPolicy);
    REQUIRE(!candidate.tapePassthrough);
    REQUIRE(candidate.policyOutputs.size() == 2);
    REQUIRE(candidate.policyOutputIndexByTick.size() == 3);
    REQUIRE(candidate.policyOutputIndexByTick[0] == 0);
    REQUIRE(candidate.policyOutputIndexByTick[1] == 1);
    REQUIRE(candidate.policyOutputIndexByTick[2] == 1);
    REQUIRE(candidate.pads.size() == 3);
    REQUIRE(candidate.pads[0].stickX == 0);
    REQUIRE(candidate.pads[1].stickX == 127);
    REQUIRE(candidate.pads[1].stickY == -128);
    REQUIRE(candidate.pads[1].substickX == 64);
    REQUIRE(candidate.pads[1].substickY == -64);
    REQUIRE(candidate.pads[1].triggerLeft == 255);
    REQUIRE(candidate.pads[1].triggerRight == 128);
    REQUIRE(candidate.pads[1].analogA == 64);
    REQUIRE(candidate.pads[1].buttons == 1);
    REQUIRE(candidate.pads[1] == candidate.pads[2]);

    std::string oldSchema = factorized_batch();
    const std::size_t schema = oldSchema.find("dusklight-suffix-batch/v4");
    REQUIRE(schema != std::string::npos);
    oldSchema.replace(schema, std::string("dusklight-suffix-batch/v4").size(),
        "dusklight-suffix-batch/v3");
    REQUIRE(!parse_suffix_batch(oldSchema, batch, error));

    std::string wrongDuration = factorized_batch();
    const std::size_t finalRow = wrongDuration.find(",0,1]\n");
    REQUIRE(finalRow != std::string::npos);
    wrongDuration.replace(finalRow, std::string(",0,1]\n").size(), ",0,0]\n");
    REQUIRE(!parse_suffix_batch(wrongDuration, batch, error));
    REQUIRE(error.find("instead of maximum_ticks") != std::string::npos);
}

void test_reactive_controller_executes_as_one_native_candidate() {
    const std::string source = R"({
        "schema":"dusklight-suffix-batch/v8","source_frame":506,
        "source_boundary_fingerprint":"e7ac8251329f22a5df682bbe5eb2a2ba",
        "checkpoint_validation":{"kind":"recorded_replay_window","ticks":8},
        "maximum_ticks":230,"verify_state_hashes":true,
        "candidates":[{"id":"reactive-heading","actions":[],
          "controller_program_hex":"4455534b4354524c0100040020004000e60000000100000040000000000000000200000000000000e60000005327864400004844b1f52bc500000000000000000000000000004842500000000000000000000000000000000000000000000000"}]
    })";
    SuffixBatchDefinition batch;
    std::string error;
    REQUIRE(parse_suffix_batch(source, batch, error));
    REQUIRE(error.empty());
    REQUIRE(batch.candidates.size() == 1);
    const auto& candidate = batch.candidates[0];
    REQUIRE(candidate.controllerProgram);
    REQUIRE(candidate.controllerStartTick == 0);
    REQUIRE(candidate.controller.duration() == 230);
    REQUIRE(candidate.pads.empty());

    std::string legacy = source;
    const std::size_t schema = legacy.find("dusklight-suffix-batch/v8");
    REQUIRE(schema != std::string::npos);
    legacy.replace(schema, std::string("dusklight-suffix-batch/v8").size(),
        "dusklight-suffix-batch/v3");
    REQUIRE(!parse_suffix_batch(legacy, batch, error));

    std::string detached = source;
    const std::size_t duration = detached.find("\"maximum_ticks\":230");
    REQUIRE(duration != std::string::npos);
    detached.replace(duration, std::string("\"maximum_ticks\":230").size(),
        "\"maximum_ticks\":229");
    REQUIRE(!parse_suffix_batch(detached, batch, error));
}

std::string frozen_policy_batch() {
    return R"({
        "schema":"dusklight-suffix-batch/v7",
        "demonstration_mode":"behavior_cloning_warm_start",
        "action_authority":"episode_policy","source_frame":500,
        "source_boundary_fingerprint":"1f849e432274771426236d60fbf7d72f",
        "checkpoint_validation":{"kind":"recorded_replay_window","ticks":2},
        "maximum_ticks":3,"verify_state_hashes":false,
        "frozen_policy":{
          "schema":"dusklight-native-frozen-policy/v2",
          "model_path":"build/learning/policy.dsfrozen",
          "model_xxh3_128":"0123456789abcdef0123456789abcdef",
          "policy_head":{"schema":"dusklight-factorized-pad-policy-head/v1",
            "maximum_duration_ticks":1,"button_logit_threshold":0.0},
          "rollout_exploration":{
            "schema":"dusklight-native-policy-rollout-exploration/v1",
            "seed":81985529216486895,
            "stick_axis_delta_probability_millionths":125000,
            "maximum_stick_axis_delta":32,
            "button_flip_probability_millionths":2000,
            "button_flip_mask":3967}},
        "candidates":[{"id":"native-policy","source":"frozen_policy"}]
    })";
}

void test_frozen_policy_is_content_bound_and_one_tick() {
    SuffixBatchDefinition batch;
    std::string error;
    REQUIRE(parse_suffix_batch(frozen_policy_batch(), batch, error));
    REQUIRE(error.empty());
    REQUIRE(batch.frozenPolicy.has_value());
    REQUIRE(batch.demonstrationMode == SuffixDemonstrationMode::BehaviorCloningWarmStart);
    REQUIRE(batch.policyActionAuthority == SuffixPolicyActionAuthority::EpisodePolicy);
    REQUIRE(batch.frozenPolicy->modelPath == "build/learning/policy.dsfrozen");
    REQUIRE(batch.frozenPolicy->modelXxh3_128 == "0123456789abcdef0123456789abcdef");
    REQUIRE(batch.frozenPolicy->policyHead.maximumDurationTicks == 1);
    REQUIRE(batch.frozenPolicy->rolloutExploration.has_value());
    REQUIRE(batch.frozenPolicy->rolloutExploration->seed == 81985529216486895ULL);
    REQUIRE(batch.candidates.size() == 1);
    REQUIRE(batch.candidates[0].frozenPolicy);
    REQUIRE(batch.candidates[0].pads.empty());

    RawPadState explored;
    explored.stickX = 120;
    explored.stickY = -120;
    apply_policy_rollout_exploration(
        explored, *batch.frozenPolicy->rolloutExploration, 7);
    REQUIRE(explored.stickX == 120);
    REQUIRE(explored.stickY == -128);
    REQUIRE(explored.flags == RawPadFlags::Connected);
    REQUIRE(explored.error == 0);
    RawPadState buttonExploration;
    apply_policy_rollout_exploration(
        buttonExploration, *batch.frozenPolicy->rolloutExploration, 26);
    REQUIRE(buttonExploration.buttons == 2048);

    constexpr std::array modes{
        std::pair{"absent", SuffixDemonstrationMode::Absent},
        std::pair{"replay_only", SuffixDemonstrationMode::ReplayOnly},
        std::pair{"behavior_cloning_warm_start",
            SuffixDemonstrationMode::BehaviorCloningWarmStart},
        std::pair{"reverse_curriculum_checkpoints",
            SuffixDemonstrationMode::ReverseCurriculumCheckpoints},
    };
    for (const auto& [name, expected] : modes) {
        std::string treatment = frozen_policy_batch();
        const std::size_t mode = treatment.find("behavior_cloning_warm_start");
        REQUIRE(mode != std::string::npos);
        treatment.replace(mode, std::string("behavior_cloning_warm_start").size(), name);
        REQUIRE(parse_suffix_batch(treatment, batch, error));
        REQUIRE(batch.demonstrationMode == expected);
    }

    std::string invalidMode = frozen_policy_batch();
    const std::size_t mode = invalidMode.find("behavior_cloning_warm_start");
    REQUIRE(mode != std::string::npos);
    invalidMode.replace(mode, std::string("behavior_cloning_warm_start").size(), "uncontrolled");
    REQUIRE(!parse_suffix_batch(invalidMode, batch, error));

    std::string invalidAuthority = frozen_policy_batch();
    const std::size_t authority = invalidAuthority.find("episode_policy");
    REQUIRE(authority != std::string::npos);
    invalidAuthority.replace(authority, std::string("episode_policy").size(), "incumbent_release");
    REQUIRE(!parse_suffix_batch(invalidAuthority, batch, error));

    std::string detached = frozen_policy_batch();
    const std::size_t hash = detached.find("0123456789abcdef0123456789abcdef");
    REQUIRE(hash != std::string::npos);
    detached[hash] = 'A';
    REQUIRE(!parse_suffix_batch(detached, batch, error));

    std::string duration = frozen_policy_batch();
    const std::size_t durationValue = duration.find("\"maximum_duration_ticks\":1");
    REQUIRE(durationValue != std::string::npos);
    duration.replace(durationValue, std::string("\"maximum_duration_ticks\":1").size(),
        "\"maximum_duration_ticks\":2");
    REQUIRE(!parse_suffix_batch(duration, batch, error));

    std::string noConsumer = frozen_policy_batch();
    const std::size_t source = noConsumer.find("\"source\":\"frozen_policy\"");
    REQUIRE(source != std::string::npos);
    noConsumer.replace(source, std::string("\"source\":\"frozen_policy\"").size(),
        "\"source\":\"tape\"");
    REQUIRE(!parse_suffix_batch(noConsumer, batch, error));
}

void test_invalid_batches_fail_closed() {
    SuffixBatchDefinition batch;
    std::string error;

    std::string wrongLength = valid_batch();
    const std::size_t duration = wrongLength.find("\"frames\":2");
    REQUIRE(duration != std::string::npos);
    wrongLength.replace(duration, std::string("\"frames\":2").size(), "\"frames\":1");
    REQUIRE(!parse_suffix_batch(wrongLength, batch, error));
    REQUIRE(error.find("instead of maximum_ticks") != std::string::npos);

    std::string unknownOperation = valid_batch();
    const std::size_t operation = unknownOperation.find("pad_run");
    REQUIRE(operation != std::string::npos);
    unknownOperation.replace(operation, std::string("pad_run").size(), "heading");
    REQUIRE(!parse_suffix_batch(unknownOperation, batch, error));
    REQUIRE(error.find("not an exact pad_run") != std::string::npos);

    std::string unknownField = valid_batch();
    const std::size_t rootEnd = unknownField.rfind('}');
    REQUIRE(rootEnd != std::string::npos);
    unknownField.insert(rootEnd, ",\"surprise\":1");
    REQUIRE(!parse_suffix_batch(unknownField, batch, error));

    std::string wrongFingerprint = valid_batch();
    const std::size_t fingerprint = wrongFingerprint.find(
        "ac7c32788fc3b5c59046386d95b9b5b4");
    REQUIRE(fingerprint != std::string::npos);
    wrongFingerprint.replace(fingerprint, 32, "AC7C32788FC3B5C59046386D95B9B5B4");
    REQUIRE(!parse_suffix_batch(wrongFingerprint, batch, error));

    std::string zeroValidation = valid_batch();
    const std::size_t ticks = zeroValidation.find("\"ticks\":2");
    REQUIRE(ticks != std::string::npos);
    zeroValidation.replace(ticks, std::string("\"ticks\":2").size(), "\"ticks\":0");
    REQUIRE(!parse_suffix_batch(zeroValidation, batch, error));

    std::string excessiveValidation = valid_batch();
    const std::size_t excessiveTicks = excessiveValidation.find("\"ticks\":2");
    REQUIRE(excessiveTicks != std::string::npos);
    excessiveValidation.replace(excessiveTicks, std::string("\"ticks\":2").size(), "\"ticks\":257");
    REQUIRE(!parse_suffix_batch(excessiveValidation, batch, error));

    std::string unknownValidation = valid_batch();
    const std::size_t kind = unknownValidation.find("recorded_replay_window");
    REQUIRE(kind != std::string::npos);
    unknownValidation.replace(kind, std::string("recorded_replay_window").size(), "milestone");
    REQUIRE(!parse_suffix_batch(unknownValidation, batch, error));

    std::string legacyWithValidation = valid_batch();
    const std::size_t schema = legacyWithValidation.find("dusklight-suffix-batch/v3");
    REQUIRE(schema != std::string::npos);
    legacyWithValidation.replace(
        schema, std::string("dusklight-suffix-batch/v3").size(), "dusklight-suffix-batch/v2");
    REQUIRE(!parse_suffix_batch(legacyWithValidation, batch, error));
}

}  // namespace

int main() {
    test_valid_batch_expands_before_the_hot_path();
    test_tape_passthrough_candidate();
    test_legacy_fixed_milestone_batch_remains_distinct();
    test_factorized_policy_rows_expand_to_an_online_native_program();
    test_reactive_controller_executes_as_one_native_candidate();
    test_frozen_policy_is_content_bound_and_one_tick();
    test_invalid_batches_fail_closed();
    std::cout << "suffix batch tests passed\n";
    return 0;
}
