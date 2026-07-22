#pragma once

#include "dusk/automation/input_tape.hpp"

#include <array>
#include <cstddef>
#include <cstdint>
#include <string>
#include <string_view>

namespace dusk::automation {

inline constexpr std::size_t kNativePolicyFeatureWidth = 120;
inline constexpr std::string_view kNativePolicyFeatureSchema = R"SCHEMA(dusk.native-policy-features/v1
player_present:1
player_is_link:1
player_position_if_present:3
player_velocity_if_present:3
player_forward_speed_if_present:1
player_current_yaw_i16_over_32768_if_present:1
player_shape_yaw_i16_over_32768_if_present:1
player_contacts_ground_wall_roof_water_water_in_if_present:5
player_ground_height_present:1
player_ground_height_if_present:1
player_roof_height_present:1
player_roof_height_if_present:1
event_running:1
event_mode_u8_over_255:1
event_status_u8_over_255:1
event_map_tool_id_u8_over_255:1
next_stage_enabled:1
camera_present:1
camera_yaw_radians_if_present:1
collision_correction_present:1
collision_correction_xz_if_present:2
remaining_ticks_u32:1
previous_input_connected:1
previous_input_axes_signed_normalized:4
previous_input_analogs_u8_over_255:4
previous_input_button_bits_lsb:16
previous_input_error_i8_over_128:1
player_damage_wait_i16_over_32768_if_present:1
player_ice_damage_wait_i16_over_32768_if_present:1
player_sword_change_wait_u8_over_255_if_present:1
player_do_status_u8_over_255_if_present:1
stage_name_ascii_u8_over_127_zero_padded:8
room_i8_over_128:1
layer_i8_over_128:1
point_i16_over_32768:1
player_procedure_bits_lsb_if_present:16
player_mode_bits_lsb_if_present:32
)SCHEMA";
inline constexpr std::string_view kNativePolicyFeatureSchemaSha256 =
    "b0b708c7594f25f74e3f82c8ebbde2b737cc4f1f0076e114efddb823a538d147";
inline constexpr std::array<std::uint8_t, 32> kNativePolicyFeatureSchemaDigest{
    0xb0, 0xb7, 0x08, 0xc7, 0x59, 0x4f, 0x25, 0xf7,
    0x4e, 0x3f, 0x82, 0xc8, 0xeb, 0xbd, 0xe2, 0xb7,
    0x37, 0xcc, 0x4f, 0x1f, 0x00, 0x76, 0xe1, 0x14,
    0xef, 0xdd, 0xb8, 0x23, 0xa5, 0x38, 0xd1, 0x47,
};

struct NativePolicyFeatureInput {
    bool playerPresent = false;
    bool playerIsLink = false;
    std::array<float, 3> playerPosition{};
    std::array<float, 3> playerVelocity{};
    float playerForwardSpeed = 0.0F;
    std::int16_t playerCurrentYaw = 0;
    std::int16_t playerShapeYaw = 0;
    bool playerGroundContact = false;
    bool playerWallContact = false;
    bool playerRoofContact = false;
    bool playerWaterContact = false;
    bool playerWaterIn = false;
    bool playerGroundHeightPresent = false;
    float playerGroundHeight = 0.0F;
    bool playerRoofHeightPresent = false;
    float playerRoofHeight = 0.0F;
    bool eventRunning = false;
    std::uint8_t eventMode = 0;
    std::uint8_t eventStatus = 0;
    std::uint8_t eventMapToolId = 0;
    bool nextStageEnabled = false;
    bool cameraPresent = false;
    float cameraYawRadians = 0.0F;
    bool collisionCorrectionPresent = false;
    float collisionCorrectionX = 0.0F;
    float collisionCorrectionZ = 0.0F;
    std::uint32_t remainingTicks = 0;
    RawPadState previousInput{};
    std::int16_t playerDamageWaitTimer = 0;
    std::int16_t playerIceDamageWaitTimer = 0;
    std::uint8_t playerSwordChangeWaitTimer = 0;
    std::uint8_t playerDoStatus = 0;
    std::array<char, 8> stageName{};
    std::int8_t room = -1;
    std::int8_t layer = -1;
    std::int16_t point = -1;
    std::uint16_t playerProcedure = 0xffff;
    std::uint32_t playerModeFlags = 0;
};

using NativePolicyFeatureRow = std::array<float, kNativePolicyFeatureWidth>;

[[nodiscard]] bool encode_native_policy_features(
    const NativePolicyFeatureInput& input, NativePolicyFeatureRow& output, std::string& error);

}  // namespace dusk::automation
