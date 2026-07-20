#include "dusk/automation/card_fixture.hpp"
#include "dusk/automation/input_recording.hpp"
#include "dusk/automation/milestones.hpp"
#include "dusk/automation/typed_facts.hpp"

#include <array>
#include <bit>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <iterator>

#include <nlohmann/json.hpp>

namespace {

void require(const bool condition, const char* expression, const int line) {
    if (!condition) {
        std::cerr << "milestones_test.cpp:" << line << ": check failed: " << expression << '\n';
        std::abort();
    }
}

#define REQUIRE(expression) require((expression), #expression, __LINE__)

constexpr std::array<std::uint8_t, 252> IntroProgram{0x44, 0x4d, 0x53, 0x50, 0x01, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0xc8, 0x00, 0x00, 0x00, 0x10, 0xe0, 0x59, 0x2e,
    0xb0, 0x01, 0x14, 0x5a, 0xb8, 0x60, 0x27, 0x1f, 0x1d, 0x66, 0x77, 0x89, 0xaa, 0xce, 0x1e, 0x1a,
    0xa1, 0x44, 0x2b, 0xa7, 0x50, 0xb2, 0x0b, 0xd2, 0x45, 0x38, 0x29, 0xd2, 0x4e, 0x00, 0x00, 0x00,
    0x0c, 0x00, 0x70, 0x72, 0x6f, 0x63, 0x65, 0x73, 0x73, 0x5f, 0x62, 0x6f, 0x6f, 0x74, 0x00, 0x00,
    0x01, 0x00, 0x07, 0x00, 0x16, 0x00, 0x00, 0x00, 0x45, 0xdc, 0x09, 0x9d, 0x98, 0x3f, 0xb5, 0x8b,
    0x7a, 0x85, 0x6f, 0x7e, 0x83, 0x10, 0xe5, 0xd2, 0xec, 0xba, 0xc3, 0xcd, 0xb3, 0xa4, 0xd7, 0xa4,
    0xc0, 0xa2, 0x9e, 0x75, 0x7d, 0x3b, 0xa5, 0xca, 0x01, 0x01, 0x15, 0x04, 0x62, 0x6f, 0x6f, 0x74,
    0x20, 0x01, 0x02, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x20, 0x31, 0x72, 0x00,
    0x00, 0x00, 0x0c, 0x00, 0x6c, 0x69, 0x6e, 0x6b, 0x5f, 0x63, 0x6f, 0x6e, 0x74, 0x72, 0x6f, 0x6c,
    0x01, 0x00, 0x01, 0x00, 0x1c, 0x00, 0x3a, 0x00, 0x00, 0x00, 0x78, 0x84, 0x86, 0x28, 0x9a, 0xa6,
    0x01, 0x35, 0x5d, 0x3a, 0x43, 0x6b, 0x58, 0x25, 0xd2, 0xf2, 0xbd, 0xf3, 0xe2, 0xea, 0x2d, 0x77,
    0x04, 0x8a, 0xcf, 0x5b, 0x19, 0xc0, 0x1d, 0x9d, 0x56, 0x15, 0x01, 0x04, 0x15, 0x07, 0x46, 0x5f,
    0x53, 0x50, 0x31, 0x30, 0x33, 0x20, 0x01, 0x05, 0x13, 0x01, 0x00, 0x00, 0x00, 0x20, 0x31, 0x01,
    0x07, 0x13, 0x01, 0x00, 0x00, 0x00, 0x20, 0x31, 0x01, 0x08, 0x10, 0x01, 0x20, 0x31, 0x01, 0x15,
    0x10, 0x01, 0x20, 0x31, 0x01, 0x0e, 0x10, 0x01, 0x20, 0x30, 0x31, 0x01, 0x0f, 0x13, 0xff, 0xff,
    0xff, 0xff, 0x20, 0x31};

constexpr std::array<std::uint8_t, 233> RichProgram{0x44, 0x4d, 0x53, 0x50, 0x01, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0xb5, 0x00, 0x00, 0x00, 0xc8,
    0x1b, 0x27, 0x33, 0x87, 0x4f, 0x1b, 0xef, 0x98, 0xd7, 0xec, 0x6e, 0xbc, 0x2c, 0x7d,
    0xcc, 0xfc, 0x1e, 0xe8, 0x29, 0x38, 0xf5, 0x53, 0x20, 0x8e, 0xb4, 0xa2, 0x2a, 0xee,
    0x05, 0xfe, 0xfd, 0xb1, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x72, 0x69, 0x63, 0x68, 0x5f,
    0x73, 0x74, 0x61, 0x74, 0x65, 0x01, 0x00, 0x03, 0x00, 0x37, 0x00, 0x7b, 0x00, 0x00,
    0x00, 0xbe, 0xd9, 0xc9, 0xb0, 0x20, 0xca, 0x84, 0x69, 0x6b, 0x68, 0x3b, 0xc0, 0xb1,
    0x66, 0xae, 0xc8, 0x2b, 0xd4, 0x41, 0xd1, 0x0e, 0x20, 0xff, 0x3b, 0x4c, 0x3d, 0xb9,
    0x84, 0x16, 0xdc, 0xd7, 0xb4, 0x01, 0x18, 0x13, 0xfd, 0x00, 0x00, 0x00, 0x20, 0x01,
    0x1a, 0x14, 0x00, 0x00, 0x00, 0x00, 0x23, 0x31, 0x01, 0x22, 0x11, 0x00, 0x04, 0x00,
    0x00, 0x26, 0x31, 0x01, 0x22, 0x11, 0x04, 0x04, 0x00, 0x00, 0x27, 0x31, 0x01, 0x23,
    0x13, 0x00, 0x00, 0x00, 0x00, 0x20, 0x31, 0x01, 0x25, 0x11, 0x04, 0x00, 0x00, 0x00,
    0x23, 0x31, 0x01, 0x26, 0x11, 0x02, 0x00, 0x00, 0x00, 0x20, 0x31, 0x01, 0x27, 0x11,
    0x01, 0x00, 0x00, 0x00, 0x25, 0x31, 0x01, 0x29, 0x10, 0x01, 0x20, 0x31, 0x01, 0x2a,
    0x11, 0x78, 0x56, 0x34, 0x12, 0x20, 0x31, 0x01, 0x2b, 0x13, 0x0b, 0x00, 0x00, 0x00,
    0x20, 0x31, 0x01, 0x32, 0x12, 0xc8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x25,
    0x31, 0x01, 0x33, 0x10, 0x01, 0x20, 0x31, 0x01, 0x3a, 0x14, 0x00, 0x00, 0x00, 0x3f,
    0x23, 0x31};

// Compiled by huntctl from selector_predicate.milestones. Keeping these bytes
// in the native test makes Rust/native wire compatibility an executable check.
constexpr std::array<std::uint8_t, 298> SelectorProgram{
    0x44, 0x4d, 0x53, 0x50, 0x01, 0x00, 0x02, 0x00, 0x01, 0x00, 0x02, 0x00,
    0x01, 0x00, 0x00, 0x00, 0xf6, 0x00, 0x00, 0x00, 0xa1, 0x73, 0xa8, 0xae,
    0x64, 0xd9, 0xda, 0x76, 0x9c, 0xbf, 0xbc, 0x0f, 0x21, 0x5e, 0x6b, 0xe3,
    0x51, 0x35, 0xa3, 0x37, 0x9a, 0x3e, 0x78, 0x28, 0xe8, 0xa9, 0xaa, 0xb6,
    0x33, 0xd8, 0x43, 0xc5, 0xf2, 0x00, 0x00, 0x00, 0x18, 0x00, 0x73, 0x65,
    0x6c, 0x65, 0x63, 0x74, 0x65, 0x64, 0x5f, 0x61, 0x63, 0x74, 0x6f, 0x72,
    0x5f, 0x61, 0x6e, 0x64, 0x5f, 0x66, 0x6c, 0x61, 0x67, 0x73, 0x01, 0x00,
    0x02, 0x00, 0x27, 0x00, 0xae, 0x00, 0x00, 0x00, 0x99, 0xca, 0x32, 0xaf,
    0xed, 0xb2, 0x89, 0x4b, 0x18, 0x76, 0x76, 0x6e, 0xcd, 0x4f, 0x89, 0x3b,
    0x91, 0xa1, 0x7f, 0x72, 0xe1, 0x74, 0x4e, 0x2f, 0x1f, 0x6c, 0xf9, 0x1f,
    0xca, 0xf1, 0x70, 0x5a, 0x02, 0x01, 0x01, 0x46, 0x5f, 0x53, 0x50, 0x31,
    0x30, 0x33, 0x00, 0x00, 0x07, 0x00, 0x2a, 0x00, 0x10, 0x01, 0x20, 0x02,
    0x01, 0x03, 0x46, 0x5f, 0x53, 0x50, 0x31, 0x30, 0x33, 0x00, 0x00, 0x07,
    0x00, 0x2a, 0x00, 0x14, 0x00, 0x00, 0xa0, 0xc1, 0x25, 0x31, 0x02, 0x01,
    0x05, 0x46, 0x5f, 0x53, 0x50, 0x31, 0x30, 0x33, 0x00, 0x00, 0x07, 0x00,
    0x2a, 0x00, 0x14, 0x00, 0x00, 0xa0, 0x40, 0x23, 0x31, 0x02, 0x01, 0x06,
    0x46, 0x5f, 0x53, 0x50, 0x31, 0x30, 0x33, 0x00, 0x00, 0x07, 0x00, 0x2a,
    0x00, 0x13, 0x00, 0x00, 0x00, 0x00, 0x20, 0x31, 0x02, 0x01, 0x07, 0x46,
    0x5f, 0x53, 0x50, 0x31, 0x30, 0x33, 0x00, 0x00, 0x07, 0x00, 0x2a, 0x00,
    0x13, 0x00, 0x00, 0x00, 0x00, 0x24, 0x31, 0x02, 0x01, 0x08, 0x46, 0x5f,
    0x53, 0x50, 0x31, 0x30, 0x33, 0x00, 0x00, 0x07, 0x00, 0x2a, 0x00, 0x11,
    0x04, 0x00, 0x00, 0x00, 0x26, 0x31, 0x02, 0x02, 0x00, 0xff, 0x35, 0x03,
    0x10, 0x01, 0x20, 0x31, 0x02, 0x02, 0x01, 0xff, 0xb8, 0x00, 0x10, 0x00,
    0x20, 0x31, 0x02, 0x02, 0x02, 0xff, 0x3f, 0x00, 0x10, 0x01, 0x20, 0x31,
    0x02, 0x02, 0x03, 0x00, 0xef, 0x00, 0x10, 0x01, 0x20, 0x31};

constexpr std::array<std::uint8_t, 227> SpatialSequenceProgram{
    0x44, 0x4d, 0x53, 0x50, 0x01, 0x00, 0x03, 0x00, 0x01, 0x00, 0x03, 0x00,
    0x01, 0x00, 0x00, 0x00, 0xaf, 0x00, 0x00, 0x00, 0x3d, 0x38, 0xe1, 0x66,
    0xac, 0xa5, 0x7d, 0xba, 0xfc, 0x50, 0x06, 0x09, 0x75, 0xec, 0xff, 0x9a,
    0xb7, 0x98, 0x1d, 0x59, 0xc6, 0x7b, 0x6f, 0x74, 0x11, 0xde, 0xd8, 0xc5,
    0x9c, 0xc6, 0x19, 0x22, 0xab, 0x00, 0x00, 0x00, 0x1b, 0x00, 0x67, 0x72,
    0x6f, 0x75, 0x6e, 0x64, 0x65, 0x64, 0x5f, 0x72, 0x65, 0x67, 0x69, 0x6f,
    0x6e, 0x5f, 0x65, 0x76, 0x65, 0x6e, 0x74, 0x5f, 0x70, 0x6c, 0x61, 0x6e,
    0x65, 0x01, 0x00, 0x01, 0x00, 0x19, 0x00, 0x64, 0x00, 0x00, 0x00, 0x55,
    0x63, 0xfb, 0x62, 0x58, 0xf1, 0x60, 0xcc, 0xf5, 0x94, 0x2e, 0xca, 0xc4,
    0xe4, 0x6a, 0x9a, 0xa9, 0x09, 0x9c, 0xb1, 0x54, 0xe5, 0xde, 0x65, 0xa5,
    0xd3, 0xad, 0xa8, 0x58, 0x69, 0x78, 0xc2, 0x40, 0x03, 0x00, 0x04, 0x01,
    0x33, 0x10, 0x01, 0x20, 0x01, 0x09, 0x14, 0x00, 0x00, 0x25, 0x44, 0x25,
    0x01, 0x09, 0x14, 0x00, 0x80, 0x27, 0x44, 0x23, 0x31, 0x31, 0x41, 0x02,
    0x03, 0x00, 0x00, 0x25, 0x44, 0x00, 0x80, 0x45, 0x44, 0x00, 0x80, 0x18,
    0xc5, 0x00, 0x80, 0x27, 0x44, 0x00, 0x80, 0x4a, 0x44, 0x00, 0x40, 0x17,
    0xc5, 0x10, 0x01, 0x20, 0x41, 0x01, 0x0f, 0x13, 0x11, 0x00, 0x00, 0x00,
    0x20, 0x41, 0x02, 0x04, 0x00, 0x80, 0x26, 0x44, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00, 0x00, 0x25, 0x41};

constexpr std::array<std::uint8_t, 181> ValueProjectionProgram{
    0x44, 0x4d, 0x53, 0x50, 0x01, 0x00, 0x04, 0x00, 0x01, 0x00, 0x04, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x81, 0x00, 0x00, 0x00, 0xad, 0x68, 0x93, 0xe0,
    0xff, 0x3a, 0x6f, 0xd7, 0xc3, 0x9f, 0x9a, 0xd2, 0x1b, 0x6e, 0xcc, 0xe5,
    0x08, 0x99, 0x42, 0x79, 0xcb, 0x41, 0xfe, 0x33, 0x94, 0x4c, 0xca, 0x6b,
    0xb6, 0x1a, 0x15, 0xc8, 0x7d, 0x00, 0x00, 0x00, 0x14, 0x00, 0x76, 0x61,
    0x6c, 0x75, 0x65, 0x2d, 0x70, 0x61, 0x72, 0x69, 0x74, 0x79, 0x2d, 0x66,
    0x69, 0x78, 0x74, 0x75, 0x72, 0x65, 0x01, 0x00, 0x01, 0x00, 0x0d, 0x00,
    0x3d, 0x00, 0x00, 0x00, 0xce, 0x2d, 0xd0, 0x22, 0x9c, 0x88, 0x94, 0x7f,
    0xd2, 0xa2, 0x80, 0x71, 0x8e, 0x6b, 0x49, 0xdc, 0x8f, 0xff, 0x0c, 0xf5,
    0x3b, 0xe8, 0x65, 0x0e, 0xda, 0x40, 0x18, 0xf5, 0x0b, 0xa3, 0xbe, 0x9c,
    0x01, 0x04, 0x15, 0x07, 0x46, 0x5f, 0x53, 0x50, 0x31, 0x30, 0x33, 0x20,
    0x01, 0x05, 0x13, 0x01, 0x00, 0x00, 0x00, 0x20, 0x31, 0x50, 0x0d, 0x68,
    0x61, 0x6e, 0x64, 0x6f, 0x66, 0x66, 0x2d, 0x73, 0x74, 0x61, 0x74, 0x65,
    0x05, 0x51, 0x00, 0x51, 0x01, 0x52, 0x46, 0x5f, 0x53, 0x50, 0x31, 0x30,
    0x33, 0x00, 0x01, 0x53, 0x00, 0xff, 0x35, 0x03, 0x53, 0x03, 0x01, 0xef,
    0x00};

// Compiled by huntctl from temporary_event_bytes.milestones. These exact bytes prove that the
// authoring/compiler side and native read-only evaluator agree on the 256-byte temporary-event
// register query that can later express an observed text-displacement state.
constexpr std::array<std::uint8_t, 164> TemporaryEventByteProgram{
    0x44, 0x4d, 0x53, 0x50, 0x01, 0x00, 0x08, 0x00, 0x01, 0x00, 0x08, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x70, 0x00, 0x00, 0x00, 0xaf, 0xfe, 0x86, 0x1c,
    0x45, 0xaf, 0xde, 0xd3, 0xbb, 0x2b, 0xa8, 0xd7, 0x7f, 0x15, 0x24, 0x6e,
    0xd1, 0xa5, 0x20, 0x35, 0x42, 0x12, 0x45, 0x71, 0xa9, 0x55, 0x43, 0x1e,
    0x64, 0xa6, 0x9f, 0x7f, 0x6c, 0x00, 0x00, 0x00, 0x20, 0x00, 0x74, 0x65,
    0x6d, 0x70, 0x6f, 0x72, 0x61, 0x72, 0x79, 0x5f, 0x65, 0x76, 0x65, 0x6e,
    0x74, 0x5f, 0x72, 0x65, 0x67, 0x69, 0x73, 0x74, 0x65, 0x72, 0x5f, 0x66,
    0x69, 0x78, 0x74, 0x75, 0x72, 0x65, 0x01, 0x00, 0x01, 0x00, 0x0b, 0x00,
    0x20, 0x00, 0x00, 0x00, 0xa7, 0xcf, 0x97, 0xbe, 0xd8, 0xda, 0xe1, 0xa0,
    0x95, 0x60, 0xd5, 0x84, 0xe5, 0x92, 0x59, 0x06, 0x7a, 0x44, 0x40, 0x5d,
    0x4a, 0xc3, 0xf0, 0x03, 0x20, 0x8a, 0xca, 0xa2, 0x7f, 0x08, 0xbf, 0x59,
    0x02, 0x05, 0x00, 0x00, 0x11,
    0x06, 0x00, 0x00, 0x00, 0x26, 0x02, 0x05, 0x01, 0x00, 0x11, 0xa5, 0x00,
    0x00, 0x00, 0x20, 0x31, 0x02, 0x05, 0x05, 0x00, 0x11, 0xc0, 0x00, 0x00,
    0x00, 0x27, 0x31};

bool noSymbols(dusk::automation::MilestoneProgramSymbolKind, std::string_view, std::uint32_t&) {
    return false;
}

std::vector<std::uint8_t> readBytes(const char* path) {
    std::ifstream stream(path, std::ios::binary);
    REQUIRE(stream.good());
    const std::vector<char> bytes{
        std::istreambuf_iterator<char>(stream), std::istreambuf_iterator<char>()};
    return {bytes.begin(), bytes.end()};
}

struct BootStartOrder {
    std::vector<int> events;
    bool recording = false;
};

bool beginBootRecorder(void* context) {
    auto& order = *static_cast<BootStartOrder*>(context);
    order.events.push_back(1);
    order.recording = true;
    return true;
}

void releaseBootInput(void* context) {
    auto& order = *static_cast<BootStartOrder*>(context);
    REQUIRE(order.recording);
    order.events.push_back(2);
}

dusk::automation::MilestoneObservation f_sp103() {
    dusk::automation::MilestoneObservation observation{
        .stageName = "F_SP103",
        .room = 1,
        .layer = -1,
        .point = 1,
        .playerPresent = true,
        .playerIsLink = true,
        .playerProcessId = 0x12345678,
        .playerActorName = 0x00fd,
        .playerProcId = 4,
        .playerPositionX = 666.0f,
        .playerPositionY = 800.0f,
        .playerPositionZ = -2431.0f,
        .playerVelocityX = 1.5f,
        .playerVelocityZ = -2.5f,
        .playerForwardSpeed = 3.0f,
        .playerCurrentAngleY = 100,
        .playerShapeAngleY = 101,
        .eventRunning = false,
        .eventId = -1,
        .eventMapToolId = 0xff,
    };
    observation.rng.streams[0] = {
        .id = dusk::automation::GameRngStreamId::Primary,
        .state0 = 11,
        .state1 = 12,
        .state2 = 13,
        .callCount = 100,
    };
    observation.rng.streams[1] = {
        .id = dusk::automation::GameRngStreamId::Secondary,
        .state0 = 21,
        .state1 = 22,
        .state2 = 23,
        .callCount = 200,
    };
    return observation;
}

void testRegistryAndParsing() {
    using namespace dusk::automation;
    REQUIRE(milestone_definitions().size() == 3);
    REQUIRE(find_milestone("gameplay-ready-f-sp103") != nullptr);
    REQUIRE(find_milestone("not-real") == nullptr);

    std::vector<MilestoneId> parsed;
    std::string error;
    REQUIRE(parse_milestone_list(
        "gameplay-ready-f-sp103,exit-f-sp103-to-f-sp104,gameplay-ready-f-sp103", parsed, error));
    REQUIRE(parsed.size() == 2);
    REQUIRE(parsed[0] == MilestoneId::GameplayReadyFSp103);
    REQUIRE(parsed[1] == MilestoneId::ExitFSp103ToFSp104);
    REQUIRE(!parse_milestone_list("nope", parsed, error));
    REQUIRE(error.find("unknown milestone") != std::string::npos);
}

void testGameplayReadinessRequiresExactMemoryState() {
    using namespace dusk::automation;
    const auto* definition = find_milestone(MilestoneId::GameplayReadyFSp103);
    REQUIRE(definition != nullptr);
    MilestoneObservation observation = f_sp103();
    REQUIRE(definition->predicate(observation));

    observation.eventRunning = true;
    REQUIRE(!definition->predicate(observation));
    observation.eventRunning = false;
    observation.eventId = 2305;  // Pre-opening gap: Link exists but is not durably controllable.
    REQUIRE(!definition->predicate(observation));
    observation.eventId = -1;
    observation.playerIsLink = false;
    REQUIRE(!definition->predicate(observation));
    observation.playerIsLink = true;
    observation.room = 0;
    REQUIRE(!definition->predicate(observation));
    observation.room = 1;
    observation.point = 0;
    REQUIRE(!definition->predicate(observation));
}

void testExitRequiresCommittedExactDestination() {
    using namespace dusk::automation;
    const auto* definition = find_milestone(MilestoneId::ExitFSp103ToFSp104);
    REQUIRE(definition != nullptr);
    MilestoneObservation observation = f_sp103();
    observation.nextStageEnabled = true;
    observation.nextStageName = "F_SP104";
    observation.nextRoom = 1;
    observation.nextPoint = 0;
    REQUIRE(definition->predicate(observation));

    observation.nextStageName = "F_SP103";  // Ordon Village exit is not the goal.
    REQUIRE(!definition->predicate(observation));
    observation.nextStageName = "F_SP104";
    observation.nextPoint = 1;
    REQUIRE(!definition->predicate(observation));
    observation.nextPoint = 0;
    observation.nextStageEnabled = false;
    REQUIRE(!definition->predicate(observation));
}

void testTrackerCapturesOnlyTheFirstHitAndSerializesEvidence() {
    using namespace dusk::automation;
    set_active_automation_card_fixture_identity(
        std::string(EmptyAutomationCardFixtureIdentity));
    MilestoneTracker tracker;
    constexpr MilestoneId requested[]{
        MilestoneId::GameplayReadyFSp103,
        MilestoneId::ExitFSp103ToFSp104,
        MilestoneId::EnteredFSp104,
    };
    std::string error;
    REQUIRE(tracker.configure(requested, MilestoneId::ExitFSp103ToFSp104, error));

    MilestoneObservation observation = f_sp103();
    tracker.observe(observation, 500, 480);
    tracker.observe(observation, 501, 481);
    REQUIRE(tracker.hits()[0].hit);
    REQUIRE(tracker.hits()[0].simulationTick == 500);
    REQUIRE(!tracker.goalReached());

    observation.nextStageEnabled = true;
    observation.nextStageName = "F_SP104";
    observation.nextRoom = 1;
    observation.nextPoint = 0;
    tracker.observe(observation, 700, 680);
    REQUIRE(tracker.goalReached());
    REQUIRE(tracker.hits()[1].simulationTick == 700);
    REQUIRE(tracker.hits()[1].evidence.nextStageName == "F_SP104");

    MilestoneObservation entered{
        .stageName = "F_SP104",
        .room = 1,
        .layer = 1,
        .point = 0,
    };
    tracker.observe(entered, 730, MilestoneNoTapeFrame);

    const auto result = nlohmann::json::parse(serialize_milestone_result(tracker));
    REQUIRE(result["schema"]["name"] == "dusklight.automation.milestones");
    REQUIRE(result["schema"]["version"] == 5);
    REQUIRE(result["boot"]["kind"] == "process");
    REQUIRE(result["boot_origin_established"] == true);
    REQUIRE(result["goal"] == "exit-f-sp103-to-f-sp104");
    REQUIRE(result["goal_reached"] == true);
    REQUIRE(result["milestones"].size() == 3);
    REQUIRE(result["milestones"][0]["sim_tick"] == 500);
    REQUIRE(result["milestones"][1]["evidence"]["next_stage"]["name"] == "F_SP104");
    REQUIRE(result["milestones"][1]["evidence"]["player"]["process_id"] == 0x12345678);
    REQUIRE(result["milestones"][1]["evidence"]["player"]["position"][0] == 666.0f);
    REQUIRE(result["milestones"][1]["evidence"]["event"]["id"] == -1);
    REQUIRE(result["milestones"][1]["evidence"]["event"]["name_fnv1a_present"] == false);
    REQUIRE(result["milestones"][1]["evidence"]["event"]["name_fnv1a"] == nullptr);
    REQUIRE(result["milestones"][1]["evidence"]["rng"]["streams"][0]["state"][0] == 11);
    REQUIRE(result["milestones"][1]["evidence"]["rng"]["streams"][1]["call_count"] == 200);
    REQUIRE(result["milestones"][1]["evidence"]["boundary_fingerprint"]["schema"] ==
            "dusklight.milestone-boundary/v6");
    REQUIRE(result["milestones"][1]["evidence"]["card_fixture_identity"] ==
            std::string(EmptyAutomationCardFixtureIdentity));
    REQUIRE(result["milestones"][1]["evidence"]["boundary_fingerprint"]["algorithm"] == "xxh3-128");
    REQUIRE(result["milestones"][1]["evidence"]["boundary_fingerprint"]["digest"]
                .get<std::string>()
                .size() == 32);
    REQUIRE(result["milestones"][2]["hit"] == true);
    REQUIRE(result["milestones"][2]["tape_frame"] == nullptr);
}

void testBoundaryFingerprintIsStableAndSensitiveToExplicitState() {
    using namespace dusk::automation;
    set_active_automation_card_fixture_identity(
        std::string(EmptyAutomationCardFixtureIdentity));
    MilestoneTracker tracker;
    constexpr MilestoneId requested[]{MilestoneId::GameplayReadyFSp103};
    std::string error;
    REQUIRE(tracker.configure(requested, std::nullopt, error));
    tracker.observe(f_sp103(), 10, 10);
    const MilestoneEvidence baseline = tracker.hits()[0].evidence;
    const std::string digest = compute_milestone_boundary_fingerprint(baseline);
    REQUIRE(digest == baseline.boundaryFingerprint);
    REQUIRE(digest.size() == 32);
    REQUIRE(compute_milestone_boundary_fingerprint(baseline) == digest);
    REQUIRE(compute_milestone_boundary_fingerprint(f_sp103(), baseline.boot) == digest);

    MilestoneEvidence changed = baseline;
    changed.cardFixtureIdentity = "card-fixture:xxh3-128:0123456789abcdef0123456789abcdef";
    REQUIRE(compute_milestone_boundary_fingerprint(changed) != digest);
    changed = baseline;
    changed.rng.streams[0].callCount++;
    REQUIRE(compute_milestone_boundary_fingerprint(changed) != digest);
    changed = baseline;
    changed.playerPositionX =
        std::bit_cast<float>(std::bit_cast<std::uint32_t>(changed.playerPositionX) + 1);
    REQUIRE(compute_milestone_boundary_fingerprint(changed) != digest);
    changed = baseline;
    changed.playerShapeAngleY++;
    REQUIRE(compute_milestone_boundary_fingerprint(changed) != digest);
    changed = baseline;
    changed.eventStatus++;
    REQUIRE(compute_milestone_boundary_fingerprint(changed) != digest);
    changed = baseline;
    changed.eventNameHashPresent = true;
    changed.eventNameHash = 0;
    REQUIRE(compute_milestone_boundary_fingerprint(changed) != digest);
    changed = baseline;
    changed.nextStageEnabled = true;
    REQUIRE(compute_milestone_boundary_fingerprint(changed) != digest);
    changed = baseline;
    changed.boot = {
        .kind = TapeBootKind::Stage,
        .stage = "F_SP103",
        .room = 1,
        .point = 1,
        .layer = 3,
        .saveSlot = 2,
    };
    REQUIRE(compute_milestone_boundary_fingerprint(changed) != digest);
    const std::string stageDigest = compute_milestone_boundary_fingerprint(changed);
    ScenarioFixture fixture;
    fixture.name = "low-health wolf";
    fixture.form = PlayerFixtureForm::Wolf;
    fixture.health = HealthFixture{4, 20};
    changed.boot.fixture = fixture;
    REQUIRE(compute_milestone_boundary_fingerprint(changed) != stageDigest);

    MilestoneTracker fixtureTracker;
    constexpr MilestoneId fixtureRequested[]{MilestoneId::GameplayReadyFSp103};
    error.clear();
    REQUIRE(fixtureTracker.configure(fixtureRequested, std::nullopt, error));
    fixtureTracker.setBootOrigin(changed.boot);
    fixtureTracker.markBootOriginEstablished();
    fixtureTracker.observe(f_sp103(), 10, 10);
    const auto result = nlohmann::json::parse(serialize_milestone_result(fixtureTracker));
    REQUIRE(result["boot"]["fixture"]["schema"] == kScenarioFixtureSchema);
    REQUIRE(result["boot"]["fixture"]["name"] == "low-health wolf");
    REQUIRE(result["boot"]["fixture"]["form"] == "wolf");
    REQUIRE(result["boot"]["fixture"]["health"]["current"] == 4);
    REQUIRE(result["milestones"][0]["evidence"]["boot"]["fixture"] ==
            result["boot"]["fixture"]);
}

void testObservationFingerprintIsPortableCompleteAndOrderIndependent() {
    using namespace dusk::automation;
    std::array<MilestoneObservation::Actor, 2> actors{{
        {.runtimeGeneration = 9, .actorName = 3, .positionX = 10.0F, .health = 4},
        {.runtimeGeneration = 2, .actorName = 7, .positionZ = -5.0F, .status = 8},
    }};
    std::array<std::uint8_t, 3> eventFlags{0, 1, 0};
    std::array<std::uint8_t, 2> temporaryFlags{1, 0};
    std::array<std::uint8_t, 2> temporaryEventBytes{0x06, 0xa5};
    std::array<std::uint8_t, 1> dungeonFlags{1};
    std::array<std::uint8_t, 2> switchFlags{0, 1};
    MilestoneObservation observation = f_sp103();
    observation.playerModeFlags = 0x1234;
    observation.talkPartner = {
        .present = true,
        .runtimeGeneration = 5,
        .actorName = 11,
        .homePositionPresent = true,
        .homePositionX = 20.0F,
    };
    observation.playerGroundContact = true;
    observation.playerGroundHeightPresent = true;
    observation.playerGroundHeight = 800.0F;
    observation.actors = actors;
    observation.actorObservedCount = 2;
    observation.eventFlags = eventFlags;
    observation.temporaryFlags = temporaryFlags;
    observation.temporaryEventBytes = temporaryEventBytes;
    observation.dungeonFlags = dungeonFlags;
    observation.switchFlags = switchFlags;
    observation.switchFlagRoom = 1;
    observation.flagsPresent = true;
    const TapeBoot boot;
    const std::string digest = compute_milestone_observation_fingerprint(observation, boot);
    REQUIRE(digest.size() == 32);
    REQUIRE(compute_milestone_observation_fingerprint(observation, boot) == digest);

    std::array<MilestoneObservation::Actor, 2> reversedActors{actors[1], actors[0]};
    MilestoneObservation changed = observation;
    changed.actors = reversedActors;
    REQUIRE(compute_milestone_observation_fingerprint(changed, boot) == digest);
    changed = observation;
    changed.playerPositionX += 1.0F;
    REQUIRE(compute_milestone_observation_fingerprint(changed, boot) != digest);
    changed = observation;
    changed.playerModeFlags++;
    REQUIRE(compute_milestone_observation_fingerprint(changed, boot) != digest);
    changed = observation;
    changed.playerGroundContact = false;
    REQUIRE(compute_milestone_observation_fingerprint(changed, boot) != digest);
    changed = observation;
    changed.actorObservedCount = 3;
    changed.actorsTruncated = true;
    REQUIRE(compute_milestone_observation_fingerprint(changed, boot) != digest);
    changed = observation;
    std::array<MilestoneObservation::Actor, 2> changedActorMetadata = actors;
    changedActorMetadata[0].parameters = 0x12345678;
    changed.actors = changedActorMetadata;
    REQUIRE(compute_milestone_observation_fingerprint(changed, boot) != digest);
    changedActorMetadata = actors;
    changedActorMetadata[0].attentionPresent = true;
    changedActorMetadata[0].attention.flags = 2;
    changedActorMetadata[0].attention.positionX = 8.0F;
    changed.actors = changedActorMetadata;
    REQUIRE(compute_milestone_observation_fingerprint(changed, boot) != digest);
    changedActorMetadata = actors;
    changedActorMetadata[0].eventParticipationPresent = true;
    changedActorMetadata[0].eventParticipation.command = 1;
    changed.actors = changedActorMetadata;
    REQUIRE(compute_milestone_observation_fingerprint(changed, boot) != digest);
    actors[0].positionX += 1.0F;
    changed = observation;
    REQUIRE(compute_milestone_observation_fingerprint(changed, boot) != digest);
    actors[0].positionX -= 1.0F;
    eventFlags[0] = 1;
    REQUIRE(compute_milestone_observation_fingerprint(observation, boot) != digest);
    eventFlags[0] = 0;
    temporaryEventBytes[1] ^= 1;
    REQUIRE(compute_milestone_observation_fingerprint(observation, boot) != digest);
    temporaryEventBytes[1] ^= 1;
    TapeBoot changedBoot = boot;
    changedBoot.point = 1;
    REQUIRE(compute_milestone_observation_fingerprint(observation, changedBoot) != digest);
}

void testCheckedStageSmokeFingerprintV4() {
    using namespace dusk::automation;
    MilestoneEvidence evidence{
        .boot = {
            .kind = TapeBootKind::Stage,
            .stage = "F_SP103",
            .room = 1,
            .point = 1,
            .layer = 3,
        },
        .stageName = "F_SP103",
        .room = 1,
        .layer = 3,
        .point = 1,
        .playerPresent = true,
        .playerIsLink = true,
        .playerProcessId = 19,
        .playerActorName = 253,
        .playerProcId = 3,
        .playerPositionX = 666.04052734375f,
        .playerPositionY = 800.0f,
        .playerPositionZ = -2306.737548828125f,
        .playerCurrentAngleZ = 1,
        .eventId = -1,
        .eventMapToolId = 255,
        .nextStageName = "F_SP103",
        .nextRoom = 1,
        .nextLayer = 3,
        .nextPoint = 1,
    };
    evidence.rng.streams[0] = {
        .id = GameRngStreamId::Primary,
        .state0 = 1110,
        .state1 = 24028,
        .state2 = 26313,
        .callCount = 1560,
    };
    evidence.rng.streams[1] = {
        .id = GameRngStreamId::Secondary,
        .state0 = 0,
        .state1 = 0,
        .state2 = 0,
        .callCount = 0,
    };
    REQUIRE(compute_milestone_boundary_fingerprint(evidence) ==
            "0e0a9c07641c6ed6e02c8a28cdf36c68");
}

void testGoalMustBeRequested() {
    using namespace dusk::automation;
    MilestoneTracker tracker;
    constexpr MilestoneId requested[]{MilestoneId::GameplayReadyFSp103};
    std::string error;
    REQUIRE(!tracker.configure(requested, MilestoneId::EnteredFSp104, error));
    REQUIRE(error.find("was not requested") != std::string::npos);
}

void testAuthoredBootStableAndExactFirstHit() {
    using namespace dusk::automation;
    MilestoneProgram program;
    REQUIRE(
        decode_milestone_program(IntroProgram, noSymbols, program) == MilestoneProgramError::None);
    REQUIRE(program.digest() == "10e0592eb001145ab860271f1d667789aace1e1aa1442ba750b20bd2453829d2");
    REQUIRE(program.find("process_boot")->definitionDigest ==
            "45dc099d983fb58b7a856f7e8310e5d2ecbac3cdb3a4d7a4c0a29e757d3ba5ca");

    MilestoneTracker bootTracker;
    const std::vector<std::string> bootNames{"process_boot"};
    std::string error;
    REQUIRE(bootTracker.configureNames(bootNames, std::string("process_boot"), program, error));
    MilestoneObservation empty;
    bootTracker.observeBoundary(empty, MilestoneProgramPhase::PreInput, MilestoneBoundaryKind::Boot,
        0, 0, MilestoneNoTapeFrame);
    REQUIRE(bootTracker.goalReached());
    REQUIRE(bootTracker.authoredHits()[0].boundaryIndex == 0);
    REQUIRE(bootTracker.authoredHits()[0].tapeFrame == MilestoneNoTapeFrame);

    // Stable evaluation records the boundary on which the required run completes, never its start.
    auto* link = const_cast<MilestoneProgramDefinition*>(program.find("link_control"));
    link->stableTicks = 2;
    MilestoneTracker tracker;
    const std::vector<std::string> names{"link_control"};
    REQUIRE(tracker.configureNames(names, std::string("link_control"), program, error));
    MilestoneObservation observation = f_sp103();
    tracker.observeBoundary(
        observation, MilestoneProgramPhase::PostSim, MilestoneBoundaryKind::Tick, 11, 10, 7);
    REQUIRE(!tracker.goalReached());
    observation.eventRunning = true;
    tracker.observeBoundary(
        observation, MilestoneProgramPhase::PostSim, MilestoneBoundaryKind::Tick, 12, 11, 8);
    observation.eventRunning = false;
    tracker.observeBoundary(
        observation, MilestoneProgramPhase::PostSim, MilestoneBoundaryKind::Tick, 13, 12, 9);
    tracker.observeBoundary(
        observation, MilestoneProgramPhase::PostSim, MilestoneBoundaryKind::Tick, 14, 13, 10);
    REQUIRE(tracker.goalReached());
    REQUIRE(tracker.authoredHits()[0].boundaryIndex == 14);
    REQUIRE(tracker.authoredHits()[0].tapeFrame == 10);
    const auto result = nlohmann::json::parse(serialize_milestone_result(tracker));
    REQUIRE(result["program_digest"] == std::string(program.digest()));
    REQUIRE(result["milestones"][0]["definition_digest"] ==
            "788486289aa601355d3a436b5825d2f2bdf3e2ea2d77048acf5b19c01d9d5615");
}

void testAuthoredObjectiveConsumesTypedFacts() {
    using namespace dusk::automation;
    MilestoneProgram program;
    REQUIRE(
        decode_milestone_program(IntroProgram, noSymbols, program) == MilestoneProgramError::None);
    const auto* link = program.find("link_control");
    REQUIRE(link != nullptr);
    const MilestoneObservation observation = f_sp103();
    auto facts = build_typed_fact_response(
        observation, TypedFactPhase::PostSimulation, 12, std::uint64_t{11});
    REQUIRE(link->evaluate(MilestoneProgramContext{.observation = observation, .facts = &facts}));

    for (std::size_t index = 0; index < facts.count; ++index) {
        if (facts.entries[index].id == TypedFactId::EventRunning) {
            facts.entries[index].status = TypedFactStatus::Unavailable;
        }
    }
    REQUIRE(!link->evaluate(MilestoneProgramContext{.observation = observation, .facts = &facts}));
}

void testCheckedInteractionProgramHasExactNativeOfflineFirstHitParity() {
    using namespace dusk::automation;
    std::ifstream fixtureStream(DUSK_INTERACTION_PARITY_FIXTURE_PATH);
    REQUIRE(fixtureStream.good());
    nlohmann::json fixture;
    fixtureStream >> fixture;
    REQUIRE(fixture["schema"] == "dusklight-objective-native-offline-parity/v1");
    REQUIRE(fixture["objective_program"] ==
            "tests/fixtures/automation/objective_interaction_parity.dmsp");

    const auto bytes = readBytes(DUSK_INTERACTION_PARITY_PROGRAM_PATH);
    MilestoneProgram program;
    REQUIRE(decode_milestone_program(bytes, noSymbols, program) == MilestoneProgramError::None);
    REQUIRE(program.digest() ==
            "fa128677db7305715963cbd88f4462836f71c685813192218e61613d7040b0b4");

    std::vector<std::string> requested;
    for (const auto& [name, _] : fixture["expected_first_hits"].items()) {
        requested.push_back(name);
    }
    MilestoneTracker tracker;
    std::string error;
    REQUIRE(tracker.configureNames(requested, std::nullopt, program, error));

    const auto actor = [](const nlohmann::json& value) {
        MilestoneObservation::ActorIdentity identity;
        if (value.is_null()) return identity;
        identity.present = true;
        identity.runtimeGeneration = value["runtime_generation"].get<std::uint32_t>();
        identity.actorName = value["actor_name"].get<std::int16_t>();
        identity.setId = value["set_id"].get<std::uint16_t>();
        identity.homeRoom = value["home_room"].get<std::int8_t>();
        identity.currentRoom = value["current_room"].get<std::int8_t>();
        identity.homePositionPresent = true;
        identity.homePositionX = value["home_position"][0].get<float>();
        identity.homePositionY = value["home_position"][1].get<float>();
        identity.homePositionZ = value["home_position"][2].get<float>();
        return identity;
    };
    for (const auto& boundary : fixture["boundaries"]) {
        const std::string stage = boundary["stage"].get<std::string>();
        const auto& position = boundary["position"];
        MilestoneObservation observation{
            .stageName = stage.c_str(),
            .room = boundary["room"].get<std::int8_t>(),
            .point = boundary["spawn"].get<std::int16_t>(),
            .playerPresent = boundary["player_exists"].get<bool>(),
            .playerIsLink = boundary["player_is_link"].get<bool>(),
            .playerPositionX = position[0].get<float>(),
            .playerPositionY = position[1].get<float>(),
            .playerPositionZ = position[2].get<float>(),
            .playerDoStatus = boundary["do_status"].get<std::uint8_t>(),
            .talkPartner = actor(boundary["talk_partner"]),
            .grabbedActor = actor(boundary["grabbed_actor"]),
            .eventRunning = boundary["event_running"].get<bool>(),
            .eventId = boundary["event_id"].get<std::int16_t>(),
        };
        tracker.observeBoundary(observation, MilestoneProgramPhase::PostSim,
            MilestoneBoundaryKind::Tick, boundary["boundary_index"].get<std::uint64_t>(),
            boundary["simulation_tick"].get<std::uint64_t>(),
            boundary["tape_frame"].get<std::uint64_t>());
    }

    REQUIRE(tracker.authoredHits().size() == fixture["expected_first_hits"].size());
    for (const auto& hit : tracker.authoredHits()) {
        REQUIRE(hit.hit);
        REQUIRE(hit.boundaryIndex == fixture["expected_first_hits"][hit.id]);
        const auto& boundary = fixture["boundaries"][hit.boundaryIndex - 1];
        REQUIRE(hit.simulationTick == boundary["simulation_tick"]);
        REQUIRE(hit.tapeFrame == boundary["tape_frame"]);
    }
}

void testMalformedAuthoredProgramIsRejected() {
    using namespace dusk::automation;
    auto corrupt = IntroProgram;
    corrupt.back() ^= 1;
    MilestoneProgram program;
    REQUIRE(decode_milestone_program(corrupt, noSymbols, program) ==
            MilestoneProgramError::InvalidProgramDigest);
    REQUIRE(program.empty());
}

void testRichV11FactsAndBitMasksEvaluateNatively() {
    using namespace dusk::automation;
    MilestoneProgram program;
    REQUIRE(
        decode_milestone_program(RichProgram, noSymbols, program) == MilestoneProgramError::None);
    REQUIRE(program.digest() == "c81b2733874f1bef98d7ec6ebc2c7dccfc1ee82938f553208eb4a22aee05fefd");
    const MilestoneProgramDefinition* rich = program.find("rich_state");
    REQUIRE(rich != nullptr);
    REQUIRE(rich->stableTicks == 3);

    MilestoneObservation observation = f_sp103();
    observation.playerActorName = 253;
    observation.playerVelocityY = 0.0f;
    observation.playerModeFlags = 0x400;
    observation.playerDamageWaitTimer = 0;
    observation.playerSwordChangeWaitTimer = 4;
    observation.eventMode = 2;
    observation.eventStatus = 1;
    observation.eventNameHashPresent = true;
    observation.eventNameHash = 0x12345678;
    observation.playerGroundContact = true;
    observation.playerGroundHeightPresent = true;
    observation.playerGroundHeight = 799.75f;
    REQUIRE(rich->evaluate(MilestoneProgramContext{.observation = observation}));

    observation.playerModeFlags = 0;
    REQUIRE(!rich->evaluate(MilestoneProgramContext{.observation = observation}));
    observation.playerModeFlags = 0x400;
    observation.eventNameHashPresent = false;
    REQUIRE(!rich->evaluate(MilestoneProgramContext{.observation = observation}));
    observation.eventNameHashPresent = true;
    observation.playerGroundHeight = 799.0f;
    REQUIRE(!rich->evaluate(MilestoneProgramContext{.observation = observation}));
}

void testV12PlacedActorGeometryAndIndexedFlagsEvaluateNatively() {
    using namespace dusk::automation;
    MilestoneProgram program;
    REQUIRE(decode_milestone_program(SelectorProgram, noSymbols, program) ==
            MilestoneProgramError::None);
    REQUIRE(program.digest() ==
            "a173a8ae64d9da769cbfbc0f215e6be35135a3379a3e7828e8a9aab633d843c5");
    const MilestoneProgramDefinition* predicate =
        program.find("selected_actor_and_flags");
    REQUIRE(predicate != nullptr);
    REQUIRE(predicate->stableTicks == 2);

    std::array<MilestoneObservation::Actor, 2> actors{{
        {
            .runtimeGeneration = 10,
            .actorName = 42,
            .setId = 7,
            .homeRoom = 0,
            .currentRoom = 0,
            .positionX = 667.0f,
            .positionY = 800.0f,
            .positionZ = -2431.0f,
            .health = 3,
            .status = 4,
        },
        {
            .runtimeGeneration = 11,
            .actorName = 99,
            .setId = 8,
            .homeRoom = 0,
        },
    }};
    std::array<std::uint8_t, 822> eventFlags{};
    std::array<std::uint8_t, 185> temporaryFlags{};
    std::array<std::uint8_t, 64> dungeonFlags{};
    std::array<std::uint8_t, 240> switchFlags{};
    eventFlags[821] = 1;
    dungeonFlags[63] = 1;
    switchFlags[239] = 1;

    MilestoneObservation observation = f_sp103();
    observation.actors = actors;
    observation.flagsPresent = true;
    observation.eventFlags = eventFlags;
    observation.temporaryFlags = temporaryFlags;
    observation.dungeonFlags = dungeonFlags;
    observation.switchFlags = switchFlags;
    observation.switchFlagRoom = 0;
    REQUIRE(predicate->evaluate(MilestoneProgramContext{.observation = observation}));

    observation.actorsTruncated = true;
    REQUIRE(!predicate->evaluate(MilestoneProgramContext{.observation = observation}));
    observation.actorsTruncated = false;
    actors[1] = actors[0];
    observation.actors = actors;
    REQUIRE(!predicate->evaluate(MilestoneProgramContext{.observation = observation}));
    actors[1].actorName = 99;
    observation.actors = actors;
    eventFlags[821] = 0;
    REQUIRE(!predicate->evaluate(MilestoneProgramContext{.observation = observation}));
    eventFlags[821] = 1;
    observation.switchFlagRoom = 1;
    REQUIRE(!predicate->evaluate(MilestoneProgramContext{.observation = observation}));
    observation.switchFlagRoom = 0;
    actors[0].positionX = 700.0f;
    REQUIRE(!predicate->evaluate(MilestoneProgramContext{.observation = observation}));
}

void testV13SpatialRelationsAndBoundedSequencesEvaluateNatively() {
    using namespace dusk::automation;
    MilestoneProgram program;
    REQUIRE(decode_milestone_program(SpatialSequenceProgram, noSymbols, program) ==
            MilestoneProgramError::None);
    REQUIRE(program.digest() ==
            "3d38e166aca57dbafc50060975ecff9ab7981d59c67b6f7411ded8c59cc61922");
    const auto* definition = program.find("grounded_region_event_plane");
    REQUIRE(definition != nullptr);
    REQUIRE(definition->sequenceStepCount() == 4);
    REQUIRE(definition->sequenceWithinTicks() == 3);

    MilestoneObservation observation = f_sp103();
    observation.playerGroundContact = true;
    REQUIRE(!definition->evaluate(MilestoneProgramContext{.observation = observation}));
    REQUIRE(definition->evaluateSequenceStep(
        0, MilestoneProgramContext{.observation = observation}));
    REQUIRE(definition->evaluateSequenceStep(
        1, MilestoneProgramContext{.observation = observation}));
    observation.eventId = 17;
    REQUIRE(definition->evaluateSequenceStep(
        2, MilestoneProgramContext{.observation = observation}));
    REQUIRE(definition->evaluateSequenceStep(
        3, MilestoneProgramContext{.observation = observation}));

    MilestoneTracker tracker;
    std::string error;
    const std::vector<std::string> requested{"grounded_region_event_plane"};
    REQUIRE(tracker.configureNames(requested, requested[0], program, error));
    observation = f_sp103();
    observation.playerGroundContact = true;
    tracker.observeBoundary(
        observation, MilestoneProgramPhase::PostSim, MilestoneBoundaryKind::Tick, 1, 0, 0);
    REQUIRE(!tracker.goalReached());
    tracker.observeBoundary(
        observation, MilestoneProgramPhase::PostSim, MilestoneBoundaryKind::Tick, 2, 1, 1);
    observation.eventId = 17;
    tracker.observeBoundary(
        observation, MilestoneProgramPhase::PostSim, MilestoneBoundaryKind::Tick, 3, 2, 2);
    REQUIRE(!tracker.goalReached());
    tracker.observeBoundary(
        observation, MilestoneProgramPhase::PostSim, MilestoneBoundaryKind::Tick, 4, 3, 3);
    REQUIRE(tracker.goalReached());
    REQUIRE(tracker.authoredHits()[0].boundaryIndex == 4);

    tracker.reset();
    observation = f_sp103();
    observation.playerGroundContact = true;
    tracker.observeBoundary(
        observation, MilestoneProgramPhase::PostSim, MilestoneBoundaryKind::Tick, 10, 9, 9);
    observation.playerPositionX = 700.0f;
    for (std::uint64_t boundary = 11; boundary <= 13; ++boundary) {
        tracker.observeBoundary(observation, MilestoneProgramPhase::PostSim,
            MilestoneBoundaryKind::Tick, boundary, boundary - 1, boundary - 1);
    }
    observation.playerPositionX = 666.0f;
    tracker.observeBoundary(
        observation, MilestoneProgramPhase::PostSim, MilestoneBoundaryKind::Tick, 14, 13, 13);
    observation.eventId = 17;
    tracker.observeBoundary(
        observation, MilestoneProgramPhase::PostSim, MilestoneBoundaryKind::Tick, 15, 14, 14);
    tracker.observeBoundary(
        observation, MilestoneProgramPhase::PostSim, MilestoneBoundaryKind::Tick, 16, 15, 15);
    REQUIRE(!tracker.goalReached());
}

void testV14NamedValueProjectionsCaptureExactNativeState() {
    using namespace dusk::automation;
    MilestoneProgram program;
    REQUIRE(decode_milestone_program(ValueProjectionProgram, noSymbols, program) ==
            MilestoneProgramError::None);
    REQUIRE(program.digest() ==
            "ad6893e0ff3a6fd7c39f9ad21b6ecce508994279cb41fe33944cca6bb61a15c8");
    const auto* definition = program.find("value-parity-fixture");
    REQUIRE(definition != nullptr);
    REQUIRE(definition->valueProjections().size() == 1);
    REQUIRE(definition->valueProjections()[0].name == "handoff-state");
    REQUIRE(definition->valueProjections()[0].identity ==
            "acb5c9cd5570ebe610e321a3f5a33856a6af7cfaaf808b5b394f471170fcf5f3");
    REQUIRE(definition->valueProjections()[0].items.size() == 5);

    std::array<MilestoneObservation::Actor, 2> actors{{
        {.runtimeGeneration = 200, .actorName = 90, .setId = 8, .homeRoom = 1,
            .currentRoom = 1, .positionX = 2.0f, .health = 4, .status = 5},
        {.runtimeGeneration = 100, .actorName = 42, .setId = 7, .homeRoom = 1,
            .currentRoom = 0, .positionX = 1.0f, .health = 3, .status = 4},
    }};
    std::array<std::uint8_t, 822> eventFlags{};
    std::array<std::uint8_t, 185> temporaryFlags{};
    std::array<std::uint8_t, 64> dungeonFlags{};
    std::array<std::uint8_t, 240> switchFlags{};
    eventFlags[821] = 1;
    switchFlags[239] = 1;
    MilestoneObservation observation = f_sp103();
    observation.actors = actors;
    observation.flagsPresent = true;
    observation.eventFlags = eventFlags;
    observation.temporaryFlags = temporaryFlags;
    observation.dungeonFlags = dungeonFlags;
    observation.switchFlags = switchFlags;
    observation.switchFlagRoom = 1;

    MilestoneTracker tracker;
    std::string error;
    const std::vector<std::string> requested{"value-parity-fixture"};
    REQUIRE(tracker.configureNames(requested, requested[0], program, error));
    tracker.observeBoundary(
        observation, MilestoneProgramPhase::PostSim, MilestoneBoundaryKind::Tick, 1, 0, 0);
    REQUIRE(tracker.goalReached());
    const auto& projection = tracker.authoredHits()[0].projections[0];
    REQUIRE(projection.available);
    REQUIRE(projection.valueDigest.size() == 32);
    REQUIRE(projection.items[2].actors.size() == 2);
    REQUIRE(projection.items[2].actors[0].actorName == 42);
    REQUIRE(projection.items[3].flagValue);
    REQUIRE(projection.items[4].flagValue);

    const auto result = nlohmann::json::parse(serialize_milestone_result(tracker));
    const auto& jsonProjection = result["milestones"][0]["projections"][0];
    REQUIRE(jsonProjection["name"] == "handoff-state");
    REQUIRE(jsonProjection["available"] == true);
    REQUIRE(jsonProjection["value_fingerprint"]["schema"] ==
            "dusklight.value-projection/v1");
    REQUIRE(jsonProjection["values"][2]["value"][0]["actor_name"] == 42);

    tracker.reset();
    observation.actorsTruncated = true;
    tracker.observeBoundary(
        observation, MilestoneProgramPhase::PostSim, MilestoneBoundaryKind::Tick, 1, 0, 0);
    REQUIRE(tracker.authoredHits()[0].hit);
    REQUIRE(!tracker.authoredHits()[0].projections[0].available);
    const auto unavailable = nlohmann::json::parse(serialize_milestone_result(tracker));
    REQUIRE(unavailable["milestones"][0]["projections"][0]["value_fingerprint"] == nullptr);
}

void testV18TemporaryEventBytesEvaluateExactNativeRegisters() {
    using namespace dusk::automation;
    MilestoneProgram program;
    REQUIRE(decode_milestone_program(TemporaryEventByteProgram, noSymbols, program) ==
            MilestoneProgramError::None);
    REQUIRE(program.digest() ==
            "affe861c45afded3bb2ba8d77f15246ed1a5203542124571a955431e64a69f7f");
    const auto* definition = program.find("temporary_event_register_fixture");
    REQUIRE(definition != nullptr);

    MilestoneObservation observation = f_sp103();
    std::array<std::uint8_t, 256> temporaryEventBytes{};
    temporaryEventBytes[0] = 0x06;
    temporaryEventBytes[1] = 0xa5;
    temporaryEventBytes[5] = 0xc0;
    observation.flagsPresent = true;
    observation.temporaryEventBytes = temporaryEventBytes;
    REQUIRE(definition->evaluate(MilestoneProgramContext{.observation = observation}));

    temporaryEventBytes[0] = 0x04;
    REQUIRE(!definition->evaluate(MilestoneProgramContext{.observation = observation}));
    temporaryEventBytes[0] = 0x06;
    temporaryEventBytes[1] = 0xa4;
    REQUIRE(!definition->evaluate(MilestoneProgramContext{.observation = observation}));
    temporaryEventBytes[1] = 0xa5;
    temporaryEventBytes[5] = 0;
    REQUIRE(!definition->evaluate(MilestoneProgramContext{.observation = observation}));
    temporaryEventBytes[5] = 0xc0;
    observation.flagsPresent = false;
    REQUIRE(!definition->evaluate(MilestoneProgramContext{.observation = observation}));
}

void testBootRecordingGuardrailsAndBeginOrdering() {
    using namespace dusk::automation;
    BootRecordingCliRequest request{.enabled = true};
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::MissingOutputTape);
    request.hasOutputTape = true;
    request.hasAutomationInput = true;
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::HasAutomationInput);
    request.hasAutomationInput = false;
    request.headless = true;
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::Headless);
    request.headless = false;
    request.unpaced = true;
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::Unpaced);
    request.unpaced = false;
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::MissingMilestoneProgram);
    request.hasMilestoneProgram = true;
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::MissingMilestoneSelection);
    request.hasMilestoneSelection = true;
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::MissingMilestoneResult);
    request.hasMilestoneResult = true;
    request.hasMilestoneGoal = true;
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::HasMilestoneGoal);
    request.hasMilestoneGoal = false;
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::MissingStartMilestone);
    request.hasStartMilestone = true;
    REQUIRE(validate_boot_recording_cli(request) == BootRecordingError::None);

    MilestoneProgram program;
    REQUIRE(
        decode_milestone_program(IntroProgram, noSymbols, program) == MilestoneProgramError::None);
    REQUIRE(validate_authored_boot_definition(program, "process_boot") == BootRecordingError::None);
    REQUIRE(validate_authored_boot_definition(program, "link_control") ==
            BootRecordingError::StartMilestoneNotPreInput);

    MilestoneTracker tracker;
    const std::vector<std::string> requested{"process_boot"};
    std::string error;
    REQUIRE(tracker.configureNames(requested, std::nullopt, program, error));
    BootStartOrder order;
    BootRecordingBinding binding;
    REQUIRE(begin_authored_boot_recording(tracker, program, "process_boot", "", beginBootRecorder,
                releaseBootInput, &order, binding) == BootRecordingError::StartMilestoneNotHit);
    REQUIRE(order.events.empty());

    tracker.observeBoundary({}, MilestoneProgramPhase::PreInput, MilestoneBoundaryKind::Boot, 0, 0,
        MilestoneNoTapeFrame);
    const std::string fingerprint = tracker.authoredHits()[0].evidence.boundaryFingerprint;
    REQUIRE(begin_authored_boot_recording(tracker, program, "process_boot",
                "00000000000000000000000000000000", beginBootRecorder, releaseBootInput, &order,
                binding) == BootRecordingError::FingerprintMismatch);
    REQUIRE(order.events.empty());
    auto& mutableHit = const_cast<AuthoredMilestoneHit&>(tracker.authoredHits()[0]);
    mutableHit.boundaryIndex = 1;
    REQUIRE(begin_authored_boot_recording(tracker, program, "process_boot", fingerprint,
                beginBootRecorder, releaseBootInput, &order,
                binding) == BootRecordingError::InvalidBootBoundary);
    mutableHit.boundaryIndex = 0;
    mutableHit.programDigest[0] = mutableHit.programDigest[0] == '0' ? '1' : '0';
    REQUIRE(begin_authored_boot_recording(tracker, program, "process_boot", fingerprint,
                beginBootRecorder, releaseBootInput, &order,
                binding) == BootRecordingError::StaleProgram);
    mutableHit.programDigest = std::string(program.digest());
    REQUIRE(begin_authored_boot_recording(tracker, program, "process_boot", fingerprint,
                beginBootRecorder, releaseBootInput, &order, binding) == BootRecordingError::None);
    // The actual first PAD read happens after this gate returns.
    REQUIRE(order.recording);
    order.events.push_back(3);
    REQUIRE(order.events == std::vector<int>({1, 2, 3}));
    REQUIRE(binding.milestone == "process_boot");
    REQUIRE(binding.boundaryIndex == 0);
    REQUIRE(binding.boundaryFingerprint == fingerprint);
    REQUIRE(binding.programDigest == program.digest());
    REQUIRE(binding.definitionDigest == program.find("process_boot")->definitionDigest);
}

