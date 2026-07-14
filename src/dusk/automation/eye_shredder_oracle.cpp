#include "dusk/automation/eye_shredder_oracle.hpp"

#include <fstream>
#include <string_view>

#include <nlohmann/json.hpp>

namespace dusk::automation {
namespace {

using json = nlohmann::ordered_json;

std::string_view status_name(const EyeShredderOracleStatus status) {
    switch (status) {
    case EyeShredderOracleStatus::Idle:
        return "idle";
    case EyeShredderOracleStatus::Running:
        return "running";
    case EyeShredderOracleStatus::Passed:
        return "pass";
    case EyeShredderOracleStatus::Failed:
        return "fail";
    case EyeShredderOracleStatus::Incomplete:
        return "incomplete";
    }
    return "unknown";
}

json bytes_json(const std::span<const std::uint8_t> bytes) {
    json output = json::array();
    for (const std::uint8_t byte : bytes) {
        output.push_back(byte);
    }
    return output;
}

json write_json(const NameEntryWriteObservation& write) {
    return {
        {"attempt", write.attempt},
        {"sim_tick", write.simTick},
        {"tape_frame", write.tapeFrame},
        {"character_index", write.characterIndex},
        {"original_offset", write.originalOffset},
        {"fresh_usa_gc_cached_address",
            EyeShredderExpectedWrite::FreshUsaDNameAddress + write.originalOffset},
        {"flags_raw", write.flags},
        {"shadow_modeled", (write.flags & NameEntryEventShadowModeled) != 0},
        {"bytes", bytes_json(write.bytes)},
    };
}

json renderer_telemetry_json(const EyeShredderRendererTelemetry& telemetry) {
    return {
        {"xf_num_chans_raw", telemetry.xfNumChansRaw},
        {"bp_num_chans_raw", telemetry.bpNumChansRaw},
        {"mismatch_latched", telemetry.mismatchLatched},
        {"eye_shredder_mismatch_latched", telemetry.eyeShredderMismatchLatched},
        {"mismatch_draw_count", telemetry.mismatchDrawCount},
    };
}

json gameplay_telemetry_json(const EyeShredderGameplayTelemetry& telemetry) {
    return {
        {"stage_name", telemetry.stageName},
        {"room", telemetry.room},
        {"point", telemetry.point},
        {"layer", telemetry.layer},
        {"player_actor_name", telemetry.playerActorName},
        {"player_actor_present", telemetry.playerActorPresent},
        {"player_is_link", telemetry.playerIsLink},
        {"event_running", telemetry.eventRunning},
    };
}

}  // namespace

void EyeShredderOracle::start() {
    mResult = {};
    mResult.status = EyeShredderOracleStatus::Running;
    mResult.reason = "waiting for retail-layout shadow write at Eye Shredder position 113";
    mLastWriteAttempt = 0;
    mRendererMismatchDrawCountAtMemoryMatch = 0;
    mSawNameEntry = false;
    mNameEntryEnded = false;
    mSawNewGameplayEvent = false;
}

void EyeShredderOracle::evaluate(const NameEntryObservation& observation,
    const std::uint64_t simTick, const std::uint64_t tapeFrame) {
    if (mResult.status != EyeShredderOracleStatus::Running) {
        return;
    }

    mResult.simTick = simTick;
    mResult.tapeFrame = tapeFrame;
    if (observation.active != 0) {
        mSawNameEntry = true;
    } else if (mSawNameEntry) {
        if (!mResult.memoryMatched) {
            fail("name-entry session ended before the expected Eye Shredder write", nullptr,
                simTick, tapeFrame);
            return;
        }
        mNameEntryEnded = true;
    }

    if (mResult.memoryMatched) {
        return;
    }

    const NameEntryWriteObservation& write = observation.lastWrite;
    if (write.attempt == 0 || write.attempt == mLastWriteAttempt) {
        return;
    }
    mLastWriteAttempt = write.attempt;

    if (write.characterIndex < EyeShredderExpectedWrite::CharacterIndex) {
        return;
    }
    if (write.characterIndex > EyeShredderExpectedWrite::CharacterIndex) {
        fail("cursor passed Eye Shredder position 113 without the expected write", &write, simTick,
            tapeFrame);
        return;
    }

    NameEntryWriteObservation actual = write;
    constexpr std::size_t shadowOffset =
        EyeShredderExpectedWrite::OriginalOffset - NameEntryOriginalLayout::NeighborWindow;
    for (std::size_t i = 0; i < actual.bytes.size(); ++i) {
        actual.bytes[i] = observation.modeledRetailBytes[shadowOffset + i];
    }
    mResult.hasActualWrite = true;
    mResult.actualWrite = actual;
    mResult.simTick = actual.simTick;
    mResult.tapeFrame = actual.tapeFrame;

    if ((actual.flags & NameEntryEventAccepted) == 0 ||
        (actual.flags & NameEntryEventShadowModeled) == 0)
    {
        fail("position 113 write was not modeled in bounded retail-layout shadow memory", &actual,
            simTick, tapeFrame);
        return;
    }
    if (actual.originalOffset != EyeShredderExpectedWrite::OriginalOffset) {
        fail("position 113 write had the wrong retail dName_c-relative offset", &actual, simTick,
            tapeFrame);
        return;
    }
    if (actual.bytes != EyeShredderExpectedWrite::Bytes) {
        fail("position 113 write bytes did not match the NTSC-U M corruption signature", &actual,
            simTick, tapeFrame);
        return;
    }

    mResult.memoryMatched = true;
    mResult.memoryMatchSimTick = actual.simTick;
    mResult.memoryMatchTapeFrame = actual.tapeFrame;
    mRendererMismatchDrawCountAtMemoryMatch =
        mResult.hasRendererTelemetry ? mResult.rendererTelemetry.mismatchDrawCount : 0;
    mResult.reason =
        "matched Eye Shredder memory write; waiting for an exact XF=12 / BP=4 renderer draw";
}

void EyeShredderOracle::observeRendererTelemetry(
    const EyeShredderRendererTelemetry& telemetry, const std::uint64_t simTick,
    const std::uint64_t tapeFrame) {
    if (mResult.status != EyeShredderOracleStatus::Running) {
        return;
    }

    mResult.hasRendererTelemetry = true;
    mResult.rendererTelemetry = telemetry;
    mResult.simTick = simTick;
    mResult.tapeFrame = tapeFrame;

    if (mResult.rendererMatched) {
        return;
    }

    const bool observedNewMismatchDraw =
        telemetry.mismatchDrawCount > mRendererMismatchDrawCountAtMemoryMatch;
    if (!mResult.memoryMatched || !observedNewMismatchDraw) {
        return;
    }
    if (!telemetry.mismatchLatched || !telemetry.eyeShredderMismatchLatched ||
        telemetry.xfNumChansRaw != 12 || telemetry.bpNumChansRaw != 4) {
        return;
    }

    mResult.rendererMatched = true;
    mResult.rendererMatchSimTick = simTick;
    mResult.rendererMatchTapeFrame = tapeFrame;
    mResult.reason =
        "matched Eye Shredder memory write and exact XF=12 / BP=4 renderer draw; waiting for "
        "controllable new-gameplay state";
}

void EyeShredderOracle::observeGameplayTelemetry(const EyeShredderGameplayTelemetry& telemetry,
    const std::uint64_t simTick, const std::uint64_t tapeFrame) {
    if (mResult.status != EyeShredderOracleStatus::Running) {
        return;
    }

    mResult.hasGameplayTelemetry = true;
    mResult.gameplayTelemetry = telemetry;
    mResult.simTick = simTick;
    mResult.tapeFrame = tapeFrame;

    const bool isNewGameStage = mNameEntryEnded && telemetry.stageName == "F_SP108" &&
        telemetry.room == 1 && telemetry.point == 21 && telemetry.layer == 13;
    if (isNewGameStage && telemetry.eventRunning) {
        mSawNewGameplayEvent = true;
    }

    const bool isPostOpeningGameplay = telemetry.stageName == "F_SP103" &&
        telemetry.room == 1 && telemetry.point == 1 && telemetry.layer == -1;
    const bool isControllableNewGame = mResult.rendererMatched && mNameEntryEnded &&
        mSawNewGameplayEvent && isPostOpeningGameplay && telemetry.playerActorPresent &&
        telemetry.playerIsLink && !telemetry.eventRunning;
    if (!isControllableNewGame) {
        return;
    }

    mResult.gameplayMatched = true;
    mResult.gameplayMatchSimTick = simTick;
    mResult.gameplayMatchTapeFrame = tapeFrame;
    mResult.status = EyeShredderOracleStatus::Passed;
    mResult.reason = "matched Eye Shredder and reached controllable new gameplay";
}

void EyeShredderOracle::observeTapeCompletion(
    const std::uint64_t simTick, const std::uint64_t tapeFrame) {
    mResult.tapeCompleted = true;
    mResult.tapeCompletionSimTick = simTick;
    mResult.tapeCompletionFrame = tapeFrame;
}

void EyeShredderOracle::finish(const std::uint64_t simTick, const std::uint64_t tapeFrame) {
    if (mResult.status != EyeShredderOracleStatus::Running) {
        return;
    }
    mResult.status = EyeShredderOracleStatus::Incomplete;
    if (mResult.rendererMatched) {
        mResult.reason =
            "matched Eye Shredder renderer draw but input ended before controllable new gameplay";
    } else if (mResult.memoryMatched) {
        mResult.reason =
            "matched Eye Shredder memory write but input ended before a subsequent exact "
            "XF=12 / BP=4 renderer draw";
    } else {
        mResult.reason = mSawNameEntry ? "input ended before the expected Eye Shredder write" :
                                         "input ended before reaching the name-entry session";
    }
    mResult.simTick = simTick;
    mResult.tapeFrame = tapeFrame;
}

void EyeShredderOracle::reject(std::string reason) {
    if (mResult.status != EyeShredderOracleStatus::Running) {
        return;
    }
    fail(std::move(reason), nullptr, NameEntryNoTick, NameEntryNoTick);
}

bool EyeShredderOracle::isTerminal() const {
    return mResult.status == EyeShredderOracleStatus::Passed ||
           mResult.status == EyeShredderOracleStatus::Failed ||
           mResult.status == EyeShredderOracleStatus::Incomplete;
}

void EyeShredderOracle::fail(std::string reason, const NameEntryWriteObservation* actual,
    const std::uint64_t simTick, const std::uint64_t tapeFrame) {
    mResult.status = EyeShredderOracleStatus::Failed;
    mResult.reason = std::move(reason);
    mResult.simTick = simTick;
    mResult.tapeFrame = tapeFrame;
    if (actual != nullptr) {
        mResult.hasActualWrite = true;
        mResult.actualWrite = *actual;
        mResult.simTick = actual->simTick;
        mResult.tapeFrame = actual->tapeFrame;
    }
}

std::string serialize_eye_shredder_oracle_result(const EyeShredderOracleResult& result) {
    json actual = nullptr;
    if (result.hasActualWrite) {
        actual = write_json(result.actualWrite);
    }
    json rendererTelemetry = nullptr;
    if (result.hasRendererTelemetry) {
        rendererTelemetry = renderer_telemetry_json(result.rendererTelemetry);
    }
    json gameplayTelemetry = nullptr;
    if (result.hasGameplayTelemetry) {
        gameplayTelemetry = gameplay_telemetry_json(result.gameplayTelemetry);
    }

    const json root = {
        {"schema", {{"name", "dusklight.eye_shredder_oracle"},
                       {"version", EyeShredderOracleSchemaVersion}}},
        {"oracle", "eye-shredder"},
        {"status", status_name(result.status)},
        {"reason", result.reason},
        {"sim_tick", result.simTick},
        {"tape_frame", result.tapeFrame},
        {"memory_model", "bounded_retail_dname_shadow"},
        {"retail_profile", "fresh_gcn_ntsc_u"},
        {"j2d_leak", false},
        {"renderer_effect",
            result.rendererMatched ? "exact_xf_bp_draw_observed" : "not_observed"},
        {"gameplay_effect", result.gameplayMatched ? "control_reached" : "not_reached"},
        {"emulator_diagnostic", "Mismatched configuration between XF and BP stages"},
        {"expected",
            {{"character_index", EyeShredderExpectedWrite::CharacterIndex},
                {"original_offset", EyeShredderExpectedWrite::OriginalOffset},
                {"fresh_usa_gc_cached_address", EyeShredderExpectedWrite::FreshUsaGcCachedAddress},
                {"bytes", bytes_json(EyeShredderExpectedWrite::Bytes)},
                {"renderer_draw",
                    {{"xf_num_chans_raw", 12}, {"bp_num_chans_raw", 4},
                        {"eye_shredder_mismatch_latched", true}}}}},
        {"stages",
            {{"memory",
                 {{"matched", result.memoryMatched},
                     {"sim_tick", result.memoryMatchSimTick},
                     {"tape_frame", result.memoryMatchTapeFrame},
                     {"actual", actual}}},
                {"renderer",
                    {{"matched", result.rendererMatched},
                        {"sim_tick", result.rendererMatchSimTick},
                        {"tape_frame", result.rendererMatchTapeFrame},
                        {"telemetry", std::move(rendererTelemetry)}}},
                {"gameplay",
                    {{"matched", result.gameplayMatched},
                        {"sim_tick", result.gameplayMatchSimTick},
                        {"tape_frame", result.gameplayMatchTapeFrame},
                        {"telemetry", std::move(gameplayTelemetry)}}},
                {"tape",
                    {{"completed", result.tapeCompleted},
                        {"sim_tick", result.tapeCompletionSimTick},
                        {"tape_frame", result.tapeCompletionFrame}}}}},
        {"actual", std::move(actual)},
    };
    return root.dump(2) + '\n';
}

bool write_eye_shredder_oracle_result(const std::filesystem::path& path,
    const EyeShredderOracleResult& result, std::string& error) noexcept {
    try {
        std::ofstream output(path, std::ios::binary | std::ios::trunc);
        if (!output) {
            error = "could not open output file";
            return false;
        }
        const std::string encoded = serialize_eye_shredder_oracle_result(result);
        output.write(encoded.data(), static_cast<std::streamsize>(encoded.size()));
        output.flush();
        if (!output) {
            error = "failed while writing output file";
            return false;
        }
        return true;
    } catch (const std::exception& exception) {
        error = exception.what();
        return false;
    }
}

}  // namespace dusk::automation
