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

void testExactRetailShadowWritePasses() {
    using namespace dusk::automation;
    EyeShredderOracle oracle;
    oracle.start();
    const NameEntryObservation observation = make_write(113, 12, 'M', true);
    oracle.evaluate(observation, 900, 700);

    REQUIRE(oracle.result().status == EyeShredderOracleStatus::Passed);
    REQUIRE(oracle.result().hasActualWrite);
    REQUIRE(oracle.result().actualWrite.characterIndex == 113);
    REQUIRE(oracle.result().actualWrite.originalOffset == 0x654);
    REQUIRE(oracle.result().actualWrite.bytes == EyeShredderExpectedWrite::Bytes);
    REQUIRE(oracle.result().simTick == 900);
    REQUIRE(oracle.result().tapeFrame == 700);

    const auto json = nlohmann::json::parse(serialize_eye_shredder_oracle_result(oracle.result()));
    REQUIRE(json["status"] == "pass");
    REQUIRE(json["memory_model"] == "bounded_retail_dname_shadow");
    REQUIRE(json["retail_profile"] == "fresh_gcn_ntsc_u");
    REQUIRE(json["j2d_leak"] == false);
    REQUIRE(json["renderer_effect"] == "console_only_not_reproduced");
    REQUIRE(json["emulator_diagnostic"] == "Mismatched configuration between XF and BP stages");
    REQUIRE(json["expected"]["character_index"] == 113);
    REQUIRE(json["expected"]["original_offset"] == 0x654);
    REQUIRE(json["expected"]["fresh_usa_gc_cached_address"] == 0x81457688);
    REQUIRE(json["expected"]["bytes"][0] == 0x0C);
    REQUIRE(json["expected"]["bytes"][7] == 0x4D);
    REQUIRE(json["actual"]["bytes"] == json["expected"]["bytes"]);
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

void testTapeEndIsIncomplete() {
    using namespace dusk::automation;
    EyeShredderOracle oracle;
    oracle.start();
    NameEntryObservation observation;
    observation.active = 1;
    oracle.evaluate(observation, 50, 50);
    oracle.finish(51, 51);
    REQUIRE(oracle.result().status == EyeShredderOracleStatus::Incomplete);
    REQUIRE(oracle.isTerminal());
}

}  // namespace

int main() {
    testExactRetailShadowWritePasses();
    testWrongSignatureFails();
    testUnmodeledWriteFails();
    testTapeEndIsIncomplete();
    std::cout << "eye shredder oracle tests passed\n";
    return 0;
}
