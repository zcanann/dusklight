#include "dusk/automation/rng.hpp"

#include "SSystem/SComponent/c_math_rng.h"

namespace dusk::automation {
namespace {

cM_RndStream native_id(const GameRngStreamId id) {
    return id == GameRngStreamId::Primary ? cM_RndStream::Primary : cM_RndStream::Secondary;
}

GameRngStreamSnapshot capture_stream(const GameRngStreamId id) {
    cM_RndState state;
    cM_getRndState(native_id(id), state);
    return {
        .id = id,
        .algorithmVersion = kGameRngAlgorithmVersion,
        .state0 = state.state0,
        .state1 = state.state1,
        .state2 = state.state2,
        .callCount = state.callCount,
    };
}

} // namespace

GameRngSnapshot capture_game_rng_snapshot() {
    GameRngSnapshot snapshot;
    snapshot.streams[0] = capture_stream(GameRngStreamId::Primary);
    snapshot.streams[1] = capture_stream(GameRngStreamId::Secondary);
    return snapshot;
}

GameRngRestoreError restore_game_rng_snapshot(const GameRngSnapshot& snapshot) {
    if (snapshot.version != kGameRngSnapshotVersion) {
        return GameRngRestoreError::UnsupportedSnapshotVersion;
    }
    if (snapshot.streamCount != kGameRngStreamCount) {
        return GameRngRestoreError::InvalidStreamCount;
    }
    for (std::size_t i = 0; i < snapshot.streams.size(); ++i) {
        if (snapshot.streams[i].id != static_cast<GameRngStreamId>(i)) {
            return GameRngRestoreError::InvalidStreamId;
        }
        if (snapshot.streams[i].algorithmVersion != kGameRngAlgorithmVersion) {
            return GameRngRestoreError::UnsupportedAlgorithmVersion;
        }
    }

    for (const GameRngStreamSnapshot& stream : snapshot.streams) {
        cM_RndState state;
        state.state0 = stream.state0;
        state.state1 = stream.state1;
        state.state2 = stream.state2;
        state.callCount = stream.callCount;
        cM_setRndState(native_id(stream.id), state);
    }
    return GameRngRestoreError::None;
}

} // namespace dusk::automation
