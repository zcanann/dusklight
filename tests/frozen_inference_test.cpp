#include "dusk/automation/frozen_inference.hpp"

#include <algorithm>
#include <array>
#include <bit>
#include <cstdint>
#include <fstream>
#include <iostream>
#include <limits>
#include <string>
#include <vector>

namespace {

int failures = 0;

#define CHECK(condition, message) check((condition), (message), __LINE__)

void check(const bool condition, const char* message, const int line) {
    if (condition) return;
    std::cerr << "frozen_inference_test.cpp:" << line << ": " << message << '\n';
    ++failures;
}

std::vector<std::uint8_t> readGolden() {
    std::ifstream stream(DUSK_FROZEN_INFERENCE_GOLDEN_PATH, std::ios::binary);
    const std::string text((std::istreambuf_iterator<char>(stream)), {});
    std::vector<std::uint8_t> bytes;
    int high = -1;
    for (const unsigned char byte : text) {
        int nibble = -1;
        if (byte >= '0' && byte <= '9') nibble = byte - '0';
        if (byte >= 'a' && byte <= 'f') nibble = 10 + byte - 'a';
        if (byte >= 'A' && byte <= 'F') nibble = 10 + byte - 'A';
        if (nibble < 0) continue;
        if (high < 0) {
            high = nibble;
        } else {
            bytes.push_back(static_cast<std::uint8_t>((high << 4) | nibble));
            high = -1;
        }
    }
    CHECK(stream.good() || stream.eof(), "could not read golden model");
    CHECK(high < 0, "golden model has an incomplete hexadecimal byte");
    return bytes;
}

}  // namespace

int main() {
    using namespace dusk::automation;
    const std::vector<std::uint8_t> golden = readGolden();
    CHECK(golden.size() == 196, "golden model byte count changed");

    FrozenInferenceModel model;
    std::string error;
    CHECK(model.decode(golden, error), "golden model did not decode");
    CHECK(error.empty(), "successful decode retained an error");
    CHECK(model.loaded(), "decoded model is not loaded");
    CHECK(model.inputWidth() == 2, "decoded input width differs");
    CHECK(model.actions().size() == 2 && model.actions()[0] == 7 && model.actions()[1] == 9,
        "decoded action catalog differs");
    CHECK(model.layers().size() == 2, "decoded layer count differs");
    CHECK(model.parameterCount() == 12, "decoded parameter count differs");
    CHECK(model.featureSchemaSha256()[0] == 0x11 && model.featureSchemaSha256()[31] == 0x11,
        "feature identity differs");
    CHECK(model.actionSchemaSha256()[0] == 0x22 && model.objectiveSha256()[0] == 0x33,
        "action or objective identity differs");

    const std::array input{1.5F, -0.5F};
    std::array<float, 2> output{};
    CHECK(model.infer(input, output, error), "golden inference failed");
    CHECK(std::bit_cast<std::uint32_t>(output[0]) == std::bit_cast<std::uint32_t>(6.0F) &&
            std::bit_cast<std::uint32_t>(output[1]) == std::bit_cast<std::uint32_t>(-0.5F),
        "golden inference output differs bitwise");
    CHECK(model.infer(input, output, error), "repeated inference failed");

    std::array<float, 1> wrongOutput{};
    CHECK(!model.infer(input, wrongOutput, error), "wrong output width was accepted");
    std::array badInput{1.5F, std::numeric_limits<float>::infinity()};
    CHECK(!model.infer(badInput, output, error), "non-finite input was accepted");

    auto badMagic = golden;
    badMagic[0] ^= 0xff;
    CHECK(!model.decode(badMagic, error), "bad magic was accepted");
    CHECK(model.infer(input, output, error), "failed decode destroyed the prior valid model");

    auto trailing = golden;
    trailing.push_back(0);
    CHECK(!model.decode(trailing, error), "trailing model data was accepted");

    auto nonfinite = golden;
    nonfinite[140] = 0x00;
    nonfinite[141] = 0x00;
    nonfinite[142] = 0xc0;
    nonfinite[143] = 0x7f;
    CHECK(!model.decode(nonfinite, error), "non-finite parameter was accepted");

    auto detachedCount = golden;
    detachedCount[120] = 11;
    CHECK(!model.decode(detachedCount, error), "detached parameter count was accepted");

    auto zeroIdentity = golden;
    std::fill(zeroIdentity.begin() + 12, zeroIdentity.begin() + 44, 0);
    CHECK(!model.decode(zeroIdentity, error), "zero feature identity was accepted");

    return failures == 0 ? 0 : 1;
}
