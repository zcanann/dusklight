#include "dusk/automation/factorized_pad_policy.hpp"

#include <nlohmann/json.hpp>

#include <cmath>
#include <cstdint>
#include <fstream>
#include <iostream>
#include <limits>
#include <string>
#include <vector>

namespace {

int failures = 0;

void check(const bool condition, const int line, const std::string& message) {
    if (!condition) {
        std::cerr << "factorized_pad_policy_test.cpp:" << line << ": " << message << '\n';
        ++failures;
    }
}

#define CHECK(condition, message) check((condition), __LINE__, (message))

}  // namespace

int main() {
    using namespace dusk::automation;
    std::ifstream input(DUSK_FACTORIZED_PAD_POLICY_GOLDEN_PATH);
    CHECK(input.good(), "shared golden fixture must open");
    if (!input.good()) return 1;
    const nlohmann::json fixture = nlohmann::json::parse(input);
    CHECK(fixture.at("schema") == "dusklight-factorized-pad-policy-golden/v1",
        "fixture schema must match");
    for (const auto& entry : fixture.at("cases")) {
        const auto output = entry.at("output").get<std::vector<float>>();
        FactorizedPadPolicyDecision decision;
        std::string error;
        const FactorizedPadPolicyHeadConfig config{
            .maximumDurationTicks = entry.at("maximum_duration_ticks").get<std::uint32_t>(),
            .buttonLogitThreshold = entry.at("button_logit_threshold").get<float>(),
        };
        CHECK(decode_factorized_pad_policy(config, output, decision, error),
            entry.at("name").get<std::string>() + ": decode failed: " + error);
        const auto& expected = entry.at("expected");
        CHECK(decision.pad.buttons == expected.at("buttons").get<std::uint16_t>(), "buttons");
        CHECK(decision.pad.stickX == expected.at("stick_x").get<std::int8_t>(), "stick_x");
        CHECK(decision.pad.stickY == expected.at("stick_y").get<std::int8_t>(), "stick_y");
        CHECK(decision.pad.substickX == expected.at("substick_x").get<std::int8_t>(),
            "substick_x");
        CHECK(decision.pad.substickY == expected.at("substick_y").get<std::int8_t>(),
            "substick_y");
        CHECK(decision.pad.triggerLeft == expected.at("trigger_left").get<std::uint8_t>(),
            "trigger_left");
        CHECK(decision.pad.triggerRight == expected.at("trigger_right").get<std::uint8_t>(),
            "trigger_right");
        CHECK(decision.pad.analogA == expected.at("analog_a").get<std::uint8_t>(), "analog_a");
        CHECK(decision.pad.analogB == expected.at("analog_b").get<std::uint8_t>(), "analog_b");
        CHECK(decision.durationTicks == expected.at("duration_ticks").get<std::uint32_t>(),
            "duration_ticks");
        CHECK(decision.pad.flags == RawPadFlags::Connected, "controller must be connected");
        CHECK(decision.pad.error == 0, "controller error must be zero");
    }

    FactorizedPadPolicyDecision decision;
    std::string error;
    std::vector<float> invalid(kFactorizedPadPolicyHeadWidth, 0.0F);
    invalid[7] = std::numeric_limits<float>::quiet_NaN();
    CHECK(!decode_factorized_pad_policy({}, invalid, decision, error),
        "nonfinite output must fail closed");
    return failures == 0 ? 0 : 1;
}
