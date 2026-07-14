#include "dusk/automation/eye_shredder_oracle.hpp"

#include <cstdlib>
#include <iostream>

#include <nlohmann/json.hpp>

namespace {

void require(bool condition, const char* expression, int line) {
    if (!condition) {
        std::cerr << "eye_shredder_oracle_test.cpp:" << line << ": check failed: " << expression
                  << '\n';
        std::abort();
    }
}

#define REQUIRE(expression) require((expression), #expression, __LINE__)

dusk::automation::NameEntryObservation make_write(const std::uint8_t index,
    const std::uint8_t column, const std::int32_t character, const bool shadowModeled) {
    using namespace dusk::automation;
    NameEntryObserver observer;
    observer.setFidelityProfile(NameEntryFidelityProfile::CursorBreakoutShadow);
    observer.setTickContext(900, 700);
    observer.beginSession();
    observer.noteCharacterWrite(index, column, 0, 2, character, true, shadowModeled);
    return observer.latest();
}

[[nodiscard]] dusk::automation::EyeShredderRendererTelemetry make_renderer_telemetry(
    const std::uint64_t mismatchDrawCount, const bool exactLatch = true) {
    return {
        .xfNumChansRaw = 12,
        .bpNumChansRaw = 4,
        .mismatchLatched = true,
        .eyeShredderMismatchLatched = exactLatch,
        .mismatchDrawCount = mismatchDrawCount,
    };
}

[[nodiscard]] dusk::automation::EyeShredderGameplayTelemetry make_gameplay_telemetry(
    const bool controllable = true) {
    return {
        .stageName = "F_SP108",
        .room = 1,
        .point = 21,
        .layer = 13,
        .playerActorName = 0x00FD,
        .playerActorPresent = true,
        .playerIsLink = true,
        .eventRunning = !controllable,
    };
}

void testExactRetailShadowWriteArmsUntilRendererDraw() {
    using namespace dusk::automation;
    EyeShredderOracle oracle;
    oracle.start();
    const NameEntryObservation observation = make_write(113, 12, 'M', true);
    oracle.evaluate(observation, 900, 700);

    REQUIRE(oracle.result().status == EyeShredderOracleStatus::Running);
    REQUIRE(!oracle.isTerminal());
    REQUIRE(oracle.result().memoryMatched);
    REQUIRE(!oracle.result().rendererMatched);
    REQUIRE(oracle.result().memoryMatchSimTick == 900);
    REQUIRE(oracle.result().memoryMatchTapeFrame == 700);
    REQUIRE(oracle.result().reason.find("waiting for an exact XF=12 / BP=4 renderer draw") !=
            std::string::npos);

    oracle.observeRendererTelemetry(make_renderer_telemetry(1), 901, 701);
    REQUIRE(oracle.result().status == EyeShredderOracleStatus::Running);
    REQUIRE(oracle.result().rendererMatched);
    REQUIRE(oracle.result().hasActualWrite);
    REQUIRE(oracle.result().hasRendererTelemetry);
    REQUIRE(oracle.result().actualWrite.characterIndex == 113);
    REQUIRE(oracle.result().actualWrite.originalOffset == 0x654);
    REQUIRE(oracle.result().actualWrite.bytes == EyeShredderExpectedWrite::Bytes);
    NameEntryObservation ended;
    oracle.evaluate(ended, 902, 702);
    oracle.observeGameplayTelemetry(make_gameplay_telemetry(), 903, 703);
    REQUIRE(oracle.result().status == EyeShredderOracleStatus::Passed);
    REQUIRE(oracle.result().gameplayMatched);
    REQUIRE(oracle.result().simTick == 903);
    REQUIRE(oracle.result().tapeFrame == 703);

    const auto json = nlohmann::json::parse(serialize_eye_shredder_oracle_result(oracle.result()));
    REQUIRE(json["schema"]["version"] == 3);
    REQUIRE(json["status"] == "pass");
    REQUIRE(json["memory_model"] == "bounded_retail_dname_shadow");
    REQUIRE(json["retail_profile"] == "fresh_gcn_ntsc_u");
    REQUIRE(json["j2d_leak"] == false);
    REQUIRE(json["renderer_effect"] == "exact_xf_bp_draw_observed");
    REQUIRE(json["gameplay_effect"] == "control_reached");
    REQUIRE(json["emulator_diagnostic"] == "Mismatched configuration between XF and BP stages");
    REQUIRE(json["expected"]["character_index"] == 113);
    REQUIRE(json["expected"]["original_offset"] == 0x654);
    REQUIRE(json["expected"]["fresh_usa_gc_cached_address"] == 0x81457688);
    REQUIRE(json["expected"]["bytes"][0] == 0x0C);
    REQUIRE(json["expected"]["bytes"][7] == 0x4D);
    REQUIRE(json["expected"]["renderer_draw"]["xf_num_chans_raw"] == 12);
    REQUIRE(json["expected"]["renderer_draw"]["bp_num_chans_raw"] == 4);
    REQUIRE(json["actual"]["bytes"] == json["expected"]["bytes"]);
    REQUIRE(json["stages"]["memory"]["matched"] == true);
    REQUIRE(json["stages"]["memory"]["actual"] == json["actual"]);
    REQUIRE(json["stages"]["renderer"]["matched"] == true);
    REQUIRE(json["stages"]["renderer"]["telemetry"]["xf_num_chans_raw"] == 12);
    REQUIRE(json["stages"]["renderer"]["telemetry"]["bp_num_chans_raw"] == 4);
    REQUIRE(json["stages"]["renderer"]["telemetry"]["mismatch_latched"] == true);
    REQUIRE(json["stages"]["renderer"]["telemetry"]["eye_shredder_mismatch_latched"] ==
            true);
    REQUIRE(json["stages"]["renderer"]["telemetry"]["mismatch_draw_count"] == 1);
    REQUIRE(json["stages"]["gameplay"]["matched"] == true);
    REQUIRE(json["stages"]["gameplay"]["sim_tick"] == 903);
    REQUIRE(json["stages"]["gameplay"]["tape_frame"] == 703);
    REQUIRE(json["stages"]["gameplay"]["telemetry"]["stage_name"] == "F_SP108");
    REQUIRE(json["stages"]["gameplay"]["telemetry"]["player_is_link"] == true);
    REQUIRE(json["stages"]["gameplay"]["telemetry"]["event_running"] == false);
}

void testWrongSignatureFails() {
    using namespace dusk::automation;
    EyeShredderOracle oracle;
    oracle.start();
    const NameEntryObservation observation = make_write(113, 0, 'A', true);
    oracle.evaluate(observation, 900, 700);
    REQUIRE(oracle.result().status == EyeShredderOracleStatus::Failed);
    REQUIRE(oracle.result().hasActualWrite);
}

void testUnmodeledWriteFails() {
    using namespace dusk::automation;
    EyeShredderOracle oracle;
    oracle.start();
    NameEntryObservation observation = make_write(113, 12, 'M', true);
    observation.lastWrite.flags &= ~NameEntryEventShadowModeled;
    oracle.evaluate(observation, 900, 700);
    REQUIRE(oracle.result().status == EyeShredderOracleStatus::Failed);
}

void testRendererLatchMustBeObservedAfterMemoryMatch() {
    using namespace dusk::automation;
    EyeShredderOracle oracle;
    oracle.start();

    oracle.observeRendererTelemetry(make_renderer_telemetry(1), 899, 699);
    oracle.evaluate(make_write(113, 12, 'M', true), 900, 700);
    oracle.observeRendererTelemetry(make_renderer_telemetry(1), 901, 701);
    REQUIRE(oracle.result().status == EyeShredderOracleStatus::Running);

    oracle.observeRendererTelemetry(make_renderer_telemetry(2), 902, 702);
    REQUIRE(oracle.result().status == EyeShredderOracleStatus::Running);
    REQUIRE(oracle.result().rendererMatchSimTick == 902);
    REQUIRE(oracle.result().rendererMatchTapeFrame == 702);
}

void testGameplayRequiresNameExitLinkAndNoEvent() {
    using namespace dusk::automation;
    EyeShredderOracle oracle;
    oracle.start();
    oracle.evaluate(make_write(113, 12, 'M', true), 900, 700);
    oracle.observeRendererTelemetry(make_renderer_telemetry(1), 901, 701);

    oracle.observeGameplayTelemetry(make_gameplay_telemetry(), 902, 702);
    REQUIRE(oracle.result().status == EyeShredderOracleStatus::Running);

    NameEntryObservation ended;
    oracle.evaluate(ended, 903, 703);
    auto gameplay = make_gameplay_telemetry();
    gameplay.playerIsLink = false;
    oracle.observeGameplayTelemetry(gameplay, 904, 704);
    REQUIRE(oracle.result().status == EyeShredderOracleStatus::Running);

    oracle.observeGameplayTelemetry(make_gameplay_telemetry(false), 905, 705);
    REQUIRE(oracle.result().status == EyeShredderOracleStatus::Running);

    oracle.observeGameplayTelemetry(make_gameplay_telemetry(), 906, 706);
    REQUIRE(oracle.result().status == EyeShredderOracleStatus::Passed);
}

void testExactRawCountsWithoutExactDrawLatchDoNotPass() {
    using namespace dusk::automation;
    EyeShredderOracle oracle;
    oracle.start();
    oracle.evaluate(make_write(113, 12, 'M', true), 900, 700);
    oracle.observeRendererTelemetry(make_renderer_telemetry(1, false), 901, 701);
    REQUIRE(oracle.result().status == EyeShredderOracleStatus::Running);

    oracle.finish(902, 702);
    REQUIRE(oracle.result().status == EyeShredderOracleStatus::Incomplete);
    REQUIRE(oracle.result().reason.find("before a subsequent exact XF=12 / BP=4 renderer draw") !=
            std::string::npos);

    const auto json = nlohmann::json::parse(serialize_eye_shredder_oracle_result(oracle.result()));
    REQUIRE(json["stages"]["memory"]["matched"] == true);
    REQUIRE(json["stages"]["renderer"]["matched"] == false);
    REQUIRE(json["stages"]["renderer"]["telemetry"]["mismatch_draw_count"] == 1);
    REQUIRE(json["stages"]["renderer"]["telemetry"]["eye_shredder_mismatch_latched"] ==
            false);
}

void testExactLatchWithNonExactRawCountsDoesNotPass() {
    using namespace dusk::automation;
    EyeShredderOracle oracle;
    oracle.start();
    oracle.evaluate(make_write(113, 12, 'M', true), 900, 700);

    EyeShredderRendererTelemetry telemetry = make_renderer_telemetry(1);
    telemetry.bpNumChansRaw = 3;
    oracle.observeRendererTelemetry(telemetry, 901, 701);
    REQUIRE(oracle.result().status == EyeShredderOracleStatus::Running);
}

void testTapeEndBeforeMemoryIsDistinctlyIncomplete() {
    using namespace dusk::automation;
    EyeShredderOracle oracle;
    oracle.start();
    NameEntryObservation observation;
    observation.active = 1;
    oracle.evaluate(observation, 50, 50);
    oracle.finish(51, 51);
    REQUIRE(oracle.result().status == EyeShredderOracleStatus::Incomplete);
    REQUIRE(oracle.isTerminal());
    REQUIRE(!oracle.result().memoryMatched);
    REQUIRE(oracle.result().reason == "input ended before the expected Eye Shredder write");
}

}  // namespace

int main() {
    testExactRetailShadowWriteArmsUntilRendererDraw();
    testWrongSignatureFails();
    testUnmodeledWriteFails();
    testRendererLatchMustBeObservedAfterMemoryMatch();
    testGameplayRequiresNameExitLinkAndNoEvent();
    testExactRawCountsWithoutExactDrawLatchDoNotPass();
    testExactLatchWithNonExactRawCountsDoesNotPass();
    testTapeEndBeforeMemoryIsDistinctlyIncomplete();
    std::cout << "eye shredder oracle tests passed\n";
    return 0;
}
