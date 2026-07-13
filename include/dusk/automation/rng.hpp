#pragma once

#include <array>
#include <cstddef>
#include <cstdint>

namespace dusk::automation {

inline constexpr std::uint32_t kGameRngSnapshotVersion = 1;
inline constexpr std::uint32_t kGameRngAlgorithmVersion = 1;
inline constexpr std::size_t kGameRngStreamCount = 2;

enum class GameRngStreamId : std::uint8_t {
    Primary = 0,
    Secondary = 1,
};

struct GameRngStreamSnapshot {
    GameRngStreamId id = GameRngStreamId::Primary;
    std::array<std::uint8_t, 3> reserved{};
    std::uint32_t algorithmVersion = kGameRngAlgorithmVersion;
    std::int32_t state0 = 0;
    std::int32_t state1 = 0;
    std::int32_t state2 = 0;
    std::uint64_t callCount = 0;

    bool operator==(const GameRngStreamSnapshot&) const = default;
};

struct GameRngSnapshot {
    std::uint32_t version = kGameRngSnapshotVersion;
    std::uint32_t streamCount = kGameRngStreamCount;
    std::array<GameRngStreamSnapshot, kGameRngStreamCount> streams{};

    bool operator==(const GameRngSnapshot&) const = default;
};

enum class GameRngRestoreError {
    None,
    UnsupportedSnapshotVersion,
    InvalidStreamCount,
    InvalidStreamId,
    UnsupportedAlgorithmVersion,
};

GameRngSnapshot capture_game_rng_snapshot();
GameRngRestoreError restore_game_rng_snapshot(const GameRngSnapshot& snapshot);

} // namespace dusk::automation