void testAcceleratedParentRecordingBoundaryAndRevealOrdering() {
    using namespace dusk::automation;
    REQUIRE(validate_fast_forward_boundary(9, 10, false, true) == FastForwardBoundaryError::None);
    REQUIRE(validate_fast_forward_boundary(10, 10, false, true) ==
            FastForwardBoundaryError::TapeEndRequiresHandoff);
    REQUIRE(validate_fast_forward_boundary(10, 10, true, false) ==
            FastForwardBoundaryError::TapeEndRequiresRelease);
    REQUIRE(validate_fast_forward_boundary(11, 10, true, true) ==
            FastForwardBoundaryError::PastTapeEnd);
    REQUIRE(validate_fast_forward_boundary(10, 10, true, true) == FastForwardBoundaryError::None);
    const ParentRecordingBoundary boundary = exact_parent_recording_boundary(10);
    REQUIRE(boundary.boundaryIndex == 10);
    REQUIRE(boundary.tapeFrame == 9);

    // The exact parent boundary is verified before reveal. Recorder activation and quarantine
    // release deliberately follow the visible host-only countdown, before the first live PAD read.
    std::vector<int> order;
    bool boundaryVerified = false;
    REQUIRE(!accelerated_recording_reveal_ready(true, boundaryVerified));
    boundaryVerified = true;
    order.push_back(1);  // verify exact parent boundary
    REQUIRE(accelerated_recording_reveal_ready(true, boundaryVerified));
    order.push_back(2);  // submit parent frame, reveal, and run host-only countdown
    order.push_back(3);  // recorder.begin() and live-input quarantine release
    order.push_back(4);  // first live PAD read
    REQUIRE(order == std::vector<int>({1, 2, 3, 4}));
    REQUIRE(accelerated_recording_reveal_ready(false, false));

    REQUIRE(recording_handoff_countdown_display_seconds(0) == 0);
    REQUIRE(recording_handoff_countdown_display_seconds(1) == 1);
    REQUIRE(recording_handoff_countdown_display_seconds(999) == 1);
    REQUIRE(recording_handoff_countdown_display_seconds(1000) == 1);
    REQUIRE(recording_handoff_countdown_display_seconds(1001) == 2);
    REQUIRE(recording_handoff_countdown_display_seconds(2999) == 3);
    REQUIRE(recording_handoff_countdown_display_seconds(3000) == 3);
    REQUIRE(recording_handoff_countdown_display_seconds(10001) ==
            RecordingHandoffCountdownMaximumSeconds);
}

