#include "dusk/automation/suffix_batch.hpp"

#include <cstdlib>
#include <iostream>
#include <string>

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
        "schema":"dusklight-suffix-batch/v1",
        "source_frame":440,
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
    REQUIRE(batch.maximumTicks == 3);
    REQUIRE(batch.verifyStateHashes);
    REQUIRE(batch.candidates.size() == 1);
    REQUIRE(batch.candidates[0].id == "baseline");
    REQUIRE(batch.candidates[0].pads.size() == 3);
    REQUIRE(batch.candidates[0].pads[0].buttons == 256);
    REQUIRE(batch.candidates[0].pads[1].stickX == 126);
    REQUIRE(batch.candidates[0].pads[1] == batch.candidates[0].pads[2]);
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
}

}  // namespace

int main() {
    test_valid_batch_expands_before_the_hot_path();
    test_invalid_batches_fail_closed();
    std::cout << "suffix batch tests passed\n";
    return 0;
}