void testAuthoredParentRecordingStartBinding() {
    using namespace dusk::automation;
    MilestoneProgram program;
    REQUIRE(
        decode_milestone_program(IntroProgram, noSymbols, program) == MilestoneProgramError::None);
    MilestoneTracker tracker;
    const std::vector<std::string> requested{"link_control"};
    std::string error;
    REQUIRE(tracker.configureNames(requested, std::nullopt, program, error));

    tracker.observeBoundary(
        f_sp103(), MilestoneProgramPhase::PostSim, MilestoneBoundaryKind::Tick, 440, 439, 439);
    REQUIRE(tracker.authoredHits()[0].hit);
    const std::string fingerprint = tracker.authoredHits()[0].evidence.boundaryFingerprint;
    RecordingStartBinding binding;
    REQUIRE(bind_recording_start(tracker, program, "link_control", fingerprint, 438, binding) ==
            RecordingStartError::WrongTapeFrame);
    REQUIRE(
        bind_recording_start(tracker, program, "link_control", "00000000000000000000000000000000",
            439, binding) == RecordingStartError::FingerprintMismatch);
    REQUIRE(bind_recording_start(tracker, program, "link_control", fingerprint, 439, binding) ==
            RecordingStartError::None);
    REQUIRE(binding.milestone == "link_control");
    REQUIRE(binding.tapeFrame == 439);
    REQUIRE(binding.boundaryIndex == 440);
    REQUIRE(binding.boundaryFingerprint == fingerprint);
    REQUIRE(binding.programDigest == program.digest());
    REQUIRE(binding.definitionDigest == program.find("link_control")->definitionDigest);
}

}  // namespace

int main() {
    testRegistryAndParsing();
    testGameplayReadinessRequiresExactMemoryState();
    testExitRequiresCommittedExactDestination();
    testTrackerCapturesOnlyTheFirstHitAndSerializesEvidence();
    testBoundaryFingerprintIsStableAndSensitiveToExplicitState();
    testObservationFingerprintIsPortableCompleteAndOrderIndependent();
    testCheckedStageSmokeFingerprintV4();
    testGoalMustBeRequested();
    testAuthoredBootStableAndExactFirstHit();
    testAuthoredObjectiveConsumesTypedFacts();
    testCheckedInteractionProgramHasExactNativeOfflineFirstHitParity();
    testRichV11FactsAndBitMasksEvaluateNatively();
    testV12PlacedActorGeometryAndIndexedFlagsEvaluateNatively();
    testV13SpatialRelationsAndBoundedSequencesEvaluateNatively();
    testV14NamedValueProjectionsCaptureExactNativeState();
    testV18TemporaryEventBytesEvaluateExactNativeRegisters();
    testMalformedAuthoredProgramIsRejected();
    testBootRecordingGuardrailsAndBeginOrdering();
    testAcceleratedParentRecordingBoundaryAndRevealOrdering();
    testAuthoredParentRecordingStartBinding();
    std::cout << "milestone tests passed\n";
    return 0;
}
